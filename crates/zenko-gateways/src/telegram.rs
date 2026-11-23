use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use zeroize::Zeroizing;
use zenko_core::{
    AgentEvent, ApprovalDecision, DEFAULT_RATE_LIMITER_MAX_ENTRIES, DEFAULT_RATE_LIMITER_TTL_SECS,
    GoalLoopRecord, PeerRole, PermissionMode, Result, SkillStatus, TELEGRAM_CB_APPROVAL_PREFIX,
    TELEGRAM_CB_CLARIFY_PREFIX, TELEGRAM_CB_SKILL_PREFIX, VAULT_KEY_TELEGRAM, ZenkoError,
    compact_preview,
};
use zenko_runtime::AgentRuntime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramBotInfo {
    pub id: i64,
    pub username: Option<String>,
    pub first_name: String,
}

pub async fn verify_bot_token(client: &Client, token: &str) -> Result<TelegramBotInfo> {
    let url = format!("https://api.telegram.org/bot{}/getMe", token.trim());
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| ZenkoError::Gateway(format!("telegram getMe failed: {e}")))?;
    let body: Value = resp
        .json()
        .await
        .map_err(|e| ZenkoError::Gateway(format!("invalid telegram json: {e}")))?;
    if !body.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        let desc = body
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("invalid bot token");
        return Err(ZenkoError::Gateway(format!("telegram rejected token: {desc}")));
    }
    let result = body
        .get("result")
        .ok_or_else(|| ZenkoError::Gateway("missing result in getMe".into()))?;
    serde_json::from_value(result.clone())
        .map_err(|e| ZenkoError::Gateway(format!("failed to decode bot info: {e}")))
}

/// Lightweight per-chat & global rate limiter honoring Telegram flood limits + 429 retry_after.
#[derive(Default)]
struct RateLimiter {
    per_chat: Mutex<HashMap<i64, Instant>>,
    global_last: Mutex<Option<Instant>>,
}

impl RateLimiter {
    async fn wait_slot(&self, chat_id: i64) {
        let wait_global = {
            let mut g = self.global_last.lock().await;
            let now = Instant::now();
            let interval = Duration::from_millis(35);
            let scheduled = g.map(|prev| (prev + interval).max(now)).unwrap_or(now);
            *g = Some(scheduled);
            scheduled.saturating_duration_since(now)
        };
        if !wait_global.is_zero() {
            tokio::time::sleep(wait_global).await;
        }

        let wait_chat = {
            let mut map = self.per_chat.lock().await;
            let now = Instant::now();
            if map.len() > DEFAULT_RATE_LIMITER_MAX_ENTRIES {
                let ttl = Duration::from_secs(DEFAULT_RATE_LIMITER_TTL_SECS);
                map.retain(|_, ts| now.saturating_duration_since(*ts) < ttl);
            }
            let interval = Duration::from_millis(950);
            let scheduled = map
                .get(&chat_id)
                .map(|prev| (*prev + interval).max(now))
                .unwrap_or(now);
            map.insert(chat_id, scheduled);
            scheduled.saturating_duration_since(now)
        };
        if !wait_chat.is_zero() {
            tokio::time::sleep(wait_chat).await;
        }
    }
}

pub struct TelegramGateway {
    runtime: Arc<AgentRuntime>,
    limiter: Arc<RateLimiter>,
    reload_notify: Arc<Notify>,
}

impl TelegramGateway {
    pub fn new(runtime: Arc<AgentRuntime>, reload_notify: Arc<Notify>) -> Self {
        Self {
            runtime,
            limiter: Arc::new(RateLimiter::default()),
            reload_notify,
        }
    }

