use super::*;

// ─── Signed Webhooks & OpenAI-Compatible Agent-to-Agent Endpoints ─────────────

pub async fn api_webhook_ingest(
    State(state): State<WebState>,
    Path(source): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let cfg = state.runtime.config();
    if !cfg.webhooks.enabled {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "webhooks disabled in zenko.toml"})),
        )
            .into_response();
    }
    let secret = match state.runtime.vault.get(VAULT_KEY_WEBHOOK_SECRET) {
        Ok(Some(s)) if !s.trim().is_empty() => s,
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "webhook HMAC secret not configured in vault"})),
            )
                .into_response();
        }
    };
    let sig = headers
        .get(WEBHOOK_SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_hmac_sha256(secret.as_str(), &body, sig) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid HMAC signature"})),
        )
            .into_response();
    }

    let payload_text = String::from_utf8_lossy(&body).to_string();
    let payload_json = serde_json::from_slice::<Value>(&body).ok();

    // Evaluate declarative `[[webhooks.rules]]` (zero-regex RFC 6901 JSON pointer matching + rate limits)
    let matched_rule = cfg
        .webhooks
        .rules
        .iter()
        .find(|r| match_webhook_rule(r, &source, payload_json.as_ref()))
        .cloned();

    if let Some(rule) = matched_rule {
        let limit_key = format!("webhook:{}", rule.id);
        if let Err(reason) = state.runtime.trigger_limiter.check_and_record(
            &limit_key,
            rule.cooldown_secs,
            rule.max_per_hour,
        ) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({
                    "accepted": false,
                    "rule_id": rule.id,
                    "error": reason,
                })),
            )
                .into_response();
        }
        if let Some(ref gid) = rule
            .goal_loop_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            match state.runtime.resume_and_wake_goal(gid) {
                Ok(Some(goal)) => {
                    return (
                        StatusCode::ACCEPTED,
                        Json(json!({
                            "accepted": true,
                            "rule_id": rule.id,
                            "goal_loop_id": goal.id,
                            "session_id": goal.session_id,
                        })),
                    )
                        .into_response();
                }
                Ok(None) => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({"error": format!("goal loop `{gid}` not found for rule `{}`", rule.id)})),
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

        let prompt = render_webhook_prompt(&rule.prompt_template, &source, &rule.id, &payload_text);
        let sess_key = rule.session_key.as_deref().unwrap_or(&rule.id);
        let session = match state.runtime.store.find_or_create_channel_session(
            "webhook",
            sess_key,
            None,
            &format!("Webhook: {}", rule.id),
            cfg.policy.mode,
        ) {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": e.to_string()})),
                )
                    .into_response();
            }
        };
        let rt = state.runtime.clone();
        let sid = session.id.clone();
        let ws_override = rule
            .workspace_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(zenko_core::expand_tilde);
        let model_override = rule
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let gw_log = zenko_core::emit_hono_line(
            zenko_core::LogDirection::In,
            zenko_core::LogChannel::Gateway,
            &format!("webhook/{source} -> rule/{} ({}B)", rule.id, body.len()),
            Some(("202", zenko_core::LogStatusTone::Ok)),
            Some("routed"),
        )
        .with_session(sid.clone())
        .with_payload(json!({ "source": source, "rule_id": rule.id, "bytes": body.len() }));
        rt.logs.push(gw_log);
        tokio::spawn(async move {
            let _ = rt
                .run_turn_with_options(
                    &sid,
                    &prompt,
                    TurnOptions {
                        workspace_override: ws_override,
                        model_override,
                        headless_no_wait: !rt.is_telegram_connected(),
                        ..TurnOptions::default()
                    },
                )
                .await;
        });
        return (
            StatusCode::ACCEPTED,
            Json(json!({
                "accepted": true,
                "rule_id": rule.id,
                "session_id": session.id,
            })),
        )
            .into_response();
    }

    // If explicit routing rules exist for this source (or `*`) and none matched, ignore at 0 tokens!
    if has_webhook_rules_for_source(&cfg.webhooks.rules, &source) {
        let gw_log = zenko_core::emit_hono_line(
            zenko_core::LogDirection::In,
            zenko_core::LogChannel::Gateway,
            &format!("webhook/{source} ({}B) -> no rule matched", body.len()),
            Some(("200", zenko_core::LogStatusTone::Info)),
            Some("ignored"),
        )
        .with_payload(json!({ "source": source, "bytes": body.len(), "ignored": true }));
        state.runtime.logs.push(gw_log);
        return (
            StatusCode::OK,
            Json(json!({
                "accepted": false,
                "ignored": true,
                "source": source,
                "reason": "no matching webhook rule for source",
            })),
        )
            .into_response();
    }

    // Generic fallback (when no rules target this source) is still protected by cooldown & hourly cap.
    let fallback_key = format!("webhook_default:{source}");
    if let Err(reason) = state.runtime.trigger_limiter.check_and_record(
        &fallback_key,
        WEBHOOK_DEFAULT_COOLDOWN_SECS,
        WEBHOOK_DEFAULT_MAX_PER_HOUR,
    ) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "accepted": false,
                "source": source,
                "error": reason,
            })),
        )
            .into_response();
    }

    let prompt = format!("Incoming signed webhook from `{source}`:\n{payload_text}");
    let session = match state.runtime.store.find_or_create_channel_session(
        "webhook",
        &source,
        None,
        &format!("Webhook: {source}"),
        cfg.policy.mode,
    ) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };

    let rt = state.runtime.clone();
    let sid = session.id.clone();
    let gw_log = zenko_core::emit_hono_line(
        zenko_core::LogDirection::In,
        zenko_core::LogChannel::Gateway,
        &format!("webhook/{source} ({}B)", body.len()),
        Some(("202", zenko_core::LogStatusTone::Ok)),
        Some("verified"),
    )
    .with_session(sid.clone())
    .with_payload(json!({
        "source": source,
        "bytes": body.len(),
    }));
    rt.logs.push(gw_log);
    tokio::spawn(async move {
        let _ = rt
            .run_turn_with_options(
                &sid,
                &prompt,
                TurnOptions {
                    headless_no_wait: !rt.is_telegram_connected(),
                    ..TurnOptions::default()
                },
            )
            .await;
    });
    (
        StatusCode::ACCEPTED,
        Json(json!({"accepted": true, "session_id": session.id})),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct OpenAiMessage {
    role: String,
    content: Value,
}

#[derive(Deserialize)]
pub struct OpenAiChatReq {
    #[serde(default)]
    model: Option<String>,
    messages: Vec<OpenAiMessage>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    permission_mode: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
}

pub async fn api_openai_models(State(state): State<WebState>) -> Response {
    let models = state.runtime.list_models_cached().await.unwrap_or_default();
    let data: Vec<Value> = models
        .into_iter()
        .map(|m| {
            json!({
                "id": m.id,
                "object": "model",
                "owned_by": "zenko",
                "context_length": m.context_length,
                "is_free": m.is_free,
            })
        })
        .collect();
    Json(json!({ "object": "list", "data": data })).into_response()
}

/// OpenAI-compatible `/v1/chat/completions` endpoint allowing external AI agents, scripts,
/// and SDKs to invoke Zenko's full autonomous ReAct runtime (tools, memory, sessions).
pub async fn api_openai_chat_completions(
    State(state): State<WebState>,
    Json(req): Json<OpenAiChatReq>,
) -> Response {
    let prompt = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| match &m.content {
            Value::String(s) => s.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            other => other.to_string(),
        })
        .unwrap_or_default();

    if prompt.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": {"message": "missing user message content"}})),
        )
            .into_response();
    }

    let cfg = state.runtime.config();
    let perm_override = req
        .permission_mode
        .as_deref()
        .map(PermissionMode::from_str_loose);
    let sid = match req.session_id.or(req.user) {
        Some(existing) if state.runtime.store.get_session(&existing).ok().flatten().is_some() => {
            existing
        }
        Some(ext_key) => match state.runtime.store.find_or_create_channel_session(
            "api",
            &ext_key,
            None,
            &format!("API: {ext_key}"),
            perm_override.unwrap_or(cfg.policy.mode),
        ) {
            Ok(s) => s.id,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": {"message": e.to_string()}})),
                )
                    .into_response();
            }
        },
        None => match state.runtime.store.create_session(
            "Agent API Session",
            "api",
            None,
            None,
            perm_override.unwrap_or(cfg.policy.mode),
        ) {
            Ok(s) => s.id,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": {"message": e.to_string()}})),
                )
                    .into_response();
            }
        },
    };

    let model_override = req
        .model
        .filter(|m| !m.trim().is_empty() && m != "zenko" && m != "auto");

    match state
        .runtime
        .run_turn_with_options(
            &sid,
            &prompt,
            TurnOptions {
                parent_event_override: None,
                workspace_override: req.workspace.map(std::path::PathBuf::from),
                model_override,
                permission_override: perm_override,
                headless_no_wait: true,
                ..TurnOptions::default()
            },
        )
        .await
    {
        Ok(turn) => Json(json!({
            "id": format!("chatcmpl-{}", new_id()),
            "object": "chat.completion",
            "created": Utc::now().timestamp(),
            "model": turn.model,
            "zenko_session_id": turn.session_id,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": turn.reply
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": turn.usage.prompt_tokens,
                "completion_tokens": turn.usage.completion_tokens,
                "total_tokens": turn.usage.prompt_tokens + turn.usage.completion_tokens
            }
        }))
        .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": {"message": e.to_string(), "session_id": sid}})),
        )
            .into_response(),
    }
}

