//! Phase 5 Async Subagent Supervisor, Nuclear-Family Mailbox & Deterministic JSON Schema Validator.
//!
//! Synthesizes:
//! - Hermes Agent: strict context isolation, bounded depth/concurrency, child shell-job teardown
//! - Claude Code: one-way distilled return valve, targeted SendMessage steering, git worktrees
//! - OpenAI Codex CLI (`codex-rs`): directed agent tree, event-driven `wait_agents` (zero polling)
//! - Oh My Pi (`omp`): schema-validated structured JSON contracts & worktree diff/merge lifecycle
//! - Prime Agent: non-blocking parallel fan-out & nuclear-family A2A messaging guard

use crate::compaction::slice_from_latest_checkpoint;
use crate::policy::{run_hooks, run_hooks_with_context};
use crate::runtime::{AgentRuntime, TurnOptions};
use crate::worktree::{
    WorktreeHandle, create_worktree, inspect_worktree_diff, merge_worktree_into_main,
    prune_orphan_worktrees as wt_prune_orphans, remove_worktree,
};
use chrono::Utc;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use zenko_core::{
    AgentDeliveryAction, AgentEvent, AgentMailMessage, AgentMessageOptions, EventKind, EventRole,
    HookEvent, JobSupervisor, NOTE_KIND_SUBAGENT_REPORT, ORCH_STAGE_SUBAGENT, PermissionMode,
    Result, SUBAGENT_CANCEL_GRACE_SECS, SUBAGENT_FORK_MAX_CHARS, SUBAGENT_FORK_RECENT_EVENTS,
    SUBAGENT_FORK_TOOL_PREVIEW_CHARS, SUBAGENT_FORK_TURN_PREVIEW_CHARS, SUBAGENT_LIST_DEFAULT_LIMIT,
    SUBAGENT_MAIL_REPORT_PREVIEW_CHARS, SUBAGENT_MAX_DURATION_SECS, SUBAGENT_ROLE_REGISTRY,
    SUBAGENT_SCHEMA_MAX_DEPTH, SUBAGENT_SUMMARY_MAX_CHARS, SUBAGENT_TIMEOUT_HARD_GRACE_SECS,
    SUBAGENT_TITLE_PREVIEW_CHARS, SendMode, SubagentModelTier, SubagentRecord,
    SubagentRoleDescriptor, SubagentSpawnRequest, SubagentStatus, SubagentWorkspaceDefault,
    SubagentWorkspaceMode, WorktreeDiffReport, WorktreePruneReport, ZenkoConfig, ZenkoError,
    compact_preview, find_subagent_role, is_operator_session,
};
use zenko_memory::SqliteStore;

struct ActiveSubagentHandle {
    parent_session_id: String,
    cancel: CancellationToken,
    status_rx: watch::Receiver<SubagentStatus>,
}

#[derive(Default)]
struct SupervisorState {
    active: HashMap<String, ActiveSubagentHandle>,
    reserved: usize,
}

/// RAII concurrency slot acquired atomically BEFORE any `.await` (eliminates TOCTOU fan-out races).
struct SlotReservation {
    state: Arc<Mutex<SupervisorState>>,
    held: bool,
}

impl SlotReservation {
    fn try_acquire(state: &Arc<Mutex<SupervisorState>>, max_concurrent: usize) -> Result<Self> {
        let mut guard = state
            .lock()
            .map_err(|_| ZenkoError::Tool("subagent supervisor lock poisoned".into()))?;
        let in_flight = guard.active.len().saturating_add(guard.reserved);
        if in_flight >= max_concurrent {
            return Err(ZenkoError::PolicyDenied(format!(
                "Maximum concurrent subagents ({max_concurrent}) already running. Wait for or kill an active subagent first."
            )));
        }
        guard.reserved = guard.reserved.saturating_add(1);
        Ok(Self {
            state: Arc::clone(state),
            held: true,
        })
    }

    /// Atomically converts the pre-spawn reservation into an active subagent handle.
    fn activate(mut self, child_id: String, handle: ActiveSubagentHandle) {
        if let Ok(mut guard) = self.state.lock() {
            guard.reserved = guard.reserved.saturating_sub(1);
            guard.active.insert(child_id, handle);
        }
        self.held = false;
    }
}

impl Drop for SlotReservation {
    fn drop(&mut self) {
        if self.held {
            if let Ok(mut guard) = self.state.lock() {
                guard.reserved = guard.reserved.saturating_sub(1);
            }
            self.held = false;
        }
    }
}

#[derive(Default)]
pub struct SubagentSupervisor {
    state: Arc<Mutex<SupervisorState>>,
    /// Keeps completed worktree handles so parent/user can inspect diff, merge, or discard after completion.
    worktrees: Mutex<HashMap<String, WorktreeHandle>>,
}

