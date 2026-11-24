use super::*;

impl AgentRuntime {
    // ─── Local Unix Domain Socket JSON-RPC Control Plane (Owner-Only 0o600) ────

    #[cfg(unix)]
    pub async fn run_ipc_server(self: Arc<Self>, cancel: CancellationToken) {
        use std::os::unix::fs::PermissionsExt;

        let sock_path = self.config().socket_path();
        if sock_path.exists() {
            let _ = std::fs::remove_file(&sock_path);
        }
        let listener = match tokio::net::UnixListener::bind(&sock_path) {
            Ok(l) => l,
            Err(e) => {
                error!(path = %sock_path.display(), error = %e, "failed to bind unix IPC socket");
                return;
            }
        };
        let _ = std::fs::set_permissions(
            &sock_path,
            std::fs::Permissions::from_mode(FILE_MODE_OWNER_ONLY),
        );
        info!(path = %sock_path.display(), "daemon unix IPC control plane ready (0600)");

        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue; };
                    let rt = self.clone();
                    let conn_cancel = cancel.clone();
                    tokio::spawn(async move {
                        const MAX_IPC_LINE_BYTES: usize = 1024 * 1024;
                        let (reader, mut writer) = stream.into_split();
                        let mut buf_reader = BufReader::new(reader);
                        let mut raw_buf = Vec::new();
                        loop {
                            raw_buf.clear();
                            let mut limited = tokio::io::AsyncReadExt::take(
                                &mut buf_reader,
                                (MAX_IPC_LINE_BYTES + 1) as u64,
                            );
                            let read_res = tokio::select! {
                                biased;
                                _ = conn_cancel.cancelled() => break,
                                res = limited.read_until(b'\n', &mut raw_buf) => res,
                            };
                            let n = match read_res {
                                Ok(0) | Err(_) => break,
                                Ok(n) => n,
                            };
                            if n > MAX_IPC_LINE_BYTES {
                                let err_resp = json!({ "ok": false, "error": "IPC request line exceeds 1 MiB limit" });
                                let mut out = err_resp.to_string();
                                out.push('\n');
                                let _ = writer.write_all(out.as_bytes()).await;
                                let _ = writer.flush().await;
                                break;
                            }
                            let line = String::from_utf8_lossy(&raw_buf);
                            let line = line.trim_end_matches(['\r', '\n']);
                            if let Ok(req) = serde_json::from_str::<Value>(line) {
                                match req.get("method").and_then(Value::as_str) {
                                    Some(IPC_METHOD_LOGS_FOLLOW) => {
                                        let filter: LogFilter = req
                                            .get("params")
                                            .cloned()
                                            .and_then(|v| serde_json::from_value(v).ok())
                                            .unwrap_or_default();
                                        rt.stream_ipc_logs(
                                            &filter,
                                            &mut writer,
                                            conn_cancel.clone(),
                                        )
                                        .await;
                                        break;
                                    }
                                    Some(IPC_METHOD_SESSION_ATTACH) => {
                                        let sid = req
                                            .get("params")
                                            .and_then(|p| p.get("session_id"))
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_string();
                                        let replay = req
                                            .get("params")
                                            .and_then(|p| p.get("replay"))
                                            .and_then(Value::as_u64)
                                            .unwrap_or(ATTACH_DEFAULT_REPLAY_EVENTS as u64)
                                            as usize;
                                        rt.stream_ipc_session(
                                            &sid,
                                            replay,
                                            &mut writer,
                                            conn_cancel.clone(),
                                        )
                                        .await;
                                        break;
                                    }
                                    Some(IPC_METHOD_JOBS_FOLLOW) => {
                                        let job_id = req
                                            .get("params")
                                            .and_then(|p| p.get("job_id"))
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_string();
                                        let offset = req
                                            .get("params")
                                            .and_then(|p| p.get("offset"))
                                            .and_then(Value::as_u64)
                                            .unwrap_or(0);
                                        rt.stream_ipc_job_follow(
                                            &job_id,
                                            offset,
                                            &mut writer,
                                            conn_cancel.clone(),
                                        )
                                        .await;
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            let resp = rt.handle_ipc_line(line).await;
                            let mut payload = serde_json::to_string(&resp).unwrap_or_default();
                            payload.push('\n');
                            if writer.write_all(payload.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    });
                }
            }
        }
        let _ = std::fs::remove_file(&sock_path);
    }

    #[cfg(not(unix))]
    pub async fn run_ipc_server(self: Arc<Self>, cancel: CancellationToken) {
        cancel.cancelled().await;
    }

    async fn handle_ipc_line(self: &Arc<Self>, line: &str) -> Value {
        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => return json!({"ok": false, "error": format!("invalid json: {e}")}),
        };
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let params = req.get("params");

        match method {
            IPC_METHOD_STATUS => json!({"ok": true, "result": self.telemetry()}),
            IPC_METHOD_PAIR => {
                let channel = params
                    .and_then(|p| p.get("channel"))
                    .and_then(Value::as_str)
                    .unwrap_or("telegram");
                let role = params
                    .and_then(|p| p.get("role"))
                    .and_then(Value::as_str)
                    .map(PeerRole::from_str_loose)
                    .unwrap_or(PeerRole::Owner);
                match self
                    .store
                    .create_pairing_code(channel, role, DEFAULT_PAIRING_TTL_MINS)
                {
                    Ok(code) => json!({"ok": true, "result": code}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_ASK => {
                let prompt = params
                    .and_then(|p| p.get("prompt"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let session_id = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str);
                let mode_override = params
                    .and_then(|p| p.get("mode"))
                    .and_then(Value::as_str)
                    .map(PermissionMode::from_str_loose);
                let model_override = params
                    .and_then(|p| p.get("model"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let workspace_override = params
                    .and_then(|p| p.get("workspace"))
                    .and_then(Value::as_str)
                    .map(PathBuf::from);
                let headless_no_wait = params
                    .and_then(|p| p.get("headless_no_wait"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);

                let sid = match session_id {
                    Some(id) => id.to_string(),
                    None => match self.store.create_session(
                        &make_session_title(prompt),
                        "cli",
                        None,
                        None,
                        mode_override.unwrap_or_else(|| self.config().policy.mode),
                    ) {
                        Ok(s) => s.id,
                        Err(e) => return json!({"ok": false, "error": e.to_string()}),
                    },
                };
                match self
                    .run_turn_with_options(
                        &sid,
                        prompt,
                        TurnOptions {
                            parent_event_override: None,
                            workspace_override,
                            model_override,
                            permission_override: mode_override,
                            headless_no_wait,
                            send_mode: params
                                .and_then(|p| p.get("send_mode"))
                                .and_then(Value::as_str)
                                .map(SendMode::from_str_loose)
                                .unwrap_or_default(),
                            ..TurnOptions::default()
                        },
                    )
                    .await
                {
                    Ok(turn) => json!({"ok": true, "result": turn}),
                    Err(e) => json!({"ok": false, "error": e.to_string(), "session_id": sid}),
                }
            }
            IPC_METHOD_STOP => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let cancelled = self.cancel_session(sid);
                json!({"ok": true, "result": {"cancelled": cancelled}})
            }
            IPC_METHOD_SESSIONS_LIST => {
                let limit = params
                    .and_then(|p| p.get("limit"))
                    .and_then(Value::as_u64)
                    .unwrap_or(JOB_DEFAULT_LIST_LIMIT as u64) as usize;
                match self.store.list_sessions(limit) {
                    Ok(list) => json!({"ok": true, "result": list}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SESSION_GET => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.store.get_session(sid) {
                    Ok(Some(session)) => {
                        let events = self.store.get_active_branch(sid).unwrap_or_default();
                        json!({"ok": true, "result": {"session": session, "events": events}})
                    }
                    Ok(None) => json!({"ok": false, "error": format!("session {sid} not found")}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SESSION_CLEAR => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let _ = self.cancel_session(sid);
                self.clear_session_deferred_tools(sid);
                self.notify_session_ended(sid).await;
                match self.store.clear_session_events(sid) {
                    Ok(()) => json!({"ok": true, "result": {"cleared": true}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SESSION_DELETE => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let _ = self.cancel_session(sid);
                self.clear_session_deferred_tools(sid);
                self.notify_session_ended(sid).await;
                match self.store.delete_session(sid) {
                    Ok(deleted) => json!({"ok": true, "result": {"deleted": deleted}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SESSION_CONTEXT => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.inspect_session_context(sid).await {
                    Ok(ctx) => json!({"ok": true, "result": ctx}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_APPROVALS_LIST => match self.store.list_pending_approvals() {
                Ok(list) => json!({"ok": true, "result": list}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            },
            IPC_METHOD_APPROVAL_RESOLVE => {
                let id = params
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let decision = match params
                    .and_then(|p| p.get("decision"))
                    .and_then(Value::as_str)
                    .unwrap_or("deny")
                {
                    "approve" | "allow" => ApprovalDecision::Approve,
                    "always" | "always_allow" => ApprovalDecision::AlwaysAllow,
                    _ => ApprovalDecision::Deny,
                };
                match self.resolve_approval(id, decision, "cli") {
                    Ok(rec) => json!({"ok": true, "result": {"resolved": rec.is_some(), "approval": rec}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MEMORY_SEARCH => {
                let q = params
                    .and_then(|p| p.get("query"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let limit = params
                    .and_then(|p| p.get("limit"))
                    .and_then(Value::as_u64)
                    .unwrap_or(20) as usize;
                let ws = params
                    .and_then(|p| p.get("workspace"))
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.config().resolved_workspace_dir());
                let proj_scope = project_scope_for_workspace(&ws);
                let (scope_opt, include_global) = match params
                    .and_then(|p| p.get("scope"))
                    .and_then(Value::as_str)
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
                match self
                    .store
                    .search_memory_hybrid(q, scope_opt, include_global, Some(&ws), limit)
                {
                    Ok(hits) => {
                        let entries: Vec<_> = hits.into_iter().map(|h| h.entry).collect();
                        json!({"ok": true, "result": entries})
                    }
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MEMORY_ADD => {
                let title = params
                    .and_then(|p| p.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or("Note");
                let content = params
                    .and_then(|p| p.get("content"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let kind = params
                    .and_then(|p| p.get("kind"))
                    .and_then(Value::as_str)
                    .unwrap_or("fact");
                let ws = params
                    .and_then(|p| p.get("workspace"))
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.config().resolved_workspace_dir());
                let scope = match params
                    .and_then(|p| p.get("scope"))
                    .and_then(Value::as_str)
                    .unwrap_or(MEMORY_SCOPE_GLOBAL)
                    .trim()
                {
                    "project" => project_scope_for_workspace(&ws),
                    "" => MEMORY_SCOPE_GLOBAL.to_string(),
                    other => other.to_string(),
                };
                match self
                    .store
                    .upsert_memory_dedup(&scope, kind, title, content, 1.0, None, None, true)
                {
                    Ok(res) => json!({"ok": true, "result": res.entry, "merged": res.merged}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MEMORY_UPDATE => {
                let id = params
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.store.update_memory(
                    id,
                    params.and_then(|p| p.get("title")).and_then(Value::as_str),
                    params.and_then(|p| p.get("content")).and_then(Value::as_str),
                    params.and_then(|p| p.get("kind")).and_then(Value::as_str),
                    params.and_then(|p| p.get("salience")).and_then(Value::as_f64),
                    params.and_then(|p| p.get("scope")).and_then(Value::as_str),
                ) {
                    Ok(Some(entry)) => json!({"ok": true, "result": entry}),
                    Ok(None) => json!({"ok": false, "error": format!("memory `{id}` not found")}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MEMORY_DELETE => {
                let id = params
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.store.delete_memory(id) {
                    Ok(deleted) => json!({"ok": true, "result": {"deleted": deleted}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MEMORY_DREAM => {
                let force = params
                    .and_then(|p| p.get("force"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let trigger = params
                    .and_then(|p| p.get("trigger"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli");
                match self.run_dream_now(trigger, force).await {
                    Ok(rec) => json!({"ok": true, "result": rec}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MEMORY_HISTORY => {
                let doc = params.and_then(|p| p.get("doc")).and_then(Value::as_str);
                match self.snapshots.list_doc_history(doc) {
                    Ok(list) => json!({"ok": true, "result": list}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MEMORY_ROLLBACK => {
                let sid = params
                    .and_then(|p| p.get("snapshot_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.snapshots.rollback_doc(sid) {
                    Ok((doc, diff)) => json!({"ok": true, "result": {"doc": doc, "diff": diff}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SESSIONS_SEARCH => {
                let query = params
                    .and_then(|p| p.get("query"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let since = params
                    .and_then(|p| p.get("since"))
                    .and_then(Value::as_str)
                    .and_then(parse_since_window);
                let channel = params
                    .and_then(|p| p.get("channel"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let role = params
                    .and_then(|p| p.get("role"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let session_id = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let limit = params
                    .and_then(|p| p.get("limit"))
                    .and_then(Value::as_u64)
                    .map(|v| v as usize);
                let workspace_dir = match params
                    .and_then(|p| p.get("workspace"))
                    .and_then(Value::as_str)
                    .unwrap_or("all")
                    .trim()
                {
                    "" | "all" => None,
                    "current" => Some(
                        resolve_git_root(&self.config().resolved_workspace_dir())
                            .display()
                            .to_string(),
                    ),
                    other => Some(resolve_git_root(&PathBuf::from(other)).display().to_string()),
                };
                let filter = SessionSearchFilter {
                    query,
                    since,
                    channel,
                    workspace_dir,
                    session_id,
                    exclude_session_id: None,
                    role,
                    limit,
                };
                match self.store.search_sessions_fts(&filter) {
                    Ok(hits) => json!({"ok": true, "result": hits}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_CRON_LIST => match self.store.list_cron_jobs() {
                Ok(jobs) => json!({"ok": true, "result": jobs}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            },
            IPC_METHOD_CRON_CREATE => {
                let name = params
                    .and_then(|p| p.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("Scheduled Task");
                let expr = params
                    .and_then(|p| p.get("cron_expr"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let prompt = params
                    .and_then(|p| p.get("prompt"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let script_path = params
                    .and_then(|p| p.get("script_path"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                let kind = CronKind::from_str_loose(
                    params
                        .and_then(|p| p.get("kind"))
                        .and_then(Value::as_str)
                        .unwrap_or("cron"),
                );
                let target_channel = params
                    .and_then(|p| p.get("target_channel"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli");
                let parsed = match Cron::new(expr).parse() {
                    Ok(c) => c,
                    Err(e) => return json!({"ok": false, "error": format!("invalid cron expression: {e}")}),
                };
                let job = CronJob {
                    id: new_id(),
                    name: name.to_string(),
                    cron_expr: expr.to_string(),
                    kind,
                    prompt: prompt.to_string(),
                    script_path,
                    target_channel: target_channel.to_string(),
                    target_chat_id: None,
                    target_thread_id: None,
                    enabled: true,
                    next_run_at: parsed.find_next_occurrence(&Utc::now(), false).ok(),
                    last_run_at: None,
                    last_status: None,
                };
                match self.store.upsert_cron_job(&job) {
                    Ok(()) => {
                        self.notify_cron_updated();
                        json!({"ok": true, "result": job})
                    }
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_CRON_UPDATE => {
                let id = params
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let mut job = match self.store.get_cron_job(id) {
                    Ok(Some(j)) => j,
                    Ok(None) => return json!({"ok": false, "error": format!("cron job `{id}` not found")}),
                    Err(e) => return json!({"ok": false, "error": e.to_string()}),
                };
                if let Some(n) = params.and_then(|p| p.get("name")).and_then(Value::as_str)
                    && !n.trim().is_empty() {
                        job.name = n.trim().to_string();
                    }
                if let Some(expr) = params.and_then(|p| p.get("cron_expr")).and_then(Value::as_str) {
                    let parsed = match Cron::new(expr).parse() {
                        Ok(c) => c,
                        Err(e) => return json!({"ok": false, "error": format!("invalid cron expression: {e}")}),
                    };
                    job.cron_expr = expr.to_string();
                    job.next_run_at = parsed.find_next_occurrence(&Utc::now(), false).ok();
                }
                if let Some(k) = params.and_then(|p| p.get("kind")).and_then(Value::as_str) {
                    job.kind = CronKind::from_str_loose(k);
                }
                if let Some(pr) = params.and_then(|p| p.get("prompt")).and_then(Value::as_str) {
                    job.prompt = pr.to_string();
                }
                if let Some(sp) = params.and_then(|p| p.get("script_path")).and_then(Value::as_str) {
                    let trimmed = sp.trim();
                    job.script_path = match trimmed.is_empty() {
                        true => None,
                        false => Some(trimmed.to_string()),
                    };
                }
                if let Some(en) = params.and_then(|p| p.get("enabled")).and_then(Value::as_bool) {
                    job.enabled = en;
                }
                match self.store.upsert_cron_job(&job) {
                    Ok(()) => {
                        self.notify_cron_updated();
                        json!({"ok": true, "result": job})
                    }
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_CRON_RUN_NOW => {
                let id = params
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.trigger_cron_job_now(id).await {
                    Ok(res) => json!({"ok": true, "result": res}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_CRON_DELETE => {
                let id = params
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.store.delete_cron_job(id) {
                    Ok(deleted) => {
                        if deleted {
                            self.notify_cron_updated();
                        }
                        json!({"ok": true, "result": {"deleted": deleted}})
                    }
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_LOGS_RECENT | IPC_METHOD_LOGS_QUERY => {
                let mut filter: LogFilter = params
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                if filter.limit.is_none() {
                    filter.limit = Some(60);
                }
                let items = match self.store.query_logs(&filter) {
                    Ok(rows) if !rows.is_empty() => rows,
                    Ok(_) => self
                        .logs
                        .recent(filter.limit.unwrap_or(60))
                        .into_iter()
                        .filter(|e| filter.matches(e))
                        .collect(),
                    Err(_) => self
                        .logs
                        .recent(filter.limit.unwrap_or(60))
                        .into_iter()
                        .filter(|e| filter.matches(e))
                        .collect(),
                };
                json!({"ok": true, "result": items})
            }
            IPC_METHOD_LOGS_STATS => {
                let counts = self.store.log_category_counts().unwrap_or_default();
                json!({
                    "ok": true,
                    "result": {
                        "logs_dir": self.log_sink.logs_dir().display().to_string(),
                        "category_counts": counts,
                        "dropped_entries": self.log_sink.dropped_entries(),
                        "wire_debug_enabled": wire_debug_enabled(),
                        "categories": LOG_CATEGORIES,
                    }
                })
            }
            IPC_METHOD_JOBS_LIST => {
                let session_id = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty());
                let kind = params
                    .and_then(|p| p.get("kind"))
                    .and_then(Value::as_str)
                    .and_then(TaskKind::parse_filter);
                let limit = params
                    .and_then(|p| p.get("limit"))
                    .and_then(Value::as_u64)
                    .unwrap_or(JOB_DEFAULT_LIST_LIMIT as u64) as usize;
                match self.jobs.list_jobs(session_id, kind, limit) {
                    Ok(list) => json!({"ok": true, "result": list}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_START => {
                let command = params
                    .and_then(|p| p.get("command"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if command.trim().is_empty() {
                    return json!({"ok": false, "error": "missing `command`"});
                }
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("cli");
                let kind = params
                    .and_then(|p| p.get("kind"))
                    .and_then(Value::as_str)
                    .and_then(TaskKind::parse_filter)
                    .unwrap_or_else(|| TaskKind::infer_from_command(command));
                let label = params
                    .and_then(|p| p.get("label"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                let ws = params
                    .and_then(|p| p.get("workspace"))
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.config().resolved_workspace_dir());
                match self
                    .jobs
                    .spawn_shell(sid, command, &ws, kind, label, false)
                    .await
                {
                    Ok(rec) => json!({"ok": true, "result": rec}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_UPDATE => {
                let job_id = params
                    .and_then(|p| p.get("job_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let label = params
                    .and_then(|p| p.get("label"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let kind = params
                    .and_then(|p| p.get("kind"))
                    .and_then(Value::as_str)
                    .and_then(TaskKind::parse_filter);
                match self.jobs.update_job_meta(job_id, label, kind) {
                    Ok(Some(rec)) => json!({"ok": true, "result": rec}),
                    Ok(None) => json!({"ok": false, "error": format!("task `{job_id}` not found")}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_RESTART => {
                let job_id = params
                    .and_then(|p| p.get("job_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.jobs.restart_job(job_id).await {
                    Ok(rec) => json!({"ok": true, "result": rec}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_DELETE => {
                let job_id = params
                    .and_then(|p| p.get("job_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.jobs.delete_job(job_id).await {
                    Ok(deleted) => json!({"ok": true, "result": {"deleted": deleted, "job_id": job_id}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_PRUNE => {
                let kind = params
                    .and_then(|p| p.get("kind"))
                    .and_then(Value::as_str)
                    .and_then(TaskKind::parse_filter);
                match self.jobs.prune_jobs(kind).await {
                    Ok(pruned) => json!({"ok": true, "result": {"pruned": pruned}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_HUB_LIST => {
                let cat = params.and_then(|p| p.get("category")).and_then(Value::as_str);
                let limit = params
                    .and_then(|p| p.get("limit"))
                    .and_then(Value::as_u64)
                    .unwrap_or(100) as usize;
                match self.list_hub_entries(cat, limit) {
                    Ok(entries) => json!({
                        "ok": true,
                        "result": {
                            "hub_dir": self.config().resolved_hub_dir().display().to_string(),
                            "categories": HUB_CATEGORIES,
                            "entries": entries,
                        }
                    }),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_READ => {
                let job_id = params
                    .and_then(|p| p.get("job_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let offset = params.and_then(|p| p.get("offset")).and_then(Value::as_u64);
                let max_bytes = params
                    .and_then(|p| p.get("max_bytes"))
                    .and_then(Value::as_u64)
                    .unwrap_or(JOB_DEFAULT_READ_BYTES as u64) as usize;
                match self.jobs.read_job_output(job_id, offset, max_bytes).await {
                    Ok(chunk) => json!({"ok": true, "result": chunk}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_INPUT => {
                let job_id = params
                    .and_then(|p| p.get("job_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let input = params
                    .and_then(|p| p.get("input"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.jobs.send_input(job_id, input).await {
                    Ok(()) => json!({"ok": true, "result": {"sent": true}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_STOP => {
                let job_id = params
                    .and_then(|p| p.get("job_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.jobs.stop_job(job_id).await {
                    Ok(rec) => json!({"ok": true, "result": {"stopped": rec.is_some(), "job": rec}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_JOBS_WAIT => {
                let job_id = params
                    .and_then(|p| p.get("job_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let timeout_secs = params
                    .and_then(|p| p.get("timeout_secs"))
                    .and_then(Value::as_u64)
                    .unwrap_or(JOB_DEFAULT_WAIT_SECS)
                    .clamp(1, JOB_MAX_WAIT_SECS);
                let tail_bytes = params
                    .and_then(|p| p.get("tail_bytes"))
                    .and_then(Value::as_u64)
                    .unwrap_or(JOB_DEFAULT_READ_BYTES as u64) as usize;
                match self
                    .jobs
                    .wait_job(job_id, Duration::from_secs(timeout_secs))
                    .await
                {
                    Ok(Some(rec)) => {
                        let chunk = self
                            .jobs
                            .read_job_output(&rec.id, None, tail_bytes)
                            .await
                            .ok();
                        json!({"ok": true, "result": {"job": rec, "output": chunk}})
                    }
                    Ok(None) => json!({"ok": false, "error": format!("job `{job_id}` not found")}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SESSION_STEER => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let text = params
                    .and_then(|p| p.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let steered = self.steer_session(sid, text);
                json!({"ok": true, "result": {"steered": steered, "session_id": sid}})
            }
            IPC_METHOD_SESSION_BACKGROUND => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let demoted = self.demote_foreground_job(sid);
                json!({"ok": true, "result": {"demoted": demoted, "session_id": sid}})
            }
            IPC_METHOD_SESSION_QUEUE_LIST => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.store.list_queued_followups(sid) {
                    Ok(list) => json!({"ok": true, "result": list}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SESSION_QUEUE_CANCEL => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let queue_id = params
                    .and_then(|p| p.get("queue_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.cancel_session_followup(sid, queue_id) {
                    Ok((cancelled, count)) => {
                        json!({"ok": true, "result": {"cancelled": cancelled, "queued_count": count}})
                    }
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_CLARIFICATIONS_LIST => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty());
                let list = self.clarifications.list_pending_clarifications(sid);
                json!({"ok": true, "result": list})
            }
            IPC_METHOD_CLARIFICATION_RESOLVE => {
                let id = params
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let selected = params
                    .and_then(|p| p.get("selected_option_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let custom = params
                    .and_then(|p| p.get("custom_answer"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                let resolved_by = params
                    .and_then(|p| p.get("resolved_by"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli");
                let rec = self
                    .clarifications
                    .resolve_clarification(id, selected, custom, resolved_by);
                json!({"ok": true, "result": {"resolved": rec.is_some(), "clarification": rec}})
            }
            IPC_METHOD_ORCHESTRATOR_TRIAGE => {
                let prompt = params
                    .and_then(|p| p.get("prompt"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let mode = params
                    .and_then(|p| p.get("mode"))
                    .and_then(Value::as_str)
                    .map(PermissionMode::from_str_loose);
                let report = self.triage_prompt(prompt, mode);
                json!({"ok": true, "result": report})
            }
            IPC_METHOD_AGENTS_LIST => {
                let parent_session_id = params
                    .and_then(|p| p.get("parent_session_id"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty());
                let limit = params
                    .and_then(|p| p.get("limit"))
                    .and_then(Value::as_u64)
                    .unwrap_or(SUBAGENT_LIST_DEFAULT_LIMIT as u64) as usize;
                match self.store.list_subagents(parent_session_id, limit) {
                    Ok(list) => json!({"ok": true, "result": list}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_GET => {
                let id = params
                    .and_then(|p| p.get("agent_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.store.get_subagent(id) {
                    Ok(Some(rec)) => {
                        let messages = self.store.list_agent_messages(id, 40).unwrap_or_default();
                        json!({"ok": true, "result": {"agent": rec, "messages": messages}})
                    }
                    Ok(None) => json!({"ok": false, "error": format!("subagent `{id}` not found")}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_SPAWN => {
                let parent_session_id = params
                    .and_then(|p| p.get("parent_session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli")
                    .to_string();
                let role = params
                    .and_then(|p| p.get("role"))
                    .and_then(Value::as_str)
                    .unwrap_or("explorer")
                    .to_string();
                let goal = params
                    .and_then(|p| p.get("goal"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let context = params
                    .and_then(|p| p.get("context"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let run_in_background = params
                    .and_then(|p| p.get("run_in_background"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let workspace_mode = params
                    .and_then(|p| p.get("workspace"))
                    .and_then(Value::as_str)
                    .map(SubagentWorkspaceMode::from_str_loose);
                let output_schema = params
                    .and_then(|p| p.get("output_schema"))
                    .filter(|v| v.is_object())
                    .cloned();
                let model_override = params
                    .and_then(|p| p.get("model"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let fork_context = params
                    .and_then(|p| p.get("fork_context"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let include_uncommitted = params
                    .and_then(|p| p.get("include_uncommitted"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let max_duration_secs = params
                    .and_then(|p| p.get("max_duration_secs"))
                    .and_then(Value::as_u64);
                let max_cost_usd = params
                    .and_then(|p| p.get("max_cost_usd"))
                    .and_then(Value::as_f64);
                let custom_directive = params
                    .and_then(|p| p.get("custom_directive"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                let cognitive_tier = params
                    .and_then(|p| p.get("cognitive_tier"))
                    .and_then(Value::as_str)
                    .and_then(ModelRoutingRole::parse);
                let read_only = params.and_then(|p| p.get("read_only")).and_then(Value::as_bool);
                let tool_subset = params
                    .and_then(|p| p.get("tool_subset"))
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                match self
                    .subagents
                    .spawn_agent(
                        self,
                        SubagentSpawnRequest {
                            parent_session_id,
                            role,
                            goal,
                            context,
                            run_in_background,
                            workspace_mode,
                            output_schema,
                            model_override,
                            fork_context,
                            include_uncommitted,
                            max_duration_secs,
                            max_cost_usd,
                            custom_directive,
                            cognitive_tier,
                            read_only,
                            tool_subset,
                        },
                    )
                    .await
                {
                    Ok(rec) => json!({"ok": true, "result": rec}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_MESSAGE => {
                let from_id = params
                    .and_then(|p| p.get("from_session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli");
                let to_id = params
                    .and_then(|p| p.get("agent_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let message = params
                    .and_then(|p| p.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let resume = params
                    .and_then(|p| p.get("resume"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let interrupt = params
                    .and_then(|p| p.get("interrupt"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                match self
                    .subagents
                    .send_message(
                        self,
                        from_id,
                        to_id,
                        message,
                        zenko_core::AgentMessageOptions { resume, interrupt },
                    )
                    .await
                {
                    Ok(mail) => json!({"ok": true, "result": mail}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_WAIT => {
                let parent_id = params
                    .and_then(|p| p.get("parent_session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let agent_ids: Vec<String> = params
                    .and_then(|p| p.get("agent_ids"))
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                let wait_all = params
                    .and_then(|p| p.get("wait_all"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let timeout_secs = params
                    .and_then(|p| p.get("timeout_secs"))
                    .and_then(Value::as_u64)
                    .unwrap_or(SUBAGENT_DEFAULT_WAIT_SECS)
                    .clamp(1, SUBAGENT_MAX_WAIT_SECS);
                match self
                    .subagents
                    .wait_agents(
                        self,
                        parent_id,
                        &agent_ids,
                        wait_all,
                        Duration::from_secs(timeout_secs),
                    )
                    .await
                {
                    Ok(list) => json!({"ok": true, "result": list}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_KILL => {
                let caller = params
                    .and_then(|p| p.get("caller_session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli");
                let agent_id = params
                    .and_then(|p| p.get("agent_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let remove_wt = params
                    .and_then(|p| p.get("remove_worktree"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                match self
                    .subagents
                    .kill_agent(self, caller, agent_id, remove_wt)
                    .await
                {
                    Ok(rec) => json!({"ok": true, "result": {"killed": rec.is_some(), "agent": rec}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_DIFF => {
                let caller = params
                    .and_then(|p| p.get("caller_session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli");
                let agent_id = params
                    .and_then(|p| p.get("agent_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self
                    .subagents
                    .inspect_worktree_diff(self, caller, agent_id)
                    .await
                {
                    Ok(Some(rep)) => json!({"ok": true, "result": rep}),
                    Ok(None) => json!({"ok": false, "error": format!("no git worktree for subagent `{agent_id}`")}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_MERGE => {
                let caller = params
                    .and_then(|p| p.get("caller_session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli");
                let agent_id = params
                    .and_then(|p| p.get("agent_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let cleanup = params
                    .and_then(|p| p.get("cleanup"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                match self
                    .subagents
                    .merge_worktree(self, caller, agent_id, cleanup)
                    .await
                {
                    Ok(rep) => json!({"ok": true, "result": rep}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_DISCARD => {
                let caller = params
                    .and_then(|p| p.get("caller_session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("cli");
                let agent_id = params
                    .and_then(|p| p.get("agent_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.subagents.discard_worktree(self, caller, agent_id).await {
                    Ok(rep) => json!({"ok": true, "result": rep}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_AGENTS_PRUNE => match self.subagents.prune_orphan_worktrees(self).await {
                Ok(rep) => json!({"ok": true, "result": rep}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            },
            IPC_METHOD_TASKS_DAG_LIST => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let ready_only = params
                    .and_then(|p| p.get("ready_only"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let res = match ready_only {
                    true => self.clarifications.ready_session_tasks(sid),
                    false => self.clarifications.list_session_tasks(sid),
                };
                match res {
                    Ok(tasks) => json!({"ok": true, "result": tasks}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_TASKS_DAG_CREATE => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let title = params
                    .and_then(|p| p.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let id = params
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .unwrap_or_else(new_session_task_id);
                let description = params
                    .and_then(|p| p.get("description"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let depends_on: Vec<String> = params
                    .and_then(|p| p.get("depends_on"))
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                let verification_cmd = params
                    .and_then(|p| p.get("verification_cmd"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let priority = params
                    .and_then(|p| p.get("priority"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0) as i32;
                let now = Utc::now();
                let task = SessionTask {
                    id,
                    session_id: sid.to_string(),
                    branch_event_id: None,
                    title,
                    description,
                    active_label: None,
                    status: SessionTaskStatus::Pending,
                    depends_on,
                    owner_agent: None,
                    verification_cmd,
                    verification_evidence: None,
                    notes: None,
                    priority,
                    created_at: now,
                    updated_at: now,
                };
                match self.clarifications.create_session_task(sid, task) {
                    Ok(saved) => json!({"ok": true, "result": saved}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_TASKS_DAG_UPDATE => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let tid = params
                    .and_then(|p| p.get("task_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let claim_owner = params
                    .and_then(|p| p.get("claim_owner"))
                    .and_then(Value::as_str);
                let res = match claim_owner {
                    Some(owner) => self.clarifications.claim_session_task(sid, tid, owner),
                    None => {
                        let parse_opt_str = |key: &str| {
                            params.and_then(|p| p.get(key)).map(|v| {
                                v.as_str()
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                    .map(String::from)
                            })
                        };
                        let parse_str_arr = |key: &str| -> Option<Vec<String>> {
                            params
                                .and_then(|p| p.get(key))
                                .and_then(Value::as_array)
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(Value::as_str)
                                        .map(str::trim)
                                        .filter(|s| !s.is_empty())
                                        .map(String::from)
                                        .collect()
                                })
                        };
                        let patch = SessionTaskPatch {
                            title: params
                                .and_then(|p| p.get("title"))
                                .and_then(Value::as_str)
                                .map(String::from),
                            description: params
                                .and_then(|p| p.get("description"))
                                .and_then(Value::as_str)
                                .map(String::from),
                            active_label: parse_opt_str("active_label"),
                            status: params
                                .and_then(|p| p.get("status"))
                                .and_then(Value::as_str)
                                .map(SessionTaskStatus::from_str_loose),
                            depends_on: parse_str_arr("depends_on"),
                            add_depends_on: parse_str_arr("add_depends_on").unwrap_or_default(),
                            owner_agent: parse_opt_str("owner_agent"),
                            verification_cmd: parse_opt_str("verification_cmd"),
                            verification_evidence: parse_opt_str("verification_evidence"),
                            notes: parse_opt_str("notes"),
                            priority: params
                                .and_then(|p| p.get("priority"))
                                .and_then(Value::as_i64)
                                .map(|v| v as i32),
                        };
                        self.clarifications.update_session_task(sid, tid, patch)
                    }
                };
                match res {
                    Ok(updated) => json!({"ok": true, "result": updated}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_TASKS_DAG_DELETE => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let clear_all = params
                    .and_then(|p| p.get("clear_all"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                match clear_all {
                    true => match self.clarifications.clear_session_tasks(sid) {
                        Ok(n) => json!({"ok": true, "result": {"cleared": n}}),
                        Err(e) => json!({"ok": false, "error": e.to_string()}),
                    },
                    false => {
                        let tid = params
                            .and_then(|p| p.get("task_id"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        match self.clarifications.delete_session_task(sid, tid) {
                            Ok(deleted) => json!({"ok": true, "result": {"deleted": deleted}}),
                            Err(e) => json!({"ok": false, "error": e.to_string()}),
                        }
                    }
                }
            }
            IPC_METHOD_CHECKPOINTS_LIST => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty());
                let limit = params
                    .and_then(|p| p.get("limit"))
                    .and_then(Value::as_u64)
                    .unwrap_or(CHECKPOINT_LIST_DEFAULT_LIMIT as u64)
                    as usize;
                match self.store.list_checkpoints(sid, limit) {
                    Ok(list) => json!({"ok": true, "result": list}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_CHECKPOINTS_DIFF => {
                let cp_id = params
                    .and_then(|p| p.get("checkpoint_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let cp_cfg = self.config().checkpoints;
                match self.checkpoints.diff_checkpoint(&cp_cfg, cp_id).await {
                    Ok(rep) => json!({"ok": true, "result": rep}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_CHECKPOINTS_REWIND => {
                let cp_id = params
                    .and_then(|p| p.get("checkpoint_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if let Ok(Some(existing)) = self.store.get_checkpoint(&cp_id)
                    && self.is_session_running(&existing.session_id) {
                        return json!({
                            "ok": false,
                            "error": "Cannot rewind while a turn is actively running on this session. Stop the turn first."
                        });
                    }
                let mode = params
                    .and_then(|p| p.get("mode"))
                    .and_then(Value::as_str)
                    .map(RewindMode::from_str_loose)
                    .unwrap_or(RewindMode::CodeAndConversation);
                let files: Vec<String> = params
                    .and_then(|p| p.get("files"))
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                let overwrite_manual_edits = params
                    .and_then(|p| p.get("overwrite_manual_edits"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let summarize_abandoned = params
                    .and_then(|p| p.get("summarize_abandoned"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let cp_cfg = self.config().checkpoints;
                match self
                    .checkpoints
                    .rewind(
                        &cp_cfg,
                        &RewindRequest {
                            checkpoint_id: cp_id,
                            mode,
                            files,
                            overwrite_manual_edits,
                            summarize_abandoned,
                        },
                    )
                    .await
                {
                    Ok(rep) => json!({"ok": true, "result": rep}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_CHECKPOINTS_PRUNE => {
                let cp_cfg = self.config().checkpoints;
                match self.checkpoints.prune_checkpoints(&cp_cfg).await {
                    Ok(rep) => json!({"ok": true, "result": rep}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SESSION_APPROVE_PLAN => {
                let sid = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let target_mode = params
                    .and_then(|p| p.get("mode"))
                    .and_then(Value::as_str)
                    .map(PermissionMode::from_str_loose)
                    .unwrap_or(PermissionMode::Autonomous);
                match self.approve_session_plan(sid, target_mode) {
                    Ok((sess, task_count)) => json!({
                        "ok": true,
                        "result": {
                            "session": sess,
                            "task_count": task_count,
                        }
                    }),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MCP_SERVERS_LIST => {
                let servers = self.mcp_clients.list_status().await;
                let configs = self.mcp_clients.configs();
                json!({
                    "ok": true,
                    "result": {
                        "servers": servers,
                        "configs": configs,
                    }
                })
            }
            IPC_METHOD_MCP_SERVER_UPSERT => {
                let sync_now = params
                    .and_then(|p| p.get("sync"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let srv_val = params
                    .and_then(|p| p.get("server"))
                    .cloned()
                    .or(params.cloned())
                    .unwrap_or(Value::Null);
                match serde_json::from_value::<McpServerConfig>(srv_val) {
                    Ok(srv) => match self.upsert_mcp_server(srv, sync_now).await {
                        Ok((servers, sync_error)) => json!({
                            "ok": true,
                            "result": {
                                "servers": servers,
                                "sync_error": sync_error,
                            }
                        }),
                        Err(e) => json!({"ok": false, "error": e.to_string()}),
                    },
                    Err(e) => json!({"ok": false, "error": format!("invalid MCP server config: {e}")}),
                }
            }
            IPC_METHOD_MCP_SERVER_DELETE => {
                let name = params
                    .and_then(|p| p.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.delete_mcp_server(name).await {
                    Ok(deleted) => json!({"ok": true, "result": {"deleted": deleted}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_MCP_SERVER_SYNC => {
                let name = params
                    .and_then(|p| p.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match self.sync_mcp_server(name).await {
                    Ok(status) => json!({"ok": true, "result": status}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_LSP_STATUS => {
                let active = self.lsp.list_status().await;
                let cfg = self.config().lsp;
                json!({
                    "ok": true,
                    "result": {
                        "enabled": cfg.enabled,
                        "idle_timeout_secs": cfg.idle_timeout_secs,
                        "active": active,
                    }
                })
            }
            IPC_METHOD_LSP_STOP => {
                let stopped = self.lsp.stop_all().await;
                json!({"ok": true, "result": {"stopped": stopped}})
            }
            IPC_METHOD_SKILLS_LIST => match self.snapshots.list_skills() {
                Ok(skills) => json!({"ok": true, "result": skills}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            },
            IPC_METHOD_SKILL_GET => {
                let slug = params
                    .and_then(|p| p.get("slug"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let subfile = params.and_then(|p| p.get("file_path")).and_then(Value::as_str);
                match self.snapshots.read_skill(slug, subfile) {
                    Ok((skill, sub)) => json!({"ok": true, "result": {"skill": skill, "subfile": sub}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SKILL_MANAGE => {
                let action_s = params
                    .and_then(|p| p.get("action"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let Some(action) = SkillManageAction::parse(action_s) else {
                    return json!({"ok": false, "error": format!("unknown skill action `{action_s}`")});
                };
                let cfg = self.config().skills;
                let secrets = self.vault.all_values();
                let slug = params
                    .and_then(|p| p.get("slug"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match action {
                    SkillManageAction::Create => {
                        let content = params
                            .and_then(|p| p.get("content"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let name = params.and_then(|p| p.get("name")).and_then(Value::as_str);
                        let desc = params
                            .and_then(|p| p.get("description"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let enabled = params
                            .and_then(|p| p.get("enabled"))
                            .and_then(Value::as_bool)
                            .unwrap_or(true);
                        match self.snapshots.create_skill_structured(
                            slug,
                            name,
                            desc,
                            content,
                            enabled,
                            SkillStatus::Active,
                            SkillAuthor::User,
                            &[],
                            &[],
                            "cli",
                            &secrets,
                            cfg.max_history_snapshots,
                        ) {
                            Ok((doc, snap)) => {
                                self.emit(AgentEvent::SkillUpdated {
                                    slug: doc.slug.clone(),
                                    action: "created".into(),
                                    status: doc.status.as_str().into(),
                                });
                                json!({"ok": true, "result": {"skill": doc, "snapshot": snap}})
                            }
                            Err(e) => json!({"ok": false, "error": e.to_string()}),
                        }
                    }
                    SkillManageAction::Toggle => {
                        let enabled = params
                            .and_then(|p| p.get("enabled"))
                            .and_then(Value::as_bool)
                            .unwrap_or(true);
                        match self
                            .snapshots
                            .toggle_skill(slug, enabled, cfg.max_history_snapshots)
                        {
                            Ok((doc, snap)) => {
                                self.emit(AgentEvent::SkillUpdated {
                                    slug: doc.slug.clone(),
                                    action: "toggled".into(),
                                    status: doc.status.as_str().into(),
                                });
                                json!({"ok": true, "result": {"skill": doc, "snapshot": snap}})
                            }
                            Err(e) => json!({"ok": false, "error": e.to_string()}),
                        }
                    }
                    SkillManageAction::Delete => match self.snapshots.delete_skill(slug) {
                        Ok(deleted) => json!({"ok": true, "result": {"deleted": deleted}}),
                        Err(e) => json!({"ok": false, "error": e.to_string()}),
                    },
                    SkillManageAction::Patch => {
                        let old_text = params
                            .and_then(|p| p.get("old_text"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let new_text = params
                            .and_then(|p| p.get("new_text"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        match self.snapshots.patch_skill_with_secrets(
                            slug,
                            old_text,
                            new_text,
                            "cli_patch",
                            &secrets,
                            cfg.max_history_snapshots,
                        ) {
                            Ok((doc, diff, snap)) => {
                                json!({"ok": true, "result": {"skill": doc, "diff": diff, "snapshot": snap}})
                            }
                            Err(e) => json!({"ok": false, "error": e.to_string()}),
                        }
                    }
                    _ => json!({"ok": false, "error": format!("unsupported IPC skill action `{action_s}`")}),
                }
            }
            IPC_METHOD_SKILL_APPROVE => {
                let slug = params
                    .and_then(|p| p.get("slug"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let max_h = self.config().skills.max_history_snapshots;
                match self.snapshots.approve_skill(slug, max_h) {
                    Ok((doc, snap)) => {
                        self.emit(AgentEvent::SkillUpdated {
                            slug: doc.slug.clone(),
                            action: "approved".into(),
                            status: doc.status.as_str().into(),
                        });
                        json!({"ok": true, "result": {"skill": doc, "snapshot": snap}})
                    }
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SKILL_HISTORY => {
                let slug = params.and_then(|p| p.get("slug")).and_then(Value::as_str);
                match self.snapshots.list_skill_history(slug) {
                    Ok(items) => json!({"ok": true, "result": items}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_SKILL_ROLLBACK => {
                let sid = params
                    .and_then(|p| p.get("snapshot_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let max_h = self.config().skills.max_history_snapshots;
                match self.snapshots.rollback_skill(sid, max_h) {
                    Ok((doc, diff)) => {
                        self.emit(AgentEvent::SkillUpdated {
                            slug: doc.slug.clone(),
                            action: "rollback".into(),
                            status: doc.status.as_str().into(),
                        });
                        json!({"ok": true, "result": {"skill": doc, "diff": diff}})
                    }
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_GOALS_LIST => {
                let limit = params
                    .and_then(|p| p.get("limit"))
                    .and_then(Value::as_u64)
                    .unwrap_or(GOAL_LOOP_LIST_DEFAULT_LIMIT as u64) as usize;
                match self.store.list_goal_loops(limit) {
                    Ok(goals) => json!({"ok": true, "result": goals}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_GOAL_GET => {
                let id = params.and_then(|p| p.get("id")).and_then(Value::as_str).unwrap_or("");
                match self.store.get_goal_loop(id) {
                    Ok(Some(goal)) => {
                        let runs = self
                            .store
                            .list_goal_loop_runs(&goal.id, GOAL_LOOP_RUNS_DEFAULT_LIMIT)
                            .unwrap_or_default();
                        json!({"ok": true, "result": {"goal": goal, "runs": runs}})
                    }
                    Ok(None) => json!({"ok": false, "error": format!("goal loop `{id}` not found")}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_GOAL_CREATE => {
                let req: std::result::Result<GoalLoopCreateRequest, _> =
                    serde_json::from_value(params.cloned().unwrap_or(Value::Null));
                match req {
                    Ok(r) => match self.create_goal_loop(r) {
                        Ok(goal) => json!({"ok": true, "result": goal}),
                        Err(e) => json!({"ok": false, "error": e.to_string()}),
                    },
                    Err(e) => json!({"ok": false, "error": format!("invalid goal create payload: {e}")}),
                }
            }
            IPC_METHOD_GOAL_UPDATE => {
                let id = params.and_then(|p| p.get("id")).and_then(Value::as_str).unwrap_or("");
                let patch: GoalLoopPatch = params
                    .and_then(|p| p.get("patch"))
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                match self.update_goal_loop(id, patch) {
                    Ok(Some(goal)) => json!({"ok": true, "result": goal}),
                    Ok(None) => json!({"ok": false, "error": format!("goal loop `{id}` not found")}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_GOAL_RUN_NOW => {
                let id = params.and_then(|p| p.get("id")).and_then(Value::as_str).unwrap_or("");
                match crate::goals::execute_goal_iteration(self, id, CancellationToken::new()).await {
                    Ok(goal) => json!({"ok": true, "result": goal}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_GOAL_DELETE => {
                let id = params.and_then(|p| p.get("id")).and_then(Value::as_str).unwrap_or("");
                match self.delete_goal_loop(id) {
                    Ok(deleted) => json!({"ok": true, "result": {"deleted": deleted}}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            IPC_METHOD_TRIGGERS_LIST => {
                json!({"ok": true, "result": self.list_triggers()})
            }
            IPC_METHOD_EXPORT_SESSION => {
                let session_id = params
                    .and_then(|p| p.get("session_id"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let opts: ExportOptions = params
                    .and_then(|p| p.get("options"))
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                let res = match session_id {
                    Some(sid) => self.export_session_trajectory(sid, &opts),
                    None => self.export_batch_trajectories(&opts),
                };
                match res {
                    Ok(rep) => json!({"ok": true, "result": rep}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                }
            }
            other => json!({"ok": false, "error": format!("unknown method: {other}")}),
        }
    }

    #[cfg(unix)]
    async fn stream_ipc_logs(
        self: &Arc<Self>,
        filter: &LogFilter,
        writer: &mut tokio::net::unix::OwnedWriteHalf,
        cancel: CancellationToken,
    ) {
        let mut rx = self.subscribe();
        let initial_limit = filter.limit.unwrap_or(20);
        if initial_limit > 0 {
            let mut init_filter = filter.clone();
            init_filter.limit = Some(initial_limit);
            if let Ok(rows) = self.store.query_logs(&init_filter) {
                for entry in rows {
                    let mut line = serde_json::to_string(&json!({
                        "ok": true,
                        "event": "log",
                        "entry": entry,
                    }))
                    .unwrap_or_default();
                    line.push('\n');
                    if writer.write_all(line.as_bytes()).await.is_err() {
                        return;
                    }
                }
            }
        }

        loop {
            let ev = tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                msg = rx.recv() => match msg {
                    Ok(e) => e,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            };
            if let AgentEvent::Log(entry) = ev {
                if !filter.matches(&entry) {
                    continue;
                }
                let mut line = serde_json::to_string(&json!({
                    "ok": true,
                    "event": "log",
                    "entry": entry,
                }))
                .unwrap_or_default();
                line.push('\n');
                if writer.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
        }
    }

    #[cfg(unix)]
    async fn stream_ipc_session(
        self: &Arc<Self>,
        session_id: &str,
        replay: usize,
        writer: &mut tokio::net::unix::OwnedWriteHalf,
        cancel: CancellationToken,
    ) {
        let mut rx = self.subscribe();
        let watch_all = session_id.trim().is_empty() || session_id == "*";

        if !watch_all {
            let Some(session) = self.store.get_session(session_id).ok().flatten() else {
                let mut err = serde_json::to_string(&json!({
                    "ok": false,
                    "error": format!("session `{session_id}` not found"),
                }))
                .unwrap_or_default();
                err.push('\n');
                let _ = writer.write_all(err.as_bytes()).await;
                return;
            };
            let branch = self.store.get_active_branch(session_id).unwrap_or_default();
            let start = branch.len().saturating_sub(replay);
            let queued = self
                .store
                .list_queued_followups(session_id)
                .unwrap_or_default();
            let jobs = self
                .jobs
                .list_jobs(Some(session_id), None, JOB_DEFAULT_LIST_LIMIT)
                .unwrap_or_default();
            let mut snap = serde_json::to_string(&json!({
                "ok": true,
                "kind": IPC_FRAME_SNAPSHOT,
                "session": session,
                "events": &branch[start..],
                "queued_prompts": queued,
                "jobs": jobs,
            }))
            .unwrap_or_default();
            snap.push('\n');
            if writer.write_all(snap.as_bytes()).await.is_err() {
                return;
            }
        }

        loop {
            let ev = tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                msg = rx.recv() => match msg {
                    Ok(e) => e,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            };
            let Some(ev_sid) = agent_event_session_id(&ev) else {
                continue;
            };
            if !watch_all && ev_sid != session_id {
                continue;
            }
            let mut line = serde_json::to_string(&json!({
                "ok": true,
                "kind": IPC_FRAME_EVENT,
                "data": ev,
            }))
            .unwrap_or_default();
            line.push('\n');
            if writer.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    }

    /// Event-driven, zero-polling live log follower for background jobs over Unix IPC.
    #[cfg(unix)]
    async fn stream_ipc_job_follow(
        self: &Arc<Self>,
        job_id: &str,
        initial_offset: u64,
        writer: &mut tokio::net::unix::OwnedWriteHalf,
        cancel: CancellationToken,
    ) {
        // Subscribe to progress watch BEFORE initial read so zero bytes can slip through.
        let mut progress = self.jobs.subscribe_progress(job_id);
        let mut offset = initial_offset;

        let (next_off, initial_chunks, mut last_job) =
            match drain_job_chunks(&self.jobs, job_id, offset).await {
                Ok(v) => v,
                Err(e) => {
                    let _ =
                        write_ipc_frame(writer, json!({"ok": false, "error": e.to_string()})).await;
                    return;
                }
            };
        offset = next_off;
        for chunk in initial_chunks {
            if !write_ipc_frame(
                writer,
                json!({"ok": true, "kind": IPC_FRAME_CHUNK, "data": chunk}),
            )
            .await
            {
                return;
            }
        }

        if let Some((ref mut bytes_rx, ref mut done_rx)) = progress {
            loop {
                let finished = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return,
                    res = done_rx.wait_for(|&d| d) => res.is_ok() || res.is_err(),
                    changed = bytes_rx.changed() => changed.is_err(),
                };

                if let Ok((next_o, chunks, job_opt)) =
                    drain_job_chunks(&self.jobs, job_id, offset).await
                {
                    offset = next_o;
                    if job_opt.is_some() {
                        last_job = job_opt;
                    }
                    for chunk in chunks {
                        if !write_ipc_frame(
                            writer,
                            json!({"ok": true, "kind": IPC_FRAME_CHUNK, "data": chunk}),
                        )
                        .await
                        {
                            return;
                        }
                    }
                }

                if finished {
                    break;
                }
            }
        }

        let final_job = self.jobs.get_job(job_id).ok().flatten().or(last_job);
        let _ = write_ipc_frame(
            writer,
            json!({"ok": true, "kind": IPC_FRAME_DONE, "job": final_job}),
        )
        .await;
    }
}

#[cfg(unix)]
async fn write_ipc_frame(writer: &mut tokio::net::unix::OwnedWriteHalf, payload: Value) -> bool {
    let mut line = serde_json::to_string(&payload).unwrap_or_default();
    line.push('\n');
    writer.write_all(line.as_bytes()).await.is_ok()
}

#[cfg(unix)]
async fn drain_job_chunks(
    jobs: &Arc<LocalJobSupervisor>,
    job_id: &str,
    start_offset: u64,
) -> Result<(
    u64,
    Vec<zenko_core::JobReadChunk>,
    Option<zenko_core::BackgroundJobRecord>,
)> {
    let mut cur = start_offset;
    let mut chunks = Vec::new();
    let latest_job = loop {
        let chunk = jobs
            .read_job_output(job_id, Some(cur), JOB_DEFAULT_READ_BYTES)
            .await?;
        let advanced = chunk.next_offset > cur;
        cur = chunk.next_offset;
        if advanced && !chunk.content.is_empty() {
            chunks.push(chunk);
        } else {
            break Some(chunk.job);
        }
    };
    Ok((cur, chunks, latest_job))
}

