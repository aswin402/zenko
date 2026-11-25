use super::*;

const SESSION_COOKIE: &str = "zenko_token";

pub fn is_authorized(state: &WebState, headers: &HeaderMap) -> bool {
    let cfg = state.runtime.config();
    if !cfg.webui.require_auth {
        return true;
    }
    // Fail-closed on vault read/decryption errors; only allow open access if no token exists yet.
    let expected = match state.runtime.vault.get(VAULT_KEY_WEBUI_ADMIN) {
        Ok(Some(tok)) if !tok.trim().is_empty() => tok,
        Ok(_) => return true,
        Err(_) => return false,
    };

    if let Some(auth) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok())
        && let Some(tok) = auth.strip_prefix("Bearer ")
            && SecretVault::verify_secret_eq(tok.trim(), expected.as_str()) {
                return true;
            }

    if let Some(cookie_hdr) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        for part in cookie_hdr.split(';') {
            if let Some((k, v)) = part.trim().split_once('=')
                && k == SESSION_COOKIE && SecretVault::verify_secret_eq(v.trim(), expected.as_str()) {
                    return true;
                }
        }
    }

    false
}

/// Axum authentication middleware applied once to the entire protected `/api/*` + `/v1/*`
/// sub-router, eliminating duplicated per-handler auth boilerplate.
pub async fn require_api_auth_middleware(
    State(state): State<WebState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if is_authorized(&state, req.headers()) {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response()
    }
}

/// Typed JSON API error helper implementing `IntoResponse` + `From<zenko_core::ZenkoError>`.
#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);

impl ApiError {
    pub fn internal(err: impl std::fmt::Display) -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
    }

    pub fn bad_request(err: impl std::fmt::Display) -> Self {
        Self(StatusCode::BAD_REQUEST, err.to_string())
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self(StatusCode::NOT_FOUND, msg.into())
    }
}