impl SubagentSupervisor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active_count(&self) -> usize {
        self.state
            .lock()
            .map(|s| s.active.len().saturating_add(s.reserved))
            .unwrap_or(0)
    }

    pub fn active_count_for_parent(&self, parent_session_id: &str) -> usize {
        self.state
            .lock()
            .map(|s| {
                s.active
                    .values()
                    .filter(|h| h.parent_session_id == parent_session_id)
                    .count()
            })
            .unwrap_or(0)
    }

    pub fn is_active(&self, agent_id: &str) -> bool {
        self.state
            .lock()
            .map(|s| s.active.contains_key(agent_id))
            .unwrap_or(false)
    }

    pub fn worktree_handle(&self, agent_id: &str) -> Option<WorktreeHandle> {
        self.worktrees
            .lock()
            .ok()
            .and_then(|m| m.get(agent_id).cloned())
    }

    fn record_worktree(&self, agent_id: &str, handle: WorktreeHandle) {
        if let Ok(mut m) = self.worktrees.lock() {
            m.insert(agent_id.to_string(), handle);
        }
    }

    fn forget_worktree(&self, agent_id: &str) {
        if let Ok(mut m) = self.worktrees.lock() {
            m.remove(agent_id);
        }
    }

    /// Reconstructs a `WorktreeHandle` from persisted SQLite session/subagent metadata if the
    /// daemon was restarted or the handle was evicted.
    fn resolve_worktree_handle(
        &self,
        rt: &AgentRuntime,
        agent_id: &str,
    ) -> Result<Option<WorktreeHandle>> {
        if let Some(h) = self.worktree_handle(agent_id) {
            return Ok(Some(h));
        }
        let Some(rec) = rt.store.get_subagent(agent_id)? else {
            return Ok(None);
        };
        if rec.workspace_mode != SubagentWorkspaceMode::Worktree {
            return Ok(None);
        }
        let Some(branch) = rec.worktree_branch else {
            return Ok(None);
        };
        let parent_ws = rt
            .store
            .get_session(&rec.parent_session_id)?
            .and_then(|s| s.workspace_dir)
            .map(PathBuf::from)
            .unwrap_or_else(|| rt.config().resolved_workspace_dir());
        let repo_root = zenko_memory::resolve_git_root(&parent_ws);
        Ok(Some(WorktreeHandle {
            agent_id: rec.id,
            repo_root,
            worktree_path: PathBuf::from(rec.workspace_dir),
            branch,
            base_commit: rec.worktree_base_commit.unwrap_or_default(),
            dirty_baseline: rec.worktree_dirty_baseline,
        }))
    }

    /// Spawns a scoped child agent (blocking until done or non-blocking when `run_in_background`).
    pub async fn spawn_agent(
        &self,
        rt: &Arc<AgentRuntime>,
        req: SubagentSpawnRequest,
    ) -> Result<SubagentRecord> {
        let cfg = rt.config();
        if !cfg.subagents.enabled {
            return Err(ZenkoError::PolicyDenied(
                "Subagent spawning is disabled in `[subagents]` configuration.".into(),
            ));
        }

        let Some(role) = find_subagent_role(&req.role) else {
            let valid: Vec<&str> = SUBAGENT_ROLE_REGISTRY.iter().map(|r| r.id).collect();
            return Err(ZenkoError::Tool(format!(
                "Unknown subagent role `{}`. Valid roles: {}.",
                req.role,
                valid.join(", ")
            )));
        };

        let goal = req.goal.trim().to_string();
        if goal.is_empty() {
            return Err(ZenkoError::Tool("Subagent `goal` cannot be empty.".into()));
        }

        if req.fork_context && !cfg.subagents.allow_fork_context {
            return Err(ZenkoError::PolicyDenied(
                "Subagent context forking (`fork_context: true`) is disabled in `[subagents]` configuration.".into(),
            ));
        }

        // Resolve or ensure parent session metadata for hierarchy + depth + permission inheritance
        let parent_session = match req.parent_session_id.trim().is_empty() {
            true => None,
            false => rt.store.get_session(&req.parent_session_id)?,
        };

        let (parent_sid, parent_depth, parent_ws, parent_perm) = match parent_session {
            Some(ref p) => {
                if let Some(ref parent_role_id) = p.agent_role {
                    if let Some(parent_role) = find_subagent_role(parent_role_id) {
                        if !parent_role.can_spawn_children {
                            return Err(ZenkoError::PolicyDenied(format!(
                                "Subagent role `{}` is not permitted to spawn nested child agents.",
                                parent_role.id
                            )));
                        }
                    }
                }
                let ws = p
                    .workspace_dir
                    .as_deref()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| cfg.resolved_workspace_dir());
                (p.id.clone(), p.agent_depth, ws, p.permission_mode)
            }
            None => {
                let fallback_parent = match req.parent_session_id.trim().is_empty() {
                    true => "operator".to_string(),
                    false => req.parent_session_id.trim().to_string(),
                };
                (
                    fallback_parent,
                    0u8,
                    cfg.resolved_workspace_dir(),
                    cfg.policy.mode,
                )
            }
        };

        let child_depth = parent_depth.saturating_add(1);
        let max_depth = cfg.subagents.max_depth.max(1);
        if child_depth > max_depth {
            return Err(ZenkoError::PolicyDenied(format!(
                "Subagent recursion depth limit ({max_depth}) reached (parent depth={parent_depth}). Complete the work directly instead of spawning deeper children."
            )));
        }

        // Atomically reserve a concurrency slot BEFORE any `.await` (zero TOCTOU race under parallel fan-out!)
        let max_concurrent = cfg.subagents.max_concurrent.max(1);
        let slot = SlotReservation::try_acquire(&self.state, max_concurrent)?;

        // Claude-Code-style deterministic SubagentStart hook (may veto or inject policy context)
        let hook_contexts = run_hooks_with_context(
            &cfg.hooks,
            HookEvent::SubagentStart,
            Some(role.id),
            json!({
                "parent_session_id": &parent_sid,
                "role": role.id,
                "depth": child_depth,
                "goal": &goal,
                "fork_context": req.fork_context,
                "include_uncommitted": req.include_uncommitted,
            }),
        )
        .await
        .map_err(|reason| ZenkoError::PolicyDenied(format!("Blocked by SubagentStart hook: {reason}")))?;

        let resolved_model = resolve_role_model(&cfg, role, req.model_override.as_deref());
        let child_title = format!(
            "[{}] {}",
            role.label,
            compact_preview(&goal, SUBAGENT_TITLE_PREVIEW_CHARS)
        );

        // Hermes monotonic permission ceiling: child can NEVER exceed parent privilege!
        let role_perm = match role.read_only {
            true => PermissionMode::Balanced,
            false => parent_perm,
        };
        let child_perm = PermissionMode::strictest(parent_perm, role_perm);

        // Determine workspace isolation mode (explicit worktree vs role default vs inherit)
        let default_mode = match role.default_workspace {
            SubagentWorkspaceDefault::Inherit => SubagentWorkspaceMode::Inherit,
            SubagentWorkspaceDefault::Worktree => SubagentWorkspaceMode::Worktree,
        };
        let requested_mode = req.workspace_mode.unwrap_or(default_mode);
        let explicit_worktree = req.workspace_mode == Some(SubagentWorkspaceMode::Worktree);

        // Create child session first so we have a durable ULID for worktree naming & SQLite FK
        let child_session = rt.store.create_child_session(
            &parent_sid,
            role.id,
            child_depth,
            &child_title,
            &parent_ws.display().to_string(),
            None,
            None,
            resolved_model.as_deref(),
            child_perm,
        )?;
        let child_id = child_session.id.clone();

        let data_dir = cfg.resolved_data_dir();
        let (effective_mode, effective_ws, worktree_handle) =
            match (requested_mode, cfg.subagents.worktrees_enabled) {
                (SubagentWorkspaceMode::Worktree, true) => {
                    match create_worktree(&data_dir, &parent_ws, &child_id, req.include_uncommitted)
                        .await
                    {
                        Ok(wt) => {
                            let wt_path_str = wt.worktree_path.display().to_string();
                            let _ = rt.store.set_session_worktree(
                                &child_id,
                                &wt_path_str,
                                &wt_path_str,
                                &wt.branch,
                            );
                            self.record_worktree(&child_id, wt.clone());
                            (
                                SubagentWorkspaceMode::Worktree,
                                wt.worktree_path.clone(),
                                Some(wt),
                            )
                        }
                        Err(e) if explicit_worktree => {
                            let _ = rt.store.delete_session(&child_id);
                            return Err(e);
                        }
                        Err(_) => (SubagentWorkspaceMode::Inherit, parent_ws.clone(), None),
                    }
                }
                (SubagentWorkspaceMode::Worktree, false) if explicit_worktree => {
                    let _ = rt.store.delete_session(&child_id);
                    return Err(ZenkoError::PolicyDenied(
                        "Git worktree isolation is disabled in `[subagents]` configuration.".into(),
                    ));
                }
                _ => (SubagentWorkspaceMode::Inherit, parent_ws.clone(), None),
            };

        let now = Utc::now();
        let initial_record = SubagentRecord {
            id: child_id.clone(),
            parent_session_id: parent_sid.clone(),
            role: role.id.to_string(),
            depth: child_depth,
            goal: goal.clone(),
            status: SubagentStatus::Running,
            workspace_mode: effective_mode,
            workspace_dir: effective_ws.display().to_string(),
            worktree_branch: worktree_handle.as_ref().map(|w| w.branch.clone()),
            worktree_base_commit: worktree_handle.as_ref().map(|w| w.base_commit.clone()),
            worktree_dirty_baseline: worktree_handle.as_ref().map_or(false, |w| w.dirty_baseline),
            model: resolved_model.clone(),
            summary: None,
            output_schema: req.output_schema.clone(),
            structured_output: None,
            diff_stat: None,
            changed_files: Vec::new(),
            tokens_used: 0,
            cost_usd: 0.0,
            tree_tokens: 0,
            tree_cost_usd: 0.0,
            descendant_count: 0,
            resume_count: 0,
            started_at: now,
            finished_at: None,
        };
        rt.store.upsert_subagent(&initial_record)?;
        rt.emit(AgentEvent::SubagentSpawned {
            agent: initial_record.clone(),
        });
        rt.emit(AgentEvent::OrchestratorUpdated {
            session_id: parent_sid.clone(),
            stage: ORCH_STAGE_SUBAGENT.into(),
            tier: zenko_core::ExecutionTier::Orchestrated,
            detail: format!(
                "Spawned `{}` subagent {} ({}, bg={}, fork={})",
                role.id,
                &child_id[..child_id.len().min(10)],
                effective_mode.as_str(),
                req.run_in_background,
                req.fork_context
            ),
            model: resolved_model.clone(),
        });

        // Compose focused child prompt (strict isolation by default, or bounded parent fork when requested)
        let mut child_prompt = String::new();
        child_prompt.push_str(&format!("Assigned Goal:\n{goal}"));
        if let Some(ctx_notes) = req.context.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            child_prompt.push_str(&format!("\n\nFocused Parent Handoff Context:\n{ctx_notes}"));
        }
        if req.fork_context {
            if let Some(fork_digest) = build_forked_parent_context(&rt.store, &parent_sid) {
                child_prompt.push_str(&format!(
                    "\n\n<forked_parent_context>\n{fork_digest}\n</forked_parent_context>"
                ));
            }
        }
        if !hook_contexts.is_empty() {
            child_prompt.push_str(&format!(
                "\n\n<subagent_hook_context>\n{}\n</subagent_hook_context>",
                hook_contexts.join("\n")
            ));
        }
        if let Some(ref wt) = worktree_handle {
            let dirty_note = match wt.dirty_baseline {
                true => "\n- Dirty Baseline: uncommitted parent workspace edits were snapshotted into this worktree",
                false => "",
            };
            child_prompt.push_str(&format!(
                "\n\nIsolated Git Worktree:\n- Path: {}\n- Branch: {}{dirty_note}\nMake edits directly inside this worktree; the parent repository stays clean until merged.",
                wt.worktree_path.display(),
                wt.branch
            ));
        }

        let max_duration_secs =
            clamp_subagent_duration(cfg.subagents.max_duration_secs, req.max_duration_secs);
        let max_cost_usd = clamp_subagent_cost(cfg.subagents.max_cost_usd, req.max_cost_usd);

        let status_rx = self.launch_child_task(
            rt,
            slot,
            initial_record.clone(),
            role,
            child_prompt,
            effective_ws,
            resolved_model,
            child_perm,
            req.output_schema.clone(),
            worktree_handle,
            req.run_in_background,
            max_duration_secs,
            max_cost_usd,
        );

        match req.run_in_background {
            true => Ok(initial_record),
            false => {
                let mut rx = status_rx;
                let _ = rx.wait_for(|s| s.is_terminal()).await;
                Ok(rt
                    .store
                    .get_subagent(&child_id)?
                    .unwrap_or(initial_record))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn launch_child_task(
        &self,
        rt: &Arc<AgentRuntime>,
        slot: SlotReservation,
        mut record_bg: SubagentRecord,
        role: &'static SubagentRoleDescriptor,
        child_prompt: String,
        effective_ws: PathBuf,
        resolved_model: Option<String>,
        child_perm: PermissionMode,
        output_schema: Option<Value>,
        wt_bg: Option<WorktreeHandle>,
        notify_parent_mailbox: bool,
        max_duration_secs: Option<u64>,
        max_cost_usd: Option<f64>,
    ) -> watch::Receiver<SubagentStatus> {
        let child_id_bg = record_bg.id.clone();
        let parent_sid_bg = record_bg.parent_session_id.clone();
        let child_cancel = CancellationToken::new();
        let (status_tx, status_rx) = watch::channel(SubagentStatus::Running);
        slot.activate(
            child_id_bg.clone(),
            ActiveSubagentHandle {
                parent_session_id: parent_sid_bg.clone(),
                cancel: child_cancel.clone(),
                status_rx: status_rx.clone(),
            },
        );

        let rt_bg = rt.clone();
        let cfg_bg = rt.config();
        let auto_cleanup_wt = cfg_bg.subagents.auto_cleanup_worktrees;
        let data_dir_bg = cfg_bg.resolved_data_dir();

        tokio::spawn(async move {
            let turn_fut = rt_bg.run_turn_with_options(
                &child_id_bg,
                &child_prompt,
                TurnOptions {
                    parent_event_override: None,
                    workspace_override: Some(effective_ws),
                    model_override: resolved_model,
                    permission_override: Some(child_perm),
                    headless_no_wait: true,
                    send_mode: SendMode::Interrupt,
                    role_id: Some(role.id.to_string()),
                    max_iterations_override: Some(role.max_iterations),
                    output_schema: output_schema.clone(),
                    external_cancel: Some(child_cancel.clone()),
                    max_cost_usd_override: max_cost_usd,
                },
            );

            // Cooperative non-dropping wall-clock timeout: signals `child_cancel` so `run_turn_with_options`
            // runs its clean session teardown (resetting `sessions.status -> idle` & clearing `active_cancels`)
            // rather than dropping the future mid-await! Secondary hard cap applies only if cleanup stalls.
            let timed_out = Arc::new(AtomicBool::new(false));
            let timer_task = max_duration_secs.map(|secs| {
                let flag = Arc::clone(&timed_out);
                let tok = child_cancel.clone();
                let rt_t = rt_bg.clone();
                let sid_t = child_id_bg.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(secs)).await;
                    flag.store(true, Ordering::SeqCst);
                    tok.cancel();
                    let _ = rt_t.cancel_session(&sid_t);
                })
            });

            let turn_res = match max_duration_secs {
                Some(secs) => {
                    let hard_cap = Duration::from_secs(
                        secs.saturating_add(SUBAGENT_TIMEOUT_HARD_GRACE_SECS),
                    );
                    match tokio::time::timeout(hard_cap, turn_fut).await {
                        Ok(res) => res,
                        Err(_) => {
                            timed_out.store(true, Ordering::SeqCst);
                            child_cancel.cancel();
                            let _ = rt_bg.cancel_session(&child_id_bg);
                            Err(ZenkoError::Cancelled)
                        }
                    }
                }
                None => turn_fut.await,
            };
            if let Some(t) = timer_task {
                t.abort();
            }

            // Hermes process ownership: clean up any shell background jobs owned by this child
            let _ = rt_bg.jobs.stop_session_jobs(&child_id_bg).await;

            match (turn_res, timed_out.load(Ordering::SeqCst)) {
                (Ok(res), _) => {
                    record_bg.status = SubagentStatus::Completed;
                    record_bg.model = Some(res.model);
                    let turn_tokens = res
                        .usage
                        .prompt_tokens
                        .saturating_add(res.usage.completion_tokens);
                    record_bg.tokens_used = record_bg.tokens_used.saturating_add(turn_tokens);
                    record_bg.cost_usd += res.usage.cost_usd;
                    if let Some(ref schema) = output_schema {
                        record_bg.structured_output =
                            extract_json_for_schema(&res.reply, schema);
                    }
                    record_bg.summary = Some(distill_summary(&res.reply, SUBAGENT_SUMMARY_MAX_CHARS));
                }
                (Err(ZenkoError::Cancelled), true) => {
                    let secs = max_duration_secs.unwrap_or(0);
                    record_bg.status = SubagentStatus::Failed;
                    record_bg.summary = Some(format!(
                        "Subagent exceeded wall-clock timeout ({secs}s)."
                    ));
                }
                (Err(ZenkoError::Cancelled), false) => {
                    record_bg.status = SubagentStatus::Killed;
                    record_bg.summary = Some("Subagent cancelled.".into());
                }
                (Err(e), _) => {
                    record_bg.status = SubagentStatus::Failed;
                    record_bg.summary = Some(format!("Subagent failed: {e}"));
                }
            }

            if let Some(ref wt) = wt_bg {
                if let Ok(diff_rep) = inspect_worktree_diff(wt).await {
                    record_bg.changed_files = diff_rep.changed_files.clone();
                    if !diff_rep.diff_stat.is_empty() {
                        record_bg.diff_stat = Some(diff_rep.diff_stat.clone());
                    }
                    if !diff_rep.has_changes
                        && auto_cleanup_wt
                        && record_bg.status == SubagentStatus::Completed
                    {
                        let _ = remove_worktree(&data_dir_bg, wt).await;
                        rt_bg.subagents.forget_worktree(&child_id_bg);
                        record_bg.worktree_branch = None;
                        record_bg.workspace_mode = SubagentWorkspaceMode::Inherit;
                    }
                }
            }

            record_bg.finished_at = Some(Utc::now());
            let _ = rt_bg.store.upsert_subagent(&record_bg);
            if let Ok(mut st) = rt_bg.subagents.state.lock() {
                st.active.remove(&child_id_bg);
            }
            let _ = status_tx.send_replace(record_bg.status);

            let hydrated = rt_bg
                .store
                .get_subagent(&child_id_bg)
                .ok()
                .flatten()
                .unwrap_or_else(|| record_bg.clone());

            rt_bg.emit(AgentEvent::SubagentCompleted {
                agent: hydrated.clone(),
            });

            // Claude-Code-style deterministic SubagentStop lifecycle hook
            let hooks_now = rt_bg.config().hooks;
            let _ = run_hooks(
                &hooks_now,
                HookEvent::SubagentStop,
                Some(&hydrated.role),
                json!({
                    "agent_id": &hydrated.id,
                    "parent_session_id": &hydrated.parent_session_id,
                    "role": &hydrated.role,
                    "status": hydrated.status.as_str(),
                    "tokens_used": hydrated.tokens_used,
                    "cost_usd": hydrated.cost_usd,
                    "resume_count": hydrated.resume_count,
                    "worktree_branch": &hydrated.worktree_branch,
                    "changed_files": &hydrated.changed_files,
                }),
            )
            .await;

            // Exhaustive dispatch for parent reporting (zero if/else-if chains)
            match (notify_parent_mailbox, hydrated.status) {
                (true, SubagentStatus::Completed | SubagentStatus::Failed) => {
                    let diff_note = match hydrated.changed_files.is_empty() {
                        true => String::new(),
                        false => format!(
                            "\nWorktree branch `{}` changed {} file(s): {}",
                            hydrated.worktree_branch.as_deref().unwrap_or(""),
                            hydrated.changed_files.len(),
                            hydrated.changed_files.join(", ")
                        ),
                    };
                    let summary_body = hydrated.summary.as_deref().unwrap_or("(no summary)");
                    let mail_body = format!(
                        "Subagent `{}` (role={}, status={}) finished.{diff_note}\nSummary:\n{}",
                        hydrated.id,
                        hydrated.role,
                        hydrated.status.as_str(),
                        compact_preview(summary_body, SUBAGENT_MAIL_REPORT_PREVIEW_CHARS)
                    );
                    if let Ok(mail) = rt_bg
                        .store
                        .insert_agent_message(&child_id_bg, &parent_sid_bg, &mail_body)
                    {
                        rt_bg.emit(AgentEvent::SubagentMessage { message: mail });
                    }
                    rt_bg.notify_subagent_finished(&parent_sid_bg, &hydrated);
                }
                (false, SubagentStatus::Completed) => {
                    if let Ok(Some(parent)) = rt_bg.store.get_session(&parent_sid_bg) {
                        let _ = rt_bg.store.append_event(
                            &parent_sid_bg,
                            parent.head_event_id.as_deref(),
                            EventRole::System,
                            EventKind::SystemNote,
                            json!({
                                "note_kind": NOTE_KIND_SUBAGENT_REPORT,
                                "agent_id": &hydrated.id,
                                "role": &hydrated.role,
                                "status": hydrated.status.as_str(),
                                "worktree_branch": &hydrated.worktree_branch,
                            }),
                            &zenko_core::UsageStats::default(),
                        );
                    }
                }
                _ => {}
            }
        });

        status_rx
    }

    /// Resumes an idle/completed/failed/killed subagent in its exact SQLite trajectory & worktree.
    async fn resume_subagent(
        &self,
        rt: &Arc<AgentRuntime>,
        mut rec: SubagentRecord,
        from_session_id: &str,
        body: &str,
    ) -> Result<()> {
        let cfg = rt.config();
        let role = find_subagent_role(&rec.role).ok_or_else(|| {
            ZenkoError::Tool(format!(
                "Cannot resume subagent `{}`: unknown role `{}`.",
                rec.id, rec.role
            ))
        })?;
        let max_concurrent = cfg.subagents.max_concurrent.max(1);
        let slot = SlotReservation::try_acquire(&self.state, max_concurrent)?;

        let parent_perm = rt
            .store
            .get_session(&rec.parent_session_id)?
            .map(|p| p.permission_mode)
            .unwrap_or(cfg.policy.mode);
        let role_perm = match role.read_only {
            true => PermissionMode::Balanced,
            false => parent_perm,
        };
        let child_perm = PermissionMode::strictest(parent_perm, role_perm);

        let wt_handle = self
            .resolve_worktree_handle(rt, &rec.id)?
            .filter(|w| w.worktree_path.exists());
        let effective_ws = match wt_handle {
            Some(ref wt) => wt.worktree_path.clone(),
            None => {
                let saved = PathBuf::from(&rec.workspace_dir);
                match saved.exists() {
                    true => saved,
                    false => cfg.resolved_workspace_dir(),
                }
            }
        };

        rec.status = SubagentStatus::Running;
        rec.finished_at = None;
        rec.resume_count = rec.resume_count.saturating_add(1);
        rt.store.upsert_subagent(&rec)?;
        rt.emit(AgentEvent::SubagentSpawned { agent: rec.clone() });

        let resume_prompt = format!("[Follow-up Message from {from_session_id}]\n{body}");
        let max_duration_secs = clamp_subagent_duration(cfg.subagents.max_duration_secs, None);
        let max_cost_usd = clamp_subagent_cost(cfg.subagents.max_cost_usd, None);

        let persisted_schema = rec.output_schema.clone();
        let _ = self.launch_child_task(
            rt,
            slot,
            rec.clone(),
            role,
            resume_prompt,
            effective_ws,
            rec.model.clone(),
            child_perm,
            persisted_schema,
            wt_handle,
            true,
            max_duration_secs,
            max_cost_usd,
        );
        Ok(())
    }

    /// Zero-CPU event-driven wait across one or more child subagents (`wait_all` or first-to-finish).
    pub async fn wait_agents(
        &self,
        rt: &AgentRuntime,
        parent_session_id: &str,
        agent_ids: &[String],
        wait_all: bool,
        timeout: Duration,
    ) -> Result<Vec<SubagentRecord>> {
        let target_ids: Vec<String> = match agent_ids.is_empty() {
            true => rt
                .store
                .list_subagents(Some(parent_session_id), SUBAGENT_LIST_DEFAULT_LIMIT)?
                .into_iter()
                .map(|r| r.id)
                .collect(),
            false => agent_ids.to_vec(),
        };

        if target_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut pending_rxs: Vec<watch::Receiver<SubagentStatus>> = Vec::new();
        if let Ok(st) = self.state.lock() {
            for id in &target_ids {
                if let Some(h) = st.active.get(id) {
                    if !h.status_rx.borrow().is_terminal() {
                        pending_rxs.push(h.status_rx.clone());
                    }
                }
            }
        }

        if !pending_rxs.is_empty() {
            match wait_all {
                true => {
                    let all_fut = futures_util::future::join_all(pending_rxs.into_iter().map(
                        |mut rx| async move {
                            let _ = rx.wait_for(|s| s.is_terminal()).await;
                        },
                    ));
                    let _ = tokio::time::timeout(timeout, all_fut).await;
                }
                false => {
                    let any_futs: Vec<_> = pending_rxs
                        .into_iter()
                        .map(|mut rx| {
                            Box::pin(async move {
                                let _ = rx.wait_for(|s| s.is_terminal()).await;
                            })
                        })
                        .collect();
                    let _ = tokio::time::timeout(
                        timeout,
                        futures_util::future::select_all(any_futs),
                    )
                    .await;
                }
            }
        }

        // Prevent spurious delayed idle-wakeup turns for children the parent already collected via `agent_wait`
        if !parent_session_id.trim().is_empty() {
            rt.clear_subagent_wakeup_notices(parent_session_id, &target_ids);
        }

        let mut out = Vec::with_capacity(target_ids.len());
        for id in target_ids {
            if let Some(rec) = rt.store.get_subagent(&id)? {
                out.push(rec);
            }
        }
        Ok(out)
    }

    /// Sends a nuclear-family-enforced mailbox message between agents (or from operator CLI/WebUI).
    /// Supports Codex/Prime-style `resume` (reloads idle/completed child keeping its SQLite trajectory &
    /// worktree) and `interrupt` (cancels active step and restarts with new guidance).
    pub async fn send_message(
        &self,
        rt: &Arc<AgentRuntime>,
        from_session_id: &str,
        to_session_id: &str,
        message: &str,
        opts: AgentMessageOptions,
    ) -> Result<AgentMailMessage> {
        let body = message.trim();
        if body.is_empty() {
            return Err(ZenkoError::Tool("Message body cannot be empty.".into()));
        }
        if rt.store.get_session(to_session_id)?.is_none() {
            return Err(ZenkoError::NotFound(format!(
                "Target agent/session `{to_session_id}` not found."
            )));
        }

        if !is_operator_session(from_session_id)
            && !is_nuclear_family(&rt.store, from_session_id, to_session_id)?
        {
            return Err(ZenkoError::PolicyDenied(format!(
                "Nuclear-family messaging violation: session `{from_session_id}` may only message its direct parent, direct children, or same-parent siblings (blocked target `{to_session_id}`)."
            )));
        }

        let mut mail = rt
            .store
            .insert_agent_message(from_session_id, to_session_id, body)?;

        let is_running = self.is_active(to_session_id) || rt.is_session_running(to_session_id);
        let subagent_opt = rt.store.get_subagent(to_session_id)?;

        let action = match (is_running, opts.interrupt, opts.resume, subagent_opt) {
            (true, true, _, Some(_)) => {
                // Codex `send_input(interrupt=true)`: halt active step cleanly, then restart with new guidance
                let rx_opt = self.state.lock().ok().and_then(|st| {
                    st.active.get(to_session_id).map(|h| {
                        h.cancel.cancel();
                        h.status_rx.clone()
                    })
                });
                let _ = rt.cancel_session(to_session_id);
                if let Some(mut rx) = rx_opt {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(SUBAGENT_CANCEL_GRACE_SECS),
                        rx.wait_for(|s| s.is_terminal()),
                    )
                    .await;
                }
                match rt.store.get_subagent(to_session_id)? {
                    Some(rec) => {
                        let _ = rt.store.mark_agent_message_read(&mail.id);
                        self.resume_subagent(rt, rec, from_session_id, body).await?;
                        AgentDeliveryAction::Interrupted
                    }
                    None => AgentDeliveryAction::Queued,
                }
            }
            (true, _, _, _) => {
                let _ = rt.steer_session(
                    to_session_id,
                    &format!("[Agent Message from {from_session_id}] {body}"),
                );
                AgentDeliveryAction::Steered
            }
            (false, _, true, Some(rec)) => {
                // Prime / Claude / Codex seamless child resume preserving SQLite history, schema & worktree
                let _ = rt.store.mark_agent_message_read(&mail.id);
                self.resume_subagent(rt, rec, from_session_id, body).await?;
                AgentDeliveryAction::Resumed
            }
            _ => AgentDeliveryAction::Queued,
        };

        mail.delivery_action = Some(action);
        rt.emit(AgentEvent::SubagentMessage {
            message: mail.clone(),
        });

        Ok(mail)
    }

    /// Cascade-cancels a subagent and all its descendants, stops owned shell jobs, and optionally
    /// removes its isolated git worktree & branch (syncing SQLite & invoking `SubagentStop` hooks).
    pub async fn kill_agent(
        &self,
        rt: &AgentRuntime,
        caller_session_id: &str,
        agent_id: &str,
        remove_wt: bool,
    ) -> Result<Option<SubagentRecord>> {
        let Some(target) = rt.store.get_subagent(agent_id)? else {
            return Ok(None);
        };
        ensure_subtree_owner(&rt.store, caller_session_id, agent_id, "kill")?;

        let mut subtree = vec![agent_id.to_string()];
        subtree.extend(rt.store.list_descendant_session_ids(agent_id)?);

        let mut waits: Vec<watch::Receiver<SubagentStatus>> = Vec::new();
        if let Ok(st) = self.state.lock() {
            for sid in &subtree {
                if let Some(h) = st.active.get(sid) {
                    h.cancel.cancel();
                    waits.push(h.status_rx.clone());
                }
            }
        }

        for sid in &subtree {
            let _ = rt.cancel_session(sid);
            let _ = rt.jobs.stop_session_jobs(sid).await;
        }

        for mut rx in waits {
            let _ = tokio::time::timeout(
                Duration::from_secs(SUBAGENT_CANCEL_GRACE_SECS),
                rx.wait_for(|s| s.is_terminal()),
            )
            .await;
        }

        let cfg_now = rt.config();
        let data_dir = cfg_now.resolved_data_dir();
        let now = Utc::now();
        for sid in &subtree {
            let mut wt_removed = false;
            if remove_wt {
                if let Ok(Some(wt)) = self.resolve_worktree_handle(rt, sid) {
                    if let Err(e) = remove_worktree(&data_dir, &wt).await {
                        warn!(agent_id = %sid, error = %e, "failed cleaning up subagent worktree");
                    }
                    self.forget_worktree(sid);
                    wt_removed = true;
                }
            }
            if let Ok(Some(mut rec)) = rt.store.get_subagent(sid) {
                let transitioned_to_killed = !rec.status.is_terminal();
                if transitioned_to_killed {
                    rec.status = SubagentStatus::Killed;
                    rec.finished_at = Some(now);
                    if rec.summary.is_none() {
                        rec.summary = Some("Subagent killed.".into());
                    }
                }
                if wt_removed {
                    rec.worktree_branch = None;
                    rec.workspace_mode = SubagentWorkspaceMode::Inherit;
                    rec.changed_files.clear();
                    rec.diff_stat = None;
                }
                if transitioned_to_killed || wt_removed {
                    let _ = rt.store.upsert_subagent(&rec);
                    rt.emit(AgentEvent::SubagentCompleted { agent: rec.clone() });
                }
                if transitioned_to_killed {
                    let _ = run_hooks(
                        &cfg_now.hooks,
                        HookEvent::SubagentStop,
                        Some(&rec.role),
                        json!({
                            "agent_id": &rec.id,
                            "parent_session_id": &rec.parent_session_id,
                            "role": &rec.role,
                            "status": rec.status.as_str(),
                            "tokens_used": rec.tokens_used,
                            "cost_usd": rec.cost_usd,
                            "resume_count": rec.resume_count,
                            "worktree_branch": &rec.worktree_branch,
                            "changed_files": &rec.changed_files,
                        }),
                    )
                    .await;
                }
            }
            if let Ok(mut st) = self.state.lock() {
                st.active.remove(sid);
            }
        }

        Ok(rt.store.get_subagent(&target.id)?)
    }

    /// Cancels all active subagents cleanly during graceful daemon shutdown.
    pub async fn kill_all(&self, rt: &AgentRuntime) {
        let active_ids: Vec<(String, CancellationToken, watch::Receiver<SubagentStatus>)> = self
            .state
            .lock()
            .map(|st| {
                st.active
                    .iter()
                    .map(|(id, h)| (id.clone(), h.cancel.clone(), h.status_rx.clone()))
                    .collect()
            })
            .unwrap_or_default();

        for (sid, tok, _) in &active_ids {
            tok.cancel();
            let _ = rt.cancel_session(sid);
            let _ = rt.jobs.stop_session_jobs(sid).await;
        }
        for (_, _, mut rx) in active_ids {
            let _ = tokio::time::timeout(
                Duration::from_secs(SUBAGENT_CANCEL_GRACE_SECS),
                rx.wait_for(|s| s.is_terminal()),
            )
            .await;
        }
    }

    /// Cascade-cancels active descendant subagents when a parent session is explicitly cancelled.
    pub fn cancel_descendants(&self, store: &SqliteStore, parent_session_id: &str) {
        let descendants = store
            .list_descendant_session_ids(parent_session_id)
            .unwrap_or_default();
        if descendants.is_empty() {
            return;
        }
        if let Ok(st) = self.state.lock() {
            for sid in descendants {
                if let Some(h) = st.active.get(&sid) {
                    h.cancel.cancel();
                }
            }
        }
    }

    pub async fn inspect_worktree_diff(
        &self,
        rt: &AgentRuntime,
        caller_session_id: &str,
        agent_id: &str,
    ) -> Result<Option<WorktreeDiffReport>> {
        ensure_subtree_owner(&rt.store, caller_session_id, agent_id, "inspect worktree of")?;
        let Some(wt) = self.resolve_worktree_handle(rt, agent_id)? else {
            return Ok(None);
        };
        Ok(Some(inspect_worktree_diff(&wt).await?))
    }

    pub async fn merge_worktree(
        &self,
        rt: &AgentRuntime,
        caller_session_id: &str,
        agent_id: &str,
        cleanup: bool,
    ) -> Result<WorktreeDiffReport> {
        let Some(mut rec) = rt.store.get_subagent(agent_id)? else {
            return Err(ZenkoError::NotFound(format!("subagent `{agent_id}` not found")));
        };
        ensure_subtree_owner(&rt.store, caller_session_id, agent_id, "merge worktree of")?;
        if !rec.status.is_terminal() {
            return Err(ZenkoError::PolicyDenied(format!(
                "Cannot merge subagent `{agent_id}` while it is still `{}`. Wait for completion or kill it first.",
                rec.status.as_str()
            )));
        }
        let wt = self.resolve_worktree_handle(rt, agent_id)?.ok_or_else(|| {
            ZenkoError::Tool(format!(
                "Subagent `{agent_id}` does not have an isolated git worktree."
            ))
        })?;
        let data_dir = rt.config().resolved_data_dir();
        let report = merge_worktree_into_main(&data_dir, &wt, cleanup).await?;
        if report.cleaned_up {
            self.forget_worktree(agent_id);
            rec.worktree_branch = None;
            rec.workspace_mode = SubagentWorkspaceMode::Inherit;
            let _ = rt.store.upsert_subagent(&rec);
            rt.emit(AgentEvent::SubagentCompleted { agent: rec });
        }
        Ok(report)
    }

    /// Discards a completed/terminated subagent's worktree & temporary branch without merging.
    pub async fn discard_worktree(
        &self,
        rt: &AgentRuntime,
        caller_session_id: &str,
        agent_id: &str,
    ) -> Result<WorktreeDiffReport> {
        let Some(mut rec) = rt.store.get_subagent(agent_id)? else {
            return Err(ZenkoError::NotFound(format!("subagent `{agent_id}` not found")));
        };
        ensure_subtree_owner(&rt.store, caller_session_id, agent_id, "discard worktree of")?;
        if !rec.status.is_terminal() {
            return Err(ZenkoError::PolicyDenied(format!(
                "Cannot discard worktree while subagent `{agent_id}` is still `{}`. Wait or kill it first.",
                rec.status.as_str()
            )));
        }
        let wt = self.resolve_worktree_handle(rt, agent_id)?.ok_or_else(|| {
            ZenkoError::Tool(format!(
                "Subagent `{agent_id}` does not have an isolated git worktree."
            ))
        })?;
        let mut report = inspect_worktree_diff(&wt).await?;
        let data_dir = rt.config().resolved_data_dir();
        remove_worktree(&data_dir, &wt).await?;
        self.forget_worktree(agent_id);
        report.merged = false;
        report.cleaned_up = true;

        rec.worktree_branch = None;
        rec.workspace_mode = SubagentWorkspaceMode::Inherit;
        rec.changed_files.clear();
        rec.diff_stat = None;
        let _ = rt.store.upsert_subagent(&rec);
        rt.emit(AgentEvent::SubagentCompleted { agent: rec });
        Ok(report)
    }

    /// Safely prunes crash-orphaned worktree directories under `<data_dir>/worktrees/`.
    pub async fn prune_orphan_worktrees(&self, rt: &AgentRuntime) -> Result<WorktreePruneReport> {
        let cfg = rt.config();
        let preserved = rt.store.list_preserved_worktree_ids()?;
        wt_prune_orphans(&cfg.resolved_data_dir(), &cfg.resolved_workspace_dir(), &preserved).await
    }
}

