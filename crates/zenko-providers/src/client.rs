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
pub struct UnifiedLlmProvider {
    client: Client,
    provider_id: Arc<RwLock<String>>,
    base_url: Arc<RwLock<String>>,
    api_key: Arc<RwLock<Option<Zeroizing<String>>>>,
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
            provider_id: Arc::new(RwLock::new(provider_id.into())),
            base_url: Arc::new(RwLock::new(base_url.into())),
            api_key: Arc::new(RwLock::new(api_key)),
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
            ProviderDialect::OpenAiCompat => DEFAULT_PROVIDER_ID,
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
        if let Some(pid) = provider_id {
            if let Ok(mut guard) = self.provider_id.write() {
                *guard = pid;
            }
        }
        if let Some(url) = base_url {
            if let Ok(mut guard) = self.base_url.write() {
                *guard = url;
            }
        }
        if let Ok(mut guard) = self.api_key.write() {
            *guard = api_key;
        }
    }

    pub fn update_credentials(&self, base_url: Option<String>, api_key: Option<Zeroizing<String>>) {
        self.update_provider(None, base_url, api_key);
    }

    pub fn current_kind(&self) -> String {
        self.provider_id
            .read()
            .map(|g| g.clone())
            .unwrap_or_else(|_| DEFAULT_PROVIDER_ID.to_string())
    }

    fn current_base_url(&self) -> String {
        let kind = self.current_kind();
        let default_url = find_provider(&kind).default_base_url;
        self.base_url
            .read()
            .ok()
            .map(|g| g.trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| default_url.to_string())
    }

    fn current_api_key(&self) -> Option<Zeroizing<String>> {
        self.api_key.read().ok().and_then(|g| g.clone())
    }

    fn current_dialect(&self, base_url: &str) -> ProviderDialect {
        resolve_dialect(&self.current_kind(), base_url)
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
            let text = resp.text().await.unwrap_or_default();
            return Err(ZenkoError::Provider(format!(
                "MiniMax verification failed ({status}): {}",
                truncate(&text, 240)
            )));
        }

        if let Ok(val) = resp.json::<Value>().await {
            check_provider_error(&val)?;
        }
        Ok(())
    }
}

#[async_trait]
impl LlmProvider for UnifiedLlmProvider {
    fn id(&self) -> &str {
        "unified"
    }

    async fn verify_key(&self) -> Result<()> {
        let kind = self.current_kind();
        let desc = find_provider(&kind);
        let base = self.current_base_url();
        let dialect = self.current_dialect(&base);
        let key = self.current_api_key();

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
            if let Ok(val) = resp.json::<Value>().await {
                check_provider_error(&val)?;
            }
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
        let text = resp.text().await.unwrap_or_default();
        Err(ZenkoError::Provider(format!(
            "{} verification failed ({status}): {}",
            desc.label,
            truncate(&text, 240)
        )))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let kind = self.current_kind();
        let base = self.current_base_url();
        let key = self.current_api_key();
        self.discover_models_for(&kind, Some(&base), key.as_deref().map(|z| z.as_str()), false)
            .await
    }

    async fn stream_chat(&self, req: ChatRequest, cancel: CancellationToken) -> Result<DeltaStream> {
        let kind = self.current_kind();
        let desc = find_provider(&kind);
        let base = self.current_base_url();
        let dialect = self.current_dialect(&base);
        let key = self.current_api_key();

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
            let err_body = resp.text().await.unwrap_or_default();
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
                        for line in decoder.push(&bytes) {
                            if let Err(e) = process_sse_line(&line, &mut acc, &tx).await {
                                let _ = tx.send(Err(e)).await;
                                return;
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

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let cut = s.floor_char_boundary(max);
        format!("{}...", &s[..cut])
    }
}
