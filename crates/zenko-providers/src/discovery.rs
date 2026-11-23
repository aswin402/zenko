//! Dynamic live model discovery, metadata hydration (`models.dev` + Ollama `/api/show`),
//! working-model filtering, and free-first deterministic ranking.
//! Zero hardcoded model IDs, zero hardcoded context windows, zero regex, zero `if/else` ladders.

use chrono::{DateTime, NaiveDate, Utc};
use futures_util::stream::{self, StreamExt};
use reqwest::Client;
use serde_json::{Value, json};
use std::cmp::{Ordering, Reverse};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use zenko_core::{
    DEPRECATED_MODEL_STATUSES, MODEL_METADATA_REGISTRY_URL, ModelInfo, NON_CHAT_MODEL_SEGMENTS,
    OLLAMA_SHOW_CONCURRENCY, OLLAMA_SHOW_TIMEOUT_SECS, ProviderDescriptor, Result, ZenkoError,
};

/// Normalized metadata hydrated dynamically from `models.dev/api.json`.
#[derive(Debug, Clone)]
pub struct ExternalModelMeta {
    pub id: String,
    pub name: String,
    pub context_length: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub prompt_price_per_m: Option<f64>,
    pub completion_price_per_m: Option<f64>,
    pub supports_tools: bool,
    pub supports_reasoning: bool,
    pub is_deprecated: bool,
    pub outputs_text: bool,
}

/// Raw model record extracted from a provider's live `/models` endpoint.
#[derive(Debug, Clone)]
pub struct RawProviderModel {
    pub id: String,
    pub name: Option<String>,
    pub context_length: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub prompt_price_per_m: Option<f64>,
    pub completion_price_per_m: Option<f64>,
    pub request_price: f64,
    pub has_negative_price: bool,
    pub supports_tools: Option<bool>,
    pub supports_reasoning: Option<bool>,
    pub is_expired_or_deprecated: bool,
    pub outputs_text: bool,
}

/// Fetches the live `models.dev/api.json` payload (best-effort; returns `None` when offline).
pub async fn fetch_metadata_registry(client: &Client) -> Option<Value> {
    let resp = client
        .get(MODEL_METADATA_REGISTRY_URL)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    match resp.status().is_success() {
        true => resp.json::<Value>().await.ok(),
        false => None,
    }
}

/// Extracts active/known models for `provider_keys` from a `models.dev/api.json` document.
pub fn extract_external_models(
    registry_json: &Value,
    provider_keys: &[&str],
) -> Vec<ExternalModelMeta> {
    let Some(root) = registry_json.as_object() else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for key in provider_keys {
        let Some(models_obj) = root
            .get(*key)
            .and_then(|p| p.get("models"))
            .and_then(Value::as_object)
        else {
            continue;
        };

        for (model_id, m) in models_obj {
            if !seen.insert(model_id.clone()) {
                continue;
            }
            out.push(parse_external_model(model_id, m));
        }
    }

    out
}

fn parse_external_model(model_id: &str, m: &Value) -> ExternalModelMeta {
    let name = m
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(model_id)
        .to_string();

    let context_length = m
        .get("limit")
        .and_then(|l| l.get("context").or_else(|| l.get("input")))
        .and_then(value_as_nonzero_u64);

    let max_output_tokens = m
        .get("limit")
        .and_then(|l| l.get("output"))
        .and_then(value_as_nonzero_u64);

    let prompt_price_per_m = m
        .get("cost")
        .and_then(|c| c.get("input"))
        .and_then(value_as_nonneg_f64);

    let completion_price_per_m = m
        .get("cost")
        .and_then(|c| c.get("output"))
        .and_then(value_as_nonneg_f64);

    let caps = m.get("capabilities");
    let supports_tools = m
        .get("tool_call")
        .and_then(Value::as_bool)
        .or_else(|| caps.and_then(|c| c.get("tool_call")).and_then(Value::as_bool))
        .unwrap_or(false);

    let supports_reasoning = m
        .get("reasoning")
        .and_then(Value::as_bool)
        .or_else(|| caps.and_then(|c| c.get("reasoning")).and_then(Value::as_bool))
        .unwrap_or(false);

    let status_lower = m
        .get("status")
        .and_then(Value::as_str)
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let is_deprecated = DEPRECATED_MODEL_STATUSES
        .iter()
        .any(|bad| status_lower == *bad);

    let outputs_text = match m
        .get("modalities")
        .and_then(|md| md.get("output"))
        .and_then(Value::as_array)
    {
        Some(arr) if !arr.is_empty() => arr
            .iter()
            .filter_map(Value::as_str)
            .any(|s| s.eq_ignore_ascii_case("text")),
        _ => true,
    };

    ExternalModelMeta {
        id: model_id.to_string(),
        name,
        context_length,
        max_output_tokens,
        prompt_price_per_m,
        completion_price_per_m,
        supports_tools,
        supports_reasoning,
        is_deprecated,
        outputs_text,
    }
}