    pub async fn run(self: Arc<Self>, cancel: CancellationToken) {
        let approval_self = self.clone();
        let approval_cancel = cancel.clone();
        tokio::spawn(async move {
            approval_self.run_approval_forwarder(approval_cancel).await;
        });

        loop {
            if cancel.is_cancelled() {
                break;
            }
            let cfg = self.runtime.config();
            let token_opt = self.runtime.vault.get(VAULT_KEY_TELEGRAM).ok().flatten();

            if !cfg.telegram.enabled || token_opt.as_deref().map(|s| s.trim().is_empty()).unwrap_or(true) {
                self.runtime.set_telegram_connected(false);
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = self.reload_notify.notified() => continue,
                    _ = tokio::time::sleep(Duration::from_secs(10)) => continue,
                }
            }

            let Some(token) = token_opt else {
                continue;
            };

            match verify_bot_token(&self.runtime.http, &token).await {
                Ok(info) => {
                    let bot_label = info.username.as_deref().unwrap_or(&info.first_name);
                    let entry = zenko_core::emit_hono_line(
                        zenko_core::LogDirection::Out,
                        zenko_core::LogChannel::Gateway,
                        &format!("telegram @{bot_label}"),
                        Some(("connected", zenko_core::LogStatusTone::Ok)),
                        Some("polling"),
                    );
                    self.runtime.logs.push(entry);
                    self.runtime.set_telegram_connected(true);
                    if cfg.telegram.bot_username != info.username {
                        let uname = info.username.clone();
                        let _ = self.runtime.update_config(|c| c.telegram.bot_username = uname);
                    }
                    self.poll_loop(token, cancel.clone()).await;
                    self.runtime.set_telegram_connected(false);
                }
                Err(e) => {
                    let entry = zenko_core::emit_hono_line(
                        zenko_core::LogDirection::Warn,
                        zenko_core::LogChannel::Gateway,
                        &format!("telegram · {}", zenko_core::compact_preview(&e.to_string(), 64)),
                        Some(("retry", zenko_core::LogStatusTone::Warn)),
                        Some("15s"),
                    );
                    self.runtime.logs.push(entry);
                    self.runtime.set_telegram_connected(false);
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = self.reload_notify.notified() => {}
                        _ = tokio::time::sleep(Duration::from_secs(15)) => {}
                    }
                }
            }
        }
    }

    async fn poll_loop(self: &Arc<Self>, token: Zeroizing<String>, cancel: CancellationToken) {
        let mut offset: i64 = self
            .runtime
            .store
            .get_kv("telegram_last_update_id")
            .ok()
            .flatten()
            .and_then(|s| s.parse::<i64>().ok())
            .map(|id| id + 1)
            .unwrap_or(0);

        loop {
            if cancel.is_cancelled() || !self.runtime.config().telegram.enabled {
                break;
            }
            let timeout = self.runtime.config().telegram.poll_timeout_secs.clamp(5, 50);
            let url = format!("https://api.telegram.org/bot{}/getUpdates", token.as_str());
            let body = json!({
                "offset": offset,
                "timeout": timeout,
                "allowed_updates": ["message", "callback_query"]
            });

            let resp = tokio::select! {
                _ = cancel.cancelled() => break,
                _ = self.reload_notify.notified() => break,
                r = self.runtime.http.post(&url).json(&body).timeout(Duration::from_secs(timeout + 10)).send() => r,
            };

            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "telegram getUpdates network error");
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    continue;
                }
            };

            let parsed: Value = match resp.json().await {
                Ok(v) => v,
                Err(_) => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };

            if !parsed.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                let retry_after = parsed
                    .get("parameters")
                    .and_then(|p| p.get("retry_after"))
                    .and_then(Value::as_u64)
                    .unwrap_or(3);
                tokio::time::sleep(Duration::from_secs(retry_after)).await;
                continue;
            }

            if let Some(updates) = parsed.get("result").and_then(Value::as_array) {
                for upd in updates {
                    if let Some(uid) = upd.get("update_id").and_then(Value::as_i64) {
                        offset = uid + 1;
                        let _ = self
                            .runtime
                            .store
                            .set_kv("telegram_last_update_id", &uid.to_string());
                    }
                    let this = self.clone();
                    let tok = token.clone();
                    let update_val = upd.clone();
                    tokio::spawn(async move {
                        this.handle_update(&tok, update_val).await;
                    });
                }
            }
        }
    }

    async fn handle_update(self: &Arc<Self>, token: &str, update: Value) {
        if let Some(cb) = update.get("callback_query") {
            self.handle_callback_query(token, cb).await;
            return;
        }
        let Some(msg) = update.get("message") else {
            return;
        };
        let Some(chat_id) = msg.get("chat").and_then(|c| c.get("id")).and_then(Value::as_i64) else {
            return;
        };
        let thread_id = msg.get("message_thread_id").and_then(Value::as_i64);
        let from_id = msg
            .get("from")
            .and_then(|f| f.get("id"))
            .and_then(Value::as_i64)
            .unwrap_or(chat_id);
        let username = msg
            .get("from")
            .and_then(|f| f.get("username"))
            .and_then(Value::as_str);
        let display_name = msg
            .get("from")
            .and_then(|f| f.get("first_name"))
            .and_then(Value::as_str);
        let Some(text) = msg.get("text").and_then(Value::as_str) else {
            return;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }

        let peer_id_str = from_id.to_string();

        // 1. Pairing command (allowed before authorization)
        if let Some(code_raw) = trimmed.strip_prefix("/pair") {
            let code = code_raw.trim();
            if code.is_empty() {
                let _ = self
                    .send_message(
                        token,
                        chat_id,
                        thread_id,
                        "Usage: `/pair <6-CHAR-CODE>` (generate a code in `zenko gateway pair` or WebUI -> Gateways).",
                        None,
                    )
                    .await;
                return;
            }
            match self
                .runtime
                .store
                .redeem_pairing_code("telegram", code, &peer_id_str, username, display_name)
            {
                Ok(Some(peer)) => {
                    self.runtime.emit(AgentEvent::GatewayPaired { peer: peer.clone() });
                    let _ = self
                        .send_message(
                            token,
                            chat_id,
                            thread_id,
                            &format!(
                                "Paired with Zenko as **{}**! Send any prompt to start.",
                                peer.role.as_str()
                            ),
                            None,
                        )
                        .await;
                }
                _ => {
                    let _ = self
                        .send_message(
                            token,
                            chat_id,
                            thread_id,
                            "Invalid or expired pairing code. Generate a fresh code from WebUI or `zenko gateway pair`.",
                            None,
                        )
                        .await;
                }
            }
            return;
        }

        // 2. Authorization check
        let peer = match self.runtime.store.get_peer("telegram", &peer_id_str) {
            Ok(Some(p)) => p,
            _ => {
                debug!(peer_id = %peer_id_str, "ignoring unauthorized telegram message");
                if trimmed == "/start" {
                    let _ = self
                        .send_message(
                            token,
                            chat_id,
                            thread_id,
                            "Zenko is locked by default. Pair this account with `/pair <CODE>` from your Zenko WebUI or CLI.",
                            None,
                        )
                        .await;
                }
                return;
            }
        };

        let chat_key = chat_id.to_string();
        let thread_key = thread_id.map(|t| t.to_string());
        let default_mode = match peer.role {
            PeerRole::ReadOnly => PermissionMode::Strict,
            _ => self.runtime.config().telegram.default_permission,
        };

        // 3. Built-in Telegram slash commands
        if trimmed.starts_with('/') {
            let mut parts = trimmed.splitn(2, ' ');
            let cmd = parts.next().unwrap_or("").split('@').next().unwrap_or("");
            let arg = parts.next().unwrap_or("").trim();

            match cmd {
                "/start" | "/help" => {
                    let help = "Zenko 24/7 Rust Agent\n\nCommands:\n\
                        /new — start a fresh session in this chat/topic\n\
                        /stop — cancel active generation\n\
                        /status — daemon RAM, uptime, cost & model\n\
                        /model <id> — switch OpenRouter model for this session\n\
                        /mode <strict|balanced|autonomous> — change tool approval policy\n\
                        /help — show this help";
                    let _ = self.send_message(token, chat_id, thread_id, help, None).await;
                    return;
                }
                "/status" => {
                    let t = self.runtime.telemetry();
                    let cfg = self.runtime.config();
                    let msg = format!(
                        "Zenko Status\n• RAM: {:.1} MB RSS\n• Uptime: {}s\n• Model: {}\n• Today Spend: ${:.4} / ${:.2}\n• Cache Hit: {:.1}%\n• Active Sessions: {}",
                        t.rss_bytes as f64 / (1024.0 * 1024.0),
                        t.uptime_secs,
                        cfg.provider.lead_model,
                        t.daily_cost_usd,
                        cfg.general.daily_spend_limit_usd,
                        t.cache_hit_pct,
                        t.active_sessions
                    );
                    let _ = self.send_message(token, chat_id, thread_id, &msg, None).await;
                    return;
                }
                "/new" => {
                    let title = format!("Telegram {}", display_name.unwrap_or(&chat_key));
                    let _ = self.runtime.store.create_session(
                        &title,
                        "telegram",
                        Some(&chat_key),
                        thread_key.as_deref(),
                        default_mode,
                    );
                    let _ = self
                        .send_message(token, chat_id, thread_id, "Started a fresh Zenko session.", None)
                        .await;
                    return;
                }
                "/stop" => {
                    if let Ok(session) = self.runtime.store.find_or_create_channel_session(
                        "telegram",
                        &chat_key,
                        thread_key.as_deref(),
                        "Telegram",
                        default_mode,
                    ) {
                        let cancelled = self.runtime.cancel_session(&session.id);
                        let reply = if cancelled {
                            "Cancelled active turn."
                        } else {
                            "No active generation in this session."
                        };
                        let _ = self.send_message(token, chat_id, thread_id, reply, None).await;
                    }
                    return;
                }
                "/model" if !arg.is_empty() => {
                    if let Ok(session) = self.runtime.store.find_or_create_channel_session(
                        "telegram",
                        &chat_key,
                        thread_key.as_deref(),
                        "Telegram",
                        default_mode,
                    ) {
                        let _ = self
                            .runtime
                            .store
                            .update_session_meta(&session.id, None, Some(Some(arg)), None);
                        let _ = self
                            .send_message(
                                token,
                                chat_id,
                                thread_id,
                                &format!("Session model switched to `{arg}`."),
                                None,
                            )
                            .await;
                    }
                    return;
                }
                "/mode" if !arg.is_empty() => {
                    if peer.role == PeerRole::ReadOnly {
                        let _ = self
                            .send_message(token, chat_id, thread_id, "ReadOnly peers cannot change permission mode.", None)
                            .await;
                        return;
                    }
                    let new_mode = PermissionMode::from_str_loose(arg);
                    if let Ok(session) = self.runtime.store.find_or_create_channel_session(
                        "telegram",
                        &chat_key,
                        thread_key.as_deref(),
                        "Telegram",
                        default_mode,
                    ) {
                        let _ = self
                            .runtime
                            .store
                            .update_session_meta(&session.id, None, None, Some(new_mode));
                        let _ = self
                            .send_message(
                                token,
                                chat_id,
                                thread_id,
                                &format!("Permission mode set to `{}`.", new_mode.as_str()),
                                None,
                            )
                            .await;
                    }
                    return;
                }
                _ => {}
            }
        }

        // 4. Resolve topic-isolated session and stream live response
        let title = format!(
            "TG: {}{}",
            display_name.unwrap_or(&chat_key),
            thread_id.map(|t| format!(" #{t}")).unwrap_or_default()
        );
        let Ok(session) = self.runtime.store.find_or_create_channel_session(
            "telegram",
            &chat_key,
            thread_key.as_deref(),
            &title,
            default_mode,
        ) else {
            return;
        };

        let _ = self.send_typing(token, chat_id, thread_id).await;

        // Spawn live draft streamer tied to explicit `turn_done` cancellation token so
        // multi-step tool calls stream seamlessly and early errors never deadlock.
        let sid = session.id.clone();
        let stream_drafts = self.runtime.config().telegram.stream_drafts;
        let turn_done = CancellationToken::new();
        let streamer_handle = match stream_drafts {
            true => {
                let mut rx = self.runtime.subscribe();
                let this = self.clone();
                let tok = token.to_string();
                let s_id = sid.clone();
                let done_sig = turn_done.clone();
                Some(tokio::spawn(async move {
                    let mut acc = String::new();
                    let mut last_flush: Option<Instant> = None;
                    let draft_id: i64 = (chrono::Utc::now().timestamp_millis() % 1_000_000).max(1);
                    let mut draft_supported = true;
                    let mut fallback_msg_id: Option<i64> = None;

                    loop {
                        let ev = tokio::select! {
                            biased;
                            _ = done_sig.cancelled() => break,
                            msg = rx.recv() => match msg {
                                Ok(e) => e,
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                            }
                        };
                        match ev {
                            AgentEvent::TextDelta { session_id, text, .. } if session_id == s_id => {
                                acc.push_str(&text);
                                let cadence = match draft_supported {
                                    true => Duration::from_millis(450),
                                    false => Duration::from_millis(1200),
                                };
                                let due = last_flush.is_none_or(|t| t.elapsed() >= cadence);
                                if due && !acc.trim().is_empty() {
                                    let preview = truncate_telegram(&acc, 3900);
                                    if draft_supported {
                                        if this
                                            .send_message_draft(&tok, chat_id, thread_id, draft_id, &preview)
                                            .await
                                            .is_err()
                                        {
                                            draft_supported = false;
                                        }
                                    }
                                    if !draft_supported {
                                        match fallback_msg_id {
                                            None => {
                                                if let Ok(mid) = this
                                                    .send_message(&tok, chat_id, thread_id, &preview, None)
                                                    .await
                                                {
                                                    if mid > 0 {
                                                        fallback_msg_id = Some(mid);
                                                    }
                                                }
                                            }
                                            Some(mid) => {
                                                let _ = this
                                                    .edit_message_text(&tok, chat_id, mid, &preview, None)
                                                    .await;
                                            }
                                        }
                                    }
                                    last_flush = Some(Instant::now());
                                }
                            }
                            AgentEvent::ToolCallStarted { session_id, .. } if session_id == s_id => {
                                acc.clear();
                            }
                            _ => {}
                        }
                    }
                    fallback_msg_id
                }))
            }
            false => None,
        };

        let turn_res = self.runtime.run_turn(&sid, trimmed, None).await;
        turn_done.cancel();
        let fallback_mid = match streamer_handle {
            Some(h) => h.await.ok().flatten(),
            None => None,
        };

        match turn_res {
            Ok(reply) => {
                let text_out = if reply.trim().is_empty() {
                    "(Done — no text output)".to_string()
                } else {
                    reply
                };
                let chunks = split_telegram_chunks(&text_out, 4000);
                for (idx, chunk) in chunks.iter().enumerate() {
                    if idx == 0 {
                        if let Some(mid) = fallback_mid.filter(|&m| m > 0) {
                            if self
                                .edit_message_text(token, chat_id, mid, chunk, None)
                                .await
                                .is_ok()
                            {
                                continue;
                            }
                        }
                    }
                    let _ = self.send_message(token, chat_id, thread_id, chunk, None).await;
                }
            }
            Err(ZenkoError::Cancelled) => {
                let _ = self
                    .send_message(token, chat_id, thread_id, "Stopped.", None)
                    .await;
            }
            Err(e) => {
                let _ = self
                    .send_message(token, chat_id, thread_id, &format!("Error: {e}"), None)
                    .await;
            }
        }
    }

    async fn handle_callback_query(self: &Arc<Self>, token: &str, cb: &Value) {
        let Some(cb_id) = cb.get("id").and_then(Value::as_str) else {
            return;
        };
        let Some(data) = cb.get("data").and_then(Value::as_str) else {
            return;
        };
        let from_id = cb
            .get("from")
            .and_then(|f| f.get("id"))
            .and_then(Value::as_i64)
            .map(|i| i.to_string())
            .unwrap_or_default();
        let username = cb
            .get("from")
            .and_then(|f| f.get("username"))
            .and_then(Value::as_str)
            .unwrap_or("telegram");

        // Verify caller is paired & check role
        let caller_peer = self
            .runtime
            .store
            .get_peer("telegram", &from_id)
            .ok()
            .flatten();
        let authorized = caller_peer
            .as_ref()
            .map(|p| p.role != PeerRole::ReadOnly)
            .unwrap_or(false);

        if !authorized {
            let _ = self
                .answer_callback_query(token, cb_id, "Not authorized to approve tools.")
                .await;
            return;
        }

        let parts: Vec<&str> = data.split(':').collect();
        match parts.as_slice() {
            [prefix, approval_id, action] if *prefix == TELEGRAM_CB_APPROVAL_PREFIX => {
                let decision = match *action {
                    "allow" => ApprovalDecision::Approve,
                    "always" => ApprovalDecision::AlwaysAllow,
                    _ => ApprovalDecision::Deny,
                };
                let label = match decision {
                    ApprovalDecision::Approve => "Approved",
                    ApprovalDecision::AlwaysAllow => "Always Allowed",
                    ApprovalDecision::Deny => "Denied",
                };
                let _ = self
                    .answer_callback_query(token, cb_id, &format!("{label} tool execution."))
                    .await;

                let _ = self.runtime.resolve_approval(
                    approval_id,
                    decision,
                    &format!("telegram:@{username}"),
                );

                self.append_callback_decision(token, cb, "Tool approval", &format!("{label} by @{username}"))
                    .await;
            }
            [prefix, clarification_id, idx_raw] if *prefix == TELEGRAM_CB_CLARIFY_PREFIX => {
                let resolved = idx_raw.parse::<usize>().ok().and_then(|idx| {
                    self.runtime.clarifications.resolve_clarification_by_index(
                        clarification_id,
                        idx,
                        &format!("telegram:@{username}"),
                    )
                });
                let (toast, suffix) = match resolved {
                    Some(rec) => {
                        let chosen = rec.selected_option_id.as_deref().unwrap_or("selected");
                        (
                            format!("Selected option `{chosen}`."),
                            format!("Selected `{chosen}` by @{username}"),
                        )
                    }
                    None => (
                        "Decision already resolved or expired.".to_string(),
                        "Expired or already resolved".to_string(),
                    ),
                };
                let _ = self.answer_callback_query(token, cb_id, &toast).await;
                self.append_callback_decision(token, cb, "Design decision", &suffix)
                    .await;
            }
            [prefix, action, slug] if *prefix == TELEGRAM_CB_SKILL_PREFIX => {
                let is_owner = caller_peer
                    .as_ref()
                    .is_some_and(|p| p.role == PeerRole::Owner);
                if !is_owner {
                    let _ = self
                        .answer_callback_query(token, cb_id, "Owner role required to approve skills.")
                        .await;
                    return;
                }
                match *action {
                    "approve" => {
                        let max_h = self.runtime.config().skills.max_history_snapshots;
                        match self.runtime.snapshots.approve_skill(slug, max_h) {
                            Ok((sk, _)) => {
                                self.runtime.emit(AgentEvent::SkillUpdated {
                                    slug: sk.slug.clone(),
                                    action: "approve".into(),
                                    status: sk.status.as_str().into(),
                                });
                                let _ = self
                                    .answer_callback_query(
                                        token,
                                        cb_id,
                                        &format!("Approved skill `{}`.", sk.slug),
                                    )
                                    .await;
                                self.append_callback_decision(
                                    token,
                                    cb,
                                    "Draft skill approval",
                                    &format!("Approved `{}` by @{username}", sk.slug),
                                )
                                .await;
                            }
                            Err(e) => {
                                let _ = self
                                    .answer_callback_query(
                                        token,
                                        cb_id,
                                        &format!("Approve failed: {}", compact_preview(&e.to_string(), 120)),
                                    )
                                    .await;
                            }
                        }
                    }
                    _ => {
                        let _ = self
                            .answer_callback_query(token, cb_id, &format!("Left `{slug}` as draft."))
                            .await;
                        self.append_callback_decision(
                            token,
                            cb,
                            "Draft skill approval",
                            &format!("Left `{slug}` as draft by @{username}"),
                        )
                        .await;
                    }
                }
            }
            _ => {}
        }
    }

    async fn append_callback_decision(
        &self,
        token: &str,
        cb: &Value,
        fallback_title: &str,
        decision_text: &str,
    ) {
        if let (Some(chat_id), Some(msg_id)) = (
            cb.get("message")
                .and_then(|m| m.get("chat"))
                .and_then(|c| c.get("id"))
                .and_then(Value::as_i64),
            cb.get("message")
                .and_then(|m| m.get("message_id"))
                .and_then(Value::as_i64),
        ) {
            let orig = cb
                .get("message")
                .and_then(|m| m.get("text"))
                .and_then(Value::as_str)
                .unwrap_or(fallback_title);
            let updated = format!("{orig}\n\nDecision: {decision_text}");
            let _ = self
                .edit_message_text(
                    token,
                    chat_id,
                    msg_id,
                    &updated,
                    Some(json!({"inline_keyboard": []})),
                )
                .await;
        }
    }

    fn resolve_telegram_owner_targets(&self) -> Vec<(i64, Option<i64>)> {
        self.runtime
            .store
            .list_peers(Some("telegram"))
            .unwrap_or_default()
            .into_iter()
            .filter(|p| p.role == PeerRole::Owner)
            .filter_map(|p| p.peer_id.parse::<i64>().ok().map(|id| (id, None)))
            .collect()
    }

    fn resolve_telegram_targets(&self, session_id: &str) -> Vec<(i64, Option<i64>)> {
        match self.runtime.store.get_session(session_id) {
            Ok(Some(s)) if s.channel == "telegram" => {
                let cid = s.external_chat_id.and_then(|c| c.parse::<i64>().ok());
                let tid = s.external_thread_id.and_then(|t| t.parse::<i64>().ok());
                cid.map(|c| vec![(c, tid)]).unwrap_or_default()
            }
            _ => self.resolve_telegram_owner_targets(),
        }
    }

    async fn run_approval_forwarder(self: Arc<Self>, cancel: CancellationToken) {
        let mut rx = self.runtime.subscribe();
        loop {
            let ev = tokio::select! {
                _ = cancel.cancelled() => break,
                item = rx.recv() => match item {
                    Ok(e) => e,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            };
            let Some(token) = self.runtime.vault.get(VAULT_KEY_TELEGRAM).ok().flatten() else {
                continue;
            };
            if !self.runtime.config().telegram.enabled {
                continue;
            }

            match ev {
                AgentEvent::ToolApprovalRequested { approval } => {
                    let targets = self.resolve_telegram_targets(&approval.session_id);
                    let text = format!(
                        "Approval Required [{}]\nTool: {}\nArgs: {}",
                        approval.risk.as_str().to_uppercase(),
                        approval.tool_name,
                        approval.args_preview
                    );
                    let markup = json!({
                        "inline_keyboard": [[
                            {"text": "Approve", "callback_data": format!("{TELEGRAM_CB_APPROVAL_PREFIX}:{}:allow", approval.id)},
                            {"text": "Always Allow", "callback_data": format!("{TELEGRAM_CB_APPROVAL_PREFIX}:{}:always", approval.id)},
                            {"text": "Deny", "callback_data": format!("{TELEGRAM_CB_APPROVAL_PREFIX}:{}:deny", approval.id)}
                        ]]
                    });
                    for (chat_id, thread_id) in targets {
                        let _ = self
                            .send_message(token.as_str(), chat_id, thread_id, &text, Some(markup.clone()))
                            .await;
                    }
                }
                AgentEvent::ClarificationRequested { clarification } => {
                    let targets = self.resolve_telegram_targets(&clarification.session_id);
                    let mut lines = vec![format!("Design Decision Needed\n{}", clarification.question)];
                    if !clarification.context.trim().is_empty() {
                        lines.push(clarification.context.clone());
                    }
                    let mut rows: Vec<Value> = Vec::with_capacity(clarification.options.len());
                    for (idx, opt) in clarification.options.iter().enumerate() {
                        let rec = match opt.recommended {
                            true => " (recommended)",
                            false => "",
                        };
                        let desc = match opt.description.trim().is_empty() {
                            true => String::new(),
                            false => format!(" — {}", opt.description),
                        };
                        lines.push(format!("{}. {}{rec}{desc}", idx + 1, opt.label));
                        rows.push(json!([{
                            "text": format!("{}{rec}", opt.label),
                            "callback_data": format!("{TELEGRAM_CB_CLARIFY_PREFIX}:{}:{idx}", clarification.id)
                        }]));
                    }
                    let text = lines.join("\n");
                    let markup = json!({ "inline_keyboard": rows });
                    for (chat_id, thread_id) in targets {
                        let _ = self
                            .send_message(token.as_str(), chat_id, thread_id, &text, Some(markup.clone()))
                            .await;
                    }
                }
                AgentEvent::GoalLoopUpdated { goal } if should_notify_telegram_goal(&goal) => {
                    let targets = self.resolve_telegram_targets(&goal.session_id);
                    let text = format_goal_loop_telegram_alert(&goal);
                    for (chat_id, thread_id) in targets {
                        let _ = self
                            .send_message(token.as_str(), chat_id, thread_id, &text, None)
                            .await;
                    }
                }
                AgentEvent::SkillUpdated { slug, action, status }
                    if status == SkillStatus::Draft.as_str()
                        && matches!(action.as_str(), "create" | "dream_propose" | "rollback") =>
                {
                    let targets = self.resolve_telegram_owner_targets();
                    let (text, markup) = format_draft_skill_telegram_alert(&slug, &action);
                    for (chat_id, thread_id) in targets {
                        let _ = self
                            .send_message(token.as_str(), chat_id, thread_id, &text, markup.clone())
                            .await;
                    }
                }
                _ => {}
            }
        }
    }

    async fn send_typing(&self, token: &str, chat_id: i64, thread_id: Option<i64>) -> Result<()> {
        let url = format!("https://api.telegram.org/bot{token}/sendChatAction");
        let mut payload = json!({ "chat_id": chat_id, "action": "typing" });
        if let (Some(tid), Some(obj)) = (thread_id, payload.as_object_mut()) {
            obj.insert("message_thread_id".into(), json!(tid));
        }
        let _ = self.runtime.http.post(&url).json(&payload).send().await;
        Ok(())
    }

    async fn send_message_draft(
        &self,
        token: &str,
        chat_id: i64,
        thread_id: Option<i64>,
        draft_id: i64,
        text: &str,
    ) -> Result<()> {
        let url = format!("https://api.telegram.org/bot{token}/sendMessageDraft");
        let mut payload = json!({
            "chat_id": chat_id,
            "draft_id": draft_id,
            "text": text
        });
        if let (Some(tid), Some(obj)) = (thread_id, payload.as_object_mut()) {
            obj.insert("message_thread_id".into(), json!(tid));
        }
        let resp = self
            .runtime
            .http
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| ZenkoError::Gateway(e.to_string()))?;
        let val: Value = resp
            .json()
            .await
            .map_err(|e| ZenkoError::Gateway(e.to_string()))?;
        if val.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            Ok(())
        } else {
            Err(ZenkoError::Gateway("sendMessageDraft unsupported".into()))
        }
    }

    async fn send_message(
        &self,
        token: &str,
        chat_id: i64,
        thread_id: Option<i64>,
        text: &str,
        reply_markup: Option<Value>,
    ) -> Result<i64> {
        self.limiter.wait_slot(chat_id).await;
        let url = format!("https://api.telegram.org/bot{token}/sendMessage");
        let mut payload = json!({
            "chat_id": chat_id,
            "text": text
        });
        if let Some(obj) = payload.as_object_mut() {
            if let Some(tid) = thread_id {
                obj.insert("message_thread_id".into(), json!(tid));
            }
            if let Some(rm) = reply_markup {
                obj.insert("reply_markup".into(), rm);
            }
        }
        let resp = self
            .runtime
            .http
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| ZenkoError::Gateway(e.to_string()))?;
        let status = resp.status();
        let val: Value = resp
            .json()
            .await
            .map_err(|e| ZenkoError::Gateway(e.to_string()))?;
        if !status.is_success() || !val.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            let desc = val
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("telegram sendMessage failed");
            return Err(ZenkoError::Gateway(format!("sendMessage ({status}): {desc}")));
        }
        let mid = val
            .get("result")
            .and_then(|r| r.get("message_id"))
            .and_then(Value::as_i64)
            .filter(|&id| id > 0)
            .ok_or_else(|| ZenkoError::Gateway("sendMessage returned missing/zero message_id".into()))?;
        Ok(mid)
    }

    async fn edit_message_text(
        &self,
        token: &str,
        chat_id: i64,
        message_id: i64,
        text: &str,
        reply_markup: Option<Value>,
    ) -> Result<()> {
        if message_id <= 0 {
            return Err(ZenkoError::Gateway("invalid telegram message_id".into()));
        }
        self.limiter.wait_slot(chat_id).await;
        let url = format!("https://api.telegram.org/bot{token}/editMessageText");
        let mut payload = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": text
        });
        if let (Some(rm), Some(obj)) = (reply_markup, payload.as_object_mut()) {
            obj.insert("reply_markup".into(), rm);
        }
        let resp = self
            .runtime
            .http
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| ZenkoError::Gateway(e.to_string()))?;
        let status = resp.status();
        let val: Value = resp
            .json()
            .await
            .map_err(|e| ZenkoError::Gateway(e.to_string()))?;
        if status.is_success() && val.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(());
        }
        let desc = val
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("telegram editMessageText failed");
        if desc.to_ascii_lowercase().contains("message is not modified") {
            return Ok(());
        }
        Err(ZenkoError::Gateway(format!(
            "editMessageText ({status}): {desc}"
        )))
    }

    async fn answer_callback_query(&self, token: &str, cb_id: &str, text: &str) -> Result<()> {
        let url = format!("https://api.telegram.org/bot{token}/answerCallbackQuery");
        let _ = self
            .runtime
            .http
            .post(&url)
            .json(&json!({ "callback_query_id": cb_id, "text": text }))
            .send()
            .await;
        Ok(())
    }
}

