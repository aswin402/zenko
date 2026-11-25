use super::*;

// ─── Native Token Economy API ─────────────────────────────────────────────────

pub async fn api_get_economy(State(state): State<WebState>) -> Response {
    match state.runtime.economy_report(None) {
        Ok(report) => {
            let mut val = serde_json::to_value(&report).unwrap_or_else(|_| json!({}));
            if let Some(obj) = val.as_object_mut() {
                obj.insert(
                    "minimalism_modes".into(),
                    serde_json::to_value(CODE_MINIMALISM_REGISTRY).unwrap_or(Value::Null),
                );
            }
            Json(val).into_response()
        }
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
    #[serde(default)]
    minimalism_mode: Option<String>,
    #[serde(default)]
    auto_bloat_hint: Option<bool>,
    #[serde(default)]
    session_id: Option<String>,
}

pub async fn api_update_economy_config(
    State(state): State<WebState>,
    Json(req): Json<UpdateEconomyConfigReq>,
) -> Response {
    let parsed_min = match req.minimalism_mode.as_deref() {
        Some(raw) => match CodeMinimalismMode::parse(raw) {
            Some(m) => Some(m),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("Invalid minimalism_mode `{raw}` (expected off, lite, full, or ultra)")})),
                )
                    .into_response();
            }
        },
        None => None,
    };
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
        if let Some(m) = parsed_min {
            c.economy.minimalism_mode = m;
        }
        if let Some(v) = req.auto_bloat_hint {
            c.economy.auto_bloat_hint = v;
        }
    });
    if let (Some(sid), Some(mode)) = (
        req.session_id.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        parsed_min,
    ) {
        state.runtime.set_session_minimalism_mode(sid, mode);
    }
    match res {
        Ok(cfg) => Json(json!({
            "ok": true,
            "economy": cfg.economy,
            "zmode_customized": cfg.is_zmode_customized(),
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

pub async fn api_economy_minimalism_debt(State(state): State<WebState>) -> Response {
    match state.runtime.minimalism_debt_scan(None).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct MinimalismAuditQuery {
    #[serde(default)]
    staged: bool,
}

pub async fn api_economy_minimalism_audit(
    State(state): State<WebState>,
    Query(q): Query<MinimalismAuditQuery>,
) -> Response {
    match state.runtime.minimalism_audit_workspace(None, q.staged).await {
        Ok((diff, repo)) => Json(json!({ "diff": diff, "repo": repo })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