/// Unified subtree ownership guard shared by `kill_agent`, `inspect_worktree_diff`,
/// `merge_worktree`, and `discard_worktree` (single source of truth; zero duplicated checks).
pub fn ensure_subtree_owner(
    store: &SqliteStore,
    caller_session_id: &str,
    agent_id: &str,
    action_label: &str,
) -> Result<()> {
    if is_operator_session(caller_session_id) {
        return Ok(());
    }
    let allowed = store.list_descendant_session_ids(caller_session_id)?;
    match allowed.iter().any(|id| id == agent_id) {
        true => Ok(()),
        false => Err(ZenkoError::PolicyDenied(format!(
            "Session `{caller_session_id}` cannot {action_label} subagent `{agent_id}` outside its own subtree."
        ))),
    }
}

/// Nuclear-family topology check (Prime Agent):
/// Allows `A <-> B` ONLY when `A` is `B`'s direct parent, `B` is `A`'s direct parent,
/// or `A` and `B` share the exact same non-empty `parent_session_id` (siblings).
pub fn is_nuclear_family(store: &SqliteStore, from_sid: &str, to_sid: &str) -> Result<bool> {
    if from_sid == to_sid {
        return Ok(false);
    }
    let (Some(from_s), Some(to_s)) = (store.get_session(from_sid)?, store.get_session(to_sid)?)
    else {
        return Ok(false);
    };
    let parent_to_child = to_s.parent_session_id.as_deref() == Some(from_sid);
    let child_to_parent = from_s.parent_session_id.as_deref() == Some(to_sid);
    let siblings = match (
        from_s.parent_session_id.as_deref(),
        to_s.parent_session_id.as_deref(),
    ) {
        (Some(p1), Some(p2)) => !p1.is_empty() && p1 == p2,
        _ => false,
    };
    Ok(parent_to_child || child_to_parent || siblings)
}