/// Parses live `/models` JSON across OpenRouter, OpenAI, DeepSeek, MiniMax, Gemini, Ollama & vLLM.
pub fn parse_provider_models_json(val: &Value, now: DateTime<Utc>) -> Vec<RawProviderModel> {
    let entries = val
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| val.get("models").and_then(Value::as_array));

    let Some(items) = entries else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|m| parse_single_provider_model(m, now))
        .collect()
}

fn parse_single_provider_model(m: &Value, now: DateTime<Utc>) -> Option<RawProviderModel> {
    let id = m
        .get("id")
        .or_else(|| m.get("name"))
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if id.is_empty() {
        return None;
    }

    let name = m
        .get("name")
        .or_else(|| m.get("displayName"))
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(String::from);

    let top = m.get("top_provider");
    let context_length = m
        .get("context_length")
        .and_then(value_as_nonzero_u64)
        .or_else(|| top.and_then(|t| t.get("context_length")).and_then(value_as_nonzero_u64))
        .or_else(|| m.get("input_token_limit").and_then(value_as_nonzero_u64))
        .or_else(|| m.get("inputTokenLimit").and_then(value_as_nonzero_u64))
        .or_else(|| m.get("max_context_length").and_then(value_as_nonzero_u64))
        .or_else(|| m.get("max_model_len").and_then(value_as_nonzero_u64));

    let max_output_tokens = top
        .and_then(|t| t.get("max_completion_tokens"))
        .and_then(value_as_nonzero_u64)
        .or_else(|| m.get("max_completion_tokens").and_then(value_as_nonzero_u64))
        .or_else(|| m.get("max_output_tokens").and_then(value_as_nonzero_u64))
        .or_else(|| m.get("output_token_limit").and_then(value_as_nonzero_u64))
        .or_else(|| m.get("outputTokenLimit").and_then(value_as_nonzero_u64));

    let pricing = m.get("pricing");
    let raw_prompt = pricing.and_then(|p| p.get("prompt")).and_then(value_as_f64);
    let raw_completion = pricing
        .and_then(|p| p.get("completion"))
        .and_then(value_as_f64);
    let raw_request = pricing
        .and_then(|p| p.get("request"))
        .and_then(value_as_f64)
        .unwrap_or(0.0);

    let has_negative_price = raw_prompt.map(|v| v < 0.0).unwrap_or(false)
        || raw_completion.map(|v| v < 0.0).unwrap_or(false)
        || raw_request < 0.0;

    // Provider `/models` endpoints (e.g. OpenRouter) report per-token USD strings; convert to USD / 1M tokens.
    let prompt_price_per_m = raw_prompt
        .filter(|v| *v >= 0.0)
        .map(|v| v * 1_000_000.0);
    let completion_price_per_m = raw_completion
        .filter(|v| *v >= 0.0)
        .map(|v| v * 1_000_000.0);

    let params = m.get("supported_parameters").and_then(Value::as_array);
    let supports_tools = params.map(|arr| {
        arr.iter()
            .filter_map(Value::as_str)
            .any(|p| p == "tools" || p == "tool_choice")
    });
    let supports_reasoning = params.map(|arr| {
        arr.iter()
            .filter_map(Value::as_str)
            .any(|p| p == "reasoning" || p == "include_reasoning")
    });

    let status_lower = m
        .get("status")
        .and_then(Value::as_str)
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    let deprecated_status = DEPRECATED_MODEL_STATUSES
        .iter()
        .any(|bad| status_lower == *bad);
    let expired = m
        .get("expiration_date")
        .and_then(Value::as_str)
        .map(|exp| is_date_expired(exp, now))
        .unwrap_or(false);

    let outputs_text = parse_outputs_text(m.get("architecture"));

    Some(RawProviderModel {
        id,
        name,
        context_length,
        max_output_tokens,
        prompt_price_per_m,
        completion_price_per_m,
        request_price: raw_request,
        has_negative_price,
        supports_tools,
        supports_reasoning,
        is_expired_or_deprecated: deprecated_status || expired,
        outputs_text,
    })
}

