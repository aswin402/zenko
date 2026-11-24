use crate::agents::{
    SubagentSupervisor, extract_json_for_schema, format_output_schema_instruction,
    validate_json_schema,
};
use crate::artifacts;
use crate::checkpoint::CheckpointManager;
use crate::compaction::{
    CheckpointPlan, build_compaction_prompt, build_deterministic_checkpoint,
    compact_messages_to_budget, estimate_messages_tokens, estimate_text_tokens,
    estimate_tool_specs_tokens, microcompact_messages, plan_checkpoint_split,
    prepend_checkpoint_to_messages, slice_from_latest_checkpoint,
};
use crate::file_state::SessionFileTracker;
use crate::jobs::LocalJobSupervisor;
use crate::log_store::{CategorizedLogSink, install_panic_hook, wire_debug_enabled};
use crate::orchestrator::{
    self, GuardVerdict, PhaseGateDecision, SharedClarificationHub, detect_text_doom_loop,
    escalate_on_pivot, execution_plan_from_session_tasks, format_plan_summary,
    format_session_tasks_prompt_block, is_read_only_shell_command, load_latest_active_plan,
};
use crate::orchestrator::brainstorm::ClarificationHub;
use crate::orchestrator::triage::{TriageInput, classify_turn};
use crate::policy::{
    evaluate_verify_command_safety, is_tool_allowed_in_plan_schema, run_hooks,
};
use crate::scheduler::execute_tool_waves;
use crate::telemetry::{LogBuffer, current_rss_bytes};
use crate::tool_router;
use async_trait::async_trait;
use chrono::Utc;
use croner::Cron;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex as AsyncMutex, Notify, broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use zenko_core::{
    APPROVAL_RESOLVER_CANCELLED, APPROVAL_RESOLVER_STEERED, APPROVAL_RESOLVER_TIMEOUT,
    ATTACH_DEFAULT_REPLAY_EVENTS, AgentEvent, ApprovalDecision, ApprovalRecord, ApprovalStatus,
    BackgroundJobRecord, BackgroundJobStatus, BatchToolRequest, BatchToolStepOutcome,
    CHECKPOINT_LABEL_MAX_CHARS, CHECKPOINT_LIST_DEFAULT_LIMIT, COMPACTION_MAX_CHECKPOINT_TOKENS, ChatMessage, ChatRequest,
    ClarificationOption, CompactionCheckpointPayload, ContextBreakdown, ContextBridge, CronJob,
    CronKind, DEFAULT_APPROVAL_TIMEOUT_SECS, DEFAULT_CONTEXT_WINDOW_FALLBACK,
    DEFAULT_EVENT_BROADCAST_CAPACITY, DEFAULT_LOG_BUFFER_CAPACITY, DEFAULT_PAIRING_TTL_MINS,
    DaemonTelemetry, DreamRunRecord, DreamStatus, ESCALATION_OPT_PIVOT, ESCALATION_OPT_STOP,
    EXTERNAL_TOOL_REAPER_INTERVAL_SECS, EventKind, EventRole, ExecutionPlan, ExportOptions,
    ExportReport, FILE_MODE_OWNER_ONLY, FileStateBridge, GOAL_LOOP_DEFAULT_INTERVAL_SECS,
    GOAL_LOOP_DEFAULT_MAX_BUDGET_USD, GOAL_LOOP_DEFAULT_MAX_ITERATIONS,
    GOAL_LOOP_DEFAULT_NOTIFY_CHANNEL, GOAL_LOOP_DEFAULT_STALL_LIMIT, GOAL_LOOP_LIST_DEFAULT_LIMIT,
    GOAL_LOOP_MAX_ITERATIONS_CAP, GOAL_LOOP_MAX_STALL_CAP, GOAL_LOOP_RUNS_DEFAULT_LIMIT,
    GoalLoopBridge, GoalLoopCreateRequest, GoalLoopPatch, GoalLoopRecord, GoalLoopRun,
    GoalLoopStatus, HUB_CATEGORIES, HUB_SUBDIR_SCRIPTS, HookEvent, HubEntry,
    IDLE_DB_COMPACT_EVERY_TICKS, IPC_FRAME_CHUNK, IPC_FRAME_DONE,
    IPC_FRAME_EVENT, IPC_FRAME_SNAPSHOT, IPC_METHOD_AGENTS_DIFF, IPC_METHOD_AGENTS_DISCARD,
    IPC_METHOD_AGENTS_GET, IPC_METHOD_AGENTS_KILL, IPC_METHOD_AGENTS_LIST, IPC_METHOD_AGENTS_MERGE,
    IPC_METHOD_AGENTS_MESSAGE, IPC_METHOD_AGENTS_PRUNE, IPC_METHOD_AGENTS_SPAWN,
    IPC_METHOD_AGENTS_WAIT, IPC_METHOD_APPROVAL_RESOLVE, IPC_METHOD_APPROVALS_LIST, IPC_METHOD_ASK,
    IPC_METHOD_CHECKPOINTS_DIFF, IPC_METHOD_CHECKPOINTS_LIST, IPC_METHOD_CHECKPOINTS_PRUNE,
    IPC_METHOD_CHECKPOINTS_REWIND, IPC_METHOD_CLARIFICATION_RESOLVE,
    IPC_METHOD_CLARIFICATIONS_LIST, IPC_METHOD_CRON_CREATE, IPC_METHOD_CRON_DELETE,
    IPC_METHOD_CRON_LIST, IPC_METHOD_CRON_RUN_NOW, IPC_METHOD_CRON_UPDATE,
    IPC_METHOD_EXPORT_SESSION, IPC_METHOD_GOAL_CREATE, IPC_METHOD_GOAL_DELETE, IPC_METHOD_GOAL_GET,
    IPC_METHOD_GOAL_RUN_NOW, IPC_METHOD_GOAL_UPDATE, IPC_METHOD_GOALS_LIST, IPC_METHOD_HUB_LIST,
    IPC_METHOD_JOBS_DELETE, IPC_METHOD_JOBS_FOLLOW, IPC_METHOD_JOBS_INPUT, IPC_METHOD_JOBS_LIST,
    IPC_METHOD_JOBS_PRUNE, IPC_METHOD_JOBS_READ, IPC_METHOD_JOBS_RESTART, IPC_METHOD_JOBS_START,
    IPC_METHOD_JOBS_STOP, IPC_METHOD_JOBS_UPDATE, IPC_METHOD_JOBS_WAIT, IPC_METHOD_LOGS_FOLLOW,
    IPC_METHOD_LOGS_QUERY, IPC_METHOD_LOGS_RECENT, IPC_METHOD_LOGS_STATS, IPC_METHOD_LSP_STATUS,
    IPC_METHOD_LSP_STOP, IPC_METHOD_MCP_SERVER_DELETE, IPC_METHOD_MCP_SERVER_SYNC,
    IPC_METHOD_MCP_SERVER_UPSERT, IPC_METHOD_MCP_SERVERS_LIST, IPC_METHOD_MEMORY_ADD,
    IPC_METHOD_MEMORY_DELETE, IPC_METHOD_MEMORY_DREAM, IPC_METHOD_MEMORY_HISTORY,
    IPC_METHOD_MEMORY_ROLLBACK, IPC_METHOD_MEMORY_SEARCH, IPC_METHOD_MEMORY_UPDATE,
    IPC_METHOD_ORCHESTRATOR_TRIAGE, IPC_METHOD_PAIR, IPC_METHOD_SESSION_APPROVE_PLAN,
    IPC_METHOD_SESSION_ATTACH, IPC_METHOD_SESSION_BACKGROUND, IPC_METHOD_SESSION_CLEAR,
    IPC_METHOD_SESSION_CONTEXT, IPC_METHOD_SESSION_DELETE, IPC_METHOD_SESSION_GET,
    IPC_METHOD_SESSION_QUEUE_CANCEL, IPC_METHOD_SESSION_QUEUE_LIST, IPC_METHOD_SESSION_STEER,
    IPC_METHOD_SESSIONS_LIST, IPC_METHOD_SESSIONS_SEARCH, IPC_METHOD_SKILL_APPROVE,
    IPC_METHOD_SKILL_GET, IPC_METHOD_SKILL_HISTORY, IPC_METHOD_SKILL_MANAGE,
    IPC_METHOD_SKILL_ROLLBACK, IPC_METHOD_SKILLS_LIST, IPC_METHOD_STATUS, IPC_METHOD_STOP,
    IPC_METHOD_TASKS_DAG_CREATE, IPC_METHOD_TASKS_DAG_DELETE, IPC_METHOD_TASKS_DAG_LIST,
    IPC_METHOD_TASKS_DAG_UPDATE, IPC_METHOD_TRIGGERS_LIST, JOB_DEFAULT_LIST_LIMIT,
    JOB_DEFAULT_READ_BYTES, JOB_DEFAULT_WAIT_SECS, JOB_MAX_WAIT_SECS,
    JOB_NOTICE_CMD_PREVIEW_CHARS, JOB_WAKE_COALESCE_MS, JobSupervisor, LOG_CATEGORIES, LlmDelta,
    LlmProvider, LogFilter, LspConfig, MAX_PARALLEL_TOOLS, MEMORY_AUTO_RECALL_LIMIT,
    MEMORY_AUTO_RECALL_MIN_SCORE, MEMORY_SCOPE_GLOBAL, MODEL_CACHE_TTL_SECS,
    MODEL_KV_COMPACT_REGISTRY_KEY, MODEL_KV_LEGACY_COMPACT_THRESHOLD_BYTES,
    MODEL_KV_PROVIDER_PREFIX, MODEL_KV_REGISTRY_KEY, MUTATING_FILE_TOOLS, McpServerConfig,
    McpServerStatus, ModelInfo, ModelRoutingRole, NOTE_KIND_BRAINSTORM_SPEC,
    NOTE_KIND_CLARIFICATION_DECISION, NOTE_KIND_DEFERRED_TOOLS, NOTE_KIND_EXECUTION_PLAN,
    NOTE_KIND_MEMORY_RECALL, NOTE_KIND_ORCHESTRATE_CONTROL, NOTE_KIND_PLAN_APPROVED,
    NOTE_KIND_REWIND_SUMMARY, ORCH_MAX_TRUNCATION_RETRIES, ORCH_MONITORING_TOOLS,
    ORCH_REASONING_MAX_OUTPUT_CAP, ORCH_REASONING_MIN_MAX_TOKENS, ORCH_STAGE_BUDGET_GUARD,
    ORCH_STAGE_CHECKPOINT, ORCH_STAGE_CIRCUIT_BREAKER, ORCH_STAGE_COMPACTION, ORCH_STAGE_ESCALATED,
    ORCH_STAGE_GEAR_SHIFT, ORCH_STAGE_INTENT_GATE, ORCH_STAGE_MICROCOMPACT, ORCH_STAGE_PACING,
    ORCH_STAGE_PHASE_GATE, ORCH_STAGE_PLAN_MODE, ORCH_STAGE_POSTURE, ORCH_STAGE_RECON,
    ORCH_STAGE_SYNTHESIS, ORCH_STAGE_TOOL_ACTIVATED, ORCH_STAGE_TRIAGED,
    ORCH_STAGE_TRUNCATION_RECOVERY, ORCH_STAGE_VERIFICATION_GATE, OrchestrateControlReceipt,
    OrchestrateControlRequest, OrchestratorBridge, PeerRole, PermissionMode, QueuedPrompt,
    ReasoningEffort, Result, RewindMode, RewindRequest, SESSION_MAX_JOB_NOTICES, SHELL_BIN_UNIX,
    STEERING_PREVIEW_CHARS, SUBAGENT_CUSTOM_DIRECTIVE_MAX_CHARS, SUBAGENT_DEFAULT_WAIT_SECS,
    SUBAGENT_LIST_DEFAULT_LIMIT, SUBAGENT_MAX_WAIT_SECS, SUBAGENT_SCHEMA_MAX_RETRIES, SecretVault,
    SendMode, Session, SessionDispatchState, SessionEvent, SessionSearchFilter, SessionStatus,
    SessionTask, AutoDispatchDecision, resolve_auto_send_mode,
    SessionTaskPatch, SessionTaskStatus, SkillAuthor, SkillManageAction, SkillStatus, SkillsConfig,
    SubagentBridge, SubagentRecord, SubagentSpawnRequest, SubagentWorkspaceMode,
    TOOL_AGENT_WORKTREE, TOOL_ASK_CLARIFICATION, TOOL_BATCH_TOOLS, TOOL_GIT,
    TOOL_LSP, TOOL_MEMORY, TOOL_ORCHESTRATE_CONTROL, TOOL_PLAN_UPDATE,
    TOOL_RECORD_BRAINSTORM, TOOL_SESSION_TASK, TOOL_SHELL_EXEC,
    TOOL_SPAWN_AGENT, TOOL_TOOL_SEARCH, TaskKind, ToolAvailabilityContext, ToolBatchBridge,
    ToolCall, ToolConcurrency, ToolContext, ToolOutput, ToolSet, ToolSetSummary, ToolSpec,
    TriageReport,
    TriggerRateLimiter, TriggerStatusEntry, TurnPosture, UsageStats, WebSearchConfig,
    WorktreeAction, WorktreeDiffReport, WorktreePruneReport, ZenkoConfig, ZenkoError,
    compact_preview, find_provider, find_subagent_role, is_git_tool_args_mutating,
    is_lsp_tool_args_mutating, is_memory_mutating_action, new_goal_loop_id, new_id,
    new_session_task_id,
};
use zenko_memory::{
    SnapshotManager, SqliteStore, WorkspaceSnapshot, filter_query_terms, parse_since_window,
    project_scope_for_workspace, resolve_git_root,
};
use zenko_providers::{CompactModelRegistry, UnifiedLlmProvider};
use zenko_tools::{
    ToolRegistry,
    coerce::coerce_and_repair_args,
    lsp::LspPoolManager,
    mcp_client::McpClientManager,
    memory_cron::MemoryTool,
};

