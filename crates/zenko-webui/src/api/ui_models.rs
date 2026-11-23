use super::*;

// ─── Models (100% Live Discovery — Zero Hardcoded Lists) ──────────────────────

#[derive(Deserialize)]
pub struct ModelsQuery {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    refresh: Option<bool>,
}

pub async fn api_models(
    State(state): State<WebState>,
    Query(q): Query<ModelsQuery>,
) -> Response {
    let cfg = state.runtime.config();
    let provider = q
        .provider
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&cfg.provider.kind);
    let force_refresh = q.refresh.unwrap_or(false);

    match state
        .runtime
        .list_models_for(provider, q.base_url.as_deref(), force_refresh)
        .await
    {
        Ok(models) => Json(models).into_response(),
        Err(_) => Json(Vec::<zenko_core::ModelInfo>::new()).into_response(),
    }
}

// ─── UI Customization API ─────────────────────────────────────────────────────

pub async fn api_update_active_ui(
    State(state): State<WebState>,
    Json(cfg): Json<UiCustomization>,
) -> Response {
    let mut active = state
        .runtime
        .store
        .get_active_ui_profile()
        .unwrap_or_else(|_| UiProfile {
            id: "default".into(),
            name: "Zenko Vermilion".into(),
            is_active: true,
            config: cfg.clone(),
            updated_at: Utc::now(),
        });
    active.config = cfg;
    active.updated_at = Utc::now();
    match state.runtime.store.save_ui_profile(&active) {
        Ok(()) => Json(active).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct ApplyPresetQuery {
    pub mode: Option<zenko_core::ui::ThemeMode>,
}

pub async fn api_apply_ui_preset(
    State(state): State<WebState>,
    Path(preset_id): Path<String>,
    Query(query): Query<ApplyPresetQuery>,
) -> Response {
    let mut active = state
        .runtime
        .store
        .get_active_ui_profile()
        .unwrap_or_else(|_| UiProfile {
            id: "default".into(),
            name: "Zenko Vermilion".into(),
            is_active: true,
            config: UiCustomization::default(),
            updated_at: Utc::now(),
        });
    let (_, legacy_mode) = UiCustomization::resolve_preset_alias(&preset_id);
    let target_mode = query.mode.or(legacy_mode).unwrap_or(active.config.mode);
    let new_preset = UiCustomization::preset_with_mode(&preset_id, Some(target_mode));
    active.config.mode = new_preset.mode;
    active.config.preset_id = new_preset.preset_id;
    active.config.colors = new_preset.colors;
    active.updated_at = Utc::now();
    match state.runtime.store.save_ui_profile(&active) {
        Ok(()) => Json(active).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct CreateUiProfileReq {
    name: String,
    config: UiCustomization,
}

pub async fn api_create_ui_profile(
    State(state): State<WebState>,
    Json(req): Json<CreateUiProfileReq>,
) -> Response {
    let profile = UiProfile {
        id: new_id(),
        name: req.name,
        is_active: true,
        config: req.config,
        updated_at: Utc::now(),
    };
    match state.runtime.store.save_ui_profile(&profile) {
        Ok(()) => Json(profile).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn api_activate_ui_profile(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Response {
    match state.runtime.store.activate_ui_profile(&id) {
        Ok(ok) => {
            let active = state.runtime.store.get_active_ui_profile().ok();
            Json(json!({"activated": ok, "active_profile": active})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

