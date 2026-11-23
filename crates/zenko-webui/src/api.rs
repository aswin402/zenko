use crate::WebState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event as SseEvent, KeepAlive};
use axum::response::{IntoResponse, Response, Sse};
use chrono::Utc;
use croner::Cron;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::convert::Infallible;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use zeroize::Zeroizing;
use axum::body::Bytes;
use zenko_core::{
    AgentEvent, AgentMessageOptions, ApprovalDecision, CHECKPOINT_LIST_DEFAULT_LIMIT, CronJob,
    CronKind, DEFAULT_PAIRING_TTL_MINS, DEFAULT_SSE_KEEPALIVE_SECS, DashboardAnalytics,
    ExportFormat, ExportOptions, GOAL_LOOP_LIST_DEFAULT_LIMIT, GOAL_LOOP_RUNS_DEFAULT_LIMIT,
    GoalLoopCreateRequest, GoalLoopPatch, HUB_CATEGORIES, JOB_DEFAULT_LIST_LIMIT,
    JOB_DEFAULT_READ_BYTES, JOB_DEFAULT_WAIT_SECS, JOB_MAX_WAIT_SECS, JobSupervisor,
    LOG_CATEGORIES, LSP_SERVER_REGISTRY, LlmProvider, LogFilter, MEMORY_KINDS, MEMORY_SCOPE_GLOBAL,
    McpServerConfig, OrchestratorBridge, PROVIDER_REGISTRY, PeerRole, PermissionMode, RewindMode,
    RewindRequest, SESSION_CONTEXT_DEFAULT_WINDOW, STUDIO_SCHEMA, SUBAGENT_LIST_DEFAULT_LIMIT,
    SUBAGENT_ROLE_REGISTRY, SecretVault, SendMode, SessionSearchFilter, SessionTask,
    SessionTaskPatch, SessionTaskStatus, StorageSlice, SubagentSpawnRequest, SubagentWorkspaceMode,
    TASK_KINDS, TaskKind, UiCustomization, UiProfile, VAULT_KEY_BRAVE_SEARCH, VAULT_KEY_OPENROUTER,
    VAULT_KEY_TAVILY_SEARCH, VAULT_KEY_TELEGRAM, VAULT_KEY_WEBHOOK_SECRET, VAULT_KEY_WEBUI_ADMIN,
    WATCHER_MIN_POLL_INTERVAL_SECS, WEB_SEARCH_PROVIDER_REGISTRY, WEBHOOK_DEFAULT_COOLDOWN_SECS,
    WEBHOOK_DEFAULT_MAX_PER_HOUR, WEBHOOK_SIGNATURE_HEADER, WebhookRuleConfig,
    WorkspaceWatcherConfig, find_provider, new_id, new_session_task_id,
};
use zenko_gateways::{
    has_webhook_rules_for_source, match_webhook_rule, render_webhook_prompt, verify_bot_token,
    verify_hmac_sha256,
};
use zenko_memory::{parse_since_window, project_scope_for_workspace, resolve_git_root};
use zenko_providers::UnifiedLlmProvider;
use zenko_runtime::TurnOptions;

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

    if let Some(auth) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(tok) = auth.strip_prefix("Bearer ") {
            if SecretVault::verify_secret_eq(tok.trim(), expected.as_str()) {
                return true;
            }
        }
    }

    if let Some(cookie_hdr) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        for part in cookie_hdr.split(';') {
            if let Some((k, v)) = part.trim().split_once('=') {
                if k == SESSION_COOKIE && SecretVault::verify_secret_eq(v.trim(), expected.as_str()) {
                    return true;
                }
            }
        }
    }

    false
}

