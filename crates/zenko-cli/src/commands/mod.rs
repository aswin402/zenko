//! Terminal command implementations designed equally for humans AND AI agents
//! (`--json`, `--stream-json` NDJSON, stdin piping, per-call overrides, REPL chat,
//! session/approval/memory/cron/log management, and deterministic exit codes).

use crate::ipc::{
    OutputMode, ipc_attach_session, ipc_call, ipc_call_timeout, ipc_follow_job, ipc_follow_logs,
    is_daemon_offline, unwrap_ipc_result,
};
use anyhow::{Context, Result};
use chrono::Utc;
use croner::Cron;
use serde_json::{Value, json};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    AgentEvent, ApprovalDecision, BackgroundJobStatus, CronJob, CronKind, DEFAULT_PAIRING_TTL_MINS,
    EXIT_DAEMON_STOPPED, EXIT_OK, HUB_CATEGORIES, IPC_DAEMON_PROBE_TIMEOUT_MS, IPC_FRAME_CHUNK,
    IPC_FRAME_DONE, IPC_FRAME_EVENT, IPC_FRAME_SNAPSHOT, IPC_METHOD_AGENTS_DIFF,
    IPC_METHOD_AGENTS_DISCARD, IPC_METHOD_AGENTS_GET, IPC_METHOD_AGENTS_KILL,
    IPC_METHOD_AGENTS_LIST, IPC_METHOD_AGENTS_MERGE, IPC_METHOD_AGENTS_MESSAGE,
    IPC_METHOD_AGENTS_PRUNE, IPC_METHOD_AGENTS_SPAWN, IPC_METHOD_AGENTS_WAIT,
    IPC_METHOD_APPROVAL_RESOLVE,
    IPC_METHOD_APPROVALS_LIST, IPC_METHOD_ASK, IPC_METHOD_CHECKPOINTS_DIFF,
    IPC_METHOD_CHECKPOINTS_LIST, IPC_METHOD_CHECKPOINTS_PRUNE, IPC_METHOD_CHECKPOINTS_REWIND,
    IPC_METHOD_CLARIFICATION_RESOLVE, IPC_METHOD_CLARIFICATIONS_LIST, IPC_METHOD_CRON_CREATE,
    IPC_METHOD_CRON_DELETE, IPC_METHOD_CRON_LIST, IPC_METHOD_CRON_RUN_NOW, IPC_METHOD_CRON_UPDATE,
    IPC_METHOD_EXPORT_SESSION, IPC_METHOD_GOAL_CREATE, IPC_METHOD_GOAL_DELETE, IPC_METHOD_GOAL_GET,
    IPC_METHOD_GOAL_RUN_NOW, IPC_METHOD_GOAL_UPDATE, IPC_METHOD_GOALS_LIST, IPC_METHOD_HUB_LIST,
    IPC_METHOD_JOBS_DELETE, IPC_METHOD_JOBS_INPUT, IPC_METHOD_JOBS_LIST, IPC_METHOD_JOBS_PRUNE,
    IPC_METHOD_JOBS_READ, IPC_METHOD_JOBS_RESTART, IPC_METHOD_JOBS_START, IPC_METHOD_JOBS_STOP,
    IPC_METHOD_JOBS_UPDATE, IPC_METHOD_JOBS_WAIT, IPC_METHOD_LOGS_QUERY, IPC_METHOD_LSP_STATUS,
    IPC_METHOD_LSP_STOP, IPC_METHOD_MCP_SERVER_DELETE, IPC_METHOD_MCP_SERVER_SYNC,
    IPC_METHOD_MCP_SERVER_UPSERT, IPC_METHOD_MCP_SERVERS_LIST, IPC_METHOD_MEMORY_ADD,
    IPC_METHOD_MEMORY_DELETE, IPC_METHOD_MEMORY_DREAM, IPC_METHOD_MEMORY_HISTORY,
    IPC_METHOD_MEMORY_ROLLBACK, IPC_METHOD_MEMORY_SEARCH, IPC_METHOD_MEMORY_UPDATE,
    IPC_METHOD_SESSION_APPROVE_PLAN, IPC_METHOD_SESSION_BACKGROUND, IPC_METHOD_SESSION_CLEAR,
    IPC_METHOD_SESSION_CONTEXT, IPC_METHOD_SESSION_DELETE, IPC_METHOD_SESSION_GET,
    IPC_METHOD_SESSION_QUEUE_CANCEL, IPC_METHOD_SESSION_QUEUE_LIST, IPC_METHOD_SESSION_STEER,
    IPC_METHOD_SESSIONS_LIST, IPC_METHOD_SESSIONS_SEARCH, IPC_METHOD_SKILL_APPROVE,
    IPC_METHOD_SKILL_GET, IPC_METHOD_SKILL_HISTORY, IPC_METHOD_SKILL_MANAGE,
    IPC_METHOD_SKILL_ROLLBACK, IPC_METHOD_SKILLS_LIST, IPC_METHOD_STATUS,
    IPC_METHOD_TASKS_DAG_CREATE, IPC_METHOD_TASKS_DAG_DELETE, IPC_METHOD_TASKS_DAG_LIST,
    IPC_METHOD_TASKS_DAG_UPDATE, IPC_METHOD_TRIGGERS_LIST, IPC_WAIT_TIMEOUT_SLACK_SECS,
    JOB_DEFAULT_READ_BYTES, JOB_DEFAULT_WAIT_SECS, LSP_SERVER_REGISTRY, LlmProvider, LogChannel,
    LogDirection, LogEntry, LogFilter, LogStatusTone, MEMORY_SCOPE_GLOBAL, McpServerConfig,
    McpTransport, OrchestratorBridge, PeerRole, PermissionMode, RewindMode, RewindRequest,
    RiskLevel, SUBAGENT_DEFAULT_WAIT_SECS, SUBAGENT_MAX_WAIT_SECS, SecretVault,
    SessionSearchFilter, SessionStatus, SessionTask, SessionTaskPatch, SessionTaskStatus,
    SkillAuthor, SkillStatus, SubagentSpawnRequest, SubagentWorkspaceMode, TaskKind,
    VAULT_KEY_TELEGRAM, VAULT_KEY_WEBUI_ADMIN, ZenkoConfig, find_provider, format_hono_entry,
    format_tokens, new_id,
    ExportFormat, ExportOptions, GOAL_LOOP_IPC_TIMEOUT_SECS, GOAL_LOOP_RUNS_DEFAULT_LIMIT,
    GoalLoopCreateRequest, GoalLoopPatch, GoalLoopStatus,
};
use zenko_gateways::verify_bot_token;
use zenko_memory::{
    SnapshotManager, SqliteStore, parse_since_window, project_scope_for_workspace, resolve_git_root,
};
use zenko_providers::UnifiedLlmProvider;
use zenko_runtime::{AgentRuntime, TurnOptions};


mod agents;
mod ask_chat;
mod dag_checkpoints;
mod economy;
mod jobs_hub_queue;
mod mcp_lsp;
mod sessions_memory;
mod skills_goals_export;

pub use agents::*;
pub use ask_chat::*;
pub use dag_checkpoints::*;
pub use economy::*;
pub use jobs_hub_queue::*;
pub use mcp_lsp::*;
pub use sessions_memory::*;
pub use skills_goals_export::*;
use sessions_memory::print_log_entry_line;
