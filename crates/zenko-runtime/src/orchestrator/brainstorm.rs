//! LLM-Sovereign Iterative Brainstorming Engine & Interactive/Headless Clarification Hub.
//! - Preserves full LLM decision authority across multi-round `exploring` -> `refined` -> `committed`
//!   brainstorms with Metis assumptions/blind-spots and Momus adversarial self-critique.
//! - Supplies purely advisory structural reflections (never overrides the LLM's selected option).
//! - Auto-resolves clarifications with zero blocking in headless 24/7 mode (`headless_no_wait = true`).
//! - Waits event-driven (zero busy-polling) via `tokio::sync::oneshot` in interactive mode.
//! - Bridges `record_brainstorm`, `plan_update`, `session_task`, `orchestrate_control`, and
//!   `ask_clarification` across WebUI, CLI, IPC, MCP, and Telegram inline-keyboard buttons.

use crate::orchestrator::plan::{
    compute_ready_session_tasks, validate_and_normalize_plan, validate_and_normalize_session_tasks,
};
use crate::orchestrator::recon::{format_recon_context, run_parallel_recon};
use crate::runtime::AgentRuntime;
use async_trait::async_trait;
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;
use tokio::sync::{broadcast, oneshot};
use tokio_util::sync::CancellationToken;
use zenko_core::{
    ASK_USER_OPTION_ACCEPT, ASK_USER_OPTION_CUSTOM, ASK_USER_OPTION_DATA_SUBMITTED,
    ASK_USER_RESOLVER_NEEDS_INPUT, AgentEvent, AskUserKind, AskUserRequestSpec, AskUserResolution,
    BrainstormOption, BrainstormSpec, BrainstormStage, CLARIFICATION_RESOLVER_HEADLESS,
    CLARIFICATION_RESOLVER_STEERED, CLARIFICATION_RESOLVER_TIMEOUT, ClarificationAnswer,
    ClarificationOption, ClarificationRecord, ClarificationStatus, ExecutionPlan,
    ORCH_BRAINSTORM_MAX_OPTIONS, ORCH_CLARIFICATION_TIMEOUT_SECS, ORCH_SCORE_BLAST_HIGH,
    ORCH_SCORE_BLAST_LOW, ORCH_SCORE_BLAST_MED, ORCH_SCORE_EVIDENCE_RISKS_BONUS,
    ORCH_SCORE_EVIDENCE_ROLLBACK_BONUS, ORCH_SCORE_INFLATED_SAFETY_PENALTY, ORCH_SCORE_REC_BONUS,
    ORCH_SCORE_REV_HIGH, ORCH_SCORE_REV_LOW, ORCH_SCORE_REV_MED, ORCH_SCORE_SAFETY_MULT,
    ORCH_SCORE_TESTABILITY_MULT, ORCH_SCORE_WIDE_FILES_THRESHOLD,
    ORCH_SCORE_WIDE_UNPROTECTED_PENALTY, OrchestrateControlReceipt, OrchestrateControlRequest,
    OrchestratorBridge, Result, SESSION_TASK_EVIDENCE_MAX_CHARS, SessionTask, SessionTaskPatch,
    SessionTaskStatus, ZenkoError, compact_preview, new_id,
};
use zenko_memory::SqliteStore;

fn reversibility_weight(tag: &str) -> u32 {
    match tag.trim().to_ascii_lowercase().as_str() {
        "high" | "easy" | "trivial" => ORCH_SCORE_REV_HIGH,
        "medium" | "moderate" => ORCH_SCORE_REV_MED,
        _ => ORCH_SCORE_REV_LOW,
    }
}

fn blast_radius_weight(tag: &str) -> u32 {
    match tag.trim().to_ascii_lowercase().as_str() {
        "low" | "small" | "isolated" => ORCH_SCORE_BLAST_LOW,
        "medium" | "moderate" => ORCH_SCORE_BLAST_MED,
        _ => ORCH_SCORE_BLAST_HIGH,
    }
}

/// Deterministic anti-confirmation-bias score combining bounded self-scores, reversibility,
/// blast radius, and concrete structural evidence (`touched_files`, `rollback_steps`, `risks`).
pub fn score_brainstorm_option(opt: &BrainstormOption) -> u32 {
    let safety = u32::from(opt.safety_score.min(10)) * ORCH_SCORE_SAFETY_MULT;
    let testability = u32::from(opt.testability_score.min(10)) * ORCH_SCORE_TESTABILITY_MULT;
    let rev = reversibility_weight(&opt.reversibility);
    let blast = blast_radius_weight(&opt.blast_radius);
    let rec_bonus = match opt.recommended {
        true => ORCH_SCORE_REC_BONUS,
        false => 0,
    };
    let rollback_bonus = match opt.rollback_steps.is_empty() {
        true => 0,
        false => ORCH_SCORE_EVIDENCE_ROLLBACK_BONUS,
    };
    let risks_bonus = match opt.risks.is_empty() {
        true => 0,
        false => ORCH_SCORE_EVIDENCE_RISKS_BONUS,
    };

    // Anti-confirmation-bias penalties:
    // 1. High claimed safety (>=8) with zero rollback plan and zero acknowledged cons/risks
    let inflated_safety_penalty = match opt.safety_score >= 8
        && opt.rollback_steps.is_empty()
        && opt.risks.is_empty()
        && opt.cons.is_empty()
    {
        true => ORCH_SCORE_INFLATED_SAFETY_PENALTY,
        false => 0,
    };
    // 2. Wide multi-file blast radius (>=4 files) without rollback steps
    let wide_unprotected_penalty = match opt.touched_files.len() >= ORCH_SCORE_WIDE_FILES_THRESHOLD
        && opt.rollback_steps.is_empty()
    {
        true => ORCH_SCORE_WIDE_UNPROTECTED_PENALTY,
        false => 0,
    };

    (safety + testability + rev + blast + rec_bonus + rollback_bonus + risks_bonus)
        .saturating_sub(inflated_safety_penalty)
        .saturating_sub(wide_unprotected_penalty)
}

/// Picks the optimal reversible, evidence-backed option deterministically.
pub fn select_optimal_option(options: &[BrainstormOption]) -> Option<&BrainstormOption> {
    options
        .iter()
        .max_by_key(|opt| score_brainstorm_option(opt))
}

/// Purely advisory, zero-hardcoding structural reflections on a `BrainstormSpec`.
/// Never blocks, never checks hardcoded shell-command lists, and respects `Exploring` drafts.
pub fn audit_brainstorm_rubric(spec: &BrainstormSpec) -> Vec<String> {
    let mut notes = Vec::new();
    let mut seen_ids: HashSet<&str> = HashSet::with_capacity(spec.options.len());
    for opt in &spec.options {
        if !seen_ids.insert(opt.id.as_str()) {
            notes.push(format!("Duplicate option ID `{}` detected.", opt.id));
        }
    }

    match spec.stage {
        BrainstormStage::Exploring => {
            if spec.open_questions.is_empty() && spec.blind_spots.is_empty() && spec.options.len() < 2 {
                notes.push(
                    "Exploration draft has a single option and no open questions/blind spots; consider widening alternatives or advancing to `committed`.".into(),
                );
            }
        }
        BrainstormStage::Refined | BrainstormStage::Committed => {
            let sel = spec.selected_option_id.trim();
            match (spec.stage, sel.is_empty(), spec.options.iter().find(|o| o.id == sel)) {
                (BrainstormStage::Committed, true, _) => {
                    notes.push(
                        "Stage is `committed` without a `selected_option_id`; set `selected_option_id` or keep `stage` as `exploring`.".into(),
                    );
                }
                (_, false, None) => {
                    notes.push(format!(
                        "`selected_option_id` (`{sel}`) does not match any listed option ID."
                    ));
                }
                (_, false, Some(opt)) => {
                    if opt.touched_files.len() >= ORCH_SCORE_WIDE_FILES_THRESHOLD
                        && opt.rollback_steps.is_empty()
                    {
                        notes.push(format!(
                            "Option `{}` touches {} files with no rollback steps noted.",
                            opt.id,
                            opt.touched_files.len()
                        ));
                    }
                }
                _ => {}
            }
            if spec.stage == BrainstormStage::Committed && !spec.open_questions.is_empty() {
                notes.push(format!(
                    "{} open question(s) remain listed on a committed brainstorm.",
                    spec.open_questions.len()
                ));
            }
        }
    }

    notes
}