fn require_api_auth(state: &WebState, headers: &HeaderMap) -> Option<Response> {
    if is_authorized(state, headers) {
        None
    } else {
        Some((StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"}))).into_response())
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
    if sessions.is_empty() {
        if let Ok(s) = state
            .runtime
            .store
            .create_session("New Session", "webui", None, None, cfg.policy.mode)
        {
            sessions.push(s);
        }
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

pub async fn api_events_sse(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
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

// ─── Sessions & Messages ──────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct ListSessionsQuery {
    #[serde(default)]
    include_children: Option<bool>,
    #[serde(default)]
    limit: Option<usize>,
}

pub async fn api_list_sessions(
    State(state): State<WebState>,
    Query(q): Query<ListSessionsQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let result = match q.include_children.unwrap_or(false) {
        true => state.runtime.store.list_sessions(limit),
        false => state.runtime.store.list_root_sessions(limit),
    };
    match result {
        Ok(list) => Json(list).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_get_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let Some(session) = state.runtime.store.get_session(&id).ok().flatten() else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "session not found"})),
        )
            .into_response();
    };
    let events = state.runtime.store.get_active_branch(&id).unwrap_or_default();
    let mut siblings = HashMap::new();
    for ev in &events {
        if let Ok(list) = state
            .runtime
            .store
            .list_sibling_branches(&id, ev.parent_id.as_deref())
        {
            siblings.insert(ev.id.clone(), list.into_iter().map(|e| e.id).collect::<Vec<_>>());
        }
    }
    let pending_approvals: Vec<_> = state
        .runtime
        .store
        .list_pending_approvals()
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.session_id == id)
        .collect();

    let queued_prompts = state
        .runtime
        .store
        .list_queued_followups(&id)
        .unwrap_or_default();
    let pending_clarifications = state
        .runtime
        .clarifications
        .list_pending_clarifications(Some(&id));
    let tasks = state
        .runtime
        .clarifications
        .list_session_tasks(&id)
        .unwrap_or_default();
    let checkpoints = state
        .runtime
        .store
        .list_checkpoints(Some(&id), 30)
        .unwrap_or_default();

    Json(json!({
        "session": session,
        "events": events,
        "siblings": siblings,
        "pending_approvals": pending_approvals,
        "pending_clarifications": pending_clarifications,
        "queued_prompts": queued_prompts,
        "tasks": tasks,
        "checkpoints": checkpoints,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct CreateSessionReq {
    title: Option<String>,
    #[serde(default)]
    permission_mode: Option<String>,
    #[serde(default)]
    model_override: Option<String>,
}

pub async fn api_create_session(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(req): Json<CreateSessionReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let cfg = state.runtime.config();
    let mode = req
        .permission_mode
        .as_deref()
        .map(PermissionMode::from_str_loose)
        .unwrap_or(cfg.policy.mode);
    match state.runtime.store.create_session(
        req.title.as_deref().unwrap_or("New Session"),
        "webui",
        None,
        None,
        mode,
    ) {
        Ok(mut s) => {
            if let Some(m) = req.model_override.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                if state
                    .runtime
                    .store
                    .update_session_meta(&s.id, None, Some(Some(m)), None)
                    .is_ok()
                {
                    s.model_override = Some(m.to_string());
                }
            }
            Json(s).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_get_session_context(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.inspect_session_context(&id).await {
        Ok(ctx) => Json(ctx).into_response(),
        Err(zenko_core::ZenkoError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_clear_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let _ = state.runtime.cancel_session(&id);
    state.runtime.clear_session_deferred_tools(&id);
    state.runtime.notify_session_ended(&id).await;
    match state.runtime.store.clear_session_events(&id) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_delete_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let _ = state.runtime.cancel_session(&id);
    state.runtime.clear_session_deferred_tools(&id);
    state.runtime.notify_session_ended(&id).await;
    match state.runtime.store.delete_session(&id) {
        Ok(deleted) => Json(json!({"deleted": deleted})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UpdateSessionReq {
    title: Option<String>,
    provider_kind: Option<String>,
    model_override: Option<String>,
    permission_mode: Option<String>,
    enable_streaming: Option<bool>,
}

pub async fn api_update_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<UpdateSessionReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }

    // Allow switching active provider / model / streaming directly from the Chat Composer!
    if req.provider_kind.is_some() || req.model_override.is_some() || req.enable_streaming.is_some()
    {
        let target_kind = req.provider_kind.clone();
        let target_model = req.model_override.clone();
        let target_stream = req.enable_streaming;
        let _ = state.runtime.update_config(|c| {
            let mut switched_provider = false;
            if let Some(ref kind) = target_kind {
                let desc = find_provider(kind);
                switched_provider = !c.provider.kind.eq_ignore_ascii_case(desc.id);
                c.provider.kind = desc.id.to_string();
                if switched_provider {
                    c.provider.base_url = desc.default_base_url.to_string();
                    c.provider.plan_model.clear();
                    c.provider.advisor_model.clear();
                    c.provider.compaction_model.clear();
                    c.provider.dream_model.clear();
                    c.provider.fallbacks.clear();
                }
            }
            if let Some(ref m) = target_model {
                let chosen = m.trim();
                if !chosen.is_empty() {
                    c.provider.lead_model = chosen.to_string();
                    // Keep fast_model synchronized with explicit Composer model picks so new
                    // sessions never silently fall back to an older/stale fast model.
                    c.provider.fast_model = chosen.to_string();
                } else if switched_provider {
                    c.provider.lead_model.clear();
                    c.provider.fast_model.clear();
                }
            }
            if let Some(stream_on) = target_stream {
                c.provider.enable_streaming = stream_on;
            }
        });
    }

    let model = req.model_override.as_deref().map(|m| {
        if m.trim().is_empty() { None } else { Some(m.trim()) }
    });
    let perm = req.permission_mode.as_deref().map(PermissionMode::from_str_loose);
    match state
        .runtime
        .store
        .update_session_meta(&id, req.title.as_deref(), model, perm)
    {
        Ok(()) => Json(json!({
            "ok": true,
            "config": state.runtime.config()
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct SendMessageReq {
    text: String,
    parent_event_id: Option<Option<String>>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

pub async fn api_send_message(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<SendMessageReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let send_mode = req
        .mode
        .as_deref()
        .map(SendMode::from_str_loose)
        .unwrap_or(SendMode::Auto);
    match state.runtime.resolve_session_dispatch(&id, send_mode) {
        zenko_core::AutoDispatchDecision::Steer => {
            if state.runtime.steer_session(&id, &req.text) {
                return Json(json!({
                    "accepted": true,
                    "steered": true,
                    "queued": false,
                    "dispatch_mode": "steered"
                }))
                .into_response();
            }
        }
        zenko_core::AutoDispatchDecision::Queue => {
            return match state.runtime.enqueue_session_followup(&id, &req.text) {
                Ok((queued, count)) => (
                    StatusCode::ACCEPTED,
                    Json(json!({
                        "accepted": true,
                        "steered": false,
                        "queued": true,
                        "dispatch_mode": "queued",
                        "queued_prompt": queued,
                        "queued_count": count
                    })),
                )
                    .into_response(),
                Err(e) => (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": e.to_string()})),
                )
                    .into_response(),
            };
        }
        zenko_core::AutoDispatchDecision::StartTurn => {}
    }
    let rt = state.runtime.clone();
    let text = req.text;
    let parent = req.parent_event_id;
    let model_override = req
        .model
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    tokio::spawn(async move {
        let _ = rt
            .run_turn_with_options(
                &id,
                &text,
                TurnOptions {
                    parent_event_override: parent,
                    model_override,
                    send_mode,
                    ..TurnOptions::default()
                },
            )
            .await;
    });
    (
        StatusCode::ACCEPTED,
        Json(json!({"accepted": true, "steered": false, "queued": false})),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct SteerReq {
    text: String,
}

pub async fn api_steer_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<SteerReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let steered = state.runtime.steer_session(&id, &req.text);
    Json(json!({"steered": steered, "session_id": id})).into_response()
}

pub async fn api_background_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let demoted = state.runtime.demote_foreground_job(&id);
    Json(json!({"demoted": demoted, "session_id": id})).into_response()
}

pub async fn api_list_session_queue(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.store.list_queued_followups(&id) {
        Ok(list) => Json(list).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_cancel_session_queue(
    State(state): State<WebState>,
    Path((id, qid)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.cancel_session_followup(&id, &qid) {
        Ok((cancelled, count)) => {
            Json(json!({"cancelled": cancelled, "queued_count": count})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_steer_queued_prompt(
    State(state): State<WebState>,
    Path((id, qid)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.promote_queued_to_steer(&id, &qid) {
        Ok((steered, count)) => {
            Json(json!({"steered": steered, "queued_count": count})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct JobsListQuery {
    session_id: Option<String>,
    kind: Option<String>,
    limit: Option<usize>,
}

pub async fn api_list_jobs(
    State(state): State<WebState>,
    Query(q): Query<JobsListQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let kind = q.kind.as_deref().and_then(TaskKind::parse_filter);
    match state.runtime.jobs.list_jobs(
        q.session_id.as_deref(),
        kind,
        q.limit.unwrap_or(JOB_DEFAULT_LIST_LIMIT),
    ) {
        Ok(list) => Json(list).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct StartJobReq {
    command: String,
    kind: Option<String>,
    label: Option<String>,
    session_id: Option<String>,
    workspace: Option<String>,
}

pub async fn api_start_job(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(req): Json<StartJobReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    if req.command.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "command cannot be empty"})),
        )
            .into_response();
    }
    let kind = req
        .kind
        .as_deref()
        .and_then(TaskKind::parse_filter)
        .unwrap_or_else(|| TaskKind::infer_from_command(&req.command));
    let sid = req
        .session_id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("webui");
    let ws = req
        .workspace
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| state.runtime.config().resolved_workspace_dir());
    match state
        .runtime
        .jobs
        .spawn_shell(sid, &req.command, &ws, kind, req.label, false)
        .await
    {
        Ok(rec) => (StatusCode::CREATED, Json(rec)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UpdateJobReq {
    label: Option<String>,
    kind: Option<String>,
}

pub async fn api_update_job(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<UpdateJobReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let kind = req.kind.as_deref().and_then(TaskKind::parse_filter);
    match state.runtime.jobs.update_job_meta(&id, req.label, kind) {
        Ok(Some(rec)) => Json(rec).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("task `{id}` not found")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_restart_job(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.jobs.restart_job(&id).await {
        Ok(rec) => Json(rec).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_delete_job(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.jobs.delete_job(&id).await {
        Ok(deleted) => Json(json!({"deleted": deleted, "id": id})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct PruneJobsReq {
    kind: Option<String>,
}

pub async fn api_prune_jobs(
    State(state): State<WebState>,
    headers: HeaderMap,
    body: Option<Json<PruneJobsReq>>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let kind = body
        .and_then(|b| b.0.kind)
        .as_deref()
        .and_then(TaskKind::parse_filter);
    match state.runtime.jobs.prune_jobs(kind).await {
        Ok(pruned) => Json(json!({"pruned": pruned})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct HubListQuery {
    category: Option<String>,
    limit: Option<usize>,
}

pub async fn api_list_hub(
    State(state): State<WebState>,
    Query(q): Query<HubListQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state
        .runtime
        .list_hub_entries(q.category.as_deref(), q.limit.unwrap_or(100))
    {
        Ok(entries) => Json(json!({
            "hub_dir": state.runtime.config().resolved_hub_dir().display().to_string(),
            "categories": HUB_CATEGORIES,
            "entries": entries,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct JobReadQuery {
    offset: Option<u64>,
    max_bytes: Option<usize>,
}

pub async fn api_read_job_output(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(q): Query<JobReadQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state
        .runtime
        .jobs
        .read_job_output(&id, q.offset, q.max_bytes.unwrap_or(JOB_DEFAULT_READ_BYTES))
        .await
    {
        Ok(chunk) => Json(chunk).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_clear_job_output(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.jobs.clear_job_output(&id).await {
        Ok(Some(job)) => Json(json!({"cleared": true, "job": job})).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("task `{id}` not found")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct JobInputReq {
    input: String,
}

pub async fn api_job_input(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<JobInputReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.jobs.send_input(&id, &req.input).await {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct JobWaitReq {
    timeout_secs: Option<u64>,
    tail_bytes: Option<usize>,
}

pub async fn api_wait_job(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Option<Json<JobWaitReq>>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let req = body.map(|b| b.0).unwrap_or_default();
    let timeout_secs = req
        .timeout_secs
        .unwrap_or(JOB_DEFAULT_WAIT_SECS)
        .clamp(1, JOB_MAX_WAIT_SECS);
    let tail_bytes = req.tail_bytes.unwrap_or(JOB_DEFAULT_READ_BYTES);
    match state
        .runtime
        .jobs
        .wait_job(&id, Duration::from_secs(timeout_secs))
        .await
    {
        Ok(Some(rec)) => {
            let output = state
                .runtime
                .jobs
                .read_job_output(&rec.id, None, tail_bytes)
                .await
                .ok();
            Json(json!({"job": rec, "output": output})).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("job `{id}` not found")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_stop_job(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.jobs.stop_job(&id).await {
        Ok(rec) => Json(json!({"stopped": rec.is_some(), "job": rec})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_stop_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let cancelled = state.runtime.cancel_session(&id);
    Json(json!({"cancelled": cancelled})).into_response()
}

#[derive(Deserialize)]
pub struct SwitchBranchReq {
    event_id: String,
}

pub async fn api_switch_branch(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<SwitchBranchReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.store.find_latest_leaf(&id, &req.event_id) {
        Ok(leaf) => {
            let _ = state.runtime.store.set_session_head(&id, Some(&leaf));
            Json(json!({"head": leaf})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct ResolveApprovalReq {
    decision: ApprovalDecision,
}

pub async fn api_resolve_approval(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ResolveApprovalReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.resolve_approval(&id, req.decision, "webui") {
        Ok(rec) => Json(json!({"resolved": rec.is_some()})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct ClarificationsQuery {
    session_id: Option<String>,
}

pub async fn api_list_clarifications(
    State(state): State<WebState>,
    Query(q): Query<ClarificationsQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let list = state
        .runtime
        .clarifications
        .list_pending_clarifications(q.session_id.as_deref());
    Json(list).into_response()
}

#[derive(Deserialize)]
pub struct ResolveClarificationReq {
    selected_option_id: String,
    #[serde(default)]
    custom_answer: Option<String>,
}

pub async fn api_resolve_clarification(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ResolveClarificationReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let custom = req
        .custom_answer
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    match state.runtime.clarifications.resolve_clarification(
        &id,
        req.selected_option_id.trim(),
        custom,
        "webui",
    ) {
        Some(rec) => Json(json!({"resolved": true, "clarification": rec})).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("clarification `{id}` not found")})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct TriageReq {
    prompt: String,
    #[serde(default)]
    mode: Option<String>,
}

pub async fn api_orchestrator_triage(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(req): Json<TriageReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let mode = req.mode.as_deref().map(PermissionMode::from_str_loose);
    let report = state.runtime.triage_prompt(&req.prompt, mode);
    Json(report).into_response()
}

// ─── Phase 5: Async Subagents, Worktrees & Mailbox API ────────────────────────

#[derive(Deserialize, Default)]
pub struct AgentsListQuery {
    parent_session_id: Option<String>,
    limit: Option<usize>,
}

pub async fn api_list_agents(
    State(state): State<WebState>,
    Query(q): Query<AgentsListQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let parent_filter = q
        .parent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let limit = q.limit.unwrap_or(SUBAGENT_LIST_DEFAULT_LIMIT);
    match state.runtime.store.list_subagents(parent_filter, limit) {
        Ok(agents) => Json(json!({
            "roles": SUBAGENT_ROLE_REGISTRY,
            "active_count": state.runtime.subagents.active_count(),
            "agents": agents,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct SpawnAgentReq {
    #[serde(default)]
    parent_session_id: Option<String>,
    role: String,
    goal: String,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    run_in_background: Option<bool>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    output_schema: Option<Value>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    fork_context: Option<bool>,
    #[serde(default)]
    include_uncommitted: Option<bool>,
    #[serde(default)]
    max_duration_secs: Option<u64>,
    #[serde(default)]
    max_cost_usd: Option<f64>,
}

pub async fn api_spawn_agent(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(req): Json<SpawnAgentReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let spawn_req = SubagentSpawnRequest {
        parent_session_id: req
            .parent_session_id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "webui".into()),
        role: req.role,
        goal: req.goal,
        context: req.context,
        run_in_background: req.run_in_background.unwrap_or(true),
        workspace_mode: req
            .workspace
            .as_deref()
            .map(SubagentWorkspaceMode::from_str_loose),
        output_schema: req.output_schema,
        model_override: req.model,
        fork_context: req.fork_context.unwrap_or(false),
        include_uncommitted: req.include_uncommitted.unwrap_or(false),
        max_duration_secs: req.max_duration_secs,
        max_cost_usd: req.max_cost_usd,
    };
    match state
        .runtime
        .subagents
        .spawn_agent(&state.runtime, spawn_req)
        .await
    {
        Ok(rec) => (StatusCode::CREATED, Json(rec)).into_response(),
        Err(zenko_core::ZenkoError::PolicyDenied(msg)) => {
            (StatusCode::FORBIDDEN, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_get_agent(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.store.get_subagent(&id) {
        Ok(Some(rec)) => {
            let messages = state
                .runtime
                .store
                .list_agent_messages(&id, 50)
                .unwrap_or_default();
            Json(json!({ "agent": rec, "messages": messages })).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("subagent `{id}` not found")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct KillAgentQuery {
    remove_worktree: Option<bool>,
}

pub async fn api_kill_agent(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(q): Query<KillAgentQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let remove_wt = q.remove_worktree.unwrap_or(true);
    match state
        .runtime
        .subagents
        .kill_agent(&state.runtime, "webui", &id, remove_wt)
        .await
    {
        Ok(Some(rec)) => Json(json!({"killed": true, "agent": rec})).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("subagent `{id}` not found")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct AgentMessageReq {
    message: String,
    #[serde(default)]
    from_session_id: Option<String>,
    #[serde(default)]
    resume: Option<bool>,
    #[serde(default)]
    interrupt: Option<bool>,
}

pub async fn api_message_agent(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<AgentMessageReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let from = req
        .from_session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("webui");
    let opts = AgentMessageOptions {
        resume: req.resume.unwrap_or(true),
        interrupt: req.interrupt.unwrap_or(false),
    };
    match state
        .runtime
        .subagents
        .send_message(&state.runtime, from, &id, &req.message, opts)
        .await
    {
        Ok(mail) => Json(mail).into_response(),
        Err(zenko_core::ZenkoError::PolicyDenied(msg)) => {
            (StatusCode::FORBIDDEN, Json(json!({"error": msg}))).into_response()
        }
        Err(zenko_core::ZenkoError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_agent_diff(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state
        .runtime
        .subagents
        .inspect_worktree_diff(&state.runtime, "webui", &id)
        .await
    {
        Ok(Some(rep)) => Json(rep).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("no git worktree for subagent `{id}`")})),
        )
            .into_response(),
        Err(zenko_core::ZenkoError::PolicyDenied(msg)) => {
            (StatusCode::FORBIDDEN, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct MergeWorktreeReq {
    cleanup: Option<bool>,
}

pub async fn api_merge_agent_worktree(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Option<Json<MergeWorktreeReq>>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let cleanup = body.and_then(|b| b.0.cleanup).unwrap_or(true);
    match state
        .runtime
        .subagents
        .merge_worktree(&state.runtime, "webui", &id, cleanup)
        .await
    {
        Ok(rep) => Json(rep).into_response(),
        Err(zenko_core::ZenkoError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
        }
        Err(zenko_core::ZenkoError::PolicyDenied(msg)) => {
            (StatusCode::CONFLICT, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_discard_agent_worktree(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state
        .runtime
        .subagents
        .discard_worktree(&state.runtime, "webui", &id)
        .await
    {
        Ok(rep) => Json(rep).into_response(),
        Err(zenko_core::ZenkoError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
        }
        Err(zenko_core::ZenkoError::PolicyDenied(msg)) => {
            (StatusCode::CONFLICT, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_prune_agent_worktrees(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state
        .runtime
        .subagents
        .prune_orphan_worktrees(&state.runtime)
        .await
    {
        Ok(rep) => Json(rep).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ─── Phase 6: Session Task DAG, Plan Approval & Workspace Checkpoints / Rewind ──

#[derive(Deserialize, Default)]
pub struct ListSessionTasksQuery {
    #[serde(default)]
    ready_only: Option<bool>,
}

pub async fn api_list_session_tasks(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(q): Query<ListSessionTasksQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let res = match q.ready_only.unwrap_or(false) {
        true => state.runtime.clarifications.ready_session_tasks(&id),
        false => state.runtime.clarifications.list_session_tasks(&id),
    };
    match res {
        Ok(tasks) => Json(json!({ "tasks": tasks })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct CreateSessionTaskReq {
    #[serde(default)]
    id: Option<String>,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    active_label: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    owner_agent: Option<String>,
    #[serde(default)]
    verification_cmd: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    priority: i32,
}

pub async fn api_create_session_task(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CreateSessionTaskReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let now = Utc::now();
    let task = SessionTask {
        id: req
            .id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(new_session_task_id),
        session_id: id.clone(),
        branch_event_id: None,
        title: req.title,
        description: req.description,
        active_label: req.active_label,
        status: req
            .status
            .as_deref()
            .map(SessionTaskStatus::from_str_loose)
            .unwrap_or(SessionTaskStatus::Pending),
        depends_on: req.depends_on,
        owner_agent: req.owner_agent,
        verification_cmd: req.verification_cmd,
        verification_evidence: None,
        notes: req.notes,
        priority: req.priority,
        created_at: now,
        updated_at: now,
    };
    match state.runtime.clarifications.create_session_task(&id, task) {
        Ok(saved) => (StatusCode::CREATED, Json(saved)).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct UpdateSessionTaskReq {
    title: Option<String>,
    description: Option<String>,
    active_label: Option<Option<String>>,
    status: Option<String>,
    depends_on: Option<Vec<String>>,
    #[serde(default)]
    add_depends_on: Vec<String>,
    owner_agent: Option<Option<String>>,
    verification_cmd: Option<Option<String>>,
    verification_evidence: Option<Option<String>>,
    notes: Option<Option<String>>,
    priority: Option<i32>,
}

pub async fn api_update_session_task(
    State(state): State<WebState>,
    Path((id, tid)): Path<(String, String)>,
    headers: HeaderMap,
    Json(req): Json<UpdateSessionTaskReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let patch = SessionTaskPatch {
        title: req.title,
        description: req.description,
        active_label: req.active_label,
        status: req.status.as_deref().map(SessionTaskStatus::from_str_loose),
        depends_on: req.depends_on,
        add_depends_on: req.add_depends_on,
        owner_agent: req.owner_agent,
        verification_cmd: req.verification_cmd,
        verification_evidence: req.verification_evidence,
        notes: req.notes,
        priority: req.priority,
    };
    match state
        .runtime
        .clarifications
        .update_session_task(&id, &tid, patch)
    {
        Ok(updated) => Json(updated).into_response(),
        Err(zenko_core::ZenkoError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct ClaimSessionTaskReq {
    owner_agent: Option<String>,
}

pub async fn api_claim_session_task(
    State(state): State<WebState>,
    Path((id, tid)): Path<(String, String)>,
    headers: HeaderMap,
    body: Option<Json<ClaimSessionTaskReq>>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let owner = body
        .and_then(|b| b.0.owner_agent)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "webui".into());
    match state
        .runtime
        .clarifications
        .claim_session_task(&id, &tid, &owner)
    {
        Ok(claimed) => Json(claimed).into_response(),
        Err(zenko_core::ZenkoError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
        }
        Err(zenko_core::ZenkoError::PolicyDenied(msg)) => {
            (StatusCode::CONFLICT, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_delete_session_task(
    State(state): State<WebState>,
    Path((id, tid)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.clarifications.delete_session_task(&id, &tid) {
        Ok(deleted) => Json(json!({ "deleted": deleted })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct ApprovePlanReq {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    execute_prompt: Option<String>,
}

pub async fn api_approve_session_plan(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Option<Json<ApprovePlanReq>>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let req = body.map(|b| b.0).unwrap_or_default();
    let target_mode = req
        .mode
        .as_deref()
        .map(PermissionMode::from_str_loose)
        .unwrap_or(PermissionMode::Autonomous);
    match state.runtime.approve_session_plan(&id, target_mode) {
        Ok((session, task_count)) => {
            let mut dispatched = false;
            if let Some(prompt) = req
                .execute_prompt
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let rt = state.runtime.clone();
                let sid = id.clone();
                let text = prompt.to_string();
                dispatched = true;
                tokio::spawn(async move {
                    let _ = rt.run_turn_with_options(&sid, &text, TurnOptions::default()).await;
                });
            }
            Json(json!({
                "ok": true,
                "session": session,
                "task_count": task_count,
                "dispatched": dispatched,
            }))
            .into_response()
        }
        Err(zenko_core::ZenkoError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct ListCheckpointsQuery {
    #[serde(default)]
    limit: Option<usize>,
}

pub async fn api_list_session_checkpoints(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(q): Query<ListCheckpointsQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let limit = q.limit.unwrap_or(CHECKPOINT_LIST_DEFAULT_LIMIT);
    match state.runtime.store.list_checkpoints(Some(&id), limit) {
        Ok(checkpoints) => Json(json!({ "checkpoints": checkpoints })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_diff_checkpoint(
    State(state): State<WebState>,
    Path(cp_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let cp_cfg = state.runtime.config().checkpoints;
    match state
        .runtime
        .checkpoints
        .diff_checkpoint(&cp_cfg, &cp_id)
        .await
    {
        Ok(rep) => Json(rep).into_response(),
        Err(zenko_core::ZenkoError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct RewindSessionReq {
    #[serde(default)]
    checkpoint_id: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    overwrite_manual_edits: bool,
    #[serde(default = "default_summarize_true")]
    summarize_abandoned: bool,
}

fn default_summarize_true() -> bool {
    true
}

pub async fn api_rewind_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<RewindSessionReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    if state.runtime.is_session_running(&id) {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "Cannot rewind while a turn is actively running on this session. Stop the turn first."})),
        )
            .into_response();
    }
    let cp_id = match req
        .checkpoint_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(explicit) => {
            match state.runtime.store.get_checkpoint(explicit) {
                Ok(Some(cp)) if cp.session_id == id => cp.id,
                Ok(Some(_)) => {
                    return (
                        StatusCode::FORBIDDEN,
                        Json(json!({"error": format!("checkpoint `{explicit}` does not belong to session `{id}`")})),
                    )
                        .into_response();
                }
                Ok(None) => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({"error": format!("checkpoint `{explicit}` not found")})),
                    )
                        .into_response();
                }
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"error": e.to_string()})),
                    )
                        .into_response();
                }
            }
        }
        None => {
            let list = state
                .runtime
                .store
                .list_checkpoints(Some(&id), CHECKPOINT_LIST_DEFAULT_LIMIT)
                .unwrap_or_default();
            match list.into_iter().find(|c| !c.is_safety_backup) {
                Some(cp) => cp.id,
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({"error": format!("no checkpoints found for session `{id}`")})),
                    )
                        .into_response();
                }
            }
        }
    };
    let cp_cfg = state.runtime.config().checkpoints;
    match state
        .runtime
        .checkpoints
        .rewind(
            &cp_cfg,
            &RewindRequest {
                checkpoint_id: cp_id,
                mode: req
                    .mode
                    .as_deref()
                    .map(RewindMode::from_str_loose)
                    .unwrap_or(RewindMode::CodeAndConversation),
                files: req.files,
                overwrite_manual_edits: req.overwrite_manual_edits,
                summarize_abandoned: req.summarize_abandoned,
            },
        )
        .await
    {
        Ok(rep) => Json(rep).into_response(),
        Err(zenko_core::ZenkoError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_prune_checkpoints(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let cp_cfg = state.runtime.config().checkpoints;
    match state.runtime.checkpoints.prune_checkpoints(&cp_cfg).await {
        Ok(rep) => Json(rep).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ─── Models (100% Live Discovery — Zero Hardcoded Lists) ──────────────────────

#[derive(Deserialize)]
pub struct ModelsQuery {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    refresh: Option<bool>,
}

pub async fn api_models(
    State(state): State<WebState>,
    Query(q): Query<ModelsQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let cfg = state.runtime.config();
    let provider = q
        .provider
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&cfg.provider.kind);
    let force_refresh = q.refresh.unwrap_or(false);

    match state
        .runtime
        .list_models_for(provider, q.base_url.as_deref(), force_refresh)
        .await
    {
        Ok(models) => Json(models).into_response(),
        Err(_) => Json(Vec::<zenko_core::ModelInfo>::new()).into_response(),
    }
}

// ─── UI Customization API ─────────────────────────────────────────────────────

pub async fn api_update_active_ui(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(cfg): Json<UiCustomization>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let mut active = state
        .runtime
        .store
        .get_active_ui_profile()
        .unwrap_or_else(|_| UiProfile {
            id: "default".into(),
            name: "Zenko Vermilion".into(),
            is_active: true,
            config: cfg.clone(),
            updated_at: Utc::now(),
        });
    active.config = cfg;
    active.updated_at = Utc::now();
    match state.runtime.store.save_ui_profile(&active) {
        Ok(()) => Json(active).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct ApplyPresetQuery {
    pub mode: Option<zenko_core::ui::ThemeMode>,
}

pub async fn api_apply_ui_preset(
    State(state): State<WebState>,
    Path(preset_id): Path<String>,
    Query(query): Query<ApplyPresetQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let mut active = state
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
    let (_, legacy_mode) = UiCustomization::resolve_preset_alias(&preset_id);
    let target_mode = query.mode.or(legacy_mode).unwrap_or(active.config.mode);
    let new_preset = UiCustomization::preset_with_mode(&preset_id, Some(target_mode));
    active.config.mode = new_preset.mode;
    active.config.preset_id = new_preset.preset_id;
    active.config.colors = new_preset.colors;
    active.updated_at = Utc::now();
    match state.runtime.store.save_ui_profile(&active) {
        Ok(()) => Json(active).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct CreateUiProfileReq {
    name: String,
    config: UiCustomization,
}

pub async fn api_create_ui_profile(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(req): Json<CreateUiProfileReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let profile = UiProfile {
        id: new_id(),
        name: req.name,
        is_active: true,
        config: req.config,
        updated_at: Utc::now(),
    };
    match state.runtime.store.save_ui_profile(&profile) {
        Ok(()) => Json(profile).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_activate_ui_profile(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.store.activate_ui_profile(&id) {
        Ok(ok) => {
            let active = state.runtime.store.get_active_ui_profile().ok();
            Json(json!({"activated": ok, "active_profile": active})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

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
    headers: HeaderMap,
    Json(req): Json<SetupSaveReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
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
    if let Some(key) = submitted_key {
        if !key.trim().is_empty() {
            if let Err(e) = state
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
        }
    }
    if let Some(tg) = req.telegram_bot_token.as_deref() {
        if !tg.trim().is_empty() {
            if let Err(e) = state.runtime.vault.set(VAULT_KEY_TELEGRAM, tg.trim()) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": e.to_string()})),
                )
                    .into_response();
            }
        }
    }

    let res = state.runtime.update_config(|c| {
        c.general.setup_completed = true;
        if !req.workspace_dir.is_empty() {
            c.general.workspace_dir = req.workspace_dir.clone();
        }
        c.general.daily_spend_limit_usd = req.daily_spend_limit_usd.max(0.0);
        c.provider.kind = provider_desc.id.to_string();
        if !req.base_url.is_empty() {
            c.provider.base_url = req.base_url.clone();
        }
        if !req.lead_model.is_empty() {
            c.provider.lead_model = req.lead_model.clone();
        }
        if !req.fast_model.is_empty() {
            c.provider.fast_model = req.fast_model.clone();
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
    headers: HeaderMap,
    Json(req): Json<VerifyProviderReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
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
    headers: HeaderMap,
    Json(req): Json<VerifyTelegramReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
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

pub async fn api_get_telegram(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
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
    headers: HeaderMap,
    Json(req): Json<TelegramUpdateReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    if let Some(tok) = req.token.as_deref() {
        if !tok.trim().is_empty() {
            if let Err(e) = state.runtime.vault.set(VAULT_KEY_TELEGRAM, tok.trim()) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": e.to_string()})),
                )
                    .into_response();
            }
        }
    }
    let _ = state.runtime.update_config(|c| {
        c.telegram.enabled = req.enabled;
        c.telegram.stream_drafts = req.stream_drafts;
    });
    state.telegram_reload.notify_one();
    Json(json!({"ok": true})).into_response()
}

#[derive(Deserialize)]
pub struct PairReq {
    role: Option<String>,
}

pub async fn api_create_telegram_pair(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(req): Json<PairReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
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
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.store.remove_peer("telegram", &peer_id) {
        Ok(removed) => Json(json!({"removed": removed})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ─── Memory, Skills, Cron & Logs API ──────────────────────────────────────────

#[derive(Deserialize)]
pub struct SessionSearchQuery {
    q: Option<String>,
    around_event_id: Option<String>,
    window: Option<usize>,
    since: Option<String>,
    channel: Option<String>,
    workspace: Option<String>,
    role: Option<String>,
    session_id: Option<String>,
    limit: Option<usize>,
}

pub async fn api_search_sessions(
    State(state): State<WebState>,
    Query(q): Query<SessionSearchQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    if let Some(anchor) = q
        .around_event_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let win = q.window.unwrap_or(SESSION_CONTEXT_DEFAULT_WINDOW);
        return match state.runtime.store.get_session_event_context(anchor, win) {
            Ok(Some(ctx)) => Json(json!({ "context": ctx })).into_response(),
            Ok(None) => (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("event `{anchor}` not found")})),
            )
                .into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        };
    }
    let query = q.q.unwrap_or_default();
    let since = q.since.as_deref().and_then(parse_since_window);
    let workspace_dir = match q.workspace.as_deref().unwrap_or("all").trim() {
        "" | "all" => None,
        "current" => Some(
            resolve_git_root(&state.runtime.config().resolved_workspace_dir())
                .display()
                .to_string(),
        ),
        other => Some(
            resolve_git_root(&std::path::PathBuf::from(other))
                .display()
                .to_string(),
        ),
    };
    let filter = SessionSearchFilter {
        query,
        since,
        channel: q.channel.filter(|s| !s.trim().is_empty()),
        workspace_dir,
        session_id: q.session_id.filter(|s| !s.trim().is_empty()),
        exclude_session_id: None,
        role: q.role.filter(|s| !s.trim().is_empty()),
        limit: q.limit,
    };
    match state.runtime.store.search_sessions_fts(&filter) {
        Ok(hits) => Json(json!({ "hits": hits })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
    scope: Option<String>,
}

pub async fn api_get_memory(
    State(state): State<WebState>,
    Query(q): Query<SearchQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let snap = match state.runtime.snapshots.load_snapshot() {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };
    let cfg = state.runtime.config();
    let ws = cfg.resolved_workspace_dir();
    let proj_scope = project_scope_for_workspace(&ws);
    let (scope_opt, include_global) = match q
        .scope
        .as_deref()
        .unwrap_or("all")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "global" => (None, false),
        "project" => (Some(proj_scope.as_str()), false),
        "current" => (Some(proj_scope.as_str()), true),
        _ => (None, true),
    };
    let ws_check = match cfg.memory.verify_file_staleness {
        true => Some(ws.as_path()),
        false => None,
    };
    let hits = state
        .runtime
        .store
        .search_memory_hybrid(
            q.q.as_deref().unwrap_or(""),
            scope_opt,
            include_global,
            ws_check,
            60,
        )
        .unwrap_or_default();
    let entries: Vec<_> = hits.iter().map(|h| h.entry.clone()).collect();
    let dream = state.runtime.dream_status();
    let history = state
        .runtime
        .snapshots
        .list_doc_history(None)
        .unwrap_or_default();
    let skill_history = state
        .runtime
        .snapshots
        .list_skill_history(None)
        .unwrap_or_default();
    Json(json!({
        "snapshot": snap,
        "entries": entries,
        "hits": hits,
        "project_scope": proj_scope,
        "memory_kinds": MEMORY_KINDS,
        "dream": dream,
        "history": history,
        "skill_history": skill_history,
        "skills_config": cfg.skills,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct SaveContentReq {
    content: String,
}

pub async fn api_save_doc(
    State(state): State<WebState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(req): Json<SaveContentReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let _ = state
        .runtime
        .snapshots
        .snapshot_doc_before_edit(&name, "webui");
    match state.runtime.snapshots.write_doc(&name, &req.content) {
        Ok(()) => {
            state
                .runtime
                .broadcast_memory_delta(format!("{name} was edited via WebUI."));
            Json(json!({"ok": true})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_list_skills(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.snapshots.list_skills() {
        Ok(skills) => Json(skills).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_save_skill(
    State(state): State<WebState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(req): Json<SaveContentReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let cfg = state.runtime.config().skills;
    let secrets = state.runtime.vault.all_values();
    match state.runtime.snapshots.save_skill_with_secrets(
        &slug,
        &req.content,
        "webui",
        &secrets,
        cfg.max_history_snapshots,
    ) {
        Ok((sk, snap)) => {
            state.runtime.emit(AgentEvent::SkillUpdated {
                slug: sk.slug.clone(),
                action: "saved".into(),
                status: sk.status.as_str().into(),
            });
            Json(json!({"skill": sk, "snapshot": snap})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

#[derive(Deserialize)]
pub struct ToggleSkillReq {
    enabled: bool,
}

pub async fn api_toggle_skill(
    State(state): State<WebState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ToggleSkillReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let max_h = state.runtime.config().skills.max_history_snapshots;
    match state
        .runtime
        .snapshots
        .toggle_skill(&slug, req.enabled, max_h)
    {
        Ok((sk, snap)) => {
            state.runtime.emit(AgentEvent::SkillUpdated {
                slug: sk.slug.clone(),
                action: "toggled".into(),
                status: sk.status.as_str().into(),
            });
            Json(json!({"skill": sk, "snapshot": snap})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_approve_skill(
    State(state): State<WebState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let max_h = state.runtime.config().skills.max_history_snapshots;
    match state.runtime.snapshots.approve_skill(&slug, max_h) {
        Ok((sk, snap)) => {
            state.runtime.emit(AgentEvent::SkillUpdated {
                slug: sk.slug.clone(),
                action: "approved".into(),
                status: sk.status.as_str().into(),
            });
            Json(json!({"skill": sk, "snapshot": snap})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

#[derive(Deserialize)]
pub struct SkillHistoryQuery {
    slug: Option<String>,
}

pub async fn api_list_skill_history(
    State(state): State<WebState>,
    Query(q): Query<SkillHistoryQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state
        .runtime
        .snapshots
        .list_skill_history(q.slug.as_deref())
    {
        Ok(items) => Json(items).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct SkillRollbackReq {
    snapshot_id: String,
}

pub async fn api_rollback_skill(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(req): Json<SkillRollbackReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let max_h = state.runtime.config().skills.max_history_snapshots;
    match state
        .runtime
        .snapshots
        .rollback_skill(&req.snapshot_id, max_h)
    {
        Ok((sk, diff)) => {
            state.runtime.emit(AgentEvent::SkillUpdated {
                slug: sk.slug.clone(),
                action: "rollback".into(),
                status: sk.status.as_str().into(),
            });
            Json(json!({"ok": true, "skill": sk, "diff": diff})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_delete_skill(
    State(state): State<WebState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.snapshots.delete_skill(&slug) {
        Ok(ok) => Json(json!({"deleted": ok})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct AddMemoryReq {
    title: String,
    content: String,
    kind: Option<String>,
    scope: Option<String>,
}

pub async fn api_add_memory(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(req): Json<AddMemoryReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let cfg = state.runtime.config();
    let scope = match req.scope.as_deref().unwrap_or(MEMORY_SCOPE_GLOBAL).trim() {
        "project" => project_scope_for_workspace(&cfg.resolved_workspace_dir()),
        "" => MEMORY_SCOPE_GLOBAL.to_string(),
        other => other.to_string(),
    };
    let secrets = state.runtime.vault.all_values();
    match state.runtime.store.upsert_memory_dedup_with_secrets(
        &scope,
        req.kind.as_deref().unwrap_or("fact"),
        &req.title,
        &req.content,
        1.0,
        None,
        None,
        cfg.memory.dedup_enabled,
        &secrets,
    ) {
        Ok(res) => {
            state.runtime.broadcast_memory_delta(format!(
                "Memory entry `{}` added/merged via WebUI.",
                res.entry.title
            ));
            Json(res.entry).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UpdateMemoryReq {
    title: Option<String>,
    content: Option<String>,
    kind: Option<String>,
    salience: Option<f64>,
    scope: Option<String>,
}

pub async fn api_update_memory(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<UpdateMemoryReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let resolved_scope = req.scope.as_deref().map(|sc| match sc.trim() {
        "project" => project_scope_for_workspace(&state.runtime.config().resolved_workspace_dir()),
        "" => MEMORY_SCOPE_GLOBAL.to_string(),
        other => other.to_string(),
    });
    match state.runtime.store.update_memory(
        &id,
        req.title.as_deref(),
        req.content.as_deref(),
        req.kind.as_deref(),
        req.salience,
        resolved_scope.as_deref(),
    ) {
        Ok(Some(entry)) => Json(entry).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("memory `{id}` not found")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_delete_memory(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.store.delete_memory(&id) {
        Ok(ok) => Json(json!({"deleted": ok})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_get_dream_status(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    Json(state.runtime.dream_status()).into_response()
}

#[derive(Deserialize, Default)]
pub struct RunDreamReq {
    #[serde(default)]
    force: bool,
}

pub async fn api_run_dream(
    State(state): State<WebState>,
    headers: HeaderMap,
    req: Option<Json<RunDreamReq>>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let force = req.map(|j| j.force).unwrap_or(true);
    match state.runtime.run_dream_now("webui", force).await {
        Ok(rec) => Json(rec).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    doc: Option<String>,
}

pub async fn api_list_memory_history(
    State(state): State<WebState>,
    Query(q): Query<HistoryQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.snapshots.list_doc_history(q.doc.as_deref()) {
        Ok(items) => Json(json!({ "snapshots": items })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct RollbackReq {
    snapshot_id: String,
}

pub async fn api_rollback_memory_doc(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(req): Json<RollbackReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.snapshots.rollback_doc(&req.snapshot_id) {
        Ok((doc, diff)) => Json(json!({"ok": true, "doc": doc, "diff": diff})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_get_automation(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let cfg = state.runtime.config();
    let jobs = state.runtime.store.list_cron_jobs().unwrap_or_default();
    let goals = state
        .runtime
        .store
        .list_goal_loops(GOAL_LOOP_LIST_DEFAULT_LIMIT)
        .unwrap_or_default();
    let triggers = state.runtime.list_triggers();
    Json(json!({
        "jobs": jobs,
        "goals": goals,
        "triggers": triggers,
        "webhooks_enabled": cfg.webhooks.enabled,
        "webhook_rules": cfg.webhooks.rules,
        "watchers": cfg.watchers,
        "policy": cfg.policy,
        "hooks": cfg.hooks,
    }))
    .into_response()
}

pub async fn api_list_goals(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state
        .runtime
        .store
        .list_goal_loops(GOAL_LOOP_LIST_DEFAULT_LIMIT)
    {
        Ok(goals) => Json(goals).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_create_goal(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(req): Json<GoalLoopCreateRequest>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.create_goal_loop(req) {
        Ok(goal) => (StatusCode::CREATED, Json(goal)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_get_goal(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.store.get_goal_loop(&id) {
        Ok(Some(goal)) => {
            let runs = state
                .runtime
                .store
                .list_goal_loop_runs(&goal.id, GOAL_LOOP_RUNS_DEFAULT_LIMIT)
                .unwrap_or_default();
            Json(json!({"goal": goal, "runs": runs})).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("goal loop `{id}` not found")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_update_goal(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(patch): Json<GoalLoopPatch>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.update_goal_loop(&id, patch) {
        Ok(Some(goal)) => Json(goal).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("goal loop `{id}` not found")})),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_run_goal_now(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match zenko_runtime::execute_goal_iteration(
        &state.runtime,
        &id,
        state.shutdown.child_token(),
    )
    .await
    {
        Ok(goal) => Json(goal).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_delete_goal(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.delete_goal_loop(&id) {
        Ok(deleted) => Json(json!({"deleted": deleted})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_list_triggers(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    Json(state.runtime.list_triggers()).into_response()
}

pub async fn api_upsert_webhook_rule(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(mut rule): Json<WebhookRuleConfig>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    rule.id = rule.id.trim().to_string();
    rule.source = rule.source.trim().to_string();
    if rule.id.is_empty() || rule.source.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "webhook rule `id` and `source` are required"})),
        )
            .into_response();
    }
    let rule_out = rule.clone();
    match state.runtime.update_config(|cfg| {
        if rule.enabled {
            cfg.webhooks.enabled = true;
        }
        match cfg
            .webhooks
            .rules
            .iter_mut()
            .find(|r| r.id.eq_ignore_ascii_case(&rule.id))
        {
            Some(existing) => *existing = rule,
            None => cfg.webhooks.rules.push(rule),
        }
    }) {
        Ok(updated) => Json(json!({
            "ok": true,
            "rule": rule_out,
            "webhook_rules": updated.webhooks.rules,
            "triggers": state.runtime.list_triggers(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_delete_webhook_rule(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let needle = id.trim().to_string();
    let mut removed = false;
    match state.runtime.update_config(|cfg| {
        let before = cfg.webhooks.rules.len();
        cfg.webhooks
            .rules
            .retain(|r| !r.id.eq_ignore_ascii_case(&needle));
        removed = cfg.webhooks.rules.len() < before;
    }) {
        Ok(updated) => Json(json!({
            "deleted": removed,
            "webhook_rules": updated.webhooks.rules,
            "triggers": state.runtime.list_triggers(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_upsert_watcher(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(mut watcher): Json<WorkspaceWatcherConfig>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    watcher.id = watcher.id.trim().to_string();
    if watcher.id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "workspace watcher `id` is required"})),
        )
            .into_response();
    }
    watcher.poll_interval_secs = watcher.poll_interval_secs.max(WATCHER_MIN_POLL_INTERVAL_SECS);
    let watcher_out = watcher.clone();
    match state.runtime.update_config(|cfg| {
        match cfg
            .watchers
            .iter_mut()
            .find(|w| w.id.eq_ignore_ascii_case(&watcher.id))
        {
            Some(existing) => *existing = watcher,
            None => cfg.watchers.push(watcher),
        }
    }) {
        Ok(updated) => Json(json!({
            "ok": true,
            "watcher": watcher_out,
            "watchers": updated.watchers,
            "triggers": state.runtime.list_triggers(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_delete_watcher(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let needle = id.trim().to_string();
    let mut removed = false;
    match state.runtime.update_config(|cfg| {
        let before = cfg.watchers.len();
        cfg.watchers.retain(|w| !w.id.eq_ignore_ascii_case(&needle));
        removed = cfg.watchers.len() < before;
    }) {
        Ok(updated) => Json(json!({
            "deleted": removed,
            "watchers": updated.watchers,
            "triggers": state.runtime.list_triggers(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct ExportQuery {
    format: Option<String>,
    include_thinking: Option<bool>,
    anonymize: Option<bool>,
    all_branches: Option<bool>,
    verified_only: Option<bool>,
    min_turns: Option<usize>,
    since: Option<String>,
    channel: Option<String>,
    limit: Option<usize>,
}

pub async fn api_export_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(q): Query<ExportQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let fmt = ExportFormat::from_str_loose(q.format.as_deref().unwrap_or("jsonl"));
    let opts = ExportOptions {
        format: fmt,
        include_thinking: q.include_thinking.unwrap_or(false),
        anonymize_paths: q.anonymize.unwrap_or(true),
        all_branches: q.all_branches.unwrap_or(false),
        ..ExportOptions::default()
    };
    match state.runtime.export_session_trajectory(&id, &opts) {
        Ok(rep) => {
            let filename = format!("zenko_{}_{}.{}", id, fmt.as_str(), fmt.file_ext());
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, fmt.content_type().to_string()),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("attachment; filename=\"{filename}\""),
                    ),
                ],
                rep.content,
            )
                .into_response()
        }
        Err(e) => (StatusCode::NOT_FOUND, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_export_batch(
    State(state): State<WebState>,
    Query(q): Query<ExportQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let fmt = ExportFormat::from_str_loose(q.format.as_deref().unwrap_or("jsonl"));
    let since = q.since.as_deref().and_then(parse_since_window);
    let opts = ExportOptions {
        format: fmt,
        include_thinking: q.include_thinking.unwrap_or(false),
        anonymize_paths: q.anonymize.unwrap_or(true),
        all_branches: q.all_branches.unwrap_or(false),
        verified_only: q.verified_only.unwrap_or(false),
        min_turns: q.min_turns.unwrap_or(0),
        since,
        channel: q.channel,
        limit: q.limit,
    };
    match state.runtime.export_batch_trajectories(&opts) {
        Ok(rep) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, fmt.content_type().to_string())],
            rep.content,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct CreateCronReq {
    name: String,
    cron_expr: String,
    kind: Option<String>,
    #[serde(default)]
    prompt: String,
    script_path: Option<String>,
    target_channel: Option<String>,
}

pub async fn api_create_cron(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(req): Json<CreateCronReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let parsed = match Cron::new(&req.cron_expr).parse() {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid cron expression: {e}")})),
            )
                .into_response()
        }
    };
    let kind = CronKind::from_str_loose(req.kind.as_deref().unwrap_or("cron"));
    let script_path = req
        .script_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    if kind == CronKind::Script && script_path.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "script_path is required for script cron jobs"})),
        )
            .into_response();
    }
    let next = parsed.find_next_occurrence(&Utc::now(), false).ok();
    let job = CronJob {
        id: new_id(),
        name: req.name,
        cron_expr: req.cron_expr,
        kind,
        prompt: req.prompt,
        script_path,
        target_channel: req.target_channel.unwrap_or_else(|| "webui".into()),
        target_chat_id: None,
        target_thread_id: None,
        enabled: true,
        next_run_at: next,
        last_run_at: None,
        last_status: None,
    };
    match state.runtime.store.upsert_cron_job(&job) {
        Ok(()) => {
            state.runtime.notify_cron_updated();
            Json(job).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UpdateCronReq {
    name: Option<String>,
    cron_expr: Option<String>,
    kind: Option<String>,
    prompt: Option<String>,
    script_path: Option<String>,
    enabled: Option<bool>,
}

pub async fn api_update_cron(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<UpdateCronReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let Some(mut job) = state.runtime.store.get_cron_job(&id).ok().flatten() else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("cron job `{id}` not found")})),
        )
            .into_response();
    };
    if let Some(n) = req.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        job.name = n.to_string();
    }
    if let Some(expr) = req.cron_expr.as_deref() {
        match Cron::new(expr).parse() {
            Ok(parsed) => {
                job.cron_expr = expr.to_string();
                job.next_run_at = parsed.find_next_occurrence(&Utc::now(), false).ok();
            }
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("Invalid cron expression: {e}")})),
                )
                    .into_response()
            }
        }
    }
    if let Some(k) = req.kind.as_deref() {
        job.kind = CronKind::from_str_loose(k);
    }
    if let Some(p) = req.prompt {
        job.prompt = p;
    }
    if let Some(sp) = req.script_path {
        let trimmed = sp.trim();
        job.script_path = match trimmed.is_empty() {
            true => None,
            false => Some(trimmed.to_string()),
        };
    }
    if let Some(en) = req.enabled {
        job.enabled = en;
    }
    match state.runtime.store.upsert_cron_job(&job) {
        Ok(()) => {
            state.runtime.notify_cron_updated();
            Json(job).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_run_cron_now(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.trigger_cron_job_now(&id).await {
        Ok(res) => Json(res).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_delete_cron(
    State(state): State<WebState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    match state.runtime.store.delete_cron_job(&id) {
        Ok(ok) => {
            state.runtime.notify_cron_updated();
            Json(json!({"deleted": ok})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_telemetry(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    Json(state.runtime.telemetry()).into_response()
}

/// Bounded iterative directory byte counter (stack-based, zero recursion, zero symlinks, capped work).
fn measure_dir_bounded(root: &std::path::Path, max_depth: usize, max_entries: usize) -> (u64, u64) {
    if !root.exists() {
        return (0, 0);
    }
    if root.is_file() {
        let len = std::fs::metadata(root).map(|m| m.len()).unwrap_or(0);
        return (len, 1);
    }
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut bytes = 0u64;
    let mut files = 0u64;
    let mut visited = 0usize;

    while let Some((dir, depth)) = stack.pop() {
        if depth > max_depth || visited >= max_entries {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            visited += 1;
            if visited > max_entries {
                break;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_file() {
                if let Ok(meta) = entry.metadata() {
                    bytes = bytes.saturating_add(meta.len());
                    files += 1;
                }
            } else if ft.is_dir() && depth < max_depth {
                stack.push((entry.path(), depth + 1));
            }
        }
    }
    (bytes, files)
}

pub async fn api_dashboard(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }

    let cfg = state.runtime.config();
    let telemetry = state.runtime.telemetry();
    let vault = state.runtime.vault.stats();
    let mut metrics = state.runtime.store.dashboard_db_metrics(365).unwrap_or_default();

    let snap = state.runtime.snapshots.load_snapshot().ok();
    if let Some(s) = &snap {
        metrics.counts.skills_total = s.skills.len() as u64;
        metrics.counts.skills_enabled = s.skills.iter().filter(|k| k.enabled).count() as u64;
    }
    metrics.counts.mcp_servers = state.runtime.mcp_clients.list_status().await.len() as u64;

    let data_dir = cfg.resolved_data_dir();
    let hub_dir = cfg.resolved_hub_dir();
    let logs_dir = state.runtime.log_sink.logs_dir().to_path_buf();
    let (db_bytes, db_files) = state.runtime.store.db_total_size_bytes();

    let storage_specs: [(&'static str, &'static str, std::path::PathBuf); 5] = [
        ("logs", "Flight Recorder Logs", logs_dir),
        ("checkpoints", "Checkpoints & Blobs", data_dir.join("checkpoints")),
        ("artifacts", "Spilled Artifacts", data_dir.join("artifacts")),
        ("skills", "Skills & Playbooks", data_dir.join("skills")),
        ("hub", "Zenko Hub Creations", hub_dir),
    ];

    let mut storage = vec![
        StorageSlice {
            id: "sqlite_db".into(),
            label: "SQLite Database (WAL)".into(),
            path: state.runtime.store.db_path().display().to_string(),
            bytes: db_bytes,
            files: db_files,
        },
        StorageSlice {
            id: "zenko_vault".into(),
            label: "Zenko Vault (ChaCha20)".into(),
            path: vault.path.clone(),
            bytes: vault.size_bytes,
            files: if vault.exists { 2 } else { 0 },
        },
    ];

    let scanned_slices = tokio::task::spawn_blocking(move || {
        let mut out = Vec::with_capacity(storage_specs.len() + 1);
        for (id, label, path) in storage_specs {
            let (bytes, files) = measure_dir_bounded(&path, 4, 8_000);
            out.push(StorageSlice {
                id: id.into(),
                label: label.into(),
                path: path.display().to_string(),
                bytes,
                files,
            });
        }
        // Also include top-level markdown memory snapshots (SOUL.md, USER.md, MEMORY.md, ZENKO.md)
        let mut md_bytes = 0u64;
        let mut md_files = 0u64;
        for name in ["SOUL.md", "USER.md", "MEMORY.md", "ZENKO.md"] {
            if let Ok(meta) = std::fs::metadata(data_dir.join(name)) {
                md_bytes = md_bytes.saturating_add(meta.len());
                md_files += 1;
            }
        }
        out.push(StorageSlice {
            id: "memory_docs".into(),
            label: "Prompt & Memory Docs".into(),
            path: data_dir.display().to_string(),
            bytes: md_bytes,
            files: md_files,
        });
        out
    })
    .await
    .unwrap_or_default();
    storage.extend(scanned_slices);

    let total_storage_bytes: u64 = storage.iter().map(|s| s.bytes).sum();

    Json(DashboardAnalytics {
        telemetry,
        vault,
        counts: metrics.counts,
        total_storage_bytes,
        storage,
        daily_activity: metrics.daily_activity,
        top_tools: metrics.top_tools,
        top_models: metrics.top_models,
        channels: metrics.channels,
        streak_days: metrics.streak_days,
        active_days: metrics.active_days,
        total_tokens: metrics.total_tokens,
        total_cache_read_tokens: metrics.total_cache_read_tokens,
        generated_at: Utc::now(),
    })
    .into_response()
}

pub async fn api_logs(
    State(state): State<WebState>,
    Query(mut filter): Query<LogFilter>,
    headers: HeaderMap,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    if filter.limit.is_none() {
        filter.limit = Some(200);
    }
    let rows = match state.runtime.store.query_logs(&filter) {
        Ok(list) if !list.is_empty() => list,
        Ok(_) => state
            .runtime
            .logs
            .recent(filter.limit.unwrap_or(200))
            .into_iter()
            .filter(|e| filter.matches(e))
            .collect(),
        Err(_) => state
            .runtime
            .logs
            .recent(filter.limit.unwrap_or(200))
            .into_iter()
            .filter(|e| filter.matches(e))
            .collect(),
    };
    Json(rows).into_response()
}

// ─── Signed Webhooks & OpenAI-Compatible Agent-to-Agent Endpoints ─────────────

pub async fn api_webhook_ingest(
    State(state): State<WebState>,
    Path(source): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let cfg = state.runtime.config();
    if !cfg.webhooks.enabled {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "webhooks disabled in zenko.toml"})),
        )
            .into_response();
    }
    let secret = match state.runtime.vault.get(VAULT_KEY_WEBHOOK_SECRET) {
        Ok(Some(s)) if !s.trim().is_empty() => s,
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "webhook HMAC secret not configured in vault"})),
            )
                .into_response();
        }
    };
    let sig = headers
        .get(WEBHOOK_SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_hmac_sha256(secret.as_str(), &body, sig) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid HMAC signature"})),
        )
            .into_response();
    }

    let payload_text = String::from_utf8_lossy(&body).to_string();
    let payload_json = serde_json::from_slice::<Value>(&body).ok();

    // Evaluate declarative `[[webhooks.rules]]` (zero-regex RFC 6901 JSON pointer matching + rate limits)
    let matched_rule = cfg
        .webhooks
        .rules
        .iter()
        .find(|r| match_webhook_rule(r, &source, payload_json.as_ref()))
        .cloned();

    if let Some(rule) = matched_rule {
        let limit_key = format!("webhook:{}", rule.id);
        if let Err(reason) = state.runtime.trigger_limiter.check_and_record(
            &limit_key,
            rule.cooldown_secs,
            rule.max_per_hour,
        ) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({
                    "accepted": false,
                    "rule_id": rule.id,
                    "error": reason,
                })),
            )
                .into_response();
        }
        if let Some(ref gid) = rule
            .goal_loop_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            match state.runtime.resume_and_wake_goal(gid) {
                Ok(Some(goal)) => {
                    return (
                        StatusCode::ACCEPTED,
                        Json(json!({
                            "accepted": true,
                            "rule_id": rule.id,
                            "goal_loop_id": goal.id,
                            "session_id": goal.session_id,
                        })),
                    )
                        .into_response();
                }
                Ok(None) => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({"error": format!("goal loop `{gid}` not found for rule `{}`", rule.id)})),
                    )
                        .into_response();
                }
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"error": e.to_string()})),
                    )
                        .into_response();
                }
            }
        }

        let prompt = render_webhook_prompt(&rule.prompt_template, &source, &rule.id, &payload_text);
        let sess_key = rule.session_key.as_deref().unwrap_or(&rule.id);
        let session = match state.runtime.store.find_or_create_channel_session(
            "webhook",
            sess_key,
            None,
            &format!("Webhook: {}", rule.id),
            cfg.policy.mode,
        ) {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": e.to_string()})),
                )
                    .into_response();
            }
        };
        let rt = state.runtime.clone();
        let sid = session.id.clone();
        let ws_override = rule
            .workspace_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(zenko_core::expand_tilde);
        let model_override = rule
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let gw_log = zenko_core::emit_hono_line(
            zenko_core::LogDirection::In,
            zenko_core::LogChannel::Gateway,
            &format!("webhook/{source} -> rule/{} ({}B)", rule.id, body.len()),
            Some(("202", zenko_core::LogStatusTone::Ok)),
            Some("routed"),
        )
        .with_session(sid.clone())
        .with_payload(json!({ "source": source, "rule_id": rule.id, "bytes": body.len() }));
        rt.logs.push(gw_log);
        tokio::spawn(async move {
            let _ = rt
                .run_turn_with_options(
                    &sid,
                    &prompt,
                    TurnOptions {
                        workspace_override: ws_override,
                        model_override,
                        headless_no_wait: !rt.is_telegram_connected(),
                        ..TurnOptions::default()
                    },
                )
                .await;
        });
        return (
            StatusCode::ACCEPTED,
            Json(json!({
                "accepted": true,
                "rule_id": rule.id,
                "session_id": session.id,
            })),
        )
            .into_response();
    }

    // If explicit routing rules exist for this source (or `*`) and none matched, ignore at 0 tokens!
    if has_webhook_rules_for_source(&cfg.webhooks.rules, &source) {
        let gw_log = zenko_core::emit_hono_line(
            zenko_core::LogDirection::In,
            zenko_core::LogChannel::Gateway,
            &format!("webhook/{source} ({}B) -> no rule matched", body.len()),
            Some(("200", zenko_core::LogStatusTone::Info)),
            Some("ignored"),
        )
        .with_payload(json!({ "source": source, "bytes": body.len(), "ignored": true }));
        state.runtime.logs.push(gw_log);
        return (
            StatusCode::OK,
            Json(json!({
                "accepted": false,
                "ignored": true,
                "source": source,
                "reason": "no matching webhook rule for source",
            })),
        )
            .into_response();
    }

    // Generic fallback (when no rules target this source) is still protected by cooldown & hourly cap.
    let fallback_key = format!("webhook_default:{source}");
    if let Err(reason) = state.runtime.trigger_limiter.check_and_record(
        &fallback_key,
        WEBHOOK_DEFAULT_COOLDOWN_SECS,
        WEBHOOK_DEFAULT_MAX_PER_HOUR,
    ) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "accepted": false,
                "source": source,
                "error": reason,
            })),
        )
            .into_response();
    }

    let prompt = format!("Incoming signed webhook from `{source}`:\n{payload_text}");
    let session = match state.runtime.store.find_or_create_channel_session(
        "webhook",
        &source,
        None,
        &format!("Webhook: {source}"),
        cfg.policy.mode,
    ) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };

    let rt = state.runtime.clone();
    let sid = session.id.clone();
    let gw_log = zenko_core::emit_hono_line(
        zenko_core::LogDirection::In,
        zenko_core::LogChannel::Gateway,
        &format!("webhook/{source} ({}B)", body.len()),
        Some(("202", zenko_core::LogStatusTone::Ok)),
        Some("verified"),
    )
    .with_session(sid.clone())
    .with_payload(json!({
        "source": source,
        "bytes": body.len(),
    }));
    rt.logs.push(gw_log);
    tokio::spawn(async move {
        let _ = rt
            .run_turn_with_options(
                &sid,
                &prompt,
                TurnOptions {
                    headless_no_wait: !rt.is_telegram_connected(),
                    ..TurnOptions::default()
                },
            )
            .await;
    });
    (
        StatusCode::ACCEPTED,
        Json(json!({"accepted": true, "session_id": session.id})),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct OpenAiMessage {
    role: String,
    content: Value,
}

#[derive(Deserialize)]
pub struct OpenAiChatReq {
    #[serde(default)]
    model: Option<String>,
    messages: Vec<OpenAiMessage>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    permission_mode: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
}

pub async fn api_openai_models(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let models = state.runtime.list_models_cached().await.unwrap_or_default();
    let data: Vec<Value> = models
        .into_iter()
        .map(|m| {
            json!({
                "id": m.id,
                "object": "model",
                "owned_by": "zenko",
                "context_length": m.context_length,
                "is_free": m.is_free,
            })
        })
        .collect();
    Json(json!({ "object": "list", "data": data })).into_response()
}

/// OpenAI-compatible `/v1/chat/completions` endpoint allowing external AI agents, scripts,
/// and SDKs to invoke Zenko's full autonomous ReAct runtime (tools, memory, sessions).
pub async fn api_openai_chat_completions(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(req): Json<OpenAiChatReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    let prompt = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| match &m.content {
            Value::String(s) => s.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            other => other.to_string(),
        })
        .unwrap_or_default();

    if prompt.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "missing user message content"}})),
        )
            .into_response();
    }

    let cfg = state.runtime.config();
    let perm_override = req
        .permission_mode
        .as_deref()
        .map(PermissionMode::from_str_loose);
    let sid = match req.session_id.or(req.user) {
        Some(existing) if state.runtime.store.get_session(&existing).ok().flatten().is_some() => {
            existing
        }
        Some(ext_key) => match state.runtime.store.find_or_create_channel_session(
            "api",
            &ext_key,
            None,
            &format!("API: {ext_key}"),
            perm_override.unwrap_or(cfg.policy.mode),
        ) {
            Ok(s) => s.id,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": {"message": e.to_string()}})),
                )
                    .into_response();
            }
        },
        None => match state.runtime.store.create_session(
            "Agent API Session",
            "api",
            None,
            None,
            perm_override.unwrap_or(cfg.policy.mode),
        ) {
            Ok(s) => s.id,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": {"message": e.to_string()}})),
                )
                    .into_response();
            }
        },
    };

    let model_override = req
        .model
        .filter(|m| !m.trim().is_empty() && m != "zenko" && m != "auto");

    match state
        .runtime
        .run_turn_with_options(
            &sid,
            &prompt,
            TurnOptions {
                parent_event_override: None,
                workspace_override: req.workspace.map(std::path::PathBuf::from),
                model_override,
                permission_override: perm_override,
                headless_no_wait: true,
                ..TurnOptions::default()
            },
        )
        .await
    {
        Ok(turn) => Json(json!({
            "id": format!("chatcmpl-{}", new_id()),
            "object": "chat.completion",
            "created": Utc::now().timestamp(),
            "model": turn.model,
            "zenko_session_id": turn.session_id,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": turn.reply
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": turn.usage.prompt_tokens,
                "completion_tokens": turn.usage.completion_tokens,
                "total_tokens": turn.usage.prompt_tokens + turn.usage.completion_tokens
            }
        }))
        .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": {"message": e.to_string(), "session_id": sid}})),
        )
            .into_response(),
    }
}

// ─── Phase 7: External MCP Client Servers, Pooled LSP & Pluggable Web Search ─

pub async fn api_list_mcp_servers(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
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
    headers: HeaderMap,
    Json(req): Json<UpsertMcpServerReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
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
        if !key.is_empty() && !val.is_empty() {
            if let Err(e) = state.runtime.vault.set(key, val) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("failed to save secret `{key}`: {e}")})),
                )
                    .into_response();
            }
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
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
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
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
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
    headers: HeaderMap,
    Path(name): Path<String>,
    body: Option<Json<ToggleMcpServerReq>>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
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

pub async fn api_lsp_status(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
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

pub async fn api_lsp_stop(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
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
    headers: HeaderMap,
    Json(req): Json<UpdateWebSearchReq>,
) -> Response {
    if let Some(r) = require_api_auth(&state, &headers) {
        return r;
    }
    if let Some(key) = req.brave_api_key.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if let Err(e) = state.runtime.vault.set(VAULT_KEY_BRAVE_SEARCH, key) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    }
    if let Some(key) = req.tavily_api_key.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if let Err(e) = state.runtime.vault.set(VAULT_KEY_TAVILY_SEARCH, key) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
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
            cfg.web_search.default_limit = lim.clamp(1, 20);
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
