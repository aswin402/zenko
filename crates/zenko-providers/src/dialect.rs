//! Declarative wire-dialect dispatch for OpenRouter, MiniMax, and OpenAI-compatible APIs.
//! Pure `match` expressions over `ProviderDialect` — zero `if/else` ladders, zero hardcoded URLs.

use reqwest::RequestBuilder;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use zenko_core::{
    ChatRequest, EventRole, MINIMAX_TEMP_MAX, MINIMAX_TEMP_MIN, PROVIDER_HTTP_REFERER,
    PROVIDER_X_TITLE, ProviderDialect, ToolSpec, find_provider,
};

/// Recursively canonicalizes JSON object keys in deterministic lexicographical order
/// (`CacheAligner`) so tool schemas (including dynamically discovered MCP schemas)
/// produce byte-identical wire payloads across turns and restarts for provider KV caches.
pub fn canonicalize_json_value(val: &Value) -> Value {
    match val {
        Value::Object(map) => {
            let sorted: BTreeMap<&str, Value> = map
                .iter()
                .map(|(k, v)| (k.as_str(), canonicalize_json_value(v)))
                .collect();
            let mut out = serde_json::Map::with_capacity(sorted.len());
            for (k, v) in sorted {
                out.insert(k.to_string(), v);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json_value).collect()),
        other => other.clone(),
    }
}

/// Resolves wire dialect from explicit provider ID, falling back to host inspection for `custom`.
pub fn resolve_dialect(provider_id: &str, base_url: &str) -> ProviderDialect {
    let desc = find_provider(provider_id);
    match desc.dialect {
        ProviderDialect::OpenAiCompat if desc.id == "custom" => {
            let lower = base_url.to_ascii_lowercase();
            match (lower.contains("openrouter.ai"), lower.contains("minimax")) {
                (true, _) => ProviderDialect::OpenRouter,
                (_, true) => ProviderDialect::MiniMax,
                _ => ProviderDialect::OpenAiCompat,
            }
        }
        explicit => explicit,
    }
}