/// Dynamic model router for subagent roles (zero hardcoded model IDs).
pub fn resolve_role_model(
    cfg: &ZenkoConfig,
    role: &SubagentRoleDescriptor,
    explicit_override: Option<&str>,
) -> Option<String> {
    if let Some(m) = explicit_override.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(m.to_string());
    }
    let candidates: [&str; 3] = match role.model_tier {
        SubagentModelTier::Fast => [&cfg.provider.fast_model, &cfg.provider.lead_model, ""],
        SubagentModelTier::Plan => [&cfg.provider.plan_model, &cfg.provider.lead_model, ""],
        SubagentModelTier::Lead => [&cfg.provider.lead_model, &cfg.provider.fast_model, ""],
        SubagentModelTier::Advisor => [
            &cfg.provider.advisor_model,
            &cfg.provider.plan_model,
            &cfg.provider.lead_model,
        ],
    };
    candidates
        .into_iter()
        .map(str::trim)
        .find(|m| !m.is_empty())
        .map(String::from)
}

fn distill_summary(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    match trimmed.len() <= max_chars {
        true => trimmed.to_string(),
        false => {
            let cut = trimmed.floor_char_boundary(max_chars);
            format!("{}...\n[distilled to {} chars]", &trimmed[..cut], max_chars)
        }
    }
}

