use super::*;

// ─── Phase 6: Session Task DAG, Plan Approval & Workspace Checkpoints / Rewind ──

#[derive(Deserialize, Default)]
pub struct ListSessionTasksQuery {
    #[serde(default)]
    ready_only: Option<bool>,
}

pub async fn api_list_session_tasks(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(q): Query<ListSessionTasksQuery>,
) -> Response {
    let res = match q.ready_only.unwrap_or(false) {
        true => state.runtime.clarifications.ready_session_tasks(&id),
        false => state.runtime.clarifications.list_session_tasks(&id),
    };
    match res {
        Ok(tasks) => Json(json!({ "tasks": tasks })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct CreateSessionTaskReq {
    #[serde(default)]
    id: Option<String>,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    active_label: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    owner_agent: Option<String>,
    #[serde(default)]
    verification_cmd: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    priority: i32,
}

pub async fn api_create_session_task(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Json(req): Json<CreateSessionTaskReq>,
) -> Response {
    let now = Utc::now();
    let task = SessionTask {
        id: req
            .id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(new_session_task_id),
        session_id: id.clone(),
        branch_event_id: None,
        title: req.title,
        description: req.description,
        active_label: req.active_label,
        status: req
            .status
            .as_deref()
            .map(SessionTaskStatus::from_str_loose)
            .unwrap_or(SessionTaskStatus::Pending),
        depends_on: req.depends_on,
        owner_agent: req.owner_agent,
        verification_cmd: req.verification_cmd,
        verification_evidence: None,
        notes: req.notes,
        priority: req.priority,
        created_at: now,
        updated_at: now,
    };
    match state.runtime.clarifications.create_session_task(&id, task) {
        Ok(saved) => (StatusCode::CREATED, Json(saved)).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct UpdateSessionTaskReq {
    title: Option<String>,
    description: Option<String>,
    active_label: Option<Option<String>>,
    status: Option<String>,
    depends_on: Option<Vec<String>>,
    #[serde(default)]
    add_depends_on: Vec<String>,
    owner_agent: Option<Option<String>>,
    verification_cmd: Option<Option<String>>,
    verification_evidence: Option<Option<String>>,
    notes: Option<Option<String>>,
    priority: Option<i32>,
}

pub async fn api_update_session_task(
    State(state): State<WebState>,
    Path((id, tid)): Path<(String, String)>,
    Json(req): Json<UpdateSessionTaskReq>,
) -> Response {
    let patch = SessionTaskPatch {
        title: req.title,
        description: req.description,
        active_label: req.active_label,
        status: req.status.as_deref().map(SessionTaskStatus::from_str_loose),
        depends_on: req.depends_on,
        add_depends_on: req.add_depends_on,
        owner_agent: req.owner_agent,
        verification_cmd: req.verification_cmd,
        verification_evidence: req.verification_evidence,
        notes: req.notes,
        priority: req.priority,
    };
    match state
        .runtime
        .clarifications
        .update_session_task(&id, &tid, patch)
    {
        Ok(updated) => Json(updated).into_response(),
        Err(zenko_core::ZenkoError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct ClaimSessionTaskReq {
    owner_agent: Option<String>,
}

pub async fn api_claim_session_task(
    State(state): State<WebState>,
    Path((id, tid)): Path<(String, String)>,
    body: Option<Json<ClaimSessionTaskReq>>,
) -> Response {
    let owner = body
        .and_then(|b| b.0.owner_agent)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "webui".into());
    match state
        .runtime
        .clarifications
        .claim_session_task(&id, &tid, &owner)
    {
        Ok(claimed) => Json(claimed).into_response(),
        Err(zenko_core::ZenkoError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
        }
        Err(zenko_core::ZenkoError::PolicyDenied(msg)) => {
            (StatusCode::CONFLICT, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_delete_session_task(
    State(state): State<WebState>,
    Path((id, tid)): Path<(String, String)>,
) -> Response {
    match state.runtime.clarifications.delete_session_task(&id, &tid) {
        Ok(deleted) => Json(json!({ "deleted": deleted })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct ApprovePlanReq {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    execute_prompt: Option<String>,
}

pub async fn api_approve_session_plan(
    State(state): State<WebState>,
    Path(id): Path<String>,
    body: Option<Json<ApprovePlanReq>>,
) -> Response {
    let req = body.map(|b| b.0).unwrap_or_default();
    let target_mode = req
        .mode
        .as_deref()
        .map(PermissionMode::from_str_loose)
        .unwrap_or(PermissionMode::Autonomous);
    match state.runtime.approve_session_plan(&id, target_mode) {
        Ok((session, task_count)) => {
            let mut dispatched = false;
            if let Some(prompt) = req
                .execute_prompt
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let rt = state.runtime.clone();
                let sid = id.clone();
                let text = prompt.to_string();
                dispatched = true;
                tokio::spawn(async move {
                    let _ = rt.run_turn_with_options(&sid, &text, TurnOptions::default()).await;
                });
            }
            Json(json!({
                "ok": true,
                "session": session,
                "task_count": task_count,
                "dispatched": dispatched,
            }))
            .into_response()
        }
        Err(zenko_core::ZenkoError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct ListCheckpointsQuery {
    #[serde(default)]
    limit: Option<usize>,
}

pub async fn api_list_session_checkpoints(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(q): Query<ListCheckpointsQuery>,
) -> Response {
    let limit = q.limit.unwrap_or(CHECKPOINT_LIST_DEFAULT_LIMIT);
    match state.runtime.store.list_checkpoints(Some(&id), limit) {
        Ok(checkpoints) => Json(json!({ "checkpoints": checkpoints })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_diff_checkpoint(
    State(state): State<WebState>,
    Path(cp_id): Path<String>,
) -> Response {
    let cp_cfg = state.runtime.config().checkpoints;
    match state
        .runtime
        .checkpoints
        .diff_checkpoint(&cp_cfg, &cp_id)
        .await
    {
        Ok(rep) => Json(rep).into_response(),
        Err(zenko_core::ZenkoError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct RewindSessionReq {
    #[serde(default)]
    checkpoint_id: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    overwrite_manual_edits: bool,
    #[serde(default = "default_summarize_true")]
    summarize_abandoned: bool,
}

fn default_summarize_true() -> bool {
    true
}

pub async fn api_rewind_session(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Json(req): Json<RewindSessionReq>,
) -> Response {
    if state.runtime.is_session_running(&id) {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "Cannot rewind while a turn is actively running on this session. Stop the turn first."})),
        )
            .into_response();
    }
    let cp_id = match req
        .checkpoint_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(explicit) => {
            match state.runtime.store.get_checkpoint(explicit) {
                Ok(Some(cp)) if cp.session_id == id => cp.id,
                Ok(Some(_)) => {
                    return (
                        StatusCode::FORBIDDEN,
                        Json(json!({"error": format!("checkpoint `{explicit}` does not belong to session `{id}`")})),
                    )
                        .into_response();
                }
                Ok(None) => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({"error": format!("checkpoint `{explicit}` not found")})),
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
        }
        None => {
            let list = state
                .runtime
                .store
                .list_checkpoints(Some(&id), CHECKPOINT_LIST_DEFAULT_LIMIT)
                .unwrap_or_default();
            match list.into_iter().find(|c| !c.is_safety_backup) {
                Some(cp) => cp.id,
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({"error": format!("no checkpoints found for session `{id}`")})),
                    )
                        .into_response();
                }
            }
        }
    };
    match state
        .runtime
        .rewind_checkpoint(&RewindRequest {
            checkpoint_id: cp_id,
            mode: req
                .mode
                .as_deref()
                .map(RewindMode::from_str_loose)
                .unwrap_or(RewindMode::CodeAndConversation),
            files: req.files,
            overwrite_manual_edits: req.overwrite_manual_edits,
            summarize_abandoned: req.summarize_abandoned,
        })
        .await
    {
        Ok(rep) => Json(rep).into_response(),
        Err(zenko_core::ZenkoError::NotFound(msg)) => {
            (StatusCode::NOT_FOUND, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_prune_checkpoints(State(state): State<WebState>) -> Response {
    let cp_cfg = state.runtime.config().checkpoints;
    match state.runtime.checkpoints.prune_checkpoints(&cp_cfg).await {
        Ok(rep) => Json(rep).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