fn truncate_telegram(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let cut = s.floor_char_boundary(max);
        format!("{}...", &s[..cut])
    }
}

pub fn split_telegram_chunks(text: &str, max: usize) -> Vec<String> {
    if text.len() <= max {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.len() <= max {
            chunks.push(remaining.to_string());
            break;
        }
        let cut = remaining.floor_char_boundary(max);
        let split_at = match cut {
            0 => remaining.chars().next().map(|c| c.len_utf8()).unwrap_or(1),
            n => remaining[..n].rfind('\n').filter(|&p| p > 0).unwrap_or(n),
        };
        chunks.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start_matches('\n');
    }
    chunks
}

fn should_notify_telegram_goal(goal: &GoalLoopRecord) -> bool {
    goal.status.is_terminal()
        && (goal.notify_channel.eq_ignore_ascii_case("telegram")
            || goal.notify_channel.eq_ignore_ascii_case("all"))
}

fn format_goal_loop_telegram_alert(goal: &GoalLoopRecord) -> String {
    let mut text = format!(
        "Goal Loop [{}] — {}\nID: {} · Iter: {}/{} · Spend: ${:.4}/${:.2}\nVerify: `{}` (exit={:?})",
        goal.status.as_str().to_uppercase(),
        goal.name,
        goal.id,
        goal.iterations_done,
        goal.max_iterations,
        goal.cost_spent_usd,
        goal.max_budget_usd,
        goal.verify_cmd.join(" "),
        goal.last_verify_exit,
    );
    if let Some(out) = goal
        .last_verify_output
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        text.push_str(&format!("\nOutput: {}", compact_preview(out, 280)));
    }
    text
}