/// Normalizes and attaches non-blocking structural reflections to a `BrainstormSpec` while
/// strictly honoring the LLM's chosen `stage` and `selected_option_id` (zero score override!).
pub fn finalize_brainstorm_spec(mut spec: BrainstormSpec) -> BrainstormSpec {
    spec.options.truncate(ORCH_BRAINSTORM_MAX_OPTIONS);
    spec.selected_option_id = spec.selected_option_id.trim().to_string();
    spec.rubric_warnings = audit_brainstorm_rubric(&spec);
    spec
}

/// Deterministically chooses the recommended option for headless / 24/7 execution.
pub fn resolve_headless_clarification(options: &[ClarificationOption]) -> (String, String) {
    let chosen = options
        .iter()
        .find(|o| o.recommended)
        .or_else(|| options.first());
    match chosen {
        Some(opt) => (
            opt.id.clone(),
            format!(
                "Headless autonomous mode selected recommended option `{}` ({}).",
                opt.label, opt.id
            ),
        ),
        None => (
            "default".to_string(),
            "Headless autonomous mode proceeded with default strategy.".to_string(),
        ),
    }
}

/// Pure deterministic headless resolver for rich `ask_user` prompts:
/// - Choices, recommendations, and multi-step flows with recommended defaults auto-resolve safely.
/// - Required `DataInput` prompts lacking defaults NEVER fabricate fake data; they return
///   `needs_user_input = true` so the orchestrator reports the exact missing input cleanly.
pub fn resolve_headless_ask_user(spec: &AskUserRequestSpec) -> ClarificationAnswer {
    match spec.kind {
        AskUserKind::OptionList => {
            let (rec_id, note) = resolve_headless_clarification(&spec.options);
            ClarificationAnswer {
                selected_option_id: rec_id.clone(),
                selected_option_ids: vec![rec_id],
                step_answers: std::collections::BTreeMap::new(),
                field_values: std::collections::BTreeMap::new(),
                custom_answer: Some(note),
                rating: None,
                resolved_by: CLARIFICATION_RESOLVER_HEADLESS.to_string(),
                auto_resolved: true,
                needs_user_input: false,
            }
        }
        AskUserKind::Recommendation => {
            let rec_id = match spec.options.is_empty() {
                true => ASK_USER_OPTION_ACCEPT.to_string(),
                false => resolve_headless_clarification(&spec.options).0,
            };
            ClarificationAnswer {
                selected_option_id: rec_id.clone(),
                selected_option_ids: vec![rec_id],
                step_answers: std::collections::BTreeMap::new(),
                field_values: std::collections::BTreeMap::new(),
                custom_answer: Some(
                    "Headless autonomous mode accepted the recommended proposal.".into(),
                ),
                rating: None,
                resolved_by: CLARIFICATION_RESOLVER_HEADLESS.to_string(),
                auto_resolved: true,
                needs_user_input: false,
            }
        }
        AskUserKind::QuestionFlow => {
            let mut step_answers = std::collections::BTreeMap::new();
            for step in &spec.steps {
                let (rec_id, _) = resolve_headless_clarification(&step.options);
                step_answers.insert(step.id.clone(), vec![rec_id]);
            }
            ClarificationAnswer {
                selected_option_id: "flow_defaults".to_string(),
                selected_option_ids: Vec::new(),
                step_answers,
                field_values: std::collections::BTreeMap::new(),
                custom_answer: Some(
                    "Headless autonomous mode selected recommended defaults across all flow steps."
                        .into(),
                ),
                rating: None,
                resolved_by: CLARIFICATION_RESOLVER_HEADLESS.to_string(),
                auto_resolved: true,
                needs_user_input: false,
            }
        }
        AskUserKind::DataInput => {
            let mut field_values = std::collections::BTreeMap::new();
            let mut missing_required: Vec<&str> = Vec::new();
            for field in &spec.fields {
                match field
                    .default_value
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    Some(def) => {
                        field_values.insert(field.id.clone(), def.to_string());
                    }
                    None if field.required => missing_required.push(field.label.as_str()),
                    None => {}
                }
            }
            let missing_unstructured = spec.fields.is_empty() && !spec.allow_auto_default;
            match !missing_required.is_empty() || missing_unstructured {
                true => ClarificationAnswer {
                    selected_option_id: ASK_USER_RESOLVER_NEEDS_INPUT.to_string(),
                    selected_option_ids: Vec::new(),
                    step_answers: std::collections::BTreeMap::new(),
                    field_values,
                    custom_answer: Some(format!(
                        "Headless mode cannot fabricate required user data ({}). Pause and report missing input to the user.",
                        match missing_required.is_empty() {
                            true => spec.question.as_str().to_string(),
                            false => missing_required.join(", "),
                        }
                    )),
                    rating: None,
                    resolved_by: ASK_USER_RESOLVER_NEEDS_INPUT.to_string(),
                    auto_resolved: false,
                    needs_user_input: true,
                },
                false => ClarificationAnswer {
                    selected_option_id: ASK_USER_OPTION_DATA_SUBMITTED.to_string(),
                    selected_option_ids: Vec::new(),
                    step_answers: std::collections::BTreeMap::new(),
                    field_values,
                    custom_answer: Some(
                        "Headless autonomous mode applied provided field defaults.".into(),
                    ),
                    rating: None,
                    resolved_by: CLARIFICATION_RESOLVER_HEADLESS.to_string(),
                    auto_resolved: true,
                    needs_user_input: false,
                },
            }
        }
        AskUserKind::Feedback => match spec.allow_auto_default {
            true => ClarificationAnswer {
                selected_option_id: ASK_USER_OPTION_ACCEPT.to_string(),
                selected_option_ids: Vec::new(),
                step_answers: std::collections::BTreeMap::new(),
                field_values: std::collections::BTreeMap::new(),
                custom_answer: Some("Headless run proceeded without interactive feedback.".into()),
                rating: None,
                resolved_by: CLARIFICATION_RESOLVER_HEADLESS.to_string(),
                auto_resolved: true,
                needs_user_input: false,
            },
            false => ClarificationAnswer {
                selected_option_id: ASK_USER_RESOLVER_NEEDS_INPUT.to_string(),
                selected_option_ids: Vec::new(),
                step_answers: std::collections::BTreeMap::new(),
                field_values: std::collections::BTreeMap::new(),
                custom_answer: Some(
                    "Interactive user feedback requested; awaiting human response.".into(),
                ),
                rating: None,
                resolved_by: ASK_USER_RESOLVER_NEEDS_INPUT.to_string(),
                auto_resolved: false,
                needs_user_input: true,
            },
        },
    }
}

