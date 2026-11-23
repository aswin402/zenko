use crate::error::Result;
use crate::types::{
    AgentMailMessage, AgentMessageOptions, BackgroundJobRecord, BrainstormSpec, ChatRequest,
    ClarificationAnswer, ClarificationOption, ClarificationRecord, ContextBreakdown, ExecutionPlan,
    GoalLoopCreateRequest, GoalLoopPatch, GoalLoopRecord, GoalLoopRun, JobReadChunk, LlmDelta,
    ModelInfo, SessionTask, SessionTaskPatch, SubagentRecord, SubagentSpawnRequest, TaskKind,
    ToolConcurrency, ToolExposure, ToolOutput, ToolSpec, WorktreeDiffReport, WorktreePruneReport,
};
use async_trait::async_trait;
use futures_core::Stream;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

pub type DeltaStream = Pin<Box<dyn Stream<Item = Result<LlmDelta>> + Send>>;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn id(&self) -> &str;

    async fn list_models(&self) -> Result<Vec<ModelInfo>>;

    async fn verify_key(&self) -> Result<()>;

    async fn stream_chat(
        &self,
        req: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<DeltaStream>;
}

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub session_id: String,
    pub workspace_dir: PathBuf,
    pub data_dir: PathBuf,
    /// User-visible default creations home (`~/zenko_hub/`).
    pub hub_dir: PathBuf,
    pub shell_timeout_secs: u64,
    pub auto_background_after_secs: u64,
    pub max_output_bytes: usize,
    /// When true, tools such as `ask_clarification` auto-resolve deterministically instead of blocking.
    pub headless_no_wait: bool,
    /// Live signal triggered when a user/API requests demoting the active foreground
    /// command to an async background job without killing it.
    pub demote_bg: Arc<Notify>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;

    /// Declarative default concurrency lock scope used by the parallel wave scheduler.
    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Global
    }

    /// Declarative schema exposure (`Eager` vs `Deferred` for prompt-cache-safe lazy tool loading).
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Eager
    }

    /// Optional argument-aware concurrency refinement (e.g. keyed file locks or read vs write actions).
    fn concurrency_for_args(&self, _args: &serde_json::Value) -> ToolConcurrency {
        self.concurrency()
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: serde_json::Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput>;
}

/// Async background process & ZenkoTasks supervisor shared between `zenko-runtime` and `zenko-tools`.
#[async_trait]
pub trait JobSupervisor: Send + Sync {
    async fn spawn_shell(
        &self,
        session_id: &str,
        command: &str,
        workspace_dir: &Path,
        kind: TaskKind,
        label: Option<String>,
        notify_on_complete: bool,
    ) -> Result<BackgroundJobRecord>;

    /// Enables reactive completion wakeups for a running task (used when demoting foreground -> bg).
    fn enable_completion_notify(&self, job_id: &str);

    fn list_jobs(
        &self,
        session_id: Option<&str>,
        kind: Option<TaskKind>,
        limit: usize,
    ) -> Result<Vec<BackgroundJobRecord>>;

    fn get_job(&self, job_id: &str) -> Result<Option<BackgroundJobRecord>>;

    fn update_job_meta(
        &self,
        job_id: &str,
        label: Option<String>,
        kind: Option<TaskKind>,
    ) -> Result<Option<BackgroundJobRecord>>;

    async fn restart_job(&self, job_id: &str) -> Result<BackgroundJobRecord>;

    async fn delete_job(&self, job_id: &str) -> Result<bool>;

    async fn prune_jobs(&self, kind: Option<TaskKind>) -> Result<usize>;

    async fn read_job_output(
        &self,
        job_id: &str,
        byte_offset: Option<u64>,
        max_bytes: usize,
    ) -> Result<JobReadChunk>;

    async fn clear_job_output(&self, job_id: &str) -> Result<Option<BackgroundJobRecord>>;

    async fn send_input(&self, job_id: &str, input: &str) -> Result<()>;

    async fn stop_job(&self, job_id: &str) -> Result<Option<BackgroundJobRecord>>;

    async fn wait_job(
        &self,
        job_id: &str,
        timeout: std::time::Duration,
    ) -> Result<Option<BackgroundJobRecord>>;

    async fn stop_session_jobs(&self, session_id: &str) -> usize;

    async fn stop_all(&self);

    fn active_count(&self) -> usize;
}

/// Bridge connecting orchestrator tools (`record_brainstorm`, `plan_update`, `ask_clarification`)
/// in `zenko-tools` to `zenko-runtime` without circular dependencies.
#[async_trait]
pub trait OrchestratorBridge: Send + Sync {
    /// Normalizes, scores, and broadcasts a structured `BrainstormSpec` to connected clients.
    fn record_brainstorm(&self, session_id: &str, spec: &BrainstormSpec) -> Result<BrainstormSpec>;

    /// Validates DAG invariants (cycle-free dependencies) and broadcasts a live `ExecutionPlan`.
    fn update_plan(&self, session_id: &str, plan: &ExecutionPlan) -> Result<ExecutionPlan>;

    /// Requests structured human clarification (or deterministically auto-resolves in headless mode).
    async fn request_clarification(
        &self,
        session_id: &str,
        question: &str,
        context: &str,
        options: Vec<ClarificationOption>,
        headless_no_wait: bool,
        cancel: CancellationToken,
    ) -> Result<ClarificationAnswer>;

    /// Lists currently pending clarifications (optionally filtered by session).
    fn list_pending_clarifications(&self, session_id: Option<&str>) -> Vec<ClarificationRecord>;

    /// Lists all SQLite-backed Task DAG nodes for a session in deterministic priority/creation order.
    fn list_session_tasks(&self, session_id: &str) -> Result<Vec<SessionTask>>;

