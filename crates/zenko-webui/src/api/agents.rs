use super::*;

// ─── Phase 5: Async Subagents, Worktrees & Mailbox API ────────────────────────

#[derive(Deserialize, Default)]
pub struct AgentsListQuery {
    parent_session_id: Option<String>,
    limit: Option<usize>,
}

pub async fn api_list_agents(
    State(state): State<WebState>,
    Query(q): Query<AgentsListQuery>,
) -> Response {
    let parent_filter = q
        .parent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let limit = q.limit.unwrap_or(SUBAGENT_LIST_DEFAULT_LIMIT);
    match state.runtime.store.list_subagents(parent_filter, limit) {
        Ok(agents) => Json(json!({
            "roles": SUBAGENT_ROLE_REGISTRY,
            "active_count": state.runtime.subagents.active_count(),
            "agents": agents,
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
pub struct SpawnAgentReq {
    #[serde(default)]
    parent_session_id: Option<String>,
    role: String,
    goal: String,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    run_in_background: Option<bool>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    output_schema: Option<Value>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    fork_context: Option<bool>,
    #[serde(default)]
    include_uncommitted: Option<bool>,
    #[serde(default)]
    max_duration_secs: Option<u64>,
    #[serde(default)]
    max_cost_usd: Option<f64>,
}

pub async fn api_spawn_agent(
    State(state): State<WebState>,
    Json(req): Json<SpawnAgentReq>,
) -> Response {
    let spawn_req = SubagentSpawnRequest {
        parent_session_id: req
            .parent_session_id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "webui".into()),
        role: req.role,
        goal: req.goal,
        context: req.context,
        run_in_background: req.run_in_background.unwrap_or(true),
        workspace_mode: req
            .workspace
            .as_deref()
            .map(SubagentWorkspaceMode::from_str_loose),
        output_schema: req.output_schema,
        model_override: req.model,
        fork_context: req.fork_context.unwrap_or(false),
        include_uncommitted: req.include_uncommitted.unwrap_or(false),
        max_duration_secs: req.max_duration_secs,
        max_cost_usd: req.max_cost_usd,
    };
    match state
        .runtime
        .subagents
        .spawn_agent(&state.runtime, spawn_req)
        .await
    {
        Ok(rec) => (StatusCode::CREATED, Json(rec)).into_response(),
        Err(zenko_core::ZenkoError::PolicyDenied(msg)) => {
            (StatusCode::FORBIDDEN, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_get_agent(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match state.runtime.store.get_subagent(&id) {
        Ok(Some(rec)) => {
            let messages = state
                .runtime
                .store
                .list_agent_messages(&id, 50)
                .unwrap_or_default();
            Json(json!({ "agent": rec, "messages": messages })).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("subagent `{id}` not found")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct KillAgentQuery {
    remove_worktree: Option<bool>,
}

pub async fn api_kill_agent(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(q): Query<KillAgentQuery>,
) -> Response {
    let remove_wt = q.remove_worktree.unwrap_or(true);
    match state
        .runtime
        .subagents
        .kill_agent(&state.runtime, "webui", &id, remove_wt)
        .await
    {
        Ok(Some(rec)) => Json(json!({"killed": true, "agent": rec})).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("subagent `{id}` not found")})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct AgentMessageReq {
    message: String,
    #[serde(default)]
    from_session_id: Option<String>,
    #[serde(default)]
    resume: Option<bool>,
    #[serde(default)]
    interrupt: Option<bool>,
}

pub async fn api_message_agent(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Json(req): Json<AgentMessageReq>,
) -> Response {
    let from = req
        .from_session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("webui");
    let opts = AgentMessageOptions {
        resume: req.resume.unwrap_or(true),
        interrupt: req.interrupt.unwrap_or(false),
    };
    match state
        .runtime
        .subagents
        .send_message(&state.runtime, from, &id, &req.message, opts)
        .await
    {
        Ok(mail) => Json(mail).into_response(),
        Err(zenko_core::ZenkoError::PolicyDenied(msg)) => {
            (StatusCode::FORBIDDEN, Json(json!({"error": msg}))).into_response()
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

pub async fn api_agent_diff(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match state
        .runtime
        .subagents
        .inspect_worktree_diff(&state.runtime, "webui", &id)
        .await
    {
        Ok(Some(rep)) => Json(rep).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("no git worktree for subagent `{id}`")})),
        )
            .into_response(),
        Err(zenko_core::ZenkoError::PolicyDenied(msg)) => {
            (StatusCode::FORBIDDEN, Json(json!({"error": msg}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct MergeWorktreeReq {
    cleanup: Option<bool>,
}

pub async fn api_merge_agent_worktree(
    State(state): State<WebState>,
    Path(id): Path<String>,
    body: Option<Json<MergeWorktreeReq>>,
) -> Response {
    let cleanup = body.and_then(|b| b.0.cleanup).unwrap_or(true);
    match state
        .runtime
        .subagents
        .merge_worktree(&state.runtime, "webui", &id, cleanup)
        .await
    {
        Ok(rep) => Json(rep).into_response(),
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

pub async fn api_discard_agent_worktree(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match state
        .runtime
        .subagents
        .discard_worktree(&state.runtime, "webui", &id)
        .await
    {
        Ok(rep) => Json(rep).into_response(),
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

pub async fn api_prune_agent_worktrees(
    State(state): State<WebState>,
) -> Response {
    match state
        .runtime
        .subagents
        .prune_orphan_worktrees(&state.runtime)
        .await
    {
        Ok(rep) => Json(rep).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

