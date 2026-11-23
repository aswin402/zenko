use super::*;

// ─── Memory, Skills, Cron & Logs API ──────────────────────────────────────────

#[derive(Deserialize)]
pub struct SessionSearchQuery {
    q: Option<String>,
    around_event_id: Option<String>,
    window: Option<usize>,
    since: Option<String>,
    channel: Option<String>,
    workspace: Option<String>,
    role: Option<String>,
    session_id: Option<String>,
    limit: Option<usize>,
}

pub async fn api_search_sessions(
    State(state): State<WebState>,
    Query(q): Query<SessionSearchQuery>,
) -> Response {
    if let Some(anchor) = q
        .around_event_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let win = q.window.unwrap_or(SESSION_CONTEXT_DEFAULT_WINDOW);
        return match state.runtime.store.get_session_event_context(anchor, win) {
            Ok(Some(ctx)) => Json(json!({ "context": ctx })).into_response(),
            Ok(None) => (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("event `{anchor}` not found")})),
            )
                .into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        };
    }
    let query = q.q.unwrap_or_default();
    let since = q.since.as_deref().and_then(parse_since_window);
    let workspace_dir = match q.workspace.as_deref().unwrap_or("all").trim() {
        "" | "all" => None,
        "current" => Some(
            resolve_git_root(&state.runtime.config().resolved_workspace_dir())
                .display()
                .to_string(),
        ),
        other => Some(
            resolve_git_root(&std::path::PathBuf::from(other))
                .display()
                .to_string(),
        ),
    };
    let filter = SessionSearchFilter {
        query,
        since,
        channel: q.channel.filter(|s| !s.trim().is_empty()),
        workspace_dir,
        session_id: q.session_id.filter(|s| !s.trim().is_empty()),
        exclude_session_id: None,
        role: q.role.filter(|s| !s.trim().is_empty()),
        limit: q.limit,
    };
    match state.runtime.store.search_sessions_fts(&filter) {
        Ok(hits) => Json(json!({ "hits": hits })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
    scope: Option<String>,
}

pub async fn api_get_memory(
    State(state): State<WebState>,
    Query(q): Query<SearchQuery>,
) -> Response {
    let snap = match state.runtime.snapshots.load_snapshot() {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };
    let cfg = state.runtime.config();
    let ws = cfg.resolved_workspace_dir();
    let proj_scope = project_scope_for_workspace(&ws);
    let (scope_opt, include_global) = match q
        .scope
        .as_deref()
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
    let ws_check = match cfg.memory.verify_file_staleness {
        true => Some(ws.as_path()),
        false => None,
    };
    let hits = state
        .runtime
        .store
        .search_memory_hybrid(
            q.q.as_deref().unwrap_or(""),
            scope_opt,
            include_global,
            ws_check,
            60,
        )
        .unwrap_or_default();
    let entries: Vec<_> = hits.iter().map(|h| h.entry.clone()).collect();
    let dream = state.runtime.dream_status();
    let history = state
        .runtime
        .snapshots
        .list_doc_history(None)
        .unwrap_or_default();
    let skill_history = state
        .runtime
        .snapshots
        .list_skill_history(None)
        .unwrap_or_default();
    Json(json!({
        "snapshot": snap,
        "entries": entries,
        "hits": hits,
        "project_scope": proj_scope,
        "memory_kinds": MEMORY_KINDS,
        "dream": dream,
        "history": history,
        "skill_history": skill_history,
        "skills_config": cfg.skills,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct SaveContentReq {
    content: String,
}

pub async fn api_save_doc(
    State(state): State<WebState>,
    Path(name): Path<String>,
    Json(req): Json<SaveContentReq>,
) -> Response {
    let _ = state
        .runtime
        .snapshots
        .snapshot_doc_before_edit(&name, "webui");
    match state.runtime.snapshots.write_doc(&name, &req.content) {
        Ok(()) => {
            state
                .runtime
                .broadcast_memory_delta(format!("{name} was edited via WebUI."));
            Json(json!({"ok": true})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_list_skills(State(state): State<WebState>) -> Response {
    match state.runtime.snapshots.list_skills() {
        Ok(skills) => Json(skills).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_save_skill(
    State(state): State<WebState>,
    Path(slug): Path<String>,
    Json(req): Json<SaveContentReq>,
) -> Response {
    let cfg = state.runtime.config().skills;
    let secrets = state.runtime.vault.all_values();
    match state.runtime.snapshots.save_skill_with_secrets(
        &slug,
        &req.content,
        "webui",
        &secrets,
        cfg.max_history_snapshots,
    ) {
        Ok((sk, snap)) => {
            state.runtime.emit(AgentEvent::SkillUpdated {
                slug: sk.slug.clone(),
                action: "saved".into(),
                status: sk.status.as_str().into(),
            });
            Json(json!({"skill": sk, "snapshot": snap})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

#[derive(Deserialize)]
pub struct ToggleSkillReq {
    enabled: bool,
}

pub async fn api_toggle_skill(
    State(state): State<WebState>,
    Path(slug): Path<String>,
    Json(req): Json<ToggleSkillReq>,
) -> Response {
    let max_h = state.runtime.config().skills.max_history_snapshots;
    match state
        .runtime
        .snapshots
        .toggle_skill(&slug, req.enabled, max_h)
    {
        Ok((sk, snap)) => {
            state.runtime.emit(AgentEvent::SkillUpdated {
                slug: sk.slug.clone(),
                action: "toggled".into(),
                status: sk.status.as_str().into(),
            });
            Json(json!({"skill": sk, "snapshot": snap})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_approve_skill(
    State(state): State<WebState>,
    Path(slug): Path<String>,
) -> Response {
    let max_h = state.runtime.config().skills.max_history_snapshots;
    match state.runtime.snapshots.approve_skill(&slug, max_h) {
        Ok((sk, snap)) => {
            state.runtime.emit(AgentEvent::SkillUpdated {
                slug: sk.slug.clone(),
                action: "approved".into(),
                status: sk.status.as_str().into(),
            });
            Json(json!({"skill": sk, "snapshot": snap})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

#[derive(Deserialize)]
pub struct SkillHistoryQuery {
    slug: Option<String>,
}

pub async fn api_list_skill_history(
    State(state): State<WebState>,
    Query(q): Query<SkillHistoryQuery>,
) -> Response {
    match state
        .runtime
        .snapshots
        .list_skill_history(q.slug.as_deref())
    {
        Ok(items) => Json(items).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct SkillRollbackReq {
    snapshot_id: String,
}

pub async fn api_rollback_skill(
    State(state): State<WebState>,
    Json(req): Json<SkillRollbackReq>,
) -> Response {
    let max_h = state.runtime.config().skills.max_history_snapshots;
    match state
        .runtime
        .snapshots
        .rollback_skill(&req.snapshot_id, max_h)
    {
        Ok((sk, diff)) => {
            state.runtime.emit(AgentEvent::SkillUpdated {
                slug: sk.slug.clone(),
                action: "rollback".into(),
                status: sk.status.as_str().into(),
            });
            Json(json!({"ok": true, "skill": sk, "diff": diff})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_delete_skill(
    State(state): State<WebState>,
    Path(slug): Path<String>,
) -> Response {
    match state.runtime.snapshots.delete_skill(&slug) {
        Ok(ok) => Json(json!({"deleted": ok})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct AddMemoryReq {
    title: String,
    content: String,
    kind: Option<String>,
    scope: Option<String>,
}

pub async fn api_add_memory(
    State(state): State<WebState>,
    Json(req): Json<AddMemoryReq>,
) -> Response {
    let cfg = state.runtime.config();
    let scope = match req.scope.as_deref().unwrap_or(MEMORY_SCOPE_GLOBAL).trim() {
        "project" => project_scope_for_workspace(&cfg.resolved_workspace_dir()),
        "" => MEMORY_SCOPE_GLOBAL.to_string(),
        other => other.to_string(),
    };
    let secrets = state.runtime.vault.all_values();
    match state.runtime.store.upsert_memory_dedup_with_secrets(
        &scope,
        req.kind.as_deref().unwrap_or("fact"),
        &req.title,
        &req.content,
        1.0,
        None,
        None,
        cfg.memory.dedup_enabled,
        &secrets,
    ) {
        Ok(res) => {
            state.runtime.broadcast_memory_delta(format!(
                "Memory entry `{}` added/merged via WebUI.",
                res.entry.title
            ));
            Json(res.entry).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UpdateMemoryReq {
    title: Option<String>,
    content: Option<String>,
    kind: Option<String>,
    salience: Option<f64>,
    scope: Option<String>,
}

pub async fn api_update_memory(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateMemoryReq>,
) -> Response {
    let resolved_scope = req.scope.as_deref().map(|sc| match sc.trim() {
        "project" => project_scope_for_workspace(&state.runtime.config().resolved_workspace_dir()),
        "" => MEMORY_SCOPE_GLOBAL.to_string(),
        other => other.to_string(),
    });
    match state.runtime.store.update_memory(
        &id,
        req.title.as_deref(),
        req.content.as_deref(),
        req.kind.as_deref(),
        req.salience,
        resolved_scope.as_deref(),
    ) {
        Ok(Some(entry)) => Json(entry).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("memory `{id}` not found")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_delete_memory(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match state.runtime.store.delete_memory(&id) {
        Ok(ok) => Json(json!({"deleted": ok})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_get_dream_status(
    State(state): State<WebState>,
) -> Response {
    Json(state.runtime.dream_status()).into_response()
}

#[derive(Deserialize, Default)]
pub struct RunDreamReq {
    #[serde(default)]
    force: bool,
}

pub async fn api_run_dream(
    State(state): State<WebState>,
    req: Option<Json<RunDreamReq>>,
) -> Response {
    let force = req.map(|j| j.force).unwrap_or(true);
    match state.runtime.run_dream_now("webui", force).await {
        Ok(rec) => Json(rec).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    doc: Option<String>,
}

pub async fn api_list_memory_history(
    State(state): State<WebState>,
    Query(q): Query<HistoryQuery>,
) -> Response {
    match state.runtime.snapshots.list_doc_history(q.doc.as_deref()) {
        Ok(items) => Json(json!({ "snapshots": items })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct RollbackReq {
    snapshot_id: String,
}

pub async fn api_rollback_memory_doc(
    State(state): State<WebState>,
    Json(req): Json<RollbackReq>,
) -> Response {
    match state.runtime.snapshots.rollback_doc(&req.snapshot_id) {
        Ok((doc, diff)) => Json(json!({"ok": true, "doc": doc, "diff": diff})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_get_automation(State(state): State<WebState>) -> Response {
    let cfg = state.runtime.config();
    let jobs = state.runtime.store.list_cron_jobs().unwrap_or_default();
    let goals = state
        .runtime
        .store
        .list_goal_loops(GOAL_LOOP_LIST_DEFAULT_LIMIT)
        .unwrap_or_default();
    let triggers = state.runtime.list_triggers();
    Json(json!({
        "jobs": jobs,
        "goals": goals,
        "triggers": triggers,
        "webhooks_enabled": cfg.webhooks.enabled,
        "webhook_rules": cfg.webhooks.rules,
        "watchers": cfg.watchers,
        "policy": cfg.policy,
        "hooks": cfg.hooks,
    }))
    .into_response()
}

pub async fn api_list_goals(State(state): State<WebState>) -> Response {
    match state
        .runtime
        .store
        .list_goal_loops(GOAL_LOOP_LIST_DEFAULT_LIMIT)
    {
        Ok(goals) => Json(goals).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_create_goal(
    State(state): State<WebState>,
    Json(req): Json<GoalLoopCreateRequest>,
) -> Response {
    match state.runtime.create_goal_loop(req) {
        Ok(goal) => (StatusCode::CREATED, Json(goal)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_get_goal(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match state.runtime.store.get_goal_loop(&id) {
        Ok(Some(goal)) => {
            let runs = state
                .runtime
                .store
                .list_goal_loop_runs(&goal.id, GOAL_LOOP_RUNS_DEFAULT_LIMIT)
                .unwrap_or_default();
            Json(json!({"goal": goal, "runs": runs})).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("goal loop `{id}` not found")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_update_goal(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Json(patch): Json<GoalLoopPatch>,
) -> Response {
    match state.runtime.update_goal_loop(&id, patch) {
        Ok(Some(goal)) => Json(goal).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("goal loop `{id}` not found")})),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_run_goal_now(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match zenko_runtime::execute_goal_iteration(
        &state.runtime,
        &id,
        state.shutdown.child_token(),
    )
    .await
    {
        Ok(goal) => Json(goal).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_delete_goal(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match state.runtime.delete_goal_loop(&id) {
        Ok(deleted) => Json(json!({"deleted": deleted})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_list_triggers(State(state): State<WebState>) -> Response {
    Json(state.runtime.list_triggers()).into_response()
}

pub async fn api_upsert_webhook_rule(
    State(state): State<WebState>,
    Json(mut rule): Json<WebhookRuleConfig>,
) -> Response {
    rule.id = rule.id.trim().to_string();
    rule.source = rule.source.trim().to_string();
    if rule.id.is_empty() || rule.source.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "webhook rule `id` and `source` are required"})),
        )
            .into_response();
    }
    let rule_out = rule.clone();
    match state.runtime.update_config(|cfg| {
        if rule.enabled {
            cfg.webhooks.enabled = true;
        }
        match cfg
            .webhooks
            .rules
            .iter_mut()
            .find(|r| r.id.eq_ignore_ascii_case(&rule.id))
        {
            Some(existing) => *existing = rule,
            None => cfg.webhooks.rules.push(rule),
        }
    }) {
        Ok(updated) => Json(json!({
            "ok": true,
            "rule": rule_out,
            "webhook_rules": updated.webhooks.rules,
            "triggers": state.runtime.list_triggers(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_delete_webhook_rule(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    let needle = id.trim().to_string();
    let mut removed = false;
    match state.runtime.update_config(|cfg| {
        let before = cfg.webhooks.rules.len();
        cfg.webhooks
            .rules
            .retain(|r| !r.id.eq_ignore_ascii_case(&needle));
        removed = cfg.webhooks.rules.len() < before;
    }) {
        Ok(updated) => Json(json!({
            "deleted": removed,
            "webhook_rules": updated.webhooks.rules,
            "triggers": state.runtime.list_triggers(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_upsert_watcher(
    State(state): State<WebState>,
    Json(mut watcher): Json<WorkspaceWatcherConfig>,
) -> Response {
    watcher.id = watcher.id.trim().to_string();
    if watcher.id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "workspace watcher `id` is required"})),
        )
            .into_response();
    }
    watcher.poll_interval_secs = watcher.poll_interval_secs.max(WATCHER_MIN_POLL_INTERVAL_SECS);
    let watcher_out = watcher.clone();
    match state.runtime.update_config(|cfg| {
        match cfg
            .watchers
            .iter_mut()
            .find(|w| w.id.eq_ignore_ascii_case(&watcher.id))
        {
            Some(existing) => *existing = watcher,
            None => cfg.watchers.push(watcher),
        }
    }) {
        Ok(updated) => Json(json!({
            "ok": true,
            "watcher": watcher_out,
            "watchers": updated.watchers,
            "triggers": state.runtime.list_triggers(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_delete_watcher(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    let needle = id.trim().to_string();
    let mut removed = false;
    match state.runtime.update_config(|cfg| {
        let before = cfg.watchers.len();
        cfg.watchers.retain(|w| !w.id.eq_ignore_ascii_case(&needle));
        removed = cfg.watchers.len() < before;
    }) {
        Ok(updated) => Json(json!({
            "deleted": removed,
            "watchers": updated.watchers,
            "triggers": state.runtime.list_triggers(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct ExportQuery {
    format: Option<String>,
    include_thinking: Option<bool>,
    anonymize: Option<bool>,
    all_branches: Option<bool>,
    verified_only: Option<bool>,
    min_turns: Option<usize>,
    since: Option<String>,
    channel: Option<String>,
    limit: Option<usize>,
}

pub async fn api_export_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(q): Query<ExportQuery>,
) -> Response {
    let fmt = ExportFormat::from_str_loose(q.format.as_deref().unwrap_or("jsonl"));
    let opts = ExportOptions {
        format: fmt,
        include_thinking: q.include_thinking.unwrap_or(false),
        anonymize_paths: q.anonymize.unwrap_or(true),
        all_branches: q.all_branches.unwrap_or(false),
        ..ExportOptions::default()
    };
    match state.runtime.export_session_trajectory(&id, &opts) {
        Ok(rep) => {
            let safe_id: String = id
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
                .collect();
            let safe_id = match safe_id.is_empty() {
                true => "session",
                false => safe_id.as_str(),
            };
            let filename = format!("zenko_{}_{}.{}", safe_id, fmt.as_str(), fmt.file_ext());
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, fmt.content_type().to_string()),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("attachment; filename=\"{filename}\""),
                    ),
                ],
                rep.content,
            )
                .into_response()
        }
        Err(e) => (StatusCode::NOT_FOUND, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn api_export_batch(
    State(state): State<WebState>,
    Query(q): Query<ExportQuery>,
) -> Response {
    let fmt = ExportFormat::from_str_loose(q.format.as_deref().unwrap_or("jsonl"));
    let since = q.since.as_deref().and_then(parse_since_window);
    let opts = ExportOptions {
        format: fmt,
        include_thinking: q.include_thinking.unwrap_or(false),
        anonymize_paths: q.anonymize.unwrap_or(true),
        all_branches: q.all_branches.unwrap_or(false),
        verified_only: q.verified_only.unwrap_or(false),
        min_turns: q.min_turns.unwrap_or(0),
        since,
        channel: q.channel,
        limit: q.limit,
    };
    match state.runtime.export_batch_trajectories(&opts) {
        Ok(rep) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, fmt.content_type().to_string())],
            rep.content,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct CreateCronReq {
    name: String,
    cron_expr: String,
    kind: Option<String>,
    #[serde(default)]
    prompt: String,
    script_path: Option<String>,
    target_channel: Option<String>,
}

pub async fn api_create_cron(
    State(state): State<WebState>,
    Json(req): Json<CreateCronReq>,
) -> Response {
    let parsed = match Cron::new(&req.cron_expr).parse() {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid cron expression: {e}")})),
            )
                .into_response()
        }
    };
    let kind = CronKind::from_str_loose(req.kind.as_deref().unwrap_or("cron"));
    let script_path = req
        .script_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    if kind == CronKind::Script && script_path.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "script_path is required for script cron jobs"})),
        )
            .into_response();
    }
    let next = parsed.find_next_occurrence(&Utc::now(), false).ok();
    let job = CronJob {
        id: new_id(),
        name: req.name,
        cron_expr: req.cron_expr,
        kind,
        prompt: req.prompt,
        script_path,
        target_channel: req.target_channel.unwrap_or_else(|| "webui".into()),
        target_chat_id: None,
        target_thread_id: None,
        enabled: true,
        next_run_at: next,
        last_run_at: None,
        last_status: None,
    };
    match state.runtime.store.upsert_cron_job(&job) {
        Ok(()) => {
            state.runtime.notify_cron_updated();
            Json(job).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UpdateCronReq {
    name: Option<String>,
    cron_expr: Option<String>,
    kind: Option<String>,
    prompt: Option<String>,
    script_path: Option<String>,
    enabled: Option<bool>,
}

pub async fn api_update_cron(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCronReq>,
) -> Response {
    let Some(mut job) = state.runtime.store.get_cron_job(&id).ok().flatten() else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("cron job `{id}` not found")})),
        )
            .into_response();
    };
    if let Some(n) = req.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        job.name = n.to_string();
    }
    if let Some(expr) = req.cron_expr.as_deref() {
        match Cron::new(expr).parse() {
            Ok(parsed) => {
                job.cron_expr = expr.to_string();
                job.next_run_at = parsed.find_next_occurrence(&Utc::now(), false).ok();
            }
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("Invalid cron expression: {e}")})),
                )
                    .into_response()
            }
        }
    }
    if let Some(k) = req.kind.as_deref() {
        job.kind = CronKind::from_str_loose(k);
    }
    if let Some(p) = req.prompt {
        job.prompt = p;
    }
    if let Some(sp) = req.script_path {
        let trimmed = sp.trim();
        job.script_path = match trimmed.is_empty() {
            true => None,
            false => Some(trimmed.to_string()),
        };
    }
    if let Some(en) = req.enabled {
        job.enabled = en;
    }
    match state.runtime.store.upsert_cron_job(&job) {
        Ok(()) => {
            state.runtime.notify_cron_updated();
            Json(job).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_run_cron_now(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match state.runtime.trigger_cron_job_now(&id).await {
        Ok(res) => Json(res).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_delete_cron(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match state.runtime.store.delete_cron_job(&id) {
        Ok(ok) => {
            state.runtime.notify_cron_updated();
            Json(json!({"deleted": ok})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_telemetry(State(state): State<WebState>) -> Response {
    Json(state.runtime.telemetry()).into_response()
}

/// Bounded iterative directory byte counter (stack-based, zero recursion, zero symlinks, capped work).
fn measure_dir_bounded(root: &std::path::Path, max_depth: usize, max_entries: usize) -> (u64, u64) {
    if !root.exists() {
        return (0, 0);
    }
    if root.is_file() {
        let len = std::fs::metadata(root).map(|m| m.len()).unwrap_or(0);
        return (len, 1);
    }
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut bytes = 0u64;
    let mut files = 0u64;
    let mut visited = 0usize;

    while let Some((dir, depth)) = stack.pop() {
        if depth > max_depth || visited >= max_entries {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            visited += 1;
            if visited > max_entries {
                break;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_file() {
                if let Ok(meta) = entry.metadata() {
                    bytes = bytes.saturating_add(meta.len());
                    files += 1;
                }
            } else if ft.is_dir() && depth < max_depth {
                stack.push((entry.path(), depth + 1));
            }
        }
    }
    (bytes, files)
}

pub async fn api_dashboard(State(state): State<WebState>) -> Response {

    let cfg = state.runtime.config();
    let telemetry = state.runtime.telemetry();
    let vault = state.runtime.vault.stats();
    let mut metrics = state.runtime.store.dashboard_db_metrics(365).unwrap_or_default();

    let snap = state.runtime.snapshots.load_snapshot().ok();
    if let Some(s) = &snap {
        metrics.counts.skills_total = s.skills.len() as u64;
        metrics.counts.skills_enabled = s.skills.iter().filter(|k| k.enabled).count() as u64;
    }
    metrics.counts.mcp_servers = state.runtime.mcp_clients.list_status().await.len() as u64;

    let data_dir = cfg.resolved_data_dir();
    let hub_dir = cfg.resolved_hub_dir();
    let logs_dir = state.runtime.log_sink.logs_dir().to_path_buf();
    let (db_bytes, db_files) = state.runtime.store.db_total_size_bytes();

    let storage_specs: [(&'static str, &'static str, std::path::PathBuf); 5] = [
        ("logs", "Flight Recorder Logs", logs_dir),
        ("checkpoints", "Checkpoints & Blobs", data_dir.join("checkpoints")),
        ("artifacts", "Spilled Artifacts", data_dir.join("artifacts")),
        ("skills", "Skills & Playbooks", data_dir.join("skills")),
        ("hub", "Zenko Hub Creations", hub_dir),
    ];

    let mut storage = vec![
        StorageSlice {
            id: "sqlite_db".into(),
            label: "SQLite Database (WAL)".into(),
            path: state.runtime.store.db_path().display().to_string(),
            bytes: db_bytes,
            files: db_files,
        },
        StorageSlice {
            id: "zenko_vault".into(),
            label: "Zenko Vault (ChaCha20)".into(),
            path: vault.path.clone(),
            bytes: vault.size_bytes,
            files: if vault.exists { 2 } else { 0 },
        },
    ];

    let scanned_slices = tokio::task::spawn_blocking(move || {
        let mut out = Vec::with_capacity(storage_specs.len() + 1);
        for (id, label, path) in storage_specs {
            let (bytes, files) = measure_dir_bounded(&path, 4, 8_000);
            out.push(StorageSlice {
                id: id.into(),
                label: label.into(),
                path: path.display().to_string(),
                bytes,
                files,
            });
        }
        // Also include top-level markdown memory snapshots (SOUL.md, USER.md, MEMORY.md, ZENKO.md)
        let mut md_bytes = 0u64;
        let mut md_files = 0u64;
        for name in ["SOUL.md", "USER.md", "MEMORY.md", "ZENKO.md"] {
            if let Ok(meta) = std::fs::metadata(data_dir.join(name)) {
                md_bytes = md_bytes.saturating_add(meta.len());
                md_files += 1;
            }
        }
        out.push(StorageSlice {
            id: "memory_docs".into(),
            label: "Prompt & Memory Docs".into(),
            path: data_dir.display().to_string(),
            bytes: md_bytes,
            files: md_files,
        });
        out
    })
    .await
    .unwrap_or_default();
    storage.extend(scanned_slices);

    let total_storage_bytes: u64 = storage.iter().map(|s| s.bytes).sum();

    Json(DashboardAnalytics {
        telemetry,
        vault,
        counts: metrics.counts,
        total_storage_bytes,
        storage,
        daily_activity: metrics.daily_activity,
        top_tools: metrics.top_tools,
        top_models: metrics.top_models,
        channels: metrics.channels,
        streak_days: metrics.streak_days,
        active_days: metrics.active_days,
        total_tokens: metrics.total_tokens,
        total_cache_read_tokens: metrics.total_cache_read_tokens,
        generated_at: Utc::now(),
    })
    .into_response()
}

pub async fn api_logs(
    State(state): State<WebState>,
    Query(mut filter): Query<LogFilter>,
) -> Response {
    if filter.limit.is_none() {
        filter.limit = Some(200);
    }
    let rows = match state.runtime.store.query_logs(&filter) {
        Ok(list) if !list.is_empty() => list,
        Ok(_) => state
            .runtime
            .logs
            .recent(filter.limit.unwrap_or(200))
            .into_iter()
            .filter(|e| filter.matches(e))
            .collect(),
        Err(_) => state
            .runtime
            .logs
            .recent(filter.limit.unwrap_or(200))
            .into_iter()
            .filter(|e| filter.matches(e))
            .collect(),
    };
    Json(rows).into_response()
}

