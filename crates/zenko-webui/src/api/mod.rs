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
    AgentEvent, AgentMessageOptions, ApprovalDecision, CHECKPOINT_LIST_DEFAULT_LIMIT,
    CONCISE_MODE_REGISTRY, ConciseMode, CronJob, CronKind, DEFAULT_PAIRING_TTL_MINS,
    DEFAULT_SSE_KEEPALIVE_SECS, DashboardAnalytics,
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


mod agents;
mod auth;
mod mcp_lsp;
mod memory_skills_automation;
mod openai_webhooks;
mod sessions;
mod setup_gateways;
mod tasks_checkpoints;
mod ui_models;

pub use agents::*;
pub use auth::*;
pub use mcp_lsp::*;
pub use memory_skills_automation::*;
pub use openai_webhooks::*;
pub use sessions::*;
pub use setup_gateways::*;
pub use tasks_checkpoints::*;
pub use ui_models::*;