fn parse_outputs_text(arch: Option<&Value>) -> bool {
    let Some(a) = arch else {
        return true;
    };
    if let Some(arr) = a.get("output_modalities").and_then(Value::as_array)
        && !arr.is_empty() {
            return arr
                .iter()
                .filter_map(Value::as_str)
                .any(|s| s.eq_ignore_ascii_case("text"));
        }
    if let Some(mod_str) = a.get("modality").and_then(Value::as_str)
        && let Some((_, rhs)) = mod_str.split_once("->") {
            return rhs.to_ascii_lowercase().contains("text");
        }
    true
}

fn is_date_expired(raw: &str, now: DateTime<Utc>) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return dt.with_timezone(&Utc) < now;
    }
    if let Ok(date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return date < now.date_naive();
    }
    false
}

/// Checks if a model ID clearly denotes a non-chat modality (audio/embedding/image/video).
fn is_non_chat_id(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    NON_CHAT_MODEL_SEGMENTS
        .iter()
        .any(|seg| lower.contains(seg))
}

fn value_as_nonzero_u64(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
        .filter(|n| *n > 0)
}

fn value_as_f64(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
        .filter(|n| n.is_finite())
}

fn value_as_nonneg_f64(v: &Value) -> Option<f64> {
    value_as_f64(v).filter(|n| *n >= 0.0)
}

/// Extracts exact `*.context_length` and capabilities from Ollama's `/api/show` payload.
pub fn parse_ollama_show_json(val: &Value) -> (Option<u64>, bool, bool) {
    let ctx = val
        .get("model_info")
        .and_then(Value::as_object)
        .and_then(|obj| {
            obj.iter()
                .find(|(k, _)| k.ends_with(".context_length"))
                .and_then(|(_, v)| value_as_nonzero_u64(v))
        });

    let caps = val.get("capabilities").and_then(Value::as_array);
    let tools = caps
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .any(|c| c.eq_ignore_ascii_case("tools"))
        })
        .unwrap_or(false);
    let thinking = caps
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .any(|c| c.eq_ignore_ascii_case("thinking"))
        })
        .unwrap_or(false);

    (ctx, tools, thinking)
}

/// Concurrently enriches local Ollama models with exact context window via `/api/show`.
async fn enrich_ollama_models(
    client: &Client,
    base_url: &str,
    models: Vec<RawProviderModel>,
) -> Vec<ModelInfo> {
    let trimmed = base_url.trim_end_matches('/');
    let origin = trimmed.strip_suffix("/v1").unwrap_or(trimmed).to_string();
    let show_url = format!("{origin}/api/show");

    let mut enriched: Vec<ModelInfo> = stream::iter(models)
        .filter(|m| {
            let keep = !m.is_expired_or_deprecated && m.outputs_text && !is_non_chat_id(&m.id);
            futures_util::future::ready(keep)
        })
        .map(|m| {
            let http = client.clone();
            let url = show_url.clone();
            async move {
                let (ctx, tools, thinking) = match http
                    .post(&url)
                    .timeout(Duration::from_secs(OLLAMA_SHOW_TIMEOUT_SECS))
                    .json(&json!({ "name": m.id }))
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => resp
                        .json::<Value>()
                        .await
                        .ok()
                        .map(|v| parse_ollama_show_json(&v))
                        .unwrap_or((None, false, false)),
                    _ => (None, false, false),
                };

                ModelInfo {
                    name: m.name.unwrap_or_else(|| m.id.clone()),
                    id: m.id,
                    context_length: m.context_length.or(ctx),
                    max_output_tokens: m.max_output_tokens,
                    prompt_price_per_m: Some(0.0),
                    completion_price_per_m: Some(0.0),
                    is_free: true,
                    supports_tools: m.supports_tools.unwrap_or(tools),
                    supports_reasoning: m.supports_reasoning.unwrap_or(thinking),
                    source: "ollama".to_string(),
                }
            }
        })
        .buffer_unordered(OLLAMA_SHOW_CONCURRENCY)
        .collect()
        .await;

    rank_models(&mut enriched);
    enriched
}

fn build_external_lookup(external: &[ExternalModelMeta]) -> HashMap<String, &ExternalModelMeta> {
    let mut map = HashMap::with_capacity(external.len() * 2);
    for meta in external {
        map.insert(meta.id.clone(), meta);
        map.insert(meta.id.to_ascii_lowercase(), meta);
        if let Some((_, short)) = meta.id.rsplit_once('/') {
            map.entry(short.to_ascii_lowercase()).or_insert(meta);
        }
    }
    map
}

