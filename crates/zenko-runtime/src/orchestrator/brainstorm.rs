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
    AgentEvent, BrainstormOption, BrainstormSpec, BrainstormStage, CLARIFICATION_RESOLVER_HEADLESS,
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

    fn emit_tasks_updated(&self, board_sid: &str) {
        if let Ok(tasks) = self.store.list_session_tasks(board_sid) {
            let _ = self.events.send(AgentEvent::SessionTasksUpdated {
                session_id: board_sid.to_string(),
                tasks,
            });
        }
    }

    pub fn resolve_clarification(
        &self,
        clarification_id: &str,
        selected_option_id: &str,
        custom_answer: Option<String>,
        resolved_by: &str,
    ) -> Option<ClarificationRecord> {
        let entry = self
            .pending
            .lock()
            .ok()
            .and_then(|mut map| map.remove(clarification_id))?;

        let mut rec = entry.record;
        rec.status = ClarificationStatus::Answered;
        rec.selected_option_id = Some(selected_option_id.to_string());
        rec.custom_answer = custom_answer.clone();
        rec.resolved_by = Some(resolved_by.to_string());
        rec.resolved_at = Some(Utc::now());

        let answer = ClarificationAnswer {
            selected_option_id: selected_option_id.to_string(),
            custom_answer: custom_answer.clone(),
            resolved_by: resolved_by.to_string(),
            auto_resolved: false,
        };
        let _ = entry.tx.send(answer);
        let _ = self.events.send(AgentEvent::ClarificationResolved {
            clarification_id: rec.id.clone(),
            session_id: rec.session_id.clone(),
            selected_option_id: selected_option_id.to_string(),
            custom_answer,
            resolved_by: resolved_by.to_string(),
        });
        Some(rec)
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
        let _ = self.events.send(AgentEvent::PlanUpdated {
            session_id: session_id.to_string(),
            plan: normalized.clone(),
        });
        Ok(normalized)
    }

    async fn request_clarification(
        &self,
        session_id: &str,
        question: &str,
        context: &str,
        options: Vec<ClarificationOption>,
        headless_no_wait: bool,
        cancel: CancellationToken,
    ) -> Result<ClarificationAnswer> {
        let (rec_opt_id, auto_note) = resolve_headless_clarification(&options);
        if headless_no_wait {
            let cid = new_id();
            let _ = self.events.send(AgentEvent::ClarificationResolved {
                clarification_id: cid,
                session_id: session_id.to_string(),
                selected_option_id: rec_opt_id.clone(),
                custom_answer: Some(auto_note.clone()),
                resolved_by: CLARIFICATION_RESOLVER_HEADLESS.to_string(),
            });
            return Ok(ClarificationAnswer {
                selected_option_id: rec_opt_id,
                custom_answer: Some(auto_note),
                resolved_by: CLARIFICATION_RESOLVER_HEADLESS.to_string(),
                auto_resolved: true,
            });
        }

        let id = new_id();
        let record = ClarificationRecord {
            id: id.clone(),
            session_id: session_id.to_string(),
            question: question.to_string(),
            context: context.to_string(),
            options,
            recommended_option_id: rec_opt_id.clone(),
            status: ClarificationStatus::Pending,
            selected_option_id: None,
            custom_answer: None,
            resolved_by: None,
            created_at: Utc::now(),
            resolved_at: None,
        };

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
            clarification: record,
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
                    let _ = self.events.send(AgentEvent::ClarificationResolved {
                        clarification_id: id,
                        session_id: session_id.to_string(),
                        selected_option_id: rec_opt_id.clone(),
                        custom_answer: Some("Timed out waiting for human response; proceeding with recommended option.".into()),
                        resolved_by: CLARIFICATION_RESOLVER_TIMEOUT.to_string(),
                    });
                    Ok(ClarificationAnswer {
                        selected_option_id: rec_opt_id,
                        custom_answer: Some("Timed out; proceeded with recommended option.".into()),
                        resolved_by: CLARIFICATION_RESOLVER_TIMEOUT.to_string(),
                        auto_resolved: true,
                    })
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
}
