use super::*;

// ─── Phase 7: External MCP Client Servers, Pooled LSP & Pluggable Web Search ─

pub async fn api_list_mcp_servers(State(state): State<WebState>) -> Response {
    let servers = state.runtime.mcp_clients.list_status().await;
    let configs = state.runtime.mcp_clients.configs();
    Json(json!({
        "servers": servers,
        "configs": configs,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct UpsertMcpServerReq {
    #[serde(flatten)]
    pub server: McpServerConfig,
    #[serde(default)]
    pub sync: bool,
    #[serde(default)]
    pub secret_values: HashMap<String, String>,
}

fn is_reserved_system_vault_key(key: &str) -> bool {
    matches!(
        key,
        VAULT_KEY_WEBUI_ADMIN
            | VAULT_KEY_TELEGRAM
            | VAULT_KEY_OPENROUTER
            | VAULT_KEY_BRAVE_SEARCH
            | VAULT_KEY_TAVILY_SEARCH
            | VAULT_KEY_WEBHOOK_SECRET
    ) || PROVIDER_REGISTRY
        .iter()
        .any(|p| !p.vault_secret_key.is_empty() && p.vault_secret_key == key)
}

pub async fn api_upsert_mcp_server(
    State(state): State<WebState>,
    Json(req): Json<UpsertMcpServerReq>,
) -> Response {
    let mut referenced_keys: std::collections::HashSet<&str> = req
        .server
        .env_secrets
        .values()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if let Some(auth_key) = req
        .server
        .auth_header_secret
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        referenced_keys.insert(auth_key);
    }
    for (vault_key, raw_val) in &req.secret_values {
        let key = vault_key.trim();
        let val = raw_val.trim();
        if key.is_empty() || val.is_empty() {
            continue;
        }
        if !referenced_keys.contains(key) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": format!(
                        "secret key `{key}` must be referenced by this server's `env_secrets` or `auth_header_secret`"
                    )
                })),
            )
                .into_response();
        }
        if is_reserved_system_vault_key(key) {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": format!(
                        "cannot overwrite reserved system or provider vault key `{key}` via MCP server config"
                    )
                })),
            )
                .into_response();
        }
    }
    for (vault_key, raw_val) in &req.secret_values {
        let key = vault_key.trim();
        let val = raw_val.trim();
        if !key.is_empty() && !val.is_empty()
            && let Err(e) = state.runtime.vault.set(key, val) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("failed to save secret `{key}`: {e}")})),
                )
                    .into_response();
            }
    }
    match state
        .runtime
        .upsert_mcp_server(req.server, req.sync)
        .await
    {
        Ok((servers, sync_error)) => Json(json!({
            "ok": true,
            "servers": servers,
            "configs": state.runtime.mcp_clients.configs(),
            "sync_error": sync_error,
        }))
        .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_delete_mcp_server(
    State(state): State<WebState>,
    Path(name): Path<String>,
) -> Response {
    match state.runtime.delete_mcp_server(&name).await {
        Ok(deleted) => Json(json!({
            "ok": true,
            "deleted": deleted,
            "servers": state.runtime.mcp_clients.list_status().await,
            "configs": state.runtime.mcp_clients.configs(),
        }))
        .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_sync_mcp_server(
    State(state): State<WebState>,
    Path(name): Path<String>,
) -> Response {
    match state.runtime.sync_mcp_server(&name).await {
        Ok(status) => Json(json!({
            "ok": true,
            "status": status,
            "servers": state.runtime.mcp_clients.list_status().await,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": e.to_string(),
                "servers": state.runtime.mcp_clients.list_status().await,
            })),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct ToggleMcpServerReq {
    pub enabled: Option<bool>,
}

pub async fn api_toggle_mcp_server(
    State(state): State<WebState>,
    Path(name): Path<String>,
    body: Option<Json<ToggleMcpServerReq>>,
) -> Response {
    let Some(mut target) = state.runtime.mcp_clients.find_config(&name) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("MCP server `{name}` not configured")})),
        )
            .into_response();
    };
    let explicit = body.and_then(|Json(b)| b.enabled);
    target.enabled = explicit.unwrap_or(!target.enabled);
    let should_sync = target.enabled;
    match state.runtime.upsert_mcp_server(target, should_sync).await {
        Ok((servers, sync_error)) => Json(json!({
            "ok": true,
            "servers": servers,
            "sync_error": sync_error,
        }))
        .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_lsp_status(State(state): State<WebState>) -> Response {
    let cfg = state.runtime.config().lsp;
    let active = state.runtime.lsp.list_status().await;
    Json(json!({
        "enabled": cfg.enabled,
        "idle_timeout_secs": cfg.idle_timeout_secs,
        "registry": LSP_SERVER_REGISTRY,
        "active": active,
    }))
    .into_response()
}

pub async fn api_lsp_stop(State(state): State<WebState>) -> Response {
    let stopped = state.runtime.lsp.stop_all().await;
    Json(json!({ "ok": true, "stopped": stopped })).into_response()
}

#[derive(Deserialize)]
pub struct UpdateWebSearchReq {
    pub enabled: Option<bool>,
    pub provider: Option<String>,
    pub searxng_url: Option<String>,
    pub allow_private_searxng: Option<bool>,
    pub default_limit: Option<usize>,
    pub timeout_secs: Option<u64>,
    pub brave_api_key: Option<String>,
    pub tavily_api_key: Option<String>,
}

pub async fn api_update_web_search(
    State(state): State<WebState>,
    Json(req): Json<UpdateWebSearchReq>,
) -> Response {
    if let Some(key) = req.brave_api_key.as_deref().map(str::trim).filter(|s| !s.is_empty())
        && let Err(e) = state.runtime.vault.set(VAULT_KEY_BRAVE_SEARCH, key) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    if let Some(key) = req.tavily_api_key.as_deref().map(str::trim).filter(|s| !s.is_empty())
        && let Err(e) = state.runtime.vault.set(VAULT_KEY_TAVILY_SEARCH, key) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    match state.runtime.update_config(|cfg| {
        if let Some(v) = req.enabled {
            cfg.web_search.enabled = v;
        }
        if let Some(p) = req.provider.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            cfg.web_search.provider = p.to_ascii_lowercase();
        }
        if let Some(u) = req.searxng_url.as_deref() {
            cfg.web_search.searxng_url = u.trim().to_string();
        }
        if let Some(v) = req.allow_private_searxng {
            cfg.web_search.allow_private_searxng = v;
        }
        if let Some(lim) = req.default_limit {
            cfg.web_search.default_limit = lim.clamp(1, zenko_core::WEB_SEARCH_MAX_LIMIT);
        }
        if let Some(ts) = req.timeout_secs {
            cfg.web_search.timeout_secs = ts.clamp(3, 60);
        }
    }) {
        Ok(updated) => Json(json!({
            "ok": true,
            "web_search": updated.web_search,
            "has_brave_search_key": state.runtime.vault.has(VAULT_KEY_BRAVE_SEARCH).unwrap_or(false),
            "has_tavily_search_key": state.runtime.vault.has(VAULT_KEY_TAVILY_SEARCH).unwrap_or(false),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