#[derive(Debug, Clone, Default)]
pub struct TurnOptions {
    pub parent_event_override: Option<Option<String>>,
    pub workspace_override: Option<PathBuf>,
    pub model_override: Option<String>,
    pub permission_override: Option<PermissionMode>,
    /// When true, tools requiring HITL approval fail fast with a clear policy message
    /// instead of blocking up to 300s (used for standalone CLI and unattended headless runs).
    pub headless_no_wait: bool,
    /// Non-destructive concurrency behavior when a turn is already active on the same session:
    /// `Queue` waits cleanly, `Steer` injects guidance mid-flight, `Interrupt` cancels & replaces.
    pub send_mode: SendMode,
    /// Optional Phase 5 subagent role ID enforcing declarative tool allowlists & prompt addendum.
    pub role_id: Option<String>,
    /// Optional per-turn tool iteration cap override (used by subagent roles).
    pub max_iterations_override: Option<usize>,
    /// Optional JSON Schema enforcing structured JSON return payload from subagents.
    pub output_schema: Option<Value>,
    /// Optional parent/supervisor cancellation token linked via `child_token()`.
    pub external_cancel: Option<CancellationToken>,
    /// Optional per-turn/subagent USD spend ceiling enforced alongside `daily_spend_limit_usd`.
    pub max_cost_usd_override: Option<f64>,
    /// Optional LLM-authored specialist directive appended to subagent system instructions.
    pub custom_directive: Option<String>,
    /// Optional explicit read-only clamp for dynamic specialist subagents.
    pub custom_read_only: Option<bool>,
    /// Optional narrowed tool allowlist intersected with role & permission policy.
    pub custom_tool_subset: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PendingControlShift {
    pub posture: Option<TurnPosture>,
    pub cognitive_tier: Option<ModelRoutingRole>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub rationale: Option<String>,
}

#[derive(Debug, Clone)]
enum WakeupNotice {
    ShellJob(String),
    Subagent { agent_id: String, summary: String },
}

impl WakeupNotice {
    fn as_text(&self) -> &str {
        match self {
            Self::ShellJob(s) => s.as_str(),
            Self::Subagent { summary, .. } => summary.as_str(),
        }
    }
}

/// Single-source-of-truth session tool visibility & invocation scope shared across
/// schema generation, `<deferred_tools>` catalogs, `tool_search`, unknown-tool fuzzy hints,
/// and `batch_tools` inner execution (zero duplicated role/Plan policy logic).
#[derive(Debug, Clone)]
pub(crate) struct SessionToolScope {
    pub role_id: Option<String>,
    pub permission: PermissionMode,
    pub custom_read_only: bool,
    pub custom_tool_subset: HashSet<String>,
    pub availability: ToolAvailabilityContext,
}

impl SessionToolScope {
    pub(crate) fn permits_spec(&self, spec: &ToolSpec) -> bool {
        if let Some(role_desc) = self.role_id.as_deref().and_then(find_subagent_role)
            && !role_desc.permits_tool(&spec.name)
        {
            return false;
        }
        if (self.permission == PermissionMode::Plan || self.custom_read_only)
            && !is_tool_allowed_in_plan_schema(spec)
        {
            return false;
        }
        if !self.custom_tool_subset.is_empty() && !self.custom_tool_subset.contains(&spec.name) {
            return false;
        }
        true
    }

    pub(crate) fn permits_invocation(
        &self,
        tool_name: &str,
        args: &Value,
    ) -> std::result::Result<(), String> {
        if !self.custom_tool_subset.is_empty() && !self.custom_tool_subset.contains(tool_name) {
            return Err(format!(
                "Tool `{tool_name}` is outside this session's scoped `tool_subset`."
            ));
        }
        match (
            self.role_id.as_deref().and_then(find_subagent_role),
            self.custom_read_only,
        ) {
            (Some(role_desc), _) => {
                let mutating_mem = tool_name == TOOL_MEMORY
                    && is_memory_mutating_action(
                        args.get("action").and_then(Value::as_str).unwrap_or(""),
                    );
                let blocked_by_ro = (role_desc.read_only || self.custom_read_only)
                    && matches!(
                        orchestrator::classify_tool_category(tool_name, args),
                        orchestrator::ToolCategory::Mutation
                    );
                match !role_desc.permits_tool(tool_name) || mutating_mem || blocked_by_ro {
                    true => Err(format!(
                        "Subagent role `{}` policy denied tool `{}`.",
                        role_desc.id, tool_name
                    )),
                    false => Ok(()),
                }
            }
            (None, true) => match orchestrator::classify_tool_category(tool_name, args) {
                orchestrator::ToolCategory::Mutation => Err(format!(
                    "Read-only session scope denied mutating tool `{tool_name}`."
                )),
                _ => Ok(()),
            },
            (None, false) => Ok(()),
        }
    }
}

#[derive(Default)]
pub(crate) struct SessionControl {
    steer_queue: Mutex<VecDeque<String>>,
    job_notices: Mutex<VecDeque<WakeupNotice>>,
    active_tools: Mutex<HashMap<String, usize>>,
    pending_control: Mutex<Option<PendingControlShift>>,
    explicit_model_locked: AtomicBool,
    active_scope: Mutex<Option<SessionToolScope>>,
}

/// RAII guard binding the live `SessionToolScope` while a turn executes (restores previous scope on drop).
struct ActiveSessionScopeGuard {
    ctrl: Arc<SessionControl>,
    prev: Option<SessionToolScope>,
}

impl ActiveSessionScopeGuard {
    fn enter(ctrl: Arc<SessionControl>, scope: SessionToolScope) -> Self {
        let prev = ctrl
            .active_scope
            .lock()
            .ok()
            .and_then(|mut slot| slot.replace(scope));
        Self { ctrl, prev }
    }
}

impl Drop for ActiveSessionScopeGuard {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.ctrl.active_scope.lock() {
            *slot = self.prev.take();
        }
    }
}

/// RAII multiset counter tracking in-flight tool names on a session (cleanup guaranteed on cancel/error).
pub(crate) struct ActiveToolGuard {
    ctrl: Arc<SessionControl>,
    tool_name: String,
}

impl ActiveToolGuard {
    pub(crate) fn enter(ctrl: Arc<SessionControl>, tool_name: String) -> Self {
        if let Ok(mut map) = ctrl.active_tools.lock() {
            *map.entry(tool_name.clone()).or_insert(0) += 1;
        }
        Self { ctrl, tool_name }
    }
}

impl Drop for ActiveToolGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = self.ctrl.active_tools.lock()
            && let Some(count) = map.get_mut(&self.tool_name) {
                match *count <= 1 {
                    true => {
                        map.remove(&self.tool_name);
                    }
                    false => *count -= 1,
                }
            }
    }
}

