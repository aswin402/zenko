//! Deterministic DAG Execution Plan validator, topological invariant propagator & continuity loader.
//! Uses Kahn's topological sort algorithm ($O(V + E)$) to guarantee cycle-free dependencies,
//! transitive blocked propagation, and prerequisite completion ordering (zero regex, zero `if/else` ladders).

use std::collections::{HashMap, HashSet, VecDeque};
use zenko_core::{
    EventRole, ExecutionPlan, NOTE_KIND_EXECUTION_PLAN, ORCH_PLAN_MAX_STEPS, PlanStep,
    PlanStepStatus, Result, SESSION_TASK_PROMPT_MAX_ITEMS, SessionEvent, SessionTask,
    SessionTaskStatus, ZenkoError,
};

/// Shared $O(V + E)$ Kahn's topological sort used by both `ExecutionPlan` and `SessionTask` DAGs.
/// Returns node indices in valid topological order or `ZenkoError::Tool` when any cycle exists.
fn kahn_topological_order<T, FId, FDeps>(
    items: &[T],
    get_id: FId,
    get_deps: FDeps,
    cycle_err: &str,
) -> Result<Vec<usize>>
where
    FId: Fn(&T) -> &str,
    FDeps: Fn(&T) -> &[String],
{
    let id_to_idx: HashMap<&str, usize> = items
        .iter()
        .enumerate()
        .map(|(i, item)| (get_id(item), i))
        .collect();
    let mut in_degree: Vec<usize> = items.iter().map(|item| get_deps(item).len()).collect();
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); items.len()];
    for (idx, item) in items.iter().enumerate() {
        for dep in get_deps(item) {
            if let Some(&dep_idx) = id_to_idx.get(dep.as_str()) {
                dependents[dep_idx].push(idx);
            }
        }
    }

    let mut queue: VecDeque<usize> = in_degree
        .iter()
        .enumerate()
        .filter(|(_, deg)| **deg == 0)
        .map(|(i, _)| i)
        .collect();
    let mut topo_order: Vec<usize> = Vec::with_capacity(items.len());
    while let Some(node) = queue.pop_front() {
        topo_order.push(node);
        for &child in &dependents[node] {
            in_degree[child] = in_degree[child].saturating_sub(1);
            if in_degree[child] == 0 {
                queue.push_back(child);
            }
        }
    }

    match topo_order.len() == items.len() {
        true => Ok(topo_order),
        false => Err(ZenkoError::Tool(cycle_err.into())),
    }
}