/// Builds the JSON payload for `/chat/completions` according to the active provider dialect.
pub fn build_request_body(dialect: ProviderDialect, req: &ChatRequest) -> Value {
    let mut messages = Vec::with_capacity(req.messages.len() + 1);

    if !req.system_prompt.trim().is_empty() {
        let sys_msg = match (dialect, req.enable_prompt_caching) {
            (ProviderDialect::OpenRouter, true) => json!({
                "role": "system",
                "content": [{
                    "type": "text",
                    "text": req.system_prompt,
                    "cache_control": { "type": "ephemeral" }
                }]
            }),
            _ => json!({
                "role": "system",
                "content": req.system_prompt
            }),
        };
        messages.push(sys_msg);
    }

    for msg in &req.messages {
        match msg.role {
            EventRole::System => {
                messages.push(json!({ "role": "system", "content": msg.content }));
            }
            EventRole::User => {
                messages.push(json!({ "role": "user", "content": msg.content }));
            }
            EventRole::Assistant => {
                let mut obj = serde_json::Map::new();
                obj.insert("role".into(), Value::String("assistant".into()));

                match msg.tool_calls.is_empty() {
                    true => {
                        obj.insert("content".into(), Value::String(msg.content.clone()));
                    }
                    false => {
                        let calls: Vec<Value> = msg
                            .tool_calls
                            .iter()
                            .map(|tc| {
                                json!({
                                    "id": tc.id,
                                    "type": "function",
                                    "function": {
                                        "name": tc.name,
                                        "arguments": canonicalize_json_value(&tc.arguments).to_string()
                                    }
                                })
                            })
                            .collect();
                        let content_val = match msg.content.is_empty() {
                            true => Value::Null,
                            false => Value::String(msg.content.clone()),
                        };
                        obj.insert("content".into(), content_val);
                        obj.insert("tool_calls".into(), Value::Array(calls));
                    }
                }

                // Preserve interleaved reasoning state across multi-turn tool calls
                // (required by MiniMax M3/M2.7/M2.5 and DeepSeek reasoner).
                if let Some(ref rc) = msg.reasoning_content
                    && !rc.trim().is_empty() {
                        obj.insert("reasoning_content".into(), Value::String(rc.clone()));
                    }

                messages.push(Value::Object(obj));
            }
            EventRole::Tool => {
                let mut obj = serde_json::Map::new();
                obj.insert("role".into(), Value::String("tool".into()));
                obj.insert(
                    "tool_call_id".into(),
                    Value::String(msg.tool_call_id.clone().unwrap_or_default()),
                );
                if let Some(ref tool_name) = msg.name
                    && !tool_name.trim().is_empty() {
                        obj.insert("name".into(), Value::String(tool_name.clone()));
                    }
                obj.insert("content".into(), Value::String(msg.content.clone()));
                messages.push(Value::Object(obj));
            }
        }
    }

    let temperature = match dialect {
        ProviderDialect::MiniMax => req.temperature.clamp(MINIMAX_TEMP_MIN, MINIMAX_TEMP_MAX),
        _ => req.temperature,
    };

    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": req.enable_streaming,
        "temperature": temperature,
        "max_tokens": req.max_tokens,
    });

    if let Some(obj) = body.as_object_mut() {
        if req.enable_streaming {
            obj.insert("stream_options".into(), json!({ "include_usage": true }));
        }

        if !req.tools.is_empty() {
            // CacheAligner: sort tool specs deterministically by name and canonicalize every
            // parameter schema's JSON keys so wire payloads stay byte-identical for KV caching.
            let mut sorted_tools: Vec<&ToolSpec> = req.tools.iter().collect();
            sorted_tools.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.description.cmp(&b.description)));
            let last_tool_idx = sorted_tools.len().saturating_sub(1);
            let attach_tool_cache =
                dialect == ProviderDialect::OpenRouter && req.enable_prompt_caching;
            let tools: Vec<Value> = sorted_tools
                .into_iter()
                .enumerate()
                .map(|(idx, t)| {
                    let mut tool_val = json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": canonicalize_json_value(&t.parameters)
                        }
                    });
                    if attach_tool_cache && idx == last_tool_idx
                        && let Some(m) = tool_val.as_object_mut() {
                            m.insert(
                                "cache_control".into(),
                                json!({ "type": "ephemeral" }),
                            );
                        }
                    tool_val
                })
                .collect();
            obj.insert("tools".into(), Value::Array(tools));
            obj.insert("tool_choice".into(), Value::String("auto".into()));
        }

        // OpenRouter/Anthropic multi-breakpoint KV caching: place a 3rd cache breakpoint on the
        // penultimate stable user message (avoid converting `role: "tool"` content into array blocks
        // which non-Anthropic upstreams behind OpenRouter can reject with HTTP 400).
        if dialect == ProviderDialect::OpenRouter && req.enable_prompt_caching
            && let Some(arr) = obj.get_mut("messages").and_then(Value::as_array_mut)
                && arr.len() >= 4 {
                    let target_idx = arr
                        .iter()
                        .enumerate()
                        .take(arr.len() - 1)
                        .skip(1)
                        .rev()
                        .find(|(_, m)| m.get("role").and_then(Value::as_str) == Some("user"))
                        .map(|(i, _)| i);
                    if let Some(idx) = target_idx
                        && let Some(mobj) = arr[idx].as_object_mut()
                            && let Some(text) =
                                mobj.get("content").and_then(Value::as_str).map(String::from)
                            {
                                mobj.insert(
                                    "content".into(),
                                    json!([{
                                        "type": "text",
                                        "text": text,
                                        "cache_control": { "type": "ephemeral" }
                                    }]),
                                );
                            }
                }

        match dialect {
            ProviderDialect::OpenRouter => {
                obj.insert("usage".into(), json!({ "include": true }));
                if req.sticky_session_routing && !req.session_id.is_empty() {
                    obj.insert("session_id".into(), Value::String(req.session_id.clone()));
                }
                if !req.fallbacks.is_empty() {
                    let mut models = Vec::with_capacity(req.fallbacks.len() + 1);
                    models.push(Value::String(req.model.clone()));
                    for fb in &req.fallbacks {
                        if fb != &req.model {
                            models.push(Value::String(fb.clone()));
                        }
                    }
                    obj.insert("models".into(), Value::Array(models));
                }
                if !req.provider_sort.is_empty() {
                    obj.insert(
                        "provider".into(),
                        json!({
                            "sort": req.provider_sort,
                            "allow_fallbacks": true
                        }),
                    );
                }
                if let Some(effort) = req.reasoning_effort {
                    obj.insert("reasoning".into(), json!({ "effort": effort.as_str() }));
                }
            }
            ProviderDialect::MiniMax => {
                // Separate thinking into dedicated reasoning_content / reasoning_details fields.
                obj.insert("reasoning_split".into(), Value::Bool(true));
            }
            ProviderDialect::OpenAiCompat => {
                // LMCache / vLLM / SGLang / OpenAI sticky prefix-routing key anchored on (SystemBase, ToolCatalog).
                if req.enable_prompt_caching
                    && let Some(ref key) = req.prompt_cache_key
                    && !key.trim().is_empty()
                {
                    obj.insert("prompt_cache_key".into(), Value::String(key.clone()));
                }
            }
        }
    }

    body
}

