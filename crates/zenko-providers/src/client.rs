//! Unified multi-provider HTTP client supporting OpenRouter, MiniMax, OpenAI, DeepSeek,
//! Gemini, Ollama, and custom OpenAI-compatible endpoints with live model discovery.

use crate::dialect::{apply_chat_headers, build_request_body, resolve_dialect, verify_endpoint};
use crate::discovery::{discover_provider_models, fetch_metadata_registry};
use crate::sse::{
    SseAccumulator, SseByteLineDecoder, check_provider_error, emit_non_streaming_response,
    process_sse_line,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;
use zenko_core::{
    ChatRequest, DEFAULT_PROVIDER_ID, DeltaStream, LlmDelta, LlmProvider, MINIMAX_TEMP_MAX,
    MODEL_CACHE_TTL_SECS, ModelInfo, ProviderDialect, Result, ZenkoError, find_provider,
};

#[derive(Clone)]
struct ProviderState {
    provider_id: String,
    base_url: String,
    api_key: Option<Zeroizing<String>>,
}

#[derive(Clone)]
pub struct UnifiedLlmProvider {
    client: Client,
    state: Arc<RwLock<ProviderState>>,
    metadata_cache: Arc<RwLock<Option<(Instant, Value)>>>,
}

impl UnifiedLlmProvider {
    pub fn for_provider(
        client: Client,
        provider_id: impl Into<String>,
        base_url: impl Into<String>,
        api_key: Option<Zeroizing<String>>,
    ) -> Self {
        Self {
            client,
            state: Arc::new(RwLock::new(ProviderState {
                provider_id: provider_id.into(),
                base_url: base_url.into(),
                api_key,
            })),
            metadata_cache: Arc::new(RwLock::new(None)),
        }
    }

    pub fn new(
        client: Client,
        base_url: impl Into<String>,
        api_key: Option<Zeroizing<String>>,
    ) -> Self {
        let url_str = base_url.into();
        let inferred_id = match resolve_dialect("custom", &url_str) {
            ProviderDialect::MiniMax => "minimax",
            ProviderDialect::OpenRouter => "openrouter",
            ProviderDialect::OpenAiCompat => "custom",
        };
        Self::for_provider(client, inferred_id, url_str, api_key)
    }

    pub fn seed_metadata_registry(&self, val: Value) {
        if let Ok(mut guard) = self.metadata_cache.write() {
            *guard = Some((Instant::now(), val));
        }
    }

    pub fn update_provider(
        &self,
        provider_id: Option<String>,
        base_url: Option<String>,
        api_key: Option<Zeroizing<String>>,
    ) {
        if let Ok(mut guard) = self.state.write() {
            if let Some(pid) = provider_id {
                guard.provider_id = pid;
            }
            if let Some(url) = base_url {
                guard.base_url = url;
            }
            guard.api_key = api_key;
        }
    }

    pub fn update_credentials(&self, base_url: Option<String>, api_key: Option<Zeroizing<String>>) {
        self.update_provider(None, base_url, api_key);
    }

    pub fn current_kind(&self) -> String {
        self.state
            .read()
            .map(|g| g.provider_id.clone())
            .unwrap_or_else(|_| DEFAULT_PROVIDER_ID.to_string())
    }

    fn snapshot_state(&self) -> (String, String, ProviderDialect, Option<Zeroizing<String>>) {
        let state = self
            .state
            .read()
            .map(|g| g.clone())
            .unwrap_or_else(|_| ProviderState {
                provider_id: DEFAULT_PROVIDER_ID.to_string(),
                base_url: String::new(),
                api_key: None,
            });
        let default_url = find_provider(&state.provider_id).default_base_url;
        let trimmed = state.base_url.trim_end_matches('/').to_string();
        let base_url = match trimmed.is_empty() {
            true => default_url.to_string(),
            false => trimmed,
        };
        let dialect = resolve_dialect(&state.provider_id, &base_url);
        (state.provider_id, base_url, dialect, state.api_key)
    }

    /// Fetches or reuses the live `models.dev/api.json` registry snapshot.
    pub async fn load_metadata_registry(&self, force_refresh: bool) -> Option<Value> {
        if !force_refresh {
            if let Ok(guard) = self.metadata_cache.read() {
                if let Some((ts, val)) = guard.as_ref() {
                    if ts.elapsed() < Duration::from_secs(MODEL_CACHE_TTL_SECS) {
                        return Some(val.clone());
                    }
                }
            }
        }
        if let Some(fresh) = fetch_metadata_registry(&self.client).await {
            if let Ok(mut guard) = self.metadata_cache.write() {
                *guard = Some((Instant::now(), fresh.clone()));
            }
            return Some(fresh);
        }
        self.metadata_cache
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|(_, v)| v.clone()))
    }

    /// Discovers live working models for ANY provider (free models sorted first) without
    /// mutating the active provider state.
    pub async fn discover_models_for(
        &self,
        provider_id: &str,
        base_url: Option<&str>,
        api_key: Option<&str>,
        force_refresh: bool,
    ) -> Result<Vec<ModelInfo>> {
        let desc = find_provider(provider_id);
        let resolved_url = base_url
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .unwrap_or(desc.default_base_url);
        let registry = match desc.metadata_keys.is_empty() {
            true => None,
            false => self.load_metadata_registry(force_refresh).await,
        };
        discover_provider_models(&self.client, desc, resolved_url, api_key, registry.as_ref()).await
    }

    /// MiniMax fallback verification when `/models` returns 404/405: discovers a real live
    /// MiniMax model ID via `models.dev` (zero hardcoding!) and sends a minimal 1-token auth probe.
    async fn verify_minimax_completion_probe(&self, base: &str, key: &str) -> Result<()> {
        let discovered = self
            .discover_models_for(&self.current_kind(), Some(base), Some(key), false)
            .await?;
        let probe_model = discovered
            .first()
            .map(|m| m.id.as_str())
            .ok_or_else(|| ZenkoError::Provider("no MiniMax models discovered for probe".into()))?;

        let endpoint = format!("{base}/chat/completions");
        let probe_body = json!({
            "model": probe_model,
            "messages": [{"role": "user", "content": "ping"}],
            "max_tokens": 1,
            "temperature": MINIMAX_TEMP_MAX,
            "stream": false
        });
        let resp = self
            .client
            .post(&endpoint)
            .bearer_auth(key)
            .json(&probe_body)
            .send()
            .await
            .map_err(|e| ZenkoError::Provider(format!("MiniMax probe failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = read_bounded_error_text(resp, None).await;
            return Err(ZenkoError::Provider(format!(
                "MiniMax verification failed ({status}): {}",
                truncate(&text, 240)
            )));
        }

        let val = resp.json::<Value>().await.map_err(|e| {
            ZenkoError::Provider(format!("MiniMax probe returned non-JSON response: {e}"))
        })?;
        check_provider_error(&val)?;
        Ok(())
    }
}

#[async_trait]
impl LlmProvider for UnifiedLlmProvider {
    fn id(&self) -> &str {
        "unified"
    }

    async fn verify_key(&self) -> Result<()> {
        let (kind, base, dialect, key) = self.snapshot_state();
        let desc = find_provider(&kind);

        let key_missing = key.as_deref().map(|s| s.trim().is_empty()).unwrap_or(true);
        if desc.requires_api_key && key_missing {
            return Err(ZenkoError::Provider(format!(
                "{} API key is not configured",
                desc.label
            )));
        }

        let endpoint = verify_endpoint(dialect, &base);
        let mut req = self.client.get(&endpoint);
        if let Some(ref k) = key {
            if !k.trim().is_empty() {
                req = req.bearer_auth(k.as_str());
            }
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ZenkoError::Provider(format!("connection failed: {e}")))?;

        if resp.status().is_success() {
            let val = resp.json::<Value>().await.map_err(|e| {
                ZenkoError::Provider(format!(
                    "{} verification returned invalid JSON (possible proxy/HTML page): {e}",
                    desc.label
                ))
            })?;
            check_provider_error(&val)?;
            return Ok(());
        }

        if dialect == ProviderDialect::MiniMax
            && matches!(resp.status(), StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED)
        {
            if let Some(ref k) = key {
                return self.verify_minimax_completion_probe(&base, k.as_str()).await;
            }
        }

        let status = resp.status();
        let text = read_bounded_error_text(resp, None).await;
        Err(ZenkoError::Provider(format!(
            "{} verification failed ({status}): {}",
            desc.label,
            truncate(&text, 240)
        )))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let (kind, base, _, key) = self.snapshot_state();
        self.discover_models_for(&kind, Some(&base), key.as_deref().map(|z| z.as_str()), false)
            .await
    }

    async fn stream_chat(&self, req: ChatRequest, cancel: CancellationToken) -> Result<DeltaStream> {
        let (kind, base, dialect, key) = self.snapshot_state();
        let desc = find_provider(&kind);

        let key_missing = key.as_deref().map(|k| k.trim().is_empty()).unwrap_or(true);
        if desc.requires_api_key && key_missing {
            return Err(ZenkoError::Provider(format!(
                "{} API key not configured. Run `zenko setup` or open Settings in WebUI.",
                desc.label
            )));
        }

        let endpoint = format!("{base}/chat/completions");
        let body = build_request_body(dialect, &req);
        let accept = match req.enable_streaming {
            true => "text/event-stream",
            false => "application/json",
        };

        let mut http_req = self
            .client
            .post(&endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", accept);

        if let Some(k) = key {
            if !k.trim().is_empty() {
                http_req = http_req.bearer_auth(k.as_str());
            }
        }
        http_req = apply_chat_headers(dialect, http_req, &req);

        let resp = tokio::select! {
            _ = cancel.cancelled() => return Err(ZenkoError::Cancelled),
            res = http_req.json(&body).send() => {
                res.map_err(|e| ZenkoError::Provider(format!("HTTP request failed: {e}")))?
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = read_bounded_error_text(resp, Some(&cancel)).await;
            if cancel.is_cancelled() {
                return Err(ZenkoError::Cancelled);
            }
            return Err(ZenkoError::Provider(format!(
                "LLM API error ({status}): {}",
                truncate(&err_body, 400)
            )));
        }

        let (tx, rx) = mpsc::channel::<Result<LlmDelta>>(64);
        let is_streaming = req.enable_streaming;

        tokio::spawn(async move {
            if !is_streaming {
                let parsed = tokio::select! {
                    _ = cancel.cancelled() => {
                        let _ = tx.send(Err(ZenkoError::Cancelled)).await;
                        return;
                    }
                    res = resp.json::<Value>() => res,
                };
                match parsed {
                    Ok(val) => {
                        if let Err(e) = emit_non_streaming_response(&val, &tx).await {
                            let _ = tx.send(Err(e)).await;
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(ZenkoError::Provider(format!(
                                "invalid completion json: {e}"
                            ))))
                            .await;
                    }
                }
                return;
            }

            let mut byte_stream = resp.bytes_stream();
            let mut decoder = SseByteLineDecoder::with_capacity(1024);
            let mut acc = SseAccumulator::default();

            loop {
                let next_chunk = tokio::select! {
                    _ = cancel.cancelled() => {
                        let _ = tx.send(Err(ZenkoError::Cancelled)).await;
                        return;
                    }
                    item = byte_stream.next() => item,
                };

                match next_chunk {
                    Some(Ok(bytes)) => {
                        let lines = match decoder.push(&bytes) {
                            Ok(l) => l,
                            Err(e) => {
                                let _ = tx.send(Err(e)).await;
                                return;
                            }
                        };
                        for line in lines {
                            match process_sse_line(&line, &mut acc, &tx).await {
                                Ok(true) => {
                                    let _ = tx
                                        .send(Ok(LlmDelta::Done {
                                            finish_reason: acc.finish_reason.take(),
                                        }))
                                        .await;
                                    return;
                                }
                                Ok(false) => {}
                                Err(e) => {
                                    let _ = tx.send(Err(e)).await;
                                    return;
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        let _ = tx
                            .send(Err(ZenkoError::Provider(format!("stream read error: {e}"))))
                            .await;
                        return;
                    }
                    None => {
                        if let Some(tail) = decoder.finish() {
                            if let Err(e) = process_sse_line(&tail, &mut acc, &tx).await {
                                let _ = tx.send(Err(e)).await;
                                return;
                            }
                        }
                        acc.flush_all(&tx).await;
                        let _ = tx
                            .send(Ok(LlmDelta::Done {
                                finish_reason: acc.finish_reason.take(),
                            }))
                            .await;
                        return;
                    }
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

const MAX_ERROR_BODY_BYTES: usize = 16 * 1024;

async fn read_bounded_error_text(
    mut resp: reqwest::Response,
    cancel: Option<&CancellationToken>,
) -> String {
    let mut buf = Vec::with_capacity(1024);
    loop {
        let next = match cancel {
            Some(c) => tokio::select! {
                _ = c.cancelled() => break,
                chunk = resp.chunk() => chunk,
            },
            None => resp.chunk().await,
        };
        match next {
            Ok(Some(bytes)) => {
                let remaining = MAX_ERROR_BODY_BYTES.saturating_sub(buf.len());
                if remaining == 0 {
                    break;
                }
                let take = bytes.len().min(remaining);
                buf.extend_from_slice(&bytes[..take]);
                if take < bytes.len() {
                    break;
                }
            }
            _ => break,
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let cut = s.floor_char_boundary(max);
        format!("{}...", &s[..cut])
    }
}