/// Validates, normalizes, checks acyclicity via Kahn's algorithm, and propagates
/// dependency invariants strictly in topological order ($O(V + E)$).
pub fn validate_and_normalize_plan(mut plan: ExecutionPlan) -> Result<ExecutionPlan> {
    plan.goal = plan.goal.trim().to_string();
    if plan.goal.is_empty() {
        return Err(ZenkoError::Tool(
            "execution plan requires a non-empty `goal`".into(),
        ));
    }

    plan.steps.truncate(ORCH_PLAN_MAX_STEPS);
    if plan.steps.is_empty() {
        return Err(ZenkoError::Tool(
            "execution plan requires at least 1 step".into(),
        ));
    }

    // 1. Deduplicate and normalize step IDs deterministically
    let mut seen_ids: HashSet<String> = HashSet::with_capacity(plan.steps.len());
    for (idx, step) in plan.steps.iter_mut().enumerate() {
        let base = match step.id.trim().is_empty() {
            true => format!("step_{}", idx + 1),
            false => step.id.trim().to_string(),
        };
        let mut unique = base.clone();
        let mut suffix = 2usize;
        while !seen_ids.insert(unique.clone()) {
            unique = format!("{base}_{suffix}");
            suffix += 1;
        }
        step.id = unique;
        step.title = step.title.trim().to_string();
    }

    // 2. Strip self-dependencies and unknown dependency references cleanly
    for step in &mut plan.steps {
        let own_id = step.id.clone();
        let mut uniq_deps: HashSet<String> = HashSet::new();
        step.depends_on.retain(|d| {
            let trimmed = d.trim();
            !trimmed.is_empty()
                && trimmed != own_id
                && seen_ids.contains(trimmed)
                && uniq_deps.insert(trimmed.to_string())
        });
    }

    // 3. Kahn's topological sort (records exact `topo_order` for transitive propagation)
    let topo_order = kahn_topological_order(
        &plan.steps,
        |s| s.id.as_str(),
        |s| &s.depends_on,
        "execution plan contains a cyclic `depends_on` dependency graph",
    )?;

    // 4. Propagate DAG invariants strictly in topological order (handles arbitrary multi-hop depth):
    //    - Any step with a `Blocked` ancestor transitions to `Blocked` transitively.
    //    - A step cannot be `Completed` while a prerequisite dependency is still `Pending` or `InProgress`.
    let mut live_status: HashMap<String, PlanStepStatus> = HashMap::with_capacity(plan.steps.len());
    for idx in topo_order {
        let step = &mut plan.steps[idx];
        let has_blocked_dep = step
            .depends_on
            .iter()
            .any(|d| matches!(live_status.get(d), Some(PlanStepStatus::Blocked)));
        let has_incomplete_dep = step.depends_on.iter().any(|d| {
            matches!(
                live_status.get(d),
                Some(PlanStepStatus::Pending | PlanStepStatus::InProgress)
            )
        });
        step.status = match (step.status, has_blocked_dep, has_incomplete_dep) {
            (_, true, _) => PlanStepStatus::Blocked,
            (PlanStepStatus::Completed, false, true) => PlanStepStatus::InProgress,
            (cur, false, _) => cur,
        };
        live_status.insert(step.id.clone(), step.status);
    }

    plan.rubric_warnings = audit_plan_rubric(&plan);

    Ok(plan)
}

/// Loads the latest unfinished `ExecutionPlan` persisted on an active session branch
/// so follow-up turns and post-compaction iterations never lose multi-step plan state.
pub fn load_latest_active_plan(events: &[SessionEvent]) -> Option<ExecutionPlan> {
    for ev in events.iter().rev() {
        if ev.role != EventRole::System {
            continue;
        }
        if ev.content_json.get("note_kind").and_then(|v| v.as_str())
            != Some(NOTE_KIND_EXECUTION_PLAN)
        {
            continue;
        }
        if let Some(plan) = ev
            .content_json
            .get("plan")
            .and_then(|v| serde_json::from_value::<ExecutionPlan>(v.clone()).ok())
        {
            return match incomplete_steps(&plan).is_empty() {
                true => None,
                false => Some(plan),
            };
        }
    }
    None
}

/// Returns `(completed, in_progress, total)` step counts.
pub fn plan_progress(plan: &ExecutionPlan) -> (usize, usize, usize) {
    let mut completed = 0usize;
    let mut in_progress = 0usize;
    for step in &plan.steps {
        match step.status {
            PlanStepStatus::Completed => completed += 1,
            PlanStepStatus::InProgress => in_progress += 1,
            PlanStepStatus::Pending | PlanStepStatus::Blocked => {}
        }
    }
    (completed, in_progress, plan.steps.len())
}

/// Returns steps that are currently `Pending` or `InProgress` (unfinished active work).
pub fn incomplete_steps(plan: &ExecutionPlan) -> Vec<&PlanStep> {
    plan.steps
        .iter()
        .filter(|s| matches!(s.status, PlanStepStatus::Pending | PlanStepStatus::InProgress))
        .collect()
}

/// Deterministic Momus-style quality rubric auditing multi-step DAG execution plans (0 LLM tokens).
pub fn audit_plan_rubric(plan: &ExecutionPlan) -> Vec<String> {
    let mut warnings = Vec::new();
    if plan.steps.len() >= 3 && !plan.steps.iter().any(|s| s.verification.is_some()) {
        warnings.push(
            "Multi-step plan has no `verification` check attached to any step; add concrete verification criteria to final steps.".into(),
        );
    }
    if plan.steps.iter().any(|s| s.title.chars().count() < 6) {
        warnings.push("One or more plan step titles are overly terse; prefer concrete file/action descriptions.".into());
    }
    warnings
}

