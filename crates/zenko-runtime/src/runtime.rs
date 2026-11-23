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
    PolicyVerdict, evaluate_policy, evaluate_verify_command_safety, is_tool_allowed_in_plan_schema,
    run_hooks,
};
use crate::scheduler::execute_tool_waves;
use crate::telemetry::{LogBuffer, current_rss_bytes};
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
    BackgroundJobRecord, BackgroundJobStatus, CHECKPOINT_LABEL_MAX_CHARS,
    CHECKPOINT_LIST_DEFAULT_LIMIT, COMPACTION_MAX_CHECKPOINT_TOKENS, ChatMessage, ChatRequest,
    ClarificationOption, CompactionCheckpointPayload, ContextBreakdown, ContextBridge, CronJob,
    CronKind, DEFAULT_APPROVAL_TIMEOUT_SECS, DEFAULT_CONTEXT_WINDOW_FALLBACK,
    DEFAULT_PAIRING_TTL_MINS, DaemonTelemetry, DreamRunRecord, DreamStatus, ESCALATION_OPT_PIVOT,
    ESCALATION_OPT_STOP, EventKind, EventRole, ExecutionPlan, ExportOptions, ExportReport,
    FILE_MODE_OWNER_ONLY, GOAL_LOOP_DEFAULT_INTERVAL_SECS, GOAL_LOOP_DEFAULT_MAX_BUDGET_USD,
    GOAL_LOOP_DEFAULT_MAX_ITERATIONS, GOAL_LOOP_DEFAULT_NOTIFY_CHANNEL,
    GOAL_LOOP_DEFAULT_STALL_LIMIT, GOAL_LOOP_LIST_DEFAULT_LIMIT, GOAL_LOOP_MAX_ITERATIONS_CAP,
    GOAL_LOOP_MAX_STALL_CAP, GOAL_LOOP_RUNS_DEFAULT_LIMIT, GoalLoopBridge,
    GoalLoopCreateRequest, GoalLoopPatch, GoalLoopRecord, GoalLoopRun, GoalLoopStatus,
    HUB_CATEGORIES, HUB_SUBDIR_SCRIPTS, HookEvent, HubEntry, IPC_FRAME_CHUNK, IPC_FRAME_DONE,
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
    MODEL_KV_PROVIDER_PREFIX, MODEL_KV_REGISTRY_KEY, MUTATING_FILE_TOOLS, McpServerConfig,
    McpServerStatus, ModelInfo, NOTE_KIND_BRAINSTORM_SPEC, NOTE_KIND_CLARIFICATION_DECISION,
    NOTE_KIND_DEFERRED_TOOLS, NOTE_KIND_EXECUTION_PLAN, NOTE_KIND_MEMORY_RECALL,
    NOTE_KIND_PLAN_APPROVED, NOTE_KIND_REWIND_SUMMARY, ORCH_MAX_TRUNCATION_RETRIES,
    ORCH_MONITORING_TOOLS, ORCH_REASONING_MAX_OUTPUT_CAP, ORCH_REASONING_MIN_MAX_TOKENS,
    ORCH_STAGE_BUDGET_GUARD,
    ORCH_STAGE_CHECKPOINT, ORCH_STAGE_CIRCUIT_BREAKER, ORCH_STAGE_COMPACTION, ORCH_STAGE_ESCALATED,
    ORCH_STAGE_INTENT_GATE, ORCH_STAGE_MICROCOMPACT, ORCH_STAGE_PACING, ORCH_STAGE_PHASE_GATE,
    ORCH_STAGE_PLAN_MODE, ORCH_STAGE_RECON, ORCH_STAGE_SYNTHESIS, ORCH_STAGE_TOOL_ACTIVATED,
    ORCH_STAGE_TRIAGED, ORCH_STAGE_TRUNCATION_RECOVERY, ORCH_STAGE_VERIFICATION_GATE,
    OrchestratorBridge, PeerRole, PermissionMode, QueuedPrompt, Result, RewindMode, RewindRequest,
    SESSION_MAX_JOB_NOTICES, SHELL_BIN_UNIX, STEERING_PREVIEW_CHARS, SUBAGENT_DEFAULT_WAIT_SECS,
    SUBAGENT_LIST_DEFAULT_LIMIT, SUBAGENT_MAX_WAIT_SECS, SUBAGENT_SCHEMA_MAX_RETRIES, SecretVault,
    SendMode, Session, SessionDispatchState, SessionEvent, SessionSearchFilter, SessionStatus,
    SessionTask, AutoDispatchDecision, resolve_auto_send_mode,
    SessionTaskPatch, SessionTaskStatus, SkillAuthor, SkillManageAction, SkillStatus, SkillsConfig,
    SubagentBridge, SubagentRecord, SubagentSpawnRequest, SubagentWorkspaceMode,
    TOOL_AGENT_WORKTREE, TOOL_ASK_CLARIFICATION, TOOL_GIT, TOOL_MEMORY, TOOL_PLAN_UPDATE,
    TOOL_RECORD_BRAINSTORM, TOOL_SESSION_TASK, TOOL_SHELL_EXEC, TOOL_SKILL_MANAGE,
    TOOL_SPAWN_AGENT, TOOL_TOOL_SEARCH,
    TaskKind, ToolCall, ToolConcurrency, ToolContext, ToolOutput, ToolSpec, TriageReport,
    TriggerRateLimiter, TriggerStatusEntry, UsageStats, WebSearchConfig, WorktreeAction,
    WorktreeDiffReport, WorktreePruneReport, ZenkoConfig, ZenkoError, compact_preview,
    find_provider, find_subagent_role, is_git_tool_args_mutating, is_memory_mutating_action,
    new_goal_loop_id, new_id, new_session_task_id,
};
use zenko_memory::{
    SnapshotManager, SqliteStore, WorkspaceSnapshot, filter_query_terms, parse_since_window,
    project_scope_for_workspace, resolve_git_root,
};
use zenko_providers::UnifiedLlmProvider;
use zenko_tools::{
    ToolRegistry, lsp::LspPoolManager, mcp_client::McpClientManager, memory_cron::MemoryTool,
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

#[derive(Default)]
struct SessionControl {
    steer_queue: Mutex<VecDeque<String>>,
    job_notices: Mutex<VecDeque<WakeupNotice>>,
    active_tools: Mutex<HashMap<String, usize>>,
}

/// RAII multiset counter tracking in-flight tool names on a session (cleanup guaranteed on cancel/error).
struct ActiveToolGuard {
    ctrl: Arc<SessionControl>,
    tool_name: String,
}

impl ActiveToolGuard {
    fn enter(ctrl: Arc<SessionControl>, tool_name: String) -> Self {
        if let Ok(mut map) = ctrl.active_tools.lock() {
            *map.entry(tool_name.clone()).or_insert(0) += 1;
        }
        Self { ctrl, tool_name }
    }
}

impl Drop for ActiveToolGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = self.ctrl.active_tools.lock() {
            if let Some(count) = map.get_mut(&self.tool_name) {
                match *count <= 1 {
                    true => {
                        map.remove(&self.tool_name);
                    }
                    false => *count -= 1,
                }
            }
        }
    }
}

struct ExecutedToolCall {
    tool_call_id: String,
    tool_name: String,
    arguments: Value,
    output: ToolOutput,
    duration_ms: u64,
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
struct RuntimeContextBridge {
    runtime: OnceLock<Weak<AgentRuntime>>,
}

impl RuntimeContextBridge {
    fn new() -> Self {
        Self {
            runtime: OnceLock::new(),
        }
    }

    fn bind(&self, weak: Weak<AgentRuntime>) {
        let _ = self.runtime.set(weak);
    }

    fn upgrade(&self) -> Option<Arc<AgentRuntime>> {
        self.runtime.get().and_then(Weak::upgrade)
    }
}

#[async_trait]
impl ContextBridge for RuntimeContextBridge {
    fn search_and_activate_deferred_tools(
        &self,
        session_id: &str,
        query: &str,
        limit: usize,
    ) -> Vec<ToolSpec> {
        let Some(rt) = self.upgrade() else {
            return Vec::new();
        };
        let matched = rt.tools.search_deferred_bm25(query, limit);
        let names: Vec<String> = matched.iter().map(|s| s.name.clone()).collect();
        let newly = rt.activate_deferred_tools(session_id, &names);
        if !newly.is_empty() {
            rt.emit(AgentEvent::OrchestratorUpdated {
                session_id: session_id.to_string(),
                stage: ORCH_STAGE_TOOL_ACTIVATED.into(),
                tier: zenko_core::ExecutionTier::Standard,
                detail: format!("Activated deferred tool(s): {}", newly.join(", ")),
                model: None,
            });
        }
        matched
    }

    async fn inspect_session_context(&self, session_id: &str) -> Result<ContextBreakdown> {
        let Some(rt) = self.upgrade() else {
            return Err(ZenkoError::Config(
                "runtime context bridge unavailable".into(),
            ));
        };
        rt.inspect_session_context(session_id).await
    }
}

/// Leak-free `Weak<AgentRuntime>` bridge connecting Phase 5 subagent tools (`spawn_agent`,
/// `agent_list`, `agent_message`, `agent_wait`, `agent_kill`) to `SubagentSupervisor`.
struct RuntimeSubagentBridge {
    runtime: OnceLock<Weak<AgentRuntime>>,
}

impl RuntimeSubagentBridge {
    fn new() -> Self {
        Self {
            runtime: OnceLock::new(),
        }
    }

    fn bind(&self, weak: Weak<AgentRuntime>) {
        let _ = self.runtime.set(weak);
    }

    fn upgrade(&self) -> Result<Arc<AgentRuntime>> {
        self.runtime
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| ZenkoError::Config("runtime subagent bridge unavailable".into()))
    }
}

#[async_trait]
impl SubagentBridge for RuntimeSubagentBridge {
    async fn spawn_agent(&self, req: SubagentSpawnRequest) -> Result<SubagentRecord> {
        let rt = self.upgrade()?;
        rt.subagents.spawn_agent(&rt, req).await
    }

    fn list_agents(
        &self,
        parent_session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SubagentRecord>> {
        let rt = self.upgrade()?;
        rt.store.list_subagents(parent_session_id, limit)
    }

    fn get_agent(&self, agent_id: &str) -> Result<Option<SubagentRecord>> {
        let rt = self.upgrade()?;
        rt.store.get_subagent(agent_id)
    }

    async fn send_message(
        &self,
        from_session_id: &str,
        to_session_id: &str,
        message: &str,
        opts: zenko_core::AgentMessageOptions,
    ) -> Result<zenko_core::AgentMailMessage> {
        let rt = self.upgrade()?;
        rt.subagents
            .send_message(&rt, from_session_id, to_session_id, message, opts)
            .await
    }

    async fn wait_agents(
        &self,
        parent_session_id: &str,
        agent_ids: &[String],
        wait_all: bool,
        timeout: Duration,
    ) -> Result<Vec<SubagentRecord>> {
        let rt = self.upgrade()?;
        rt.subagents
            .wait_agents(&rt, parent_session_id, agent_ids, wait_all, timeout)
            .await
    }

    async fn kill_agent(
        &self,
        caller_session_id: &str,
        agent_id: &str,
        remove_worktree: bool,
    ) -> Result<Option<SubagentRecord>> {
        let rt = self.upgrade()?;
        rt.subagents
            .kill_agent(&rt, caller_session_id, agent_id, remove_worktree)
            .await
    }

    async fn inspect_worktree_diff(
        &self,
        caller_session_id: &str,
        agent_id: &str,
    ) -> Result<Option<WorktreeDiffReport>> {
        let rt = self.upgrade()?;
        rt.subagents
            .inspect_worktree_diff(&rt, caller_session_id, agent_id)
            .await
    }

    async fn merge_worktree(
        &self,
        caller_session_id: &str,
        agent_id: &str,
        cleanup: bool,
    ) -> Result<WorktreeDiffReport> {
        let rt = self.upgrade()?;
        rt.subagents
            .merge_worktree(&rt, caller_session_id, agent_id, cleanup)
            .await
    }

    async fn discard_worktree(
        &self,
        caller_session_id: &str,
        agent_id: &str,
    ) -> Result<WorktreeDiffReport> {
        let rt = self.upgrade()?;
        rt.subagents
            .discard_worktree(&rt, caller_session_id, agent_id)
            .await
    }

    async fn prune_orphan_worktrees(&self) -> Result<WorktreePruneReport> {
        let rt = self.upgrade()?;
        rt.subagents.prune_orphan_worktrees(&rt).await
    }
}

/// Leak-free `Weak<AgentRuntime>` bridge connecting Phase 8 `goal_loop` tool to Goal Loop CRUD & execution.
struct RuntimeGoalLoopBridge {
    runtime: OnceLock<Weak<AgentRuntime>>,
}

impl RuntimeGoalLoopBridge {
    fn new() -> Self {
        Self {
            runtime: OnceLock::new(),
        }
    }

    fn bind(&self, weak: Weak<AgentRuntime>) {
        let _ = self.runtime.set(weak);
    }

    fn upgrade(&self) -> Result<Arc<AgentRuntime>> {
        self.runtime
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| ZenkoError::Config("runtime goal loop bridge unavailable".into()))
    }
}

#[async_trait]
impl GoalLoopBridge for RuntimeGoalLoopBridge {
    fn list_goals(&self, limit: usize) -> Result<Vec<GoalLoopRecord>> {
        let rt = self.upgrade()?;
        rt.store.list_goal_loops(limit)
    }

    fn get_goal(&self, id: &str) -> Result<Option<(GoalLoopRecord, Vec<GoalLoopRun>)>> {
        let rt = self.upgrade()?;
        match rt.store.get_goal_loop(id)? {
            Some(g) => {
                let runs = rt
                    .store
                    .list_goal_loop_runs(&g.id, GOAL_LOOP_RUNS_DEFAULT_LIMIT)
                    .unwrap_or_default();
                Ok(Some((g, runs)))
            }
            None => Ok(None),
        }
    }

    fn create_goal(&self, req: GoalLoopCreateRequest) -> Result<GoalLoopRecord> {
        let rt = self.upgrade()?;
        rt.create_goal_loop(req)
    }

    fn update_goal(&self, id: &str, patch: GoalLoopPatch) -> Result<Option<GoalLoopRecord>> {
        let rt = self.upgrade()?;
        rt.update_goal_loop(id, patch)
    }

    async fn trigger_goal_now(&self, id: &str) -> Result<GoalLoopRecord> {
        let rt = self.upgrade()?;
        crate::goals::execute_goal_iteration(&rt, id, CancellationToken::new()).await
    }

