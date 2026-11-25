use super::*;

// ─── Setup & Gateways API ─────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SetupSaveReq {
    #[serde(default)]
    provider_kind: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    openrouter_api_key: Option<String>,
    base_url: String,
    lead_model: String,
    fast_model: String,
    fallbacks: Vec<String>,
    telegram_bot_token: Option<String>,
    telegram_enabled: bool,
    workspace_dir: String,
    daily_spend_limit_usd: f64,
    policy_mode: String,
}

pub async fn api_save_setup(
    State(state): State<WebState>,
    Json(req): Json<SetupSaveReq>,
) -> Response {
    let current_kind = state.runtime.config().provider.kind;
    let target_kind = req
        .provider_kind
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&current_kind);
    let provider_desc = find_provider(target_kind);

    let submitted_key = req
        .api_key
        .as_deref()
        .or(req.openrouter_api_key.as_deref());
    if let Some(key) = submitted_key
        && !key.trim().is_empty()
            && let Err(e) = state
                .runtime
                .vault
                .set(provider_desc.vault_secret_key, key.trim())
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": e.to_string()})),
                )
                    .into_response();
            }
    if let Some(tg) = req.telegram_bot_token.as_deref()
        && !tg.trim().is_empty()
            && let Err(e) = state.runtime.vault.set(VAULT_KEY_TELEGRAM, tg.trim()) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": e.to_string()})),
                )
                    .into_response();
            }

    let res = state.runtime.update_config(|c| {
        c.general.setup_completed = true;
        if !req.workspace_dir.is_empty() {
            c.general.workspace_dir = req.workspace_dir.clone();
        }
        c.general.daily_spend_limit_usd = req.daily_spend_limit_usd.max(0.0);
        c.provider.switch_provider(
            provider_desc.id,
            Some(req.base_url.as_str()),
            Some(req.lead_model.as_str()),
            false,
        );
        if !req.fast_model.trim().is_empty() {
            c.provider.fast_model = req.fast_model.trim().to_string();
        }
        c.provider.fallbacks = req.fallbacks.clone();
        c.telegram.enabled = req.telegram_enabled;
        c.policy.mode = PermissionMode::from_str_loose(&req.policy_mode);
    });

    state.telegram_reload.notify_one();

    match res {
        Ok(cfg) => Json(json!({"ok": true, "config": cfg})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct VerifyProviderReq {
    #[serde(default)]
    provider_kind: Option<String>,
    api_key: Option<String>,
    base_url: Option<String>,
}

pub async fn api_verify_provider(
    State(state): State<WebState>,
    Json(req): Json<VerifyProviderReq>,
) -> Response {
    let cfg = state.runtime.config();
    let kind = req
        .provider_kind
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&cfg.provider.kind);
    let desc = find_provider(kind);
    let key = match req.api_key {
        Some(k) if !k.trim().is_empty() => Some(Zeroizing::new(k)),
        _ => state.runtime.vault.get(desc.vault_secret_key).ok().flatten(),
    };
    let base = req
        .base_url
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| desc.default_base_url.to_string());
    let probe = UnifiedLlmProvider::for_provider(state.runtime.http.clone(), desc.id, base, key);
    if let Err(e) = probe.verify_key().await {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response();
    }
    let count = probe.list_models().await.map(|v| v.len()).unwrap_or(0);
    Json(json!({"ok": true, "provider": desc.label, "model_count": count})).into_response()
}

#[derive(Deserialize)]
pub struct VerifyTelegramReq {
    token: Option<String>,
}

pub async fn api_verify_telegram(
    State(state): State<WebState>,
    Json(req): Json<VerifyTelegramReq>,
) -> Response {
    let tok = match req.token {
        Some(t) if !t.trim().is_empty() => Zeroizing::new(t),
        _ => match state.runtime.vault.get(VAULT_KEY_TELEGRAM).ok().flatten() {
            Some(existing) => existing,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "No Telegram token provided"})),
                )
                    .into_response()
            }
        },
    };
    match verify_bot_token(&state.runtime.http, &tok).await {
        Ok(info) => Json(info).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_get_telegram(State(state): State<WebState>) -> Response {
    let cfg = state.runtime.config();
    let peers = state.runtime.store.list_peers(Some("telegram")).unwrap_or_default();
    let codes = state
        .runtime
        .store
        .list_active_pairing_codes("telegram")
        .unwrap_or_default();
    let has_token = state.runtime.vault.has(VAULT_KEY_TELEGRAM).unwrap_or(false);
    Json(json!({
        "config": cfg.telegram,
        "connected": state.runtime.is_telegram_connected(),
        "has_token": has_token,
        "peers": peers,
        "pairing_codes": codes,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct TelegramUpdateReq {
    token: Option<String>,
    enabled: bool,
    stream_drafts: bool,
}

pub async fn api_update_telegram(
    State(state): State<WebState>,
    Json(req): Json<TelegramUpdateReq>,
) -> Response {
    if let Some(tok) = req.token.as_deref()
        && !tok.trim().is_empty()
            && let Err(e) = state.runtime.vault.set(VAULT_KEY_TELEGRAM, tok.trim()) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": e.to_string()})),
                )
                    .into_response();
            }
    if let Err(e) = state.runtime.update_config(|c| {
        c.telegram.enabled = req.enabled;
        c.telegram.stream_drafts = req.stream_drafts;
    }) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response();
    }
    state.telegram_reload.notify_one();
    Json(json!({"ok": true})).into_response()
}

#[derive(Deserialize)]
pub struct PairReq {
    role: Option<String>,
}

pub async fn api_create_telegram_pair(
    State(state): State<WebState>,
    Json(req): Json<PairReq>,
) -> Response {
    let role = PeerRole::from_str_loose(req.role.as_deref().unwrap_or("owner"));
    match state
        .runtime
        .store
        .create_pairing_code("telegram", role, DEFAULT_PAIRING_TTL_MINS)
    {
        Ok(code) => Json(code).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_delete_telegram_peer(
    State(state): State<WebState>,
    Path(peer_id): Path<String>,
) -> Response {
    match state.runtime.store.remove_peer("telegram", &peer_id) {
        Ok(removed) => Json(json!({"removed": removed})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UpdateConciseReq {
    #[serde(default)]
    concise_mode: Option<String>,
    #[serde(default)]
    auto_clarity: Option<bool>,
    #[serde(default)]
    read_less: Option<bool>,
    #[serde(default)]
    session_id: Option<String>,
}

pub async fn api_update_context_concise(
    State(state): State<WebState>,
    Json(req): Json<UpdateConciseReq>,
) -> Response {
    let parsed_mode = match req.concise_mode.as_deref() {
        Some(raw) => match ConciseMode::parse(raw) {
            Some(mode) => Some(mode),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("Invalid concise_mode `{raw}` (expected off, lite, full, or ultra)")})),
                )
                    .into_response();
            }
        },
        None => None,
    };

    let res = state.runtime.update_config(|c| {
        if let Some(m) = parsed_mode {
            c.context.concise_mode = m;
        }
        if let Some(ac) = req.auto_clarity {
            c.context.auto_clarity = ac;
        }
        if let Some(rl) = req.read_less {
            c.context.read_less = rl;
        }
    });

    if let (Some(sid), Some(mode)) = (
        req.session_id.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        parsed_mode,
    ) {
        state.runtime.set_session_concise_mode(sid, mode);
    }

    match res {
        Ok(cfg) => Json(json!({"ok": true, "context": cfg.context})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