pub(crate) struct ExecutedToolCall {
    pub(crate) tool_call_id: String,
    pub(crate) tool_name: String,
    pub(crate) arguments: Value,
    pub(crate) output: ToolOutput,
    pub(crate) duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnResult {
    pub session_id: String,
    pub reply: String,
    pub model: String,
    pub usage: UsageStats,
    pub tool_calls_count: usize,
}

/// Leak-free `Weak<AgentRuntime>` bridge connecting `tool_search` & `context_inspect`
/// in `zenko-tools` to live session context & deferred tool state without reference cycles.
mod bridges;
mod ipc;
mod loops;
mod turn;
#[cfg(test)]
mod tests;
use bridges::*;

pub struct AgentRuntime {
    config_path: PathBuf,
    config: RwLock<ZenkoConfig>,
    pub store: SqliteStore,
    pub snapshots: Arc<SnapshotManager>,
    pub vault: Arc<SecretVault>,
    pub provider: UnifiedLlmProvider,
    pub tools: Arc<ToolRegistry>,
    pub http: Client,
    pub events: broadcast::Sender<AgentEvent>,
    pub logs: LogBuffer,
    pub log_sink: Arc<CategorizedLogSink>,
    pub jobs: Arc<LocalJobSupervisor>,
    pub subagents: Arc<SubagentSupervisor>,
    pub checkpoints: Arc<CheckpointManager>,
    pub clarifications: SharedClarificationHub,
    web_search_cfg: Arc<RwLock<WebSearchConfig>>,
    lsp_cfg: Arc<RwLock<LspConfig>>,
    mcp_servers_cfg: Arc<RwLock<Vec<McpServerConfig>>>,
    skills_cfg: Arc<RwLock<SkillsConfig>>,
    pub lsp: Arc<LspPoolManager>,
    pub mcp_clients: Arc<McpClientManager>,
    pub file_state: Arc<SessionFileTracker>,
    pub trigger_limiter: Arc<TriggerRateLimiter>,
    started_at: Instant,
    turn_seq: AtomicU64,
    last_workspace_mutation_ms: AtomicU64,
    session_locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    goal_locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    session_controls: Arc<Mutex<HashMap<String, Arc<SessionControl>>>>,
    active_cancels: Mutex<HashMap<String, (u64, CancellationToken)>>,
    demote_signals: Mutex<HashMap<String, (u64, Arc<Notify>)>>,
    pending_approvals: Mutex<HashMap<String, (String, oneshot::Sender<ApprovalDecision>)>>,
    always_allowed: RwLock<HashSet<String>>,
    activated_deferred: Mutex<HashMap<String, BTreeSet<String>>>,
    /// Hermes + Oh My Pi KV-cache lock: freezes each session's initial prompt snapshot so
    /// mid-session memory edits never bust Tier-1/Tier-2 prompt caches.
    frozen_snapshots: Mutex<HashMap<String, (PathBuf, Arc<WorkspaceSnapshot>)>>,
    /// Tier-3 volatile delta notices surfaced cleanly when memory changes mid-session.
    memory_deltas: Mutex<HashMap<String, Vec<String>>>,
    cron_notify: Arc<Notify>,
    pub dream_notify: Arc<Notify>,
    pub goal_notify: Arc<Notify>,
    pub watcher_notify: Arc<Notify>,
    wake_tx: mpsc::UnboundedSender<String>,
    wake_rx: Mutex<Option<mpsc::UnboundedReceiver<String>>>,
    telegram_connected: AtomicBool,
    cached_models: Mutex<Option<(Instant, Vec<ModelInfo>)>>,
}

impl AgentRuntime {
    pub fn build(config_path: PathBuf, config: ZenkoConfig) -> Result<(Arc<Self>, LogBuffer)> {
        let data_dir = config.resolved_data_dir();
        let workspace_dir = config.resolved_workspace_dir();
        let hub_dir = config.ensure_hub_layout()?;
        std::fs::create_dir_all(&data_dir)?;

        let store = SqliteStore::open(config.db_path())?;
        let recovered = store.recover_interrupted_sessions()?;
        if recovered > 0 {
            info!(recovered, "recovered interrupted sessions after restart");
        }
        let recovered_jobs = store.recover_interrupted_jobs().unwrap_or(0);
        if recovered_jobs > 0 {
            info!(recovered_jobs, "marked stale background jobs as interrupted");
        }
        let recovered_agents = store.recover_interrupted_subagents().unwrap_or(0);
        if recovered_agents > 0 {
            info!(recovered_agents, "marked stale subagents as killed after restart");
        }

        let snapshots = Arc::new(
            SnapshotManager::new(&data_dir, &workspace_dir).with_hub_dir(&hub_dir),
        );
        snapshots.ensure_defaults()?;

        let vault = Arc::new(SecretVault::open(config.vault_path(), config.vault_key_path()));

        let http = Client::builder()
            .pool_max_idle_per_host(4)
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| ZenkoError::Config(format!("failed to build http client: {e}")))?;

        let provider_desc = find_provider(&config.provider.kind);
        let api_key = vault.get(provider_desc.vault_secret_key)?;
        let provider = UnifiedLlmProvider::for_provider(
            http.clone(),
            config.provider.kind.clone(),
            config.provider.base_url.clone(),
            api_key,
        );

        let (events, _) = broadcast::channel(DEFAULT_EVENT_BROADCAST_CAPACITY);
        let logs = LogBuffer::new(DEFAULT_LOG_BUFFER_CAPACITY, events.clone());
        let log_sink = CategorizedLogSink::start(&data_dir, store.clone(), vault.clone());
        logs.attach_sink(log_sink.clone());
        install_panic_hook(log_sink.logs_dir().to_path_buf());

        let session_controls: Arc<Mutex<HashMap<String, Arc<SessionControl>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (wake_tx, wake_rx) = mpsc::unbounded_channel();
        let notice_controls = session_controls.clone();
        let notice_wake_tx = wake_tx.clone();
        let jobs = LocalJobSupervisor::new(
            &data_dir,
            store.clone(),
            events.clone(),
            Some(Arc::new(move |rec| {
                let exit_tag = rec
                    .exit_code
                    .map(|c| format!(" (exit_code={c})"))
                    .unwrap_or_default();
                let notice = format!(
                    "Background job `{}` finished with status `{}`{exit_tag}. Command: `{}`. Output ({} bytes) at {}. Use `job_read` if you need its output.",
                    rec.id,
                    rec.status.as_str(),
                    compact_preview(&rec.command, JOB_NOTICE_CMD_PREVIEW_CHARS),
                    rec.bytes_written,
                    rec.log_path
                );
                push_job_notice(&notice_controls, &rec.session_id, WakeupNotice::ShellJob(notice));
                if matches!(
                    rec.status,
                    BackgroundJobStatus::Completed | BackgroundJobStatus::Failed
                ) {
                    let _ = notice_wake_tx.send(rec.session_id.clone());
                }
            })),
        );

        let cron_notify = Arc::new(Notify::new());
        let clarifications = ClarificationHub::new(store.clone(), events.clone());
        let checkpoints = CheckpointManager::new(data_dir.clone(), store.clone(), events.clone());
        let subagents = Arc::new(SubagentSupervisor::new());
        let ctx_bridge = Arc::new(RuntimeContextBridge::new());
        let batch_bridge = Arc::new(RuntimeBatchBridge::new());
        let subagent_bridge = Arc::new(RuntimeSubagentBridge::new());
        let goal_bridge = Arc::new(RuntimeGoalLoopBridge::new());
        let web_search_cfg = Arc::new(RwLock::new(config.web_search.clone()));
        let lsp_cfg = Arc::new(RwLock::new(config.lsp.clone()));
        let mcp_servers_cfg = Arc::new(RwLock::new(config.mcp_servers.clone()));
        let skills_cfg = Arc::new(RwLock::new(config.skills.clone()));
        let lsp = LspPoolManager::new(lsp_cfg.clone());
        let mcp_clients = McpClientManager::new(
            mcp_servers_cfg.clone(),
            store.clone(),
            vault.clone(),
            http.clone(),
        );

        let tool_reg = ToolRegistry::default_suite(
            http.clone(),
            store.clone(),
            snapshots.clone(),
            cron_notify.clone(),
            Some(jobs.clone() as Arc<dyn JobSupervisor>),
            Some(clarifications.clone() as Arc<dyn OrchestratorBridge>),
        );
        tool_reg.register_phase7_tools(
            http.clone(),
            web_search_cfg.clone(),
            vault.clone(),
            lsp.clone(),
            mcp_clients.clone(),
        );
        tool_reg.register_phase8_tools(
            snapshots.clone(),
            vault.clone(),
            skills_cfg.clone(),
            goal_bridge.clone() as Arc<dyn GoalLoopBridge>,
        );
        tool_reg.register_context_tools(ctx_bridge.clone() as Arc<dyn ContextBridge>);
        tool_reg.register_batch_tool(batch_bridge.clone() as Arc<dyn ToolBatchBridge>);
        tool_reg.register_subagent_tools(subagent_bridge.clone() as Arc<dyn SubagentBridge>);
        tool_reg.register(Arc::new(MemoryTool::new_with_vault(
            store.clone(),
            snapshots.clone(),
            vault.clone(),
        )));
        mcp_clients.hydrate_cached_tools(&tool_reg);
        let tools = Arc::new(tool_reg);

        let rt = Arc::new(Self {
            config_path,
            config: RwLock::new(config),
            store,
            snapshots,
            vault,
            provider,
            tools,
            http,
            events,
            logs: logs.clone(),
            log_sink,
            jobs,
            subagents,
            checkpoints,
            clarifications,
            web_search_cfg,
            lsp_cfg,
            mcp_servers_cfg,
            skills_cfg,
            lsp,
            mcp_clients,
            file_state: Arc::new(SessionFileTracker::new()),
            trigger_limiter: Arc::new(TriggerRateLimiter::new()),
            started_at: Instant::now(),
            turn_seq: AtomicU64::new(1),
            last_workspace_mutation_ms: AtomicU64::new(0),
            session_locks: Mutex::new(HashMap::new()),
            goal_locks: Mutex::new(HashMap::new()),
            session_controls,
            active_cancels: Mutex::new(HashMap::new()),
            demote_signals: Mutex::new(HashMap::new()),
            pending_approvals: Mutex::new(HashMap::new()),
            always_allowed: RwLock::new(HashSet::new()),
            activated_deferred: Mutex::new(HashMap::new()),
            frozen_snapshots: Mutex::new(HashMap::new()),
            memory_deltas: Mutex::new(HashMap::new()),
            cron_notify,
            dream_notify: Arc::new(Notify::new()),
            goal_notify: Arc::new(Notify::new()),
            watcher_notify: Arc::new(Notify::new()),
            wake_tx,
            wake_rx: Mutex::new(Some(wake_rx)),
            telegram_connected: AtomicBool::new(false),
            cached_models: Mutex::new(None),
        });
        ctx_bridge.bind(Arc::downgrade(&rt));
        batch_bridge.bind(Arc::downgrade(&rt));
        subagent_bridge.bind(Arc::downgrade(&rt));
        goal_bridge.bind(Arc::downgrade(&rt));
        rt.clarifications.bind_runtime(Arc::downgrade(&rt));

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let art_dir = data_dir.clone();
            let cp_mgr = rt.checkpoints.clone();
            let cp_cfg = rt.config().checkpoints;
            let bg_store = rt.store.clone();
            handle.spawn(async move {
                artifacts::prune_old_artifacts(&art_dir).await;
                if cp_cfg.enabled {
                    let _ = cp_mgr.prune_checkpoints(&cp_cfg).await;
                }
                // One-time migration of oversized legacy raw `models.dev` JSON (~6.8 MB) into
                // compact Zenko-only index (~145 KB), reclaiming DB + WAL disk & RAM overhead.
                let _ = tokio::task::spawn_blocking(move || {
                    if let Ok(Some(len)) = bg_store.kv_value_len(MODEL_KV_REGISTRY_KEY) {
                        if len >= MODEL_KV_LEGACY_COMPACT_THRESHOLD_BYTES {
                            if let Ok(Some(raw)) = bg_store.get_kv(MODEL_KV_REGISTRY_KEY)
                                && let Some(compact) =
                                    CompactModelRegistry::parse_raw_models_dev_bytes(raw.as_bytes())
                                && let Some(compact_json) = compact.to_compact_json()
                            {
                                let _ = bg_store.set_kv(MODEL_KV_COMPACT_REGISTRY_KEY, &compact_json);
                            }
                            let _ = bg_store.delete_kv(MODEL_KV_REGISTRY_KEY);
                        } else {
                            let _ = bg_store.delete_kv(MODEL_KV_REGISTRY_KEY);
                        }
                    }
                    let _ = bg_store.vacuum_if_bloated();
                })
                .await;
            });
        }

