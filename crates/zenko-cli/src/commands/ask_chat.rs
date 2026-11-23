use super::*;

pub struct AskParams {
    pub prompt: Vec<String>,
    pub session: Option<String>,
    pub workspace: Option<PathBuf>,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub timeout_secs: Option<u64>,
    pub standalone: bool,
    pub wait_approval: bool,
    pub output: OutputMode,
}

fn resolve_prompt_text(args: &[String]) -> Result<String> {
    let joined = args.join(" ");
    let use_stdin = joined.trim() == "-" || (joined.trim().is_empty() && !std::io::stdin().is_terminal());
    match use_stdin {
        true => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("failed to read prompt from stdin")?;
            let trimmed = buf.trim().to_string();
            match trimmed.is_empty() {
                true => anyhow::bail!("stdin was empty; provide a prompt argument or pipe text"),
                false => Ok(trimmed),
            }
        }
        false => match joined.trim().is_empty() {
            true => anyhow::bail!("Provide a prompt, e.g. `zenko ask \"summarize this repo\"` or pipe via stdin"),
            false => Ok(joined),
        },
    }
}

pub async fn run_ask(config_path: &Path, params: AskParams) -> Result<()> {
    let prompt = resolve_prompt_text(&params.prompt)?;
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let timeout = params.timeout_secs.map(Duration::from_secs);
    let perm_override = params.mode.as_deref().map(PermissionMode::from_str_loose);

    // Prefer live running daemon over Unix IPC unless standalone or streaming NDJSON is requested.
    let try_ipc = !params.standalone && params.output != OutputMode::StreamJson;
    if try_ipc {
        let ipc_params = json!({
            "prompt": prompt,
            "session_id": params.session,
            "workspace": params.workspace.as_ref().map(|p| p.display().to_string()),
            "model": params.model,
            "mode": perm_override.map(PermissionMode::as_str),
            "headless_no_wait": !params.wait_approval,
        });
        match ipc_call_timeout(&cfg, IPC_METHOD_ASK, ipc_params, timeout).await {
            Ok(resp) => {
                let data = unwrap_ipc_result(resp)?;
                return emit_ask_output(&data, params.output);
            }
            Err(e) if is_daemon_offline(&e) => {}
            Err(e) => return Err(e),
        }
    }

    // Standalone inline execution (also powers `--stream-json` live event emission).
    let (runtime, _logs) = AgentRuntime::build(config_path.to_path_buf(), cfg.clone())?;
    let sid = match params.session {
        Some(id) => id,
        None => runtime
            .store
            .create_session(
                "CLI Session",
                "cli",
                None,
                None,
                perm_override.unwrap_or(cfg.policy.mode),
            )?
            .id,
    };

    let stream_done = CancellationToken::new();
    let stream_task = match params.output {
        OutputMode::StreamJson => {
            let mut rx = runtime.subscribe();
            let target_sid = sid.clone();
            let done_sig = stream_done.clone();
            Some(tokio::spawn(async move {
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
                    if let Some(line) = event_to_ndjson(&target_sid, &ev) {
                        println!("{line}");
                    }
                }
            }))
        }
        OutputMode::Text | OutputMode::Json => None,
    };

    let run_fut = runtime.run_turn_with_options(
        &sid,
        &prompt,
        TurnOptions {
            parent_event_override: None,
            workspace_override: params.workspace,
            model_override: params.model,
            permission_override: perm_override,
            headless_no_wait: !params.wait_approval,
            ..TurnOptions::default()
        },
    );

    let turn_res = match timeout {
        Some(dur) => match tokio::time::timeout(dur, run_fut).await {
            Ok(inner) => inner,
            Err(_) => {
                runtime.cancel_session(&sid);
                let _ = runtime
                    .store
                    .update_session_status(&sid, SessionStatus::Idle);
                tokio::task::yield_now().await;
                stream_done.cancel();
                if let Some(handle) = stream_task {
                    let _ = handle.await;
                }
                runtime.logs.flush();
                anyhow::bail!("turn timed out after {}s", dur.as_secs());
            }
        },
        None => run_fut.await,
    };

    tokio::task::yield_now().await;
    stream_done.cancel();
    if let Some(handle) = stream_task {
        let _ = handle.await;
    }

    let turn = turn_res?;
    runtime.logs.flush();
    let data = serde_json::to_value(&turn)?;
    emit_ask_output(&data, params.output)
}

