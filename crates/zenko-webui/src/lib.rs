pub mod api;
pub mod middleware;

use axum::Router;
use axum::body::Body;
use axum::http::{StatusCode, Uri, header};
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use rust_embed::Embed;
use std::future::IntoFuture;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tower_http::compression::CompressionLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tracing::info;
use zenko_core::{DEFAULT_SHUTDOWN_GRACE_SECS, Result, ZenkoError};
use zenko_runtime::AgentRuntime;

#[derive(Embed)]
#[folder = "../../webui/dist"]
struct EmbeddedWebAssets;

#[derive(Clone)]
pub struct WebState {
    pub runtime: Arc<AgentRuntime>,
    pub telegram_reload: Arc<Notify>,
    pub shutdown: CancellationToken,
}

async fn serve_spa(uri: Uri) -> Response {
    let raw_path = uri.path().trim_start_matches('/');
    // Never mask missing `/api/*` or `/v1/*` endpoints with `200 OK index.html`
    if raw_path == "api"
        || raw_path.starts_with("api/")
        || raw_path == "v1"
        || raw_path.starts_with("v1/")
    {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({ "error": "not found" })),
        )
            .into_response();
    }
    let path = if raw_path.is_empty() {
        "index.html"
    } else {
        raw_path
    };

    if let Some(asset) = EmbeddedWebAssets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        let cache = if path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime.as_ref())
            .header(header::CACHE_CONTROL, cache)
            .body(Body::from(asset.data.into_owned()))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }

    // SPA client-route fallback to index.html
    if let Some(index) = EmbeddedWebAssets::get("index.html") {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(Body::from(index.data.into_owned()))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }

    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Zenko WebUI assets not built. Run `bun run build` inside webui/.",
    )
        .into_response()
}