fn format_draft_skill_telegram_alert(slug: &str, action: &str) -> (String, Option<Value>) {
    let approve_cb = format!("{TELEGRAM_CB_SKILL_PREFIX}:approve:{slug}");
    let dismiss_cb = format!("{TELEGRAM_CB_SKILL_PREFIX}:dismiss:{slug}");
    // Telegram enforces a strict 64-byte callback_data ceiling.
    match approve_cb.len() <= 64 && dismiss_cb.len() <= 64 {
        true => (
            format!(
                "Draft Skill Awaiting Approval\nSlug: `{slug}` (source: {action})\nTap Approve to activate in <available_skills> or Leave Draft."
            ),
            Some(json!({
                "inline_keyboard": [[
                    {"text": "Approve Skill", "callback_data": approve_cb},
                    {"text": "Leave Draft", "callback_data": dismiss_cb}
                ]]
            })),
        ),
        false => (
            format!(
                "Draft Skill Awaiting Approval\nSlug: `{slug}` (source: {action})\nApprove via `zenko skills approve {slug}` or WebUI Memory -> Skills."
            ),
            None,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use zenko_core::GoalLoopStatus;

    #[test]
    fn splits_long_telegram_messages_safely() {
        let long = "line\n".repeat(1200);
        let chunks = split_telegram_chunks(&long, 4000);
        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|c| c.len() <= 4000));
    }

    #[test]
    fn formats_phase8_telegram_goal_and_draft_skill_alerts() {
        let now = Utc::now();
        let goal = GoalLoopRecord {
            id: "goal_tg1".into(),
            name: "Fix Clippy".into(),
            session_id: "sess_1".into(),
            workspace_dir: ".".into(),
            goal_text: "Fix".into(),
            verify_cmd: vec!["cargo".into(), "clippy".into()],
            interval_secs: 0,
            max_iterations: 5,
            max_budget_usd: 1.5,
            max_consecutive_stalls: 2,
            checkpoint_each_iter: true,
            auto_rewind_on_stall: false,
            model: None,
            notify_channel: "telegram".into(),
            status: GoalLoopStatus::Succeeded,
            iterations_done: 2,
            consecutive_stalls: 0,
            cost_spent_usd: 0.04,
            last_verify_exit: Some(0),
            last_verify_hash: Some("ok".into()),
            last_verify_output: Some("0 warnings".into()),
            next_run_at: None,
            created_at: now,
            updated_at: now,
        };
        assert!(should_notify_telegram_goal(&goal));
        let alert = format_goal_loop_telegram_alert(&goal);
        assert!(alert.contains("SUCCEEDED"));
        assert!(alert.contains("Fix Clippy"));

        let (sk_text, sk_markup) =
            format_draft_skill_telegram_alert("rust-release", "dream_propose");
        assert!(sk_text.contains("rust-release"));
        let cb = sk_markup.expect("markup")["inline_keyboard"][0][0]["callback_data"]
            .as_str()
            .expect("cb")
            .to_string();
        assert_eq!(cb, "ska:approve:rust-release");

        // Slugs that would exceed Telegram's 64-byte callback_data limit gracefully omit buttons
        let long_slug = "a".repeat(60);
        let (_, long_markup) = format_draft_skill_telegram_alert(&long_slug, "create");
        assert!(long_markup.is_none());
    }
}