/// Formats a compact 1-line summary for zero-token recon and compaction continuity.
pub fn format_plan_summary(plan: &ExecutionPlan) -> String {
    let (done, active, total) = plan_progress(plan);
    let next = plan
        .steps
        .iter()
        .find(|s| matches!(s.status, PlanStepStatus::InProgress | PlanStepStatus::Pending))
        .map(|s| format!("; next=`{}: {}`", s.id, s.title))
        .unwrap_or_default();
    format!(
        "goal=\"{}\" ({done}/{total} done, {active} in_progress{next})",
        plan.goal
    )
}

/// Validates and normalizes durable SQLite `SessionTask` DAG nodes in $O(V + E)$ via Kahn's
/// topological sort:
/// - Rejects any cyclic `depends_on` graph.
/// - Strips self-loops and unknown dangling IDs.
/// - Propagates `Blocked` transitively whenever any upstream prerequisite is `Blocked` or `Failed`.
/// - Prevents marking a node `Done` while an upstream prerequisite is still incomplete.
pub fn validate_and_normalize_session_tasks(tasks: &mut [SessionTask]) -> Result<()> {
    let valid_ids: HashSet<String> = tasks.iter().map(|t| t.id.clone()).collect();
    for task in tasks.iter_mut() {
        task.title = task.title.trim().to_string();
        if task.title.is_empty() {
            return Err(ZenkoError::Tool(format!(
                "session task `{}` requires a non-empty `title`",
                task.id
            )));
        }
        let own = task.id.clone();
        let mut uniq: HashSet<String> = HashSet::new();
        task.depends_on.retain(|d| {
            let trimmed = d.trim();
            !trimmed.is_empty()
                && trimmed != own
                && valid_ids.contains(trimmed)
                && uniq.insert(trimmed.to_string())
        });
    }

    let topo_order = kahn_topological_order(
        tasks,
        |t| t.id.as_str(),
        |t| &t.depends_on,
        "session task DAG contains a cyclic `depends_on` dependency graph",
    )?;

    let mut live_status: HashMap<String, SessionTaskStatus> = HashMap::with_capacity(tasks.len());
    for idx in topo_order {
        let task = &mut tasks[idx];
        let has_blocked_dep = task.depends_on.iter().any(|d| {
            matches!(
                live_status.get(d),
                Some(SessionTaskStatus::Blocked | SessionTaskStatus::Failed)
            )
        });
        let has_incomplete_dep = task.depends_on.iter().any(|d| {
            matches!(
                live_status.get(d),
                Some(SessionTaskStatus::Pending | SessionTaskStatus::InProgress)
            )
        });
        task.status = match (task.status, has_blocked_dep, has_incomplete_dep) {
            (_, true, _) => SessionTaskStatus::Blocked,
            (SessionTaskStatus::Done, false, true) => SessionTaskStatus::InProgress,
            (cur, false, _) => cur,
        };
        live_status.insert(task.id.clone(), task.status);
    }
    Ok(())
}