pub fn build_router(state: WebState) -> Router {
    Router::new()
        // Core Health, Auth, Bootstrap & SSE
        .route("/api/health", get(api::api_health))
        .route("/api/login", post(api::api_login))
        .route("/api/bootstrap", get(api::api_bootstrap))
        .route("/api/events", get(api::api_events_sse))
        .route("/api/telemetry", get(api::api_telemetry))
        .route("/api/dashboard", get(api::api_dashboard))
        .route("/api/logs", get(api::api_logs))
        .route("/api/models", get(api::api_models))
        // Sessions & Approvals
        .route(
            "/api/sessions",
            get(api::api_list_sessions).post(api::api_create_session),
        )
        .route("/api/sessions/search", get(api::api_search_sessions))
        .route(
            "/api/sessions/{id}",
            get(api::api_get_session)
                .patch(api::api_update_session)
                .delete(api::api_delete_session),
        )
        .route("/api/sessions/{id}/clear", post(api::api_clear_session))
        .route("/api/sessions/{id}/messages", post(api::api_send_message))
        .route("/api/sessions/{id}/steer", post(api::api_steer_session))
        .route(
            "/api/sessions/{id}/background",
            post(api::api_background_session),
        )
        .route("/api/sessions/{id}/queue", get(api::api_list_session_queue))
        .route(
            "/api/sessions/{id}/queue/{qid}",
            delete(api::api_cancel_session_queue),
        )
        .route(
            "/api/sessions/{id}/queue/{qid}/steer",
            post(api::api_steer_queued_prompt),
        )
        .route("/api/sessions/{id}/stop", post(api::api_stop_session))
        .route("/api/sessions/{id}/branch", post(api::api_switch_branch))
        .route("/api/sessions/{id}/context", get(api::api_get_session_context))
        .route("/api/sessions/{id}/export", get(api::api_export_session))
        .route("/api/export", get(api::api_export_batch))
        // Phase 6: Durable SQLite Task DAG, Plan Approval & Shadow-Git Checkpoints / Rewind
        .route(
            "/api/sessions/{id}/tasks",
            get(api::api_list_session_tasks).post(api::api_create_session_task),
        )
        .route(
            "/api/sessions/{id}/tasks/{tid}",
            patch(api::api_update_session_task).delete(api::api_delete_session_task),
        )
        .route(
            "/api/sessions/{id}/tasks/{tid}/claim",
            post(api::api_claim_session_task),
        )
        .route(
            "/api/sessions/{id}/approve-plan",
            post(api::api_approve_session_plan),
        )
        .route(
            "/api/sessions/{id}/checkpoints",
            get(api::api_list_session_checkpoints),
        )
        .route(
            "/api/checkpoints/{cp_id}/diff",
            get(api::api_diff_checkpoint),
        )
        .route("/api/sessions/{id}/rewind", post(api::api_rewind_session))
        .route("/api/checkpoints/prune", post(api::api_prune_checkpoints))
        .route("/api/approvals/{id}", post(api::api_resolve_approval))
        // Autonomous Orchestrator & Interactive Clarifications
        .route("/api/clarifications", get(api::api_list_clarifications))
        .route("/api/clarifications/{id}", post(api::api_resolve_clarification))
        .route("/api/orchestrator/triage", post(api::api_orchestrator_triage))
        // Phase 5 / 5.5 Async Subagents, Worktrees & Inter-Agent Mailbox
        .route(
            "/api/agents",
            get(api::api_list_agents).post(api::api_spawn_agent),
        )
        .route("/api/agents/prune", post(api::api_prune_agent_worktrees))
        .route(
            "/api/agents/{id}",
            get(api::api_get_agent).delete(api::api_kill_agent),
        )
        .route("/api/agents/{id}/message", post(api::api_message_agent))
        .route("/api/agents/{id}/diff", get(api::api_agent_diff))
        .route("/api/agents/{id}/merge", post(api::api_merge_agent_worktree))
        .route(
            "/api/agents/{id}/discard",
            post(api::api_discard_agent_worktree),
        )
        // Unified ZenkoTasks Control Plane (full CRUD; `/api/tasks` mirrors `/api/jobs`)
        .route(
            "/api/jobs",
            get(api::api_list_jobs).post(api::api_start_job),
        )
        .route(
            "/api/tasks",
            get(api::api_list_jobs).post(api::api_start_job),
        )
        .route("/api/jobs/prune", post(api::api_prune_jobs))
        .route("/api/tasks/prune", post(api::api_prune_jobs))
        .route(
            "/api/jobs/{id}",
            patch(api::api_update_job).delete(api::api_delete_job),
        )
        .route(
            "/api/tasks/{id}",
            patch(api::api_update_job).delete(api::api_delete_job),
        )
        .route("/api/jobs/{id}/output", get(api::api_read_job_output))
        .route("/api/tasks/{id}/output", get(api::api_read_job_output))
        .route("/api/jobs/{id}/clear", post(api::api_clear_job_output))
        .route("/api/tasks/{id}/clear", post(api::api_clear_job_output))
        .route("/api/jobs/{id}/wait", post(api::api_wait_job))
        .route("/api/tasks/{id}/wait", post(api::api_wait_job))
        .route("/api/jobs/{id}/input", post(api::api_job_input))
        .route("/api/tasks/{id}/input", post(api::api_job_input))
        .route("/api/jobs/{id}/stdin", post(api::api_job_input))
        .route("/api/tasks/{id}/stdin", post(api::api_job_input))
        .route("/api/jobs/{id}/restart", post(api::api_restart_job))
        .route("/api/tasks/{id}/restart", post(api::api_restart_job))
        .route("/api/jobs/{id}/stop", post(api::api_stop_job))
        .route("/api/tasks/{id}/stop", post(api::api_stop_job))
        // Zenko Hub (`~/zenko_hub/` creations browser)
        .route("/api/hub", get(api::api_list_hub))
        // Component Customization Engine
        .route("/api/ui/active", put(api::api_update_active_ui))
        .route("/api/ui/preset/{preset_id}", post(api::api_apply_ui_preset))
        .route("/api/ui/profiles", post(api::api_create_ui_profile))
        .route(
            "/api/ui/profiles/{id}/activate",
            post(api::api_activate_ui_profile),
        )
        // Setup & Gateways
        .route("/api/setup", post(api::api_save_setup))
        .route(
            "/api/setup/verify-provider",
            post(api::api_verify_provider),
        )
        .route(
            "/api/setup/verify-openrouter",
            post(api::api_verify_provider),
        )
        .route("/api/setup/verify-telegram", post(api::api_verify_telegram))
        .route(
            "/api/gateways/telegram",
            get(api::api_get_telegram).post(api::api_update_telegram),
        )
        .route(
            "/api/gateways/telegram/pair",
            post(api::api_create_telegram_pair),
        )
        .route(
            "/api/gateways/telegram/peers/{peer_id}",
            delete(api::api_delete_telegram_peer),
        )
        // Memory, Skills & Automation
        .route("/api/memory", get(api::api_get_memory))
        .route("/api/memory/doc/{name}", put(api::api_save_doc))
        .route("/api/memory/entries", post(api::api_add_memory))
        .route(
            "/api/memory/entries/{id}",
            patch(api::api_update_memory).delete(api::api_delete_memory),
        )
        .route(
            "/api/memory/dream",
            get(api::api_get_dream_status).post(api::api_run_dream),
        )
        .route("/api/memory/history", get(api::api_list_memory_history))
        .route("/api/memory/rollback", post(api::api_rollback_memory_doc))
        .route("/api/skills", get(api::api_list_skills))
        .route("/api/skills/history", get(api::api_list_skill_history))
        .route("/api/skills/rollback", post(api::api_rollback_skill))
        .route(
            "/api/skills/{slug}",
            put(api::api_save_skill).delete(api::api_delete_skill),
        )
        .route("/api/skills/{slug}/approve", post(api::api_approve_skill))
        .route("/api/skills/{slug}/toggle", post(api::api_toggle_skill))
        .route("/api/automation", get(api::api_get_automation))
        .route(
            "/api/goals",
            get(api::api_list_goals).post(api::api_create_goal),
        )
        .route(
            "/api/goals/{id}",
            get(api::api_get_goal)
                .patch(api::api_update_goal)
                .delete(api::api_delete_goal),
        )
        .route("/api/goals/{id}/run", post(api::api_run_goal_now))
        .route("/api/triggers", get(api::api_list_triggers))
        .route("/api/triggers/webhooks", post(api::api_upsert_webhook_rule))
        .route(
            "/api/triggers/webhooks/{id}",
            delete(api::api_delete_webhook_rule),
        )
        .route("/api/triggers/watchers", post(api::api_upsert_watcher))
        .route(
            "/api/triggers/watchers/{id}",
            delete(api::api_delete_watcher),
        )
        .route("/api/cron", post(api::api_create_cron))
        .route(
            "/api/cron/{id}",
            patch(api::api_update_cron).delete(api::api_delete_cron),
        )
        .route("/api/cron/{id}/run", post(api::api_run_cron_now))
        // Phase 7: External MCP Client Servers, Pooled LSP & Pluggable Web Search
        .route(
            "/api/mcp/servers",
            get(api::api_list_mcp_servers).post(api::api_upsert_mcp_server),
        )
        .route(
            "/api/mcp/servers/{name}",
            delete(api::api_delete_mcp_server),
        )
        .route(
            "/api/mcp/servers/{name}/sync",
            post(api::api_sync_mcp_server),
        )
        .route(
            "/api/mcp/servers/{name}/toggle",
            post(api::api_toggle_mcp_server),
        )
        .route("/api/lsp/status", get(api::api_lsp_status))
        .route("/api/lsp/stop", post(api::api_lsp_stop))
        .route("/api/web-search", put(api::api_update_web_search))
        // Signed Webhooks & OpenAI-Compatible Agent-to-Agent API
        .route("/api/webhooks/{source}", post(api::api_webhook_ingest))
        .route("/v1/models", get(api::api_openai_models))
        .route(
            "/v1/chat/completions",
            post(api::api_openai_chat_completions),
        )
        // Embedded Static SPA Fallback (zero Node.js at runtime)
        .fallback(serve_spa)
        .layer(from_fn_with_state(
            state.clone(),
            middleware::hono_logger_middleware,
        ))
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024))
        .layer(CompressionLayer::new())
        .with_state(state)
}