struct PendingClarification {
    record: ClarificationRecord,
    tx: oneshot::Sender<ClarificationAnswer>,
}

/// Leak-free shared hub bridging `zenko-tools` (`record_brainstorm`, `plan_update`, `session_task`,
/// `ask_clarification`) with `AgentRuntime`, SQLite `session_tasks`, WebUI, CLI, Telegram, IPC, and MCP.
pub struct ClarificationHub {
    store: SqliteStore,
    events: broadcast::Sender<AgentEvent>,
    pending: Mutex<HashMap<String, PendingClarification>>,
    /// Serializes read-modify-write DAG operations per hub so concurrent subagents & tool waves
    /// never suffer lost updates across `list -> validate -> replace_session_tasks`.
    dag_lock: Mutex<()>,
    /// Leak-free weak handle to `AgentRuntime` powering `orchestrate_control` live shifts & recon.
    runtime: OnceLock<Weak<AgentRuntime>>,
}

impl ClarificationHub {
    pub fn new(store: SqliteStore, events: broadcast::Sender<AgentEvent>) -> Arc<Self> {
        Arc::new(Self {
            store,
            events,
            pending: Mutex::new(HashMap::new()),
            dag_lock: Mutex::new(()),
            runtime: OnceLock::new(),
        })
    }

    pub fn bind_runtime(&self, weak: Weak<AgentRuntime>) {
        let _ = self.runtime.set(weak);
    }

    /// Resolves the shared root session Task DAG board so spawned Phase 5 subagents collaborate
    /// seamlessly on their ancestor root session's ACID SQLite task graph.
    fn resolve_board_session_id(&self, session_id: &str) -> String {
        let mut cur = session_id.to_string();
        let mut seen = HashSet::new();
        seen.insert(cur.clone());
        while let Ok(Some(sess)) = self.store.get_session(&cur) {
            let Some(parent) = sess.parent_session_id.filter(|p| !p.trim().is_empty()) else {
                break;
            };
            if !seen.insert(parent.clone()) {
                break;
            }
            cur = parent;
        }
        cur
    }

    fn sync_zpowers_tasks(&self, board_sid: &str, tasks: &[SessionTask]) {
        let Some(rt) = self.runtime.get().and_then(Weak::upgrade) else {
            return;
        };
        let cfg = rt.config();
        if !cfg.orchestrator.zpowers.sync_artifacts_to_workspace {
            return;
        }
        let ws = rt.session_workspace_dir(board_sid);
        let (title, parent) = self
            .store
            .get_session(board_sid)
            .ok()
            .flatten()
            .map(|s| (s.title, s.parent_session_id))
            .unwrap_or_else(|| (String::new(), None));
        let _ = crate::zpowers::sync_session_tasks_plan(
            &ws,
            board_sid,
            &title,
            parent.as_deref(),
            tasks,
            &cfg.orchestrator.zpowers,
        );
    }

    fn sync_zpowers_spec(&self, session_id: &str, spec: &BrainstormSpec) {
        let Some(rt) = self.runtime.get().and_then(Weak::upgrade) else {
            return;
        };
        let cfg = rt.config();
        if !cfg.orchestrator.zpowers.sync_artifacts_to_workspace {
            return;
        }
        let board_sid = self.resolve_board_session_id(session_id);
        let ws = rt.session_workspace_dir(&board_sid);
        let (title, parent) = self
            .store
            .get_session(&board_sid)
            .ok()
            .flatten()
            .map(|s| (s.title, s.parent_session_id))
            .unwrap_or_else(|| (String::new(), None));
        let _ = crate::zpowers::sync_brainstorm_spec(
            &ws,
            &board_sid,
            &title,
            parent.as_deref(),
            spec,
            &cfg.orchestrator.zpowers,
        );
    }

    fn sync_zpowers_plan(&self, session_id: &str, plan: &ExecutionPlan) {
        let Some(rt) = self.runtime.get().and_then(Weak::upgrade) else {
            return;
        };
        let cfg = rt.config();
        if !cfg.orchestrator.zpowers.sync_artifacts_to_workspace {
            return;
        }
        let board_sid = self.resolve_board_session_id(session_id);
        let ws = rt.session_workspace_dir(&board_sid);
        let (title, parent) = self
            .store
            .get_session(&board_sid)
            .ok()
            .flatten()
            .map(|s| (s.title, s.parent_session_id))
            .unwrap_or_else(|| (String::new(), None));
        let _ = crate::zpowers::sync_execution_plan(
            &ws,
            &board_sid,
            &title,
            parent.as_deref(),
            plan,
            &cfg.orchestrator.zpowers,
        );
    }

    fn sync_zpowers_decision(
        &self,
        session_id: &str,
        rec: &ClarificationRecord,
        ans: &ClarificationAnswer,
    ) {
        if ans.needs_user_input {
            return;
        }
        let Some(rt) = self.runtime.get().and_then(Weak::upgrade) else {
            return;
        };
        let cfg = rt.config();
        if !cfg.orchestrator.zpowers.sync_artifacts_to_workspace {
            return;
        }
        let board_sid = self.resolve_board_session_id(session_id);
        let ws = rt.session_workspace_dir(&board_sid);
        let (title, parent) = self
            .store
            .get_session(&board_sid)
            .ok()
            .flatten()
            .map(|s| (s.title, s.parent_session_id))
            .unwrap_or_else(|| (String::new(), None));
        let _ = crate::zpowers::append_user_decision(
            &ws,
            &board_sid,
            &title,
            parent.as_deref(),
            rec,
            ans,
            &cfg.orchestrator.zpowers,
        );
    }

    fn emit_tasks_updated(&self, board_sid: &str) {
        if let Ok(tasks) = self.store.list_session_tasks(board_sid) {
            self.sync_zpowers_tasks(board_sid, &tasks);
            let _ = self.events.send(AgentEvent::SessionTasksUpdated {
                session_id: board_sid.to_string(),
                tasks,
            });
        }
    }

    pub fn resolve_ask_user(
        &self,
        clarification_id: &str,
        resolution: AskUserResolution,
        resolved_by: &str,
    ) -> Option<ClarificationRecord> {
        let entry = self
            .pending
            .lock()
            .ok()
            .and_then(|mut map| map.remove(clarification_id))?;

        let mut rec = entry.record;
        let selected_ids: Vec<String> = resolution
            .selected_option_ids
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let custom_answer = resolution
            .custom_answer
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let rating = resolution
            .rating
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let primary_id = match resolution.selected_option_id.trim() {
            "" => match selected_ids.first() {
                Some(first) => first.clone(),
                None => match rec.kind {
                    AskUserKind::DataInput => ASK_USER_OPTION_DATA_SUBMITTED.to_string(),
                    AskUserKind::QuestionFlow => "flow_completed".to_string(),
                    AskUserKind::Feedback => rating
                        .clone()
                        .unwrap_or_else(|| ASK_USER_OPTION_ACCEPT.to_string()),
                    AskUserKind::Recommendation => ASK_USER_OPTION_ACCEPT.to_string(),
                    AskUserKind::OptionList => ASK_USER_OPTION_CUSTOM.to_string(),
                },
            },
            id => id.to_string(),
        };

        rec.status = ClarificationStatus::Answered;
        rec.selected_option_id = Some(primary_id.clone());
        rec.selected_option_ids = selected_ids.clone();
        rec.step_answers = resolution.step_answers.clone();
        rec.field_values = resolution.field_values.clone();
        rec.custom_answer = custom_answer.clone();
        rec.rating = rating.clone();
        rec.resolved_by = Some(resolved_by.to_string());
        rec.resolved_at = Some(Utc::now());

        let answer = ClarificationAnswer {
            selected_option_id: primary_id.clone(),
            selected_option_ids: selected_ids,
            step_answers: resolution.step_answers,
            field_values: resolution.field_values,
            custom_answer: custom_answer.clone(),
            rating,
            resolved_by: resolved_by.to_string(),
            auto_resolved: false,
            needs_user_input: false,
        };
        self.sync_zpowers_decision(&rec.session_id, &rec, &answer);
        let _ = entry.tx.send(answer);
        let _ = self.events.send(AgentEvent::ClarificationResolved {
            clarification_id: rec.id.clone(),
            session_id: rec.session_id.clone(),
            selected_option_id: primary_id,
            custom_answer,
            resolved_by: resolved_by.to_string(),
        });
        Some(rec)
    }