/// Computes the exact topological Ready Set: `Pending` tasks whose every upstream dependency is `Done`.
pub fn compute_ready_session_tasks(tasks: &[SessionTask]) -> Vec<SessionTask> {
    let status_map: HashMap<&str, SessionTaskStatus> =
        tasks.iter().map(|t| (t.id.as_str(), t.status)).collect();
    let mut ready: Vec<SessionTask> = tasks
        .iter()
        .filter(|t| {
            t.status == SessionTaskStatus::Pending
                && t.depends_on
                    .iter()
                    .all(|dep| status_map.get(dep.as_str()) == Some(&SessionTaskStatus::Done))
        })
        .cloned()
        .collect();
    ready.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.created_at.cmp(&b.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    ready
}

/// Formats a compact, KV-cache-safe `<active_task_dag>` tail block so multi-hour tasks survive
/// Stage-B compaction and daemon restarts without invalidating Tier 1/2 cached prompts.
pub fn format_session_tasks_prompt_block(tasks: &[SessionTask]) -> Option<String> {
    if tasks.is_empty() {
        return None;
    }
    let done = tasks
        .iter()
        .filter(|t| t.status == SessionTaskStatus::Done)
        .count();
    let ready_ids: HashSet<String> = compute_ready_session_tasks(tasks)
        .into_iter()
        .map(|t| t.id)
        .collect();
    if done == tasks.len() {
        return None;
    }
    let mut lines = Vec::with_capacity(tasks.len().min(SESSION_TASK_PROMPT_MAX_ITEMS) + 2);
    lines.push(format!(
        "<active_task_dag completed=\"{done}\" total=\"{}\" ready=\"{}\">",
        tasks.len(),
        ready_ids.len()
    ));
    for t in tasks.iter().take(SESSION_TASK_PROMPT_MAX_ITEMS) {
        let ready_tag = match ready_ids.contains(&t.id) {
            true => " [READY]",
            false => "",
        };
        let owner_tag = t
            .owner_agent
            .as_deref()
            .map(|o| format!(" owner={o}"))
            .unwrap_or_default();
        let deps_tag = match t.depends_on.is_empty() {
            true => String::new(),
            false => format!(" deps=[{}]", t.depends_on.join(",")),
        };
        let verify_tag = t
            .verification_cmd
            .as_deref()
            .map(|v| format!(" verify=`{v}`"))
            .unwrap_or_default();
        lines.push(format!(
            "- `{}` [{}]{ready_tag}: {}{owner_tag}{deps_tag}{verify_tag}",
            t.id,
            t.status.as_str(),
            t.title
        ));
    }
    if tasks.len() > SESSION_TASK_PROMPT_MAX_ITEMS {
        lines.push(format!(
            "... (+{} more tasks; call `session_task(action=\"list\")`)",
            tasks.len() - SESSION_TASK_PROMPT_MAX_ITEMS
        ));
    }
    lines.push("</active_task_dag>".into());
    Some(lines.join("\n"))
}

/// Projects durable SQLite `SessionTask`s into an `ExecutionPlan` snapshot for `ExecutionGuard`.
pub fn execution_plan_from_session_tasks(goal: &str, tasks: &[SessionTask]) -> Option<ExecutionPlan> {
    if tasks.is_empty() {
        return None;
    }
    let steps: Vec<PlanStep> = tasks
        .iter()
        .map(|t| PlanStep {
            id: t.id.clone(),
            title: t.title.clone(),
            status: t.status.to_plan_status(),
            depends_on: t.depends_on.clone(),
            verification: t.verification_cmd.clone(),
        })
        .collect();
    let updated_at = tasks
        .iter()
        .map(|t| t.updated_at)
        .max()
        .unwrap_or_else(chrono::Utc::now);
    Some(ExecutionPlan {
        id: "sqlite_dag".into(),
        goal: goal.to_string(),
        steps,
        rubric_warnings: Vec::new(),
        updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_step(id: &str, status: PlanStepStatus, deps: &[&str]) -> PlanStep {
        PlanStep {
            id: id.into(),
            title: format!("Step {id}"),
            status,
            depends_on: deps.iter().map(|s| (*s).to_string()).collect(),
            verification: None,
        }
    }

    #[test]
    fn rejects_cyclic_dag_and_normalizes_valid_plan() {
        let cyclic = ExecutionPlan {
            id: "p_cyc".into(),
            goal: "Cyclic plan".into(),
            steps: vec![
                sample_step("a", PlanStepStatus::Pending, &["b"]),
                sample_step("b", PlanStepStatus::Pending, &["a"]),
            ],
            rubric_warnings: vec![],
            updated_at: Utc::now(),
        };
        assert!(validate_and_normalize_plan(cyclic).is_err());

        let valid = ExecutionPlan {
            id: "p_ok".into(),
            goal: "Implement feature".into(),
            steps: vec![
                sample_step("s1", PlanStepStatus::Completed, &[]),
                sample_step("s2", PlanStepStatus::InProgress, &["s1", "nonexistent"]),
                sample_step("s3", PlanStepStatus::Pending, &["s2"]),
            ],
            rubric_warnings: vec![],
            updated_at: Utc::now(),
        };
        let norm = validate_and_normalize_plan(valid).expect("valid DAG");
        assert_eq!(norm.steps[1].depends_on, vec!["s1"]);
        assert_eq!(plan_progress(&norm), (1, 1, 3));
        assert_eq!(incomplete_steps(&norm).len(), 2);
        // 3-step plan with no verification criteria triggers Momus plan rubric warning automatically
        assert!(
            !norm.rubric_warnings.is_empty(),
            "expected Momus plan rubric warning for missing verification"
        );
    }

    #[test]
    fn propagates_blocked_transitively_and_enforces_prerequisite_completion() {
        // Multi-hop transitive cascade even when declared out of topological order
        let plan = ExecutionPlan {
            id: "p_blk".into(),
            goal: "Multi-hop cascade".into(),
            steps: vec![
                sample_step("s3", PlanStepStatus::InProgress, &["s2"]),
                sample_step("s2", PlanStepStatus::Pending, &["s1"]),
                sample_step("s1", PlanStepStatus::Blocked, &[]),
            ],
            rubric_warnings: vec![],
            updated_at: Utc::now(),
        };
        let norm = validate_and_normalize_plan(plan).expect("normalized");
        assert_eq!(norm.steps[0].status, PlanStepStatus::Blocked);
        assert_eq!(norm.steps[1].status, PlanStepStatus::Blocked);
        assert_eq!(norm.steps[2].status, PlanStepStatus::Blocked);

        // Prerequisite invariant: cannot mark child Completed while parent is Pending
        let prereq = ExecutionPlan {
            id: "p_pre".into(),
            goal: "Prereq order".into(),
            steps: vec![
                sample_step("a", PlanStepStatus::Pending, &[]),
                sample_step("b", PlanStepStatus::Completed, &["a"]),
            ],
            rubric_warnings: vec![],
            updated_at: Utc::now(),
        };
        let norm_pre = validate_and_normalize_plan(prereq).expect("normalized");
        assert_eq!(norm_pre.steps[1].status, PlanStepStatus::InProgress);
    }

    #[test]
    fn session_task_dag_ready_queue_cycle_guard_and_failed_cascade() {
        let now = Utc::now();
        let make = |id: &str, status: SessionTaskStatus, deps: &[&str], prio: i32| SessionTask {
            id: id.into(),
            session_id: "s1".into(),
            branch_event_id: None,
            title: format!("Task {id}"),
            description: String::new(),
            active_label: None,
            status,
            depends_on: deps.iter().map(|s| (*s).to_string()).collect(),
            owner_agent: None,
            verification_cmd: None,
            verification_evidence: None,
            notes: None,
            priority: prio,
            created_at: now,
            updated_at: now,
        };

        // Cycle must be rejected
        let mut cyclic = vec![
            make("a", SessionTaskStatus::Pending, &["b"], 0),
            make("b", SessionTaskStatus::Pending, &["a"], 0),
        ];
        assert!(validate_and_normalize_session_tasks(&mut cyclic).is_err());

        // Ready-set computes only Pending tasks whose upstream deps are all Done
        let mut dag = vec![
            make("t1", SessionTaskStatus::Done, &[], 5),
            make("t2", SessionTaskStatus::Pending, &["t1"], 10),
            make("t3", SessionTaskStatus::Pending, &["t2"], 20),
            make("t4", SessionTaskStatus::Failed, &[], 1),
            make("t5", SessionTaskStatus::Pending, &["t4"], 99),
        ];
        validate_and_normalize_session_tasks(&mut dag).expect("valid dag");
        assert_eq!(dag[4].status, SessionTaskStatus::Blocked);
        let ready = compute_ready_session_tasks(&dag);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "t2");
        let block = format_session_tasks_prompt_block(&dag).expect("block");
        assert!(block.contains("<active_task_dag"));
        assert!(block.contains("[READY]"));
    }
}
