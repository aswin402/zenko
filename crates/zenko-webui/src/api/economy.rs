use super::*;

// ─── Native Token Economy API ─────────────────────────────────────────────────

pub async fn api_get_economy(State(state): State<WebState>) -> Response {
    match state.runtime.economy_report(None) {
        Ok(report) => Json(report).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct UpdateEconomyConfigReq {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    auto_micro_skills: Option<bool>,
    #[serde(default)]
    auto_tool_prune: Option<bool>,
    #[serde(default)]
    p90_loop_guard: Option<bool>,
    #[serde(default)]
    optional_fast_judge: Option<bool>,
}

pub async fn api_update_economy_config(
    State(state): State<WebState>,
    Json(req): Json<UpdateEconomyConfigReq>,
) -> Response {
    let res = state.runtime.update_config(|c| {
        if let Some(v) = req.enabled {
            c.economy.enabled = v;
        }
        if let Some(v) = req.auto_micro_skills {
            c.economy.auto_micro_skills = v;
        }
        if let Some(v) = req.auto_tool_prune {
            c.economy.auto_tool_prune = v;
        }
        if let Some(v) = req.p90_loop_guard {
            c.economy.p90_loop_guard = v;
        }
        if let Some(v) = req.optional_fast_judge {
            c.economy.optional_fast_judge = v;
        }
    });
    match res {
        Ok(cfg) => Json(json!({"ok": true, "economy": cfg.economy})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct AddMicroSkillReq {
    fact: String,
    #[serde(default)]
    failed_cmd: Option<String>,
    #[serde(default)]
    fixed_cmd: Option<String>,
}

pub async fn api_add_micro_skill(
    State(state): State<WebState>,
    Json(req): Json<AddMicroSkillReq>,
) -> Response {
    if req.fact.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Micro-skill fact cannot be empty"})),
        )
            .into_response();
    }
    match state.runtime.economy_add_micro_skill(
        None,
        &req.fact,
        req.failed_cmd.as_deref(),
        req.fixed_cmd.as_deref(),
    ) {
        Ok(skill) => Json(json!({"ok": true, "skill": skill})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_retire_micro_skill(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match state.runtime.economy_retire_micro_skill(&id) {
        Ok(retired) => Json(json!({"ok": retired})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct ToolPruneReq {
    tool_name: String,
    #[serde(default = "default_pin_true")]
    pin: bool,
}

fn default_pin_true() -> bool {
    true
}

pub async fn api_prune_tool(
    State(state): State<WebState>,
    Json(req): Json<ToolPruneReq>,
) -> Response {
    match state.runtime.economy_apply_tool_prune(None, &req.tool_name) {
        Ok(true) => Json(json!({"ok": true})).into_response(),
        Ok(false) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Cannot prune protected or invalid tool `{}`", req.tool_name)})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_restore_tool(
    State(state): State<WebState>,
    Json(req): Json<ToolPruneReq>,
) -> Response {
    match state.runtime.economy_restore_tool(None, &req.tool_name, req.pin) {
        Ok(ok) => Json(json!({"ok": ok})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_reaudit_tools(State(state): State<WebState>) -> Response {
    match state.runtime.economy_reaudit_tools(None) {
        Ok(restored) => Json(json!({"ok": true, "restored": restored})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