/// Monotonic wall-clock duration clamp: child overrides may tighten `[subagents].max_duration_secs`
/// but can never exceed the configured ceiling when `cfg_max > 0`.
pub fn clamp_subagent_duration(cfg_max: u64, req_opt: Option<u64>) -> Option<u64> {
    let hard = SUBAGENT_MAX_DURATION_SECS.max(1);
    match (cfg_max, req_opt) {
        (0, None) => None,
        (0, Some(r)) => Some(r.clamp(1, hard)),
        (c, None) => Some(c.clamp(1, hard)),
        (c, Some(r)) => Some(r.clamp(1, c.clamp(1, hard))),
    }
}

/// Monotonic USD spend clamp: child overrides may tighten `[subagents].max_cost_usd`
/// but can never raise it above the configured ceiling when `cfg_max > 0.0`.
pub fn clamp_subagent_cost(cfg_max: f64, req_opt: Option<f64>) -> Option<f64> {
    let cfg_pos = match cfg_max > 0.0 && cfg_max.is_finite() {
        true => Some(cfg_max),
        false => None,
    };
    let req_pos = req_opt.filter(|v| *v > 0.0 && v.is_finite());
    match (cfg_pos, req_pos) {
        (None, None) => None,
        (Some(c), None) => Some(c),
        (None, Some(r)) => Some(r),
        (Some(c), Some(r)) => Some(r.min(c)),
    }
}

