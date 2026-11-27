use super::*;

// ─── Sessions & Messages ──────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct ListSessionsQuery {
    #[serde(default)]
    include_children: Option<bool>,
    #[serde(default)]
    limit: Option<usize>,
}

pub async fn api_list_sessions(
    State(state): State<WebState>,
    Query(q): Query<ListSessionsQuery>,
) -> Response {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let result = match q.include_children.unwrap_or(false) {
        true => state.runtime.store.list_sessions(limit),
        false => state.runtime.store.list_root_sessions(limit),
    };
    match result {
        Ok(list) => Json(list).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_get_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    let Some(session) = state.runtime.store.get_session(&id).ok().flatten() else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "session not found"})),
        )
            .into_response();
    };
    let events = state.runtime.store.get_active_branch(&id).unwrap_or_default();
    let mut siblings = HashMap::new();
    for ev in &events {
        if let Ok(list) = state
            .runtime
            .store
            .list_sibling_branches(&id, ev.parent_id.as_deref())
        {
            siblings.insert(ev.id.clone(), list.into_iter().map(|e| e.id).collect::<Vec<_>>());
        }
    }
    let pending_approvals: Vec<_> = state
        .runtime
        .store
        .list_pending_approvals()
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.session_id == id)
        .collect();

    let queued_prompts = state
        .runtime
        .store
        .list_queued_followups(&id)
        .unwrap_or_default();
    let pending_clarifications = state
        .runtime
        .clarifications
        .list_pending_clarifications(Some(&id));
    let tasks = state
        .runtime
        .clarifications
        .list_session_tasks(&id)
        .unwrap_or_default();
    let checkpoints = state
        .runtime
        .store
        .list_checkpoints(Some(&id), 30)
        .unwrap_or_default();

    Json(json!({
        "session": session,
        "events": events,
        "siblings": siblings,
        "pending_approvals": pending_approvals,
        "pending_clarifications": pending_clarifications,
        "queued_prompts": queued_prompts,
        "tasks": tasks,
        "checkpoints": checkpoints,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct CreateSessionReq {
    title: Option<String>,
    #[serde(default)]
    permission_mode: Option<String>,
    #[serde(default)]
    model_override: Option<String>,
}

pub async fn api_create_session(
    State(state): State<WebState>,
    Json(req): Json<CreateSessionReq>,
) -> Response {
    let cfg = state.runtime.config();
    let mode = req
        .permission_mode
        .as_deref()
        .map(PermissionMode::from_str_loose)
        .unwrap_or(cfg.policy.mode);
    match state.runtime.store.create_session(
        req.title.as_deref().unwrap_or("New Session"),
        "webui",
        None,
        None,
        mode,
    ) {
        Ok(mut s) => {
            if let Some(m) = req.model_override.as_deref().map(str::trim).filter(|v| !v.is_empty())
                && state
                    .runtime
                    .store
                    .update_session_meta(&s.id, None, Some(Some(m)), None)
                    .is_ok()
                {
                    s.model_override = Some(m.to_string());
                }
            Json(s).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_get_session_context(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match state.runtime.inspect_session_context(&id).await {
        Ok(ctx) => Json(ctx).into_response(),
        Err(zenko_core::ZenkoError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct ZPowersQuery {
    #[serde(default)]
    include_content: bool,
}

pub async fn api_get_session_zpowers(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(q): Query<ZPowersQuery>,
) -> Response {
    match state
        .runtime
        .inspect_session_zpowers(&id, q.include_content)
    {
        Ok(view) => Json(view).into_response(),
        Err(zenko_core::ZenkoError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_clear_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    let _ = state.runtime.cancel_session(&id);
    state.runtime.clear_session_deferred_tools(&id);
    state.runtime.cleanup_session_zpowers(&id);
    state.runtime.notify_session_ended(&id).await;
    match state.runtime.store.clear_session_events(&id) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_delete_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    let _ = state.runtime.cancel_session(&id);
    state.runtime.clear_session_deferred_tools(&id);
    state.runtime.cleanup_session_zpowers(&id);
    state.runtime.notify_session_ended(&id).await;
    match state.runtime.store.delete_session(&id) {
        Ok(deleted) => Json(json!({"deleted": deleted})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UpdateSessionReq {
    title: Option<String>,
    provider_kind: Option<String>,
    model_override: Option<String>,
    permission_mode: Option<String>,
    enable_streaming: Option<bool>,
}

pub async fn api_update_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSessionReq>,
) -> Response {

    // Verify session exists BEFORE mutating global provider/streaming config.
    match state.runtime.store.get_session(&id) {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("session `{id}` not found")})),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    }

    // Allow switching active provider / model / streaming directly from the Chat Composer!
    if req.provider_kind.is_some() || req.model_override.is_some() || req.enable_streaming.is_some()
    {
        let target_kind = req.provider_kind.clone();
        let target_model = req.model_override.clone();
        let target_stream = req.enable_streaming;
        if let Err(e) = state.runtime.update_config(|c| {
            let kind = target_kind.as_deref().unwrap_or(&c.provider.kind).to_string();
            c.provider
                .switch_provider(&kind, None, target_model.as_deref(), true);
            if let Some(stream_on) = target_stream {
                c.provider.enable_streaming = stream_on;
            }
        }) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    }

    let model = req.model_override.as_deref().map(|m| {
        if m.trim().is_empty() { None } else { Some(m.trim()) }
    });
    let perm = req.permission_mode.as_deref().map(PermissionMode::from_str_loose);
    match state
        .runtime
        .store
        .update_session_meta(&id, req.title.as_deref(), model, perm)
    {
        Ok(()) => Json(json!({
            "ok": true,
            "config": state.runtime.config()
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct SendMessageReq {
    text: String,
    parent_event_id: Option<Option<String>>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

pub async fn api_send_message(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Json(req): Json<SendMessageReq>,
) -> Response {
    let send_mode = req
        .mode
        .as_deref()
        .map(SendMode::from_str_loose)
        .unwrap_or(SendMode::Auto);
    match state.runtime.resolve_session_dispatch(&id, send_mode) {
        zenko_core::AutoDispatchDecision::Steer => {
            if state.runtime.steer_session(&id, &req.text) {
                return Json(json!({
                    "accepted": true,
                    "steered": true,
                    "queued": false,
                    "dispatch_mode": "steered"
                }))
                .into_response();
            }
        }
        zenko_core::AutoDispatchDecision::Queue => {
            return match state.runtime.enqueue_session_followup(&id, &req.text) {
                Ok((queued, count)) => (
                    StatusCode::ACCEPTED,
                    Json(json!({
                        "accepted": true,
                        "steered": false,
                        "queued": true,
                        "dispatch_mode": "queued",
                        "queued_prompt": queued,
                        "queued_count": count
                    })),
                )
                    .into_response(),
                Err(e) => (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": e.to_string()})),
                )
                    .into_response(),
            };
        }
        zenko_core::AutoDispatchDecision::StartTurn => {}
    }
    let rt = state.runtime.clone();
    let text = req.text;
    let parent = req.parent_event_id;
    let model_override = req
        .model
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    tokio::spawn(async move {
        let _ = rt
            .run_turn_with_options(
                &id,
                &text,
                TurnOptions {
                    parent_event_override: parent,
                    model_override,
                    send_mode,
                    ..TurnOptions::default()
                },
            )
            .await;
    });
    (
        StatusCode::ACCEPTED,
        Json(json!({"accepted": true, "steered": false, "queued": false})),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct SteerReq {
    text: String,
}

pub async fn api_steer_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Json(req): Json<SteerReq>,
) -> Response {
    let steered = state.runtime.steer_session(&id, &req.text);
    Json(json!({"steered": steered, "session_id": id})).into_response()
}

pub async fn api_background_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    let demoted = state.runtime.demote_foreground_job(&id);
    Json(json!({"demoted": demoted, "session_id": id})).into_response()
}

pub async fn api_list_session_queue(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match state.runtime.store.list_queued_followups(&id) {
        Ok(list) => Json(list).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_cancel_session_queue(
    State(state): State<WebState>,
    Path((id, qid)): Path<(String, String)>,
) -> Response {
    match state.runtime.cancel_session_followup(&id, &qid) {
        Ok((cancelled, count)) => {
            Json(json!({"cancelled": cancelled, "queued_count": count})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_steer_queued_prompt(
    State(state): State<WebState>,
    Path((id, qid)): Path<(String, String)>,
) -> Response {
    match state.runtime.promote_queued_to_steer(&id, &qid) {
        Ok((steered, count)) => {
            Json(json!({"steered": steered, "queued_count": count})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct JobsListQuery {
    session_id: Option<String>,
    kind: Option<String>,
    limit: Option<usize>,
}

pub async fn api_list_jobs(
    State(state): State<WebState>,
    Query(q): Query<JobsListQuery>,
) -> Response {
    let kind = q.kind.as_deref().and_then(TaskKind::parse_filter);
    match state.runtime.jobs.list_jobs(
        q.session_id.as_deref(),
        kind,
        q.limit.unwrap_or(JOB_DEFAULT_LIST_LIMIT),
    ) {
        Ok(list) => Json(list).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct StartJobReq {
    command: String,
    kind: Option<String>,
    label: Option<String>,
    session_id: Option<String>,
    workspace: Option<String>,
}

pub async fn api_start_job(
    State(state): State<WebState>,
    Json(req): Json<StartJobReq>,
) -> Response {
    if req.command.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "command cannot be empty"})),
        )
            .into_response();
    }
    let kind = req
        .kind
        .as_deref()
        .and_then(TaskKind::parse_filter)
        .unwrap_or_else(|| TaskKind::infer_from_command(&req.command));
    let sid = req
        .session_id
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("webui");
    let ws = req
        .workspace
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| state.runtime.config().resolved_workspace_dir());
    match state
        .runtime
        .jobs
        .spawn_shell(sid, &req.command, &ws, kind, req.label, false)
        .await
    {
        Ok(rec) => (StatusCode::CREATED, Json(rec)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UpdateJobReq {
    label: Option<String>,
    kind: Option<String>,
}

pub async fn api_update_job(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateJobReq>,
) -> Response {
    let kind = req.kind.as_deref().and_then(TaskKind::parse_filter);
    match state.runtime.jobs.update_job_meta(&id, req.label, kind) {
        Ok(Some(rec)) => Json(rec).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("task `{id}` not found")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_restart_job(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match state.runtime.jobs.restart_job(&id).await {
        Ok(rec) => Json(rec).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_delete_job(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match state.runtime.jobs.delete_job(&id).await {
        Ok(deleted) => Json(json!({"deleted": deleted, "id": id})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct PruneJobsReq {
    kind: Option<String>,
}

pub async fn api_prune_jobs(
    State(state): State<WebState>,
    body: Option<Json<PruneJobsReq>>,
) -> Response {
    let kind = body
        .and_then(|b| b.0.kind)
        .as_deref()
        .and_then(TaskKind::parse_filter);
    match state.runtime.jobs.prune_jobs(kind).await {
        Ok(pruned) => Json(json!({"pruned": pruned})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct HubListQuery {
    category: Option<String>,
    limit: Option<usize>,
}

pub async fn api_list_hub(
    State(state): State<WebState>,
    Query(q): Query<HubListQuery>,
) -> Response {
    match state
        .runtime
        .list_hub_entries(q.category.as_deref(), q.limit.unwrap_or(100))
    {
        Ok(entries) => Json(json!({
            "hub_dir": state.runtime.config().resolved_hub_dir().display().to_string(),
            "categories": HUB_CATEGORIES,
            "entries": entries,
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
pub struct JobReadQuery {
    offset: Option<u64>,
    max_bytes: Option<usize>,
}

pub async fn api_read_job_output(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(q): Query<JobReadQuery>,
) -> Response {
    match state
        .runtime
        .jobs
        .read_job_output(&id, q.offset, q.max_bytes.unwrap_or(JOB_DEFAULT_READ_BYTES))
        .await
    {
        Ok(chunk) => Json(chunk).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_clear_job_output(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match state.runtime.jobs.clear_job_output(&id).await {
        Ok(Some(job)) => Json(json!({"cleared": true, "job": job})).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("task `{id}` not found")})),
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
pub struct JobInputReq {
    input: String,
}

pub async fn api_job_input(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Json(req): Json<JobInputReq>,
) -> Response {
    match state.runtime.jobs.send_input(&id, &req.input).await {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct JobWaitReq {
    timeout_secs: Option<u64>,
    tail_bytes: Option<usize>,
}

pub async fn api_wait_job(
    State(state): State<WebState>,
    Path(id): Path<String>,
    body: Option<Json<JobWaitReq>>,
) -> Response {
    let req = body.map(|b| b.0).unwrap_or_default();
    let timeout_secs = req
        .timeout_secs
        .unwrap_or(JOB_DEFAULT_WAIT_SECS)
        .clamp(1, JOB_MAX_WAIT_SECS);
    let tail_bytes = req.tail_bytes.unwrap_or(JOB_DEFAULT_READ_BYTES);
    match state
        .runtime
        .jobs
        .wait_job(&id, Duration::from_secs(timeout_secs))
        .await
    {
        Ok(Some(rec)) => {
            let output = state
                .runtime
                .jobs
                .read_job_output(&rec.id, None, tail_bytes)
                .await
                .ok();
            Json(json!({"job": rec, "output": output})).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("job `{id}` not found")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_stop_job(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match state.runtime.jobs.stop_job(&id).await {
        Ok(rec) => Json(json!({"stopped": rec.is_some(), "job": rec})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_stop_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    let cancelled = state.runtime.cancel_session(&id);
    Json(json!({"cancelled": cancelled})).into_response()
}

#[derive(Deserialize)]
pub struct SwitchBranchReq {
    event_id: String,
}

pub async fn api_switch_branch(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Json(req): Json<SwitchBranchReq>,
) -> Response {
    match state.runtime.store.find_latest_leaf(&id, &req.event_id) {
        Ok(leaf) => match state.runtime.store.set_session_head(&id, Some(&leaf)) {
            Ok(()) => Json(json!({"head": leaf})).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct ResolveApprovalReq {
    decision: ApprovalDecision,
}

pub async fn api_resolve_approval(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Json(req): Json<ResolveApprovalReq>,
) -> Response {
    match state.runtime.resolve_approval(&id, req.decision, "webui") {
        Ok(rec) => Json(json!({"resolved": rec.is_some()})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct ClarificationsQuery {
    session_id: Option<String>,
}

pub async fn api_list_clarifications(
    State(state): State<WebState>,
    Query(q): Query<ClarificationsQuery>,
) -> Response {
    let list = state
        .runtime
        .clarifications
        .list_pending_clarifications(q.session_id.as_deref());
    Json(list).into_response()
}

#[derive(Deserialize, Default)]
pub struct ResolveClarificationReq {
    #[serde(default)]
    selected_option_id: String,
    #[serde(default)]
    selected_option_ids: Vec<String>,
    #[serde(default)]
    step_answers: std::collections::BTreeMap<String, Vec<String>>,
    #[serde(default)]
    field_values: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    custom_answer: Option<String>,
    #[serde(default)]
    rating: Option<String>,
}

pub async fn api_resolve_clarification(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Json(req): Json<ResolveClarificationReq>,
) -> Response {
    let resolution = zenko_core::AskUserResolution {
        selected_option_id: req.selected_option_id,
        selected_option_ids: req.selected_option_ids,
        step_answers: req.step_answers,
        field_values: req.field_values,
        custom_answer: req.custom_answer,
        rating: req.rating,
    };
    match state
        .runtime
        .clarifications
        .resolve_ask_user(&id, resolution, "webui")
    {
        Some(rec) => Json(json!({"resolved": true, "clarification": rec})).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("clarification `{id}` not found")})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct TriageReq {
    prompt: String,
    #[serde(default)]
    mode: Option<String>,
}

pub async fn api_orchestrator_triage(
    State(state): State<WebState>,
    Json(req): Json<TriageReq>,
) -> Response {
    let mode = req.mode.as_deref().map(PermissionMode::from_str_loose);
    let report = state.runtime.triage_prompt(&req.prompt, mode);
    Json(report).into_response()
}