fn emit_ask_output(data: &Value, mode: OutputMode) -> Result<()> {
    match mode {
        OutputMode::Json => {
            println!("{}", serde_json::to_string_pretty(&json!({ "ok": true, "result": data }))?);
        }
        OutputMode::StreamJson => {
            println!("{}", serde_json::to_string(&json!({ "event": "result", "data": data }))?);
        }
        OutputMode::Text => {
            if let Some(sid) = data.get("session_id").and_then(Value::as_str) {
                eprintln!("[session: {sid}]");
            }
            let reply = data.get("reply").and_then(Value::as_str).unwrap_or("");
            println!("{reply}");
        }
    }
    Ok(())
}

fn event_to_ndjson(target_sid: &str, ev: &AgentEvent) -> Option<String> {
    let payload = match ev {
        AgentEvent::ThinkingDelta { session_id, text, .. } if session_id == target_sid => {
            json!({ "event": "thinking", "session_id": session_id, "text": text })
        }
        AgentEvent::TextDelta { session_id, text, .. } if session_id == target_sid => {
            json!({ "event": "text", "session_id": session_id, "text": text })
        }
        AgentEvent::ToolCallStarted {
            session_id,
            call_id,
            tool_name,
            args_preview,
            ..
        } if session_id == target_sid => json!({
            "event": "tool_start",
            "session_id": session_id,
            "call_id": call_id,
            "tool": tool_name,
            "args": args_preview,
        }),
        AgentEvent::ToolCallFinished {
            session_id,
            call_id,
            tool_name,
            is_error,
            duration_ms,
            ..
        } if session_id == target_sid => json!({
            "event": "tool_done",
            "session_id": session_id,
            "call_id": call_id,
            "tool": tool_name,
            "is_error": is_error,
            "duration_ms": duration_ms,
        }),
        AgentEvent::OrchestratorUpdated {
            session_id,
            stage,
            tier,
            detail,
            model,
        } if session_id == target_sid => json!({
            "event": "orchestrator_updated",
            "session_id": session_id,
            "stage": stage,
            "tier": tier,
            "detail": detail,
            "model": model,
        }),
        AgentEvent::BrainstormRecorded { session_id, spec } if session_id == target_sid => json!({
            "event": "brainstorm_recorded",
            "session_id": session_id,
            "spec": spec,
        }),
        AgentEvent::PlanUpdated { session_id, plan } if session_id == target_sid => json!({
            "event": "plan_updated",
            "session_id": session_id,
            "plan": plan,
        }),
        AgentEvent::ClarificationRequested { clarification }
            if clarification.session_id == target_sid =>
        {
            json!({
                "event": "clarification_requested",
                "clarification": clarification,
            })
        }
        AgentEvent::ClarificationResolved {
            clarification_id,
            session_id,
            selected_option_id,
            custom_answer,
            resolved_by,
        } if session_id == target_sid => json!({
            "event": "clarification_resolved",
            "clarification_id": clarification_id,
            "session_id": session_id,
            "selected_option_id": selected_option_id,
            "custom_answer": custom_answer,
            "resolved_by": resolved_by,
        }),
        AgentEvent::TurnError { session_id, message } if session_id == target_sid => {
            json!({ "event": "error", "session_id": session_id, "message": message })
        }
        _ => return None,
    };
    serde_json::to_string(&payload).ok()
}