    fn delete_goal(&self, id: &str) -> Result<bool> {
        let rt = self.upgrade()?;
        rt.delete_goal_loop(id)
    }
}

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
        if let Ok(Some(raw_reg)) = store.get_kv(MODEL_KV_REGISTRY_KEY) {
            if let Ok(parsed) = serde_json::from_str::<Value>(&raw_reg) {
                provider.seed_metadata_registry(parsed);
            }
        }

        let (events, _) = broadcast::channel(512);
        let logs = LogBuffer::new(300, events.clone());
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
        subagent_bridge.bind(Arc::downgrade(&rt));
        goal_bridge.bind(Arc::downgrade(&rt));

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let art_dir = data_dir.clone();
            let cp_mgr = rt.checkpoints.clone();
            let cp_cfg = rt.config().checkpoints;
            handle.spawn(async move {
                artifacts::prune_old_artifacts(&art_dir).await;
                if cp_cfg.enabled {
                    let _ = cp_mgr.prune_checkpoints(&cp_cfg).await;
                }
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
        } else if sync_now {
            if let Err(e) = self.mcp_clients.sync_server(&target_name, &self.tools).await {
                sync_err = Some(e.to_string());
            }
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

    /// Low-RAM periodic idle reaper evicting idle LSP servers and external MCP client sessions,
    /// plus refreshing any MCP servers that emitted `notifications/tools/list_changed`.
    pub async fn run_external_tool_reaper(self: Arc<Self>, cancel: CancellationToken) {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = interval.tick() => {
                    for dirty_name in self.mcp_clients.dirty_servers().await {
                        let _ = self.sync_mcp_server(&dirty_name).await;
                    }
                    let _ = self.lsp.evict_idle().await;
                    let _ = self.mcp_clients.evict_idle().await;
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

        if is_active && !force_refresh {
            if let Ok(guard) = self.cached_models.lock() {
                if let Some((ts, models)) = guard.as_ref() {
                    if ts.elapsed() < Duration::from_secs(MODEL_CACHE_TTL_SECS)
                        && !models.is_empty()
                    {
                        return Ok(models.clone());
                    }
                }
            }
        }

        let effective_url = base_url_override
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .unwrap_or(match is_active {
                true => cfg.provider.base_url.as_str(),
                false => desc.default_base_url,
            });
        let api_key = self.vault.get(desc.vault_secret_key).ok().flatten();

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
                if is_active {
                    if let Ok(mut guard) = self.cached_models.lock() {
                        *guard = Some((Instant::now(), models.clone()));
                    }
                }
                if let Ok(serialized) = serde_json::to_string(&models) {
                    let _ = self.store.set_kv(&kv_key, &serialized);
                }
                if let Some(reg) = self.provider.load_metadata_registry(false).await {
                    if let Ok(reg_str) = serde_json::to_string(&reg) {
                        let _ = self.store.set_kv(MODEL_KV_REGISTRY_KEY, &reg_str);
                    }
                }
                Ok(models)
            }
            Err(err) => {
                if let Ok(Some(cached_json)) = self.store.get_kv(&kv_key) {
                    if let Ok(cached_models) = serde_json::from_str::<Vec<ModelInfo>>(&cached_json)
                    {
                        if !cached_models.is_empty() {
                            return Ok(cached_models);
                        }
                    }
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

    fn session_control(&self, session_id: &str) -> Arc<SessionControl> {
        let mut map = match self.session_controls.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        map.entry(session_id.to_string())
            .or_insert_with(|| Arc::new(SessionControl::default()))
            .clone()
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
            {
                if let Some(arr) = ev.content_json.get("tools").and_then(Value::as_array) {
                    for name in arr.iter().filter_map(Value::as_str) {
                        if self.tools.is_deferred(name) {
                            set.insert(name.to_string());
                        }
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

    fn activate_deferred_tools(&self, session_id: &str, names: &[String]) -> Vec<String> {
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
        if let Ok(map) = self.frozen_snapshots.lock() {
            if let Some((cached_ws, snap)) = map.get(session_id) {
                if cached_ws == workspace_dir {
                    return Ok(snap.clone());
                }
            }
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
                if map.len() > 64 {
                    if let Some(evict_key) = map
                        .keys()
                        .find(|k| k.as_str() != session_id && !active_sids.contains(k.as_str()))
                        .cloned()
                    {
                        map.remove(&evict_key);
                    }
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

        let activated = self.hydrate_activated_deferred(session_id, &branch);
        let active_specs = self
            .tools
            .specs_for_session(cfg.context.deferred_tool_loading, &activated);
        let (eager_tools_count, deferred_available_count) = self.tools.exposure_counts();
        let deferred_catalog = match cfg.context.deferred_tool_loading {
            true => self.tools.deferred_catalog_text(),
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
        if let Ok(map) = self.demote_signals.lock() {
            if let Some((_, notify)) = map.get(session_id) {
                notify.notify_waiters();
                return true;
            }
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
                self.emit(AgentEvent::QueueUpdated {
                    session_id: session_id.to_string(),
                    queued_count: count,
                });
                let steered = self.steer_session(session_id, &item.text);
                Ok((steered, count))
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
        if let Ok(map) = self.active_cancels.lock() {
            if let Some((_, tok)) = map.get(session_id) {
                tok.cancel();
                return true;
            }
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
            if decision == ApprovalDecision::AlwaysAllow {
                if let Ok(mut set) = self.always_allowed.write() {
                    set.insert(r.tool_name.clone());
                }
            }
            if let Ok(mut map) = self.pending_approvals.lock() {
                if let Some((_, tx)) = map.remove(approval_id) {
                    let _ = tx.send(decision);
                }
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

    /// Convenience wrapper running a streaming ReAct turn with default options.
    pub async fn run_turn(
        self: &Arc<Self>,
        session_id: &str,
        user_text: &str,
        parent_event_override: Option<Option<String>>,
    ) -> Result<String> {
        let res = self
            .run_turn_with_options(
                session_id,
                user_text,
                TurnOptions {
                    parent_event_override,
                    ..TurnOptions::default()
                },
            )
            .await?;
        Ok(res.reply)
    }

    /// Runs a full streaming ReAct turn with per-session serialization, generational
    /// cancellation safety, context compaction, and optional headless/workspace/mode overrides.
    pub async fn run_turn_with_options(
        self: &Arc<Self>,
        session_id: &str,
        user_text: &str,
        opts: TurnOptions,
    ) -> Result<TurnResult> {
        let cfg = self.config();

        if cfg.general.daily_spend_limit_usd > 0.0 {
            let today = self.store.daily_cost_usd().unwrap_or(0.0);
            if today >= cfg.general.daily_spend_limit_usd {
                return Err(ZenkoError::BudgetExceeded(format!(
                    "Daily spend limit (${:.2}) reached (today: ${:.4})",
                    cfg.general.daily_spend_limit_usd, today
                )));
            }
        }

        // Non-destructive concurrency control (algebraic dispatch via `resolve_session_dispatch`):
        // - Auto: Queue during focused foreground work, Steer while managing bg jobs / subagents
        // - Steer: inject guidance mid-flight if a turn is active (otherwise starts normally)
        // - Queue: persist into durable SQLite `session_queue` if busy (auto-runs FIFO on completion)
        // - Interrupt: explicitly cancel active turn and start immediately
        match opts.send_mode {
            SendMode::Interrupt => {
                self.cancel_session(session_id);
            }
            mode => match self.resolve_session_dispatch(session_id, mode) {
                AutoDispatchDecision::Steer => {
                    if self.steer_session(session_id, user_text) {
                        return Ok(TurnResult {
                            session_id: session_id.to_string(),
                            reply: "Steering guidance queued for active turn.".into(),
                            model: cfg.provider.lead_model,
                            usage: UsageStats::default(),
                            tool_calls_count: 0,
                        });
                    }
                }
                AutoDispatchDecision::Queue => {
                    let (queued, count) = self.enqueue_session_followup(session_id, user_text)?;
                    return Ok(TurnResult {
                        session_id: session_id.to_string(),
                        reply: format!(
                            "Queued follow-up prompt (#{count}, id={}). It will run automatically after the current turn.",
                            queued.id
                        ),
                        model: cfg.provider.lead_model,
                        usage: UsageStats::default(),
                        tool_calls_count: 0,
                    });
                }
                AutoDispatchDecision::StartTurn => {}
            },
        }

        let headless_for_drain = opts.headless_no_wait;
        let result = self
            .run_single_turn(&cfg, session_id, user_text, opts)
            .await;

        if result.is_ok() {
            let has_queued = self
                .store
                .list_queued_followups(session_id)
                .map(|v| !v.is_empty())
                .unwrap_or(false);
            match (has_queued, self.has_job_notices(session_id)) {
                (true, _) => {
                    let rt = self.clone();
                    let sid = session_id.to_string();
                    tokio::spawn(async move {
                        rt.drain_followup_queue(&sid, headless_for_drain).await;
                    });
                }
                (false, true) => {
                    let _ = self.wake_tx.send(session_id.to_string());
                }
                (false, false) => {}
            }
        }

        result
    }

    async fn drain_followup_queue(self: &Arc<Self>, session_id: &str, headless_no_wait: bool) {
        while let Ok(Some(next_prompt)) = self.store.pop_next_followup(session_id) {
            let remaining = self
                .store
                .list_queued_followups(session_id)
                .map(|v| v.len())
                .unwrap_or(0);
            self.emit(AgentEvent::QueueUpdated {
                session_id: session_id.to_string(),
                queued_count: remaining,
            });
            let cfg = self.config();
            match self
                .run_single_turn(
                    &cfg,
                    session_id,
                    &next_prompt.text,
                    TurnOptions {
                        headless_no_wait,
                        send_mode: SendMode::Queue,
                        ..TurnOptions::default()
                    },
                )
                .await
            {
                Ok(_) => {}
                Err(e) => {
                    warn!(session_id = %session_id, error = %e, "queued follow-up turn failed");
                    break;
                }
            }
        }
        if self.has_job_notices(session_id) {
            let _ = self.wake_tx.send(session_id.to_string());
        }
    }

    async fn run_single_turn(
        self: &Arc<Self>,
        cfg: &ZenkoConfig,
        session_id: &str,
        user_text: &str,
        opts: TurnOptions,
    ) -> Result<TurnResult> {
        let session_lock = self.session_mutex(session_id);
        let _turn_guard = session_lock.lock().await;

        let mut session = self
            .store
            .get_session(session_id)?
            .ok_or_else(|| ZenkoError::NotFound(format!("session {session_id} not found")))?;

        let my_gen = self.turn_seq.fetch_add(1, Ordering::Relaxed);
        let cancel = opts
            .external_cancel
            .as_ref()
            .map(CancellationToken::child_token)
            .unwrap_or_else(CancellationToken::new);
        let demote_bg = Arc::new(Notify::new());
        {
            let mut map = self
                .active_cancels
                .lock()
                .map_err(|_| ZenkoError::Config("cancel lock poisoned".into()))?;
            map.insert(session_id.to_string(), (my_gen, cancel.clone()));
        }
        if let Ok(mut dmap) = self.demote_signals.lock() {
            dmap.insert(session_id.to_string(), (my_gen, demote_bg.clone()));
        }

        let mut turn_checkpoint_id: Option<String> = None;
        let result = self
            .execute_turn_inner(
                cfg,
                &mut session,
                user_text,
                opts,
                cancel,
                demote_bg,
                &mut turn_checkpoint_id,
            )
            .await;

        // Always finalize pre-mutation workspace checkpoint regardless of Ok/Err/Cancel so
        // partial mutations never leave `post_ref = None` or break hand-edit detection!
        if let Some(ref cp_id) = turn_checkpoint_id {
            let _ = self
                .checkpoints
                .finalize_checkpoint(&cfg.checkpoints, cp_id)
                .await;
        }

        // Generational cleanup: only remove our own tokens so a newer turn is never clobbered.
        if let Ok(mut map) = self.active_cancels.lock() {
            if map.get(session_id).map(|(g, _)| *g) == Some(my_gen) {
                map.remove(session_id);
            }
        }
        if let Ok(mut dmap) = self.demote_signals.lock() {
            if dmap.get(session_id).map(|(g, _)| *g) == Some(my_gen) {
                dmap.remove(session_id);
            }
        }

        match &result {
            Ok(_) | Err(ZenkoError::Cancelled) => {
                let _ = self.store.update_session_status(session_id, SessionStatus::Idle);
                self.emit(AgentEvent::SessionUpdated {
                    session_id: session_id.to_string(),
                    status: SessionStatus::Idle,
                    title: None,
                });
            }
            Err(e) => {
                let _ = self.store.update_session_status(session_id, SessionStatus::Error);
                self.emit(AgentEvent::TurnError {
                    session_id: session_id.to_string(),
                    message: e.to_string(),
                });
                self.emit(AgentEvent::SessionUpdated {
                    session_id: session_id.to_string(),
                    status: SessionStatus::Error,
                    title: None,
                });
            }
        }

        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_turn_inner(
        self: &Arc<Self>,
        cfg: &ZenkoConfig,
        session: &mut Session,
        user_text: &str,
        opts: TurnOptions,
        cancel: CancellationToken,
        demote_bg: Arc<Notify>,
        turn_checkpoint_id: &mut Option<String>,
    ) -> Result<TurnResult> {
        let session_id = session.id.clone();
        self.store.update_session_status(&session_id, SessionStatus::Running)?;

        let is_first_turn = session.head_event_id.is_none();
        if is_first_turn {
            let _ = run_hooks(
                &cfg.hooks,
                HookEvent::SessionStart,
                None,
                json!({ "session_id": session_id, "channel": session.channel }),
            )
            .await;
        }

        let new_title = match is_first_turn
            && (session.title == "New Session"
                || session.title.starts_with("Session ")
                || session.title == "CLI Session")
        {
            true => {
                let t = make_session_title(user_text);
                self.store.update_session_meta(&session_id, Some(&t), None, None)?;
                session.title = t.clone();
                Some(t)
            }
            false => None,
        };

        self.emit(AgentEvent::SessionUpdated {
            session_id: session_id.clone(),
            status: SessionStatus::Running,
            title: new_title,
        });

        if let Err(reason) = run_hooks(
            &cfg.hooks,
            HookEvent::UserPromptSubmit,
            None,
            json!({ "session_id": session_id, "text": user_text }),
        )
        .await
        {
            return Err(ZenkoError::PolicyDenied(reason));
        }

        let workspace_dir = opts
            .workspace_override
            .clone()
            .or_else(|| {
                session
                    .workspace_dir
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| cfg.resolved_workspace_dir());
        let canonical_ws_str = resolve_git_root(&workspace_dir).display().to_string();
        if session.workspace_dir.as_deref() != Some(canonical_ws_str.as_str()) {
            let _ = self.store.set_session_workspace(&session_id, &canonical_ws_str);
            session.workspace_dir = Some(canonical_ws_str);
        }
        let project_scope = project_scope_for_workspace(&workspace_dir);
        let snapshot = self.frozen_snapshot_for_session(&session_id, &workspace_dir, cfg)?;
        if session.frozen_system_hash.is_none() {
            let _ = self
                .store
                .set_session_frozen_hash(&session_id, &snapshot.tier1_hash);
        }

        let parent_for_user = match opts.parent_event_override {
            Some(explicit_parent) => explicit_parent,
            None => session.head_event_id.clone(),
        };
        let user_ev = self.store.append_event(
            &session_id,
            parent_for_user.as_deref(),
            EventRole::User,
            EventKind::Message,
            json!({ "text": user_text }),
            &UsageStats::default(),
        )?;
        let mut head_id = Some(user_ev.id.clone());

        self.emit(AgentEvent::UserMessageAccepted {
            session_id: session_id.clone(),
            event_id: user_ev.id,
            text: user_text.to_string(),
        });

        let effective_permission = opts.permission_override.unwrap_or(session.permission_mode);
        let initial_branch = self
            .store
            .get_active_branch(&session_id)
            .unwrap_or_default();
        let prior_turn_errors = initial_branch
            .iter()
            .rev()
            .take(8)
            .filter(|ev| {
                ev.role == EventRole::Tool
                    && ev
                        .content_json
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
            })
            .count();

        let mut orch = orchestrator::prepare_turn(
            cfg,
            &session_id,
            user_text,
            workspace_dir.clone(),
            cfg.resolved_hub_dir(),
            effective_permission,
            is_first_turn,
            prior_turn_errors,
            opts.model_override.as_deref(),
            session.model_override.as_deref(),
            opts.headless_no_wait,
            self.store.clone(),
            self.jobs.clone(),
        )
        .await;

        // Multi-turn plan continuity: hydrate durable SQLite `session_tasks` DAG first
        // (surviving compaction & daemon restarts!), falling back to latest event-branch plan.
        let persisted_tasks = self
            .clarifications
            .list_session_tasks(&session_id)
            .unwrap_or_default();
        if let Some(sqlite_plan) = execution_plan_from_session_tasks(&session.title, &persisted_tasks) {
            if !orchestrator::incomplete_steps(&sqlite_plan).is_empty() {
                orch.guard.hydrate_active_plan(sqlite_plan);
            }
        } else if let Some(carried_plan) = load_latest_active_plan(&initial_branch) {
            orch.guard.hydrate_active_plan(carried_plan);
        }
        // Hydrate latest abandoned-branch rewind summary if the session was just rewound
        let latest_rewind_summary: Option<String> = initial_branch
            .iter()
            .rev()
            .take(3)
            .find_map(|ev| {
                if ev.role == EventRole::System
                    && ev.kind == EventKind::SystemNote
                    && ev.content_json.get("note_kind").and_then(Value::as_str)
                        == Some(NOTE_KIND_REWIND_SUMMARY)
                {
                    ev.content_json
                        .get("summary")
                        .and_then(Value::as_str)
                        .map(String::from)
                } else {
                    None
                }
            });
        // Hydrate session-scoped activated deferred tools from prior turns
        let _ = self.hydrate_activated_deferred(&session_id, &initial_branch);

        // Oh My Pi cache-safe First-Turn Auto-Recall for Standard turns (Orchestrated turns
        // already recall inside parallel recon). Persisted as `NOTE_KIND_MEMORY_RECALL` once.
        let mut first_turn_recall_block: Option<String> = None;
        if is_first_turn
            && orch.recon.is_none()
            && orch.triage.tier != zenko_core::ExecutionTier::Instant
            && cfg.memory.workspace_scoping
        {
            let already_recalled = initial_branch.iter().any(|ev| {
                ev.role == EventRole::System
                    && ev.kind == EventKind::SystemNote
                    && ev.content_json.get("note_kind").and_then(Value::as_str)
                        == Some(NOTE_KIND_MEMORY_RECALL)
            });
            if !already_recalled {
                let q = filter_query_terms(user_text)
                    .into_iter()
                    .take(8)
                    .collect::<Vec<_>>()
                    .join(" ");
                if !q.is_empty() {
                    let ws_check = match cfg.memory.verify_file_staleness {
                        true => Some(workspace_dir.as_path()),
                        false => None,
                    };
                    if let Ok(hits) = self.store.search_memory_hybrid(
                        &q,
                        Some(&project_scope),
                        true,
                        ws_check,
                        MEMORY_AUTO_RECALL_LIMIT,
                    ) {
                        let strong: Vec<_> = hits
                            .into_iter()
                            .filter(|h| h.score >= MEMORY_AUTO_RECALL_MIN_SCORE)
                            .collect();
                        if !strong.is_empty() {
                            let lines: Vec<String> = strong
                                .iter()
                                .map(|h| {
                                    let stale = match h.stale_paths.is_empty() {
                                        true => String::new(),
                                        false => format!(
                                            " [stale-path: {}]",
                                            h.stale_paths.join(", ")
                                        ),
                                    };
                                    format!(
                                        "- [{}][{}] {}: {}{}",
                                        h.entry.scope,
                                        h.entry.kind,
                                        h.entry.title,
                                        compact_preview(&h.entry.content, 120),
                                        stale
                                    )
                                })
                                .collect();
                            let block = lines.join("\n");
                            if let Ok(rec_ev) = self.store.append_event(
                                &session_id,
                                head_id.as_deref(),
                                EventRole::System,
                                EventKind::SystemNote,
                                json!({
                                    "note_kind": NOTE_KIND_MEMORY_RECALL,
                                    "scope": &project_scope,
                                    "summary": &block,
                                }),
                                &UsageStats::default(),
                            ) {
                                head_id = Some(rec_ev.id);
                            }
                            first_turn_recall_block = Some(block);
                        }
                    }
                }
            }
        }

        let mut model = match orch.route.as_ref() {
            Some(dec) => dec.model_id.clone(),
            None => self
                .list_models_cached()
                .await
                .ok()
                .and_then(|list| list.into_iter().next().map(|m| m.id))
                .ok_or_else(|| {
                    ZenkoError::Config(
                        "No model selected and live model discovery returned empty. Choose a model in Settings or `zenko setup`.".into(),
                    )
                })?,
        };
        let mut reasoning_effort = orch.reasoning_effort;

        if cfg.orchestrator.enabled {
            let route_tag = orch.route.as_ref().map(|r| r.source).unwrap_or("discovered");
            self.emit(AgentEvent::OrchestratorUpdated {
                session_id: session_id.clone(),
                stage: ORCH_STAGE_TRIAGED.into(),
                tier: orch.triage.tier,
                detail: format!("{} (route={route_tag})", orch.triage.reason),
                model: Some(model.clone()),
            });
            if let Some(ref rec) = orch.recon {
                self.emit(AgentEvent::OrchestratorUpdated {
                    session_id: session_id.clone(),
                    stage: ORCH_STAGE_RECON.into(),
                    tier: orch.triage.tier,
                    detail: format!(
                        "{} entries, {} markers, {} prompt files, {} memory hits ({}ms)",
                        rec.workspace_entries.len(),
                        rec.repo_markers.len(),
                        rec.mentioned_files.len(),
                        rec.memory_hits.len(),
                        rec.elapsed_ms
                    ),
                    model: None,
                });
            }
        }

        let mut context_window = self.resolve_context_window(&model);
        let mut active_max_tokens = self.effective_output_budget(
            cfg.provider.max_tokens,
            &model,
            cfg.orchestrator.enabled && orch.triage.tier == zenko_core::ExecutionTier::Orchestrated,
        );
        let active_role = opts
            .role_id
            .as_deref()
            .or(session.agent_role.as_deref())
            .and_then(find_subagent_role);
        let deferred_catalog = match cfg.context.deferred_tool_loading {
            true => self.tools.deferred_catalog_text(),
            false => None,
        };
        let mut effective_system_prompt = snapshot.compose_system_prompt(deferred_catalog.as_deref());
        if let Some(role_desc) = active_role {
            effective_system_prompt.push_str(&format!(
                "\n\n<subagent_directive role=\"{}\" read_only=\"{}\">\n{}\n</subagent_directive>",
                role_desc.id, role_desc.read_only, role_desc.system_addendum
            ));
        }
        if let Some(ref schema) = opts.output_schema {
            effective_system_prompt.push_str("\n\n");
            effective_system_prompt.push_str(&format_output_schema_instruction(schema));
        }
        let effective_sys_tokens = estimate_text_tokens(&effective_system_prompt);
        let tool_ctx = ToolContext {
            session_id: session_id.clone(),
            workspace_dir,
            data_dir: cfg.resolved_data_dir(),
            hub_dir: cfg.resolved_hub_dir(),
            shell_timeout_secs: cfg.policy.shell_timeout_secs,
            auto_background_after_secs: cfg.policy.auto_background_after_secs,
            max_output_bytes: cfg.policy.max_output_bytes,
            headless_no_wait: opts.headless_no_wait,
            demote_bg,
        };

        let mut final_assistant_text = String::new();
        let mut ended_with_final_text = false;
        let mut truncation_retries = 0usize;
        let mut schema_retries = 0usize;
        let mut total_usage = UsageStats::default();
        let mut tool_calls_count = 0usize;
        let mut pending_orch_hints: Vec<String> = Vec::new();
        let turn_checkpoint_event_id = parent_for_user.clone();
        let turn_checkpoint_label =
            format!("Before: {}", compact_preview(user_text, CHECKPOINT_LABEL_MAX_CHARS));
        let max_iters = opts
            .max_iterations_override
            .unwrap_or(cfg.general.max_tool_iterations)
            .clamp(1, 64);

        if effective_permission == PermissionMode::Plan {
            self.emit(AgentEvent::OrchestratorUpdated {
                session_id: session_id.clone(),
                stage: ORCH_STAGE_PLAN_MODE.into(),
                tier: orch.triage.tier,
                detail: "Read-only Plan Mode active — mutations blocked until plan is approved.".into(),
                model: Some(model.clone()),
            });
        }

        for iter_idx in 0..max_iters {
            if cancel.is_cancelled() {
                return Err(ZenkoError::Cancelled);
            }

            if iter_idx > 0 {
                for steer_text in self.drain_steering(&session_id) {
                    let steer_ev = self.store.append_event(
                        &session_id,
                        head_id.as_deref(),
                        EventRole::User,
                        EventKind::Message,
                        json!({
                            "text": format!("[Live Steering Update] {steer_text}"),
                            "steering": true,
                        }),
                        &UsageStats::default(),
                    )?;
                    head_id = Some(steer_ev.id);
                }
            }

            let active_deferred = self.activated_deferred_for(&session_id);
            let mut active_tool_specs = self
                .tools
                .specs_for_session(cfg.context.deferred_tool_loading, &active_deferred);
            if let Some(role_desc) = active_role {
                active_tool_specs.retain(|spec| role_desc.permits_tool(&spec.name));
            }
            if effective_permission == PermissionMode::Plan {
                active_tool_specs.retain(is_tool_allowed_in_plan_schema);
            }
            let prefix_tokens =
                effective_sys_tokens + estimate_tool_specs_tokens(&active_tool_specs);

            let mut branch_events = self.store.get_active_branch(&session_id)?;
            let mut ckpt_slice = slice_from_latest_checkpoint(&branch_events);
            let mut raw_messages = events_to_chat_messages_refs(&ckpt_slice.kept_events, &model);
            if let Some(ref ckpt) = ckpt_slice.checkpoint {
                prepend_checkpoint_to_messages(&mut raw_messages, ckpt);
            }

            // Stage A: Zero-cost selective microcompaction (protects recent tail + planning tools)
            let (mut messages, mc_stats) = match cfg.context.enabled {
                true => microcompact_messages(
                    raw_messages,
                    context_window,
                    prefix_tokens,
                    cfg.context.microcompact_trigger_ratio,
                ),
                false => (raw_messages, Default::default()),
            };
            if mc_stats.compacted_tools > 0 {
                self.emit(AgentEvent::OrchestratorUpdated {
                    session_id: session_id.clone(),
                    stage: ORCH_STAGE_MICROCOMPACT.into(),
                    tier: orch.triage.tier,
                    detail: format!(
                        "Microcompacted {} old tool result(s), reclaimed ~{} tok",
                        mc_stats.compacted_tools, mc_stats.tokens_saved
                    ),
                    model: None,
                });
            }

            // Stage B: Persisted iterative-merge CompactionCheckpoint when context still exceeds trigger
            let stage_b_budget =
                ((context_window as f64) * cfg.context.compaction_trigger_ratio.clamp(0.45, 0.94))
                    as usize;
            let mut compacted_this_iter = false;
            if cfg.context.enabled
                && prefix_tokens + estimate_messages_tokens(&messages) > stage_b_budget
            {
                if let Some(plan) = plan_checkpoint_split(
                    &ckpt_slice,
                    context_window,
                    prefix_tokens,
                    cfg.context.compaction_trigger_ratio,
                ) {
                    let ckpt_payload = self
                        .run_stage_b_compaction(
                            cfg,
                            &session_id,
                            &model,
                            &plan,
                            ckpt_slice.checkpoint.as_ref(),
                            orch.guard.active_plan(),
                            prefix_tokens,
                            &cancel,
                        )
                        .await;
                    let ckpt_json = serde_json::to_value(&ckpt_payload).unwrap_or_else(|_| json!({}));
                    let ckpt_ev = self.store.append_event(
                        &session_id,
                        head_id.as_deref(),
                        EventRole::System,
                        EventKind::CompactionCheckpoint,
                        ckpt_json,
                        &UsageStats::default(),
                    )?;
                    head_id = Some(ckpt_ev.id);
                    compacted_this_iter = true;
                    self.emit(AgentEvent::OrchestratorUpdated {
                        session_id: session_id.clone(),
                        stage: ORCH_STAGE_COMPACTION.into(),
                        tier: orch.triage.tier,
                        detail: format!(
                            "Compacted {} events ({} -> {} tok, model={})",
                            ckpt_payload.compacted_events_count,
                            ckpt_payload.estimated_tokens_before,
                            ckpt_payload.estimated_tokens_after,
                            ckpt_payload.model
                        ),
                        model: Some(ckpt_payload.model.clone()),
                    });

                    branch_events = self.store.get_active_branch(&session_id)?;
                    ckpt_slice = slice_from_latest_checkpoint(&branch_events);
                    let mut rebuilt = events_to_chat_messages_refs(&ckpt_slice.kept_events, &model);
                    if let Some(ref ckpt) = ckpt_slice.checkpoint {
                        prepend_checkpoint_to_messages(&mut rebuilt, ckpt);
                    }
                    messages = microcompact_messages(
                        rebuilt,
                        context_window,
                        prefix_tokens,
                        cfg.context.microcompact_trigger_ratio,
                    )
                    .0;
                }
            }

            // Emergency pair-preserving hard ceiling (covers Stage-B cooldown edge cases)
            let before_hard_len = messages.len();
            messages = compact_messages_to_budget(messages, context_window, prefix_tokens);
            let hard_compacted_now = messages.len() < before_hard_len
                && messages
                    .first()
                    .is_some_and(|m| m.content.contains("<compacted_history>"));
            let compacted_now = compacted_this_iter || hard_compacted_now;

            // Collect all volatile turn guidance, hints, plan continuity, and background job notices
            // into a single cache-safe `<system_reminder>` (never injecting mid-array `role: "system"`
            // that would be hoisted by Anthropic/OpenRouter proxies and bust Tier-1 KV cache!).
            let mut tail_reminders: Vec<String> = Vec::with_capacity(4);
            if iter_idx == 0 {
                if effective_permission == PermissionMode::Plan {
                    tail_reminders.push(
                        "<plan_mode active=\"true\">\n\
                         STRICT READ-ONLY PLAN MODE IS ACTIVE.\n\
                         - You may inspect files (`read_file`, `code_search`, `list_dir`), run strictly read-only shell commands (`git status`, `git diff`, `git log`, `rg`, `ls`), spawn read-only subagents (`explorer`, `architect`), and create structured plans via `record_brainstorm`, `plan_update`, and `session_task`.\n\
                         - All file mutations, mutating shell commands, and memory/cron writes are blocked until the user approves the plan.\n\
                         - Produce a concrete, actionable Task DAG (`session_task` or `plan_update`) with dependencies and verification commands, then summarize clearly for approval.\n\
                         </plan_mode>".into(),
                    );
                }
                if let Some(ref rw_sum) = latest_rewind_summary {
                    tail_reminders.push(format!(
                        "<rewind_summary>\nPrevious attempt was rewound. Use these lessons to avoid repeating failed paths:\n{rw_sum}\n</rewind_summary>"
                    ));
                }
                if let Some(ref guide) = orch.guidance_block {
                    tail_reminders.push(guide.clone());
                }
                if let Some(ref recall) = first_turn_recall_block {
                    tail_reminders.push(format!(
                        "<recalled_memories scope=\"{project_scope}\">\nRelevant persistent memories auto-recalled for this workspace:\n{recall}\n</recalled_memories>"
                    ));
                }
                let live_tasks = self
                    .clarifications
                    .list_session_tasks(&session_id)
                    .unwrap_or_default();
                if let Some(dag_block) = format_session_tasks_prompt_block(&live_tasks) {
                    tail_reminders.push(dag_block);
                } else {
                    let plan_already_in_recon = orch.guidance_block.is_some()
                        && orch
                            .recon
                            .as_ref()
                            .and_then(|r| r.prior_plan_summary.as_ref())
                            .is_some();
                    if !plan_already_in_recon {
                        if let Some(plan) = orch.guard.active_plan() {
                            tail_reminders.push(format!(
                                "<active_execution_plan>\nContinue active DAG plan (treat decided steps as locked): {}\n</active_execution_plan>",
                                format_plan_summary(plan)
                            ));
                        }
                    }
                }
            }
            if compacted_now {
                if iter_idx > 0 {
                    let live_tasks = self
                        .clarifications
                        .list_session_tasks(&session_id)
                        .unwrap_or_default();
                    if let Some(dag_block) = format_session_tasks_prompt_block(&live_tasks) {
                        tail_reminders.push(format!(
                            "Context compacted — resume directly from the durable SQLite Task DAG below without re-planning:\n{dag_block}"
                        ));
                    } else if let Some(plan) = orch.guard.active_plan() {
                        tail_reminders.push(format!(
                            "<active_execution_plan>\nContext compacted — treat existing plan as DECIDED work and resume directly without re-planning: {}\n</active_execution_plan>",
                            format_plan_summary(plan)
                        ));
                    }
                }
                // Oh My Pi post-compaction recall (placed in Tier-3 tail so Tier-1 KV cache stays frozen!)
                if let Ok(proj_mems) = self.store.search_memory_hybrid(
                    "",
                    Some(&project_scope),
                    false,
                    Some(&tool_ctx.workspace_dir),
                    3,
                ) {
                    if !proj_mems.is_empty() {
                        let bullets = proj_mems
                            .into_iter()
                            .map(|h| {
                                format!(
                                    "- [{}] {}: {}",
                                    h.entry.kind,
                                    h.entry.title,
                                    compact_preview(&h.entry.content, 120)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        tail_reminders.push(format!(
                            "<project_memory_invariants>\nKey workspace invariants recalled post-compaction:\n{bullets}\n</project_memory_invariants>"
                        ));
                    }
                }
            }
            if !pending_orch_hints.is_empty() {
                tail_reminders.extend(pending_orch_hints.drain(..));
            }
            let mem_deltas = self.drain_memory_deltas(&session_id);
            if !mem_deltas.is_empty() {
                tail_reminders.push(format!(
                    "<memory_delta>\nPersistent memory updated mid-session (Tier-1/Tier-2 prefix stays frozen for KV cache):\n- {}\n</memory_delta>",
                    mem_deltas.join("\n- ")
                ));
            }
            let job_notices = self.drain_job_notices(&session_id);
            if !job_notices.is_empty() {
                let lines: Vec<&str> = job_notices.iter().map(WakeupNotice::as_text).collect();
                tail_reminders.push(format!(
                    "<background_job_updates>\n{}\n</background_job_updates>",
                    lines.join("\n")
                ));
            }
            if let Ok(unread_mail) = self.store.drain_unread_agent_messages(&session_id) {
                if !unread_mail.is_empty() {
                    let lines: Vec<String> = unread_mail
                        .into_iter()
                        .map(|m| format!("- [from {}]: {}", m.from_session_id, m.body))
                        .collect();
                    tail_reminders.push(format!(
                        "<agent_mailbox>\nMessages received from related agents:\n{}\n</agent_mailbox>",
                        lines.join("\n")
                    ));
                }
            }
            if !tail_reminders.is_empty() {
                append_cache_safe_reminder(&mut messages, tail_reminders.join("\n\n"));
            }

            let effective_effort = match (reasoning_effort, self.resolve_supports_reasoning(&model)) {
                (Some(_), Some(false)) => None,
                (eff, _) => eff,
            };

            let chat_req = ChatRequest {
                session_id: session_id.clone(),
                model: model.clone(),
                fallbacks: cfg.provider.fallbacks.clone(),
                system_prompt: effective_system_prompt.clone(),
                messages,
                tools: active_tool_specs,
                temperature: cfg.provider.temperature,
                max_tokens: active_max_tokens,
                enable_streaming: cfg.provider.enable_streaming,
                enable_prompt_caching: cfg.provider.enable_prompt_caching,
                sticky_session_routing: cfg.provider.sticky_session_routing,
                provider_sort: cfg.provider.provider_sort.clone(),
                reasoning_effort: effective_effort,
            };

            let wire_req_meta = match wire_debug_enabled() {
                true => Some(json!({
                    "iteration": iter_idx,
                    "system_prompt_chars": chat_req.system_prompt.len(),
                    "message_count": chat_req.messages.len(),
                    "messages": &chat_req.messages,
                    "tools_count": chat_req.tools.len(),
                    "temperature": chat_req.temperature,
                    "max_tokens": chat_req.max_tokens,
                    "enable_streaming": chat_req.enable_streaming,
                })),
                false => None,
            };

            let turn_id = new_id();
            let iter_started = std::time::Instant::now();
            let mut thinking_started: Option<std::time::Instant> = None;
            let mut thinking_ended: Option<std::time::Instant> = None;
            let mut stream = self.provider.stream_chat(chat_req, cancel.clone()).await?;

            let mut thinking_buf = String::new();
            let mut text_buf = String::new();
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut usage = UsageStats::default();
            let mut finish_reason: Option<String> = None;

            while let Some(item) = stream.next().await {
                match item? {
                    LlmDelta::Thinking(t) => {
                        let now = std::time::Instant::now();
                        thinking_started.get_or_insert(now);
                        thinking_ended = Some(now);
                        thinking_buf.push_str(&t);
                        self.emit(AgentEvent::ThinkingDelta {
                            session_id: session_id.clone(),
                            turn_id: turn_id.clone(),
                            text: t,
                        });
                    }
                    LlmDelta::Text(t) => {
                        text_buf.push_str(&t);
                        self.emit(AgentEvent::TextDelta {
                            session_id: session_id.clone(),
                            turn_id: turn_id.clone(),
                            text: t,
                        });
                    }
                    LlmDelta::ToolCallComplete(tc) => {
                        tool_calls.push(tc);
                    }
                    LlmDelta::Usage(u) => {
                        usage = u;
                    }
                    LlmDelta::Done { finish_reason: fr } => {
                        finish_reason = fr;
                    }
                }
            }

            if let Some(req_meta) = wire_req_meta {
                self.log_sink.record_wire_exchange(
                    &session_id,
                    &turn_id,
                    &cfg.provider.kind,
                    &model,
                    req_meta,
                    json!({
                        "thinking": &thinking_buf,
                        "text": &text_buf,
                        "tool_calls": &tool_calls,
                        "usage": &usage,
                    }),
                );
            }

            total_usage.prompt_tokens += usage.prompt_tokens;
            total_usage.completion_tokens += usage.completion_tokens;
            total_usage.cache_read_tokens += usage.cache_read_tokens;
            total_usage.cost_usd += usage.cost_usd;

            if usage.prompt_tokens > 0 || usage.completion_tokens > 0 || usage.cost_usd > 0.0 {
                let _ = self
                    .store
                    .record_usage(&session_id, &cfg.provider.kind, &model, &usage);
            }

            // Live mid-turn spend-limit guard (checks BOTH global daily cap and per-subagent cost ceiling)
            let live_daily_cost = self.store.daily_cost_usd().unwrap_or(total_usage.cost_usd);
            let daily_exceeded = cfg.general.daily_spend_limit_usd > 0.0
                && total_usage.cost_usd > 0.0
                && live_daily_cost >= cfg.general.daily_spend_limit_usd;
            let turn_cap_exceeded = opts
                .max_cost_usd_override
                .filter(|cap| *cap > 0.0)
                .map_or(false, |cap| total_usage.cost_usd >= cap);
            let budget_exceeded = daily_exceeded || turn_cap_exceeded;

            let iter_duration_ms = iter_started.elapsed().as_millis() as u64;
            let thinking_ms = match (thinking_started, thinking_ended) {
                (Some(start), Some(end)) if !thinking_buf.is_empty() => {
                    end.saturating_duration_since(start).as_millis().max(100) as u64
                }
                _ => 0,
            };

            let assistant_ev = self.store.append_event(
                &session_id,
                head_id.as_deref(),
                EventRole::Assistant,
                match tool_calls.is_empty() {
                    true => EventKind::Message,
                    false => EventKind::ToolCall,
                },
                json!({
                    "text": text_buf,
                    "thinking": thinking_buf,
                    "tool_calls": tool_calls,
                    "model": model,
                    "duration_ms": iter_duration_ms,
                    "thinking_ms": thinking_ms,
                }),
                &usage,
            )?;
            head_id = Some(assistant_ev.id.clone());

            self.emit(AgentEvent::TurnCompleted {
                session_id: session_id.clone(),
                turn_id,
                event_id: assistant_ev.id,
                usage,
            });

            let doom_reason = match cfg.orchestrator.enabled && cfg.orchestrator.loop_circuit_breaker
            {
                true => detect_text_doom_loop(&text_buf)
                    .or_else(|| detect_text_doom_loop(&thinking_buf)),
                false => None,
            };

            let raw_tool_leak = text_buf.contains("<tool_call>")
                || text_buf.contains("<invoke name=");
            let was_truncated = finish_reason.as_deref() == Some("length")
                || (tool_calls.is_empty() && text_buf.trim().is_empty())
                || (tool_calls.is_empty() && raw_tool_leak);

            if !text_buf.is_empty() && !(was_truncated && raw_tool_leak) {
                final_assistant_text = text_buf.clone();
            }

            if budget_exceeded {
                // Keep session history 100% protocol-valid if the LLM emitted tool_calls on the halting iteration
                for tc in &tool_calls {
                    let skip_ev = self.store.append_event(
                        &session_id,
                        head_id.as_deref(),
                        EventRole::Tool,
                        EventKind::ToolResult,
                        json!({
                            "tool_call_id": tc.id,
                            "tool_name": tc.name,
                            "content": "Skipped: daily spend limit reached mid-turn.",
                            "is_error": true,
                            "diff": Value::Null,
                            "duration_ms": 0,
                        }),
                        &UsageStats::default(),
                    )?;
                    head_id = Some(skip_ev.id);
                }
                let msg = match (turn_cap_exceeded, opts.max_cost_usd_override) {
                    (true, Some(cap)) => format!(
                        "Paused turn cleanly: subagent spend limit (${cap:.4}) reached mid-turn (${:.4} spent).",
                        total_usage.cost_usd
                    ),
                    _ => format!(
                        "Paused turn cleanly: daily spend limit (${:.2}) reached mid-turn (${:.4} spent today).",
                        cfg.general.daily_spend_limit_usd, live_daily_cost
                    ),
                };
                self.emit(AgentEvent::OrchestratorUpdated {
                    session_id: session_id.clone(),
                    stage: ORCH_STAGE_BUDGET_GUARD.into(),
                    tier: orch.triage.tier,
                    detail: msg.clone(),
                    model: None,
                });
                let budget_ev = self.store.append_event(
                    &session_id,
                    head_id.as_deref(),
                    EventRole::Assistant,
                    EventKind::Message,
                    json!({
                        "text": msg,
                        "thinking": "",
                        "tool_calls": [],
                        "model": model,
                    }),
                    &UsageStats::default(),
                )?;
                self.emit(AgentEvent::TurnCompleted {
                    session_id: session_id.clone(),
                    turn_id: new_id(),
                    event_id: budget_ev.id,
                    usage: UsageStats::default(),
                });
                final_assistant_text = msg;
                ended_with_final_text = true;
                break;
            }

            if tool_calls.is_empty() {
                if was_truncated
                    && iter_idx + 1 < max_iters
                    && truncation_retries < ORCH_MAX_TRUNCATION_RETRIES
                {
                    truncation_retries += 1;
                    let cap = self
                        .resolve_max_output_tokens(&model)
                        .unwrap_or(ORCH_REASONING_MAX_OUTPUT_CAP)
                        .min(ORCH_REASONING_MAX_OUTPUT_CAP);
                    active_max_tokens = active_max_tokens
                        .saturating_mul(2)
                        .clamp(active_max_tokens, cap.max(active_max_tokens));
                    self.emit(AgentEvent::OrchestratorUpdated {
                        session_id: session_id.clone(),
                        stage: ORCH_STAGE_TRUNCATION_RECOVERY.into(),
                        tier: orch.triage.tier,
                        detail: format!(
                            "Output truncated/empty (finish_reason={:?}, text_chars={}, max_tokens={active_max_tokens}); recovering ({truncation_retries}/{ORCH_MAX_TRUNCATION_RETRIES})",
                            finish_reason,
                            text_buf.len()
                        ),
                        model: None,
                    });
                    pending_orch_hints.push(format!(
                        "<orchestrator_hint type=\"truncation_recovery\">\n\
                         Your previous response was cut off (finish_reason={:?}) or emitted an unparsed/empty payload before finishing. \
                         Keep your internal reasoning brief and keep JSON tool arguments compact (short bullets <= 15 words), then immediately emit your tool call or complete final answer NOW.\n\
                         </orchestrator_hint>",
                        finish_reason.as_deref().unwrap_or("empty")
                    ));
                    continue;
                }
                if let Some(reason) = doom_reason {
                    self.emit(AgentEvent::OrchestratorUpdated {
                        session_id: session_id.clone(),
                        stage: ORCH_STAGE_CIRCUIT_BREAKER.into(),
                        tier: orch.triage.tier,
                        detail: reason.clone(),
                        model: None,
                    });
                    if iter_idx + 1 < max_iters && orch.guard.pivot_count() == 0 {
                        orch.guard.record_text_doom_pivot();
                        let (next_model, next_effort, src) = escalate_on_pivot(
                            cfg,
                            &model,
                            reasoning_effort,
                            orch.has_explicit_model_override,
                        );
                        if next_model != model {
                            context_window = self.resolve_context_window(&next_model);
                        }
                        model = next_model;
                        reasoning_effort = next_effort;
                        pending_orch_hints.push(format!(
                            "<orchestrator_hint type=\"text_doom_loop\">\n\
                             Repetitive output detected ({reason}; escalated={src}). Provide a concise, non-repetitive final answer or concrete next tool call.\n\
                             </orchestrator_hint>"
                        ));
                        continue;
                    }
                    ended_with_final_text = true;
                    break;
                }
                let trailing_steers = self.drain_steering(&session_id);
                if !trailing_steers.is_empty() {
                    for steer_text in trailing_steers {
                        let steer_ev = self.store.append_event(
                            &session_id,
                            head_id.as_deref(),
                            EventRole::User,
                            EventKind::Message,
                            json!({
                                "text": format!("[Live Steering Update] {steer_text}"),
                                "steering": true,
                            }),
                            &UsageStats::default(),
                        )?;
                        head_id = Some(steer_ev.id);
                    }
                    continue;
                }
                if iter_idx + 1 < max_iters {
                    if let Some(ref schema) = opts.output_schema {
                        if schema_retries < SUBAGENT_SCHEMA_MAX_RETRIES {
                            let schema_err = match extract_json_for_schema(&final_assistant_text, schema) {
                                Some(parsed) => match validate_json_schema(schema, &parsed) {
                                    Ok(()) => None,
                                    Err(errs) => Some(errs.join("; ")),
                                },
                                None => Some("Response did not contain a valid JSON value.".into()),
                            };
                            if let Some(reason) = schema_err {
                                schema_retries += 1;
                                pending_orch_hints.push(format!(
                                    "<orchestrator_hint type=\"schema_repair\">\n\
                                     Your final reply violated the required JSON Schema contract ({reason}). \
                                     Respond immediately with strictly valid JSON matching the schema and zero extra prose.\n\
                                     </orchestrator_hint>"
                                ));
                                continue;
                            }
                        }
                    }
                    if active_role.is_none() && !effective_permission.is_read_only_plan() {
                        if let Some(nudge) = orch.guard.check_completion_gate(
                            cfg.orchestrator.enabled && cfg.orchestrator.verification_gate,
                        ) {
                            self.emit(AgentEvent::OrchestratorUpdated {
                                session_id: session_id.clone(),
                                stage: ORCH_STAGE_VERIFICATION_GATE.into(),
                                tier: orch.triage.tier,
                                detail: format!(
                                    "Completion/verification gate active (phase={}, modified={})",
                                    orch.guard.phase().as_str(),
                                    orch.guard.mutated_files_count()
                                ),
                                model: None,
                            });
                            pending_orch_hints.push(nudge);
                            continue;
                        }
                    }
                }
                ended_with_final_text = true;
                break;
            }

            // Pre-tool deterministic Role Policy Guard (subagent allowlist/denylist) +
            // Phase Gate (`Recon -> Plan -> Execute` on Orchestrated turns) & Read-Only Intent Gate.
            let mut allowed_indexed: Vec<(usize, ToolCall)> = Vec::with_capacity(tool_calls.len());
            let mut redirected_indexed: Vec<(usize, ExecutedToolCall)> = Vec::new();
            let mut phase_gate_event: Option<(&'static str, String, String)> = None;
            let mut saw_phase_gate_redirect = false;

            for (idx, tc) in tool_calls.into_iter().enumerate() {
                if let Some(role_desc) = active_role {
                    // Hermes hygiene: ALL subagent roles may search/read memory, but NEVER mutate
                    // durable global/project memory docs or entries (parent decides what is durable).
                    let mutating_mem = tc.name == TOOL_MEMORY
                        && is_memory_mutating_action(
                            tc.arguments.get("action").and_then(Value::as_str).unwrap_or(""),
                        );
                    if !role_desc.permits_tool(&tc.name) || mutating_mem {
                        redirected_indexed.push((
                            idx,
                            ExecutedToolCall {
                                tool_call_id: tc.id,
                                tool_name: tc.name.clone(),
                                arguments: tc.arguments,
                                output: ToolOutput::err(format!(
                                    "Subagent role `{}` policy denied tool `{}`.",
                                    role_desc.id, tc.name
                                )),
                                duration_ms: 0,
                            },
                        ));
                        continue;
                    }
                }
                match active_role.is_none() && cfg.orchestrator.enabled && cfg.orchestrator.phase_gating {
                    true => match orch
                        .guard
                        .check_pre_tool_phase_gate(true, &tc.name, &tc.arguments)
                    {
                        PhaseGateDecision::Allow => allowed_indexed.push((idx, tc)),
                        PhaseGateDecision::Redirect {
                            stage,
                            reason,
                            hint,
                        } => {
                            if stage == ORCH_STAGE_PHASE_GATE {
                                saw_phase_gate_redirect = true;
                            }
                            if phase_gate_event.is_none() {
                                phase_gate_event = Some((stage, reason.clone(), hint));
                            }
                            let tool_err = match stage {
                                ORCH_STAGE_INTENT_GATE => format!(
                                    "Read-only intent gate: {reason}. Answer strictly via read-only inspection without mutating files or state."
                                ),
                                _ => format!(
                                    "Phase gate redirect: {reason}. Inspect files with read-only tools or call `record_brainstorm`/`plan_update` first."
                                ),
                            };
                            redirected_indexed.push((
                                idx,
                                ExecutedToolCall {
                                    tool_call_id: tc.id,
                                    tool_name: tc.name,
                                    arguments: tc.arguments,
                                    output: ToolOutput::err(tool_err),
                                    duration_ms: 0,
                                },
                            ));
                        }
                    },
                    false => allowed_indexed.push((idx, tc)),
                }
            }

            if saw_phase_gate_redirect {
                orch.guard.record_phase_gate_nudge();
            }

            if let Some((stage, reason, hint)) = phase_gate_event {
                self.emit(AgentEvent::OrchestratorUpdated {
                    session_id: session_id.clone(),
                    stage: stage.into(),
                    tier: orch.triage.tier,
                    detail: reason,
                    model: None,
                });
                pending_orch_hints.push(hint);
            }

            let total_wave_calls = allowed_indexed.len() + redirected_indexed.len();
            tool_calls_count += allowed_indexed.len();

            // Phase 6 Lazy Pre-Mutation Workspace Checkpoint:
            // Captures at most once per turn right before the first workspace-mutating tool call
            // (file edits, non-read-only shell, worktree merge, or inherited mutating subagent).
            if cfg.checkpoints.enabled
                && turn_checkpoint_id.is_none()
                && effective_permission != PermissionMode::Plan
            {
                let mutating_trigger = allowed_indexed
                    .iter()
                    .find_map(|(_, tc)| is_workspace_mutating_call(tc));
                if let Some(trigger_tool) = mutating_trigger {
                    match self
                        .checkpoints
                        .capture_checkpoint(
                            &cfg.checkpoints,
                            &session_id,
                            &format!("turn_{}", self.turn_seq.load(Ordering::Relaxed)),
                            turn_checkpoint_event_id.clone(),
                            &tool_ctx.workspace_dir,
                            trigger_tool,
                            turn_checkpoint_label.clone(),
                            false,
                        )
                        .await
                    {
                        Ok(cp) => {
                            self.emit(AgentEvent::OrchestratorUpdated {
                                session_id: session_id.clone(),
                                stage: ORCH_STAGE_CHECKPOINT.into(),
                                tier: orch.triage.tier,
                                detail: format!(
                                    "Saved pre-mutation checkpoint {} ({}) before `{trigger_tool}`",
                                    &cp.id[..cp.id.len().min(8)],
                                    cp.backend.as_str()
                                ),
                                model: None,
                            });
                            *turn_checkpoint_id = Some(cp.id);
                        }
                        Err(e) => {
                            warn!(error = %e, "pre-mutation workspace checkpoint skipped");
                        }
                    }
                }
            }

            let executed_allowed: Vec<ExecutedToolCall> = match allowed_indexed.is_empty() {
                true => Vec::new(),
                false => {
                    let calls_to_run: Vec<ToolCall> =
                        allowed_indexed.iter().map(|(_, tc)| tc.clone()).collect();
                    match execute_tool_waves(
                        calls_to_run,
                        MAX_PARALLEL_TOOLS,
                        |tc| self.classify_tool_call(cfg, effective_permission, tc),
                        |tc| {
                            let sid = session_id.clone();
                            let ctx_ref = tool_ctx.clone();
                            let cancel_ref = cancel.clone();
                            async move {
                                self.execute_single_tool_call(
                                    cfg,
                                    &sid,
                                    effective_permission,
                                    &ctx_ref,
                                    opts.headless_no_wait,
                                    tc,
                                    &cancel_ref,
                                )
                                .await
                            }
                        },
                    )
                    .await
                    {
                        Ok(results) => results,
                        Err(err) => {
                            // Keep SQLite protocol-valid: every emitted `Assistant(tool_calls)` MUST
                            // have matching `Tool(ToolResult)` events so future turns never 400!
                            let abort_msg = match err {
                                ZenkoError::Cancelled => {
                                    "Skipped: tool execution cancelled by user.".to_string()
                                }
                                ref other => format!("Skipped: tool wave aborted ({other})."),
                            };
                            for (_, tc) in &allowed_indexed {
                                if let Ok(skip_ev) = self.store.append_event(
                                    &session_id,
                                    head_id.as_deref(),
                                    EventRole::Tool,
                                    EventKind::ToolResult,
                                    json!({
                                        "tool_call_id": tc.id,
                                        "tool_name": tc.name,
                                        "content": abort_msg,
                                        "is_error": true,
                                        "diff": Value::Null,
                                        "duration_ms": 0,
                                    }),
                                    &UsageStats::default(),
                                ) {
                                    head_id = Some(skip_ev.id);
                                }
                            }
                            for (_, redir) in &redirected_indexed {
                                if let Ok(skip_ev) = self.store.append_event(
                                    &session_id,
                                    head_id.as_deref(),
                                    EventRole::Tool,
                                    EventKind::ToolResult,
                                    json!({
                                        "tool_call_id": redir.tool_call_id,
                                        "tool_name": redir.tool_name,
                                        "content": redir.output.content,
                                        "is_error": true,
                                        "diff": Value::Null,
                                        "duration_ms": 0,
                                    }),
                                    &UsageStats::default(),
                                ) {
                                    head_id = Some(skip_ev.id);
                                }
                            }
                            return Err(err);
                        }
                    }
                }
            };

            let mut ordered_results: Vec<Option<(ExecutedToolCall, bool)>> =
                (0..total_wave_calls).map(|_| None).collect();
            for ((idx, _), exec) in allowed_indexed.into_iter().zip(executed_allowed.into_iter()) {
                ordered_results[idx] = Some((exec, false));
            }
            for (idx, redir) in redirected_indexed {
                ordered_results[idx] = Some((redir, true));
            }

            let mut escalate_trigger: Option<(String, String)> = None;
            let mut wave_had_plan_or_mutation = false;

            for (done, was_phase_redirect) in ordered_results.into_iter().flatten() {
                if !was_phase_redirect && !done.output.is_error {
                    let cat = orchestrator::classify_tool_category(&done.tool_name, &done.arguments);
                    if matches!(
                        cat,
                        orchestrator::ToolCategory::Planning | orchestrator::ToolCategory::Mutation
                    ) {
                        wave_had_plan_or_mutation = true;
                    }
                }
                if !was_phase_redirect && done.tool_name == TOOL_MEMORY && !done.output.is_error {
                    let act = done
                        .arguments
                        .get("action")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if is_memory_mutating_action(act) {
                        self.record_memory_delta(
                            &session_id,
                            compact_preview(&done.output.content, 160),
                        );
                    }
                }
                if !was_phase_redirect && done.tool_name == TOOL_PLAN_UPDATE && !done.output.is_error
                {
                    if let Some(plan) = done
                        .output
                        .metadata
                        .as_ref()
                        .and_then(|v| serde_json::from_value::<ExecutionPlan>(v.clone()).ok())
                    {
                        orch.guard.update_active_plan(plan);
                    }
                }
                if !was_phase_redirect
                    && done.tool_name == TOOL_SESSION_TASK
                    && !done.output.is_error
                {
                    if let Ok(live_tasks) = self.clarifications.list_session_tasks(&session_id) {
                        if let Some(plan) =
                            execution_plan_from_session_tasks(&session.title, &live_tasks)
                        {
                            orch.guard.update_active_plan(plan);
                        }
                    }
                }

                if !was_phase_redirect
                    && cfg.orchestrator.enabled
                    && cfg.orchestrator.loop_circuit_breaker
                {
                    match orch.guard.record_tool_outcome(
                        &done.tool_name,
                        &done.arguments,
                        done.output.is_error,
                        &done.output.content,
                        done.output.metadata.as_ref(),
                    ) {
                        GuardVerdict::Continue => {}
                        GuardVerdict::Pivot { reason, hint } => {
                            let (next_model, next_effort, src) = escalate_on_pivot(
                                cfg,
                                &model,
                                reasoning_effort,
                                orch.has_explicit_model_override,
                            );
                            let switched_model = match next_model != model {
                                true => {
                                    context_window = self.resolve_context_window(&next_model);
                                    Some(next_model.clone())
                                }
                                false => None,
                            };
                            model = next_model;
                            reasoning_effort = next_effort;
                            self.emit(AgentEvent::OrchestratorUpdated {
                                session_id: session_id.clone(),
                                stage: ORCH_STAGE_CIRCUIT_BREAKER.into(),
                                tier: orch.triage.tier,
                                detail: format!("{reason} ({src})"),
                                model: switched_model,
                            });
                            pending_orch_hints.push(hint);
                        }
                        GuardVerdict::Escalate { reason, summary } => {
                            self.emit(AgentEvent::OrchestratorUpdated {
                                session_id: session_id.clone(),
                                stage: ORCH_STAGE_ESCALATED.into(),
                                tier: orch.triage.tier,
                                detail: summary.clone(),
                                model: None,
                            });
                            escalate_trigger = Some((reason, summary));
                        }
                    }
                }

                let ev = self.store.append_event(
                    &session_id,
                    head_id.as_deref(),
                    EventRole::Tool,
                    EventKind::ToolResult,
                    json!({
                        "tool_call_id": done.tool_call_id,
                        "tool_name": done.tool_name,
                        "content": done.output.content,
                        "is_error": done.output.is_error,
                        "diff": done.output.diff,
                        "duration_ms": done.duration_ms,
                    }),
                    &UsageStats::default(),
                )?;
                head_id = Some(ev.id);

                // Persist structured orchestrator artifacts strictly onto the active branch
                let note_payload = match (
                    was_phase_redirect,
                    done.tool_name.as_str(),
                    done.output.is_error,
                ) {
                    (false, TOOL_RECORD_BRAINSTORM, false) => {
                        done.output.metadata.as_ref().map(|spec| {
                            json!({
                                "note_kind": NOTE_KIND_BRAINSTORM_SPEC,
                                "spec": spec,
                            })
                        })
                    }
                    (false, TOOL_PLAN_UPDATE, false) => done.output.metadata.as_ref().map(|plan| {
                        json!({
                            "note_kind": NOTE_KIND_EXECUTION_PLAN,
                            "plan": plan,
                        })
                    }),
                    (false, TOOL_ASK_CLARIFICATION, false) => {
                        done.output.metadata.as_ref().map(|dec| {
                            json!({
                                "note_kind": NOTE_KIND_CLARIFICATION_DECISION,
                                "decision": dec,
                            })
                        })
                    }
                    (false, TOOL_TOOL_SEARCH, false) => done
                        .output
                        .metadata
                        .as_ref()
                        .and_then(|m| m.get("activated_tools"))
                        .and_then(Value::as_array)
                        .filter(|arr| !arr.is_empty())
                        .map(|arr| {
                            json!({
                                "note_kind": NOTE_KIND_DEFERRED_TOOLS,
                                "tools": arr,
                            })
                        }),
                    (false, name, false) if self.tools.is_deferred(name) => Some(json!({
                        "note_kind": NOTE_KIND_DEFERRED_TOOLS,
                        "tools": [name],
                    })),
                    _ => None,
                };
                if let Some(payload) = note_payload {
                    let note_ev = self.store.append_event(
                        &session_id,
                        head_id.as_deref(),
                        EventRole::System,
                        EventKind::SystemNote,
                        payload,
                        &UsageStats::default(),
                    )?;
                    head_id = Some(note_ev.id);
                }
            }

            if let Some((reason, summary)) = escalate_trigger {
                let halt_message: Option<String> =
                    match opts.headless_no_wait || iter_idx + 1 >= max_iters {
                        true => Some(format!(
                            "Stopped cleanly after circuit-breaker escalation: {summary}"
                        )),
                    false => {
                        let ans = self
                            .clarifications
                            .request_clarification(
                                &session_id,
                                &format!("Execution loop detected: {reason}. How should Zenko proceed?"),
                                &summary,
                                vec![
                                    ClarificationOption {
                                        id: ESCALATION_OPT_PIVOT.into(),
                                        label: "Pivot strategy & retry".into(),
                                        description: "Try a fundamentally different approach or inspect ground truth.".into(),
                                        recommended: true,
                                    },
                                    ClarificationOption {
                                        id: ESCALATION_OPT_STOP.into(),
                                        label: "Stop here & report".into(),
                                        description: "Halt tool execution and summarize the blocker.".into(),
                                        recommended: false,
                                    },
                                ],
                                false,
                                cancel.clone(),
                            )
                            .await?;
                        match ans.selected_option_id.as_str() {
                            ESCALATION_OPT_STOP => Some(format!(
                                "Halted by user after loop escalation: {summary}"
                            )),
                            _ => {
                                orch.guard.reset_after_escalation();
                                let note = ans.custom_answer.unwrap_or_default();
                                pending_orch_hints.push(format!(
                                    "<orchestrator_hint type=\"escalation_guidance\">\n\
                                     User chose `{}` after loop escalation ({reason}). {note} Change strategy fundamentally.\n\
                                     </orchestrator_hint>",
                                    ans.selected_option_id
                                ));
                                None
                            }
                        }
                    }
                };
                if let Some(msg) = halt_message {
                    let halt_ev = self.store.append_event(
                        &session_id,
                        head_id.as_deref(),
                        EventRole::Assistant,
                        EventKind::Message,
                        json!({
                            "text": msg,
                            "thinking": "",
                            "tool_calls": [],
                            "model": model,
                        }),
                        &UsageStats::default(),
                    )?;
                    self.emit(AgentEvent::TurnCompleted {
                        session_id: session_id.clone(),
                        turn_id: new_id(),
                        event_id: halt_ev.id,
                        usage: UsageStats::default(),
                    });
                    final_assistant_text = msg;
                    ended_with_final_text = true;
                    break;
                }
            }

            if let Some((detail, hint)) = orch.guard.check_iteration_pacing(
                cfg.orchestrator.enabled,
                iter_idx,
                max_iters,
                wave_had_plan_or_mutation,
            ) {
                self.emit(AgentEvent::OrchestratorUpdated {
                    session_id: session_id.clone(),
                    stage: ORCH_STAGE_PACING.into(),
                    tier: orch.triage.tier,
                    detail,
                    model: None,
                });
                pending_orch_hints.push(hint);
            }
        }

        // Tool-free final wrap-up synthesis pass if max_iters ended on a tool-calling turn,
        // preventing stale intermediate filler prose from being returned as the final answer.
        if !ended_with_final_text && !cancel.is_cancelled() {
            self.emit(AgentEvent::OrchestratorUpdated {
                session_id: session_id.clone(),
                stage: ORCH_STAGE_SYNTHESIS.into(),
                tier: orch.triage.tier,
                detail: format!("Max tool iterations ({max_iters}) reached; running tool-free wrap-up synthesis"),
                model: Some(model.clone()),
            });
            let branch_events = self.store.get_active_branch(&session_id)?;
            let ckpt_slice = slice_from_latest_checkpoint(&branch_events);
            let mut raw_messages = events_to_chat_messages_refs(&ckpt_slice.kept_events, &model);
            if let Some(ref ckpt) = ckpt_slice.checkpoint {
                prepend_checkpoint_to_messages(&mut raw_messages, ckpt);
            }
            let (mc_messages, _) = microcompact_messages(
                raw_messages,
                context_window,
                effective_sys_tokens,
                cfg.context.microcompact_trigger_ratio,
            );
            let mut messages = compact_messages_to_budget(
                mc_messages,
                context_window,
                effective_sys_tokens,
            );
            append_cache_safe_reminder(
                &mut messages,
                format!(
                    "<orchestrator_hint type=\"final_synthesis\">\n\
                     Tool iteration budget ({max_iters}) reached after {tool_calls_count} tool call(s). Tools are now disabled for this wrap-up turn. \
                     Synthesize a complete, clear final answer directly from all evidence and tool results gathered above.\n\
                     </orchestrator_hint>"
                ),
            );
            let effective_effort = match (reasoning_effort, self.resolve_supports_reasoning(&model)) {
                (Some(_), Some(false)) => None,
                (eff, _) => eff,
            };
            let synth_req = ChatRequest {
                session_id: session_id.clone(),
                model: model.clone(),
                fallbacks: cfg.provider.fallbacks.clone(),
                system_prompt: effective_system_prompt.clone(),
                messages,
                tools: Vec::new(),
                temperature: cfg.provider.temperature,
                max_tokens: active_max_tokens,
                enable_streaming: cfg.provider.enable_streaming,
                enable_prompt_caching: cfg.provider.enable_prompt_caching,
                sticky_session_routing: cfg.provider.sticky_session_routing,
                provider_sort: cfg.provider.provider_sort.clone(),
                reasoning_effort: effective_effort,
            };
            if let Ok(mut stream) = self.provider.stream_chat(synth_req, cancel.clone()).await {
                let synth_turn_id = new_id();
                let synth_started = std::time::Instant::now();
                let mut synth_think_start: Option<std::time::Instant> = None;
                let mut synth_think_end: Option<std::time::Instant> = None;
                let mut synth_thinking = String::new();
                let mut synth_text = String::new();
                let mut synth_usage = UsageStats::default();
                let mut synth_stream_err: Option<String> = None;
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(delta) => match delta {
                            LlmDelta::Thinking(t) => {
                                let now = std::time::Instant::now();
                                synth_think_start.get_or_insert(now);
                                synth_think_end = Some(now);
                                synth_thinking.push_str(&t);
                                self.emit(AgentEvent::ThinkingDelta {
                                    session_id: session_id.clone(),
                                    turn_id: synth_turn_id.clone(),
                                    text: t,
                                });
                            }
                            LlmDelta::Text(t) => {
                                synth_text.push_str(&t);
                                self.emit(AgentEvent::TextDelta {
                                    session_id: session_id.clone(),
                                    turn_id: synth_turn_id.clone(),
                                    text: t,
                                });
                            }
                            LlmDelta::Usage(u) => synth_usage = u,
                            LlmDelta::ToolCallComplete(_) | LlmDelta::Done { .. } => {}
                        },
                        Err(err) => {
                            synth_stream_err = Some(err.to_string());
                            break;
                        }
                    }
                }
                total_usage.prompt_tokens += synth_usage.prompt_tokens;
                total_usage.completion_tokens += synth_usage.completion_tokens;
                total_usage.cache_read_tokens += synth_usage.cache_read_tokens;
                total_usage.cost_usd += synth_usage.cost_usd;
                if synth_usage.prompt_tokens > 0
                    || synth_usage.completion_tokens > 0
                    || synth_usage.cost_usd > 0.0
                {
                    let _ = self.store.record_usage(
                        &session_id,
                        &cfg.provider.kind,
                        &model,
                        &synth_usage,
                    );
                }
                if let Some(err_msg) = synth_stream_err {
                    warn!(
                        session_id = %session_id,
                        error = %err_msg,
                        "wrap-up synthesis stream failed; skipping partial assistant persistence"
                    );
                } else if !cancel.is_cancelled() && !synth_text.trim().is_empty() {
                    let synth_duration_ms = synth_started.elapsed().as_millis() as u64;
                    let synth_thinking_ms = match (synth_think_start, synth_think_end) {
                        (Some(start), Some(end)) if !synth_thinking.is_empty() => {
                            end.saturating_duration_since(start).as_millis().max(100) as u64
                        }
                        _ => 0,
                    };
                    let synth_ev = self.store.append_event(
                        &session_id,
                        head_id.as_deref(),
                        EventRole::Assistant,
                        EventKind::Message,
                        json!({
                            "text": synth_text,
                            "thinking": synth_thinking,
                            "tool_calls": [],
                            "model": model,
                            "duration_ms": synth_duration_ms,
                            "thinking_ms": synth_thinking_ms,
                        }),
                        &synth_usage,
                    )?;
                    self.emit(AgentEvent::TurnCompleted {
                        session_id: session_id.clone(),
                        turn_id: synth_turn_id,
                        event_id: synth_ev.id,
                        usage: synth_usage,
                    });
                    final_assistant_text = synth_text;
                }
            }
            if final_assistant_text.trim().is_empty() {
                final_assistant_text = format!(
                    "Reached max tool iterations ({max_iters}) after {tool_calls_count} tool calls without a final summary."
                );
            }
        }

        let _ = run_hooks(
            &cfg.hooks,
            HookEvent::Stop,
            None,
            json!({ "session_id": session_id }),
        )
        .await;

        if cfg.memory.dream_enabled && session.parent_session_id.is_none() {
            self.dream_notify.notify_one();
        }

        Ok(TurnResult {
            session_id,
            reply: final_assistant_text,
            model,
            usage: total_usage,
            tool_calls_count,
        })
    }

    fn classify_tool_call(
        &self,
        cfg: &ZenkoConfig,
        effective_permission: PermissionMode,
        tc: &ToolCall,
    ) -> ToolConcurrency {
        let Some(tool) = self.tools.get(&tc.name) else {
            return ToolConcurrency::Global;
        };
        let base = tool.concurrency_for_args(&tc.arguments);
        if base == ToolConcurrency::Global {
            return ToolConcurrency::Global;
        }
        let allowed_set = self
            .always_allowed
            .read()
            .map(|g| g.clone())
            .unwrap_or_default();
        match evaluate_policy(
            effective_permission,
            &cfg.policy,
            &tool.spec(),
            &tc.arguments,
            &allowed_set,
        ) {
            PolicyVerdict::Allow => base,
            PolicyVerdict::AskUser | PolicyVerdict::Deny(_) => ToolConcurrency::Global,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_single_tool_call(
        &self,
        cfg: &ZenkoConfig,
        session_id: &str,
        effective_permission: PermissionMode,
        tool_ctx: &ToolContext,
        headless_no_wait: bool,
        tc: ToolCall,
        cancel: &CancellationToken,
    ) -> Result<ExecutedToolCall> {
        if cancel.is_cancelled() {
            return Err(ZenkoError::Cancelled);
        }

        let start_ts = Instant::now();
        let args_preview = truncate_preview(&tc.arguments.to_string(), 320);

        let Some(tool) = self.tools.get(&tc.name) else {
            let err_out = ToolOutput::err(format!("Unknown tool: {}", tc.name));
            return Ok(ExecutedToolCall {
                tool_call_id: tc.id,
                tool_name: tc.name,
                arguments: tc.arguments,
                output: err_out,
                duration_ms: 0,
            });
        };

        let spec = tool.spec();
        if self.tools.is_deferred(&tc.name) {
            let newly = self.activate_deferred_tools(session_id, &[tc.name.clone()]);
            if !newly.is_empty() {
                self.emit(AgentEvent::OrchestratorUpdated {
                    session_id: session_id.to_string(),
                    stage: ORCH_STAGE_TOOL_ACTIVATED.into(),
                    tier: zenko_core::ExecutionTier::Standard,
                    detail: format!("Auto-activated deferred tool `{}` on direct call", tc.name),
                    model: None,
                });
            }
        }
        let _active_tool_guard =
            ActiveToolGuard::enter(self.session_control(session_id), tc.name.clone());
        self.emit(AgentEvent::ToolCallStarted {
            session_id: session_id.to_string(),
            call_id: tc.id.clone(),
            tool_name: tc.name.clone(),
            args_preview: args_preview.clone(),
            risk: spec.risk,
        });

        let hook_res = run_hooks(
            &cfg.hooks,
            HookEvent::PreToolUse,
            Some(&tc.name),
            json!({ "session_id": session_id, "args": tc.arguments }),
        )
        .await;

        let mut tool_output = match hook_res {
            Err(blocked) => ToolOutput::err(format!("Blocked by PreToolUse hook: {blocked}")),
            Ok(()) => {
                let allowed_set = self
                    .always_allowed
                    .read()
                    .map(|g| g.clone())
                    .unwrap_or_default();
                let verdict = evaluate_policy(
                    effective_permission,
                    &cfg.policy,
                    &spec,
                    &tc.arguments,
                    &allowed_set,
                );

                match verdict {
                    PolicyVerdict::Deny(reason) => {
                        ToolOutput::err(format!("Denied by execpolicy: {reason}"))
                    }
                    PolicyVerdict::AskUser if headless_no_wait => ToolOutput::err(format!(
                        "Tool `{}` requires approval in `{}` mode (headless run did not block). Re-run with `--mode autonomous` or approve via daemon/WebUI.",
                        tc.name,
                        effective_permission.as_str()
                    )),
                    PolicyVerdict::AskUser => {
                        let _ = run_hooks(
                            &cfg.hooks,
                            HookEvent::PermissionRequest,
                            Some(&tc.name),
                            json!({ "session_id": session_id, "args": tc.arguments }),
                        )
                        .await;
                        match self
                            .request_hitl_approval(
                                session_id,
                                &tc.id,
                                &tc.name,
                                &args_preview,
                                spec.risk,
                                cancel,
                            )
                            .await?
                        {
                            ApprovalDecision::Approve | ApprovalDecision::AlwaysAllow => {
                                match tool
                                    .execute(tool_ctx, tc.arguments.clone(), cancel.clone())
                                    .await
                                {
                                    Ok(out) => out,
                                    Err(ZenkoError::Cancelled) => {
                                        return Err(ZenkoError::Cancelled);
                                    }
                                    Err(e) => ToolOutput::err(format!("Tool error: {e}")),
                                }
                            }
                            ApprovalDecision::Deny => {
                                ToolOutput::err(
                                    "Tool execution skipped (user denied or steered the turn with updated instructions).",
                                )
                            }
                        }
                    }
                    PolicyVerdict::Allow => {
                        match tool
                            .execute(tool_ctx, tc.arguments.clone(), cancel.clone())
                            .await
                        {
                            Ok(out) => out,
                            Err(ZenkoError::Cancelled) => return Err(ZenkoError::Cancelled),
                            Err(e) => ToolOutput::err(format!("Tool error: {e}")),
                        }
                    }
                }
            }
        };

        if !tool_output.is_error
            && (MUTATING_FILE_TOOLS.contains(&tc.name.as_str())
                || (tc.name == TOOL_GIT && is_git_tool_args_mutating(&tc.arguments))
                || tc.name == TOOL_SHELL_EXEC)
        {
            self.record_workspace_mutation();
        }

        // Oh My Pi–style Diagnostics-on-Edit: if a warm LSP server is already running for an edited
        // file, attach fresh compiler/type errors directly to the edit output (zero cold spawns!).
        if !tool_output.is_error
            && cfg.lsp.enabled
            && cfg.lsp.auto_diagnostics_on_edit
            && MUTATING_FILE_TOOLS.contains(&tc.name.as_str())
        {
            for edited_rel in extract_edited_rel_paths(&tc.name, &tc.arguments) {
                let candidate = tool_ctx.workspace_dir.join(&edited_rel);
                if let Some(diag_block) = self
                    .lsp
                    .warm_diagnostics_for_edited_file(
                        &tool_ctx.workspace_dir,
                        &candidate,
                        cancel.clone(),
                    )
                    .await
                {
                    tool_output.content.push_str(&diag_block);
                }
            }
        }

        // Async Prime-RLM / Codex Head+Tail Artifact Spilling for oversized tool outputs
        if cfg.context.artifact_spilling && tool_output.content.len() > tool_ctx.max_output_bytes {
            let (spilled_content, spill_rec) = artifacts::spill_tool_output_if_oversized(
                &tool_ctx.data_dir,
                session_id,
                &tc.id,
                &tc.name,
                tool_output.content,
                tool_ctx.max_output_bytes,
            )
            .await;
            tool_output.content = spilled_content;
            if let Some(rec) = spill_rec {
                let art_val = serde_json::to_value(&rec).unwrap_or(Value::Null);
                tool_output.metadata = match tool_output.metadata {
                    Some(Value::Object(mut map)) => {
                        map.insert("artifact".into(), art_val);
                        Some(Value::Object(map))
                    }
                    Some(other) => Some(json!({ "data": other, "artifact": art_val })),
                    None => Some(json!({ "artifact": art_val })),
                };
            }
        }

        let duration_ms = start_ts.elapsed().as_millis() as u64;

        let _ = run_hooks(
            &cfg.hooks,
            HookEvent::PostToolUse,
            Some(&tc.name),
            json!({
                "session_id": session_id,
                "is_error": tool_output.is_error,
                "duration_ms": duration_ms
            }),
        )
        .await;

        self.emit(AgentEvent::ToolCallFinished {
            session_id: session_id.to_string(),
            call_id: tc.id.clone(),
            tool_name: tc.name.clone(),
            is_error: tool_output.is_error,
            duration_ms,
            diff: tool_output.diff.clone(),
            output_preview: truncate_preview(&tool_output.content, 600),
        });

        // Broadcast `SkillUpdated` when an agent mutates a procedural skill (notifies WebUI SSE & Telegram draft gate)
        if tc.name == TOOL_SKILL_MANAGE && !tool_output.is_error {
            if let Some(action_raw) = tc.arguments.get("action").and_then(Value::as_str) {
                if let Some(action) = SkillManageAction::parse(action_raw) {
                    if action.is_mutating() {
                        let meta_skill = tool_output.metadata.as_ref().and_then(|m| m.get("skill"));
                        let slug = meta_skill
                            .and_then(|s| s.get("slug"))
                            .and_then(Value::as_str)
                            .or_else(|| tc.arguments.get("slug").and_then(Value::as_str))
                            .unwrap_or("")
                            .to_string();
                        let status = meta_skill
                            .and_then(|s| s.get("status"))
                            .and_then(Value::as_str)
                            .unwrap_or("active")
                            .to_string();
                        if !slug.is_empty() {
                            self.emit(AgentEvent::SkillUpdated {
                                slug,
                                action: action_raw.to_string(),
                                status,
                            });
                        }
                    }
                }
            }
        }

        Ok(ExecutedToolCall {
            tool_call_id: tc.id,
            tool_name: tc.name,
            arguments: tc.arguments,
            output: tool_output,
            duration_ms,
        })
    }

    async fn request_hitl_approval(
        &self,
        session_id: &str,
        call_id: &str,
        tool_name: &str,
        args_preview: &str,
        risk: zenko_core::RiskLevel,
        cancel: &CancellationToken,
    ) -> Result<ApprovalDecision> {
        let approval_id = new_id();
        let record = ApprovalRecord {
            id: approval_id.clone(),
            session_id: session_id.to_string(),
            call_id: call_id.to_string(),
            tool_name: tool_name.to_string(),
            args_preview: args_preview.to_string(),
            risk,
            status: ApprovalStatus::Pending,
            resolved_by: None,
            created_at: Utc::now(),
            resolved_at: None,
        };

        self.store.insert_approval(&record)?;
        self.store
            .update_session_status(session_id, SessionStatus::WaitingApproval)?;
        self.emit(AgentEvent::SessionUpdated {
            session_id: session_id.to_string(),
            status: SessionStatus::WaitingApproval,
            title: None,
        });

        let (tx, rx) = oneshot::channel();
        if let Ok(mut map) = self.pending_approvals.lock() {
            map.insert(approval_id.clone(), (session_id.to_string(), tx));
        }

        self.emit(AgentEvent::ToolApprovalRequested { approval: record });

        let decision = tokio::select! {
            _ = cancel.cancelled() => {
                if let Ok(mut map) = self.pending_approvals.lock() {
                    map.remove(&approval_id);
                }
                let _ = self.store.resolve_approval(&approval_id, ApprovalDecision::Deny, APPROVAL_RESOLVER_CANCELLED);
                return Err(ZenkoError::Cancelled);
            }
            res = tokio::time::timeout(Duration::from_secs(DEFAULT_APPROVAL_TIMEOUT_SECS), rx) => {
                match res {
                    Ok(Ok(d)) => d,
                    _ => {
                        if let Ok(mut map) = self.pending_approvals.lock() {
                            map.remove(&approval_id);
                        }
                        let _ = self.store.resolve_approval(&approval_id, ApprovalDecision::Deny, APPROVAL_RESOLVER_TIMEOUT);
                        ApprovalDecision::Deny
                    }
                }
            }
        };

        self.store
            .update_session_status(session_id, SessionStatus::Running)?;
        self.emit(AgentEvent::SessionUpdated {
            session_id: session_id.to_string(),
            status: SessionStatus::Running,
            title: None,
        });

        Ok(decision)
    }

    // ─── Reactive Batched Idle-Session Wakeup Loop (Zero Busy-Polling) ─────────

    /// Event-driven daemon task that wakes idle sessions when their background jobs complete,
    /// coalescing burst completions within `JOB_WAKE_COALESCE_MS` into a single follow-up turn.
    pub async fn run_job_wakeup_loop(self: Arc<Self>, cancel: CancellationToken) {
        let Some(mut rx) = self.wake_rx.lock().ok().and_then(|mut g| g.take()) else {
            return;
        };
        info!("reactive background-job wakeup loop started (zero-polling)");

        loop {
            let first_sid = tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                msg = rx.recv() => match msg {
                    Some(sid) => sid,
                    None => break,
                }
            };

            let mut batch: HashSet<String> = HashSet::with_capacity(4);
            batch.insert(first_sid);
            let deadline =
                tokio::time::Instant::now() + Duration::from_millis(JOB_WAKE_COALESCE_MS);
            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep_until(deadline) => break,
                    more = rx.recv() => match more {
                        Some(sid) => { batch.insert(sid); }
                        None => break,
                    }
                }
            }

            for sid in batch {
                if self.is_session_running(&sid) || !self.has_job_notices(&sid) {
                    continue;
                }
                if self.store.get_session(&sid).ok().flatten().is_none() {
                    let _ = self.drain_job_notices(&sid);
                    continue;
                }
                let rt = self.clone();
                tokio::spawn(async move {
                    if rt.is_session_running(&sid) {
                        return;
                    }
                    let notices = rt.drain_job_notices(&sid);
                    if notices.is_empty() {
                        return;
                    }
                    let mut shell_lines: Vec<&str> = Vec::new();
                    let mut agent_lines: Vec<&str> = Vec::new();
                    for n in &notices {
                        match n {
                            WakeupNotice::ShellJob(s) => shell_lines.push(s.as_str()),
                            WakeupNotice::Subagent { summary, .. } => agent_lines.push(summary.as_str()),
                        }
                    }
                    let wakeup_prompt = match (shell_lines.is_empty(), agent_lines.is_empty()) {
                        (false, true) => format!(
                            "[Background Job Update]\n{}\n\nReview the completed background job(s), call `job_read` if you need detailed output, and report concise findings or continue the task.",
                            shell_lines.join("\n")
                        ),
                        (true, false) => format!(
                            "[Subagent Completion Update]\n{}\n\nReview the completed subagent result(s), use `agent_worktree` (`diff`/`merge`/`discard`) for worktree edits or `agent_message` to resume if follow-up is needed, and synthesize concise findings.",
                            agent_lines.join("\n")
                        ),
                        _ => format!(
                            "[Async Completion Update]\nBackground Jobs:\n{}\n\nSubagents:\n{}\n\nReview all completed items (use `job_read` for shell jobs and `agent_worktree`/`agent_list` for subagents) and report concise findings or continue.",
                            shell_lines.join("\n"),
                            agent_lines.join("\n")
                        ),
                    };
                    let headless = !rt.is_telegram_connected();
                    if let Err(e) = rt
                        .run_turn_with_options(
                            &sid,
                            &wakeup_prompt,
                            TurnOptions {
                                headless_no_wait: headless,
                                send_mode: SendMode::Queue,
                                ..TurnOptions::default()
                            },
                        )
                        .await
                    {
                        warn!(session_id = %sid, error = %e, "reactive job wakeup turn failed");
                    }
                });
            }
        }
    }

    /// Lists categorized creations inside `~/zenko_hub/` (projects, scripts, media, artifacts).
    pub fn list_hub_entries(
        &self,
        category_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<HubEntry>> {
        self.config().list_hub_entries(category_filter, limit)
    }

    /// Executes a cron job immediately (shared by both `run_cron_scheduler` and manual Run-Now).
    /// Supports both LLM-driven (`Cron`/`Heartbeat`) and zero-token script-only (`Script`) modes.
    pub async fn dispatch_cron_job(self: &Arc<Self>, job: &CronJob) -> Result<Value> {
        let cfg = self.config();
        match job.kind {
            CronKind::Script => {
                let raw_script = job
                    .script_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        ZenkoError::Tool(format!(
                            "script cron job `{}` is missing `script_path`",
                            job.name
                        ))
                    })?;
                let scripts_dir = cfg.ensure_hub_layout()?.join(HUB_SUBDIR_SCRIPTS);
                let resolved =
                    zenko_tools::jail::resolve_jailed_path(&scripts_dir, raw_script, true)?;
                let cmd = build_script_command(&resolved);
                let task = self
                    .jobs
                    .spawn_shell(
                        "cron",
                        &cmd,
                        &scripts_dir,
                        TaskKind::Cron,
                        Some(format!("cron: {}", job.name)),
                        false,
                    )
                    .await?;
                let cron_entry = zenko_core::emit_hono_line(
                    zenko_core::LogDirection::Work,
                    zenko_core::LogChannel::Cron,
                    &format!("{} ({}) -> {}", job.name, job.cron_expr, raw_script),
                    Some(("script", zenko_core::LogStatusTone::Info)),
                    Some("0-tok"),
                )
                .with_call(task.id.clone())
                .with_payload(json!({
                    "job_id": job.id,
                    "task_id": task.id,
                    "script_path": raw_script,
                    "log_path": task.log_path,
                }));
                self.logs.push(cron_entry);
                Ok(json!({
                    "mode": "script",
                    "task": task,
                }))
            }
            CronKind::Cron | CronKind::Heartbeat => {
                let session = self.store.find_or_create_channel_session(
                    &job.target_channel,
                    job.target_chat_id.as_deref().unwrap_or(&job.id),
                    job.target_thread_id.as_deref(),
                    &format!("Cron: {}", job.name),
                    cfg.policy.mode,
                )?;
                let cron_entry = zenko_core::emit_hono_line(
                    zenko_core::LogDirection::Work,
                    zenko_core::LogChannel::Cron,
                    &format!("{} ({})", job.name, job.cron_expr),
                    Some((job.kind.as_str(), zenko_core::LogStatusTone::Info)),
                    Some("firing"),
                )
                .with_session(session.id.clone())
                .with_payload(json!({
                    "job_id": job.id,
                    "job_name": job.name,
                    "cron_expr": job.cron_expr,
                    "kind": job.kind.as_str(),
                    "target_channel": job.target_channel,
                }));
                self.logs.push(cron_entry);

                let rt = self.clone();
                let prompt = job.prompt.clone();
                let job_name = job.name.clone();
                let job_id = job.id.clone();
                let sid = session.id.clone();
                let headless = !rt.is_telegram_connected();
                tokio::spawn(async move {
                    if let Err(e) = rt
                        .run_turn_with_options(
                            &sid,
                            &prompt,
                            TurnOptions {
                                headless_no_wait: headless,
                                ..TurnOptions::default()
                            },
                        )
                        .await
                    {
                        warn!(session_id = %sid, error = %e, "cron job run failed");
                        let err_entry = zenko_core::emit_hono_line(
                            zenko_core::LogDirection::Err,
                            zenko_core::LogChannel::Cron,
                            &format!("{job_name} · {e}"),
                            Some(("err", zenko_core::LogStatusTone::Err)),
                            None,
                        )
                        .with_session(sid.clone())
                        .with_payload(json!({
                            "job_id": job_id,
                            "error": e.to_string(),
                        }));
                        rt.logs.push(err_entry);
                    }
                });
                Ok(json!({
                    "mode": job.kind.as_str(),
                    "session_id": session.id,
                }))
            }
        }
    }

    pub async fn trigger_cron_job_now(self: &Arc<Self>, id: &str) -> Result<Value> {
        let mut job = self
            .store
            .get_cron_job(id)?
            .ok_or_else(|| ZenkoError::NotFound(format!("cron job `{id}` not found")))?;
        job.last_run_at = Some(Utc::now());
        match self.dispatch_cron_job(&job).await {
            Ok(info) => {
                job.last_status = Some("dispatched".into());
                let _ = self.store.upsert_cron_job(&job);
                Ok(json!({ "ok": true, "job": job, "dispatch": info }))
            }
            Err(e) => {
                job.last_status = Some(format!("error: {e}"));
                let _ = self.store.upsert_cron_job(&job);
                Err(e)
            }
        }
    }

    /// Records a workspace mutation timestamp for watcher self-edit feedback-loop suppression.
    pub fn record_workspace_mutation(&self) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.last_workspace_mutation_ms
            .store(now_ms, Ordering::Relaxed);
    }

    pub fn recently_mutated_workspace(&self, window_secs: u64) -> bool {
        let last = self.last_workspace_mutation_ms.load(Ordering::Relaxed);
        if last == 0 {
            return false;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        now_ms.saturating_sub(last) <= window_secs.saturating_mul(1_000)
    }

    /// Runs a reusable zero-token script inside `~/zenko_hub/scripts/` via `LocalJobSupervisor`.
    pub async fn dispatch_hub_script(
        self: &Arc<Self>,
        label: &str,
        script_rel: &str,
    ) -> Result<BackgroundJobRecord> {
        let cfg = self.config();
        let scripts_dir = cfg.ensure_hub_layout()?.join(HUB_SUBDIR_SCRIPTS);
        let resolved = zenko_tools::jail::resolve_jailed_path(&scripts_dir, script_rel, true)?;
        let cmd = build_script_command(&resolved);
        self.jobs
            .spawn_shell(
                "trigger",
                &cmd,
                &scripts_dir,
                TaskKind::Script,
                Some(label.to_string()),
                false,
            )
            .await
    }

    // ─── Phase 8: Persistent Goal Loops, Event Triggers & Trajectory Export ────

    pub fn create_goal_loop(&self, req: GoalLoopCreateRequest) -> Result<GoalLoopRecord> {
        let cfg = self.config();
        if req.name.trim().is_empty() {
            return Err(ZenkoError::Tool("goal loop `name` cannot be empty".into()));
        }
        if req.goal_text.trim().is_empty() {
            return Err(ZenkoError::Tool(
                "goal loop `goal_text` cannot be empty".into(),
            ));
        }
        evaluate_verify_command_safety(&cfg.policy, &req.verify_cmd)
            .map_err(ZenkoError::PolicyDenied)?;

        let ws_dir = req
            .workspace_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(zenko_core::expand_tilde)
            .unwrap_or_else(|| cfg.resolved_workspace_dir());

        let session_id = match req
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(sid) => sid.to_string(),
            None => {
                let s = self.store.create_session(
                    &format!("Goal: {}", req.name.trim()),
                    "goal",
                    None,
                    None,
                    cfg.policy.mode,
                )?;
                let _ = self
                    .store
                    .set_session_workspace(&s.id, &ws_dir.display().to_string());
                s.id
            }
        };

        let now = Utc::now();
        let goal = GoalLoopRecord {
            id: new_goal_loop_id(),
            name: req.name.trim().to_string(),
            session_id,
            workspace_dir: ws_dir.display().to_string(),
            goal_text: req.goal_text.trim().to_string(),
            verify_cmd: req.verify_cmd,
            interval_secs: req
                .interval_secs
                .unwrap_or(GOAL_LOOP_DEFAULT_INTERVAL_SECS),
            max_iterations: req
                .max_iterations
                .unwrap_or(GOAL_LOOP_DEFAULT_MAX_ITERATIONS)
                .clamp(1, GOAL_LOOP_MAX_ITERATIONS_CAP),
            max_budget_usd: req
                .max_budget_usd
                .unwrap_or(GOAL_LOOP_DEFAULT_MAX_BUDGET_USD)
                .max(0.0),
            max_consecutive_stalls: req
                .max_consecutive_stalls
                .unwrap_or(GOAL_LOOP_DEFAULT_STALL_LIMIT)
                .clamp(1, GOAL_LOOP_MAX_STALL_CAP),
            checkpoint_each_iter: req.checkpoint_each_iter.unwrap_or(true),
            auto_rewind_on_stall: req.auto_rewind_on_stall.unwrap_or(false),
            model: req
                .model
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from),
            notify_channel: req
                .notify_channel
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(GOAL_LOOP_DEFAULT_NOTIFY_CHANNEL)
                .to_string(),
            status: GoalLoopStatus::Active,
            iterations_done: 0,
            consecutive_stalls: 0,
            cost_spent_usd: 0.0,
            last_verify_exit: None,
            last_verify_hash: None,
            last_verify_output: None,
            next_run_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        self.store.upsert_goal_loop(&goal)?;
        self.goal_notify.notify_one();
        self.emit(AgentEvent::GoalLoopUpdated { goal: goal.clone() });
        Ok(goal)
    }

    pub fn update_goal_loop(
        &self,
        id: &str,
        patch: GoalLoopPatch,
    ) -> Result<Option<GoalLoopRecord>> {
        let Some(mut goal) = self.store.get_goal_loop(id)? else {
            return Ok(None);
        };
        let cfg = self.config();
        if let Some(name) = patch.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            goal.name = name.to_string();
        }
        if let Some(text) = patch
            .goal_text
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            goal.goal_text = text.to_string();
        }
        if let Some(cmd) = patch.verify_cmd {
            evaluate_verify_command_safety(&cfg.policy, &cmd).map_err(ZenkoError::PolicyDenied)?;
            goal.verify_cmd = cmd;
        }
        if let Some(iv) = patch.interval_secs {
            goal.interval_secs = iv;
        }
        if let Some(mi) = patch.max_iterations {
            goal.max_iterations = mi.clamp(1, GOAL_LOOP_MAX_ITERATIONS_CAP);
        }
        if let Some(mb) = patch.max_budget_usd {
            goal.max_budget_usd = mb.max(0.0);
        }
        if patch.reset_stalls == Some(true) {
            goal.consecutive_stalls = 0;
        }
        if let Some(status) = patch.status {
            goal.status = status;
            goal.next_run_at = match status {
                GoalLoopStatus::Active => Some(Utc::now()),
                _ => None,
            };
        }
        goal.updated_at = Utc::now();
        self.store.upsert_goal_loop(&goal)?;
        self.goal_notify.notify_one();
        self.emit(AgentEvent::GoalLoopUpdated { goal: goal.clone() });
        Ok(Some(goal))
    }

    pub fn resume_and_wake_goal(&self, id: &str) -> Result<Option<GoalLoopRecord>> {
        self.update_goal_loop(
            id,
            GoalLoopPatch {
                status: Some(GoalLoopStatus::Active),
                reset_stalls: Some(true),
                ..GoalLoopPatch::default()
            },
        )
    }

    pub fn delete_goal_loop(&self, id: &str) -> Result<bool> {
        let deleted = self.store.delete_goal_loop(id)?;
        if deleted {
            self.goal_notify.notify_one();
        }
        Ok(deleted)
    }

    pub fn list_triggers(&self) -> Vec<TriggerStatusEntry> {
        let cfg = self.config();
        let mut out = Vec::new();
        for rule in &cfg.webhooks.rules {
            let snap = self.trigger_limiter.snapshot(&format!("webhook:{}", rule.id));
            let target = match rule.goal_loop_id.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(gid) => format!("goal:{gid}"),
                None => format!(
                    "session:{}",
                    rule.session_key.as_deref().unwrap_or(rule.source.as_str())
                ),
            };
            let filter_desc = match rule.match_pointer.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(ptr) => format!(
                    "source={} ptr={} values=[{}]",
                    rule.source,
                    ptr,
                    rule.match_values.join(",")
                ),
                None => format!("source={}", rule.source),
            };
            out.push(TriggerStatusEntry {
                id: rule.id.clone(),
                kind: "webhook_rule".into(),
                enabled: cfg.webhooks.enabled && rule.enabled,
                target,
                fires_last_hour: snap.fires_last_hour,
                max_per_hour: rule.max_per_hour,
                cooldown_secs: rule.cooldown_secs,
                last_fired_at: snap.last_fired_at,
                detail: filter_desc,
            });
        }
        for w in &cfg.watchers {
            let snap = self.trigger_limiter.snapshot(&format!("watcher:{}", w.id));
            let target = match w.action {
                zenko_core::WatcherActionKind::GoalLoop => {
                    format!("goal:{}", w.goal_loop_id.as_deref().unwrap_or("-"))
                }
                zenko_core::WatcherActionKind::Script => {
                    format!("script:{}", w.script_path.as_deref().unwrap_or("-"))
                }
                zenko_core::WatcherActionKind::Prompt => format!("watcher:{}", w.id),
            };
            out.push(TriggerStatusEntry {
                id: w.id.clone(),
                kind: "watcher".into(),
                enabled: w.enabled,
                target,
                fires_last_hour: snap.fires_last_hour,
                max_per_hour: w.max_per_hour,
                cooldown_secs: w.cooldown_secs,
                last_fired_at: snap.last_fired_at,
                detail: format!("path={} glob={} action={}", w.path, w.glob, w.action.as_str()),
            });
        }
        out
    }

    pub fn export_session_trajectory(
        &self,
        session_id: &str,
        opts: &ExportOptions,
    ) -> Result<ExportReport> {
        let secrets = self.vault.all_values();
        let home = zenko_core::expand_tilde("~");
        crate::export::export_session(&self.store, session_id, opts, &secrets, Some(&home))
    }

    pub fn export_batch_trajectories(&self, opts: &ExportOptions) -> Result<ExportReport> {
        let secrets = self.vault.all_values();
        let home = zenko_core::expand_tilde("~");
        crate::export::export_batch(&self.store, opts, &secrets, Some(&home))
    }

    // ─── Zero-CPU `sleep_until` Cron & Heartbeat Scheduler ─────────────────────

    pub async fn run_cron_scheduler(self: Arc<Self>, cancel: CancellationToken) {
        info!("cron & heartbeat scheduler started (zero-polling sleep_until)");
        loop {
            if cancel.is_cancelled() {
                break;
            }
            let jobs = self.store.list_cron_jobs().unwrap_or_default();
            let now = Utc::now();
            let mut next_due: Option<(chrono::DateTime<Utc>, CronJob)> = None;

            for mut job in jobs.into_iter().filter(|j| j.enabled) {
                let due = match job.next_run_at {
                    Some(t) => t,
                    None => {
                        let Ok(cron) = Cron::new(&job.cron_expr).parse() else {
                            continue;
                        };
                        let Ok(n) = cron.find_next_occurrence(&now, false) else {
                            continue;
                        };
                        job.next_run_at = Some(n);
                        let _ = self.store.upsert_cron_job(&job);
                        n
                    }
                };
                if next_due.as_ref().map(|(t, _)| due < *t).unwrap_or(true) {
                    next_due = Some((due, job));
                }
            }

            match next_due {
                None => {
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = self.cron_notify.notified() => continue,
                    }
                }
                Some((due, mut job)) => {
                    let wait_ms = (due - now).num_milliseconds();
                    if wait_ms > 0 {
                        let sleep = tokio::time::sleep(Duration::from_millis(wait_ms as u64));
                        tokio::select! {
                            _ = cancel.cancelled() => break,
                            _ = self.cron_notify.notified() => continue,
                            _ = sleep => {}
                        }
                    }

                    let next = Cron::new(&job.cron_expr)
                        .parse()
                        .ok()
                        .and_then(|c| c.find_next_occurrence(&Utc::now(), false).ok());
                    job.last_run_at = Some(Utc::now());
                    job.next_run_at = next;

                    match self.dispatch_cron_job(&job).await {
                        Ok(_) => {
                            job.last_status = Some("dispatched".into());
                        }
                        Err(e) => {
                            let err_entry = zenko_core::emit_hono_line(
                                zenko_core::LogDirection::Err,
                                zenko_core::LogChannel::Cron,
                                &format!("{} · {e}", job.name),
                                Some(("err", zenko_core::LogStatusTone::Err)),
                                None,
                            )
                            .with_payload(json!({
                                "job_id": job.id,
                                "error": e.to_string(),
                            }));
                            self.logs.push(err_entry);
                            job.last_status = Some(format!("error: {e}"));
                        }
                    }
                    let _ = self.store.upsert_cron_job(&job);
                }
            }
        }
    }

    // ─── Local Unix Domain Socket JSON-RPC Control Plane (Owner-Only 0o600) ────

    #[cfg(unix)]
    pub async fn run_ipc_server(self: Arc<Self>, cancel: CancellationToken) {
        use std::os::unix::fs::PermissionsExt;

        let sock_path = self.config().socket_path();
        if sock_path.exists() {
            let _ = std::fs::remove_file(&sock_path);
        }
        let listener = match tokio::net::UnixListener::bind(&sock_path) {
            Ok(l) => l,
            Err(e) => {
                error!(path = %sock_path.display(), error = %e, "failed to bind unix IPC socket");
                return;
            }
        };
        let _ = std::fs::set_permissions(
            &sock_path,
            std::fs::Permissions::from_mode(FILE_MODE_OWNER_ONLY),
        );
        info!(path = %sock_path.display(), "daemon unix IPC control plane ready (0600)");

        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue; };
                    let rt = self.clone();
                    let conn_cancel = cancel.clone();
                    tokio::spawn(async move {
                        let (reader, mut writer) = stream.into_split();
                        let mut lines = BufReader::new(reader).lines();
                        while let Ok(Some(line)) = lines.next_line().await {
                            if let Ok(req) = serde_json::from_str::<Value>(&line) {
                                match req.get("method").and_then(Value::as_str) {
                                    Some(IPC_METHOD_LOGS_FOLLOW) => {
                                        let filter: LogFilter = req
                                            .get("params")
                                            .cloned()
                                            .and_then(|v| serde_json::from_value(v).ok())
                                            .unwrap_or_default();
                                        rt.stream_ipc_logs(
                                            &filter,
                                            &mut writer,
                                            conn_cancel.clone(),
                                        )
                                        .await;
                                        break;
                                    }
                                    Some(IPC_METHOD_SESSION_ATTACH) => {
                                        let sid = req
                                            .get("params")
                                            .and_then(|p| p.get("session_id"))
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_string();
                                        let replay = req
                                            .get("params")
                                            .and_then(|p| p.get("replay"))
                                            .and_then(Value::as_u64)
                                            .unwrap_or(ATTACH_DEFAULT_REPLAY_EVENTS as u64)
                                            as usize;
                                        rt.stream_ipc_session(
                                            &sid,
                                            replay,
                                            &mut writer,
                                            conn_cancel.clone(),
                                        )
                                        .await;
                                        break;
                                    }
                                    Some(IPC_METHOD_JOBS_FOLLOW) => {
                                        let job_id = req
                                            .get("params")
                                            .and_then(|p| p.get("job_id"))
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_string();
                                        let offset = req
                                            .get("params")
                                            .and_then(|p| p.get("offset"))
                                            .and_then(Value::as_u64)
                                            .unwrap_or(0);
                                        rt.stream_ipc_job_follow(
                                            &job_id,
                                            offset,
                                            &mut writer,
                                            conn_cancel.clone(),
                                        )
                                        .await;
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            let resp = rt.handle_ipc_line(&line).await;
                            let mut payload = serde_json::to_string(&resp).unwrap_or_default();
                            payload.push('\n');
                            if writer.write_all(payload.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    });
                }
            }
        }
        let _ = std::fs::remove_file(&sock_path);
    }

    #[cfg(not(unix))]
    pub async fn run_ipc_server(self: Arc<Self>, cancel: CancellationToken) {
        cancel.cancelled().await;
    }

    async fn handle_ipc_line(self: &Arc<Self>, line: &str) -> Value {
        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => return json!({"ok": false, "error": format!("invalid json: {e}")}),
        };
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let params = req.get("params");

        match method {
            IPC_METHOD_STATUS => json!({"ok": true, "result": self.telemetry()}),
            IPC_METHOD_PAIR => {
                let channel = params
                    .and_then(|p| p.get("channel"))
                    .and_then(Value::as_str)
                    .unwrap_or("telegram");
                let role = params
                    .and_then(|p| p.get("role"))
                    .and_then(Value::as_str)
                    .map(PeerRole::from_str_loose)
                    .unwrap_or(PeerRole::Owner);
                match self
                    .store
                    .create_pairing_code(channel, role, DEFAULT_PAIRING_TTL_MINS)
                {
                    Ok(code) => json!({"ok": true, "result": code}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_ASK => {
                let prompt = params
                    .and_then(|p| p.get("prompt"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let session_id = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str);
                let mode_override = params
                    .and_then(|p| p.get("mode"))
                    .and_then(Value::as_str)
                    .map(PermissionMode::from_str_loose);
                let model_override = params
                    .and_then(|p| p.get("model"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let workspace_override = params
                    .and_then(|p| p.get("workspace"))
                    .and_then(Value::as_str)
                    .map(PathBuf::from);
                let headless_no_wait = params
                    .and_then(|p| p.get("headless_no_wait"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);

                let sid = match session_id {
                    Some(id) => id.to_string(),
                    None => match self.store.create_session(
                        &make_session_title(prompt),
                        "cli",
                        None,
                        None,
                        mode_override.unwrap_or_else(|| self.config().policy.mode),
                    ) {
                        Ok(s) => s.id,
                        Err(e) => return json!({"ok": false, "error": e.to_string()}),
                    },
                };
                match self
                    .run_turn_with_options(
                        &sid,
                        prompt,
                        TurnOptions {
                            parent_event_override: None,
                            workspace_override,
                            model_override,
                            permission_override: mode_override,
                            headless_no_wait,
                            send_mode: params
                                .and_then(|p| p.get("send_mode"))
                                .and_then(Value::as_str)
                                .map(SendMode::from_str_loose)
                                .unwrap_or_default(),
                            ..TurnOptions::default()
                        },
                    )
                    .await
                {
                    Ok(turn) => json!({"ok": true, "result": turn}),
                    Err(e) => json!({"ok": false, "error": e.to_string(), "session_id": sid}),
                }
            }
            IPC_METHOD_STOP => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let cancelled = self.cancel_session(sid);
                json!({"ok": true, "result": {"cancelled": cancelled}})
            }
            IPC_METHOD_SESSIONS_LIST => {
                let limit = params
                    .and_then(|p| p.get("limit"))
                    .and_then(Value::as_u64)
                    .unwrap_or(JOB_DEFAULT_LIST_LIMIT as u64) as usize;
                match self.store.list_sessions(limit) {
                    Ok(list) => json!({"ok": true, "result": list}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SESSION_GET => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.store.get_session(sid) {
                    Ok(Some(session)) => {
                        let events = self.store.get_active_branch(sid).unwrap_or_default();
                        json!({"ok": true, "result": {"session": session, "events": events}})
                    }
                    Ok(None) => json!({"ok": false, "error": format!("session {sid} not found")}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SESSION_CLEAR => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let _ = self.cancel_session(sid);
                self.clear_session_deferred_tools(sid);
                self.notify_session_ended(sid).await;
                match self.store.clear_session_events(sid) {
                    Ok(()) => json!({"ok": true, "result": {"cleared": true}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SESSION_DELETE => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let _ = self.cancel_session(sid);
                self.clear_session_deferred_tools(sid);
                self.notify_session_ended(sid).await;
                match self.store.delete_session(sid) {
                    Ok(deleted) => json!({"ok": true, "result": {"deleted": deleted}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SESSION_CONTEXT => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.inspect_session_context(sid).await {
                    Ok(ctx) => json!({"ok": true, "result": ctx}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_APPROVALS_LIST => match self.store.list_pending_approvals() {
                Ok(list) => json!({"ok": true, "result": list}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            },
            IPC_METHOD_APPROVAL_RESOLVE => {
                let id = params
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let decision = match params
                    .and_then(|p| p.get("decision"))
                    .and_then(Value::as_str)
                    .unwrap_or("deny")
                {
                    "approve" | "allow" => ApprovalDecision::Approve,
                    "always" | "always_allow" => ApprovalDecision::AlwaysAllow,
                    _ => ApprovalDecision::Deny,
                };
                match self.resolve_approval(id, decision, "cli") {
                    Ok(rec) => json!({"ok": true, "result": {"resolved": rec.is_some(), "approval": rec}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MEMORY_SEARCH => {
                let q = params
                    .and_then(|p| p.get("query"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let limit = params
                    .and_then(|p| p.get("limit"))
                    .and_then(Value::as_u64)
                    .unwrap_or(20) as usize;
                let ws = params
                    .and_then(|p| p.get("workspace"))
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.config().resolved_workspace_dir());
                let proj_scope = project_scope_for_workspace(&ws);
                let (scope_opt, include_global) = match params
                    .and_then(|p| p.get("scope"))
                    .and_then(Value::as_str)
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
                match self
                    .store
                    .search_memory_hybrid(q, scope_opt, include_global, Some(&ws), limit)
                {
                    Ok(hits) => {
                        let entries: Vec<_> = hits.into_iter().map(|h| h.entry).collect();
                        json!({"ok": true, "result": entries})
                    }
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MEMORY_ADD => {
                let title = params
                    .and_then(|p| p.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or("Note");
                let content = params
                    .and_then(|p| p.get("content"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let kind = params
                    .and_then(|p| p.get("kind"))
                    .and_then(Value::as_str)
                    .unwrap_or("fact");
                let ws = params
                    .and_then(|p| p.get("workspace"))
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.config().resolved_workspace_dir());
                let scope = match params
                    .and_then(|p| p.get("scope"))
                    .and_then(Value::as_str)
                    .unwrap_or(MEMORY_SCOPE_GLOBAL)
                    .trim()
                {
                    "project" => project_scope_for_workspace(&ws),
                    "" => MEMORY_SCOPE_GLOBAL.to_string(),
                    other => other.to_string(),
                };
                match self
                    .store
                    .upsert_memory_dedup(&scope, kind, title, content, 1.0, None, None, true)
                {
                    Ok(res) => json!({"ok": true, "result": res.entry, "merged": res.merged}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MEMORY_UPDATE => {
                let id = params
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.store.update_memory(
                    id,
                    params.and_then(|p| p.get("title")).and_then(Value::as_str),
                    params.and_then(|p| p.get("content")).and_then(Value::as_str),
                    params.and_then(|p| p.get("kind")).and_then(Value::as_str),
                    params.and_then(|p| p.get("salience")).and_then(Value::as_f64),
                    params.and_then(|p| p.get("scope")).and_then(Value::as_str),
                ) {
                    Ok(Some(entry)) => json!({"ok": true, "result": entry}),
                    Ok(None) => json!({"ok": false, "error": format!("memory `{id}` not found")}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MEMORY_DELETE => {
                let id = params
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.store.delete_memory(id) {
                    Ok(deleted) => json!({"ok": true, "result": {"deleted": deleted}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MEMORY_DREAM => {
                let force = params
                    .and_then(|p| p.get("force"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let trigger = params
                    .and_then(|p| p.get("trigger"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli");
                match self.run_dream_now(trigger, force).await {
                    Ok(rec) => json!({"ok": true, "result": rec}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MEMORY_HISTORY => {
                let doc = params.and_then(|p| p.get("doc")).and_then(Value::as_str);
                match self.snapshots.list_doc_history(doc) {
                    Ok(list) => json!({"ok": true, "result": list}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MEMORY_ROLLBACK => {
                let sid = params
                    .and_then(|p| p.get("snapshot_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.snapshots.rollback_doc(sid) {
                    Ok((doc, diff)) => json!({"ok": true, "result": {"doc": doc, "diff": diff}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SESSIONS_SEARCH => {
                let query = params
                    .and_then(|p| p.get("query"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let since = params
                    .and_then(|p| p.get("since"))
                    .and_then(Value::as_str)
                    .and_then(parse_since_window);
                let channel = params
                    .and_then(|p| p.get("channel"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let role = params
                    .and_then(|p| p.get("role"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let session_id = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let limit = params
                    .and_then(|p| p.get("limit"))
                    .and_then(Value::as_u64)
                    .map(|v| v as usize);
                let workspace_dir = match params
                    .and_then(|p| p.get("workspace"))
                    .and_then(Value::as_str)
                    .unwrap_or("all")
                    .trim()
                {
                    "" | "all" => None,
                    "current" => Some(
                        resolve_git_root(&self.config().resolved_workspace_dir())
                            .display()
                            .to_string(),
                    ),
                    other => Some(resolve_git_root(&PathBuf::from(other)).display().to_string()),
                };
                let filter = SessionSearchFilter {
                    query,
                    since,
                    channel,
                    workspace_dir,
                    session_id,
                    exclude_session_id: None,
                    role,
                    limit,
                };
                match self.store.search_sessions_fts(&filter) {
                    Ok(hits) => json!({"ok": true, "result": hits}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_CRON_LIST => match self.store.list_cron_jobs() {
                Ok(jobs) => json!({"ok": true, "result": jobs}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            },
            IPC_METHOD_CRON_CREATE => {
                let name = params
                    .and_then(|p| p.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("Scheduled Task");
                let expr = params
                    .and_then(|p| p.get("cron_expr"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let prompt = params
                    .and_then(|p| p.get("prompt"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let script_path = params
                    .and_then(|p| p.get("script_path"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                let kind = CronKind::from_str_loose(
                    params
                        .and_then(|p| p.get("kind"))
                        .and_then(Value::as_str)
                        .unwrap_or("cron"),
                );
                let target_channel = params
                    .and_then(|p| p.get("target_channel"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli");
                let parsed = match Cron::new(expr).parse() {
                    Ok(c) => c,
                    Err(e) => return json!({"ok": false, "error": format!("invalid cron expression: {e}")}),
                };
                let job = CronJob {
                    id: new_id(),
                    name: name.to_string(),
                    cron_expr: expr.to_string(),
                    kind,
                    prompt: prompt.to_string(),
                    script_path,
                    target_channel: target_channel.to_string(),
                    target_chat_id: None,
                    target_thread_id: None,
                    enabled: true,
                    next_run_at: parsed.find_next_occurrence(&Utc::now(), false).ok(),
                    last_run_at: None,
                    last_status: None,
                };
                match self.store.upsert_cron_job(&job) {
                    Ok(()) => {
                        self.notify_cron_updated();
                        json!({"ok": true, "result": job})
                    }
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_CRON_UPDATE => {
                let id = params
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let mut job = match self.store.get_cron_job(id) {
                    Ok(Some(j)) => j,
                    Ok(None) => return json!({"ok": false, "error": format!("cron job `{id}` not found")}),
                    Err(e) => return json!({"ok": false, "error": e.to_string()}),
                };
                if let Some(n) = params.and_then(|p| p.get("name")).and_then(Value::as_str) {
                    if !n.trim().is_empty() {
                        job.name = n.trim().to_string();
                    }
                }
                if let Some(expr) = params.and_then(|p| p.get("cron_expr")).and_then(Value::as_str) {
                    let parsed = match Cron::new(expr).parse() {
                        Ok(c) => c,
                        Err(e) => return json!({"ok": false, "error": format!("invalid cron expression: {e}")}),
                    };
                    job.cron_expr = expr.to_string();
                    job.next_run_at = parsed.find_next_occurrence(&Utc::now(), false).ok();
                }
                if let Some(k) = params.and_then(|p| p.get("kind")).and_then(Value::as_str) {
                    job.kind = CronKind::from_str_loose(k);
                }
                if let Some(pr) = params.and_then(|p| p.get("prompt")).and_then(Value::as_str) {
                    job.prompt = pr.to_string();
                }
                if let Some(sp) = params.and_then(|p| p.get("script_path")).and_then(Value::as_str) {
                    let trimmed = sp.trim();
                    job.script_path = match trimmed.is_empty() {
                        true => None,
                        false => Some(trimmed.to_string()),
                    };
                }
                if let Some(en) = params.and_then(|p| p.get("enabled")).and_then(Value::as_bool) {
                    job.enabled = en;
                }
                match self.store.upsert_cron_job(&job) {
                    Ok(()) => {
                        self.notify_cron_updated();
                        json!({"ok": true, "result": job})
                    }
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_CRON_RUN_NOW => {
                let id = params
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.trigger_cron_job_now(id).await {
                    Ok(res) => json!({"ok": true, "result": res}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_CRON_DELETE => {
                let id = params
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.store.delete_cron_job(id) {
                    Ok(deleted) => {
                        if deleted {
                            self.notify_cron_updated();
                        }
                        json!({"ok": true, "result": {"deleted": deleted}})
                    }
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_LOGS_RECENT | IPC_METHOD_LOGS_QUERY => {
                let mut filter: LogFilter = params
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                if filter.limit.is_none() {
                    filter.limit = Some(60);
                }
                let items = match self.store.query_logs(&filter) {
                    Ok(rows) if !rows.is_empty() => rows,
                    Ok(_) => self
                        .logs
                        .recent(filter.limit.unwrap_or(60))
                        .into_iter()
                        .filter(|e| filter.matches(e))
                        .collect(),
                    Err(_) => self
                        .logs
                        .recent(filter.limit.unwrap_or(60))
                        .into_iter()
                        .filter(|e| filter.matches(e))
                        .collect(),
                };
                json!({"ok": true, "result": items})
            }
            IPC_METHOD_LOGS_STATS => {
                let counts = self.store.log_category_counts().unwrap_or_default();
                json!({
                    "ok": true,
                    "result": {
                        "logs_dir": self.log_sink.logs_dir().display().to_string(),
                        "category_counts": counts,
                        "dropped_entries": self.log_sink.dropped_entries(),
                        "wire_debug_enabled": wire_debug_enabled(),
                        "categories": LOG_CATEGORIES,
                    }
                })
            }
            IPC_METHOD_JOBS_LIST => {
                let session_id = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty());
                let kind = params
                    .and_then(|p| p.get("kind"))
                    .and_then(Value::as_str)
                    .and_then(TaskKind::parse_filter);
                let limit = params
                    .and_then(|p| p.get("limit"))
                    .and_then(Value::as_u64)
                    .unwrap_or(JOB_DEFAULT_LIST_LIMIT as u64) as usize;
                match self.jobs.list_jobs(session_id, kind, limit) {
                    Ok(list) => json!({"ok": true, "result": list}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_START => {
                let command = params
                    .and_then(|p| p.get("command"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if command.trim().is_empty() {
                    return json!({"ok": false, "error": "missing `command`"});
                }
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("cli");
                let kind = params
                    .and_then(|p| p.get("kind"))
                    .and_then(Value::as_str)
                    .and_then(TaskKind::parse_filter)
                    .unwrap_or_else(|| TaskKind::infer_from_command(command));
                let label = params
                    .and_then(|p| p.get("label"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                let ws = params
                    .and_then(|p| p.get("workspace"))
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.config().resolved_workspace_dir());
                match self
                    .jobs
                    .spawn_shell(sid, command, &ws, kind, label, false)
                    .await
                {
                    Ok(rec) => json!({"ok": true, "result": rec}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_UPDATE => {
                let job_id = params
                    .and_then(|p| p.get("job_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let label = params
                    .and_then(|p| p.get("label"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let kind = params
                    .and_then(|p| p.get("kind"))
                    .and_then(Value::as_str)
                    .and_then(TaskKind::parse_filter);
                match self.jobs.update_job_meta(job_id, label, kind) {
                    Ok(Some(rec)) => json!({"ok": true, "result": rec}),
                    Ok(None) => json!({"ok": false, "error": format!("task `{job_id}` not found")}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_RESTART => {
                let job_id = params
                    .and_then(|p| p.get("job_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.jobs.restart_job(job_id).await {
                    Ok(rec) => json!({"ok": true, "result": rec}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_DELETE => {
                let job_id = params
                    .and_then(|p| p.get("job_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.jobs.delete_job(job_id).await {
                    Ok(deleted) => json!({"ok": true, "result": {"deleted": deleted, "job_id": job_id}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_PRUNE => {
                let kind = params
                    .and_then(|p| p.get("kind"))
                    .and_then(Value::as_str)
                    .and_then(TaskKind::parse_filter);
                match self.jobs.prune_jobs(kind).await {
                    Ok(pruned) => json!({"ok": true, "result": {"pruned": pruned}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_HUB_LIST => {
                let cat = params.and_then(|p| p.get("category")).and_then(Value::as_str);
                let limit = params
                    .and_then(|p| p.get("limit"))
                    .and_then(Value::as_u64)
                    .unwrap_or(100) as usize;
                match self.list_hub_entries(cat, limit) {
                    Ok(entries) => json!({
                        "ok": true,
                        "result": {
                            "hub_dir": self.config().resolved_hub_dir().display().to_string(),
                            "categories": HUB_CATEGORIES,
                            "entries": entries,
                        }
                    }),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_READ => {
                let job_id = params
                    .and_then(|p| p.get("job_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let offset = params.and_then(|p| p.get("offset")).and_then(Value::as_u64);
                let max_bytes = params
                    .and_then(|p| p.get("max_bytes"))
                    .and_then(Value::as_u64)
                    .unwrap_or(JOB_DEFAULT_READ_BYTES as u64) as usize;
                match self.jobs.read_job_output(job_id, offset, max_bytes).await {
                    Ok(chunk) => json!({"ok": true, "result": chunk}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_INPUT => {
                let job_id = params
                    .and_then(|p| p.get("job_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let input = params
                    .and_then(|p| p.get("input"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.jobs.send_input(job_id, input).await {
                    Ok(()) => json!({"ok": true, "result": {"sent": true}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_STOP => {
                let job_id = params
                    .and_then(|p| p.get("job_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.jobs.stop_job(job_id).await {
                    Ok(rec) => json!({"ok": true, "result": {"stopped": rec.is_some(), "job": rec}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_WAIT => {
                let job_id = params
                    .and_then(|p| p.get("job_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let timeout_secs = params
                    .and_then(|p| p.get("timeout_secs"))
                    .and_then(Value::as_u64)
                    .unwrap_or(JOB_DEFAULT_WAIT_SECS)
                    .clamp(1, JOB_MAX_WAIT_SECS);
                let tail_bytes = params
                    .and_then(|p| p.get("tail_bytes"))
                    .and_then(Value::as_u64)
                    .unwrap_or(JOB_DEFAULT_READ_BYTES as u64) as usize;
                match self
                    .jobs
                    .wait_job(job_id, Duration::from_secs(timeout_secs))
                    .await
                {
                    Ok(Some(rec)) => {
                        let chunk = self
                            .jobs
                            .read_job_output(&rec.id, None, tail_bytes)
                            .await
                            .ok();
                        json!({"ok": true, "result": {"job": rec, "output": chunk}})
                    }
                    Ok(None) => json!({"ok": false, "error": format!("job `{job_id}` not found")}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SESSION_STEER => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let text = params
                    .and_then(|p| p.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let steered = self.steer_session(sid, text);
                json!({"ok": true, "result": {"steered": steered, "session_id": sid}})
            }
            IPC_METHOD_SESSION_BACKGROUND => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let demoted = self.demote_foreground_job(sid);
                json!({"ok": true, "result": {"demoted": demoted, "session_id": sid}})
            }
            IPC_METHOD_SESSION_QUEUE_LIST => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.store.list_queued_followups(sid) {
                    Ok(list) => json!({"ok": true, "result": list}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SESSION_QUEUE_CANCEL => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let queue_id = params
                    .and_then(|p| p.get("queue_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.cancel_session_followup(sid, queue_id) {
                    Ok((cancelled, count)) => {
                        json!({"ok": true, "result": {"cancelled": cancelled, "queued_count": count}})
                    }
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_CLARIFICATIONS_LIST => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty());
                let list = self.clarifications.list_pending_clarifications(sid);
                json!({"ok": true, "result": list})
            }
            IPC_METHOD_CLARIFICATION_RESOLVE => {
                let id = params
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let selected = params
                    .and_then(|p| p.get("selected_option_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let custom = params
                    .and_then(|p| p.get("custom_answer"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                let resolved_by = params
                    .and_then(|p| p.get("resolved_by"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli");
                let rec = self
                    .clarifications
                    .resolve_clarification(id, selected, custom, resolved_by);
                json!({"ok": true, "result": {"resolved": rec.is_some(), "clarification": rec}})
            }
            IPC_METHOD_ORCHESTRATOR_TRIAGE => {
                let prompt = params
                    .and_then(|p| p.get("prompt"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let mode = params
                    .and_then(|p| p.get("mode"))
                    .and_then(Value::as_str)
                    .map(PermissionMode::from_str_loose);
                let report = self.triage_prompt(prompt, mode);
                json!({"ok": true, "result": report})
            }
            IPC_METHOD_AGENTS_LIST => {
                let parent_session_id = params
                    .and_then(|p| p.get("parent_session_id"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty());
                let limit = params
                    .and_then(|p| p.get("limit"))
                    .and_then(Value::as_u64)
                    .unwrap_or(SUBAGENT_LIST_DEFAULT_LIMIT as u64) as usize;
                match self.store.list_subagents(parent_session_id, limit) {
                    Ok(list) => json!({"ok": true, "result": list}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_GET => {
                let id = params
                    .and_then(|p| p.get("agent_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.store.get_subagent(id) {
                    Ok(Some(rec)) => {
                        let messages = self.store.list_agent_messages(id, 40).unwrap_or_default();
                        json!({"ok": true, "result": {"agent": rec, "messages": messages}})
                    }
                    Ok(None) => json!({"ok": false, "error": format!("subagent `{id}` not found")}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_SPAWN => {
                let parent_session_id = params
                    .and_then(|p| p.get("parent_session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli")
                    .to_string();
                let role = params
                    .and_then(|p| p.get("role"))
                    .and_then(Value::as_str)
                    .unwrap_or("explorer")
                    .to_string();
                let goal = params
                    .and_then(|p| p.get("goal"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let context = params
                    .and_then(|p| p.get("context"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let run_in_background = params
                    .and_then(|p| p.get("run_in_background"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let workspace_mode = params
                    .and_then(|p| p.get("workspace"))
                    .and_then(Value::as_str)
                    .map(SubagentWorkspaceMode::from_str_loose);
                let output_schema = params
                    .and_then(|p| p.get("output_schema"))
                    .filter(|v| v.is_object())
                    .cloned();
                let model_override = params
                    .and_then(|p| p.get("model"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let fork_context = params
                    .and_then(|p| p.get("fork_context"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let include_uncommitted = params
                    .and_then(|p| p.get("include_uncommitted"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let max_duration_secs = params
                    .and_then(|p| p.get("max_duration_secs"))
                    .and_then(Value::as_u64);
                let max_cost_usd = params
                    .and_then(|p| p.get("max_cost_usd"))
                    .and_then(Value::as_f64);
                match self
                    .subagents
                    .spawn_agent(
                        self,
                        SubagentSpawnRequest {
                            parent_session_id,
                            role,
                            goal,
                            context,
                            run_in_background,
                            workspace_mode,
                            output_schema,
                            model_override,
                            fork_context,
                            include_uncommitted,
                            max_duration_secs,
                            max_cost_usd,
                        },
                    )
                    .await
                {
                    Ok(rec) => json!({"ok": true, "result": rec}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_MESSAGE => {
                let from_id = params
                    .and_then(|p| p.get("from_session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli");
                let to_id = params
                    .and_then(|p| p.get("agent_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let message = params
                    .and_then(|p| p.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let resume = params
                    .and_then(|p| p.get("resume"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let interrupt = params
                    .and_then(|p| p.get("interrupt"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                match self
                    .subagents
                    .send_message(
                        self,
                        from_id,
                        to_id,
                        message,
                        zenko_core::AgentMessageOptions { resume, interrupt },
                    )
                    .await
                {
                    Ok(mail) => json!({"ok": true, "result": mail}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_WAIT => {
                let parent_id = params
                    .and_then(|p| p.get("parent_session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let agent_ids: Vec<String> = params
                    .and_then(|p| p.get("agent_ids"))
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                let wait_all = params
                    .and_then(|p| p.get("wait_all"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let timeout_secs = params
                    .and_then(|p| p.get("timeout_secs"))
                    .and_then(Value::as_u64)
                    .unwrap_or(SUBAGENT_DEFAULT_WAIT_SECS)
                    .clamp(1, SUBAGENT_MAX_WAIT_SECS);
                match self
                    .subagents
                    .wait_agents(
                        self,
                        parent_id,
                        &agent_ids,
                        wait_all,
                        Duration::from_secs(timeout_secs),
                    )
                    .await
                {
                    Ok(list) => json!({"ok": true, "result": list}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_KILL => {
                let caller = params
                    .and_then(|p| p.get("caller_session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli");
                let agent_id = params
                    .and_then(|p| p.get("agent_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let remove_wt = params
                    .and_then(|p| p.get("remove_worktree"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                match self
                    .subagents
                    .kill_agent(self, caller, agent_id, remove_wt)
                    .await
                {
                    Ok(rec) => json!({"ok": true, "result": {"killed": rec.is_some(), "agent": rec}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_DIFF => {
                let caller = params
                    .and_then(|p| p.get("caller_session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli");
                let agent_id = params
                    .and_then(|p| p.get("agent_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self
                    .subagents
                    .inspect_worktree_diff(self, caller, agent_id)
                    .await
                {
                    Ok(Some(rep)) => json!({"ok": true, "result": rep}),
                    Ok(None) => json!({"ok": false, "error": format!("no git worktree for subagent `{agent_id}`")}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_MERGE => {
                let caller = params
                    .and_then(|p| p.get("caller_session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli");
                let agent_id = params
                    .and_then(|p| p.get("agent_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let cleanup = params
                    .and_then(|p| p.get("cleanup"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                match self
                    .subagents
                    .merge_worktree(self, caller, agent_id, cleanup)
                    .await
                {
                    Ok(rep) => json!({"ok": true, "result": rep}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_DISCARD => {
                let caller = params
                    .and_then(|p| p.get("caller_session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli");
                let agent_id = params
                    .and_then(|p| p.get("agent_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.subagents.discard_worktree(self, caller, agent_id).await {
                    Ok(rep) => json!({"ok": true, "result": rep}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_PRUNE => match self.subagents.prune_orphan_worktrees(self).await {
                Ok(rep) => json!({"ok": true, "result": rep}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            },
            IPC_METHOD_TASKS_DAG_LIST => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let ready_only = params
                    .and_then(|p| p.get("ready_only"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let res = match ready_only {
                    true => self.clarifications.ready_session_tasks(sid),
                    false => self.clarifications.list_session_tasks(sid),
                };
                match res {
                    Ok(tasks) => json!({"ok": true, "result": tasks}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_TASKS_DAG_CREATE => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let title = params
                    .and_then(|p| p.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let id = params
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .unwrap_or_else(new_session_task_id);
                let description = params
                    .and_then(|p| p.get("description"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let depends_on: Vec<String> = params
                    .and_then(|p| p.get("depends_on"))
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                let verification_cmd = params
                    .and_then(|p| p.get("verification_cmd"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let priority = params
                    .and_then(|p| p.get("priority"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0) as i32;
                let now = Utc::now();
                let task = SessionTask {
                    id,
                    session_id: sid.to_string(),
                    branch_event_id: None,
                    title,
                    description,
                    active_label: None,
                    status: SessionTaskStatus::Pending,
                    depends_on,
                    owner_agent: None,
                    verification_cmd,
                    verification_evidence: None,
                    notes: None,
                    priority,
                    created_at: now,
                    updated_at: now,
                };
                match self.clarifications.create_session_task(sid, task) {
                    Ok(saved) => json!({"ok": true, "result": saved}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_TASKS_DAG_UPDATE => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let tid = params
                    .and_then(|p| p.get("task_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let claim_owner = params
                    .and_then(|p| p.get("claim_owner"))
                    .and_then(Value::as_str);
                let res = match claim_owner {
                    Some(owner) => self.clarifications.claim_session_task(sid, tid, owner),
                    None => {
                        let parse_opt_str = |key: &str| {
                            params.and_then(|p| p.get(key)).map(|v| {
                                v.as_str()
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                    .map(String::from)
                            })
                        };
                        let parse_str_arr = |key: &str| -> Option<Vec<String>> {
                            params
                                .and_then(|p| p.get(key))
                                .and_then(Value::as_array)
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(Value::as_str)
                                        .map(str::trim)
                                        .filter(|s| !s.is_empty())
                                        .map(String::from)
                                        .collect()
                                })
                        };
                        let patch = SessionTaskPatch {
                            title: params
                                .and_then(|p| p.get("title"))
                                .and_then(Value::as_str)
                                .map(String::from),
                            description: params
                                .and_then(|p| p.get("description"))
                                .and_then(Value::as_str)
                                .map(String::from),
                            active_label: parse_opt_str("active_label"),
                            status: params
                                .and_then(|p| p.get("status"))
                                .and_then(Value::as_str)
                                .map(SessionTaskStatus::from_str_loose),
                            depends_on: parse_str_arr("depends_on"),
                            add_depends_on: parse_str_arr("add_depends_on").unwrap_or_default(),
                            owner_agent: parse_opt_str("owner_agent"),
                            verification_cmd: parse_opt_str("verification_cmd"),
                            verification_evidence: parse_opt_str("verification_evidence"),
                            notes: parse_opt_str("notes"),
                            priority: params
                                .and_then(|p| p.get("priority"))
                                .and_then(Value::as_i64)
                                .map(|v| v as i32),
                        };
                        self.clarifications.update_session_task(sid, tid, patch)
                    }
                };
                match res {
                    Ok(updated) => json!({"ok": true, "result": updated}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_TASKS_DAG_DELETE => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let clear_all = params
                    .and_then(|p| p.get("clear_all"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                match clear_all {
                    true => match self.clarifications.clear_session_tasks(sid) {
                        Ok(n) => json!({"ok": true, "result": {"cleared": n}}),
                        Err(e) => json!({"ok": false, "error": e.to_string()}),
                    },
                    false => {
                        let tid = params
                            .and_then(|p| p.get("task_id"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        match self.clarifications.delete_session_task(sid, tid) {
                            Ok(deleted) => json!({"ok": true, "result": {"deleted": deleted}}),
                            Err(e) => json!({"ok": false, "error": e.to_string()}),
                        }
                    }
                }
            }
            IPC_METHOD_CHECKPOINTS_LIST => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty());
                let limit = params
                    .and_then(|p| p.get("limit"))
                    .and_then(Value::as_u64)
                    .unwrap_or(CHECKPOINT_LIST_DEFAULT_LIMIT as u64)
                    as usize;
                match self.store.list_checkpoints(sid, limit) {
                    Ok(list) => json!({"ok": true, "result": list}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_CHECKPOINTS_DIFF => {
                let cp_id = params
                    .and_then(|p| p.get("checkpoint_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let cp_cfg = self.config().checkpoints;
                match self.checkpoints.diff_checkpoint(&cp_cfg, cp_id).await {
                    Ok(rep) => json!({"ok": true, "result": rep}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_CHECKPOINTS_REWIND => {
                let cp_id = params
                    .and_then(|p| p.get("checkpoint_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if let Ok(Some(existing)) = self.store.get_checkpoint(&cp_id) {
                    if self.is_session_running(&existing.session_id) {
                        return json!({
                            "ok": false,
                            "error": "Cannot rewind while a turn is actively running on this session. Stop the turn first."
                        });
                    }
                }
                let mode = params
                    .and_then(|p| p.get("mode"))
                    .and_then(Value::as_str)
                    .map(RewindMode::from_str_loose)
                    .unwrap_or(RewindMode::CodeAndConversation);
                let files: Vec<String> = params
                    .and_then(|p| p.get("files"))
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                let overwrite_manual_edits = params
                    .and_then(|p| p.get("overwrite_manual_edits"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let summarize_abandoned = params
                    .and_then(|p| p.get("summarize_abandoned"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let cp_cfg = self.config().checkpoints;
                match self
                    .checkpoints
                    .rewind(
                        &cp_cfg,
                        &RewindRequest {
                            checkpoint_id: cp_id,
                            mode,
                            files,
                            overwrite_manual_edits,
                            summarize_abandoned,
                        },
                    )
                    .await
                {
                    Ok(rep) => json!({"ok": true, "result": rep}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_CHECKPOINTS_PRUNE => {
                let cp_cfg = self.config().checkpoints;
                match self.checkpoints.prune_checkpoints(&cp_cfg).await {
                    Ok(rep) => json!({"ok": true, "result": rep}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SESSION_APPROVE_PLAN => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let target_mode = params
                    .and_then(|p| p.get("mode"))
                    .and_then(Value::as_str)
                    .map(PermissionMode::from_str_loose)
                    .unwrap_or(PermissionMode::Autonomous);
                match self.approve_session_plan(sid, target_mode) {
                    Ok((sess, task_count)) => json!({
                        "ok": true,
                        "result": {
                            "session": sess,
                            "task_count": task_count,
                        }
                    }),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MCP_SERVERS_LIST => {
                let servers = self.mcp_clients.list_status().await;
                let configs = self.mcp_clients.configs();
                json!({
                    "ok": true,
                    "result": {
                        "servers": servers,
                        "configs": configs,
                    }
                })
            }
            IPC_METHOD_MCP_SERVER_UPSERT => {
                let sync_now = params
                    .and_then(|p| p.get("sync"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let srv_val = params
                    .and_then(|p| p.get("server"))
                    .cloned()
                    .or(params.cloned())
                    .unwrap_or(Value::Null);
                match serde_json::from_value::<McpServerConfig>(srv_val) {
                    Ok(srv) => match self.upsert_mcp_server(srv, sync_now).await {
                        Ok((servers, sync_error)) => json!({
                            "ok": true,
                            "result": {
                                "servers": servers,
                                "sync_error": sync_error,
                            }
                        }),
                        Err(e) => json!({"ok": false, "error": e.to_string()}),
                    },
                    Err(e) => json!({"ok": false, "error": format!("invalid MCP server config: {e}")}),
                }
            }
            IPC_METHOD_MCP_SERVER_DELETE => {
                let name = params
                    .and_then(|p| p.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.delete_mcp_server(name).await {
                    Ok(deleted) => json!({"ok": true, "result": {"deleted": deleted}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MCP_SERVER_SYNC => {
                let name = params
                    .and_then(|p| p.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.sync_mcp_server(name).await {
                    Ok(status) => json!({"ok": true, "result": status}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_LSP_STATUS => {
                let active = self.lsp.list_status().await;
                let cfg = self.config().lsp;
                json!({
                    "ok": true,
                    "result": {
                        "enabled": cfg.enabled,
                        "idle_timeout_secs": cfg.idle_timeout_secs,
                        "active": active,
                    }
                })
            }
            IPC_METHOD_LSP_STOP => {
                let stopped = self.lsp.stop_all().await;
                json!({"ok": true, "result": {"stopped": stopped}})
            }
            IPC_METHOD_SKILLS_LIST => match self.snapshots.list_skills() {
                Ok(skills) => json!({"ok": true, "result": skills}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            },
            IPC_METHOD_SKILL_GET => {
                let slug = params
                    .and_then(|p| p.get("slug"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let subfile = params.and_then(|p| p.get("file_path")).and_then(Value::as_str);
                match self.snapshots.read_skill(slug, subfile) {
                    Ok((skill, sub)) => json!({"ok": true, "result": {"skill": skill, "subfile": sub}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SKILL_MANAGE => {
                let action_s = params
                    .and_then(|p| p.get("action"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let Some(action) = SkillManageAction::parse(action_s) else {
                    return json!({"ok": false, "error": format!("unknown skill action `{action_s}`")});
                };
                let cfg = self.config().skills;
                let secrets = self.vault.all_values();
                let slug = params
                    .and_then(|p| p.get("slug"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match action {
                    SkillManageAction::Create => {
                        let content = params
                            .and_then(|p| p.get("content"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let name = params.and_then(|p| p.get("name")).and_then(Value::as_str);
                        let desc = params
                            .and_then(|p| p.get("description"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let enabled = params
                            .and_then(|p| p.get("enabled"))
                            .and_then(Value::as_bool)
                            .unwrap_or(true);
                        match self.snapshots.create_skill_structured(
                            slug,
                            name,
                            desc,
                            content,
                            enabled,
                            SkillStatus::Active,
                            SkillAuthor::User,
                            &[],
                            &[],
                            "cli",
                            &secrets,
                            cfg.max_history_snapshots,
                        ) {
                            Ok((doc, snap)) => {
                                self.emit(AgentEvent::SkillUpdated {
                                    slug: doc.slug.clone(),
                                    action: "created".into(),
                                    status: doc.status.as_str().into(),
                                });
                                json!({"ok": true, "result": {"skill": doc, "snapshot": snap}})
                            }
                            Err(e) => json!({"ok": false, "error": e.to_string()}),
                        }
                    }
                    SkillManageAction::Toggle => {
                        let enabled = params
                            .and_then(|p| p.get("enabled"))
                            .and_then(Value::as_bool)
                            .unwrap_or(true);
                        match self
                            .snapshots
                            .toggle_skill(slug, enabled, cfg.max_history_snapshots)
                        {
                            Ok((doc, snap)) => {
                                self.emit(AgentEvent::SkillUpdated {
                                    slug: doc.slug.clone(),
                                    action: "toggled".into(),
                                    status: doc.status.as_str().into(),
                                });
                                json!({"ok": true, "result": {"skill": doc, "snapshot": snap}})
                            }
                            Err(e) => json!({"ok": false, "error": e.to_string()}),
                        }
                    }
                    SkillManageAction::Delete => match self.snapshots.delete_skill(slug) {
                        Ok(deleted) => json!({"ok": true, "result": {"deleted": deleted}}),
                        Err(e) => json!({"ok": false, "error": e.to_string()}),
                    },
                    SkillManageAction::Patch => {
                        let old_text = params
                            .and_then(|p| p.get("old_text"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let new_text = params
                            .and_then(|p| p.get("new_text"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        match self.snapshots.patch_skill_with_secrets(
                            slug,
                            old_text,
                            new_text,
                            "cli_patch",
                            &secrets,
                            cfg.max_history_snapshots,
                        ) {
                            Ok((doc, diff, snap)) => {
                                json!({"ok": true, "result": {"skill": doc, "diff": diff, "snapshot": snap}})
                            }
                            Err(e) => json!({"ok": false, "error": e.to_string()}),
                        }
                    }
                    _ => json!({"ok": false, "error": format!("unsupported IPC skill action `{action_s}`")}),
                }
            }
            IPC_METHOD_SKILL_APPROVE => {
                let slug = params
                    .and_then(|p| p.get("slug"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let max_h = self.config().skills.max_history_snapshots;
                match self.snapshots.approve_skill(slug, max_h) {
                    Ok((doc, snap)) => {
                        self.emit(AgentEvent::SkillUpdated {
                            slug: doc.slug.clone(),
                            action: "approved".into(),
                            status: doc.status.as_str().into(),
                        });
                        json!({"ok": true, "result": {"skill": doc, "snapshot": snap}})
                    }
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SKILL_HISTORY => {
                let slug = params.and_then(|p| p.get("slug")).and_then(Value::as_str);
                match self.snapshots.list_skill_history(slug) {
                    Ok(items) => json!({"ok": true, "result": items}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SKILL_ROLLBACK => {
                let sid = params
                    .and_then(|p| p.get("snapshot_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let max_h = self.config().skills.max_history_snapshots;
                match self.snapshots.rollback_skill(sid, max_h) {
                    Ok((doc, diff)) => {
                        self.emit(AgentEvent::SkillUpdated {
                            slug: doc.slug.clone(),
                            action: "rollback".into(),
                            status: doc.status.as_str().into(),
                        });
                        json!({"ok": true, "result": {"skill": doc, "diff": diff}})
                    }
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_GOALS_LIST => {
                let limit = params
                    .and_then(|p| p.get("limit"))
                    .and_then(Value::as_u64)
                    .unwrap_or(GOAL_LOOP_LIST_DEFAULT_LIMIT as u64) as usize;
                match self.store.list_goal_loops(limit) {
                    Ok(goals) => json!({"ok": true, "result": goals}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_GOAL_GET => {
                let id = params.and_then(|p| p.get("id")).and_then(Value::as_str).unwrap_or("");
                match self.store.get_goal_loop(id) {
                    Ok(Some(goal)) => {
                        let runs = self
                            .store
                            .list_goal_loop_runs(&goal.id, GOAL_LOOP_RUNS_DEFAULT_LIMIT)
                            .unwrap_or_default();
                        json!({"ok": true, "result": {"goal": goal, "runs": runs}})
                    }
                    Ok(None) => json!({"ok": false, "error": format!("goal loop `{id}` not found")}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_GOAL_CREATE => {
                let req: std::result::Result<GoalLoopCreateRequest, _> =
                    serde_json::from_value(params.cloned().unwrap_or(Value::Null));
                match req {
                    Ok(r) => match self.create_goal_loop(r) {
                        Ok(goal) => json!({"ok": true, "result": goal}),
                        Err(e) => json!({"ok": false, "error": e.to_string()}),
                    },
                    Err(e) => json!({"ok": false, "error": format!("invalid goal create payload: {e}")}),
                }
            }
            IPC_METHOD_GOAL_UPDATE => {
                let id = params.and_then(|p| p.get("id")).and_then(Value::as_str).unwrap_or("");
                let patch: GoalLoopPatch = params
                    .and_then(|p| p.get("patch"))
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                match self.update_goal_loop(id, patch) {
                    Ok(Some(goal)) => json!({"ok": true, "result": goal}),
                    Ok(None) => json!({"ok": false, "error": format!("goal loop `{id}` not found")}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_GOAL_RUN_NOW => {
                let id = params.and_then(|p| p.get("id")).and_then(Value::as_str).unwrap_or("");
                match crate::goals::execute_goal_iteration(self, id, CancellationToken::new()).await {
                    Ok(goal) => json!({"ok": true, "result": goal}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_GOAL_DELETE => {
                let id = params.and_then(|p| p.get("id")).and_then(Value::as_str).unwrap_or("");
                match self.delete_goal_loop(id) {
                    Ok(deleted) => json!({"ok": true, "result": {"deleted": deleted}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_TRIGGERS_LIST => {
                json!({"ok": true, "result": self.list_triggers()})
            }
            IPC_METHOD_EXPORT_SESSION => {
                let session_id = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let opts: ExportOptions = params
                    .and_then(|p| p.get("options"))
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                let res = match session_id {
                    Some(sid) => self.export_session_trajectory(sid, &opts),
                    None => self.export_batch_trajectories(&opts),
                };
                match res {
                    Ok(rep) => json!({"ok": true, "result": rep}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            other => json!({"ok": false, "error": format!("unknown method: {other}")}),
        }
    }

    #[cfg(unix)]
    async fn stream_ipc_logs(
        self: &Arc<Self>,
        filter: &LogFilter,
        writer: &mut tokio::net::unix::OwnedWriteHalf,
        cancel: CancellationToken,
    ) {
        let mut rx = self.subscribe();
        let initial_limit = filter.limit.unwrap_or(20);
        if initial_limit > 0 {
            let mut init_filter = filter.clone();
            init_filter.limit = Some(initial_limit);
            if let Ok(rows) = self.store.query_logs(&init_filter) {
                for entry in rows {
                    let mut line = serde_json::to_string(&json!({
                        "ok": true,
                        "event": "log",
                        "entry": entry,
                    }))
                    .unwrap_or_default();
                    line.push('\n');
                    if writer.write_all(line.as_bytes()).await.is_err() {
                        return;
                    }
                }
            }
        }

        loop {
            let ev = tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                msg = rx.recv() => match msg {
                    Ok(e) => e,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            };
            if let AgentEvent::Log(entry) = ev {
                if !filter.matches(&entry) {
                    continue;
                }
                let mut line = serde_json::to_string(&json!({
                    "ok": true,
                    "event": "log",
                    "entry": entry,
                }))
                .unwrap_or_default();
                line.push('\n');
                if writer.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
        }
    }

    #[cfg(unix)]
    async fn stream_ipc_session(
        self: &Arc<Self>,
        session_id: &str,
        replay: usize,
        writer: &mut tokio::net::unix::OwnedWriteHalf,
        cancel: CancellationToken,
    ) {
        let mut rx = self.subscribe();
        let watch_all = session_id.trim().is_empty() || session_id == "*";

        if !watch_all {
            let Some(session) = self.store.get_session(session_id).ok().flatten() else {
                let mut err = serde_json::to_string(&json!({
                    "ok": false,
                    "error": format!("session `{session_id}` not found"),
                }))
                .unwrap_or_default();
                err.push('\n');
                let _ = writer.write_all(err.as_bytes()).await;
                return;
            };
            let branch = self.store.get_active_branch(session_id).unwrap_or_default();
            let start = branch.len().saturating_sub(replay);
            let queued = self
                .store
                .list_queued_followups(session_id)
                .unwrap_or_default();
            let jobs = self
                .jobs
                .list_jobs(Some(session_id), None, JOB_DEFAULT_LIST_LIMIT)
                .unwrap_or_default();
            let mut snap = serde_json::to_string(&json!({
                "ok": true,
                "kind": IPC_FRAME_SNAPSHOT,
                "session": session,
                "events": &branch[start..],
                "queued_prompts": queued,
                "jobs": jobs,
            }))
            .unwrap_or_default();
            snap.push('\n');
            if writer.write_all(snap.as_bytes()).await.is_err() {
                return;
            }
        }

        loop {
            let ev = tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                msg = rx.recv() => match msg {
                    Ok(e) => e,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            };
            let Some(ev_sid) = agent_event_session_id(&ev) else {
                continue;
            };
            if !watch_all && ev_sid != session_id {
                continue;
            }
            let mut line = serde_json::to_string(&json!({
                "ok": true,
                "kind": IPC_FRAME_EVENT,
                "data": ev,
            }))
            .unwrap_or_default();
            line.push('\n');
            if writer.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    }

    /// Event-driven, zero-polling live log follower for background jobs over Unix IPC.
    #[cfg(unix)]
    async fn stream_ipc_job_follow(
        self: &Arc<Self>,
        job_id: &str,
        initial_offset: u64,
        writer: &mut tokio::net::unix::OwnedWriteHalf,
        cancel: CancellationToken,
    ) {
        // Subscribe to progress watch BEFORE initial read so zero bytes can slip through.
        let mut progress = self.jobs.subscribe_progress(job_id);
        let mut offset = initial_offset;

        let (next_off, initial_chunks, mut last_job) =
            match drain_job_chunks(&self.jobs, job_id, offset).await {
                Ok(v) => v,
                Err(e) => {
                    let _ =
                        write_ipc_frame(writer, json!({"ok": false, "error": e.to_string()})).await;
                    return;
                }
            };
        offset = next_off;
        for chunk in initial_chunks {
            if !write_ipc_frame(
                writer,
                json!({"ok": true, "kind": IPC_FRAME_CHUNK, "data": chunk}),
            )
            .await
            {
                return;
            }
        }

        if let Some((ref mut bytes_rx, ref mut done_rx)) = progress {
            loop {
                let finished = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    res = done_rx.wait_for(|&d| d) => res.is_ok() || res.is_err(),
                    changed = bytes_rx.changed() => changed.is_err(),
                };

                if let Ok((next_o, chunks, job_opt)) =
                    drain_job_chunks(&self.jobs, job_id, offset).await
                {
                    offset = next_o;
                    if job_opt.is_some() {
                        last_job = job_opt;
                    }
                    for chunk in chunks {
                        if !write_ipc_frame(
                            writer,
                            json!({"ok": true, "kind": IPC_FRAME_CHUNK, "data": chunk}),
                        )
                        .await
                        {
                            return;
                        }
                    }
                }

                if finished {
                    break;
                }
            }
        }

        let final_job = self.jobs.get_job(job_id).ok().flatten().or(last_job);
        let _ = write_ipc_frame(
            writer,
            json!({"ok": true, "kind": IPC_FRAME_DONE, "job": final_job}),
        )
        .await;
    }
}

#[cfg(unix)]
async fn write_ipc_frame(writer: &mut tokio::net::unix::OwnedWriteHalf, payload: Value) -> bool {
    let mut line = serde_json::to_string(&payload).unwrap_or_default();
    line.push('\n');
    writer.write_all(line.as_bytes()).await.is_ok()
}

#[cfg(unix)]
async fn drain_job_chunks(
    jobs: &Arc<LocalJobSupervisor>,
    job_id: &str,
    start_offset: u64,
) -> Result<(
    u64,
    Vec<zenko_core::JobReadChunk>,
    Option<zenko_core::BackgroundJobRecord>,
)> {
    let mut cur = start_offset;
    let mut chunks = Vec::new();
    let latest_job = loop {
        let chunk = jobs
            .read_job_output(job_id, Some(cur), JOB_DEFAULT_READ_BYTES)
            .await?;
        let advanced = chunk.next_offset > cur;
        cur = chunk.next_offset;
        if advanced && !chunk.content.is_empty() {
            chunks.push(chunk);
        } else {
            break Some(chunk.job);
        }
    };
    Ok((cur, chunks, latest_job))
}

/// Declarative single-match classifier identifying tool calls that may mutate the workspace on disk
/// (triggering lazy once-per-turn pre-mutation checkpoint capture):
/// - Direct file mutation tools (`write_file`, `hash_edit`, `apply_patch`)
/// - Non-read-only `shell_exec` commands
/// - `agent_worktree(action="merge")` merging isolated branches into the main workspace
/// - Mutating `spawn_agent` roles inheriting the parent's working tree directly
/// Deterministically extracts workspace-relative paths modified by `write_file`, `hash_edit`,
/// or `apply_patch` (zero regex) for Oh My Pi–style warm LSP Diagnostics-on-Edit.
fn extract_edited_rel_paths(tool_name: &str, args: &Value) -> Vec<String> {
    let mut out = Vec::new();
    match tool_name {
        "write_file" | "hash_edit" => {
            if let Some(p) = args
                .get("path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty() && !s.starts_with("hub:"))
            {
                out.push(p.to_string());
            }
        }
        "apply_patch" => {
            if let Some(patch) = args.get("patch").and_then(Value::as_str) {
                for line in patch.lines() {
                    let trimmed = line.trim();
                    let rel = trimmed
                        .strip_prefix("*** Update File:")
                        .or_else(|| trimmed.strip_prefix("*** Add File:"))
                        .map(str::trim)
                        .filter(|s| !s.is_empty() && !s.starts_with("hub:"));
                    if let Some(p) = rel {
                        if !out.iter().any(|existing| existing == p) {
                            out.push(p.to_string());
                        }
                    }
                }
            }
        }
        _ => {}
    }
    out
}

fn is_workspace_mutating_call(tc: &ToolCall) -> Option<&str> {
    let name = tc.name.as_str();
    match name {
        n if MUTATING_FILE_TOOLS.contains(&n) => Some(n),
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
                    .map_or(true, |m| m.is_empty() || m == active_model);
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

fn truncate_preview(s: &str, max: usize) -> String {
    match s.len() <= max {
        true => s.to_string(),
        false => {
            let cut = s.floor_char_boundary(max);
            format!("{}...", &s[..cut])
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steering_injects_only_while_session_active_without_cancelling() {
        let dir = std::env::temp_dir().join(format!("zenko_rt_steer_{}", new_id()));
        let mut cfg = ZenkoConfig::default();
        cfg.general.data_dir = dir.display().to_string();
        cfg.general.hub_dir = dir.join("hub").display().to_string();
        cfg.general.workspace_dir = dir.display().to_string();
        let (rt, _logs) = AgentRuntime::build(dir.join("zenko.toml"), cfg).expect("build runtime");

        // Idle session cannot be steered mid-flight
        assert!(!rt.steer_session("s_active", "focus on tests"));

        // Register an active token simulating an in-flight turn
        let tok = CancellationToken::new();
        rt.active_cancels
            .lock()
            .expect("lock")
            .insert("s_active".into(), (1, tok.clone()));

        assert!(rt.steer_session("s_active", "focus on unit tests first"));
        assert!(!tok.is_cancelled(), "steering must NEVER cancel an active turn");
        let drained = rt.drain_steering("s_active");
        assert_eq!(drained, vec!["focus on unit tests first"]);
        assert!(rt.drain_steering("s_active").is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn durable_sqlite_followup_queue_and_3state_tool_classification() {
        let dir = std::env::temp_dir().join(format!("zenko_rt_queue_{}", new_id()));
        let mut cfg = ZenkoConfig::default();
        cfg.general.data_dir = dir.display().to_string();
        cfg.general.hub_dir = dir.join("hub").display().to_string();
        cfg.general.workspace_dir = dir.display().to_string();
        let (rt, _logs) = AgentRuntime::build(dir.join("zenko.toml"), cfg).expect("build runtime");

        let session = rt
            .store
            .create_session("Queue Test", "cli", None, None, PermissionMode::Balanced)
            .expect("create session");

        // 1. Durable SQLite follow-up queue enqueue / list / cancel / pop FIFO
        let (q1, c1) = rt
            .enqueue_session_followup(&session.id, "first follow-up")
            .expect("enqueue 1");
        let (q2, c2) = rt
            .enqueue_session_followup(&session.id, "second follow-up")
            .expect("enqueue 2");
        let (q3, c3) = rt
            .enqueue_session_followup(&session.id, "third follow-up")
            .expect("enqueue 3");
        assert_eq!((c1, c2, c3), (1, 2, 3));

        let (cancelled, rem) = rt
            .cancel_session_followup(&session.id, &q2.id)
            .expect("cancel 2");
        assert!(cancelled);
        assert_eq!(rem, 2);

        let popped1 = rt
            .store
            .pop_next_followup(&session.id)
            .expect("pop 1")
            .expect("q1");
        assert_eq!(popped1.id, q1.id);
        let popped2 = rt
            .store
            .pop_next_followup(&session.id)
            .expect("pop 2")
            .expect("q3");
        assert_eq!(popped2.id, q3.id);
        assert!(rt.store.pop_next_followup(&session.id).expect("pop empty").is_none());

        // 2. 3-state concurrency classification respects permission policy
        let cfg_now = rt.config();
        let write_a = ToolCall {
            id: "c1".into(),
            name: "write_file".into(),
            arguments: json!({"path": "./src/a.rs", "content": "x"}),
        };
        let write_b = ToolCall {
            id: "c2".into(),
            name: "write_file".into(),
            arguments: json!({"path": "src/b.rs", "content": "y"}),
        };
        let read_a = ToolCall {
            id: "c3".into(),
            name: "read_file".into(),
            arguments: json!({"path": "src/a.rs"}),
        };
        assert_eq!(
            rt.classify_tool_call(&cfg_now, PermissionMode::Balanced, &read_a),
            ToolConcurrency::Shared
        );
        assert_eq!(
            rt.classify_tool_call(&cfg_now, PermissionMode::Balanced, &write_a),
            ToolConcurrency::Keyed(zenko_core::ResourceLockKey::File("src/a.rs".into()))
        );
        assert_eq!(
            rt.classify_tool_call(&cfg_now, PermissionMode::Balanced, &write_b),
            ToolConcurrency::Keyed(zenko_core::ResourceLockKey::File("src/b.rs".into()))
        );
        // Strict mode forces HITL approval -> downgrades writes to Global barrier
        assert_eq!(
            rt.classify_tool_call(&cfg_now, PermissionMode::Strict, &write_a),
            ToolConcurrency::Global
        );

        // 3. Per-goal async lock returns the same Arc mutex for identical goal IDs and distinct locks across goals
        let lock_a1 = rt.goal_lock("goal_alpha");
        let lock_a2 = rt.goal_lock("goal_alpha");
        let lock_b = rt.goal_lock("goal_beta");
        assert!(Arc::ptr_eq(&lock_a1, &lock_a2));
        assert!(!Arc::ptr_eq(&lock_a1, &lock_b));
        let _guard = lock_a1.try_lock().expect("first lock acquires");
        assert!(
            lock_a2.try_lock().is_err(),
            "concurrent acquisition on same goal_id must serialize"
        );
        assert!(
            lock_b.try_lock().is_ok(),
            "different goal_id must run independently without blocking"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn auto_dispatch_queues_foreground_and_steers_during_bg_or_subagents() {
        // Pure state-machine table check (zero I/O)
        assert_eq!(
            resolve_auto_send_mode(SessionDispatchState {
                is_running: false,
                active_jobs: 0,
                active_subagents: 0,
                is_monitoring_tool_active: false,
            }),
            AutoDispatchDecision::StartTurn
        );
        assert_eq!(
            resolve_auto_send_mode(SessionDispatchState {
                is_running: true,
                active_jobs: 0,
                active_subagents: 0,
                is_monitoring_tool_active: false,
            }),
            AutoDispatchDecision::Queue
        );
        assert_eq!(
            resolve_auto_send_mode(SessionDispatchState {
                is_running: true,
                active_jobs: 1,
                active_subagents: 0,
                is_monitoring_tool_active: false,
            }),
            AutoDispatchDecision::Steer
        );
        assert_eq!(
            resolve_auto_send_mode(SessionDispatchState {
                is_running: true,
                active_jobs: 0,
                active_subagents: 2,
                is_monitoring_tool_active: false,
            }),
            AutoDispatchDecision::Steer
        );
        assert_eq!(
            resolve_auto_send_mode(SessionDispatchState {
                is_running: true,
                active_jobs: 0,
                active_subagents: 0,
                is_monitoring_tool_active: true,
            }),
            AutoDispatchDecision::Steer
        );

        // Runtime integration check with active-tool RAII guard + running session map
        let dir = std::env::temp_dir().join(format!("zenko_autodisp_{}", new_id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let mut cfg = ZenkoConfig::default();
        cfg.general.data_dir = dir.display().to_string();
        cfg.general.hub_dir = dir.join("hub").display().to_string();
        cfg.general.workspace_dir = dir.display().to_string();
        let (rt, _logs) = AgentRuntime::build(dir.join("zenko.toml"), cfg).expect("runtime");
        let sess = rt
            .store
            .create_session("Auto Dispatch", "webui", None, None, PermissionMode::Autonomous)
            .expect("session");

        // Idle -> StartTurn
        assert_eq!(
            rt.resolve_session_dispatch(&sess.id, SendMode::Auto),
            AutoDispatchDecision::StartTurn
        );

        // Mark running (foreground work) -> Queue
        rt.active_cancels
            .lock()
            .expect("lock")
            .insert(sess.id.clone(), (1, CancellationToken::new()));
        assert_eq!(
            rt.resolve_session_dispatch(&sess.id, SendMode::Auto),
            AutoDispatchDecision::Queue
        );

        // Monitoring tool active (`job_wait`) -> Steer, then automatically reverts to Queue on Drop!
        {
            let _guard = ActiveToolGuard::enter(
                rt.session_control(&sess.id),
                zenko_core::TOOL_JOB_WAIT.to_string(),
            );
            assert_eq!(
                rt.resolve_session_dispatch(&sess.id, SendMode::Auto),
                AutoDispatchDecision::Steer
            );
        }
        assert_eq!(
            rt.resolve_session_dispatch(&sess.id, SendMode::Auto),
            AutoDispatchDecision::Queue
        );

        // Promote a queued follow-up directly into live steering
        let (queued, count) = rt
            .enqueue_session_followup(&sess.id, "switch to async")
            .expect("enqueue");
        assert_eq!(count, 1);
        let (promoted, remaining) = rt
            .promote_queued_to_steer(&sess.id, &queued.id)
            .expect("promote");
        assert!(promoted);
        assert_eq!(remaining, 0);
        assert_eq!(rt.drain_steering(&sess.id), vec!["switch to async".to_string()]);

        rt.active_cancels.lock().expect("lock").remove(&sess.id);
        let _ = std::fs::remove_dir_all(dir);
    }
}