/// Applies provider-specific HTTP headers onto a request builder.
pub fn apply_chat_headers(
    dialect: ProviderDialect,
    req_builder: RequestBuilder,
    chat_req: &ChatRequest,
) -> RequestBuilder {
    match dialect {
        ProviderDialect::OpenRouter => {
            let mut b = req_builder
                .header("HTTP-Referer", PROVIDER_HTTP_REFERER)
                .header("X-Title", PROVIDER_X_TITLE);
            if chat_req.sticky_session_routing && !chat_req.session_id.is_empty() {
                b = b.header("x-session-id", &chat_req.session_id);
            }
            b
        }
        ProviderDialect::MiniMax | ProviderDialect::OpenAiCompat => req_builder,
    }
}

/// Primary endpoint used to verify API key connectivity for a given dialect.
pub fn verify_endpoint(dialect: ProviderDialect, base_url: &str) -> String {
    match dialect {
        ProviderDialect::OpenRouter => format!("{base_url}/key"),
        ProviderDialect::MiniMax | ProviderDialect::OpenAiCompat => format!("{base_url}/models"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenko_core::{ChatMessage, ToolCall};

    fn sample_request(temp: f32) -> ChatRequest {
        ChatRequest {
            session_id: "sess_1".into(),
            model: "MiniMax-M3.1-Flash".into(),
            fallbacks: vec!["MiniMax-M2.7".into()],
            system_prompt: "You are Zenko.".into(),
            messages: vec![
                ChatMessage {
                    role: EventRole::User,
                    content: "check files".into(),
                    reasoning_content: None,
                    tool_calls: vec![],
                    tool_call_id: None,
                    name: None,
                },
                ChatMessage {
                    role: EventRole::Assistant,
                    content: String::new(),
                    reasoning_content: Some("Need to read README first".into()),
                    tool_calls: vec![ToolCall {
                        id: "call_1".into(),
                        name: "read_file".into(),
                        arguments: json!({"path": "README.md"}),
                    }],
                    tool_call_id: None,
                    name: None,
                },
            ],
            tools: vec![],
            temperature: temp,
            max_tokens: 1024,
            enable_streaming: true,
            enable_prompt_caching: true,
            sticky_session_routing: true,
            provider_sort: "price".into(),
            reasoning_effort: Some(zenko_core::ReasoningEffort::High),
            prompt_cache_key: None,
        }
    }

    #[test]
    fn openai_compat_dialect_emits_prompt_cache_key_when_caching_enabled() {
        let mut req = sample_request(0.2);
        req.prompt_cache_key = Some("zk-pfx-0123456789abcdef01234567".into());
        let body = build_request_body(ProviderDialect::OpenAiCompat, &req);
        assert_eq!(
            body["prompt_cache_key"],
            Value::String("zk-pfx-0123456789abcdef01234567".into())
        );

        req.enable_prompt_caching = false;
        let disabled = build_request_body(ProviderDialect::OpenAiCompat, &req);
        assert!(disabled.get("prompt_cache_key").is_none());
    }

    #[test]
    fn minimax_dialect_sets_reasoning_split_clamps_temp_and_keeps_thinking() {
        let body = build_request_body(ProviderDialect::MiniMax, &sample_request(0.0));
        assert_eq!(body["reasoning_split"], Value::Bool(true));
        let temp = body["temperature"].as_f64().expect("temp");
        assert!((temp - MINIMAX_TEMP_MIN as f64).abs() < 1e-4);
        assert!(body.get("provider").is_none());
        assert!(body.get("models").is_none());
        assert_eq!(
            body["messages"][2]["reasoning_content"],
            Value::String("Need to read README first".into())
        );

        let high = build_request_body(ProviderDialect::MiniMax, &sample_request(1.9));
        let high_temp = high["temperature"].as_f64().expect("high temp");
        assert!((high_temp - MINIMAX_TEMP_MAX as f64).abs() < 1e-4);
    }

    #[test]
    fn openrouter_dialect_adds_cache_control_fallbacks_and_routing() {
        let mut req = sample_request(0.2);
        req.tools.push(zenko_core::ToolSpec {
            name: "read_file".into(),
            description: "Read file".into(),
            parameters: json!({"type": "object"}),
            risk: zenko_core::RiskLevel::Safe,
        });
        req.messages.push(ChatMessage {
            role: EventRole::Tool,
            content: "README contents".into(),
            reasoning_content: None,
            tool_calls: vec![],
            tool_call_id: Some("call_1".into()),
            name: Some("read_file".into()),
        });
        req.messages.push(ChatMessage {
            role: EventRole::User,
            content: "next step".into(),
            reasoning_content: None,
            tool_calls: vec![],
            tool_call_id: None,
            name: None,
        });
        let body = build_request_body(ProviderDialect::OpenRouter, &req);
        assert!(body["messages"][0]["content"].is_array());
        assert_eq!(body["tools"][0]["cache_control"]["type"], "ephemeral");
        // Tool messages must remain string content (never array blocks) and preserve `name`
        assert!(body["messages"][3]["content"].is_string());
        assert_eq!(body["messages"][3]["name"], Value::String("read_file".into()));
        // Penultimate user message receives the 3rd KV cache breakpoint
        assert!(body["messages"][1]["content"].is_array());
        assert_eq!(
            body["messages"][1]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        assert_eq!(body["session_id"], Value::String("sess_1".into()));
        assert_eq!(body["models"][1], Value::String("MiniMax-M2.7".into()));
        assert_eq!(body["provider"]["sort"], Value::String("price".into()));
        assert_eq!(body["reasoning"]["effort"], Value::String("high".into()));
        assert!(body.get("reasoning_split").is_none());
    }

    #[test]
    fn resolves_custom_minimax_host_dialect() {
        assert_eq!(
            resolve_dialect("minimax", "https://api.minimax.io/v1"),
            ProviderDialect::MiniMax
        );
        assert_eq!(
            resolve_dialect("custom", "https://api.minimaxi.com/v1"),
            ProviderDialect::MiniMax
        );
        assert_eq!(
            resolve_dialect("openrouter", "https://openrouter.ai/api/v1"),
            ProviderDialect::OpenRouter
        );
    }

    #[test]
    fn cache_aligner_sorts_tools_and_canonicalizes_nested_schema_keys() {
        let mut req_a = sample_request(0.2);
        req_a.tools = vec![
            ToolSpec {
                name: "zeta_tool".into(),
                description: "Zeta".into(),
                parameters: json!({
                    "type": "object",
                    "required": ["b", "a"],
                    "properties": {
                        "z": { "type": "string", "description": "last" },
                        "a": { "type": "integer", "minimum": 1 }
                    }
                }),
                risk: zenko_core::RiskLevel::Safe,
            },
            ToolSpec {
                name: "alpha_tool".into(),
                description: "Alpha".into(),
                parameters: json!({
                    "properties": { "x": { "type": "boolean" } },
                    "type": "object"
                }),
                risk: zenko_core::RiskLevel::Safe,
            },
        ];

        let mut req_b = sample_request(0.2);
        req_b.tools = vec![
            ToolSpec {
                name: "alpha_tool".into(),
                description: "Alpha".into(),
                parameters: json!({
                    "type": "object",
                    "properties": { "x": { "type": "boolean" } }
                }),
                risk: zenko_core::RiskLevel::Safe,
            },
            ToolSpec {
                name: "zeta_tool".into(),
                description: "Zeta".into(),
                parameters: json!({
                    "properties": {
                        "a": { "minimum": 1, "type": "integer" },
                        "z": { "description": "last", "type": "string" }
                    },
                    "required": ["b", "a"],
                    "type": "object"
                }),
                risk: zenko_core::RiskLevel::Safe,
            },
        ];

        let body_a = build_request_body(ProviderDialect::OpenRouter, &req_a);
        let body_b = build_request_body(ProviderDialect::OpenRouter, &req_b);
        assert_eq!(
            serde_json::to_string(&body_a["tools"]).expect("ser a"),
            serde_json::to_string(&body_b["tools"]).expect("ser b"),
        );
        assert_eq!(body_a["tools"][0]["function"]["name"], "alpha_tool");
        assert_eq!(body_a["tools"][1]["function"]["name"], "zeta_tool");
        assert_eq!(body_a["tools"][1]["cache_control"]["type"], "ephemeral");
    }
}