fn find_external<'a>(
    index: &HashMap<String, &'a ExternalModelMeta>,
    id: &str,
) -> Option<&'a ExternalModelMeta> {
    let lower = id.to_ascii_lowercase();
    let base_without_tier = lower.strip_suffix(":free").unwrap_or(&lower);
    let short_slug = base_without_tier
        .rsplit_once('/')
        .map(|(_, s)| s)
        .unwrap_or(base_without_tier);

    index
        .get(id)
        .or_else(|| index.get(&lower))
        .or_else(|| index.get(base_without_tier))
        .or_else(|| index.get(short_slug))
        .copied()
}

/// Pure merge, working-model filter, free detection, and free-first ranking.
pub fn merge_and_rank_models(
    live: Vec<RawProviderModel>,
    external: Vec<ExternalModelMeta>,
    desc: &ProviderDescriptor,
) -> Vec<ModelInfo> {
    let ext_index = build_external_lookup(&external);
    let is_local_free = desc.is_local_free;

    let mut resolved: Vec<ModelInfo> = match (live.is_empty(), external.is_empty()) {
        // Provider returned live `/models`: treat as authoritative working inventory and hydrate missing metadata.
        (false, _) => live
            .into_iter()
            .filter_map(|raw| {
                let ext = find_external(&ext_index, &raw.id);
                let deprecated = raw.is_expired_or_deprecated
                    || ext.map(|e| e.is_deprecated).unwrap_or(false);
                let outputs_text =
                    raw.outputs_text && ext.map(|e| e.outputs_text).unwrap_or(true);

                if deprecated || raw.has_negative_price || !outputs_text || is_non_chat_id(&raw.id)
                {
                    return None;
                }

                let prompt_price = raw
                    .prompt_price_per_m
                    .or_else(|| ext.and_then(|e| e.prompt_price_per_m));
                let completion_price = raw
                    .completion_price_per_m
                    .or_else(|| ext.and_then(|e| e.completion_price_per_m));

                let is_free = is_verified_free(
                    &raw.id,
                    is_local_free,
                    prompt_price,
                    completion_price,
                    raw.request_price,
                );

                let (final_prompt, final_completion) = match is_free {
                    true => (Some(0.0), Some(0.0)),
                    false => (prompt_price, completion_price),
                };

                let source = match ext.is_some() {
                    true => "provider+registry",
                    false => "provider",
                };

                Some(ModelInfo {
                    name: raw
                        .name
                        .or_else(|| ext.map(|e| e.name.clone()))
                        .unwrap_or_else(|| raw.id.clone()),
                    id: raw.id,
                    context_length: raw
                        .context_length
                        .or_else(|| ext.and_then(|e| e.context_length)),
                    max_output_tokens: raw
                        .max_output_tokens
                        .or_else(|| ext.and_then(|e| e.max_output_tokens)),
                    prompt_price_per_m: final_prompt,
                    completion_price_per_m: final_completion,
                    is_free,
                    supports_tools: raw
                        .supports_tools
                        .unwrap_or_else(|| ext.map(|e| e.supports_tools).unwrap_or(false)),
                    supports_reasoning: raw
                        .supports_reasoning
                        .unwrap_or_else(|| ext.map(|e| e.supports_reasoning).unwrap_or(false)),
                    source: source.to_string(),
                })
            })
            .collect(),

        // Provider `/models` unavailable (e.g., MiniMax plan without `/v1/models` or key not yet entered):
        // hydrate directly from live `models.dev` active text-chat entries for this provider.
        (true, false) => external
            .into_iter()
            .filter(|e| !e.is_deprecated && e.outputs_text && !is_non_chat_id(&e.id))
            .map(|e| {
                let is_free = is_verified_free(
                    &e.id,
                    is_local_free,
                    e.prompt_price_per_m,
                    e.completion_price_per_m,
                    0.0,
                );
                ModelInfo {
                    id: e.id,
                    name: e.name,
                    context_length: e.context_length,
                    max_output_tokens: e.max_output_tokens,
                    prompt_price_per_m: e.prompt_price_per_m,
                    completion_price_per_m: e.completion_price_per_m,
                    is_free,
                    supports_tools: e.supports_tools,
                    supports_reasoning: e.supports_reasoning,
                    source: "registry".to_string(),
                }
            })
            .collect(),

        (true, true) => Vec::new(),
    };

    rank_models(&mut resolved);
    resolved
}