pub async fn serve(
    runtime: Arc<AgentRuntime>,
    telegram_reload: Arc<Notify>,
    cancel: CancellationToken,
) -> Result<()> {
    let cfg = runtime.config();
    let addr_str = format!("{}:{}", cfg.webui.bind, cfg.webui.port);
    let addr: SocketAddr = addr_str
        .parse()
        .map_err(|e| ZenkoError::Config(format!("invalid webui bind `{addr_str}`: {e}")))?;

    let state = WebState {
        runtime,
        telegram_reload,
        shutdown: cancel.clone(),
    };
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(url = %format!("http://{addr}"), "zenko WebUI listening (embedded static bundle)");

    let srv_cancel = cancel.clone();
    let mut server = Box::pin(
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                srv_cancel.cancelled().await;
            })
            .into_future(),
    );

    let grace = Duration::from_secs(DEFAULT_SHUTDOWN_GRACE_SECS);
    tokio::select! {
        res = &mut server => {
            res.map_err(|e| ZenkoError::Gateway(format!("webui server error: {e}")))?;
        }
        _ = cancel.cancelled() => {
            if let Ok(res) = tokio::time::timeout(grace, &mut server).await {
                res.map_err(|e| ZenkoError::Gateway(format!("webui server error: {e}")))?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use futures_util::StreamExt;
    use tower::ServiceExt;
    use zenko_core::ZenkoConfig;

    #[tokio::test]
    async fn webui_serves_embedded_spa_and_api() {
        let dir = std::env::temp_dir().join(format!("zenko_web_test_{}", zenko_core::new_id()));
        let mut cfg = ZenkoConfig::default();
        cfg.general.data_dir = dir.display().to_string();
        cfg.general.hub_dir = dir.join("hub").display().to_string();
        cfg.general.workspace_dir = dir.display().to_string();
        cfg.general.setup_completed = true;
        cfg.webui.require_auth = false;

        let (rt, _logs) =
            AgentRuntime::build(dir.join("zenko.toml"), cfg).expect("build runtime");
        let shutdown = CancellationToken::new();
        let app = build_router(WebState {
            runtime: rt.clone(),
            telegram_reload: Arc::new(Notify::new()),
            shutdown: shutdown.clone(),
        });

        for path in [
            "/",
            "/api/health",
            "/api/bootstrap",
            "/api/dashboard",
            "/api/gateways/telegram",
            "/api/memory",
            "/api/memory/dream",
            "/api/memory/history",
            "/api/sessions/search?q=test",
            "/api/automation",
            "/api/logs",
            "/api/jobs",
            "/api/tasks",
            "/api/hub",
            "/api/clarifications",
            "/api/agents",
            "/api/models",
            "/v1/models",
        ] {
            let resp = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).expect("req"))
                .await
                .expect("resp");
            assert_eq!(resp.status(), StatusCode::OK, "path {path} failed");
        }

        let preset_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/ui/preset/pure_oled?mode=light")
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("preset resp");
        assert_eq!(preset_resp.status(), StatusCode::OK);
        let active = rt.store.get_active_ui_profile().expect("active profile");
        assert_eq!(active.config.preset_id, "pure_oled");
        assert_eq!(active.config.mode, zenko_core::ui::ThemeMode::Light);

        let dark_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/ui/preset/pure_oled?mode=dark")
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("dark preset resp");
        assert_eq!(dark_resp.status(), StatusCode::OK);
        let active_dark = rt.store.get_active_ui_profile().expect("active profile");
        assert_eq!(active_dark.config.preset_id, "pure_oled");
        assert_eq!(active_dark.config.mode, zenko_core::ui::ThemeMode::Dark);

        // Switch active provider to MiniMax via /api/setup and confirm vault + catalog models
        let setup_body = serde_json::json!({
            "provider_kind": "minimax",
            "api_key": "mm-test-key-123",
            "base_url": "https://api.minimax.io/v1",
            "lead_model": "MiniMax-M3.1-Flash",
            "fast_model": "MiniMax-M2.7",
            "fallbacks": [],
            "telegram_bot_token": null,
            "telegram_enabled": false,
            "workspace_dir": dir.display().to_string(),
            "daily_spend_limit_usd": 5.0,
            "policy_mode": "balanced"
        });
        let setup_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/setup")
                    .header("content-type", "application/json")
                    .body(Body::from(setup_body.to_string()))
                    .expect("setup req"),
            )
            .await
            .expect("setup resp");
        assert_eq!(setup_resp.status(), StatusCode::OK);
        assert_eq!(rt.config().provider.kind, "minimax");
        assert_eq!(rt.config().provider.lead_model, "MiniMax-M3.1-Flash");
        assert!(rt.vault.has(zenko_core::VAULT_KEY_MINIMAX).expect("vault"));

        // Verify signed webhook route rejects disabled / unsigned requests cleanly
        let wh_disabled = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webhooks/github")
                    .body(Body::from(r#"{"event":"push"}"#))
                    .expect("wh req"),
            )
            .await
            .expect("wh resp");
        assert_eq!(wh_disabled.status(), StatusCode::FORBIDDEN);

        rt.update_config(|c| c.webhooks.enabled = true)
            .expect("enable webhooks");
        rt.vault
            .set(zenko_core::VAULT_KEY_WEBHOOK_SECRET, "wh-secret-123")
            .expect("set wh secret");
        let wh_unsigned = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webhooks/github")
                    .body(Body::from(r#"{"event":"push"}"#))
                    .expect("wh unsigned"),
            )
            .await
            .expect("wh unsigned resp");
        assert_eq!(wh_unsigned.status(), StatusCode::UNAUTHORIZED);

        // Regression test: missing /api/* and /v1/* routes must return JSON 404 (not 200 index.html)
        for missing_uri in ["/api/does-not-exist", "/v1/unknown"] {
            let miss = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(missing_uri)
                        .body(Body::empty())
                        .expect("miss req"),
                )
                .await
                .expect("miss resp");
            assert_eq!(miss.status(), StatusCode::NOT_FOUND, "expected 404 for {missing_uri}");
        }

        // Regression test: /api/mcp/servers rejects unreferenced and reserved vault secret overwrites
        let bad_unreferenced = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/mcp/servers")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"name":"demo","transport":"stdio","command":"echo","secret_values":{"WEBUI_ADMIN_TOKEN":"evil"}}"#,
                    ))
                    .expect("mcp bad req"),
            )
            .await
            .expect("mcp bad resp");
        assert_eq!(bad_unreferenced.status(), StatusCode::BAD_REQUEST);

        let bad_reserved = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/mcp/servers")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"name":"demo","transport":"stdio","command":"echo","env_secrets":{"X":"WEBUI_ADMIN_TOKEN"},"secret_values":{"WEBUI_ADMIN_TOKEN":"evil"}}"#,
                    ))
                    .expect("mcp reserved req"),
            )
            .await
            .expect("mcp reserved resp");
        assert_eq!(bad_reserved.status(), StatusCode::FORBIDDEN);

        // Regression test: an open /api/events SSE stream must terminate immediately on shutdown
        let sse_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/events")
                    .body(Body::empty())
                    .expect("sse req"),
            )
            .await
            .expect("sse resp");
        assert_eq!(sse_resp.status(), StatusCode::OK);
        let mut body_stream = sse_resp.into_body().into_data_stream();
        shutdown.cancel();
        let ended = tokio::time::timeout(Duration::from_millis(500), body_stream.next())
            .await
            .expect("SSE stream must close promptly on shutdown cancel");
        assert!(ended.is_none());

        let _ = std::fs::remove_dir_all(dir);
    }
}