        Ok((rt, logs))
    }

    pub fn config(&self) -> ZenkoConfig {
        self.config
            .read()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub fn update_config(&self, mutate: impl FnOnce(&mut ZenkoConfig)) -> Result<ZenkoConfig> {
        let updated = {
            let mut guard = self
                .config
                .write()
                .map_err(|_| ZenkoError::Config("config lock poisoned".into()))?;
            mutate(&mut guard);
            guard.save(&self.config_path)?;
            guard.clone()
        };
        self.snapshots.set_workspace_dir(updated.resolved_workspace_dir());
        let _ = updated.ensure_hub_layout();
        self.snapshots.set_hub_dir(updated.resolved_hub_dir());
        let provider_desc = find_provider(&updated.provider.kind);
        let api_key = self.vault.get(provider_desc.vault_secret_key)?;
        self.provider.update_provider(
            Some(updated.provider.kind.clone()),
            Some(updated.provider.base_url.clone()),
            api_key,
        );
        if let Ok(mut guard) = self.cached_models.lock() {
            *guard = None;
        }
        if let Ok(mut w) = self.web_search_cfg.write() {
            *w = updated.web_search.clone();
        }
        if let Ok(mut l) = self.lsp_cfg.write() {
            *l = updated.lsp.clone();
        }
        if let Ok(mut m) = self.mcp_servers_cfg.write() {
            *m = updated.mcp_servers.clone();
        }
        if let Ok(mut sk) = self.skills_cfg.write() {
            *sk = updated.skills.clone();
        }
        self.cron_notify.notify_one();
        self.goal_notify.notify_one();
        self.watcher_notify.notify_one();
        Ok(updated)
    }

    /// Synchronizes one configured external MCP server (`tools/list`), refreshing SQLite cache,
    /// hot-swapping `mcp__<server>__*` proxies inside `ToolRegistry`, and emitting SSE status.
    pub async fn sync_mcp_server(&self, name: &str) -> Result<McpServerStatus> {
        let status = self.mcp_clients.sync_server(name, &self.tools).await?;
        let servers = self.mcp_clients.list_status().await;
        self.emit(AgentEvent::McpServersUpdated { servers });
        Ok(status)
    }

    /// Adds or replaces an external MCP server in `zenko.toml`, disconnects any stale live session
    /// for that server, and optionally triggers immediate `tools/list` discovery.
    pub async fn upsert_mcp_server(
        &self,
        mut server: McpServerConfig,
        sync_now: bool,
    ) -> Result<(Vec<McpServerStatus>, Option<String>)> {
        server.name = server.name.trim().to_string();
        if server.name.is_empty() {
            return Err(ZenkoError::Config("MCP server `name` cannot be empty".into()));
        }
        let target_name = server.name.clone();
        let enabled = server.enabled;
        self.mcp_clients.disconnect_server(&target_name).await;
        self.update_config(|cfg| {
            match cfg
                .mcp_servers
                .iter_mut()
                .find(|s| s.name.eq_ignore_ascii_case(&target_name))
            {
                Some(existing) => *existing = server,
                None => cfg.mcp_servers.push(server),
            }
        })?;
        let mut sync_err = None;
        if !enabled {
            let _ = self.mcp_clients.sync_server(&target_name, &self.tools).await;
        } else if sync_now
            && let Err(e) = self.mcp_clients.sync_server(&target_name, &self.tools).await {
                sync_err = Some(e.to_string());
            }
        let servers = self.mcp_clients.list_status().await;
        self.emit(AgentEvent::McpServersUpdated {
            servers: servers.clone(),
        });
        Ok((servers, sync_err))
    }

    /// Removes an external MCP server from `zenko.toml`, tears down its connection, clears its
    /// cached SQLite catalog, and unregisters all `mcp__<name>__*` tools live.
    pub async fn delete_mcp_server(&self, name: &str) -> Result<bool> {
        let trimmed = name.trim().to_string();
        let mut removed = false;
        self.update_config(|cfg| {
            let before = cfg.mcp_servers.len();
            cfg.mcp_servers
                .retain(|s| !s.name.eq_ignore_ascii_case(&trimmed));
            removed = cfg.mcp_servers.len() < before;
        })?;
        self.mcp_clients.remove_server(&trimmed, &self.tools).await;
        let servers = self.mcp_clients.list_status().await;
        self.emit(AgentEvent::McpServersUpdated { servers });
        Ok(removed)
    }

    /// Low-RAM periodic idle reaper evicting idle LSP servers, external MCP client sessions,
    /// expired model metadata caches, and truncating idle SQLite WAL files.
    pub async fn run_external_tool_reaper(self: Arc<Self>, cancel: CancellationToken) {
        let mut interval =
            tokio::time::interval(Duration::from_secs(EXTERNAL_TOOL_REAPER_INTERVAL_SECS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut ticks: u64 = 0;
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = interval.tick() => {
                    ticks = ticks.wrapping_add(1);
                    for dirty_name in self.mcp_clients.dirty_servers().await {
                        let _ = self.sync_mcp_server(&dirty_name).await;
                    }
                    let _ = self.lsp.evict_idle().await;
                    let _ = self.mcp_clients.evict_idle().await;
                    if ticks.is_multiple_of(IDLE_DB_COMPACT_EVERY_TICKS) {
                        self.provider.evict_expired_metadata_cache();
                        if let Ok(mut guard) = self.cached_models.lock()
                            && let Some((ts, _)) = guard.as_ref()
                            && ts.elapsed() >= Duration::from_secs(MODEL_CACHE_TTL_SECS)
                        {
                            *guard = None;
                        }
                        let is_idle = self
                            .active_cancels
                            .lock()
                            .map(|g| g.is_empty())
                            .unwrap_or(false);
                        if is_idle {
                            let st = self.store.clone();
                            let _ = tokio::task::spawn_blocking(move || {
                                let _ = st.compact_wal();
                            })
                            .await;
                        }
                    }
                }
            }
        }
    }

    /// Cleanly terminates all pooled LSP servers and external MCP client sessions on shutdown.
    pub async fn shutdown_external_tools(&self) {
        self.lsp.stop_all().await;
        self.mcp_clients.stop_all().await;
    }

    pub fn refresh_provider_credentials(&self) -> Result<()> {
        let cfg = self.config();
        let provider_desc = find_provider(&cfg.provider.kind);
        let api_key = self.vault.get(provider_desc.vault_secret_key)?;
        self.provider.update_provider(
            Some(cfg.provider.kind),
            Some(cfg.provider.base_url),
            api_key,
        );
        if let Ok(mut guard) = self.cached_models.lock() {
            *guard = None;
        }
        Ok(())
    }

    pub fn set_telegram_connected(&self, connected: bool) {
        self.telegram_connected.store(connected, Ordering::Relaxed);
    }

    pub fn is_telegram_connected(&self) -> bool {
        self.telegram_connected.load(Ordering::Relaxed)
    }

    pub fn notify_cron_updated(&self) {
        self.cron_notify.notify_one();
    }

    pub fn dream_status(&self) -> DreamStatus {
        crate::dream::dream_status(self)
    }

    pub async fn run_dream_now(
        self: &Arc<Self>,
        trigger: &str,
        force: bool,
    ) -> Result<DreamRunRecord> {
        crate::dream::run_dream_cycle(self, trigger, force, CancellationToken::new()).await
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.events.subscribe()
    }

    pub fn emit(&self, ev: AgentEvent) {
        let _ = self.events.send(ev);
    }

    pub async fn notify_session_ended(&self, session_id: &str) {
        let cfg = self.config();
        let _ = run_hooks(
            &cfg.hooks,
            HookEvent::SessionEnd,
            None,
            json!({ "session_id": session_id }),
        )
        .await;
    }

    /// Promotes a session out of Read-Only Plan Mode (`PermissionMode::Plan`) into `target_mode`
    /// (`Balanced` or `Autonomous` or `Strict`), persists a `NOTE_KIND_PLAN_APPROVED` trace note,
    /// and broadcasts `AgentEvent::PlanApproved` so WebUI/CLI/Telegram update instantaneously.
    pub fn approve_session_plan(
        &self,
        session_id: &str,
        target_mode: PermissionMode,
    ) -> Result<(Session, usize)> {
        let session = self
            .store
            .get_session(session_id)?
            .ok_or_else(|| ZenkoError::NotFound(format!("session `{session_id}` not found")))?;
        let new_mode = match target_mode {
            PermissionMode::Plan => PermissionMode::Autonomous,
            other => other,
        };
        self.store
            .update_session_meta(session_id, None, None, Some(new_mode))?;
        let tasks = self
            .clarifications
            .list_session_tasks(session_id)
            .unwrap_or_default();
        let task_count = tasks.len();
        let _ = self.store.append_event(
            session_id,
            session.head_event_id.as_deref(),
            EventRole::System,
            EventKind::SystemNote,
            json!({
                "note_kind": NOTE_KIND_PLAN_APPROVED,
                "previous_mode": session.permission_mode.as_str(),
                "new_mode": new_mode.as_str(),
                "task_count": task_count,
                "text": format!("Plan approved; promoted session permission mode from `{}` to `{}` ({task_count} tracked DAG task(s)).", session.permission_mode.as_str(), new_mode.as_str()),
            }),
            &UsageStats::default(),
        );
        let updated = self
            .store
            .get_session(session_id)?
            .unwrap_or(session);
        self.emit(AgentEvent::PlanApproved {
            session_id: session_id.to_string(),
            new_mode,
            task_count,
        });
        self.emit(AgentEvent::SessionUpdated {
            session_id: session_id.to_string(),
            status: updated.status,
            title: Some(updated.title.clone()),
        });
        Ok((updated, task_count))
    }

    pub fn telemetry(&self) -> DaemonTelemetry {
        let cfg = self.config();
        let active_sessions = self
            .active_cancels
            .lock()
            .map(|m| m.len())
            .unwrap_or(0);
        let pending_approvals = self
            .store
            .list_pending_approvals()
            .map(|v| v.len())
            .unwrap_or(0);
        let daily_cost_usd = self.store.daily_cost_usd().unwrap_or(0.0);
        let (total_cost_usd, cache_hit_pct) = self
            .store
            .total_usage_summary()
            .unwrap_or((0.0, 0.0));

        DaemonTelemetry {
            pid: std::process::id(),
            uptime_secs: self.started_at.elapsed().as_secs(),
            rss_bytes: current_rss_bytes(),
            db_size_bytes: self.store.db_size_bytes(),
            active_sessions,
            active_jobs: self.jobs.active_count(),
            pending_approvals,
            daily_cost_usd,
            total_cost_usd,
            cache_hit_pct,
            telegram_connected: self.is_telegram_connected(),
            telegram_bot_username: cfg.telegram.bot_username,
        }
    }

    pub async fn list_models_cached(&self) -> Result<Vec<ModelInfo>> {
        let cfg = self.config();
        self.list_models_for(&cfg.provider.kind, Some(&cfg.provider.base_url), false)
            .await
    }

    /// Discovers live working models for any provider (FREE models sorted first),
    /// hydrating exact context/pricing via live `models.dev` + Ollama `/api/show`,
    /// with SQLite `kv_store` persistence for offline resilience (zero hardcoded models).
    pub async fn list_models_for(
        &self,
        provider_kind: &str,
        base_url_override: Option<&str>,
        force_refresh: bool,
    ) -> Result<Vec<ModelInfo>> {
        let cfg = self.config();
        let desc = find_provider(provider_kind);
        let is_active = desc.id.eq_ignore_ascii_case(&cfg.provider.kind)
            && base_url_override
                .map(|u| u.trim().is_empty() || u.trim() == cfg.provider.base_url.trim())
                .unwrap_or(true);

        if is_active && !force_refresh
            && let Ok(guard) = self.cached_models.lock()
                && let Some((ts, models)) = guard.as_ref()
                    && ts.elapsed() < Duration::from_secs(MODEL_CACHE_TTL_SECS)
                        && !models.is_empty()
                    {
                        return Ok(models.clone());
                    }

        let effective_url = base_url_override
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .unwrap_or(match is_active {
                true => cfg.provider.base_url.as_str(),
                false => desc.default_base_url,
            });
        let api_key = self.vault.get(desc.vault_secret_key).ok().flatten();

        // Lazily hydrate compact `models.dev` index (~145 KB) only on demand (never at daemon boot)
        if !force_refresh
            && !desc.metadata_keys.is_empty()
            && self.provider.cached_metadata_registry().is_none()
            && let Ok(Some(compact_raw)) = self.store.get_kv(MODEL_KV_COMPACT_REGISTRY_KEY)
            && let Some(compact) = CompactModelRegistry::from_compact_json(&compact_raw)
        {
            self.provider.seed_compact_registry(Arc::new(compact));
        }

        let kv_key = format!("{MODEL_KV_PROVIDER_PREFIX}{}", desc.id);
        match self
            .provider
            .discover_models_for(
                desc.id,
                Some(effective_url),
                api_key.as_deref().map(|z| z.as_str()),
                force_refresh,
            )
            .await
        {
            Ok(models) => {
                if is_active
                    && let Ok(mut guard) = self.cached_models.lock() {
                        *guard = Some((Instant::now(), models.clone()));
                    }
                if let Ok(serialized) = serde_json::to_string(&models) {
                    let _ = self.store.set_kv(&kv_key, &serialized);
                }
                if let Some(reg) = self.provider.cached_metadata_registry()
                    && let Some(reg_str) = reg.to_compact_json() {
                        let _ = self.store.set_kv(MODEL_KV_COMPACT_REGISTRY_KEY, &reg_str);
                    }
                Ok(models)
            }
            Err(err) => {
                if let Ok(Some(cached_json)) = self.store.get_kv(&kv_key)
                    && let Ok(cached_models) = serde_json::from_str::<Vec<ModelInfo>>(&cached_json)
                        && !cached_models.is_empty() {
                            return Ok(cached_models);
                        }
                Err(err)
            }
        }
    }

    fn resolve_context_window(&self, model_id: &str) -> u64 {
        self.cached_models
            .lock()
            .ok()
            .and_then(|guard| {
                guard.as_ref().and_then(|(_, list)| {
                    list.iter()
                        .find(|m| m.id == model_id)
                        .and_then(|m| m.context_length)
                })
            })
            .filter(|&n| n >= 4_096)
            .unwrap_or(DEFAULT_CONTEXT_WINDOW_FALLBACK)
    }

    /// Returns `Some(true|false)` when `model_id` is hydrated in `cached_models`, else `None`.
    fn resolve_supports_reasoning(&self, model_id: &str) -> Option<bool> {
        self.cached_models.lock().ok().and_then(|guard| {
            guard.as_ref().and_then(|(_, list)| {
                list.iter()
                    .find(|m| m.id == model_id)
                    .map(|m| m.supports_reasoning)
            })
        })
    }

    fn resolve_max_output_tokens(&self, model_id: &str) -> Option<u32> {
        self.cached_models.lock().ok().and_then(|guard| {
            guard.as_ref().and_then(|(_, list)| {
                list.iter()
                    .find(|m| m.id == model_id)
                    .and_then(|m| m.max_output_tokens)
                    .and_then(|v| u32::try_from(v).ok())
            })
        })
    }

    /// Computes a safe output-token ceiling so reasoning models (which share `max_tokens` across
    /// thinking + tool JSON + final text) are never starved by a legacy 4096 cap.
    fn effective_output_budget(&self, configured: u32, model_id: &str, orchestrated: bool) -> u32 {
        let needs_reasoning_headroom =
            orchestrated || self.resolve_supports_reasoning(model_id) != Some(false);
        let desired = match needs_reasoning_headroom {
            true => configured.max(ORCH_REASONING_MIN_MAX_TOKENS),
            false => configured,
        };
        match self.resolve_max_output_tokens(model_id) {
            Some(model_cap) if model_cap >= configured => desired.min(model_cap),
            _ => desired,
        }
    }

    fn session_mutex(&self, session_id: &str) -> Arc<AsyncMutex<()>> {
        let mut map = match self.session_locks.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if map.len() > 256 {
            map.retain(|_, lock| Arc::strong_count(lock) > 1);
        }
        map.entry(session_id.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    /// Per-goal async serialization lock preventing concurrent scheduler & manual `/run` races.
    pub fn goal_lock(&self, goal_id: &str) -> Arc<AsyncMutex<()>> {
        let mut map = match self.goal_locks.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if map.len() > 256 {
            map.retain(|_, lock| Arc::strong_count(lock) > 1);
        }
        map.entry(goal_id.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    pub(crate) fn session_control(&self, session_id: &str) -> Arc<SessionControl> {
        let mut map = match self.session_controls.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        map.entry(session_id.to_string())
            .or_insert_with(|| Arc::new(SessionControl::default()))
            .clone()
    }

    pub(crate) fn snapshot_always_allowed(&self) -> HashSet<String> {
        self.always_allowed
            .read()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub fn is_session_running(&self, session_id: &str) -> bool {
        self.active_cancels
            .lock()
            .map(|m| m.contains_key(session_id))
            .unwrap_or(false)
    }

    fn is_monitoring_tool_active(&self, session_id: &str) -> bool {
        let ctrl_opt = self
            .session_controls
            .lock()
            .ok()
            .and_then(|map| map.get(session_id).cloned());
        let Some(ctrl) = ctrl_opt else {
            return false;
        };
        ctrl.active_tools
            .lock()
            .map(|map| {
                map.keys()
                    .any(|name| ORCH_MONITORING_TOOLS.contains(&name.as_str()))
            })
            .unwrap_or(false)
    }

    /// Deterministically resolves how an incoming prompt should be dispatched (`StartTurn`,
    /// `Steer`, or `Queue`) without any imperative `if/else` ladders.
    pub fn resolve_session_dispatch(
        &self,
        session_id: &str,
        requested: SendMode,
    ) -> AutoDispatchDecision {
        let is_running = self.is_session_running(session_id);
        match (requested, is_running) {
            (_, false) => AutoDispatchDecision::StartTurn,
            (SendMode::Interrupt, true) => AutoDispatchDecision::StartTurn,
            (SendMode::Steer, true) => AutoDispatchDecision::Steer,
            (SendMode::Queue, true) => AutoDispatchDecision::Queue,
            (SendMode::Auto, true) => resolve_auto_send_mode(SessionDispatchState {
                is_running: true,
                active_jobs: self.jobs.active_background_jobs_count_for_session(session_id),
                active_subagents: self.subagents.active_count_for_parent(session_id),
                is_monitoring_tool_active: self.is_monitoring_tool_active(session_id),
            }),
        }
    }

    pub fn active_sessions_count(&self) -> usize {
        self.active_cancels
            .lock()
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// Injects non-destructive steering guidance into an actively running session turn
    /// so it pivots cleanly at the next iteration boundary without cancelling in-flight work.
    /// If the session is paused on a pending HITL approval, steering immediately unblocks it.
    pub fn steer_session(&self, session_id: &str, text: &str) -> bool {
        let trimmed = text.trim();
        if trimmed.is_empty() || !self.is_session_running(session_id) {
            return false;
        }
        let ctrl = self.session_control(session_id);
        if let Ok(mut q) = ctrl.steer_queue.lock() {
            q.push_back(trimmed.to_string());
        }
        self.emit(AgentEvent::SteeringAccepted {
            session_id: session_id.to_string(),
            preview: compact_preview(trimmed, STEERING_PREVIEW_CHARS),
        });

        let pending_ids: Vec<String> = self
            .pending_approvals
            .lock()
            .map(|map| {
                map.iter()
                    .filter(|(_, (sid, _))| sid == session_id)
                    .map(|(aid, _)| aid.clone())
                    .collect()
            })
            .unwrap_or_default();
        for aid in pending_ids {
            let _ = self.resolve_approval(&aid, ApprovalDecision::Deny, APPROVAL_RESOLVER_STEERED);
        }
        self.clarifications.unblock_for_steering(session_id, trimmed);
        true
    }

    /// Runs the deterministic Phase 2 triage classifier for any prompt (zero LLM tokens).
    pub fn triage_prompt(
        &self,
        prompt: &str,
        permission_mode: Option<PermissionMode>,
    ) -> TriageReport {
        let cfg = self.config();
        classify_turn(&TriageInput {
            prompt,
            prior_turn_errors: 0,
            is_first_turn: true,
            permission_mode: permission_mode.unwrap_or(cfg.policy.mode),
            has_fast_model: !cfg.provider.fast_model.trim().is_empty(),
            has_plan_model: !cfg.provider.plan_model.trim().is_empty(),
        })
    }

    /// Hydrates session-scoped activated deferred tools from persisted `NOTE_KIND_DEFERRED_TOOLS`
    /// notes on the active branch and merges them with in-memory state.
    fn hydrate_activated_deferred(
        &self,
        session_id: &str,
        branch_events: &[SessionEvent],
    ) -> BTreeSet<String> {
        let mut set = BTreeSet::new();
        for ev in branch_events {
            if ev.role == EventRole::System
                && ev.kind == EventKind::SystemNote
                && ev.content_json.get("note_kind").and_then(Value::as_str)
                    == Some(NOTE_KIND_DEFERRED_TOOLS)
                && let Some(arr) = ev.content_json.get("tools").and_then(Value::as_array) {
                    for name in arr.iter().filter_map(Value::as_str) {
                        if self.tools.is_deferred(name) {
                            set.insert(name.to_string());
                        }
                    }
                }
        }
        if let Ok(mut map) = self.activated_deferred.lock() {
            let entry = map.entry(session_id.to_string()).or_default();
            entry.extend(set.iter().cloned());
            return entry.clone();
        }
        set
    }

    pub(crate) fn activate_deferred_tools(&self, session_id: &str, names: &[String]) -> Vec<String> {
        let mut newly = Vec::new();
        if let Ok(mut map) = self.activated_deferred.lock() {
            let entry = map.entry(session_id.to_string()).or_default();
            for name in names {
                if self.tools.is_deferred(name) && entry.insert(name.clone()) {
                    newly.push(name.clone());
                }
            }
        }
        newly
    }

    fn activated_deferred_for(&self, session_id: &str) -> BTreeSet<String> {
        self.activated_deferred
            .lock()
            .ok()
            .and_then(|map| map.get(session_id).cloned())
            .unwrap_or_default()
    }

    pub fn clear_session_deferred_tools(&self, session_id: &str) {
        if let Ok(mut map) = self.activated_deferred.lock() {
            map.remove(session_id);
        }
        if let Ok(mut snaps) = self.frozen_snapshots.lock() {
            snaps.remove(session_id);
        }
        if let Ok(mut deltas) = self.memory_deltas.lock() {
            deltas.remove(session_id);
        }
        if let Ok(mut ctrls) = self.session_controls.lock() {
            ctrls.remove(session_id);
        }
    }

    /// Freezes a session's initial `WorkspaceSnapshot` at session boot (Hermes / Oh My Pi pattern)
    /// so mid-session `MEMORY.md` / `PROJECT_MEMORY.md` / Dream writes never bust active KV caches.
    fn frozen_snapshot_for_session(
        &self,
        session_id: &str,
        workspace_dir: &std::path::Path,
        cfg: &ZenkoConfig,
    ) -> Result<Arc<WorkspaceSnapshot>> {
        if let Ok(map) = self.frozen_snapshots.lock()
            && let Some((cached_ws, snap)) = map.get(session_id)
                && cached_ws == workspace_dir {
                    return Ok(snap.clone());
                }
        let loaded = match workspace_dir == self.snapshots.workspace_dir() {
            true => self
                .snapshots
                .load_snapshot_with_options(cfg.context.auto_repo_rules)?,
            false => SnapshotManager::new(cfg.resolved_data_dir(), workspace_dir)
                .with_hub_dir(cfg.resolved_hub_dir())
                .load_snapshot_with_options(cfg.context.auto_repo_rules)?,
        };
        let arc = Arc::new(loaded);
        if let Ok(mut map) = self.frozen_snapshots.lock() {
            if map.len() > 64 {
                let active_sids: HashSet<String> = self
                    .active_cancels
                    .lock()
                    .map(|m| m.keys().cloned().collect())
                    .unwrap_or_default();
                map.retain(|sid, _| active_sids.contains(sid));
                if map.len() > 64
                    && let Some(evict_key) = map
                        .keys()
                        .find(|k| k.as_str() != session_id && !active_sids.contains(k.as_str()))
                        .cloned()
                    {
                        map.remove(&evict_key);
                    }
            }
            map.insert(session_id.to_string(), (workspace_dir.to_path_buf(), arc.clone()));
        }
        Ok(arc)
    }

    pub fn record_memory_delta(&self, session_id: &str, note: String) {
        if let Ok(mut map) = self.memory_deltas.lock() {
            let list = map.entry(session_id.to_string()).or_default();
            if list.len() < 8 {
                list.push(note);
            }
        }
    }

    /// Surfaces a non-cache-breaking Tier-3 `<memory_delta>` to all sessions currently holding a
    /// frozen snapshot when persistent memory changes out-of-band (Dream or WebUI edit).
    pub fn broadcast_memory_delta(&self, note: String) {
        let sids: Vec<String> = self
            .frozen_snapshots
            .lock()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        if sids.is_empty() {
            return;
        }
        if let Ok(mut map) = self.memory_deltas.lock() {
            for sid in sids {
                let list = map.entry(sid).or_default();
                if list.len() < 8 {
                    list.push(note.clone());
                }
            }
        }
    }

    fn drain_memory_deltas(&self, session_id: &str) -> Vec<String> {
        self.memory_deltas
            .lock()
            .ok()
            .and_then(|mut map| map.remove(session_id))
            .unwrap_or_default()
    }

    /// Computes live Phase-3 per-tier token usage, KV-cache hit %, Stage-A microcompaction
    /// savings, active Stage-B checkpoint, deferred tool stats, and spilled artifact count.
    pub async fn inspect_session_context(&self, session_id: &str) -> Result<ContextBreakdown> {
        let cfg = self.config();
        let session = self
            .store
            .get_session(session_id)?
            .ok_or_else(|| ZenkoError::NotFound(format!("session `{session_id}` not found")))?;
        let branch = self.store.get_active_branch(session_id)?;
        let snapshot = self
            .snapshots
            .load_snapshot_with_options(cfg.context.auto_repo_rules)?;

        let active_model = session
            .model_override
            .clone()
            .filter(|m| !m.trim().is_empty())
            .or_else(|| {
                branch.iter().rev().find_map(|ev| {
                    ev.content_json
                        .get("model")
                        .and_then(Value::as_str)
                        .filter(|m| !m.trim().is_empty())
                        .map(String::from)
                })
            })
            .unwrap_or_else(|| cfg.provider.lead_model.clone());
        let context_window = self.resolve_context_window(&active_model);

        let scope = self.effective_session_scope(session_id);
        let activated = self.hydrate_activated_deferred(session_id, &branch);
        let active_specs = self.tools.specs_for_session_filtered(
            cfg.context.deferred_tool_loading,
            &activated,
            &scope.availability,
            |spec| scope.permits_spec(spec),
        );
        let (eager_tools_count, deferred_available_count) = self
            .tools
            .exposure_counts_filtered(&scope.availability, |spec| scope.permits_spec(spec));
        let deferred_catalog = match cfg.context.deferred_tool_loading {
            true => self
                .tools
                .deferred_catalog_text_filtered(&scope.availability, |spec| {
                    scope.permits_spec(spec)
                }),
            false => None,
        };
        let effective_sys = snapshot.compose_system_prompt(deferred_catalog.as_deref());
        let sys_tokens = estimate_text_tokens(&effective_sys);
        let tool_schema_tokens = estimate_tool_specs_tokens(&active_specs);
        let prefix_tokens = sys_tokens + tool_schema_tokens;

        let ckpt_slice = slice_from_latest_checkpoint(&branch);
        let mut raw_messages = events_to_chat_messages_refs(&ckpt_slice.kept_events, &active_model);
        if let Some(ref ckpt) = ckpt_slice.checkpoint {
            prepend_checkpoint_to_messages(&mut raw_messages, ckpt);
        }
        let (mc_messages, mc_stats) = match cfg.context.enabled {
            true => microcompact_messages(
                raw_messages,
                context_window,
                prefix_tokens,
                cfg.context.microcompact_trigger_ratio,
            ),
            false => (raw_messages, Default::default()),
        };

        let history_tokens = estimate_messages_tokens(&mc_messages);
        let total_tokens = prefix_tokens + history_tokens;
        let utilization_pct = match context_window > 0 {
            true => ((total_tokens as f64 / context_window as f64) * 1000.0).round() / 10.0,
            false => 0.0,
        };

        let (sum_in, sum_cache) = branch.iter().fold((0u64, 0u64), |(acc_in, acc_c), ev| {
            (acc_in + ev.tokens_in, acc_c + ev.cache_read_tokens)
        });
        let cache_hit_pct = match sum_in > 0 {
            true => ((sum_cache as f64 / sum_in as f64) * 1000.0).round() / 10.0,
            false => 0.0,
        };

        let spilled_artifacts_count =
            artifacts::count_session_artifacts(&cfg.resolved_data_dir(), session_id).await;

        Ok(ContextBreakdown {
            session_id: session_id.to_string(),
            model: active_model,
            context_window,
            total_tokens,
            utilization_pct,
            tier1_tokens: snapshot.tier1_tokens,
            tier1_hash: snapshot.tier1_hash,
            tier2_tokens: sys_tokens.saturating_sub(snapshot.tier1_tokens),
            repo_rules_loaded: snapshot.repo_rules_loaded,
            tool_schema_tokens,
            eager_tools_count,
            deferred_active_count: activated.len(),
            deferred_available_count,
            history_tokens,
            microcompacted_tools_count: mc_stats.compacted_tools,
            microcompacted_tokens_saved: mc_stats.tokens_saved,
            active_checkpoint_id: ckpt_slice.checkpoint_event_id,
            checkpoints_count: ckpt_slice.checkpoints_count,
            spilled_artifacts_count,
            cache_hit_pct,
        })
    }

    /// Async Stage-B iterative-merge checkpoint summarizer using `compaction_model -> fast_model
    /// -> active_model` with automatic zero-LLM deterministic fallback on timeout or error.
    #[allow(clippy::too_many_arguments)]
    async fn run_stage_b_compaction(
        &self,
        cfg: &ZenkoConfig,
        session_id: &str,
        active_model: &str,
        plan: &CheckpointPlan<'_>,
        prior_checkpoint: Option<&CompactionCheckpointPayload>,
        active_exec_plan: Option<&ExecutionPlan>,
        prefix_tokens: usize,
        cancel: &CancellationToken,
    ) -> CompactionCheckpointPayload {
        let plan_summary = active_exec_plan.map(format_plan_summary);
        let (sys_prompt, user_body) =
            build_compaction_prompt(plan, prior_checkpoint, plan_summary.as_deref());

        let comp_model = [
            cfg.provider.compaction_model.as_str(),
            cfg.provider.fast_model.as_str(),
            active_model,
        ]
        .into_iter()
        .map(str::trim)
        .find(|m| !m.is_empty())
        .unwrap_or(active_model)
        .to_string();

        let comp_req = ChatRequest {
            session_id: session_id.to_string(),
            model: comp_model.clone(),
            fallbacks: Vec::new(),
            system_prompt: sys_prompt,
            messages: vec![ChatMessage {
                role: EventRole::User,
                content: user_body,
                reasoning_content: None,
                tool_calls: Vec::new(),
                tool_call_id: None,
                name: None,
            }],
            tools: Vec::new(),
            temperature: 0.1,
            max_tokens: COMPACTION_MAX_CHECKPOINT_TOKENS,
            enable_streaming: false,
            enable_prompt_caching: false,
            sticky_session_routing: false,
            provider_sort: cfg.provider.provider_sort.clone(),
            reasoning_effort: None,
        };

        let llm_attempt = tokio::time::timeout(Duration::from_secs(45), async {
            let mut stream = self.provider.stream_chat(comp_req, cancel.clone()).await?;
            let mut summary = String::new();
            let mut usage = UsageStats::default();
            while let Some(item) = stream.next().await {
                match item? {
                    LlmDelta::Text(t) => summary.push_str(&t),
                    LlmDelta::Usage(u) => usage = u,
                    LlmDelta::Thinking(_) | LlmDelta::ToolCallComplete(_) | LlmDelta::Done { .. } => {}
                }
            }
            Ok::<(String, UsageStats), ZenkoError>((summary, usage))
        })
        .await;

        match llm_attempt {
            Ok(Ok((summary, usage))) if !summary.trim().is_empty() => {
                if usage.prompt_tokens > 0 || usage.completion_tokens > 0 || usage.cost_usd > 0.0 {
                    let _ = self
                        .store
                        .record_usage(session_id, &cfg.provider.kind, &comp_model, &usage);
                }
                let clean = summary.trim().to_string();
                let after =
                    prefix_tokens + plan.estimated_kept_tokens + estimate_text_tokens(&clean);
                CompactionCheckpointPayload {
                    version: 1,
                    summary: clean,
                    first_kept_event_id: plan.first_kept_event_id.clone(),
                    compacted_events_count: plan.compacted_events_count,
                    estimated_tokens_before: plan.estimated_tokens_before,
                    estimated_tokens_after: after,
                    model: comp_model,
                    created_at: Utc::now(),
                }
            }
            other => {
                if let Ok(Err(e)) = other {
                    warn!(session_id = %session_id, error = %e, "Stage-B compaction LLM failed; using deterministic fallback");
                }
                build_deterministic_checkpoint(
                    plan,
                    prior_checkpoint,
                    plan_summary.as_deref(),
                    prefix_tokens,
                )
            }
        }
    }

    /// Signals an active foreground `shell_exec` tool call on `session_id` to seamlessly
    /// hand off to the background job supervisor without losing output or killing the process.
    pub fn demote_foreground_job(&self, session_id: &str) -> bool {
        if let Ok(map) = self.demote_signals.lock()
            && let Some((_, notify)) = map.get(session_id) {
                notify.notify_waiters();
                return true;
            }
        false
    }

    /// Atomically persists a follow-up prompt into the durable SQLite `session_queue` and
    /// broadcasts the updated queue count so WebUI/CLI clients reflect it immediately.
    pub fn enqueue_session_followup(
        &self,
        session_id: &str,
        text: &str,
    ) -> Result<(QueuedPrompt, usize)> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(ZenkoError::Config("queued prompt cannot be empty".into()));
        }
        let (queued, count) = self.store.enqueue_followup(session_id, trimmed)?;
        self.emit(AgentEvent::QueueUpdated {
            session_id: session_id.to_string(),
            queued_count: count,
        });
        Ok((queued, count))
    }

    /// Cancels a persisted follow-up prompt from `session_queue` and broadcasts updated count.
    pub fn cancel_session_followup(
        &self,
        session_id: &str,
        queue_id: &str,
    ) -> Result<(bool, usize)> {
        let cancelled = self.store.cancel_queued_followup(session_id, queue_id)?;
        let count = self
            .store
            .list_queued_followups(session_id)
            .map(|v| v.len())
            .unwrap_or(0);
        if cancelled {
            self.emit(AgentEvent::QueueUpdated {
                session_id: session_id.to_string(),
                queued_count: count,
            });
        }
        Ok((cancelled, count))
    }

    /// Atomically removes a specific queued follow-up from `session_queue` and injects it as
    /// live steering guidance into the running turn (or returns `false` if already dispatched/idle).
    pub fn promote_queued_to_steer(
        &self,
        session_id: &str,
        queue_id: &str,
    ) -> Result<(bool, usize)> {
        if !self.is_session_running(session_id) {
            let count = self
                .store
                .list_queued_followups(session_id)
                .map(|v| v.len())
                .unwrap_or(0);
            return Ok((false, count));
        }
        let popped = self.store.pop_queued_followup_by_id(session_id, queue_id)?;
        let count = self
            .store
            .list_queued_followups(session_id)
            .map(|v| v.len())
            .unwrap_or(0);
        match popped {
            Some(item) => {
                let steered = self.steer_session(session_id, &item.text);
                if !steered {
                    // Session went idle between check and pop; re-enqueue so the prompt is never dropped.
                    let (_, new_count) = self.store.enqueue_followup(session_id, &item.text)?;
                    self.emit(AgentEvent::QueueUpdated {
                        session_id: session_id.to_string(),
                        queued_count: new_count,
                    });
                    return Ok((false, new_count));
                }
                self.emit(AgentEvent::QueueUpdated {
                    session_id: session_id.to_string(),
                    queued_count: count,
                });
                Ok((true, count))
            }
            None => Ok((false, count)),
        }
    }

    fn drain_steering(&self, session_id: &str) -> Vec<String> {
        let ctrl = self.session_control(session_id);
        ctrl.steer_queue
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }

    /// Queues an LLM-sovereign live orchestration directive (`posture`, `cognitive_tier`,
    /// `reasoning_effort`) onto the active session control block and emits live WebUI/CLI events.
    pub(crate) fn queue_orchestrate_control(
        &self,
        session_id: &str,
        req: &OrchestrateControlRequest,
    ) -> OrchestrateControlReceipt {
        let cfg = self.config();
        let ctrl = self.session_control(session_id);
        let model_locked = ctrl.explicit_model_locked.load(Ordering::Relaxed);
        let resolved_model = match (req.cognitive_tier, model_locked) {
            (Some(role), false) => {
                orchestrator::router::select_turn_model(&cfg, None, None, role, None)
                    .map(|dec| dec.model_id)
            }
            _ => None,
        };

        let has_shift = req.posture.is_some()
            || req.cognitive_tier.is_some()
            || req.reasoning_effort.is_some();
        if has_shift && let Ok(mut slot) = ctrl.pending_control.lock() {
            let entry = slot.get_or_insert_with(PendingControlShift::default);
            if req.posture.is_some() {
                entry.posture = req.posture;
            }
            if req.cognitive_tier.is_some() {
                entry.cognitive_tier = req.cognitive_tier;
            }
            if req.reasoning_effort.is_some() {
                entry.reasoning_effort = req.reasoning_effort;
            }
            if req.rationale.is_some() {
                entry.rationale = req.rationale.clone();
            }
        }

        if let Some(posture) = req.posture {
            let reason_suffix = req
                .rationale
                .as_deref()
                .map(|r| format!(" — {r}"))
                .unwrap_or_default();
            self.emit(AgentEvent::OrchestratorUpdated {
                session_id: session_id.to_string(),
                stage: ORCH_STAGE_POSTURE.into(),
                tier: zenko_core::ExecutionTier::Orchestrated,
                detail: format!("LLM posture -> `{}`{reason_suffix}", posture.as_str()),
                model: None,
            });
        }
        if req.cognitive_tier.is_some() || req.reasoning_effort.is_some() {
            let mut tags = Vec::new();
            if let Some(t) = req.cognitive_tier {
                tags.push(format!("tier={}", t.as_str()));
            }
            if let Some(e) = req.reasoning_effort {
                tags.push(format!("effort={}", e.as_str()));
            }
            if model_locked && req.cognitive_tier.is_some() {
                tags.push("model_locked".into());
            }
            self.emit(AgentEvent::OrchestratorUpdated {
                session_id: session_id.to_string(),
                stage: ORCH_STAGE_GEAR_SHIFT.into(),
                tier: zenko_core::ExecutionTier::Orchestrated,
                detail: format!("LLM gear shift ({})", tags.join(", ")),
                model: resolved_model.clone(),
            });
        }

        OrchestrateControlReceipt {
            posture: req.posture,
            cognitive_tier: req.cognitive_tier,
            resolved_model,
            reasoning_effort: req.reasoning_effort,
            recon_summary: None,
            rationale: req.rationale.clone(),
            model_locked_by_override: model_locked,
        }
    }

    fn take_pending_control(&self, session_id: &str) -> Option<PendingControlShift> {
        let ctrl = self.session_control(session_id);
        ctrl.pending_control
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }

    fn drain_job_notices(&self, session_id: &str) -> Vec<WakeupNotice> {
        let ctrl = self.session_control(session_id);
        ctrl.job_notices
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }

    fn has_job_notices(&self, session_id: &str) -> bool {
        let ctrl = self.session_control(session_id);
        ctrl.job_notices
            .lock()
            .map(|q| !q.is_empty())
            .unwrap_or(false)
    }

    /// Removes pending wakeup notices for subagents that the parent already awaited via `agent_wait`,
    /// preventing spurious delayed wakeup turns after synchronous collection.
    pub fn clear_subagent_wakeup_notices(&self, parent_session_id: &str, agent_ids: &[String]) {
        if agent_ids.is_empty() {
            return;
        }
        let ctrl = self.session_control(parent_session_id);
        if let Ok(mut q) = ctrl.job_notices.lock() {
            q.retain(|n| match n {
                WakeupNotice::Subagent { agent_id, .. } => !agent_ids.iter().any(|id| id == agent_id),
                WakeupNotice::ShellJob(_) => true,
            });
        }
    }

    pub fn cancel_session(&self, session_id: &str) -> bool {
        self.subagents.cancel_descendants(&self.store, session_id);
        if let Ok(map) = self.active_cancels.lock()
            && let Some((_, tok)) = map.get(session_id) {
                tok.cancel();
                return true;
            }
        false
    }

    /// Pushes a typed background-subagent completion notice into `parent_session_id`
    /// and signals the zero-polling coalesced wakeup loop.
    pub fn notify_subagent_finished(&self, parent_session_id: &str, rec: &SubagentRecord) {
        let wt_info = match (&rec.worktree_branch, rec.changed_files.is_empty()) {
            (Some(br), false) => format!(
                " Worktree branch `{br}` modified {} file(s): {}.",
                rec.changed_files.len(),
                rec.changed_files.join(", ")
            ),
            (Some(br), true) => format!(" Worktree branch `{br}` had no file changes."),
            (None, _) => String::new(),
        };
        let summary = compact_preview(rec.summary.as_deref().unwrap_or("(no summary)"), 400);
        let notice = format!(
            "Subagent `{}` (role={}, status={}) finished.{wt_info} Summary: {summary}",
            rec.id,
            rec.role,
            rec.status.as_str()
        );
        push_job_notice(
            &self.session_controls,
            parent_session_id,
            WakeupNotice::Subagent {
                agent_id: rec.id.clone(),
                summary: notice,
            },
        );
        let _ = self.wake_tx.send(parent_session_id.to_string());
    }

    pub(crate) fn has_configured_mcp_servers(&self) -> bool {
        self.mcp_servers_cfg
            .read()
            .map(|list| list.iter().any(|s| s.enabled))
            .unwrap_or(false)
    }

    pub(crate) fn effective_session_scope(&self, session_id: &str) -> SessionToolScope {
        let ctrl = self.session_control(session_id);
        if let Ok(slot) = ctrl.active_scope.lock()
            && let Some(scope) = slot.as_ref()
        {
            return scope.clone();
        }
        let sess = self.store.get_session(session_id).ok().flatten();
        let role_id = sess.as_ref().and_then(|s| s.agent_role.clone());
        let permission = sess
            .as_ref()
            .map(|s| s.permission_mode)
            .unwrap_or_else(|| self.config().policy.mode);
        let role_ro = role_id
            .as_deref()
            .and_then(find_subagent_role)
            .is_some_and(|r| r.read_only);
        SessionToolScope {
            role_id: role_id.clone(),
            permission,
            custom_read_only: false,
            custom_tool_subset: HashSet::new(),
            availability: ToolAvailabilityContext {
                has_mcp_servers: self.has_configured_mcp_servers(),
                is_subagent: role_id.is_some(),
                read_only: permission == PermissionMode::Plan || role_ro,
            },
        }
    }

    pub(crate) fn coerce_tool_call_args(&self, tool_name: &str, args: Value) -> Value {
        let mut coerced = match self.tools.get(tool_name) {
            Some(tool) => coerce_and_repair_args(args, &tool.spec().parameters),
            None => args,
        };
        // Also coerce inner `batch_tools` step payloads upfront so pre-mutation checkpoint
        // detection and role/read-only classifiers inspect clean normalized arguments.
        if tool_name == TOOL_BATCH_TOOLS
            && let Some(calls) = coerced.get_mut("calls").and_then(Value::as_array_mut)
        {
            for item in calls.iter_mut() {
                let inner_name = item
                    .get("tool")
                    .or_else(|| item.get("name"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or("")
                    .to_string();
                if inner_name.is_empty() || inner_name == TOOL_BATCH_TOOLS {
                    continue;
                }
                if let Some(inner_tool) = self.tools.get(&inner_name) {
                    let schema = inner_tool.spec().parameters;
                    let arg_key = match item.get("args").is_some() {
                        true => "args",
                        false => "arguments",
                    };
                    if let Some(obj) = item.as_object_mut()
                        && let Some(raw_inner) = obj.remove(arg_key)
                    {
                        obj.insert(
                            "args".to_string(),
                            coerce_and_repair_args(raw_inner, &schema),
                        );
                    }
                }
            }
        }
        coerced
    }

    pub fn cancel_all_sessions(&self) {
        if let Ok(map) = self.active_cancels.lock() {
            for (_, tok) in map.values() {
                tok.cancel();
            }
        }
    }

    pub fn resolve_approval(
        &self,
        approval_id: &str,
        decision: ApprovalDecision,
        resolved_by: &str,
    ) -> Result<Option<ApprovalRecord>> {
        let rec = self
            .store
            .resolve_approval(approval_id, decision, resolved_by)?;
        if let Some(ref r) = rec {
            if decision == ApprovalDecision::AlwaysAllow
                && let Ok(mut set) = self.always_allowed.write() {
                    set.insert(r.tool_name.clone());
                }
            if let Ok(mut map) = self.pending_approvals.lock()
                && let Some((_, tx)) = map.remove(approval_id) {
                    let _ = tx.send(decision);
                }
            self.emit(AgentEvent::ToolApprovalResolved {
                approval_id: approval_id.to_string(),
                session_id: r.session_id.clone(),
                decision,
                resolved_by: resolved_by.to_string(),
            });
        }
        Ok(rec)
    }

}

fn is_workspace_mutating_call(tc: &ToolCall) -> Option<&str> {
    let name = tc.name.as_str();
    match name {
        n if MUTATING_FILE_TOOLS.contains(&n) => Some(n),
        TOOL_BATCH_TOOLS => {
            let calls = tc.arguments.get("calls").and_then(Value::as_array)?;
            for item in calls {
                let inner_name = item
                    .get("tool")
                    .or_else(|| item.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if inner_name.is_empty() || inner_name == TOOL_BATCH_TOOLS {
                    continue;
                }
                let inner_tc = ToolCall {
                    id: String::new(),
                    name: inner_name.to_string(),
                    arguments: item
                        .get("args")
                        .or_else(|| item.get("arguments"))
                        .cloned()
                        .unwrap_or(Value::Null),
                };
                if is_workspace_mutating_call(&inner_tc).is_some() {
                    return Some(TOOL_BATCH_TOOLS);
                }
            }
            None
        }
        TOOL_SHELL_EXEC => {
            let cmd = tc
                .arguments
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("");
            match is_read_only_shell_command(cmd) {
                true => None,
                false => Some(name),
            }
        }
        TOOL_GIT => match is_git_tool_args_mutating(&tc.arguments) {
            true => Some(name),
            false => None,
        },
        TOOL_LSP => match is_lsp_tool_args_mutating(&tc.arguments) {
            true => Some(name),
            false => None,
        },
        TOOL_AGENT_WORKTREE => {
            let action = tc
                .arguments
                .get("action")
                .and_then(Value::as_str)
                .and_then(WorktreeAction::parse);
            match action {
                Some(WorktreeAction::Merge) => Some(name),
                _ => None,
            }
        }
        TOOL_SPAWN_AGENT => {
            let role = tc
                .arguments
                .get("role")
                .and_then(Value::as_str)
                .and_then(find_subagent_role);
            let ws_mode = tc
                .arguments
                .get("workspace")
                .and_then(Value::as_str)
                .map(SubagentWorkspaceMode::from_str_loose)
                .unwrap_or(SubagentWorkspaceMode::Inherit);
            match (role.map(|r| r.read_only), ws_mode) {
                (Some(false), SubagentWorkspaceMode::Inherit) => Some(name),
                _ => None,
            }
        }
        _ => None,
    }
}

fn agent_event_session_id(ev: &AgentEvent) -> Option<&str> {
    match ev {
        AgentEvent::SessionUpdated { session_id, .. }
        | AgentEvent::UserMessageAccepted { session_id, .. }
        | AgentEvent::ThinkingDelta { session_id, .. }
        | AgentEvent::TextDelta { session_id, .. }
        | AgentEvent::ToolCallStarted { session_id, .. }
        | AgentEvent::ToolApprovalResolved { session_id, .. }
        | AgentEvent::ToolCallFinished { session_id, .. }
        | AgentEvent::TurnCompleted { session_id, .. }
        | AgentEvent::TurnError { session_id, .. }
        | AgentEvent::SteeringAccepted { session_id, .. }
        | AgentEvent::QueueUpdated { session_id, .. }
        | AgentEvent::OrchestratorUpdated { session_id, .. }
        | AgentEvent::BrainstormRecorded { session_id, .. }
        | AgentEvent::PlanUpdated { session_id, .. }
        | AgentEvent::ClarificationResolved { session_id, .. }
        | AgentEvent::SessionTasksUpdated { session_id, .. }
        | AgentEvent::PlanApproved { session_id, .. } => Some(session_id.as_str()),
        AgentEvent::CheckpointCreated { checkpoint } => Some(checkpoint.session_id.as_str()),
        AgentEvent::SessionRewound { report } => Some(report.session_id.as_str()),
        AgentEvent::ToolApprovalRequested { approval } => Some(approval.session_id.as_str()),
        AgentEvent::ClarificationRequested { clarification } => Some(clarification.session_id.as_str()),
        AgentEvent::JobUpdated { job } => Some(job.session_id.as_str()),
        AgentEvent::JobOutputDelta { session_id, .. } => Some(session_id.as_str()),
        AgentEvent::SubagentSpawned { agent } | AgentEvent::SubagentCompleted { agent } => {
            Some(agent.parent_session_id.as_str())
        }
        AgentEvent::SubagentMessage { message } => Some(message.to_session_id.as_str()),
        AgentEvent::GoalLoopUpdated { goal } => Some(goal.session_id.as_str()),
        AgentEvent::GatewayPaired { .. }
        | AgentEvent::McpServersUpdated { .. }
        | AgentEvent::SkillUpdated { .. }
        | AgentEvent::Telemetry(_)
        | AgentEvent::Log(_) => None,
    }
}

fn append_cache_safe_reminder(messages: &mut Vec<ChatMessage>, reminder: String) {
    let wrapped = format!("<system_reminder>\n{reminder}\n</system_reminder>");
    match messages.last_mut() {
        Some(last) if last.role == EventRole::User => {
            if !last.content.is_empty() {
                last.content.push_str("\n\n");
            }
            last.content.push_str(&wrapped);
        }
        _ => {
            messages.push(ChatMessage {
                role: EventRole::User,
                content: wrapped,
                reasoning_content: None,
                tool_calls: Vec::new(),
                tool_call_id: None,
                name: None,
            });
        }
    }
}

fn events_to_chat_messages_refs(events: &[&SessionEvent], active_model: &str) -> Vec<ChatMessage> {
    let mut out = Vec::with_capacity(events.len());
    for ev in events.iter().copied() {
        match ev.role {
            EventRole::User => {
                let text = ev
                    .content_json
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                out.push(ChatMessage {
                    role: EventRole::User,
                    content: text,
                    reasoning_content: None,
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    name: None,
                });
            }
            EventRole::Assistant => {
                let text = ev
                    .content_json
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                // Only replay interleaved `reasoning_content` to the SAME model family that produced it
                // (prevents provider rejection after mid-turn model escalation!).
                let same_model = ev
                    .content_json
                    .get("model")
                    .and_then(Value::as_str)
                    .is_none_or(|m| m.is_empty() || m == active_model);
                let reasoning_content = match same_model {
                    true => ev
                        .content_json
                        .get("thinking")
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                        .map(String::from),
                    false => None,
                };
                let tool_calls: Vec<ToolCall> = ev
                    .content_json
                    .get("tool_calls")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                out.push(ChatMessage {
                    role: EventRole::Assistant,
                    content: text,
                    reasoning_content,
                    tool_calls,
                    tool_call_id: None,
                    name: None,
                });
            }
            EventRole::Tool => {
                let content = ev
                    .content_json
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let call_id = ev
                    .content_json
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .map(String::from);
                let name = ev
                    .content_json
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .map(String::from);
                out.push(ChatMessage {
                    role: EventRole::Tool,
                    content,
                    reasoning_content: None,
                    tool_calls: Vec::new(),
                    tool_call_id: call_id,
                    name,
                });
            }
            EventRole::System => {}
        }
    }
    out
}

fn make_session_title(prompt: &str) -> String {
    let clean: String = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    match (clean.is_empty(), clean.len() <= 48) {
        (true, _) => "New Session".to_string(),
        (false, true) => clean,
        (false, false) => {
            let cut = clean.floor_char_boundary(48);
            format!("{}...", &clean[..cut])
        }
    }
}

fn push_job_notice(
    controls: &Arc<Mutex<HashMap<String, Arc<SessionControl>>>>,
    session_id: &str,
    notice: WakeupNotice,
) {
    let ctrl = {
        let mut map = match controls.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        map.entry(session_id.to_string())
            .or_insert_with(|| Arc::new(SessionControl::default()))
            .clone()
    };
    if let Ok(mut q) = ctrl.job_notices.lock() {
        if q.len() >= SESSION_MAX_JOB_NOTICES {
            q.pop_front();
        }
        q.push_back(notice);
    }
}

fn build_script_command(script_path: &std::path::Path) -> String {
    let quoted = format!("'{}'", script_path.display().to_string().replace('\'', "'\\''"));
    let ext = script_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    match ext.as_deref() {
        Some("py") => format!("python3 {quoted}"),
        Some("js" | "mjs" | "cjs") => format!("node {quoted}"),
        _ => format!("{SHELL_BIN_UNIX} {quoted}"),
    }
}

