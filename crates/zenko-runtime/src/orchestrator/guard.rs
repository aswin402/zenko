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
    ORCH_NON_ERROR_REPEAT_THRESHOLD, ORCH_OSCILLATION_THRESHOLD, ORCH_TEXT_DOOM_MIN_LINE_CHARS,
    ORCH_TEXT_DOOM_MIN_LINES, ORCH_TEXT_DOOM_REPEAT_THRESHOLD, OrchestratorPhase, PLANNING_TOOLS,
    READ_ONLY_FIND_MUTATING_FLAGS, READ_ONLY_SHELL_CMD_PREFIXES, READ_ONLY_TOOLS, TOOL_AGENT_KILL,
    TOOL_AGENT_WORKTREE, TOOL_BATCH_TOOLS, TOOL_CRON, TOOL_GIT, TOOL_GOAL_LOOP, TOOL_JOB_WAIT,
    TOOL_LSP, TOOL_MEMORY, TOOL_PLAN_UPDATE, TOOL_RECORD_BRAINSTORM, TOOL_SHELL_EXEC,
    TOOL_SKILL_MANAGE, TOOL_SPAWN_AGENT, VERIFICATION_CMD_PREFIXES, VERIFICATION_SHELL_TOOLS, VERIFICATION_SPOOF_FLAGS_CI,
    VERIFICATION_SPOOF_FLAGS_EXACT, WorktreeAction, compact_preview, find_subagent_role,
    is_git_tool_args_mutating, is_goal_loop_args_mutating, is_memory_mutating_action,
    is_skill_manage_args_mutating,
};

const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

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
        TOOL_SKILL_MANAGE => is_skill_manage_args_mutating(arguments),
        TOOL_GOAL_LOOP => is_goal_loop_args_mutating(arguments),
        TOOL_MEMORY => {
            let action = arguments.get("action").and_then(Value::as_str).unwrap_or("");
            is_memory_mutating_action(action)
        }
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
        READ_ONLY_TOOLS.contains(&tool_name),
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
            ..Self::default()
        }
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

    /// LLM-sovereign pre-tool evaluation: never blocks tool calls based on guessed prompt keywords
    /// or artificial phase ordering. Explicit user-selected `PermissionMode::Plan` and security
    /// rules are enforced cleanly by `ExecPolicy` and schema filtering.
    pub fn check_pre_tool_phase_gate(
        &self,
        _enabled: bool,
        _tool_name: &str,
        _arguments: &Value,
    ) -> PhaseGateDecision {
        PhaseGateDecision::Allow
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

    pub fn record_tool_outcome(
        &mut self,
        tool_name: &str,
        arguments: &Value,
        is_error: bool,
        output: &str,
        metadata: Option<&Value>,
    ) -> GuardVerdict {
        let category = classify_tool_category(tool_name, arguments);
        self.advance_phase(category, is_error);

        // Track structured planning artifacts recorded during the turn
        if !is_error {
            match tool_name {
                TOOL_RECORD_BRAINSTORM => self.brainstorm_recorded = true,
                TOOL_PLAN_UPDATE => self.plan_recorded = true,
                _ => {}
            }
        }

        // Track mutated workspace files & verified completions (foreground, background-then-awaited, or inside `batch_tools`)
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
        if !is_error && MUTATING_FILE_TOOLS.contains(&tool_name)
            && let Some(p) = arguments.get("path").and_then(Value::as_str) {
                let norm = p.trim().trim_start_matches("./").to_string();
                if !norm.is_empty() {
                    self.mutated_files.insert(norm.clone());
                    *self.file_edit_counts.entry(norm).or_insert(0) += 1;
                    self.verification_ran = false;
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
                    if !self.mutated_files.is_empty() {
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

        match is_error {
            true => self.consecutive_errors = self.consecutive_errors.saturating_add(1),
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
                     Re-read the target file/error carefully, inspect assumptions, and pivot to a different approach or call `record_brainstorm` / `ask_clarification` if blocked.\n\
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

        // 4. Consecutive error streak across tools
        if self.consecutive_errors >= ORCH_MAX_CONSECUTIVE_ERRORS {
            let count = self.consecutive_errors;
            self.consecutive_errors = 0;
            let reason = format!("{count} consecutive failing tool calls");
            let hint = format!(
                "<orchestrator_hint type=\"error_streak\">\n\
                 Circuit breaker triggered after {count} consecutive tool errors. Pause mutations, summarize what failed, inspect ground truth with read-only tools, and pivot strategy.\n\
                 </orchestrator_hint>"
            );
            return self.escalate_or_pivot(reason, hint);
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

        let needs_verify = !self.mutated_files.is_empty()
            && !self.verification_ran
            && self.verify_nudges == 0;
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
        let needs_plan_continue = !unfinished_plan.is_empty()
            && self.plan_nudges < ORCH_MAX_PLAN_CONTINUATION_NUDGES.min(1)
            && self.total_continuation_nudges < ORCH_MAX_TOTAL_CONTINUATION_NUDGES;

        let fail_note = match self.last_verification_failed {
            true => " Your last verification command FAILED — consider fixing the root cause and re-checking.",
            false => "",
        };

        match (needs_verify, needs_plan_continue) {
            (false, false) => {
                self.phase = OrchestratorPhase::Done;
                None
            }
            (true, false) => {
                self.verify_nudges += 1;
                let files: Vec<&str> = self.mutated_files.iter().map(String::as_str).collect();
                Some(format!(
                    "<orchestrator_hint type=\"advisory_check\">\n\
                     Advisory: {} file(s) were modified ({}) with no passing check recorded yet.{fail_note} \
                     Run a relevant verification check if appropriate, or respond directly if verification is unnecessary.\n\
                     </orchestrator_hint>",
                    files.len(),
                    files.join(", ")
                ))
            }
            (false, true) => {
                self.plan_nudges += 1;
                self.total_continuation_nudges += 1;
                Some(format!(
                    "<orchestrator_hint type=\"advisory_check\">\n\
                     Advisory: your active DAG still lists {} open step(s): {}. \
                     Continue executing or update/close them via `plan_update`/`session_task`, or conclude directly if done.\n\
                     </orchestrator_hint>",
                    unfinished_plan.len(),
                    unfinished_plan.join(", ")
                ))
            }
            (true, true) => {
                self.verify_nudges += 1;
                self.plan_nudges += 1;
                self.total_continuation_nudges += 1;
                let files: Vec<&str> = self.mutated_files.iter().map(String::as_str).collect();
                Some(format!(
                    "<orchestrator_hint type=\"advisory_check\">\n\
                     Advisory: (1) modified file(s) ({}) have no passing check yet{fail_note}, and \
                     (2) active DAG still has {} open step(s) ({}). Verify/update if needed, or conclude directly.\n\
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
}
