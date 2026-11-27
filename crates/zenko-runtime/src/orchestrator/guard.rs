//! Deterministic Orchestrator State Machine, Canonical Loop Detector, Doom-Loop Guard &
//! Verification/Plan Completion Gate.
//! Zero regex, zero `if/else` ladders, zero LLM cost — pure `match` dispatch, quote-aware
//! shell operator lexer, sorted-key canonical JSON FNV-1a hashing, digit-normalized error
//! signatures, and exact line-hash doom-loop detection.

use crate::orchestrator::plan::{incomplete_steps, plan_progress};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, VecDeque};
use zenko_core::{
    BackgroundJobStatus, DeliberationDepth, ExecutionPlan, ExecutionTier, IntentKind,
    MUTATING_FILE_TOOLS, ORCH_ITER_WRAPUP_REMAINING, ORCH_LOOP_REPEAT_THRESHOLD,
    ORCH_LOOP_WINDOW_SIZE, ORCH_MAX_CONSECUTIVE_ERRORS, ORCH_MAX_PIVOTS_BEFORE_ESCALATE,
    ORCH_MAX_PLAN_CONTINUATION_NUDGES, ORCH_MAX_TOTAL_CONTINUATION_NUDGES,
    ORCH_NON_ERROR_REPEAT_THRESHOLD, ORCH_OSCILLATION_THRESHOLD, ORCH_STAGE_PHASE_GATE,
    ORCH_TEXT_DOOM_MIN_LINE_CHARS, ORCH_TEXT_DOOM_MIN_LINES, ORCH_TEXT_DOOM_REPEAT_THRESHOLD,
    OrchestratorPhase, PLANNING_TOOLS, READ_ONLY_FIND_MUTATING_FLAGS,
    READ_ONLY_SHELL_CMD_PREFIXES, READ_ONLY_TOOLS, ScriptBlockAction, TOOL_AGENT_KILL,
    TOOL_AGENT_WORKTREE, TOOL_BATCH_TOOLS, TOOL_CODE_REFACTOR, TOOL_CRON, TOOL_GIT,
    TOOL_GOAL_LOOP, TOOL_INVOKE_TOOL, TOOL_JOB_WAIT, TOOL_LSP, TOOL_MEMORY, TOOL_PLAN_UPDATE,
    TOOL_PROJECT_BRAIN, TOOL_RECORD_BRAINSTORM, TOOL_SCRIPT_BLOCK, TOOL_SESSION_TASK,
    TOOL_SHELL_EXEC, TOOL_SKILL_MANAGE, TOOL_SPAWN_AGENT, TOOL_TOKEN_ECONOMY, TOOL_ZPOWERS,
    VERIFICATION_CMD_PREFIXES, VERIFICATION_SHELL_TOOLS, VERIFICATION_SPOOF_FLAGS_CI,
    VERIFICATION_SPOOF_FLAGS_EXACT, WorktreeAction, ZPOWERS_DEFAULT_DEBUG_STRIKE_LIMIT,
    ZPOWERS_GUIDED_MAX_GATE_REDIRECTS, ZPOWERS_STRICT_HANDOFF_MIN_FILES,
    ZPOWERS_STRICT_MAX_GATE_REDIRECTS, ZPowersAction, ZPowersMode, ZPowersPhase, ZPowersState,
    compact_preview, find_subagent_role,
    is_brain_tool_args_mutating, is_code_refactor_args_mutating, is_economy_args_mutating,
    is_git_tool_args_mutating, is_goal_loop_args_mutating, is_lsp_tool_args_mutating,
    is_memory_mutating_action, is_skill_manage_args_mutating, is_zpowers_args_mutating,
};