/// Strictly distinguishes verified free models from unknown-price models.
fn is_verified_free(
    id: &str,
    is_local_free: bool,
    prompt_price: Option<f64>,
    completion_price: Option<f64>,
    request_price: f64,
) -> bool {
    let explicit_zero = matches!(
        (prompt_price, completion_price),
        (Some(p), Some(c)) if p == 0.0 && c == 0.0 && request_price <= 0.0
    );
    is_local_free || id.to_ascii_lowercase().ends_with(":free") || explicit_zero
}

/// Deterministic multi-key ranking:
/// 1. FREE models first
/// 2. Tool-capable models first (essential for Zenko autonomous tool loop)
/// 3. Reasoning-capable models first
/// 4. Largest real max context window first
/// 5. Lowest total USD/1M cost first (for paid models)
/// 6. Lexicographical ID tie-breaker
pub fn rank_models(models: &mut [ModelInfo]) {
    models.sort_by(|a, b| {
        let primary = (
            !a.is_free,
            !a.supports_tools,
            !a.supports_reasoning,
            Reverse(a.context_length.unwrap_or(0)),
        )
            .cmp(&(
                !b.is_free,
                !b.supports_tools,
                !b.supports_reasoning,
                Reverse(b.context_length.unwrap_or(0)),
            ));

        match primary {
            Ordering::Equal => {
                let cost_a = total_cost_key(a);
                let cost_b = total_cost_key(b);
                cost_a
                    .total_cmp(&cost_b)
                    .then_with(|| a.id.cmp(&b.id))
            }
            other => other,
        }
    });
}

fn total_cost_key(m: &ModelInfo) -> f64 {
    match (m.prompt_price_per_m, m.completion_price_per_m) {
        (Some(p), Some(c)) => p + c,
        (Some(p), None) => p,
        (None, Some(c)) => c,
        (None, None) => f64::INFINITY,
    }
}