    pub fn resolve_clarification(
        &self,
        clarification_id: &str,
        selected_option_id: &str,
        custom_answer: Option<String>,
        resolved_by: &str,
    ) -> Option<ClarificationRecord> {
        let trimmed = selected_option_id.trim().to_string();
        let selected_option_ids = match trimmed.is_empty() {
            true => Vec::new(),
            false => vec![trimmed.clone()],
        };
        self.resolve_ask_user(
            clarification_id,
            AskUserResolution {
                selected_option_id: trimmed,
                selected_option_ids,
                custom_answer,
                ..Default::default()
            },
            resolved_by,
        )
    }

    /// Resolves a pending clarification by zero-based option index (used by Telegram inline
    /// keyboard callbacks to stay comfortably inside Telegram's 64-byte `callback_data` limit).
    pub fn resolve_clarification_by_index(
        &self,
        clarification_id: &str,
        option_index: usize,
        resolved_by: &str,
    ) -> Option<ClarificationRecord> {
        let opt_id = self
            .pending
            .lock()
            .ok()
            .and_then(|map| {
                map.get(clarification_id)
                    .and_then(|p| p.record.options.get(option_index))
                    .map(|o| o.id.clone())
            })?;
        self.resolve_clarification(clarification_id, &opt_id, None, resolved_by)
    }

    /// Immediately unblocks any pending clarification on `session_id` when steering arrives.
    pub fn unblock_for_steering(&self, session_id: &str, steer_text: &str) {
        let ids: Vec<String> = self
            .pending
            .lock()
            .map(|map| {
                map.iter()
                    .filter(|(_, p)| p.record.session_id == session_id)
                    .map(|(id, _)| id.clone())
                    .collect()
            })
            .unwrap_or_default();
        for id in ids {
            let rec_opt = self
                .pending
                .lock()
                .ok()
                .and_then(|map| map.get(&id).map(|p| p.record.recommended_option_id.clone()));
            if let Some(rec_id) = rec_opt {
                let _ = self.resolve_clarification(
                    &id,
                    &rec_id,
                    Some(steer_text.to_string()),
                    CLARIFICATION_RESOLVER_STEERED,
                );
            }
        }
    }
}

#[async_trait]
impl OrchestratorBridge for ClarificationHub {
    fn record_brainstorm(&self, session_id: &str, spec: &BrainstormSpec) -> Result<BrainstormSpec> {
        let normalized = finalize_brainstorm_spec(spec.clone());
        self.sync_zpowers_spec(session_id, &normalized);
        let _ = self.events.send(AgentEvent::BrainstormRecorded {
            session_id: session_id.to_string(),
            spec: normalized.clone(),
        });
        Ok(normalized)
    }

    fn update_plan(&self, session_id: &str, plan: &ExecutionPlan) -> Result<ExecutionPlan> {
        let normalized = validate_and_normalize_plan(plan.clone())?;
        let _dag_guard = self.dag_lock.lock().unwrap_or_else(|e| e.into_inner());
        let board_sid = self.resolve_board_session_id(session_id);
        let existing_by_id: HashMap<String, SessionTask> = self
            .store
            .list_session_tasks(&board_sid)
            .unwrap_or_default()
            .into_iter()
            .map(|t| (t.id.clone(), t))
            .collect();
        let head_event = self
            .store
            .get_session(&board_sid)
            .ok()
            .flatten()
            .and_then(|s| s.head_event_id);
        let now = Utc::now();
        let total = normalized.steps.len() as i32;
        let synced_tasks: Vec<SessionTask> = normalized
            .steps
            .iter()
            .enumerate()
            .map(|(idx, step)| {
                let prev = existing_by_id.get(&step.id);
                SessionTask {
                    id: step.id.clone(),
                    session_id: board_sid.clone(),
                    branch_event_id: prev
                        .and_then(|p| p.branch_event_id.clone())
                        .or_else(|| head_event.clone()),
                    title: step.title.clone(),
                    description: prev.map(|p| p.description.clone()).unwrap_or_default(),
                    active_label: prev.and_then(|p| p.active_label.clone()),
                    status: SessionTaskStatus::from_plan_status(step.status),
                    depends_on: step.depends_on.clone(),
                    owner_agent: prev.and_then(|p| p.owner_agent.clone()),
                    verification_cmd: step
                        .verification
                        .clone()
                        .or_else(|| prev.and_then(|p| p.verification_cmd.clone())),
                    verification_evidence: prev.and_then(|p| p.verification_evidence.clone()),
                    notes: prev.and_then(|p| p.notes.clone()),
                    priority: total - (idx as i32),
                    created_at: prev.map(|p| p.created_at).unwrap_or(now),
                    updated_at: now,
                }
            })
            .collect();
        if self.store.replace_session_tasks(&board_sid, &synced_tasks).is_ok() {
            let _ = self.events.send(AgentEvent::SessionTasksUpdated {
                session_id: board_sid,
                tasks: synced_tasks,
            });
        }
        self.sync_zpowers_plan(session_id, &normalized);
        let _ = self.events.send(AgentEvent::PlanUpdated {
            session_id: session_id.to_string(),
            plan: normalized.clone(),
        });
        Ok(normalized)
    }