/// Builds a bounded, KV-cache-safe digest of the parent session's active branch for `fork_context: true`
/// (Claude Code `context: fork` + Codex `fork_context` bounded by `SUBAGENT_FORK_MAX_CHARS`).
fn build_forked_parent_context(store: &SqliteStore, parent_sid: &str) -> Option<String> {
    let branch = store.get_active_branch(parent_sid).ok()?;
    if branch.is_empty() {
        return None;
    }
    let slice = slice_from_latest_checkpoint(&branch);
    let mut parts: Vec<String> = Vec::new();
    if let Some(ref ckpt) = slice.checkpoint {
        if !ckpt.summary.trim().is_empty() {
            parts.push(format!(
                "Parent Checkpoint Summary:\n{}",
                compact_preview(&ckpt.summary, SUBAGENT_FORK_MAX_CHARS / 2)
            ));
        }
    }

    let mut recent_lines: Vec<String> = Vec::new();
    for ev in slice.kept_events.iter().rev().take(SUBAGENT_FORK_RECENT_EVENTS) {
        let line = match (ev.role, ev.kind) {
            (EventRole::User, EventKind::Message) => ev
                .content_json
                .get("text")
                .and_then(Value::as_str)
                .map(|t| format!("User: {}", compact_preview(t, SUBAGENT_FORK_TURN_PREVIEW_CHARS))),
            (EventRole::Assistant, EventKind::Message | EventKind::ToolCall) => ev
                .content_json
                .get("text")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(|t| {
                    format!(
                        "Assistant: {}",
                        compact_preview(t, SUBAGENT_FORK_TURN_PREVIEW_CHARS)
                    )
                }),
            (EventRole::Tool, EventKind::ToolResult) => {
                let tn = ev
                    .content_json
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let body = ev
                    .content_json
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                Some(format!(
                    "Tool({tn}): {}",
                    compact_preview(body, SUBAGENT_FORK_TOOL_PREVIEW_CHARS)
                ))
            }
            _ => None,
        };
        if let Some(l) = line {
            recent_lines.push(l);
        }
    }
    recent_lines.reverse();
    if !recent_lines.is_empty() {
        parts.push(format!("Recent Parent Turns:\n{}", recent_lines.join("\n")));
    }

    match parts.is_empty() {
        true => None,
        false => Some(distill_summary(&parts.join("\n\n"), SUBAGENT_FORK_MAX_CHARS)),
    }
}

/// Formats a concise prompt instruction binding a subagent's final reply to a JSON Schema contract.
pub fn format_output_schema_instruction(schema: &Value) -> String {
    let pretty = serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string());
    format!(
        "<subagent_output_contract>\n\
         Your FINAL response MUST be strictly valid JSON matching the following JSON Schema (no surrounding prose or markdown outside the JSON value):\n\
         {pretty}\n\
         </subagent_output_contract>"
    )
}