pub async fn run_chat(
    config_path: &Path,
    session_id: Option<String>,
    workspace: Option<PathBuf>,
    model: Option<String>,
    mode: Option<String>,
    standalone: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let mut active_mode = mode
        .as_deref()
        .map(PermissionMode::from_str_loose)
        .unwrap_or(cfg.policy.mode);
    let mut active_model = model;

    // Auto-attach to a running `zenko web` daemon over Unix IPC unless `--standalone` is set.
    let daemon_online = !standalone
        && ipc_call_timeout(
            &cfg,
            IPC_METHOD_STATUS,
            json!({}),
            Some(Duration::from_millis(IPC_DAEMON_PROBE_TIMEOUT_MS)),
        )
        .await
        .is_ok();

    if daemon_online {
        return run_daemon_chat(cfg, session_id, workspace, active_model, active_mode).await;
    }

    let (runtime, _logs) = AgentRuntime::build(config_path.to_path_buf(), cfg.clone())?;
    let mut sid = match session_id {
        Some(id) => id,
        None => runtime
            .store
            .create_session("Terminal Chat", "cli", None, None, active_mode)?
            .id,
    };

    runtime.logs.push(
        format_hono_entry(
            LogDirection::In,
            LogChannel::Tui,
            &format!("chat session started (mode={})", active_mode.as_str()),
            Some(("ready", LogStatusTone::Ok)),
            None,
        )
        .with_session(sid.clone()),
    );

    eprintln!(
        "Zenko Interactive Chat · session={sid} · mode={} (commands: /new /clear /model /mode /exit)",
        active_mode.as_str()
    );

    let mut stdin_reader = tokio::io::BufReader::new(tokio::io::stdin());
    let mut stdout = std::io::stdout();

    loop {
        eprint!("\nzenko> ");
        let _ = std::io::stderr().flush();

        let mut line = String::new();
        let bytes = tokio::io::AsyncBufReadExt::read_line(&mut stdin_reader, &mut line).await?;
        if bytes == 0 {
            break;
        }
        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        if let Some(cmd_body) = input.strip_prefix('/') {
            let mut parts = cmd_body.splitn(2, ' ');
            let cmd = parts.next().unwrap_or("");
            let arg = parts.next().unwrap_or("").trim();
            match cmd {
                "exit" | "quit" | "q" => break,
                "new" => {
                    sid = runtime
                        .store
                        .create_session("Terminal Chat", "cli", None, None, active_mode)?
                        .id;
                    eprintln!("Switched to new session: {sid}");
                    continue;
                }
                "clear" => {
                    runtime.store.clear_session_events(&sid)?;
                    eprintln!("Cleared session {sid}.");
                    continue;
                }
                "model" if !arg.is_empty() => {
                    active_model = Some(arg.to_string());
                    eprintln!("Active model override: {arg}");
                    continue;
                }
                "mode" if !arg.is_empty() => {
                    active_mode = PermissionMode::from_str_loose(arg);
                    eprintln!("Active policy mode: {}", active_mode.as_str());
                    continue;
                }
                "help" => {
                    eprintln!("Commands: /new, /clear, /model <id>, /mode <strict|balanced|autonomous>, /exit");
                    continue;
                }
                _ => {
                    eprintln!("Unknown command `/{cmd}` (try /help)");
                    continue;
                }
            }
        }

        let done_sig = CancellationToken::new();
        let mut rx = runtime.subscribe();
        let watch_sid = sid.clone();
        let watch_done = done_sig.clone();
        let printer = tokio::spawn(async move {
            let mut out = std::io::stdout();
            loop {
                let ev = tokio::select! {
                    biased;
                    _ = watch_done.cancelled() => break,
                    msg = rx.recv() => match msg {
                        Ok(e) => e,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                };
                match ev {
                    AgentEvent::TextDelta { session_id, text, .. } if session_id == watch_sid => {
                        let _ = write!(out, "{text}");
                        let _ = out.flush();
                    }
                    AgentEvent::ToolCallStarted {
                        session_id,
                        tool_name,
                        args_preview,
                        ..
                    } if session_id == watch_sid => {
                        eprintln!("\n  --> tool {tool_name} ({args_preview})");
                    }
                    AgentEvent::ToolCallFinished {
                        session_id,
                        tool_name,
                        is_error,
                        duration_ms,
                        ..
                    } if session_id == watch_sid => {
                        let tag = match is_error {
                            true => "err",
                            false => "ok",
                        };
                        eprintln!("  <-- tool {tool_name} {tag} {duration_ms}ms");
                    }
                    _ => {}
                }
            }
        });

        let res = runtime
            .run_turn_with_options(
                &sid,
                input,
                TurnOptions {
                    parent_event_override: None,
                    workspace_override: workspace.clone(),
                    model_override: active_model.clone(),
                    permission_override: Some(active_mode),
                    headless_no_wait: true,
                    ..TurnOptions::default()
                },
            )
            .await;

        done_sig.cancel();
        let _ = printer.await;
        println!();
        let _ = stdout.flush();

        if let Err(err) = res {
            eprintln!("error: {err}");
        }
    }
    runtime.logs.push(
        format_hono_entry(
            LogDirection::Out,
            LogChannel::Tui,
            "chat session ended",
            Some(("done", LogStatusTone::Neutral)),
            None,
        )
        .with_session(sid),
    );
    runtime.logs.flush();
    Ok(())
}

async fn run_daemon_chat(
    cfg: ZenkoConfig,
    mut sid: Option<String>,
    workspace: Option<PathBuf>,
    mut active_model: Option<String>,
    mut active_mode: PermissionMode,
) -> Result<()> {
    eprintln!(
        "Zenko Live Chat (attached to `zenko web` daemon) · session={} · mode={} (commands: /new /clear /bg /model /mode /exit)",
        sid.as_deref().unwrap_or("auto"),
        active_mode.as_str()
    );

    let mut stdin_reader = tokio::io::BufReader::new(tokio::io::stdin());
    let mut stdout = std::io::stdout();

    loop {
        eprint!("\nzenko> ");
        let _ = std::io::stderr().flush();

        let mut line = String::new();
        let bytes = tokio::io::AsyncBufReadExt::read_line(&mut stdin_reader, &mut line).await?;
        if bytes == 0 {
            break;
        }
        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        if let Some(cmd_body) = input.strip_prefix('/') {
            let mut parts = cmd_body.splitn(2, ' ');
            let cmd = parts.next().unwrap_or("");
            let arg = parts.next().unwrap_or("").trim();
            match cmd {
                "exit" | "quit" | "q" => break,
                "new" => {
                    sid = None;
                    eprintln!("Next prompt will start a new daemon session.");
                    continue;
                }
                "clear" => {
                    if let Some(ref cur) = sid {
                        let _ = ipc_call(&cfg, IPC_METHOD_SESSION_CLEAR, json!({ "session_id": cur })).await;
                        eprintln!("Cleared session {cur}.");
                    }
                    continue;
                }
                "bg" => {
                    if let Some(ref cur) = sid {
                        let _ = ipc_call(&cfg, IPC_METHOD_SESSION_BACKGROUND, json!({ "session_id": cur })).await;
                        eprintln!("Requested background demotion for session {cur}.");
                    }
                    continue;
                }
                "model" if !arg.is_empty() => {
                    active_model = Some(arg.to_string());
                    eprintln!("Active model override: {arg}");
                    continue;
                }
                "mode" if !arg.is_empty() => {
                    active_mode = PermissionMode::from_str_loose(arg);
                    eprintln!("Active policy mode: {}", active_mode.as_str());
                    continue;
                }
                "help" => {
                    eprintln!("Commands: /new, /clear, /bg, /model <id>, /mode <strict|balanced|autonomous>, /exit");
                    continue;
                }
                _ => {
                    eprintln!("Unknown command `/{cmd}` (try /help)");
                    continue;
                }
            }
        }

        let resp = ipc_call(
            &cfg,
            IPC_METHOD_ASK,
            json!({
                "prompt": input,
                "session_id": sid,
                "workspace": workspace.as_ref().map(|p| p.display().to_string()),
                "model": active_model,
                "mode": active_mode.as_str(),
                "headless_no_wait": true,
                "send_mode": "queue",
            }),
        )
        .await?;
        match unwrap_ipc_result(resp) {
            Ok(data) => {
                if let Some(new_sid) = data.get("session_id").and_then(Value::as_str)
                    && sid.as_deref() != Some(new_sid) {
                        sid = Some(new_sid.to_string());
                        eprintln!("[session: {new_sid}]");
                    }
                let reply = data.get("reply").and_then(Value::as_str).unwrap_or("");
                println!("{reply}");
                let _ = stdout.flush();
            }
            Err(err) => eprintln!("error: {err}"),
        }
    }
    eprintln!("Detached from `zenko web` daemon.");
    Ok(())
}

pub async fn run_status(config_path: &Path, json_out: bool) -> Result<i32> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    if let Ok(resp) = ipc_call(&cfg, IPC_METHOD_STATUS, json!({})).await
        && let Ok(t) = unwrap_ipc_result(resp) {
            match json_out {
                true => println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "running": true,
                        "webui": format!("http://{}:{}", cfg.webui.bind, cfg.webui.port),
                        "telemetry": t,
                    }))?
                ),
                false => {
                    let rss_mb = t.get("rss_bytes").and_then(Value::as_u64).unwrap_or(0) as f64
                        / (1024.0 * 1024.0);
                    let tg = match t["telegram_connected"].as_bool().unwrap_or(false) {
                        true => "CONNECTED",
                        false => "OFFLINE",
                    };
                    println!("Zenko Daemon: RUNNING (PID {})", t["pid"]);
                    println!("  WebUI:          http://{}:{}", cfg.webui.bind, cfg.webui.port);
                    println!("  RAM (VmRSS):    {rss_mb:.1} MB");
                    println!("  Uptime:         {}s", t["uptime_secs"]);
                    println!("  Active Runs:    {}", t["active_sessions"]);
                    println!("  Pending Appr:   {}", t["pending_approvals"]);
                    println!("  Daily Spend:    ${:.4}", t["daily_cost_usd"].as_f64().unwrap_or(0.0));
                    println!("  Cache Hit:      {:.1}%", t["cache_hit_pct"].as_f64().unwrap_or(0.0));
                    println!("  Telegram:       {tg}");
                }
            }
            return Ok(EXIT_OK);
        }

    match json_out {
        true => println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "running": false,
                "config": config_path.display().to_string(),
            }))?
        ),
        false => {
            println!("Zenko Daemon: STOPPED");
            println!("  Config: {}", config_path.display());
            println!("  Start with: zenko web");
        }
    }
    Ok(EXIT_DAEMON_STOPPED)
}