/// Full live discovery pipeline for any registered provider (zero hardcoded models).
pub async fn discover_provider_models(
    client: &Client,
    desc: &ProviderDescriptor,
    base_url: &str,
    api_key: Option<&str>,
    registry_json: Option<&Value>,
) -> Result<Vec<ModelInfo>> {
    let endpoint = format!("{}/models", base_url.trim_end_matches('/'));
    let mut req = client.get(&endpoint).timeout(Duration::from_secs(10));
    if let Some(k) = api_key
        && !k.trim().is_empty() {
            req = req.bearer_auth(k.trim());
        }

    let live_res = req.send().await;
    let live_models = match live_res {
        Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
            Ok(val) => parse_provider_models_json(&val, Utc::now()),
            Err(_) => Vec::new(),
        },
        _ => Vec::new(),
    };

    if desc.id == "ollama" && !live_models.is_empty() {
        return Ok(enrich_ollama_models(client, base_url, live_models).await);
    }

    let external_models = registry_json
        .map(|reg| extract_external_models(reg, desc.metadata_keys))
        .unwrap_or_default();

    let merged = merge_and_rank_models(live_models, external_models, desc);
    match merged.is_empty() {
        true => Err(ZenkoError::Provider(format!(
            "no live models returned by {} ({endpoint})",
            desc.label
        ))),
        false => Ok(merged),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenko_core::find_provider;

    #[test]
    fn filters_expired_non_chat_negative_sentinels_and_sorts_free_first() {
        let now = DateTime::parse_from_rfc3339("2026-11-20T12:00:00Z")
            .expect("now")
            .with_timezone(&Utc);

        let openrouter_payload = json!({
            "data": [
                {
                    "id": "openrouter/auto",
                    "name": "Auto Router",
                    "context_length": 2000000,
                    "pricing": { "prompt": "-1", "completion": "-1" },
                    "supported_parameters": ["tools"]
                },
                {
                    "id": "vendor/expired-free:free",
                    "name": "Old Expired Model",
                    "context_length": 128000,
                    "expiration_date": "2025-01-01",
                    "pricing": { "prompt": "0", "completion": "0" },
                    "supported_parameters": ["tools"]
                },
                {
                    "id": "vendor/flux-image",
                    "name": "Flux Image Gen",
                    "context_length": 4096,
                    "architecture": { "output_modalities": ["image"] },
                    "pricing": { "prompt": "0", "completion": "0" }
                },
                {
                    "id": "paid/frontier-model",
                    "name": "Frontier Paid",
                    "context_length": 1000000,
                    "top_provider": { "max_completion_tokens": 65536 },
                    "pricing": { "prompt": "0.000003", "completion": "0.000015" },
                    "supported_parameters": ["tools", "reasoning"]
                },
                {
                    "id": "free/chat-only:free",
                    "name": "Free Chat Only",
                    "context_length": 256000,
                    "pricing": { "prompt": "0", "completion": "0" },
                    "supported_parameters": ["temperature"]
                },
                {
                    "id": "free/agent-tools:free",
                    "name": "Free Agent Tools",
                    "context_length": 131072,
                    "pricing": { "prompt": "0", "completion": "0" },
                    "supported_parameters": ["tools", "reasoning"]
                }
            ]
        });

        let raw = parse_provider_models_json(&openrouter_payload, now);
        let ranked = merge_and_rank_models(raw, Vec::new(), find_provider("openrouter"));

        assert_eq!(ranked.len(), 3);
        // Free + tools + reasoning must be #1!
        assert_eq!(ranked[0].id, "free/agent-tools:free");
        assert!(ranked[0].is_free);
        assert!(ranked[0].supports_tools);
        assert!(ranked[0].supports_reasoning);
        assert_eq!(ranked[0].context_length, Some(131072));
        // Other free model #2!
        assert_eq!(ranked[1].id, "free/chat-only:free");
        assert!(ranked[1].is_free);
        // Paid model #3 with exact USD/1M pricing & max output tokens!
        assert_eq!(ranked[2].id, "paid/frontier-model");
        assert!(!ranked[2].is_free);
        assert_eq!(ranked[2].context_length, Some(1_000_000));
        assert_eq!(ranked[2].max_output_tokens, Some(65536));
        assert!((ranked[2].prompt_price_per_m.expect("prompt") - 3.0).abs() < 1e-6);
    }

    #[test]
    fn hydrates_minimax_from_models_dev_and_filters_speech_or_deprecated() {
        let models_dev = json!({
            "minimax": {
                "models": {
                    "MiniMax-M3.1-Flash": {
                        "name": "MiniMax M3.1 Flash",
                        "status": "active",
                        "limit": { "context": 1000000, "output": 131072 },
                        "cost": { "input": 0.30, "output": 1.20 },
                        "tool_call": true,
                        "reasoning": true,
                        "modalities": { "input": ["text"], "output": ["text"] }
                    },
                    "MiniMax-Speech-2.8": {
                        "name": "MiniMax Speech",
                        "status": "active",
                        "modalities": { "input": ["text"], "output": ["audio"] }
                    },
                    "MiniMax-Old": {
                        "name": "Old Deprecated",
                        "status": "deprecated",
                        "limit": { "context": 32000 },
                        "modalities": { "input": ["text"], "output": ["text"] }
                    }
                }
            }
        });

        let desc = find_provider("minimax");
        let ext = extract_external_models(&models_dev, desc.metadata_keys);

        // Case A: provider `/models` returns bare ID without context/pricing -> hydrated from models.dev!
        let live_bare = parse_provider_models_json(
            &json!({"data": [{"id": "MiniMax-M3.1-Flash"}]}),
            Utc::now(),
        );
        let merged = merge_and_rank_models(live_bare, ext.clone(), desc);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "MiniMax-M3.1-Flash");
        assert_eq!(merged[0].context_length, Some(1_000_000));
        assert_eq!(merged[0].max_output_tokens, Some(131_072));
        assert_eq!(merged[0].prompt_price_per_m, Some(0.30));
        assert!(merged[0].supports_tools);
        assert!(merged[0].supports_reasoning);
        assert_eq!(merged[0].source, "provider+registry");

        // Case B: provider `/models` unavailable -> falls back cleanly to live models.dev entries!
        let from_reg = merge_and_rank_models(Vec::new(), ext, desc);
        assert_eq!(from_reg.len(), 1);
        assert_eq!(from_reg[0].id, "MiniMax-M3.1-Flash");
        assert_eq!(from_reg[0].source, "registry");
    }

    #[test]
    fn parses_ollama_show_context_length_without_regex() {
        let show = json!({
            "model_info": {
                "general.architecture": "qwen2",
                "qwen2.context_length": 131072
            },
            "capabilities": ["completion", "tools", "thinking"]
        });
        assert_eq!(parse_ollama_show_json(&show), (Some(131072), true, true));
    }
}