    /// Computes topologically ready tasks (`pending` whose `depends_on` prerequisites are all `done`).
    fn ready_session_tasks(&self, session_id: &str) -> Result<Vec<SessionTask>>;

    /// Validates DAG acyclicity and inserts a durable SQLite task node.
    fn create_session_task(&self, session_id: &str, task: SessionTask) -> Result<SessionTask>;

    /// Applies a partial update to an existing task node while preserving DAG acyclicity & transitive blocks.
    fn update_session_task(
        &self,
        session_id: &str,
        task_id: &str,
        patch: SessionTaskPatch,
    ) -> Result<SessionTask>;

    /// Atomically claims an unclaimed (or already self-owned) ready task for `owner_agent` in SQLite.
    fn claim_session_task(
        &self,
        session_id: &str,
        task_id: &str,
        owner_agent: &str,
    ) -> Result<SessionTask>;

    /// Deletes a single task node and removes dangling dependency references from downstream nodes.
    fn delete_session_task(&self, session_id: &str, task_id: &str) -> Result<bool>;

    /// Clears all task DAG nodes for `session_id`.
    fn clear_session_tasks(&self, session_id: &str) -> Result<usize>;
}

/// Bridge connecting Phase 3 context tools (`tool_search`, `context_inspect`) in `zenko-tools`
/// to `AgentRuntime` without circular dependencies.
#[async_trait]
pub trait ContextBridge: Send + Sync {
    /// Searches deferred tool schemas via BM25/token scoring, activates matching tools for
    /// `session_id`, and returns their specifications.
    fn search_and_activate_deferred_tools(
        &self,
        session_id: &str,
        query: &str,
        limit: usize,
    ) -> Vec<ToolSpec>;

    /// Computes live per-tier token usage, cache hit %, microcompaction savings, active checkpoint,
    /// and spilled artifacts count for `session_id`.
    async fn inspect_session_context(&self, session_id: &str) -> Result<ContextBreakdown>;
}

/// Bridge connecting Phase 5 subagent tools (`spawn_agent`, `agent_list`, `agent_message`,
/// `agent_wait`, `agent_kill`) in `zenko-tools` to `AgentRuntime` without circular dependencies.
#[async_trait]
pub trait SubagentBridge: Send + Sync {
    /// Spawns a scoped child agent (blocking until completion or returning immediately when `run_in_background`).
    async fn spawn_agent(&self, req: SubagentSpawnRequest) -> Result<SubagentRecord>;

    /// Lists spawned subagents (optionally filtered by `parent_session_id`).
    fn list_agents(
        &self,
        parent_session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SubagentRecord>>;

    /// Fetches a single subagent record by ID.
    fn get_agent(&self, agent_id: &str) -> Result<Option<SubagentRecord>>;

    /// Sends a nuclear-family-enforced mailbox message from `from_session_id` to `to_session_id`,
    /// optionally resuming idle/completed children or interrupting active ones.
    async fn send_message(
        &self,
        from_session_id: &str,
        to_session_id: &str,
        message: &str,
        opts: AgentMessageOptions,
    ) -> Result<AgentMailMessage>;

    /// Zero-CPU event-driven await for one or more background child agents.
    async fn wait_agents(
        &self,
        parent_session_id: &str,
        agent_ids: &[String],
        wait_all: bool,
        timeout: std::time::Duration,
    ) -> Result<Vec<SubagentRecord>>;

    /// Cascade-cancels a subagent and its descendants, stops child shell jobs, and optionally removes its worktree.
    async fn kill_agent(
        &self,
        caller_session_id: &str,
        agent_id: &str,
        remove_worktree: bool,
    ) -> Result<Option<SubagentRecord>>;

    /// Inspects git diff / changed files inside a worktree-isolated subagent (subtree-ownership enforced).
    async fn inspect_worktree_diff(
        &self,
        caller_session_id: &str,
        agent_id: &str,
    ) -> Result<Option<WorktreeDiffReport>>;

    /// Cleanly merges a completed worktree subagent's branch into the parent repository (subtree-ownership enforced).
    async fn merge_worktree(
        &self,
        caller_session_id: &str,
        agent_id: &str,
        cleanup: bool,
    ) -> Result<WorktreeDiffReport>;

    /// Discards a completed/terminated subagent's git worktree & temporary branch without merging.
    async fn discard_worktree(
        &self,
        caller_session_id: &str,
        agent_id: &str,
    ) -> Result<WorktreeDiffReport>;

    /// Safely prunes orphaned crash-leftover worktree directories under `<data_dir>/worktrees/`.
    async fn prune_orphan_worktrees(&self) -> Result<WorktreePruneReport>;
}

/// Bridge connecting Phase 8 `goal_loop` tool in `zenko-tools` to `AgentRuntime` without cycles.
#[async_trait]
pub trait GoalLoopBridge: Send + Sync {
    fn list_goals(&self, limit: usize) -> Result<Vec<GoalLoopRecord>>;
    fn get_goal(&self, id: &str) -> Result<Option<(GoalLoopRecord, Vec<GoalLoopRun>)>>;
    fn create_goal(&self, req: GoalLoopCreateRequest) -> Result<GoalLoopRecord>;
    fn update_goal(&self, id: &str, patch: GoalLoopPatch) -> Result<Option<GoalLoopRecord>>;
    async fn trigger_goal_now(&self, id: &str) -> Result<GoalLoopRecord>;
    fn delete_goal(&self, id: &str) -> Result<bool>;
}

#[derive(Debug, Clone, Copy)]
pub struct ChannelCapabilities {
    pub streaming_draft: bool,
    pub inline_buttons: bool,
    pub forum_topics: bool,
}