/// Zero-regex deterministic multi-candidate JSON extractor collecting raw JSON, ```json fenced
/// blocks, and balanced top-level `{...}` or `[...]` spans in appearance order.
pub fn extract_json_candidates(text: &str) -> Vec<Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return vec![v];
    }

    let mut candidates: Vec<Value> = Vec::new();

    // 1. Check fenced markdown code blocks cleanly without regex
    let mut rest = trimmed;
    while let Some(fence_pos) = rest.find("```") {
        let after_fence = &rest[fence_pos + 3..];
        let Some(nl) = after_fence.find('\n') else {
            break;
        };
        let after_header = &after_fence[nl + 1..];
        let Some(close_pos) = after_header.find("```") else {
            break;
        };
        let candidate = after_header[..close_pos].trim();
        if let Ok(v) = serde_json::from_str::<Value>(candidate) {
            candidates.push(v);
        }
        rest = &after_header[close_pos + 3..];
    }

    // 2. Balanced JSON delimiter scanner respecting string literals & backslash escapes
    let bytes = trimmed.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let open_ch = bytes[cursor];
        let close_target = match open_ch {
            b'{' => b'}',
            b'[' => b']',
            _ => {
                cursor += 1;
                continue;
            }
        };
        let mut depth = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        let mut matched_end: Option<usize> = None;
        for (offset, &ch) in bytes[cursor..].iter().enumerate() {
            if in_string {
                match (escaped, ch) {
                    (true, _) => escaped = false,
                    (false, b'\\') => escaped = true,
                    (false, b'"') => in_string = false,
                    _ => {}
                }
                continue;
            }
            match ch {
                b'"' => in_string = true,
                c if c == open_ch => depth += 1,
                c if c == close_target => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        let slice = &trimmed[cursor..=cursor + offset];
                        if let Ok(v) = serde_json::from_str::<Value>(slice) {
                            candidates.push(v);
                            matched_end = Some(cursor + offset + 1);
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
        cursor = matched_end.unwrap_or(cursor + 1);
    }

    candidates
}

/// Convenience extractor returning the first valid JSON value found.
pub fn extract_json_payload(text: &str) -> Option<Value> {
    extract_json_candidates(text).into_iter().next()
}

/// Schema-guided JSON extractor: selects the first extracted JSON candidate that satisfies
/// `schema`, eliminating false positives when prose precedes the payload with e.g. `[0, 1]`.
/// Falls back to the last candidate when none satisfies `schema` so repair hints cite exact fields.
pub fn extract_json_for_schema(text: &str, schema: &Value) -> Option<Value> {
    let candidates = extract_json_candidates(text);
    if let Some(valid) = candidates
        .iter()
        .find(|cand| validate_json_schema(schema, cand).is_ok())
    {
        return Some(valid.clone());
    }
    candidates.into_iter().last()
}

/// Pure-Rust deterministic recursive JSON Schema validator (zero regex, bounded recursion depth,
/// supports `type`, `enum`, `const`, `anyOf`, `oneOf`, numeric/string/array/object constraints).
pub fn validate_json_schema(schema: &Value, instance: &Value) -> std::result::Result<(), Vec<String>> {
    let mut errors = Vec::new();
    validate_node(schema, instance, "$", 0, &mut errors);
    match errors.is_empty() {
        true => Ok(()),
        false => Err(errors),
    }
}

fn json_type_matches(expected: &str, val: &Value) -> bool {
    match expected {
        "object" => val.is_object(),
        "array" => val.is_array(),
        "string" => val.is_string(),
        "number" => val.is_number(),
        "integer" => val.as_i64().is_some() || val.as_u64().is_some(),
        "boolean" => val.is_boolean(),
        "null" => val.is_null(),
        _ => true,
    }
}

fn validate_node(
    schema: &Value,
    val: &Value,
    path: &str,
    depth: usize,
    errors: &mut Vec<String>,
) {
    if depth > SUBAGENT_SCHEMA_MAX_DEPTH {
        errors.push(format!(
            "{path}: exceeded max JSON Schema nesting depth ({SUBAGENT_SCHEMA_MAX_DEPTH})"
        ));
        return;
    }
    let Some(obj) = schema.as_object() else {
        return;
    };

    if let Some(Value::Array(branches)) = obj.get("anyOf") {
        let matched = branches
            .iter()
            .any(|sub| validate_branch(sub, val, depth + 1));
        if !matched {
            errors.push(format!("{path}: value did not satisfy any `anyOf` branch"));
        }
    }

    if let Some(Value::Array(branches)) = obj.get("oneOf") {
        let count = branches
            .iter()
            .filter(|sub| validate_branch(sub, val, depth + 1))
            .count();
        if count != 1 {
            errors.push(format!(
                "{path}: expected value to match exactly 1 `oneOf` branch, matched {count}"
            ));
        }
    }

    if let Some(ty) = obj.get("type") {
        let ok = match ty {
            Value::String(s) => json_type_matches(s, val),
            Value::Array(arr) => arr
                .iter()
                .filter_map(Value::as_str)
                .any(|s| json_type_matches(s, val)),
            _ => true,
        };
        if !ok {
            errors.push(format!("{path}: expected type {ty}, got {}", value_kind(val)));
            return;
        }
    }

    if let Some(Value::Array(allowed)) = obj.get("enum") {
        if !allowed.iter().any(|cand| cand == val) {
            errors.push(format!("{path}: value not in allowed enum"));
        }
    }

    if let Some(const_val) = obj.get("const") {
        if const_val != val {
            errors.push(format!("{path}: value does not match required const"));
        }
    }

    match val {
        Value::String(s) => {
            let len = s.chars().count();
            if let Some(min_l) = obj.get("minLength").and_then(Value::as_u64) {
                if (len as u64) < min_l {
                    errors.push(format!("{path}: length {len} < minLength {min_l}"));
                }
            }
            if let Some(max_l) = obj.get("maxLength").and_then(Value::as_u64) {
                if (len as u64) > max_l {
                    errors.push(format!("{path}: length {len} > maxLength {max_l}"));
                }
            }
        }
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if let Some(min_v) = obj.get("minimum").and_then(Value::as_f64) {
                    if f < min_v {
                        errors.push(format!("{path}: {f} < minimum {min_v}"));
                    }
                }
                if let Some(max_v) = obj.get("maximum").and_then(Value::as_f64) {
                    if f > max_v {
                        errors.push(format!("{path}: {f} > maximum {max_v}"));
                    }
                }
            }
        }
        Value::Array(items_val) => {
            if let Some(min_i) = obj.get("minItems").and_then(Value::as_u64) {
                if (items_val.len() as u64) < min_i {
                    errors.push(format!(
                        "{path}: array len {} < minItems {min_i}",
                        items_val.len()
                    ));
                }
            }
            if let Some(max_i) = obj.get("maxItems").and_then(Value::as_u64) {
                if (items_val.len() as u64) > max_i {
                    errors.push(format!(
                        "{path}: array len {} > maxItems {max_i}",
                        items_val.len()
                    ));
                }
            }
            if let Some(item_schema) = obj.get("items") {
                for (i, item) in items_val.iter().enumerate() {
                    validate_node(item_schema, item, &format!("{path}[{i}]"), depth + 1, errors);
                }
            }
        }
        Value::Object(map) => {
            if let Some(Value::Array(req_keys)) = obj.get("required") {
                for k in req_keys.iter().filter_map(Value::as_str) {
                    if !map.contains_key(k) {
                        errors.push(format!("{path}: missing required field `{k}`"));
                    }
                }
            }
            let props_opt = obj.get("properties").and_then(Value::as_object);
            if let Some(props) = props_opt {
                for (k, prop_schema) in props {
                    if let Some(v) = map.get(k) {
                        validate_node(prop_schema, v, &format!("{path}.{k}"), depth + 1, errors);
                    }
                }
            }
            if obj.get("additionalProperties").and_then(Value::as_bool) == Some(false) {
                for k in map.keys() {
                    let known = props_opt.map_or(false, |p| p.contains_key(k));
                    if !known {
                        errors.push(format!("{path}: unexpected additional property `{k}`"));
                    }
                }
            }
        }
        _ => {}
    }
}

fn validate_branch(schema: &Value, val: &Value, depth: usize) -> bool {
    let mut sub_errors = Vec::new();
    validate_node(schema, val, "$", depth, &mut sub_errors);
    sub_errors.is_empty()
}