use zenko_core::{FNV1A_OFFSET_BASIS, FNV1A_PRIME};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCategory {
    ReadOnly,
    Planning,
    Mutation,
    Verification,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhaseGateDecision {
    Allow,
    Redirect {
        stage: &'static str,
        reason: String,
        hint: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardVerdict {
    Continue,
    Pivot { reason: String, hint: String },
    Escalate { reason: String, summary: String },
}

#[derive(Debug, Clone)]
struct CallFingerprint {
    tool_name: String,
    args_hash: u64,
    out_hash: u64,
    is_error: bool,
}

/// Declarative classifier for built-in scoped tools that mutate state on specific actions.
fn is_mutating_scoped_tool_call(tool_name: &str, arguments: &Value) -> bool {
    match tool_name {
        TOOL_GIT => is_git_tool_args_mutating(arguments),
        TOOL_LSP => is_lsp_tool_args_mutating(arguments),
        TOOL_SKILL_MANAGE => is_skill_manage_args_mutating(arguments),
        TOOL_GOAL_LOOP => is_goal_loop_args_mutating(arguments),
        TOOL_TOKEN_ECONOMY => is_economy_args_mutating(arguments),
        TOOL_PROJECT_BRAIN => is_brain_tool_args_mutating(arguments),
        zenko_core::TOOL_KNOWLEDGE_CATALOG => zenko_core::is_okf_tool_args_mutating(arguments),
        TOOL_ZPOWERS => is_zpowers_args_mutating(arguments),
        TOOL_CODE_REFACTOR => is_code_refactor_args_mutating(arguments),
        TOOL_MEMORY => {
            let action = arguments.get("action").and_then(Value::as_str).unwrap_or("");
            let dry_run_compress = action == "compress_doc"
                && arguments.get("dry_run").and_then(Value::as_bool) == Some(true);
            !dry_run_compress && is_memory_mutating_action(action)
        }
        TOOL_SCRIPT_BLOCK => arguments
            .get("action")
            .and_then(Value::as_str)
            .and_then(ScriptBlockAction::parse)
            .is_none_or(ScriptBlockAction::is_mutating),
        TOOL_CRON => arguments
            .get("action")
            .and_then(Value::as_str)
            .map(|a| !a.eq_ignore_ascii_case("list"))
            .unwrap_or(true),
        TOOL_SPAWN_AGENT => {
            let role_id = arguments.get("role").and_then(Value::as_str).unwrap_or("");
            !find_subagent_role(role_id).is_some_and(|r| r.read_only)
        }
        TOOL_AGENT_WORKTREE => {
            arguments
                .get("action")
                .and_then(Value::as_str)
                .and_then(WorktreeAction::parse)
                != Some(WorktreeAction::Diff)
        }
        TOOL_AGENT_KILL => true,
        _ => false,
    }
}

/// Deterministic tool classifier driving the `OrchestratorPhase` state machine.
pub fn classify_tool_category(tool_name: &str, arguments: &Value) -> ToolCategory {
    if tool_name == TOOL_INVOKE_TOOL {
        let inner_name = arguments
            .get("tool")
            .or_else(|| arguments.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        if inner_name.is_empty() || inner_name == TOOL_INVOKE_TOOL || inner_name == TOOL_BATCH_TOOLS
        {
            return ToolCategory::ReadOnly;
        }
        let inner_args = arguments
            .get("args")
            .or_else(|| arguments.get("arguments"))
            .unwrap_or(&Value::Null);
        return classify_tool_category(inner_name, inner_args);
    }
    if tool_name == TOOL_BATCH_TOOLS {
        let Some(calls) = arguments.get("calls").and_then(Value::as_array) else {
            return ToolCategory::ReadOnly;
        };
        let mut saw_verify = false;
        let mut saw_plan = false;
        for item in calls {
            let inner_name = item
                .get("tool")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if inner_name == TOOL_BATCH_TOOLS || inner_name.is_empty() {
                continue;
            }
            let inner_args = item
                .get("args")
                .or_else(|| item.get("arguments"))
                .unwrap_or(&Value::Null);
            match classify_tool_category(inner_name, inner_args) {
                ToolCategory::Mutation => return ToolCategory::Mutation,
                ToolCategory::Verification => saw_verify = true,
                ToolCategory::Planning => saw_plan = true,
                _ => {}
            }
        }
        return match (saw_verify, saw_plan) {
            (true, _) => ToolCategory::Verification,
            (false, true) => ToolCategory::Planning,
            (false, false) => ToolCategory::ReadOnly,
        };
    }
    match (
        MUTATING_FILE_TOOLS.contains(&tool_name) || is_mutating_scoped_tool_call(tool_name, arguments),
        PLANNING_TOOLS.contains(&tool_name),
        READ_ONLY_TOOLS.contains(&tool_name)
            || (tool_name == TOOL_LSP && !is_lsp_tool_args_mutating(arguments))
            || (tool_name == TOOL_TOKEN_ECONOMY && !is_economy_args_mutating(arguments))
            || (tool_name == TOOL_PROJECT_BRAIN && !is_brain_tool_args_mutating(arguments))
            || (tool_name == zenko_core::TOOL_KNOWLEDGE_CATALOG
                && !zenko_core::is_okf_tool_args_mutating(arguments))
            || (tool_name == TOOL_CODE_REFACTOR && !is_code_refactor_args_mutating(arguments)),
        VERIFICATION_SHELL_TOOLS.contains(&tool_name),
    ) {
        (true, _, _, _) => ToolCategory::Mutation,
        (_, true, _, _) => ToolCategory::Planning,
        (_, _, true, _) => ToolCategory::ReadOnly,
        (_, _, _, true) => {
            let cmd = arguments
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("");
            match tool_name == TOOL_JOB_WAIT || is_verification_command(cmd) {
                true => ToolCategory::Verification,
                false => ToolCategory::Other,
            }
        }
        _ => ToolCategory::Other,
    }
}

/// Stateful per-turn execution monitor enforcing:
/// 1. `OrchestratorPhase` state machine (`Recon -> Plan -> Execute -> Verify -> Done`)
/// 2. Pre-tool read/plan-before-write phase gate on `Orchestrated` turns
/// 3. Canonical-JSON & digit-normalized failing + non-error spinning loop detection
/// 4. Oscillating file-edit thrashing & consecutive error streak circuit breaker
/// 5. Hard `Escalate` verdict after `ORCH_MAX_PIVOTS_BEFORE_ESCALATE` pivots
/// 6. Post-edit build/test verification gate + unfinished DAG plan-step completion gate
#[derive(Debug, Default)]
pub struct ExecutionGuard {
    intent: IntentKind,
    deliberation: DeliberationDepth,
    phase: OrchestratorPhase,
    phase_gate_nudges: usize,
    read_or_planned: bool,
    window: VecDeque<CallFingerprint>,
    consecutive_errors: usize,
    file_edit_counts: HashMap<String, usize>,
    mutated_files: BTreeSet<String>,
    pending_verification_jobs: BTreeSet<String>,
    verification_ran: bool,
    last_verification_failed: bool,
    verify_nudges: usize,
    plan_nudges: usize,
    total_continuation_nudges: usize,
    last_completed_steps: usize,
    pivot_count: usize,
    active_plan: Option<ExecutionPlan>,
    brainstorm_recorded: bool,
    plan_recorded: bool,
    read_probe_streak: usize,
    wrapup_nudge_sent: bool,
    p90_envelope_calls: usize,
    p90_redirect_sent: bool,
    total_tool_calls: usize,
    total_tool_errors: usize,
    zpowers_mode: ZPowersMode,
    fast_track_reason: Option<String>,
    debug_strike_limit: usize,
    zpowers_debug_triggered: bool,
    reviews_recorded: usize,
    walkthrough_recorded: bool,
    active_session_id: Option<String>,
    workspace_dir: Option<std::path::PathBuf>,
    mutating_iters_since_task_update: usize,
    todo_drift_nudges: usize,
}

impl ExecutionGuard {
    pub fn new(tier: ExecutionTier, intent: IntentKind, deliberation: DeliberationDepth) -> Self {
        let initial_phase = match (tier, intent.is_read_only()) {
            (ExecutionTier::Orchestrated, _) | (_, true) => OrchestratorPhase::Recon,
            (ExecutionTier::Instant | ExecutionTier::Standard, false) => OrchestratorPhase::Execute,
        };
        Self {
            intent,
            deliberation,
            phase: initial_phase,
            zpowers_mode: ZPowersMode::Advisory,
            debug_strike_limit: ZPOWERS_DEFAULT_DEBUG_STRIKE_LIMIT,
            ..Self::default()
        }
    }

    /// Configures ZPowers enforcement mode and hydrates persisted session-level progress
    /// purely from live workstream state (zero hardcoded turn-number rules):
    /// - When open todo tasks exist (`has_open_tasks == true`) or a freshly recorded plan has
    ///   not yet started mutating files (e.g., Plan Mode -> Execute), `plan_recorded` carries over.
    /// - When a prior workstream already finished (`Done`, mutated & closed all todos, or prior
    ///   `fast_track`), planning readiness resets cleanly so the orchestrator decides fresh.
    pub fn configure_zpowers(
        &mut self,
        mode: ZPowersMode,
        has_open_tasks: bool,
        strike_limit: usize,
        workspace_dir: std::path::PathBuf,
        session_id: String,
        state: &ZPowersState,
    ) {
        self.zpowers_mode = mode;
        self.debug_strike_limit = strike_limit.max(1);
        self.workspace_dir = Some(workspace_dir);
        self.active_session_id = Some(session_id);

        let prior_workstream_finished = !has_open_tasks
            && (state.phase == ZPowersPhase::Done
                || state.fast_track_reason.is_some()
                || !state.mutated_files.is_empty());

        if !prior_workstream_finished {
            if state.spec_recorded {
                self.brainstorm_recorded = true;
            }
            if has_open_tasks || state.plan_recorded {
                self.plan_recorded = true;
                self.read_or_planned = true;
            }
        }
        self.reviews_recorded = state.reviews.len();
        self.walkthrough_recorded = match prior_workstream_finished {
            true => false,
            false => state.walkthrough_recorded,
        };
    }

    pub fn zpowers_debug_triggered(&self) -> bool {
        self.zpowers_debug_triggered
    }

    pub fn clear_zpowers_debug_triggered(&mut self) {
        self.zpowers_debug_triggered = false;
    }

    pub fn mutated_files_set(&self) -> &BTreeSet<String> {
        &self.mutated_files
    }

    pub fn verification_state(&self) -> (bool, bool) {
        (self.verification_ran, self.last_verification_failed)
    }

    pub fn phase(&self) -> OrchestratorPhase {
        self.phase
    }

    pub fn intent(&self) -> IntentKind {
        self.intent
    }

    pub fn deliberation(&self) -> DeliberationDepth {
        self.deliberation
    }

    pub fn active_plan(&self) -> Option<&ExecutionPlan> {
        self.active_plan.as_ref()
    }

    /// Hydrates an existing unfinished DAG plan carried forward from an earlier turn.
    pub fn hydrate_active_plan(&mut self, plan: ExecutionPlan) {
        let (done, _, _) = plan_progress(&plan);
        if done > self.last_completed_steps {
            self.last_completed_steps = done;
            // Genuine forward progress resets the local continuation nudge counter (bounded by total cap)
            self.plan_nudges = 0;
        }
        self.read_or_planned = true;
        self.plan_recorded = true;
        self.phase = match self.phase {
            OrchestratorPhase::Recon => OrchestratorPhase::Plan,
            other => other,
        };
        self.active_plan = Some(plan);
    }

    pub fn update_active_plan(&mut self, plan: ExecutionPlan) {
        self.hydrate_active_plan(plan);
    }

    /// Grants a clean retry budget after interactive human approval to pivot & continue.
    pub fn reset_after_escalation(&mut self) {
        self.pivot_count = 0;
        self.consecutive_errors = 0;
        self.window.clear();
    }

    /// Records a wave-level Phase Gate nudge ONCE after partitioning all tool calls in a wave,
    /// guaranteeing multi-mutation waves cannot bypass the gate on the 2nd write in the same wave.
    pub fn record_phase_gate_nudge(&mut self) {
        self.phase_gate_nudges = self.phase_gate_nudges.saturating_add(1);
    }

    /// Records a text/reasoning doom-loop recovery pivot so retries stay strictly bounded.
    pub fn record_text_doom_pivot(&mut self) {
        self.pivot_count = self.pivot_count.saturating_add(1);
    }

    /// Configures the empirical per-repository `1.25 x P90` turn-envelope threshold (`0` disables).
    pub fn set_p90_envelope(&mut self, envelope_calls: usize) {
        self.p90_envelope_calls = envelope_calls;
    }

    pub fn p90_redirect_triggered(&self) -> bool {
        self.p90_redirect_sent
    }

    fn is_mutation_inside_own_session_dir(&self, tool_name: &str, arguments: &Value) -> bool {
        let (Some(ws), Some(sid)) = (self.workspace_dir.as_deref(), self.active_session_id.as_deref())
        else {
            return false;
        };
        match tool_name {
            zenko_core::TOOL_WRITE_FILE | zenko_core::TOOL_HASH_EDIT => arguments
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|p| crate::zpowers::is_path_inside_active_session_workspace(ws, p, sid)),
            _ => false,
        }
    }

    /// ZPowers Sovereign Plan-First Pre-Tool Gate (purely state-driven; zero turn-number rules):
    /// - Read-only Q&A, exploration, planning, verification, and writes inside the session's own
    ///   `.zenko/sessions/<id>/` folder ALWAYS pass through with zero overhead.
    /// - Before mutating workspace code when no active plan/todo exists (and no `fast_track`
    ///   was declared), redirects cleanly so the orchestrator itself decides: either create a
    ///   bite-sized plan + todo checklist (`plan_update` / `session_task` / `zpowers.write_plan`)
    ///   or explicitly declare `zpowers.fast_track` for a tiny surgical fix.
    pub fn check_pre_tool_phase_gate(
        &self,
        enabled: bool,
        tool_name: &str,
        arguments: &Value,
    ) -> PhaseGateDecision {
        if !enabled
            || self.fast_track_reason.is_some()
            || matches!(self.zpowers_mode, ZPowersMode::Off | ZPowersMode::Advisory)
        {
            return PhaseGateDecision::Allow;
        }
        let category = classify_tool_category(tool_name, arguments);
        if category != ToolCategory::Mutation {
            return PhaseGateDecision::Allow;
        }
        if self.is_mutation_inside_own_session_dir(tool_name, arguments) {
            return PhaseGateDecision::Allow;
        }

        let zp_dir = self
            .active_session_id
            .as_deref()
            .map(crate::zpowers::workspace_rel_zpowers_dir)
            .unwrap_or_else(|| ".zenko/sessions/<id>/zpowers".to_string());

        match self.zpowers_mode {
            ZPowersMode::Off | ZPowersMode::Advisory => PhaseGateDecision::Allow,
            ZPowersMode::Guided => match !self.plan_recorded
                && self.phase_gate_nudges < ZPOWERS_GUIDED_MAX_GATE_REDIRECTS
            {
                true => PhaseGateDecision::Redirect {
                    stage: ORCH_STAGE_PHASE_GATE,
                    reason: format!(
                        "ZPowers plan-first gate: create a plan/todo checklist or declare `fast_track` before editing (`{tool_name}`)"
                    ),
                    hint: format!(
                        "<orchestrator_hint type=\"zpowers_plan_first\">\n\
                         ZPowers Plan-First Gate: before modifying workspace files, decide your approach and either:\n\
                         1. Record a bite-sized plan + live todo list via `plan_update`, `session_task`, or `invoke_tool(tool=\"{TOOL_ZPOWERS}\", args={{\"action\":\"write_plan\",...}})` (all bidirectionally synced to `{zp_dir}/plan.md` and the live Todo DAG), OR\n\
                         2. If you judge this edit genuinely trivial/surgical, call `invoke_tool(tool=\"{TOOL_ZPOWERS}\", args={{\"action\":\"fast_track\",\"reason\":\"...\"}})` to proceed immediately.\n\
                         </orchestrator_hint>"
                    ),
                },
                false => PhaseGateDecision::Allow,
            },
            ZPowersMode::Strict => match (
                self.deliberation == DeliberationDepth::Deliberate && !self.brainstorm_recorded,
                !self.plan_recorded,
                self.phase_gate_nudges < ZPOWERS_STRICT_MAX_GATE_REDIRECTS,
            ) {
                (true, _, true) => PhaseGateDecision::Redirect {
                    stage: ORCH_STAGE_PHASE_GATE,
                    reason: format!(
                        "ZPowers strict spec gate: record a design spec before editing (`{tool_name}`)"
                    ),
                    hint: format!(
                        "<orchestrator_hint type=\"zpowers_spec_first\">\n\
                         ZPowers Strict Gate: record a concise design spec first via `record_brainstorm` or `invoke_tool(tool=\"{TOOL_ZPOWERS}\", args={{\"action\":\"write_spec\",...}})` (saved to `{zp_dir}/spec.md`), then create a bite-sized todo plan in `{zp_dir}/plan.md`.\n\
                         </orchestrator_hint>"
                    ),
                },
                (false, true, true) => PhaseGateDecision::Redirect {
                    stage: ORCH_STAGE_PHASE_GATE,
                    reason: format!(
                        "ZPowers strict plan gate: record an implementation plan/todo checklist before editing (`{tool_name}`)"
                    ),
                    hint: format!(
                        "<orchestrator_hint type=\"zpowers_plan_first\">\n\
                         ZPowers Strict Gate: translate your spec into bite-sized DAG todos via `plan_update`, `session_task`, or `invoke_tool(tool=\"{TOOL_ZPOWERS}\", args={{\"action\":\"write_plan\",...}})` (synced to `{zp_dir}/plan.md` and live Todo DAG) before modifying files.\n\
                         </orchestrator_hint>"
                    ),
                },
                _ => PhaseGateDecision::Allow,
            },
        }
    }

    fn advance_phase(&mut self, category: ToolCategory, is_error: bool) {
        self.phase = match (self.phase, category, is_error) {
            (OrchestratorPhase::Recon, ToolCategory::ReadOnly, false) => {
                self.read_or_planned = true;
                OrchestratorPhase::Plan
            }
            (OrchestratorPhase::Recon | OrchestratorPhase::Plan, ToolCategory::Planning, false) => {
                self.read_or_planned = true;
                OrchestratorPhase::Plan
            }
            (_, ToolCategory::Mutation, false) => {
                self.read_or_planned = true;
                OrchestratorPhase::Execute
            }
            (_, ToolCategory::Verification, false) => OrchestratorPhase::Verify,
            (cur, _, _) => cur,
        };
    }

    fn escalate_or_pivot(&mut self, reason: String, pivot_hint: String) -> GuardVerdict {
        self.pivot_count += 1;
        match self.pivot_count >= ORCH_MAX_PIVOTS_BEFORE_ESCALATE {
            true => {
                let summary = format!(
                    "Escalated after {} circuit-breaker pivots (latest: {reason}; phase={}).",
                    self.pivot_count,
                    self.phase.as_str()
                );
                GuardVerdict::Escalate { reason, summary }
            }
            false => GuardVerdict::Pivot {
                reason,
                hint: pivot_hint,
            },
        }
    }

    fn record_planning_outcome(&mut self, tool_name: &str, arguments: &Value) {
        match tool_name {
            TOOL_RECORD_BRAINSTORM => self.brainstorm_recorded = true,
            TOOL_PLAN_UPDATE => self.plan_recorded = true,
            TOOL_SESSION_TASK => {
                let act = arguments
                    .get("action")
                    .and_then(Value::as_str)
                    .and_then(zenko_core::SessionTaskAction::parse);
                if matches!(
                    act,
                    Some(
                        zenko_core::SessionTaskAction::Create
                            | zenko_core::SessionTaskAction::Update
                            | zenko_core::SessionTaskAction::Batch
                            | zenko_core::SessionTaskAction::Claim
                    )
                ) {
                    self.plan_recorded = true;
                }
            }
            TOOL_ZPOWERS => {
                let act = arguments
                    .get("action")
                    .and_then(Value::as_str)
                    .and_then(ZPowersAction::parse);
                match act {
                    Some(ZPowersAction::WriteSpec) => self.brainstorm_recorded = true,
                    Some(ZPowersAction::WritePlan) => self.plan_recorded = true,
                    Some(ZPowersAction::FastTrack) => {
                        let reason = arguments
                            .get("reason")
                            .and_then(Value::as_str)
                            .unwrap_or("fast_track");
                        self.fast_track_reason = Some(reason.to_string());
                    }
                    Some(ZPowersAction::RecordReview | ZPowersAction::RunReview) => {
                        self.reviews_recorded = self.reviews_recorded.saturating_add(1);
                    }
                    Some(ZPowersAction::WriteWalkthrough) => {
                        self.walkthrough_recorded = true;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    pub fn record_tool_outcome(
        &mut self,
        tool_name: &str,
        arguments: &Value,
        is_error: bool,
        output: &str,
        metadata: Option<&Value>,
    ) -> GuardVerdict {
        // Unwrap KV-cache-safe `invoke_tool` proxy so mutation, verification, and planning
        // guards observe the exact underlying deferred tool seamlessly.
        let (tool_name, arguments) = match tool_name == TOOL_INVOKE_TOOL {
            true => {
                let inner_name = arguments
                    .get("tool")
                    .or_else(|| arguments.get("name"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && *s != TOOL_INVOKE_TOOL && *s != TOOL_BATCH_TOOLS);
                match inner_name {
                    Some(n) => (
                        n,
                        arguments
                            .get("args")
                            .or_else(|| arguments.get("arguments"))
                            .unwrap_or(&Value::Null),
                    ),
                    None => (tool_name, arguments),
                }
            }
            false => (tool_name, arguments),
        };
        let category = classify_tool_category(tool_name, arguments);
        self.advance_phase(category, is_error);

        // Track structured planning & ZPowers artifacts recorded during the turn
        if !is_error {
            self.record_planning_outcome(tool_name, arguments);
        }

        // Track mutated workspace files, planning steps & verified completions inside `batch_tools`
        if tool_name == TOOL_BATCH_TOOLS
            && let Some(calls) = arguments.get("calls").and_then(Value::as_array)
        {
            let steps_meta = metadata.and_then(|m| m.get("steps")).and_then(Value::as_array);
            for (idx, item) in calls.iter().enumerate() {
                let inner_name = item
                    .get("tool")
                    .or_else(|| item.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if inner_name.is_empty() || inner_name == TOOL_BATCH_TOOLS {
                    continue;
                }
                let step_status = steps_meta
                    .and_then(|arr| arr.get(idx))
                    .and_then(|s| s.get("status"))
                    .and_then(Value::as_str)
                    .unwrap_or(match is_error {
                        true => "error",
                        false => "ok",
                    });
                let inner_args = item
                    .get("args")
                    .or_else(|| item.get("arguments"))
                    .unwrap_or(&Value::Null);
                if step_status == "ok" {
                    self.record_planning_outcome(inner_name, inner_args);
                }
                match (step_status, inner_name) {
                    ("ok", n) if MUTATING_FILE_TOOLS.contains(&n) => {
                        if let Some(p) = inner_args.get("path").and_then(Value::as_str) {
                            let norm = p.trim().trim_start_matches("./").to_string();
                            if !norm.is_empty() {
                                self.mutated_files.insert(norm.clone());
                                *self.file_edit_counts.entry(norm).or_insert(0) += 1;
                                self.verification_ran = false;
                            }
                        }
                    }
                    ("ok", TOOL_SHELL_EXEC) => {
                        let cmd = inner_args
                            .get("command")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let is_verify = is_verification_command(cmd)
                            || (!self.mutated_files.is_empty()
                                && !cmd.trim().is_empty()
                                && !is_read_only_shell_command(cmd));
                        if is_verify {
                            self.verification_ran = true;
                            self.last_verification_failed = false;
                        }
                    }
                    ("ok", TOOL_LSP) => {
                        if inner_args
                            .get("action")
                            .and_then(Value::as_str)
                            .is_some_and(|a| a.eq_ignore_ascii_case("diagnostics"))
                        {
                            self.verification_ran = true;
                            self.last_verification_failed = false;
                        }
                    }
                    ("error", TOOL_SHELL_EXEC) => {
                        let cmd = inner_args
                            .get("command")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        if is_verification_command(cmd) {
                            self.verification_ran = false;
                            self.last_verification_failed = true;
                        }
                    }
                    _ => {}
                }
            }
        }
        if !is_error
            && (MUTATING_FILE_TOOLS.contains(&tool_name)
                || (tool_name == TOOL_LSP && is_lsp_tool_args_mutating(arguments)))
        {
            let mut touched: Vec<String> = Vec::new();
            if let Some(p) = arguments.get("path").and_then(Value::as_str) {
                touched.push(p.to_string());
            }
            if let Some(files) = arguments.get("files").and_then(Value::as_array) {
                for item in files {
                    if let Some(p) = item.get("move_to").or_else(|| item.get("path")).and_then(Value::as_str) {
                        touched.push(p.to_string());
                    }
                }
            }
            if let Some(meta_files) = metadata
                .and_then(|m| m.get("edited_files"))
                .and_then(Value::as_array)
            {
                for item in meta_files {
                    if let Some(p) = item.as_str() {
                        touched.push(p.to_string());
                    }
                }
            }
            for raw_p in touched {
                let norm = raw_p.trim().trim_start_matches("./").to_string();
                if !norm.is_empty() {
                    self.mutated_files.insert(norm.clone());
                    *self.file_edit_counts.entry(norm).or_insert(0) += 1;
                    self.verification_ran = false;
                }
            }
        }
        if is_error && tool_name == TOOL_SHELL_EXEC {
            let cmd = arguments
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("");
            if is_verification_command(cmd) {
                self.verification_ran = false;
                self.last_verification_failed = true;
            }
        }
        if !is_error {
            match tool_name {
                TOOL_SHELL_EXEC => {
                    let cmd = arguments
                        .get("command")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let status = metadata.and_then(|m| m.get("status")).and_then(Value::as_str);
                    let job_id = metadata.and_then(|m| m.get("id")).and_then(Value::as_str);
                    // Accept any passing non-spoof shell check (not restricted solely to hardcoded command prefixes)
                    let is_explicit_verify = is_verification_command(cmd);
                    let is_substantive_check = !self.mutated_files.is_empty()
                        && !cmd.trim().is_empty()
                        && !is_read_only_shell_command(cmd);
                    if is_explicit_verify || is_substantive_check {
                        match (status, job_id) {
                            (Some(s), Some(jid)) if s == BackgroundJobStatus::Running.as_str() => {
                                self.pending_verification_jobs.insert(jid.to_string());
                            }
                            (None, _) => {
                                self.verification_ran = true;
                                self.last_verification_failed = false;
                            }
                            (Some(s), _)
                                if s == BackgroundJobStatus::Completed.as_str()
                                    && metadata
                                        .and_then(|m| m.get("exit_code"))
                                        .and_then(Value::as_i64)
                                        == Some(0) =>
                            {
                                self.verification_ran = true;
                                self.last_verification_failed = false;
                            }
                            _ => {}
                        }
                    }
                }
                TOOL_LSP => {
                    if !is_lsp_tool_args_mutating(arguments) && !self.mutated_files.is_empty() {
                        self.verification_ran = true;
                        self.last_verification_failed = false;
                    }
                }
                TOOL_SPAWN_AGENT => {
                    let role = arguments.get("role").and_then(Value::as_str).unwrap_or("");
                    let bg = arguments
                        .get("run_in_background")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    if !bg && matches!(role, "verifier" | "critic") && !self.mutated_files.is_empty()
                    {
                        self.verification_ran = true;
                        self.last_verification_failed = false;
                    }
                }
                TOOL_JOB_WAIT => {
                    let jid = arguments
                        .get("job_id")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let job_meta = metadata.and_then(|m| m.get("job"));
                    let status_done = job_meta
                        .and_then(|j| j.get("status"))
                        .and_then(Value::as_str)
                        == Some(BackgroundJobStatus::Completed.as_str());
                    let exit_zero = job_meta
                        .and_then(|j| j.get("exit_code"))
                        .and_then(Value::as_i64)
                        == Some(0);
                    let cmd_in_meta = job_meta
                        .and_then(|j| j.get("command"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if status_done
                        && exit_zero
                        && (self.pending_verification_jobs.remove(jid)
                            || is_verification_command(cmd_in_meta))
                    {
                        self.verification_ran = true;
                        self.last_verification_failed = false;
                    } else if job_meta
                        .and_then(|j| j.get("status"))
                        .and_then(Value::as_str)
                        == Some(BackgroundJobStatus::Failed.as_str())
                        && (self.pending_verification_jobs.remove(jid)
                            || is_verification_command(cmd_in_meta))
                    {
                        self.verification_ran = false;
                        self.last_verification_failed = true;
                    }
                }
                _ => {}
            }
        }

        self.total_tool_calls = self.total_tool_calls.saturating_add(1);
        match is_error {
            true => {
                self.consecutive_errors = self.consecutive_errors.saturating_add(1);
                self.total_tool_errors = self.total_tool_errors.saturating_add(1);
            }
            false => self.consecutive_errors = 0,
        }

        let args_hash = hash_canonical_json(arguments);
        let out_hash = match is_error {
            true => hash_normalized_error(&compact_preview(output, 180)),
            false => fingerprint_u64(&compact_preview(output, 180)),
        };

        if self.window.len() >= ORCH_LOOP_WINDOW_SIZE {
            self.window.pop_front();
        }
        self.window.push_back(CallFingerprint {
            tool_name: tool_name.to_string(),
            args_hash,
            out_hash,
            is_error,
        });

        // 1. Repeated identical failing call detection (canonical JSON + digit-normalized stderr)
        if is_error {
            let same_failures = self
                .window
                .iter()
                .filter(|fp| {
                    fp.is_error
                        && fp.tool_name == tool_name
                        && fp.args_hash == args_hash
                        && fp.out_hash == out_hash
                })
                .count();
            if same_failures >= ORCH_LOOP_REPEAT_THRESHOLD {
                self.window.clear();
                let reason = format!(
                    "Repeated identical failing `{tool_name}` call ({same_failures}x)"
                );
                let hint = format!(
                    "<orchestrator_hint type=\"circuit_breaker\">\n\
                     Loop detector triggered: {reason}. Stop repeating the same invocation. \
                     Re-read the target file/error carefully, inspect assumptions, and pivot to a different approach or call `record_brainstorm` / `ask_user` if blocked.\n\
                     </orchestrator_hint>"
                );
                return self.escalate_or_pivot(reason, hint);
            }
        }

        // 2. Non-error spinning detection (identical successful calls yielding identical output)
        if !is_error && category != ToolCategory::Planning {
            let same_noop = self
                .window
                .iter()
                .filter(|fp| {
                    !fp.is_error
                        && fp.tool_name == tool_name
                        && fp.args_hash == args_hash
                        && fp.out_hash == out_hash
                })
                .count();
            if same_noop >= ORCH_NON_ERROR_REPEAT_THRESHOLD {
                self.window.clear();
                let reason = format!(
                    "Non-error spin loop on `{tool_name}` ({same_noop}x identical args & output)"
                );
                let hint = format!(
                    "<orchestrator_hint type=\"spin_loop\">\n\
                     Spin detector triggered: `{tool_name}` was called {same_noop} times with identical arguments and returned identical output. \
                     Use the information already in context to move forward with the next concrete step instead of re-querying.\n\
                     </orchestrator_hint>"
                );
                return self.escalate_or_pivot(reason, hint);
            }
        }

        // 3. Oscillating file-edit detection (same file mutated repeatedly with errors or thrashing)
        if MUTATING_FILE_TOOLS.contains(&tool_name)
            && let Some(path) = arguments.get("path").and_then(Value::as_str) {
                let norm = path.trim().trim_start_matches("./");
                let edit_err_repeats = self
                    .window
                    .iter()
                    .filter(|fp| {
                        MUTATING_FILE_TOOLS.contains(&fp.tool_name.as_str()) && fp.is_error
                    })
                    .count();
                let total_edits = self.file_edit_counts.get(norm).copied().unwrap_or(0);
                if edit_err_repeats >= ORCH_OSCILLATION_THRESHOLD
                    || (total_edits >= ORCH_OSCILLATION_THRESHOLD + 2 && !self.verification_ran)
                {
                    self.file_edit_counts.remove(norm);
                    let reason = format!("Repeated edit thrashing on `{norm}`");
                    let hint = format!(
                        "<orchestrator_hint type=\"edit_oscillation\">\n\
                         Edit oscillation detected on `{norm}`. Re-read the relevant section cleanly with `read_file`, verify hashes/structure, and plan a single clean edit before modifying again.\n\
                         </orchestrator_hint>"
                    );
                    return self.escalate_or_pivot(reason, hint);
                }
            }

        // 4. Consecutive error streak & ZPowers 3-Strike Systematic Debugging Breaker
        let zpowers_debug_enforced =
            matches!(self.zpowers_mode, ZPowersMode::Guided | ZPowersMode::Strict);
        let strike_threshold = match zpowers_debug_enforced {
            false => ORCH_MAX_CONSECUTIVE_ERRORS,
            true => self.debug_strike_limit.max(1).min(ORCH_MAX_CONSECUTIVE_ERRORS),
        };
        if self.consecutive_errors >= strike_threshold {
            let count = self.consecutive_errors;
            self.consecutive_errors = 0;
            let zp_dir = self
                .active_session_id
                .as_deref()
                .map(crate::zpowers::workspace_rel_zpowers_dir)
                .unwrap_or_else(|| ".zenko/sessions/<id>/zpowers".to_string());
            let (reason, hint) = match zpowers_debug_enforced {
                false => (
                    format!("{count} consecutive failing tool calls"),
                    format!(
                        "<orchestrator_hint type=\"error_streak\">\n\
                         Circuit breaker triggered after {count} consecutive tool errors. Pause mutations, summarize what failed, inspect ground truth with read-only tools, and pivot strategy.\n\
                         </orchestrator_hint>"
                    ),
                ),
                true => {
                    self.zpowers_debug_triggered = true;
                    (
                        format!(
                            "ZPowers {count}-strike systematic debugging breaker triggered"
                        ),
                        format!(
                            "<orchestrator_hint type=\"zpowers_debug_breaker\">\n\
                             ZPowers 3-Strike Systematic Debugging Breaker ({count} consecutive failures): STOP patching symptoms immediately.\n\
                             1. Read the full error/stack trace and inspect ground-truth code with read-only tools.\n\
                             2. Log one falsifiable root-cause hypothesis via `invoke_tool(tool=\"{TOOL_ZPOWERS}\", args={{\"action\":\"log_debug\",...}})` (recorded in `{zp_dir}/debug.md`).\n\
                             3. Question the underlying architecture/assumptions before making another edit, or consult the user via `ask_user` if genuinely blocked.\n\
                             </orchestrator_hint>"
                        ),
                    )
                }
            };
            return self.escalate_or_pivot(reason, hint);
        }

        // 5. Empirical P90 Turn-Envelope + Arg-Varying Oscillation Guard (bounded to 1 redirect/turn)
        if self.p90_envelope_calls > 0
            && !self.p90_redirect_sent
            && self.total_tool_calls > self.p90_envelope_calls
            && is_error
        {
            let err_density =
                (self.total_tool_errors as f64) / (self.total_tool_calls.max(1) as f64);
            let same_tool_errors = self
                .window
                .iter()
                .filter(|fp| fp.is_error && fp.tool_name == tool_name)
                .count();
            if err_density >= zenko_core::ECONOMY_LOOP_ERROR_DENSITY_RATIO
                || same_tool_errors >= zenko_core::ECONOMY_LOOP_OSCILLATION_MIN_ERRORS
            {
                self.p90_redirect_sent = true;
                let reason = format!(
                    "P90 turn envelope exceeded ({}/{} calls, {:.0}% errors; {} recent `{tool_name}` failures)",
                    self.total_tool_calls,
                    self.p90_envelope_calls,
                    err_density * 100.0,
                    same_tool_errors
                );
                let hint = format!(
                    "<orchestrator_hint type=\"p90_loop_guard\">\n\
                     Adaptive loop guard triggered: {reason}. This task has exceeded this repository's historical P90 call budget while hitting recurring errors.\n\
                     Stop incremental trial-and-error immediately: step back, inspect root-cause ground truth cleanly, change strategy, or summarize blockers for the user.\n\
                     </orchestrator_hint>"
                );
                return self.escalate_or_pivot(reason, hint);
            }
        }

        GuardVerdict::Continue
    }

    /// Purely objective iteration-budget awareness: notifies the LLM ONCE when only a few
    /// iterations remain before wrap-up, without ever nagging or cutting off research/brainstorming.
    pub fn check_iteration_pacing(
        &mut self,
        enabled: bool,
        iter_idx: usize,
        max_iters: usize,
        wave_had_plan_or_mutation: bool,
    ) -> Option<(String, String)> {
        if !enabled {
            return None;
        }
        match wave_had_plan_or_mutation {
            true => self.read_probe_streak = 0,
            false => self.read_probe_streak = self.read_probe_streak.saturating_add(1),
        }

        let remaining = max_iters.saturating_sub(iter_idx + 1);
        if remaining > 0 && remaining <= ORCH_ITER_WRAPUP_REMAINING && !self.wrapup_nudge_sent {
            self.wrapup_nudge_sent = true;
            let detail = format!("Iteration budget low ({remaining} remaining)");
            let hint = format!(
                "<orchestrator_hint type=\"pacing\">\n\
                 Heads-up: {remaining} tool iteration(s) remain in this turn's budget. Wrap up any pending steps or synthesize your response cleanly.\n\
                 </orchestrator_hint>"
            );
            return Some((detail, hint));
        }
        None
    }

    /// Single-shot, non-blocking advisory completion check:
    /// - Never forces brainstorming or nags on read-only turns.
    /// - Offers at most ONE concise advisory note if files were mutated without any verification
    ///   check yet, or if the LLM's own active DAG plan still has unfinished steps (resets only
    ///   when the LLM makes genuine forward DAG progress, capped by `ORCH_MAX_TOTAL_CONTINUATION_NUDGES`).
    pub fn check_completion_gate(&mut self, enabled: bool) -> Option<String> {
        if !enabled || self.intent.is_read_only() {
            self.phase = OrchestratorPhase::Done;
            return None;
        }

        let iron_law_enforced = matches!(self.zpowers_mode, ZPowersMode::Guided | ZPowersMode::Strict);
        let needs_verify = !self.mutated_files.is_empty()
            && !self.verification_ran
            && self.verify_nudges == 0;
        let needs_strict_handoff = self.zpowers_mode == ZPowersMode::Strict
            && self.mutated_files.len() >= ZPOWERS_STRICT_HANDOFF_MIN_FILES
            && self.verification_ran
            && !self.walkthrough_recorded
            && self.verify_nudges < 2;
        let unfinished_plan: Vec<String> = self
            .active_plan
            .as_ref()
            .map(|p| {
                incomplete_steps(p)
                    .into_iter()
                    .map(|s| format!("`{}: {}` ({})", s.id, s.title, s.status.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        let local_plan_nudge_cap = match iron_law_enforced {
            true => ORCH_MAX_PLAN_CONTINUATION_NUDGES,
            false => ORCH_MAX_PLAN_CONTINUATION_NUDGES.min(1),
        };
        let needs_plan_continue = !unfinished_plan.is_empty()
            && self.plan_nudges < local_plan_nudge_cap
            && self.total_continuation_nudges < ORCH_MAX_TOTAL_CONTINUATION_NUDGES;

        let fail_note = match self.last_verification_failed {
            true => " Your last verification command FAILED -- fix the root cause and re-run verification before claiming completion.",
            false => "",
        };

        match (needs_verify, needs_plan_continue, needs_strict_handoff) {
            (false, false, false) => {
                self.phase = OrchestratorPhase::Done;
                None
            }
            (false, false, true) => {
                self.verify_nudges += 1;
                Some(format!(
                    "<orchestrator_hint type=\"zpowers_strict_handoff\">\n\
                     ZPowers Strict Gate: {} files were modified and verified. Before concluding, run independent two-stage review (`{TOOL_ZPOWERS}(action=\"run_review\")` or `record_review`) and write the final handoff (`{TOOL_ZPOWERS}(action=\"write_walkthrough\")`).\n\
                     </orchestrator_hint>",
                    self.mutated_files.len()
                ))
            }
            (true, false, _) => {
                self.verify_nudges += 1;
                let files: Vec<&str> = self.mutated_files.iter().map(String::as_str).collect();
                Some(match iron_law_enforced {
                    true => format!(
                        "<orchestrator_hint type=\"zpowers_iron_law\">\n\
                         ZPowers Verification Iron Law: {} file(s) were modified ({}) with NO fresh passing verification run after your last edit.{fail_note} \
                         Run the relevant build, typecheck, or test command directly (without piping through `| tail` or `| head`) and confirm a passing exit code before claiming completion.\n\
                         </orchestrator_hint>",
                        files.len(),
                        files.join(", ")
                    ),
                    false => format!(
                        "<orchestrator_hint type=\"advisory_check\">\n\
                         Advisory: {} file(s) were modified ({}) with no passing check recorded yet.{fail_note} \
                         Run a relevant verification check if appropriate, or respond directly if verification is unnecessary.\n\
                         </orchestrator_hint>",
                        files.len(),
                        files.join(", ")
                    ),
                })
            }
            (false, true, _) => {
                self.plan_nudges += 1;
                self.total_continuation_nudges += 1;
                Some(format!(
                    "<orchestrator_hint type=\"advisory_check\">\n\
                     Active plan still lists {} open step(s): {}. \
                     Continue executing or update/close them via `plan_update`/`session_task` before concluding.\n\
                     </orchestrator_hint>",
                    unfinished_plan.len(),
                    unfinished_plan.join(", ")
                ))
            }
            (true, true, _) => {
                self.verify_nudges += 1;
                self.plan_nudges += 1;
                self.total_continuation_nudges += 1;
                let files: Vec<&str> = self.mutated_files.iter().map(String::as_str).collect();
                Some(format!(
                    "<orchestrator_hint type=\"zpowers_iron_law\">\n\
                     ZPowers Gate: (1) modified file(s) ({}) have no fresh passing post-edit verification check yet{fail_note}, and \
                     (2) active plan still has {} open step(s) ({}). Verify your edits and update/close the plan steps before concluding.\n\
                     </orchestrator_hint>",
                    files.join(", "),
                    unfinished_plan.len(),
                    unfinished_plan.join(", ")
                ))
            }
        }
    }

    pub fn pivot_count(&self) -> usize {
        self.pivot_count
    }

    pub fn mutated_files_count(&self) -> usize {
        self.mutated_files.len()
    }

    /// Detects silent Todo-DAG drift during execution: if the agent mutates workspace files across
    /// `TODO_DRIFT_MUTATION_ITERS` consecutive waves while open todos exist and zero task updates
    /// were recorded, emits one bounded cache-safe `<active_task_dag>` reminder so todos stay live.
    pub fn check_todo_drift(
        &mut self,
        wave_mutated: bool,
        wave_updated_tasks: bool,
        tasks: &[zenko_core::SessionTask],
    ) -> Option<String> {
        if wave_updated_tasks {
            self.mutating_iters_since_task_update = 0;
            return None;
        }
        if !wave_mutated {
            return None;
        }
        let open_count = tasks
            .iter()
            .filter(|t| t.status != zenko_core::SessionTaskStatus::Done)
            .count();
        if open_count == 0 {
            self.mutating_iters_since_task_update = 0;
            return None;
        }
        self.mutating_iters_since_task_update =
            self.mutating_iters_since_task_update.saturating_add(1);
        if self.mutating_iters_since_task_update < zenko_core::TODO_DRIFT_MUTATION_ITERS
            || self.todo_drift_nudges >= zenko_core::TODO_DRIFT_MAX_NUDGES
        {
            return None;
        }
        self.mutating_iters_since_task_update = 0;
        self.todo_drift_nudges = self.todo_drift_nudges.saturating_add(1);
        let dag = super::plan::format_session_tasks_prompt_block(tasks)?;
        Some(format!(
            "<orchestrator_hint type=\"todo_sync\">\n\
             Live Todo Check: you have mutated files across multiple iterations without updating your tracked todos ({open_count}/{} still open). \
             Batch-mark completed steps `done` and the active step `in_progress` in ONE call via `session_task(action=\"batch\", tasks=[...])` alongside your next tool call:\n\
             {dag}\n\
             </orchestrator_hint>",
            tasks.len()
        ))
    }
}

/// Zero-regex exact line-hash doom-loop detector for degenerate repetitive LLM prose/thinking.
/// Skips fenced code blocks and Markdown table rows, and preserves digits so numbered tables
/// and structured test output never trigger false positives.
pub fn detect_text_doom_loop(text: &str) -> Option<String> {
    let mut in_fence = false;
    let mut prose_lines: Vec<String> = Vec::new();
    for raw in text.lines() {
        let trimmed = raw.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || (trimmed.starts_with('|') && trimmed.ends_with('|')) {
            continue;
        }
        if trimmed.chars().count() >= ORCH_TEXT_DOOM_MIN_LINE_CHARS {
            prose_lines.push(trimmed.to_ascii_lowercase());
        }
    }
    if prose_lines.len() < ORCH_TEXT_DOOM_MIN_LINES {
        return None;
    }
    let mut counts: HashMap<u64, usize> = HashMap::with_capacity(prose_lines.len());
    for line in &prose_lines {
        let h = fingerprint_u64(line);
        let c = counts.entry(h).or_insert(0);
        *c += 1;
        if *c >= ORCH_TEXT_DOOM_REPEAT_THRESHOLD {
            return Some(format!(
                "Degenerate text/reasoning repetition detected ({c}x identical line)"
            ));
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellChainOp {
    None,
    And,
    Or,
    Pipe,
    Semi,
}

/// Parses a heredoc delimiter specification right after `<<` (rejecting bash `<<<` here-strings).
fn parse_heredoc_spec(bytes: &[u8], start: usize) -> Option<(String, bool, usize)> {
    if start < bytes.len() && bytes[start] == b'<' {
        return None;
    }
    let mut pos = start;
    let strip_tabs = match pos < bytes.len() && bytes[pos] == b'-' {
        true => {
            pos += 1;
            true
        }
        false => false,
    };
    while pos < bytes.len() && matches!(bytes[pos], b' ' | b'\t') {
        pos += 1;
    }
    let mut delim_bytes: Vec<u8> = Vec::new();
    let mut q_single = false;
    let mut q_double = false;
    while pos < bytes.len() {
        let c = bytes[pos];
        match (q_single, q_double, c) {
            (false, false, b'\'') => {
                q_single = true;
                pos += 1;
            }
            (true, false, b'\'') => {
                q_single = false;
                pos += 1;
            }
            (false, false, b'"') => {
                q_double = true;
                pos += 1;
            }
            (false, true, b'"') => {
                q_double = false;
                pos += 1;
            }
            (false, _, b'\\') if pos + 1 < bytes.len() => {
                delim_bytes.push(bytes[pos + 1]);
                pos += 2;
            }
            (
                false,
                false,
                b' ' | b'\t' | b'\n' | b'\r' | b';' | b'|' | b'&' | b'<' | b'>' | b'(' | b')',
            ) => break,
            _ => {
                delim_bytes.push(c);
                pos += 1;
            }
        }
    }
    let delim = String::from_utf8_lossy(&delim_bytes).into_owned();
    match delim.is_empty() {
        true => None,
        false => Some((delim, strip_tabs, pos)),
    }
}

/// Quote- and heredoc-aware zero-regex scanner splitting shell commands into
/// `(segment, following_operator)` pairs while consuming heredoc bodies without
/// misclassifying embedded newlines as `;` exit-masking chains.
fn split_shell_segments(cmd: &str) -> Vec<(String, ShellChainOp)> {
    let bytes = cmd.as_bytes();
    let mut out = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut pending_heredocs: VecDeque<(String, bool)> = VecDeque::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        match (in_single, in_double, b) {
            (false, false, b'\'') => {
                in_single = true;
                cur.push(b'\'');
                i += 1;
            }
            (true, false, b'\'') => {
                in_single = false;
                cur.push(b'\'');
                i += 1;
            }
            (false, false, b'"') => {
                in_double = true;
                cur.push(b'"');
                i += 1;
            }
            (false, true, b'"') => {
                in_double = false;
                cur.push(b'"');
                i += 1;
            }
            (false, _, b'\\') if i + 1 < bytes.len() => {
                cur.push(b);
                cur.push(bytes[i + 1]);
                i += 2;
            }
            (false, false, b'<') if i + 1 < bytes.len() && bytes[i + 1] == b'<' => {
                if let Some((delim, strip_tabs, end_pos)) = parse_heredoc_spec(bytes, i + 2) {
                    cur.extend_from_slice(&bytes[i..end_pos]);
                    pending_heredocs.push_back((delim, strip_tabs));
                    i = end_pos;
                } else {
                    cur.push(b);
                    i += 1;
                }
            }
            (false, false, b'&') if i + 1 < bytes.len() && bytes[i + 1] == b'&' => {
                out.push((
                    String::from_utf8_lossy(&cur).into_owned(),
                    ShellChainOp::And,
                ));
                cur.clear();
                i += 2;
            }
            (false, false, b'&') => {
                // Keep fd redirections (`2>&1`, `>&2`, `<&0`) intact; split background separator `&`!
                if matches!(cur.last(), Some(b'>' | b'<')) {
                    cur.push(b'&');
                } else {
                    out.push((
                        String::from_utf8_lossy(&cur).into_owned(),
                        ShellChainOp::Semi,
                    ));
                    cur.clear();
                }
                i += 1;
            }
            (false, false, b'|') if i + 1 < bytes.len() && bytes[i + 1] == b'|' => {
                out.push((
                    String::from_utf8_lossy(&cur).into_owned(),
                    ShellChainOp::Or,
                ));
                cur.clear();
                i += 2;
            }
            (false, false, b'|') => {
                out.push((
                    String::from_utf8_lossy(&cur).into_owned(),
                    ShellChainOp::Pipe,
                ));
                cur.clear();
                i += 1;
            }
            (false, false, b';') => {
                out.push((
                    String::from_utf8_lossy(&cur).into_owned(),
                    ShellChainOp::Semi,
                ));
                cur.clear();
                i += 1;
            }
            (false, false, b'\r') if pending_heredocs.is_empty() => {
                out.push((
                    String::from_utf8_lossy(&cur).into_owned(),
                    ShellChainOp::Semi,
                ));
                cur.clear();
                i += 1;
            }
            (false, false, b'\n') => {
                i += 1;
                if !pending_heredocs.is_empty() {
                    while let Some((delim, strip_tabs)) = pending_heredocs.pop_front() {
                        while i < bytes.len() {
                            let line_start = i;
                            while i < bytes.len() && bytes[i] != b'\n' {
                                i += 1;
                            }
                            let line_end = i;
                            if i < bytes.len() && bytes[i] == b'\n' {
                                i += 1;
                            }
                            let raw_line = String::from_utf8_lossy(&bytes[line_start..line_end]);
                            let trimmed = raw_line.trim_end_matches('\r');
                            let candidate = match strip_tabs {
                                true => trimmed.trim_start_matches('\t'),
                                false => trimmed,
                            };
                            if candidate == delim {
                                break;
                            }
                        }
                    }
                    // Only emit `;` separator if another real command follows the closed heredoc
                    if bytes[i..].iter().any(|c| !c.is_ascii_whitespace()) {
                        out.push((
                            String::from_utf8_lossy(&cur).into_owned(),
                            ShellChainOp::Semi,
                        ));
                        cur.clear();
                    }
                } else {
                    out.push((
                        String::from_utf8_lossy(&cur).into_owned(),
                        ShellChainOp::Semi,
                    ));
                    cur.clear();
                }
            }
            _ => {
                cur.push(b);
                i += 1;
            }
        }
    }
    let tail = String::from_utf8_lossy(&cur).into_owned();
    if !tail.trim().is_empty() {
        out.push((tail, ShellChainOp::None));
    }
    out
}

fn is_shell_boundary(bytes: &[u8], idx: usize) -> bool {
    idx >= bytes.len()
        || matches!(
            bytes[idx],
            b' ' | b'\t' | b'\n' | b'\r' | b';' | b'|' | b'&' | b')'
        )
}

/// Quote-aware zero-regex check for shell redirection (`>`), process substitution (`<(` / `>(`),
/// or command substitution (`$(...)` / backticks) that could mutate state on read-only turns.
/// Permits strictly benign stderr discards (`2>/dev/null`) and stream merges (`2>&1`) on token boundaries.
fn has_shell_mutation_syntax(cmd: &str) -> bool {
    let bytes = cmd.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        match (in_single, in_double, b) {
            (false, false, b'\'') => {
                in_single = true;
                i += 1;
            }
            (true, false, b'\'') => {
                in_single = false;
                i += 1;
            }
            (false, false, b'"') => {
                in_double = true;
                i += 1;
            }
            (false, true, b'"') => {
                in_double = false;
                i += 1;
            }
            (false, _, b'\\') if i + 1 < bytes.len() => {
                i += 2;
            }
            (false, false, b'2')
                if (i == 0 || is_shell_boundary(bytes, i - 1))
                    && bytes[i..].starts_with(b"2>/dev/null")
                    && is_shell_boundary(bytes, i + 11) =>
            {
                i += 11;
            }
            (false, false, b'2')
                if (i == 0 || is_shell_boundary(bytes, i - 1))
                    && bytes[i..].starts_with(b"2>&1")
                    && is_shell_boundary(bytes, i + 4) =>
            {
                i += 4;
            }
            (false, false, b'>') => return true,
            (false, false, b'<') if i + 1 < bytes.len() && bytes[i + 1] == b'(' => return true,
            (false, _, b'$') if i + 1 < bytes.len() && bytes[i + 1] == b'(' => return true,
            (false, _, b'`') => return true,
            _ => i += 1,
        }
    }
    false
}

fn segment_is_read_only_shell(segment: &str) -> bool {
    let raw_tokens: Vec<&str> = segment.split_whitespace().collect();
    if raw_tokens.is_empty() {
        return false;
    }
    let lower_tokens: Vec<String> = raw_tokens
        .iter()
        .take(4)
        .map(|t| t.to_ascii_lowercase())
        .collect();
    let matches_prefix = READ_ONLY_SHELL_CMD_PREFIXES.iter().any(|prefix| {
        lower_tokens.len() >= prefix.len()
            && lower_tokens[..prefix.len()]
                .iter()
                .zip(*prefix)
                .all(|(a, b)| a.as_str() == *b)
    });
    if !matches_prefix {
        return false;
    }
    if lower_tokens.first().map(String::as_str) == Some("find")
        && raw_tokens.iter().any(|tok| {
            READ_ONLY_FIND_MUTATING_FLAGS
                .iter()
                .any(|flag| tok.eq_ignore_ascii_case(flag))
        })
    {
        return false;
    }
    true
}

/// Deterministic zero-regex allowlist verifying that EVERY segment of a shell command is strictly
/// read-only with zero redirection or command/process substitution (enforces Phase-0 Intent Gate).
pub fn is_read_only_shell_command(cmd: &str) -> bool {
    if has_shell_mutation_syntax(cmd) {
        return false;
    }
    let segments = split_shell_segments(cmd);
    !segments.is_empty()
        && segments
            .iter()
            .all(|(seg, _)| segment_is_read_only_shell(seg))
}

fn segment_matches_verification(segment: &str) -> bool {
    let raw_tokens: Vec<&str> = segment.split_whitespace().collect();
    if raw_tokens.is_empty() {
        return false;
    }
    // Reject case-sensitive (`-V`, `-h`) and case-insensitive (`--version`, `--help`, `help`) spoofs
    // while explicitly allowing lowercase `-v` (verbose in `pytest -v`, `cargo test -v`, `go test -v`).
    let is_spoof = raw_tokens.iter().any(|tok| {
        VERIFICATION_SPOOF_FLAGS_EXACT.contains(tok)
            || VERIFICATION_SPOOF_FLAGS_CI
                .iter()
                .any(|f| tok.eq_ignore_ascii_case(f))
    });
    if is_spoof {
        return false;
    }
    let lower_tokens: Vec<String> = raw_tokens
        .iter()
        .take(4)
        .map(|t| t.to_ascii_lowercase())
        .collect();
    VERIFICATION_CMD_PREFIXES.iter().any(|prefix| {
        lower_tokens.len() >= prefix.len()
            && lower_tokens[..prefix.len()]
                .iter()
                .zip(*prefix)
                .all(|(a, b)| a.as_str() == *b)
    })
}

/// Deterministic zero-regex detector for genuine build/test/lint verification shell commands.
/// - Rejects version/help spoofs (`--version`, `-V`, `--help`, `-h`, `help`) while allowing `-v` (verbose).
/// - Rejects exit-status masking chains (`cargo test || true`, `cargo test ; echo ok`, `cargo test | tail`).
/// - Allows strict short-circuit `&&` chains (`cd webui && npm run build`, `cargo check && cargo test`).
pub fn is_verification_command(cmd: &str) -> bool {
    let segments = split_shell_segments(cmd);
    let mut matched_real_verifier = false;

    for (seg, next_op) in segments {
        if !segment_matches_verification(&seg) {
            continue;
        }
        // Only count a verification segment if its failure CANNOT be masked by `||`, `;`, or `|`
        match next_op {
            ShellChainOp::None | ShellChainOp::And => {
                matched_real_verifier = true;
            }
            ShellChainOp::Or | ShellChainOp::Pipe | ShellChainOp::Semi => {
                return false;
            }
        }
    }

    matched_real_verifier
}

/// Canonical JSON 64-bit FNV-1a hash independent of object key insertion order.
pub fn hash_canonical_json(val: &Value) -> u64 {
    let mut h = FNV1A_OFFSET_BASIS;
    feed_canonical_json(&mut h, val);
    h
}

fn feed_bytes(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *h ^= u64::from(b);
        *h = h.wrapping_mul(FNV1A_PRIME);
    }
}

fn feed_canonical_json(h: &mut u64, val: &Value) {
    match val {
        Value::Null => feed_bytes(h, b"N"),
        Value::Bool(b) => feed_bytes(h, match b { true => b"T", false => b"F" }),
        Value::Number(n) => {
            feed_bytes(h, b"#");
            feed_bytes(h, n.to_string().as_bytes());
            feed_bytes(h, b";");
        }
        Value::String(s) => {
            feed_bytes(h, b"S");
            feed_bytes(h, s.as_bytes());
            feed_bytes(h, b"\0");
        }
        Value::Array(arr) => {
            feed_bytes(h, b"[");
            for item in arr {
                feed_canonical_json(h, item);
                feed_bytes(h, b",");
            }
            feed_bytes(h, b"]");
        }
        Value::Object(map) => {
            let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
            keys.sort_unstable();
            feed_bytes(h, b"{");
            for k in keys {
                feed_bytes(h, k.as_bytes());
                feed_bytes(h, b":");
                if let Some(v) = map.get(k) {
                    feed_canonical_json(h, v);
                }
                feed_bytes(h, b",");
            }
            feed_bytes(h, b"}");
        }
    }
}

/// Digit-normalized FNV-1a hash collapsing volatile numbers (PIDs, timestamps, line numbers).
fn hash_normalized_error(input: &str) -> u64 {
    let mut h = FNV1A_OFFSET_BASIS;
    let mut in_digits = false;
    for b in input.bytes() {
        match b.is_ascii_digit() {
            true => {
                if !in_digits {
                    feed_bytes(&mut h, b"#");
                    in_digits = true;
                }
            }
            false => {
                in_digits = false;
                feed_bytes(&mut h, &[b.to_ascii_lowercase()]);
            }
        }
    }
    h
}

fn fingerprint_u64(input: &str) -> u64 {
    let mut hash = FNV1A_OFFSET_BASIS;
    feed_bytes(&mut hash, input.as_bytes());
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;
    use zenko_core::{PlanStep, PlanStepStatus};

    #[test]
    fn canonical_json_and_normalized_error_catch_reordered_loops_and_reset_after_escalation() {
        let mut guard = ExecutionGuard::new(
            ExecutionTier::Orchestrated,
            IntentKind::Implement,
            DeliberationDepth::Standard,
        );
        let a1 = json!({ "path": "src/main.rs", "start_line": 1 });
        let a2 = json!({ "start_line": 1, "path": "src/main.rs" });
        assert_eq!(hash_canonical_json(&a1), hash_canonical_json(&a2));

        assert_eq!(
            guard.record_tool_outcome("hash_edit", &a1, true, "Hash mismatch at pid 101 line 12", None),
            GuardVerdict::Continue
        );
        assert_eq!(
            guard.record_tool_outcome("hash_edit", &a2, true, "Hash mismatch at pid 202 line 13", None),
            GuardVerdict::Continue
        );
        let third = guard.record_tool_outcome(
            "hash_edit",
            &a1,
            true,
            "Hash mismatch at pid 303 line 14",
            None,
        );
        assert!(matches!(third, GuardVerdict::Pivot { .. }));
        assert_eq!(guard.pivot_count(), 1);

        // Second trip escalates cleanly
        for _ in 0..2 {
            assert_eq!(
                guard.record_tool_outcome("read_file", &json!({"path": "x.rs"}), false, "same content", None),
                GuardVerdict::Continue
            );
        }
        let spin_esc = guard.record_tool_outcome(
            "read_file",
            &json!({"path": "x.rs"}),
            false,
            "same content",
            None,
        );
        assert!(matches!(spin_esc, GuardVerdict::Escalate { .. }));

        // Reset after user approval restores a clean pivot budget
        guard.reset_after_escalation();
        assert_eq!(guard.pivot_count(), 0);
    }

    #[test]
    fn p90_turn_envelope_triggers_single_bounded_redirect_on_arg_varying_oscillation() {
        let mut guard = ExecutionGuard::new(
            ExecutionTier::Standard,
            IntentKind::Implement,
            DeliberationDepth::Standard,
        );
        guard.set_p90_envelope(4);
        // 2 good reads + 2 distinct failing shell calls (at budget=4 -> Continue)
        assert_eq!(
            guard.record_tool_outcome("read_file", &json!({"path": "a.rs"}), false, "ok", None),
            GuardVerdict::Continue
        );
        assert_eq!(
            guard.record_tool_outcome("read_file", &json!({"path": "b.rs"}), false, "ok", None),
            GuardVerdict::Continue
        );
        assert_eq!(
            guard.record_tool_outcome("shell_exec", &json!({"command": "cmd_1"}), true, "err 1", None),
            GuardVerdict::Continue
        );
        assert_eq!(
            guard.record_tool_outcome("shell_exec", &json!({"command": "cmd_2"}), true, "err 2", None),
            GuardVerdict::Continue
        );
        // 5th call breaches P90 envelope (5 > 4) with 3rd shell error -> fires single bounded redirect!
        let fifth = guard.record_tool_outcome(
            "shell_exec",
            &json!({"command": "cmd_3"}),
            true,
            "err 3",
            None,
        );
        assert!(
            matches!(fifth, GuardVerdict::Pivot { ref hint, .. } if hint.contains("p90_loop_guard")),
            "expected p90_loop_guard pivot, got {fifth:?}"
        );
        assert!(guard.p90_redirect_triggered());
    }

    #[test]
    fn sovereign_guard_allows_all_llm_tools_while_keeping_shell_classifier_and_wrapup_intact() {
        // LLM-sovereign guard NEVER keyword-blocks writes or interrupts exploration
        let mut guard = ExecutionGuard::new(
            ExecutionTier::Orchestrated,
            IntentKind::Implement,
            DeliberationDepth::Standard,
        );
        assert_eq!(
            guard.check_pre_tool_phase_gate(true, "write_file", &json!({"path": "src/a.rs"})),
            PhaseGateDecision::Allow
        );
        assert_eq!(
            guard.check_pre_tool_phase_gate(true, "shell_exec", &json!({"command": "cargo test"})),
            PhaseGateDecision::Allow
        );

        // Read-only shell classifier (used by ExecPolicy in explicit Plan Mode) remains strict
        for unsafe_cmd in [
            "sed -i 's/a/b/g' src/app.rs",
            "echo x > src/app.rs",
            "echo x 2>/dev/null > src/app.rs",
            "ls 2>/dev/null_evil",
            "ls 2> err.log",
            "cat src/app.rs | tee copy.rs",
            "ls && rm -f src/app.rs",
            "ls \"$(touch pwned)\"",
            "find . -name '*.rs' -delete",
        ] {
            assert!(
                !is_read_only_shell_command(unsafe_cmd),
                "expected `{unsafe_cmd}` to be classified as non-read-only"
            );
        }
        for safe_cmd in [
            "git status",
            "rg -i \"foo > bar\" src/ | head -n 20",
            "ls -la && git diff --stat",
            "ls -la target/debug/zenko 2>/dev/null || ls target 2>/dev/null || echo \"no target dir\"",
            "cd crates && ls -la 2>&1 | head -n 5",
            "find src -name '*.rs'",
        ] {
            assert!(
                is_read_only_shell_command(safe_cmd),
                "expected `{safe_cmd}` to be classified as read-only"
            );
        }

        // Exploration never gets nagged mid-research; only near-end wrap-up notice fires
        for i in 0..10 {
            assert!(guard.check_iteration_pacing(true, i, 24, false).is_none());
        }
        let wrap = guard.check_iteration_pacing(true, 20, 24, false);
        assert!(matches!(wrap, Some((_, ref h)) if h.contains("3 tool iteration")));

        // Text doom-loop pivot counter increments cleanly so retries stay bounded
        assert_eq!(guard.pivot_count(), 0);
        guard.record_text_doom_pivot();
        assert_eq!(guard.pivot_count(), 1);

        // Advisory turns NEVER get nagged to call record_brainstorm
        let mut advisory = ExecutionGuard::new(
            ExecutionTier::Orchestrated,
            IntentKind::Advisory,
            DeliberationDepth::Standard,
        );
        assert!(advisory.check_completion_gate(true).is_none());
    }

    #[test]
    fn verification_detector_allows_verbose_and_heredocs_while_rejecting_spoofs_and_chaining() {
        // Spoofs and exit-masking chains MUST be rejected
        assert!(!is_verification_command("cargo check --version"));
        assert!(!is_verification_command("cargo check -V"));
        assert!(!is_verification_command("pytest --help"));
        assert!(!is_verification_command("pytest -h"));
        assert!(!is_verification_command("cargo test || true"));
        assert!(!is_verification_command("cargo test ; echo ok"));
        assert!(!is_verification_command("cargo test | tail -n 10"));
        assert!(!is_verification_command(
            "rustc --crate-type lib - -o /tmp/a.rlib <<'EOF'\npub fn ok() {}\nEOF\necho masked"
        ));

        // Verbose `-v`, strict `&&` chains, and clean heredocs MUST be accepted
        assert!(is_verification_command("pytest -v"));
        assert!(is_verification_command("cargo test -v --workspace"));
        assert!(is_verification_command("go test -v ./..."));
        assert!(is_verification_command("cd webui && npm run build"));
        assert!(is_verification_command("cargo pretty check --workspace && cargo pretty test"));
        assert!(is_verification_command(
            "rustc --crate-type lib - -o /tmp/a.rlib <<'EOF'\n// help comment inside heredoc\npub fn ok() {}\nEOF"
        ));

        // Background shell_exec (`status: running`) must NOT mark verified until job_wait completes with exit_code=0!
        let mut guard = ExecutionGuard::new(
            ExecutionTier::Orchestrated,
            IntentKind::Implement,
            DeliberationDepth::Deliberate,
        );
        guard.record_tool_outcome("write_file", &json!({"path": "src/a.rs"}), false, "ok", None);
        guard.record_tool_outcome(
            "shell_exec",
            &json!({"command": "cargo pretty test --workspace", "run_in_background": true}),
            false,
            "Started background task",
            Some(&json!({"id": "task_123", "status": "running"})),
        );
        // Timed-out job_wait (`status: running, exit_code: null`) must NOT mark verified!
        guard.record_tool_outcome(
            "job_wait",
            &json!({"job_id": "task_123"}),
            false,
            "still running",
            Some(&json!({"job": {"id": "task_123", "status": "running", "exit_code": null, "command": "cargo pretty test --workspace"}})),
        );
        assert!(!guard.verification_ran, "running bg job must not pass verification");

        // Final completed job_wait with exit_code=0 DOES satisfy verification!
        guard.record_tool_outcome(
            "job_wait",
            &json!({"job_id": "task_123"}),
            false,
            "ok",
            Some(&json!({"job": {"id": "task_123", "status": "completed", "exit_code": 0, "command": "cargo pretty test --workspace"}})),
        );
        assert!(guard.check_completion_gate(true).is_none());
    }

    #[test]
    fn text_doom_loop_ignores_markdown_tables_and_code_blocks_while_catching_true_loops() {
        let valid_table = "\
            Summary of checks:\n\
            | Step 1 | cargo check | passed in 120ms |\n\
            | Step 2 | cargo test  | passed in 240ms |\n\
            | Step 3 | npm build   | passed in 310ms |\n\
            | Step 4 | live e2e    | passed in 450ms |\n\
            | Step 5 | clippy lint | passed in 180ms |\n\
            | Step 6 | trace check | passed in 90ms |\n\
            ```rust\n\
            assert_eq!(items[0], expected[0]);\n\
            assert_eq!(items[1], expected[1]);\n\
            assert_eq!(items[2], expected[2]);\n\
            assert_eq!(items[3], expected[3]);\n\
            assert_eq!(items[4], expected[4]);\n\
            assert_eq!(items[5], expected[5]);\n\
            ```";
        assert!(detect_text_doom_loop(valid_table).is_none());

        let loopy = "I should check the cargo build output again now.\n".repeat(6);
        assert!(detect_text_doom_loop(&loopy).is_some());
    }

    #[test]
    fn completion_gate_is_single_shot_advisory_and_progress_aware() {
        let mut guard = ExecutionGuard::new(
            ExecutionTier::Orchestrated,
            IntentKind::Implement,
            DeliberationDepth::Standard,
        );
        guard.record_tool_outcome("write_file", &json!({"path": "src/lib.rs"}), false, "ok", None);
        // Single non-blocking advisory reminder when unverified
        let first = guard.check_completion_gate(true).expect("single advisory check");
        assert!(first.contains("advisory_check"));
        // Second call respects LLM decision without nagging repeatedly
        assert!(guard.check_completion_gate(true).is_none());

        // Passing LSP check clears verification state cleanly
        let mut lsp_guard = ExecutionGuard::new(
            ExecutionTier::Orchestrated,
            IntentKind::Implement,
            DeliberationDepth::Standard,
        );
        lsp_guard.record_tool_outcome("write_file", &json!({"path": "src/lib.rs"}), false, "ok", None);
        lsp_guard.record_tool_outcome("lsp_tool", &json!({"action": "diagnostics"}), false, "0 errors", None);
        assert!(lsp_guard.check_completion_gate(true).is_none());

        // Progress-aware plan continuation emits 1 advisory note, unlocks on genuine forward progress
        let mut pguard = ExecutionGuard::new(
            ExecutionTier::Standard,
            IntentKind::Implement,
            DeliberationDepth::Standard,
        );
        pguard.update_active_plan(ExecutionPlan {
            id: "p1".into(),
            goal: "Ship feature".into(),
            steps: vec![
                PlanStep {
                    id: "s1".into(),
                    title: "Read files".into(),
                    status: PlanStepStatus::InProgress,
                    depends_on: vec![],
                    verification: None,
                },
                PlanStep {
                    id: "s2".into(),
                    title: "Write tests".into(),
                    status: PlanStepStatus::Pending,
                    depends_on: vec!["s1".into()],
                    verification: None,
                },
            ],
            rubric_warnings: vec![],
            updated_at: Utc::now(),
        });
        assert!(pguard.check_completion_gate(true).is_some());
        assert!(pguard.check_completion_gate(true).is_none(), "single-shot advisory when no progress");
        pguard.update_active_plan(ExecutionPlan {
            id: "p2".into(),
            goal: "Ship feature".into(),
            steps: vec![
                PlanStep {
                    id: "s1".into(),
                    title: "Read files".into(),
                    status: PlanStepStatus::Completed,
                    depends_on: vec![],
                    verification: None,
                },
                PlanStep {
                    id: "s2".into(),
                    title: "Write tests".into(),
                    status: PlanStepStatus::InProgress,
                    depends_on: vec!["s1".into()],
                    verification: None,
                },
            ],
            rubric_warnings: vec![],
            updated_at: Utc::now(),
        });
        assert!(pguard.check_completion_gate(true).is_some(), "progress unlocks next advisory note");
    }

    #[test]
    fn zpowers_guided_enforces_plan_first_fast_track_3_strike_debug_and_iron_law() {
        let ws = std::env::temp_dir().join("zenko_guard_zp_guided");
        let zp_state = ZPowersState::new("sess_g", ZPowersMode::Guided);
        let mut guard = ExecutionGuard::new(
            ExecutionTier::Orchestrated,
            IntentKind::Implement,
            DeliberationDepth::Standard,
        );
        guard.configure_zpowers(
            ZPowersMode::Guided,
            false,
            3,
            ws.clone(),
            "sess_g".into(),
            &zp_state,
        );

        // Read-only recon and own-session scratch writes are ALWAYS allowed
        assert_eq!(
            guard.check_pre_tool_phase_gate(true, "read_file", &json!({"path": "src/main.rs"})),
            PhaseGateDecision::Allow
        );
        assert_eq!(
            guard.check_pre_tool_phase_gate(
                true,
                "write_file",
                &json!({"path": ".zenko/sessions/sess_g/scratch/notes.md"})
            ),
            PhaseGateDecision::Allow
        );

        // Premature workspace mutation before any plan triggers plan-first redirect
        assert!(matches!(
            guard.check_pre_tool_phase_gate(true, "write_file", &json!({"path": "src/lib.rs"})),
            PhaseGateDecision::Redirect { ref hint, .. } if hint.contains("zpowers_plan_first")
        ));

        // Recording a plan via `zpowers(action="write_plan")` unlocks mutations immediately
        guard.record_tool_outcome(
            TOOL_ZPOWERS,
            &json!({"action": "write_plan", "content": "- [ ] edit src/lib.rs"}),
            false,
            "Saved ZPowers plan",
            None,
        );
        assert_eq!(
            guard.check_pre_tool_phase_gate(true, "write_file", &json!({"path": "src/lib.rs"})),
            PhaseGateDecision::Allow
        );

        // Mutating a file without verification triggers Iron Law completion gate
        guard.record_tool_outcome(
            "write_file",
            &json!({"path": "src/lib.rs"}),
            false,
            "ok",
            None,
        );
        let iron_law = guard
            .check_completion_gate(true)
            .expect("expected Iron Law verification gate");
        assert!(iron_law.contains("zpowers_iron_law"));

        // 3 consecutive failing tool calls trigger the ZPowers 3-strike systematic debugging breaker
        for i in 0..2 {
            assert_eq!(
                guard.record_tool_outcome(
                    "shell_exec",
                    &json!({"command": format!("cargo test case_{i}")}),
                    true,
                    &format!("failed case {i}"),
                    None,
                ),
                GuardVerdict::Continue
            );
        }
        let strike3 = guard.record_tool_outcome(
            "shell_exec",
            &json!({"command": "cargo test case_2"}),
            true,
            "failed case 2",
            None,
        );
        assert!(matches!(
            strike3,
            GuardVerdict::Pivot { ref hint, .. } if hint.contains("zpowers_debug_breaker")
        ));
        assert!(guard.zpowers_debug_triggered());

        // Fast-track bypass unlocks a fresh Guided guard immediately
        let mut ft_guard = ExecutionGuard::new(
            ExecutionTier::Standard,
            IntentKind::Implement,
            DeliberationDepth::Standard,
        );
        ft_guard.configure_zpowers(
            ZPowersMode::Guided,
            false,
            3,
            ws,
            "sess_ft".into(),
            &ZPowersState::new("sess_ft", ZPowersMode::Guided),
        );
        ft_guard.record_tool_outcome(
            TOOL_ZPOWERS,
            &json!({"action": "fast_track", "reason": "1-line typo fix"}),
            false,
            "fast-track recorded",
            None,
        );
        assert_eq!(
            ft_guard.check_pre_tool_phase_gate(true, "hash_edit", &json!({"path": "src/main.rs"})),
            PhaseGateDecision::Allow
        );
    }

    #[test]
    fn zpowers_strict_and_state_driven_workstream_reset_work_without_turn_hardcoding() {
        let ws = std::env::temp_dir().join("zenko_guard_zp_strict");
        let mut strict = ExecutionGuard::new(
            ExecutionTier::Orchestrated,
            IntentKind::Implement,
            DeliberationDepth::Deliberate,
        );
        strict.configure_zpowers(
            ZPowersMode::Strict,
            false,
            3,
            ws.clone(),
            "sess_s".into(),
            &ZPowersState::new("sess_s", ZPowersMode::Strict),
        );

        // Read-only recon always passes immediately with zero ceremony
        assert_eq!(
            strict.check_pre_tool_phase_gate(true, "read_file", &json!({"path": "src/a.rs"})),
            PhaseGateDecision::Allow
        );

        // Strict mode asks for spec first on deliberate tasks
        assert!(matches!(
            strict.check_pre_tool_phase_gate(true, "write_file", &json!({"path": "src/a.rs"})),
            PhaseGateDecision::Redirect { ref hint, .. } if hint.contains("zpowers_spec_first")
        ));
        strict.record_tool_outcome(
            TOOL_ZPOWERS,
            &json!({"action": "write_spec", "content": "# Spec"}),
            false,
            "saved spec",
            None,
        );
        // After spec, asks for plan/todo
        assert!(matches!(
            strict.check_pre_tool_phase_gate(true, "write_file", &json!({"path": "src/a.rs"})),
            PhaseGateDecision::Redirect { ref hint, .. } if hint.contains("zpowers_plan_first")
        ));
        strict.record_tool_outcome(
            TOOL_SESSION_TASK,
            &json!({"action": "create", "title": "Implement a.rs"}),
            false,
            "created task",
            None,
        );
        // After spec + plan, mutation is allowed
        assert_eq!(
            strict.check_pre_tool_phase_gate(true, "write_file", &json!({"path": "src/a.rs"})),
            PhaseGateDecision::Allow
        );

        // State-driven reset: once a prior workstream finished (`Done` + 0 open tasks),
        // a new turn starts fresh and requires a new plan/todo or fast_track (no stale leak!)
        let mut finished_state = ZPowersState::new("sess_next", ZPowersMode::Guided);
        finished_state.plan_recorded = true;
        finished_state.fast_track_reason = Some("previous typo".into());
        finished_state.phase = ZPowersPhase::Done;
        finished_state.mutated_files = vec!["src/old.rs".into()];

        let mut next_turn = ExecutionGuard::new(
            ExecutionTier::Standard,
            IntentKind::Implement,
            DeliberationDepth::Standard,
        );
        next_turn.configure_zpowers(
            ZPowersMode::Guided,
            false,
            3,
            ws.clone(),
            "sess_next".into(),
            &finished_state,
        );
        assert!(matches!(
            next_turn.check_pre_tool_phase_gate(true, "write_file", &json!({"path": "src/new.rs"})),
            PhaseGateDecision::Redirect { ref hint, .. } if hint.contains("zpowers_plan_first")
        ));

        // Conversely, if open todos still remain (`has_open_tasks == true`), execution carries over!
        let mut resume_turn = ExecutionGuard::new(
            ExecutionTier::Standard,
            IntentKind::Implement,
            DeliberationDepth::Standard,
        );
        resume_turn.configure_zpowers(
            ZPowersMode::Guided,
            true,
            3,
            ws,
            "sess_next".into(),
            &finished_state,
        );
        assert_eq!(
            resume_turn.check_pre_tool_phase_gate(true, "write_file", &json!({"path": "src/new.rs"})),
            PhaseGateDecision::Allow
        );

        // Batched `session_task(action="create")` calls inside `batch_tools` MUST satisfy
        // the ZPowers plan-first gate immediately (regression from session 01M8EVBZ05N3E006N5SSBADVT6).
        let mut batched = ExecutionGuard::new(
            ExecutionTier::Orchestrated,
            IntentKind::Implement,
            DeliberationDepth::Standard,
        );
        batched.configure_zpowers(
            ZPowersMode::Guided,
            false,
            3,
            std::env::temp_dir(),
            "sess_batch".into(),
            &ZPowersState::new("sess_batch", ZPowersMode::Guided),
        );
        batched.record_tool_outcome(
            TOOL_BATCH_TOOLS,
            &json!({
                "calls": [
                    { "tool": "session_task", "args": { "action": "create", "title": "Task 1" } },
                    { "tool": "session_task", "args": { "action": "create", "title": "Task 2" } }
                ]
            }),
            false,
            "Batch completed (2/2 ok)",
            Some(&json!({
                "steps": [
                    { "status": "ok" },
                    { "status": "ok" }
                ]
            })),
        );
        assert_eq!(
            batched.check_pre_tool_phase_gate(true, "write_file", &json!({"path": "index.html"})),
            PhaseGateDecision::Allow
        );

        // Native `session_task(action="batch")` unlocks plan-first gate & drift guard catches stale todos
        let mut drift_guard = ExecutionGuard::new(
            ExecutionTier::Orchestrated,
            IntentKind::Implement,
            DeliberationDepth::Standard,
        );
        drift_guard.configure_zpowers(
            ZPowersMode::Guided,
            false,
            3,
            std::env::temp_dir(),
            "sess_drift".into(),
            &ZPowersState::new("sess_drift", ZPowersMode::Guided),
        );
        drift_guard.record_tool_outcome(
            TOOL_SESSION_TASK,
            &json!({ "action": "batch", "tasks": [{ "title": "Edit main.js" }] }),
            false,
            "Batch synced tasks (1 created, 0 updated).",
            None,
        );
        assert_eq!(
            drift_guard.check_pre_tool_phase_gate(true, "write_file", &json!({"path": "main.js"})),
            PhaseGateDecision::Allow
        );
        let open_tasks = vec![zenko_core::SessionTask {
            id: "task_1".into(),
            session_id: "sess_drift".into(),
            branch_event_id: None,
            title: "Edit main.js".into(),
            description: String::new(),
            active_label: None,
            status: zenko_core::SessionTaskStatus::Pending,
            depends_on: Vec::new(),
            owner_agent: None,
            verification_cmd: None,
            verification_evidence: None,
            notes: None,
            priority: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }];
        assert!(drift_guard.check_todo_drift(true, false, &open_tasks).is_none());
        let nudge = drift_guard
            .check_todo_drift(true, false, &open_tasks)
            .expect("must nudge after 2 mutating waves with stale todos");
        assert!(nudge.contains("task_1"));
        assert!(nudge.contains("action=\"batch\""));
        assert!(drift_guard.check_todo_drift(false, true, &open_tasks).is_none());
    }
}