impl From<zenko_core::ZenkoError> for ApiError {
    fn from(err: zenko_core::ZenkoError) -> Self {
        match &err {
            zenko_core::ZenkoError::NotFound(_) => Self(StatusCode::NOT_FOUND, err.to_string()),
            zenko_core::ZenkoError::PolicyDenied(_) => Self(StatusCode::FORBIDDEN, err.to_string()),
            zenko_core::ZenkoError::Config(_) => Self(StatusCode::BAD_REQUEST, err.to_string()),
            _ => Self(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}

pub async fn api_health() -> Json<Value> {
    Json(json!({"status": "ok", "agent": "zenko"}))
}

#[derive(Deserialize)]
pub struct LoginReq {
    token: String,
}

fn is_safe_cookie_token(tok: &str) -> bool {
    !tok.is_empty()
        && tok.len() <= 512
        && tok
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~'))
}

pub async fn api_login(State(state): State<WebState>, Json(req): Json<LoginReq>) -> Response {
    let provided = req.token.trim();
    // Fail-closed on vault read/decryption errors, mirroring `is_authorized`.
    match state.runtime.vault.get(VAULT_KEY_WEBUI_ADMIN) {
        Ok(Some(exp)) if !exp.trim().is_empty() => {
            if is_safe_cookie_token(provided)
                && SecretVault::verify_secret_eq(provided, exp.as_str())
            {
                let cookie = format!(
                    "{SESSION_COOKIE}={provided}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000"
                );
                return ([(header::SET_COOKIE, cookie)], Json(json!({"ok": true}))).into_response();
            }
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Invalid admin token"})),
            )
                .into_response()
        }
        Ok(_) => {
            // No admin token configured in vault yet: do not set an untrusted cookie header.
            Json(json!({"ok": true})).into_response()
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Vault unavailable; authentication failed closed"})),
        )
            .into_response(),
    }
}

pub async fn api_bootstrap(State(state): State<WebState>, headers: HeaderMap) -> Response {
    let authorized = is_authorized(&state, &headers);
    let active_profile = state
        .runtime
        .store
        .get_active_ui_profile()
        .unwrap_or_else(|_| UiProfile {
            id: "default".into(),
            name: "Zenko Vermilion".into(),
            is_active: true,
            config: UiCustomization::default(),
            updated_at: Utc::now(),
        });

    if !authorized {
        return Json(json!({
            "authorized": false,
            "active_profile": active_profile,
            "presets": UiCustomization::builtin_presets(),
        }))
        .into_response();
    }

    let cfg = state.runtime.config();
    let mut sessions = state.runtime.store.list_root_sessions(40).unwrap_or_default();
    if sessions.is_empty()
        && let Ok(s) = state
            .runtime
            .store
            .create_session("New Session", "webui", None, None, cfg.policy.mode)
        {
            sessions.push(s);
        }

    let telemetry = state.runtime.telemetry();
    let profiles = state.runtime.store.list_ui_profiles().unwrap_or_default();
    let pending_approvals = state.runtime.store.list_pending_approvals().unwrap_or_default();
    let pending_clarifications = state.runtime.clarifications.list_pending_clarifications(None);
    let configured_providers: HashMap<&'static str, bool> = PROVIDER_REGISTRY
        .iter()
        .map(|p| {
            let has = !p.requires_api_key
                || state.runtime.vault.has(p.vault_secret_key).unwrap_or(false);
            (p.id, has)
        })
        .collect();
    let active_provider = find_provider(&cfg.provider.kind);
    let has_provider_key = !active_provider.requires_api_key
        || state
            .runtime
            .vault
            .has(active_provider.vault_secret_key)
            .unwrap_or(false);
    let has_openrouter_key = state
        .runtime
        .vault
        .has(VAULT_KEY_OPENROUTER)
        .unwrap_or(false);
    let has_telegram_token = state
        .runtime
        .vault
        .has(VAULT_KEY_TELEGRAM)
        .unwrap_or(false);
    let has_brave_search_key = state
        .runtime
        .vault
        .has(VAULT_KEY_BRAVE_SEARCH)
        .unwrap_or(false);
    let has_tavily_search_key = state
        .runtime
        .vault
        .has(VAULT_KEY_TAVILY_SEARCH)
        .unwrap_or(false);
    let mcp_servers = state.runtime.mcp_clients.list_status().await;
    let lsp_active = state.runtime.lsp.list_status().await;
    let snap = state.runtime.snapshots.load_snapshot().ok();
    let log_counts = state.runtime.store.log_category_counts().unwrap_or_default();

    Json(json!({
        "authorized": true,
        "config": cfg,
        "telemetry": telemetry,
        "sessions": sessions,
        "active_profile": active_profile,
        "profiles": profiles,
        "presets": UiCustomization::builtin_presets(),
        "studio_schema": STUDIO_SCHEMA,
        "providers": PROVIDER_REGISTRY,
        "concise_modes": CONCISE_MODE_REGISTRY,
        "minimalism_modes": CODE_MINIMALISM_REGISTRY,
        "configured_providers": configured_providers,
        "pending_approvals": pending_approvals,
        "pending_clarifications": pending_clarifications,
        "has_provider_key": has_provider_key,
        "has_openrouter_key": has_openrouter_key,
        "has_telegram_token": has_telegram_token,
        "has_brave_search_key": has_brave_search_key,
        "has_tavily_search_key": has_tavily_search_key,
        "web_search_providers": WEB_SEARCH_PROVIDER_REGISTRY,
        "mcp_servers": mcp_servers,
        "lsp_registry": LSP_SERVER_REGISTRY,
        "lsp_active": lsp_active,
        "workspace_dir": cfg.resolved_workspace_dir().display().to_string(),
        "hub_dir": cfg.resolved_hub_dir().display().to_string(),
        "hub_categories": HUB_CATEGORIES,
        "task_kinds": TASK_KINDS,
        "snapshot_tokens": snap.as_ref().map(|s| s.total_tokens_estimate).unwrap_or(0),
        "skills_count": snap.as_ref().map(|s| s.skills.iter().filter(|k| k.enabled).count()).unwrap_or(0),
        "log_categories": LOG_CATEGORIES,
        "log_counts": log_counts,
        "logs_dir": state.runtime.log_sink.logs_dir().display().to_string(),
        "jobs": state.runtime.jobs.list_jobs(None, None, JOB_DEFAULT_LIST_LIMIT).unwrap_or_default(),
        "subagent_roles": SUBAGENT_ROLE_REGISTRY,
        "subagents": state.runtime.store.list_subagents(None, SUBAGENT_LIST_DEFAULT_LIMIT).unwrap_or_default(),
    }))
    .into_response()
}

pub async fn api_events_sse(State(state): State<WebState>) -> Response {
    let rx = state.runtime.subscribe();
    let shutdown = state.shutdown.clone();
    let stream = futures_util::stream::unfold((rx, shutdown), |(mut rx, shutdown)| async move {
        loop {
            let item = tokio::select! {
                biased;
                _ = shutdown.cancelled() => return None,
                msg = rx.recv() => msg,
            };
            match item {
                Ok(ev) => {
                    if let Ok(data) = serde_json::to_string(&ev) {
                        return Some((
                            Ok::<SseEvent, Infallible>(SseEvent::default().data(data)),
                            (rx, shutdown),
                        ));
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return None,
            }
        }
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(DEFAULT_SSE_KEEPALIVE_SECS)))
        .into_response()
}