fn value_kind(val: &Value) -> &'static str {
    match val {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenko_core::new_id;

    #[test]
    fn nuclear_family_topology_blocks_cross_tree_messaging() {
        let dir = std::env::temp_dir().join(format!("zenko_nf_test_{}", new_id()));
        let store = SqliteStore::open(dir.join("zenko.db")).expect("db");

        let root_a = store
            .create_session("Root A", "webui", None, None, PermissionMode::Balanced)
            .expect("root_a");
        let root_b = store
            .create_session("Root B", "webui", None, None, PermissionMode::Balanced)
            .expect("root_b");
        let child_a1 = store
            .create_child_session(&root_a.id, "explorer", 1, "A1", "/tmp", None, None, None, PermissionMode::Balanced)
            .expect("a1");
        let child_a2 = store
            .create_child_session(&root_a.id, "coder", 1, "A2", "/tmp", None, None, None, PermissionMode::Balanced)
            .expect("a2");
        let child_b1 = store
            .create_child_session(&root_b.id, "explorer", 1, "B1", "/tmp", None, None, None, PermissionMode::Balanced)
            .expect("b1");

        // Parent <-> Child allowed
        assert!(is_nuclear_family(&store, &root_a.id, &child_a1.id).unwrap());
        assert!(is_nuclear_family(&store, &child_a1.id, &root_a.id).unwrap());
        // Siblings allowed
        assert!(is_nuclear_family(&store, &child_a1.id, &child_a2.id).unwrap());
        // Cross-tree & self strictly blocked!
        assert!(!is_nuclear_family(&store, &child_a1.id, &child_b1.id).unwrap());
        assert!(!is_nuclear_family(&store, &child_a1.id, &root_b.id).unwrap());
        assert!(!is_nuclear_family(&store, &root_a.id, &root_b.id).unwrap());
        assert!(!is_nuclear_family(&store, &child_a1.id, &child_a1.id).unwrap());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn json_schema_extraction_and_recursive_validation_works() {
        let schema = json!({
            "type": "object",
            "required": ["files", "risk", "count"],
            "additionalProperties": false,
            "properties": {
                "files": { "type": "array", "minItems": 1, "items": { "type": "string", "minLength": 1 } },
                "risk": { "type": "string", "enum": ["low", "medium", "high"] },
                "count": { "type": "integer", "minimum": 1 }
            }
        });

        let fenced = "Here is the report:\n```json\n{\"files\":[\"src/lib.rs\"],\"risk\":\"low\",\"count\":2}\n```";
        let extracted = extract_json_payload(fenced).expect("extract fenced json");
        assert!(validate_json_schema(&schema, &extracted).is_ok());

        let bad = json!({"files": [], "risk": "critical", "count": 0, "extra": true});
        let errs = validate_json_schema(&schema, &bad).expect_err("must fail");
        assert!(errs.len() >= 4, "expected 4 schema errors, got {errs:?}");

        // Schema-guided extractor skips earlier false-positive JSON arrays `[0, 1]` in prose!
        let noisy = "Checked indexes [0, 1] first, final payload: {\"files\":[\"src/main.rs\"],\"risk\":\"medium\",\"count\":1}";
        let picked = extract_json_for_schema(noisy, &schema).expect("pick schema-valid object");
        assert!(validate_json_schema(&schema, &picked).is_ok());

        // `anyOf` / `oneOf` + standalone `additionalProperties: false` support
        let union_schema = json!({
            "oneOf": [
                { "type": "object", "required": ["ok"], "additionalProperties": false, "properties": { "ok": { "type": "boolean" } } },
                { "type": "string", "minLength": 2 }
            ]
        });
        assert!(validate_json_schema(&union_schema, &json!({"ok": true})).is_ok());
        assert!(validate_json_schema(&union_schema, &json!("yes")).is_ok());
        assert!(validate_json_schema(&union_schema, &json!({"ok": true, "bad": 1})).is_err());
    }

    #[test]
    fn dynamic_role_model_routing_has_zero_hardcoded_models() {
        let mut cfg = ZenkoConfig::default();
        cfg.provider.lead_model = "vendor/lead-v1".into();
        cfg.provider.fast_model = "vendor/fast-v1".into();
        cfg.provider.plan_model = "vendor/plan-v1".into();
        cfg.provider.advisor_model = "".into();

        let explorer = find_subagent_role("explorer").unwrap();
        let architect = find_subagent_role("architect").unwrap();
        let coder = find_subagent_role("coder").unwrap();
        let verifier = find_subagent_role("verifier").unwrap();

        assert_eq!(resolve_role_model(&cfg, explorer, None).as_deref(), Some("vendor/fast-v1"));
        assert_eq!(resolve_role_model(&cfg, architect, None).as_deref(), Some("vendor/plan-v1"));
        assert_eq!(resolve_role_model(&cfg, coder, None).as_deref(), Some("vendor/lead-v1"));
        assert_eq!(resolve_role_model(&cfg, verifier, None).as_deref(), Some("vendor/plan-v1"));
        assert_eq!(
            resolve_role_model(&cfg, explorer, Some("custom/override")).as_deref(),
            Some("custom/override")
        );
    }

    #[test]
    fn slot_reservation_enforces_exact_concurrency_and_raii_cleanup() {
        let sup = SubagentSupervisor::new();
        let s1 = SlotReservation::try_acquire(&sup.state, 2).expect("slot 1");
        let s2 = SlotReservation::try_acquire(&sup.state, 2).expect("slot 2");
        assert_eq!(sup.active_count(), 2);
        assert!(
            SlotReservation::try_acquire(&sup.state, 2).is_err(),
            "3rd slot must be denied before any await"
        );

        // Dropping an unactivated reservation releases its slot automatically
        drop(s1);
        assert_eq!(sup.active_count(), 1);

        // Activating moves reserved -> active atomically without changing count
        let (_tx, rx) = watch::channel(SubagentStatus::Running);
        s2.activate(
            "child_1".into(),
            ActiveSubagentHandle {
                parent_session_id: "root_1".into(),
                cancel: CancellationToken::new(),
                status_rx: rx,
            },
        );
        assert_eq!(sup.active_count(), 1);
        assert!(sup.is_active("child_1"));
    }

    #[test]
    fn budget_clamping_is_strictly_monotonic() {
        assert_eq!(clamp_subagent_duration(300, None), Some(300));
        assert_eq!(clamp_subagent_duration(300, Some(60)), Some(60));
        assert_eq!(clamp_subagent_duration(300, Some(999)), Some(300));
        assert_eq!(clamp_subagent_duration(0, None), None);
        assert_eq!(clamp_subagent_duration(0, Some(90)), Some(90));

        assert_eq!(clamp_subagent_cost(0.50, None), Some(0.50));
        assert_eq!(clamp_subagent_cost(0.50, Some(0.10)), Some(0.10));
        assert_eq!(clamp_subagent_cost(0.50, Some(5.00)), Some(0.50));
        assert_eq!(clamp_subagent_cost(0.0, None), None);
        assert_eq!(clamp_subagent_cost(0.0, Some(0.25)), Some(0.25));
    }

    #[test]
    fn subtree_ownership_guard_allows_ancestors_and_operators_only() {
        let dir = std::env::temp_dir().join(format!("zenko_own_test_{}", new_id()));
        let store = SqliteStore::open(dir.join("zenko.db")).expect("db");

        let root_a = store
            .create_session("Root A", "webui", None, None, PermissionMode::Balanced)
            .expect("root_a");
        let root_b = store
            .create_session("Root B", "webui", None, None, PermissionMode::Balanced)
            .expect("root_b");
        let child_a = store
            .create_child_session(&root_a.id, "architect", 1, "A1", "/tmp", None, None, None, PermissionMode::Balanced)
            .expect("child_a");
        let grand_a = store
            .create_child_session(&child_a.id, "coder", 2, "A1.1", "/tmp", None, None, None, PermissionMode::Balanced)
            .expect("grand_a");
        let sibling_a = store
            .create_child_session(&root_a.id, "explorer", 1, "A2", "/tmp", None, None, None, PermissionMode::Balanced)
            .expect("sibling_a");

        for (id, parent, role, depth) in [
            (&child_a.id, &root_a.id, "architect", 1u8),
            (&grand_a.id, &child_a.id, "coder", 2u8),
            (&sibling_a.id, &root_a.id, "explorer", 1u8),
        ] {
            store
                .upsert_subagent(&SubagentRecord {
                    id: id.clone(),
                    parent_session_id: parent.clone(),
                    role: role.into(),
                    depth,
                    goal: "test".into(),
                    status: SubagentStatus::Completed,
                    workspace_mode: SubagentWorkspaceMode::Inherit,
                    workspace_dir: "/tmp".into(),
                    worktree_branch: None,
                    worktree_base_commit: None,
                    worktree_dirty_baseline: false,
                    model: None,
                    summary: None,
                    output_schema: None,
                    structured_output: None,
                    diff_stat: None,
                    changed_files: vec![],
                    tokens_used: 0,
                    cost_usd: 0.0,
                    tree_tokens: 0,
                    tree_cost_usd: 0.0,
                    descendant_count: 0,
                    resume_count: 0,
                    started_at: Utc::now(),
                    finished_at: Some(Utc::now()),
                })
                .expect("upsert subagent");
        }

        // Operator consoles & ancestors allowed
        assert!(ensure_subtree_owner(&store, "webui", &grand_a.id, "merge").is_ok());
        assert!(ensure_subtree_owner(&store, "cli", &grand_a.id, "diff").is_ok());
        assert!(ensure_subtree_owner(&store, &child_a.id, &grand_a.id, "merge").is_ok());
        assert!(ensure_subtree_owner(&store, &root_a.id, &grand_a.id, "merge").is_ok());
        // Unrelated root, sibling, and child-itself are strictly denied!
        assert!(ensure_subtree_owner(&store, &root_b.id, &grand_a.id, "merge").is_err());
        assert!(ensure_subtree_owner(&store, &sibling_a.id, &grand_a.id, "diff").is_err());
        assert!(ensure_subtree_owner(&store, &grand_a.id, &grand_a.id, "merge").is_err());

        let _ = std::fs::remove_dir_all(dir);
    }
}