    async fn request_ask_user(
        &self,
        session_id: &str,
        spec: AskUserRequestSpec,
        headless_no_wait: bool,
        cancel: CancellationToken,
    ) -> Result<ClarificationAnswer> {
        let rec_opt_id = match spec.kind {
            AskUserKind::OptionList => resolve_headless_clarification(&spec.options).0,
            AskUserKind::Recommendation => match spec.options.is_empty() {
                true => ASK_USER_OPTION_ACCEPT.to_string(),
                false => resolve_headless_clarification(&spec.options).0,
            },
            AskUserKind::QuestionFlow => "flow_defaults".to_string(),
            AskUserKind::DataInput => ASK_USER_OPTION_DATA_SUBMITTED.to_string(),
            AskUserKind::Feedback => ASK_USER_OPTION_ACCEPT.to_string(),
        };

        let id = new_id();
        let record = ClarificationRecord {
            id: id.clone(),
            session_id: session_id.to_string(),
            kind: spec.kind,
            title: spec.title.clone(),
            question: spec.question.clone(),
            context: spec.context.clone(),
            selection_mode: spec.selection_mode,
            options: spec.options.clone(),
            steps: spec.steps.clone(),
            fields: spec.fields.clone(),
            recommendation: spec.recommendation.clone(),
            allow_custom: spec.allow_custom,
            allow_auto_default: spec.allow_auto_default,
            recommended_option_id: rec_opt_id,
            status: ClarificationStatus::Pending,
            selected_option_id: None,
            selected_option_ids: Vec::new(),
            step_answers: std::collections::BTreeMap::new(),
            field_values: std::collections::BTreeMap::new(),
            custom_answer: None,
            rating: None,
            resolved_by: None,
            created_at: Utc::now(),
            resolved_at: None,
        };

        if headless_no_wait {
            let answer = resolve_headless_ask_user(&spec);
            self.sync_zpowers_decision(session_id, &record, &answer);
            let _ = self.events.send(AgentEvent::ClarificationResolved {
                clarification_id: id,
                session_id: session_id.to_string(),
                selected_option_id: answer.selected_option_id.clone(),
                custom_answer: answer.custom_answer.clone(),
                resolved_by: answer.resolved_by.clone(),
            });
            return Ok(answer);
        }

        let (tx, rx) = oneshot::channel();
        if let Ok(mut map) = self.pending.lock() {
            map.insert(
                id.clone(),
                PendingClarification {
                    record: record.clone(),
                    tx,
                },
            );
        }
        let _ = self.events.send(AgentEvent::ClarificationRequested {
            clarification: record.clone(),
        });

        let wait = tokio::time::timeout(Duration::from_secs(ORCH_CLARIFICATION_TIMEOUT_SECS), rx);
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                if let Ok(mut map) = self.pending.lock() {
                    map.remove(&id);
                }
                Err(ZenkoError::Cancelled)
            }
            res = wait => match res {
                Ok(Ok(ans)) => Ok(ans),
                _ => {
                    if let Ok(mut map) = self.pending.lock() {
                        map.remove(&id);
                    }
                    let mut fallback = resolve_headless_ask_user(&spec);
                    match fallback.needs_user_input {
                        true => {
                            fallback.resolved_by = ASK_USER_RESOLVER_NEEDS_INPUT.to_string();
                        }
                        false => {
                            fallback.resolved_by = CLARIFICATION_RESOLVER_TIMEOUT.to_string();
                            fallback.custom_answer = Some(
                                "Timed out waiting for human response; proceeding with recommended default.".into(),
                            );
                        }
                    }
                    self.sync_zpowers_decision(session_id, &record, &fallback);
                    let _ = self.events.send(AgentEvent::ClarificationResolved {
                        clarification_id: id,
                        session_id: session_id.to_string(),
                        selected_option_id: fallback.selected_option_id.clone(),
                        custom_answer: fallback.custom_answer.clone(),
                        resolved_by: fallback.resolved_by.clone(),
                    });
                    Ok(fallback)
                }
            }
        }
    }

    fn list_pending_clarifications(&self, session_id: Option<&str>) -> Vec<ClarificationRecord> {
        self.pending
            .lock()
            .map(|map| {
                map.values()
                    .filter(|p| session_id.is_none_or(|sid| p.record.session_id == sid))
                    .map(|p| p.record.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn list_session_tasks(&self, session_id: &str) -> Result<Vec<SessionTask>> {
        let board_sid = self.resolve_board_session_id(session_id);
        self.store.list_session_tasks(&board_sid)
    }

    fn ready_session_tasks(&self, session_id: &str) -> Result<Vec<SessionTask>> {
        let board_sid = self.resolve_board_session_id(session_id);
        let tasks = self.store.list_session_tasks(&board_sid)?;
        Ok(compute_ready_session_tasks(&tasks))
    }

    fn create_session_task(&self, session_id: &str, mut task: SessionTask) -> Result<SessionTask> {
        let _dag_guard = self.dag_lock.lock().unwrap_or_else(|e| e.into_inner());
        let board_sid = self.resolve_board_session_id(session_id);
        task.session_id = board_sid.clone();
        if task.id.trim().is_empty() {
            task.id = zenko_core::new_session_task_id();
        }
        if task.branch_event_id.is_none() {
            task.branch_event_id = self
                .store
                .get_session(&board_sid)?
                .and_then(|s| s.head_event_id);
        }
        if let Some(ev) = task.verification_evidence.take() {
            task.verification_evidence =
                Some(compact_preview(&ev, SESSION_TASK_EVIDENCE_MAX_CHARS));
        }

        let mut all = self.store.list_session_tasks(&board_sid)?;
        if let Some(slot) = all.iter_mut().find(|t| t.id == task.id) {
            *slot = task.clone();
        } else {
            all.push(task.clone());
        }
        validate_and_normalize_session_tasks(&mut all)?;
        let saved = all
            .iter()
            .find(|t| t.id == task.id)
            .cloned()
            .unwrap_or(task);
        self.store.replace_session_tasks(&board_sid, &all)?;
        self.emit_tasks_updated(&board_sid);
        Ok(saved)
    }

    fn update_session_task(
        &self,
        session_id: &str,
        task_id: &str,
        patch: SessionTaskPatch,
    ) -> Result<SessionTask> {
        let _dag_guard = self.dag_lock.lock().unwrap_or_else(|e| e.into_inner());
        let board_sid = self.resolve_board_session_id(session_id);
        let mut all = self.store.list_session_tasks(&board_sid)?;
        let Some(target) = all.iter_mut().find(|t| t.id == task_id) else {
            return Err(ZenkoError::NotFound(format!(
                "session task `{task_id}` not found"
            )));
        };
        if let Some(title) = patch.title {
            target.title = title;
        }
        if let Some(desc) = patch.description {
            target.description = desc;
        }
        if let Some(active) = patch.active_label {
            target.active_label = active;
        }
        if let Some(status) = patch.status {
            target.status = status;
        }
        if let Some(deps) = patch.depends_on {
            target.depends_on = deps;
        }
        for dep in patch.add_depends_on {
            if !dep.trim().is_empty() && !target.depends_on.iter().any(|d| d == dep.trim()) {
                target.depends_on.push(dep.trim().to_string());
            }
        }
        if let Some(owner) = patch.owner_agent {
            target.owner_agent = owner;
        }
        if let Some(cmd) = patch.verification_cmd {
            target.verification_cmd = cmd;
        }
        if let Some(ev) = patch.verification_evidence {
            target.verification_evidence =
                ev.map(|s| compact_preview(&s, SESSION_TASK_EVIDENCE_MAX_CHARS));
        }
        if let Some(notes) = patch.notes {
            target.notes = notes;
        }
        if let Some(prio) = patch.priority {
            target.priority = prio;
        }
        target.updated_at = Utc::now();

        validate_and_normalize_session_tasks(&mut all)?;
        let updated = all
            .iter()
            .find(|t| t.id == task_id)
            .cloned()
            .ok_or_else(|| ZenkoError::NotFound(format!("session task `{task_id}` not found")))?;
        self.store.replace_session_tasks(&board_sid, &all)?;
        self.emit_tasks_updated(&board_sid);
        Ok(updated)
    }

    fn batch_upsert_session_tasks(
        &self,
        session_id: &str,
        creates: Vec<SessionTask>,
        updates: Vec<(String, SessionTaskPatch)>,
    ) -> Result<(usize, usize, Vec<SessionTask>)> {
        let _dag_guard = self.dag_lock.lock().unwrap_or_else(|e| e.into_inner());
        let board_sid = self.resolve_board_session_id(session_id);
        let default_branch_ev = self
            .store
            .get_session(&board_sid)?
            .and_then(|s| s.head_event_id);
        let mut all = self.store.list_session_tasks(&board_sid)?;
        let mut created = 0usize;
        let mut updated = 0usize;

        for mut task in creates {
            task.session_id = board_sid.clone();
            if task.id.trim().is_empty() {
                task.id = zenko_core::new_session_task_id();
            }
            if task.branch_event_id.is_none() {
                task.branch_event_id = default_branch_ev.clone();
            }
            if let Some(ev) = task.verification_evidence.take() {
                task.verification_evidence =
                    Some(compact_preview(&ev, SESSION_TASK_EVIDENCE_MAX_CHARS));
            }
            if let Some(slot) = all.iter_mut().find(|t| t.id == task.id) {
                *slot = task;
                updated += 1;
            } else {
                all.push(task);
                created += 1;
            }
        }

        let now = Utc::now();
        for (task_id, patch) in updates {
            let Some(target) = all.iter_mut().find(|t| t.id == task_id) else {
                return Err(ZenkoError::NotFound(format!(
                    "session task `{task_id}` not found"
                )));
            };
            if let Some(title) = patch.title {
                target.title = title;
            }
            if let Some(desc) = patch.description {
                target.description = desc;
            }
            if let Some(active) = patch.active_label {
                target.active_label = active;
            }
            if let Some(status) = patch.status {
                target.status = status;
            }
            if let Some(deps) = patch.depends_on {
                target.depends_on = deps;
            }
            for dep in patch.add_depends_on {
                if !dep.trim().is_empty() && !target.depends_on.iter().any(|d| d == dep.trim()) {
                    target.depends_on.push(dep.trim().to_string());
                }
            }
            if let Some(owner) = patch.owner_agent {
                target.owner_agent = owner;
            }
            if let Some(cmd) = patch.verification_cmd {
                target.verification_cmd = cmd;
            }
            if let Some(ev) = patch.verification_evidence {
                target.verification_evidence =
                    ev.map(|s| compact_preview(&s, SESSION_TASK_EVIDENCE_MAX_CHARS));
            }
            if let Some(notes) = patch.notes {
                target.notes = notes;
            }
            if let Some(prio) = patch.priority {
                target.priority = prio;
            }
            target.updated_at = now;
            updated += 1;
        }

        validate_and_normalize_session_tasks(&mut all)?;
        self.store.replace_session_tasks(&board_sid, &all)?;
        self.emit_tasks_updated(&board_sid);
        Ok((created, updated, all))
    }

    fn claim_session_task(
        &self,
        session_id: &str,
        task_id: &str,
        owner_agent: &str,
    ) -> Result<SessionTask> {
        let _dag_guard = self.dag_lock.lock().unwrap_or_else(|e| e.into_inner());
        let board_sid = self.resolve_board_session_id(session_id);
        let all = self.store.list_session_tasks(&board_sid)?;
        let Some(target) = all.iter().find(|t| t.id == task_id) else {
            return Err(ZenkoError::NotFound(format!(
                "session task `{task_id}` not found"
            )));
        };
        let status_map: HashMap<&str, SessionTaskStatus> =
            all.iter().map(|t| (t.id.as_str(), t.status)).collect();
        let unfinished_deps: Vec<&str> = target
            .depends_on
            .iter()
            .filter(|d| status_map.get(d.as_str()) != Some(&SessionTaskStatus::Done))
            .map(String::as_str)
            .collect();
        if !unfinished_deps.is_empty() {
            return Err(ZenkoError::PolicyDenied(format!(
                "Cannot claim task `{task_id}` while upstream dependencies are unfinished: {}",
                unfinished_deps.join(", ")
            )));
        }
        let owner = match owner_agent.trim().is_empty() {
            true => session_id,
            false => owner_agent.trim(),
        };
        match self.store.claim_session_task(&board_sid, task_id, owner)? {
            Some(claimed) => {
                self.emit_tasks_updated(&board_sid);
                Ok(claimed)
            }
            None => Err(ZenkoError::PolicyDenied(format!(
                "Task `{task_id}` is already claimed by another agent or in a terminal/blocked state."
            ))),
        }
    }

    fn delete_session_task(&self, session_id: &str, task_id: &str) -> Result<bool> {
        let _dag_guard = self.dag_lock.lock().unwrap_or_else(|e| e.into_inner());
        let board_sid = self.resolve_board_session_id(session_id);
        let deleted = self.store.delete_session_task(&board_sid, task_id)?;
        if deleted {
            // Re-normalize remaining tasks so dangling dependency edges are pruned cleanly
            let mut remaining = self.store.list_session_tasks(&board_sid)?;
            if validate_and_normalize_session_tasks(&mut remaining).is_ok() {
                let _ = self.store.replace_session_tasks(&board_sid, &remaining);
            }
            self.emit_tasks_updated(&board_sid);
        }
        Ok(deleted)
    }

    fn clear_session_tasks(&self, session_id: &str) -> Result<usize> {
        let _dag_guard = self.dag_lock.lock().unwrap_or_else(|e| e.into_inner());
        let board_sid = self.resolve_board_session_id(session_id);
        let removed = self.store.clear_session_tasks(&board_sid)?;
        if removed > 0 {
            self.emit_tasks_updated(&board_sid);
        }
        Ok(removed)
    }

    async fn apply_orchestrate_control(
        &self,
        session_id: &str,
        workspace_dir: &Path,
        req: OrchestrateControlRequest,
    ) -> Result<OrchestrateControlReceipt> {
        let rt_opt = self.runtime.get().and_then(Weak::upgrade);
        let mut receipt = match rt_opt.as_ref() {
            Some(rt) => rt.queue_orchestrate_control(session_id, &req),
            None => OrchestrateControlReceipt {
                posture: req.posture,
                cognitive_tier: req.cognitive_tier,
                resolved_model: None,
                reasoning_effort: req.reasoning_effort,
                zmode: req.zmode,
                concise_mode: req.concise_mode,
                minimalism_mode: req.minimalism_mode,
                recon_summary: None,
                rationale: req.rationale.clone(),
                model_locked_by_override: false,
            },
        };

        if req.refresh_recon
            && let Some(rt) = rt_opt
        {
            let cfg = rt.config();
            let query = req.recon_query.as_deref().unwrap_or("");
            let recon = run_parallel_recon(
                workspace_dir.to_path_buf(),
                cfg.resolved_hub_dir(),
                session_id.to_string(),
                query,
                rt.store.clone(),
                rt.jobs.clone(),
            )
            .await;
            let _ = self.events.send(AgentEvent::OrchestratorUpdated {
                session_id: session_id.to_string(),
                stage: zenko_core::ORCH_STAGE_RECON.into(),
                tier: zenko_core::ExecutionTier::Orchestrated,
                detail: format!(
                    "On-demand recon: {} entries, {} markers, {} prompt files, {} memory hits ({}ms)",
                    recon.workspace_entries.len(),
                    recon.repo_markers.len(),
                    recon.mentioned_files.len(),
                    recon.memory_hits.len(),
                    recon.elapsed_ms
                ),
                model: None,
            });
            let formatted = format_recon_context(&recon);
            if !formatted.is_empty() {
                receipt.recon_summary = Some(formatted);
            }
        }

        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn evidence_weights_penalize_unbacked_self_scores_and_index_resolve_works() {
        let opts = vec![
            BrainstormOption {
                id: "inflated_rewrite".into(),
                title: "Big Rewrite (No Rollback)".into(),
                summary: "Touch 5 files with inflated self-score".into(),
                pros: vec!["Fast".into()],
                cons: vec![],
                blast_radius: "medium".into(),
                reversibility: "medium".into(),
                safety_score: 10,
                testability_score: 9,
                touched_files: vec!["a.rs".into(), "b.rs".into(), "c.rs".into(), "d.rs".into()],
                rollback_steps: vec![],
                risks: vec![],
                recommended: true,
            },
            BrainstormOption {
                id: "evidence_adapter".into(),
                title: "Evidence-Backed Adapter".into(),
                summary: "Isolated behind trait with explicit rollback and risks".into(),
                pros: vec!["Reversible".into()],
                cons: vec!["Extra trait".into()],
                blast_radius: "low".into(),
                reversibility: "high".into(),
                safety_score: 8,
                testability_score: 8,
                touched_files: vec!["adapter.rs".into()],
                rollback_steps: vec!["git revert single commit".into()],
                risks: vec!["minor indirection".into()],
                recommended: false,
            },
        ];
        let best = select_optimal_option(&opts).expect("best option");
        assert_eq!(best.id, "evidence_adapter");

        // LLM-sovereign brainstorm: honors Exploring drafts (no forced selection) and Committed choices without score override
        let exploring_spec = finalize_brainstorm_spec(BrainstormSpec {
            id: "bs_explore".into(),
            stage: BrainstormStage::Exploring,
            parent_spec_id: None,
            problem: "Explore cache invalidation strategies".into(),
            invariants: vec!["No stale auth reads".into()],
            assumptions: vec!["Single-node SQLite WAL".into()],
            blind_spots: vec!["Multi-process reload behavior".into()],
            decision_criteria: vec!["Simplicity".into(), "Zero lock contention".into()],
            options: vec![opts[0].clone(), opts[1].clone()],
            selected_option_id: String::new(),
            selection_rationale: String::new(),
            critique: vec!["Big rewrite risks silent regressions".into()],
            open_questions: vec!["Do we need cross-process invalidation?".into()],
            pre_mortem: vec![],
            verification_plan: vec![],
            rubric_warnings: vec![],
            created_at: Utc::now(),
        });
        assert!(
            exploring_spec.selected_option_id.is_empty(),
            "exploring brainstorm must not force an option selection"
        );
        assert!(exploring_spec.rubric_warnings.is_empty());

        let committed_spec = finalize_brainstorm_spec(BrainstormSpec {
            id: "bs_commit".into(),
            stage: BrainstormStage::Committed,
            parent_spec_id: Some("bs_explore".into()),
            problem: "Migrate auth DB".into(),
            invariants: vec![],
            assumptions: vec![],
            blind_spots: vec![],
            decision_criteria: vec![],
            options: vec![opts[0].clone(), opts[1].clone()],
            selected_option_id: "inflated_rewrite".into(),
            selection_rationale: "User explicitly requested full rewrite.".into(),
            critique: vec![],
            open_questions: vec![],
            pre_mortem: vec![],
            verification_plan: vec![],
            rubric_warnings: vec![],
            created_at: Utc::now(),
        });
        assert_eq!(
            committed_spec.selected_option_id, "inflated_rewrite",
            "LLM-selected option must never be overridden"
        );
        assert!(
            committed_spec
                .rubric_warnings
                .iter()
                .any(|w| w.contains("rollback")),
            "wide multi-file edit without rollback should surface a neutral reflection, got {:?}",
            committed_spec.rubric_warnings
        );

        let test_dir = std::env::temp_dir().join(format!("zenko_bs_hub_{}", new_id()));
        let store = SqliteStore::open(test_dir.join("zenko.db")).expect("db");
        let (tx, _) = broadcast::channel(16);
        let hub = ClarificationHub::new(store, tx);

        // Headless auto-resolve
        let headless_ans = hub
            .request_clarification(
                "s1",
                "Storage backend?",
                "Need persistence",
                vec![
                    ClarificationOption {
                        id: "json".into(),
                        label: "Flat JSON".into(),
                        description: "Simple".into(),
                        recommended: false,
                    },
                    ClarificationOption {
                        id: "sqlite".into(),
                        label: "SQLite WAL".into(),
                        description: "Transactional".into(),
                        recommended: true,
                    },
                ],
                true,
                CancellationToken::new(),
            )
            .await
            .expect("headless answer");
        assert!(headless_ans.auto_resolved);
        assert_eq!(headless_ans.selected_option_id, "sqlite");

        // Interactive resolve by index (Telegram callback pattern)
        let hub_bg = hub.clone();
        let wait_handle = tokio::spawn(async move {
            hub_bg
                .request_clarification(
                    "s2",
                    "API protocol?",
                    "Need transport",
                    vec![
                        ClarificationOption {
                            id: "rest".into(),
                            label: "REST".into(),
                            description: "Simple HTTP".into(),
                            recommended: true,
                        },
                        ClarificationOption {
                            id: "grpc".into(),
                            label: "gRPC".into(),
                            description: "Binary RPC".into(),
                            recommended: false,
                        },
                    ],
                    false,
                    CancellationToken::new(),
                )
                .await
        });

        tokio::time::sleep(Duration::from_millis(30)).await;
        let pending = hub.list_pending_clarifications(Some("s2"));
        assert_eq!(pending.len(), 1);
        let resolved = hub
            .resolve_clarification_by_index(&pending[0].id, 1, "telegram:@owner")
            .expect("resolved by index");
        assert_eq!(resolved.status, ClarificationStatus::Answered);

        let interactive_ans = wait_handle.await.expect("join").expect("answer");
        assert!(!interactive_ans.auto_resolved);
        assert_eq!(interactive_ans.selected_option_id, "grpc");
        assert_eq!(interactive_ans.resolved_by, "telegram:@owner");
    }

    #[tokio::test]
    async fn ask_user_headless_safety_rich_resolution_and_zpowers_decisions_persistence() {
        let test_dir = std::env::temp_dir().join(format!("zenko_ask_user_{}", new_id()));
        std::fs::create_dir_all(&test_dir).expect("mkdir");
        let store = SqliteStore::open(test_dir.join("zenko.db")).expect("db");
        let (tx, _) = broadcast::channel(16);
        let hub = ClarificationHub::new(store, tx);

        // 1. Required DataInput with no defaults MUST NOT fabricate data in headless mode.
        let data_spec = AskUserRequestSpec {
            kind: AskUserKind::DataInput,
            title: "Deployment target".into(),
            question: "Which cluster URL and token should be used?".into(),
            context: "Cannot infer production target from workspace.".into(),
            selection_mode: zenko_core::AskUserSelectionMode::Single,
            options: Vec::new(),
            steps: Vec::new(),
            fields: vec![
                zenko_core::AskUserField {
                    id: "cluster_url".into(),
                    label: "Cluster URL".into(),
                    description: String::new(),
                    placeholder: "https://...".into(),
                    default_value: None,
                    required: true,
                    secret: false,
                    multiline: false,
                },
                zenko_core::AskUserField {
                    id: "api_key".into(),
                    label: "API Key".into(),
                    description: String::new(),
                    placeholder: "sk-...".into(),
                    default_value: None,
                    required: true,
                    secret: true,
                    multiline: false,
                },
            ],
            recommendation: None,
            allow_custom: true,
            allow_auto_default: false,
        };
        let headless_data = hub
            .request_ask_user("s_headless", data_spec.clone(), true, CancellationToken::new())
            .await
            .expect("headless data result");
        assert!(headless_data.needs_user_input);
        assert!(!headless_data.auto_resolved);
        assert_eq!(headless_data.resolved_by, ASK_USER_RESOLVER_NEEDS_INPUT);

        // 2. QuestionFlow headless auto-resolution picks recommended defaults cleanly.
        let flow_spec = AskUserRequestSpec {
            kind: AskUserKind::QuestionFlow,
            title: "Setup flow".into(),
            question: "Configure target runtime".into(),
            context: String::new(),
            selection_mode: zenko_core::AskUserSelectionMode::Single,
            options: Vec::new(),
            steps: vec![
                zenko_core::AskUserStep {
                    id: "db".into(),
                    question: "Database?".into(),
                    description: String::new(),
                    selection_mode: zenko_core::AskUserSelectionMode::Single,
                    options: vec![
                        ClarificationOption {
                            id: "pg".into(),
                            label: "Postgres".into(),
                            description: String::new(),
                            recommended: false,
                        },
                        ClarificationOption {
                            id: "sqlite".into(),
                            label: "SQLite".into(),
                            description: String::new(),
                            recommended: true,
                        },
                    ],
                    min_selections: None,
                    max_selections: None,
                    allow_custom: false,
                },
            ],
            fields: Vec::new(),
            recommendation: None,
            allow_custom: false,
            allow_auto_default: true,
        };
        let flow_headless = hub
            .request_ask_user("s_flow", flow_spec, true, CancellationToken::new())
            .await
            .expect("flow headless");
        assert!(!flow_headless.needs_user_input);
        assert!(flow_headless.auto_resolved);
        assert_eq!(
            flow_headless.step_answers.get("db"),
            Some(&vec!["sqlite".to_string()])
        );

        // 3. Interactive DataInput roundtrip + decisions.md persistence with secret masking.
        let hub_bg = hub.clone();
        let data_spec_bg = data_spec.clone();
        let waiter = tokio::spawn(async move {
            hub_bg
                .request_ask_user("s_interactive", data_spec_bg, false, CancellationToken::new())
                .await
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        let pending = hub.list_pending_clarifications(Some("s_interactive"));
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].kind, AskUserKind::DataInput);

        let mut field_values = std::collections::BTreeMap::new();
        field_values.insert("cluster_url".into(), "https://prod.example.internal".into());
        field_values.insert("api_key".into(), "super-secret-value-123".into());
        let resolved_rec = hub
            .resolve_ask_user(
                &pending[0].id,
                AskUserResolution {
                    selected_option_id: ASK_USER_OPTION_DATA_SUBMITTED.into(),
                    field_values,
                    ..Default::default()
                },
                "webui",
            )
            .expect("resolved");
        let interactive_ans = waiter.await.expect("join").expect("ans");
        assert!(!interactive_ans.needs_user_input);
        assert_eq!(
            interactive_ans.field_values.get("cluster_url").map(String::as_str),
            Some("https://prod.example.internal")
        );

        let dec_path = crate::zpowers::append_user_decision(
            &test_dir,
            "s_interactive",
            "Deploy task",
            None,
            &resolved_rec,
            &interactive_ans,
            &zenko_core::ZPowersConfig::default(),
        )
        .expect("append decision");
        let md = std::fs::read_to_string(dec_path).expect("read decisions.md");
        assert!(md.contains("https://prod.example.internal"));
        assert!(md.contains("[provided]"));
        assert!(!md.contains("super-secret-value-123"));
        let _ = std::fs::remove_dir_all(&test_dir);
    }

    #[tokio::test]
    async fn batch_upsert_session_tasks_emits_single_atomic_broadcast() {
        let test_dir = std::env::temp_dir().join(format!("zenko_batch_dag_{}", new_id()));
        std::fs::create_dir_all(&test_dir).expect("mkdir");
        let store = SqliteStore::open(test_dir.join("zenko.db")).expect("db");
        let sess = store
            .create_session(
                "Batch DAG test",
                "webui",
                None,
                None,
                zenko_core::PermissionMode::default(),
            )
            .expect("session");
        let sid = sess.id;
        let (tx, mut rx) = broadcast::channel(32);
        let hub = ClarificationHub::new(store, tx);

        // Create 4 tasks in a single atomic batch -> must emit exactly ONE SessionTasksUpdated.
        let creates: Vec<zenko_core::SessionTask> = (1..=4)
            .map(|i| zenko_core::SessionTask {
                id: format!("task_{i}"),
                session_id: sid.clone(),
                branch_event_id: None,
                title: format!("Step {i}"),
                description: String::new(),
                active_label: None,
                status: zenko_core::SessionTaskStatus::Pending,
                depends_on: Vec::new(),
                owner_agent: None,
                verification_cmd: None,
                verification_evidence: None,
                notes: None,
                priority: i as i32,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .collect();

        let (c1, u1, all1) = hub
            .batch_upsert_session_tasks(&sid, creates, Vec::new())
            .expect("batch create");
        assert_eq!((c1, u1, all1.len()), (4, 0, 4));

        let mut broadcasts = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let AgentEvent::SessionTasksUpdated { tasks, .. } = ev {
                broadcasts.push(tasks.len());
            }
        }
        assert_eq!(
            broadcasts,
            vec![4],
            "batch create of 4 tasks must emit exactly one SessionTasksUpdated snapshot"
        );

        // Now mixed create (1) + update (2) in one atomic batch -> still exactly ONE broadcast!
        let extra = vec![zenko_core::SessionTask {
            id: "task_5".into(),
            session_id: sid.clone(),
            branch_event_id: None,
            title: "Step 5".into(),
            description: String::new(),
            active_label: None,
            status: zenko_core::SessionTaskStatus::Pending,
            depends_on: vec!["task_1".into()],
            owner_agent: None,
            verification_cmd: None,
            verification_evidence: None,
            notes: None,
            priority: 5,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }];
        let updates = vec![
            (
                "task_1".to_string(),
                zenko_core::SessionTaskPatch {
                    status: Some(zenko_core::SessionTaskStatus::Done),
                    ..Default::default()
                },
            ),
            (
                "task_2".to_string(),
                zenko_core::SessionTaskPatch {
                    status: Some(zenko_core::SessionTaskStatus::InProgress),
                    ..Default::default()
                },
            ),
        ];
        let (c2, u2, all2) = hub
            .batch_upsert_session_tasks(&sid, extra, updates)
            .expect("batch mixed");
        assert_eq!((c2, u2, all2.len()), (1, 2, 5));
        assert_eq!(
            all2.iter().find(|t| t.id == "task_1").map(|t| t.status),
            Some(zenko_core::SessionTaskStatus::Done)
        );
        assert_eq!(
            all2.iter().find(|t| t.id == "task_2").map(|t| t.status),
            Some(zenko_core::SessionTaskStatus::InProgress)
        );

        let mut broadcasts2 = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let AgentEvent::SessionTasksUpdated { tasks, .. } = ev {
                broadcasts2.push(tasks.len());
            }
        }
        assert_eq!(
            broadcasts2,
            vec![5],
            "mixed batch create+update must emit exactly one SessionTasksUpdated snapshot"
        );
        let _ = std::fs::remove_dir_all(&test_dir);
    }
}
