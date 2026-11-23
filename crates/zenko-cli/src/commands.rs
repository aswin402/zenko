//! Terminal command implementations designed equally for humans AND AI agents
//! (`--json`, `--stream-json` NDJSON, stdin piping, per-call overrides, REPL chat,
//! session/approval/memory/cron/log management, and deterministic exit codes).

use crate::ipc::{
    OutputMode, ipc_attach_session, ipc_call, ipc_call_timeout, ipc_follow_job, ipc_follow_logs,
    unwrap_ipc_result,
};
use anyhow::{Context, Result};
use chrono::Utc;
use croner::Cron;
use serde_json::{Value, json};
use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    AgentEvent, ApprovalDecision, BackgroundJobStatus, CronJob, CronKind, DEFAULT_PAIRING_TTL_MINS,
    EXIT_DAEMON_STOPPED, EXIT_OK, HUB_CATEGORIES, IPC_DAEMON_PROBE_TIMEOUT_MS, IPC_FRAME_CHUNK,
    IPC_FRAME_DONE, IPC_FRAME_EVENT, IPC_FRAME_SNAPSHOT, IPC_METHOD_AGENTS_DIFF,
    IPC_METHOD_AGENTS_DISCARD, IPC_METHOD_AGENTS_GET, IPC_METHOD_AGENTS_KILL,
    IPC_METHOD_AGENTS_LIST, IPC_METHOD_AGENTS_MERGE, IPC_METHOD_AGENTS_MESSAGE,
    IPC_METHOD_AGENTS_PRUNE, IPC_METHOD_AGENTS_SPAWN, IPC_METHOD_AGENTS_WAIT,
    IPC_METHOD_APPROVAL_RESOLVE,
    IPC_METHOD_APPROVALS_LIST, IPC_METHOD_ASK, IPC_METHOD_CHECKPOINTS_DIFF,
    IPC_METHOD_CHECKPOINTS_LIST, IPC_METHOD_CHECKPOINTS_PRUNE, IPC_METHOD_CHECKPOINTS_REWIND,
    IPC_METHOD_CLARIFICATION_RESOLVE, IPC_METHOD_CLARIFICATIONS_LIST, IPC_METHOD_CRON_CREATE,
    IPC_METHOD_CRON_DELETE, IPC_METHOD_CRON_LIST, IPC_METHOD_CRON_RUN_NOW, IPC_METHOD_CRON_UPDATE,
    IPC_METHOD_EXPORT_SESSION, IPC_METHOD_GOAL_CREATE, IPC_METHOD_GOAL_DELETE, IPC_METHOD_GOAL_GET,
    IPC_METHOD_GOAL_RUN_NOW, IPC_METHOD_GOAL_UPDATE, IPC_METHOD_GOALS_LIST, IPC_METHOD_HUB_LIST,
    IPC_METHOD_JOBS_DELETE, IPC_METHOD_JOBS_INPUT, IPC_METHOD_JOBS_LIST, IPC_METHOD_JOBS_PRUNE,
    IPC_METHOD_JOBS_READ, IPC_METHOD_JOBS_RESTART, IPC_METHOD_JOBS_START, IPC_METHOD_JOBS_STOP,
    IPC_METHOD_JOBS_UPDATE, IPC_METHOD_JOBS_WAIT, IPC_METHOD_LOGS_QUERY, IPC_METHOD_LSP_STATUS,
    IPC_METHOD_LSP_STOP, IPC_METHOD_MCP_SERVER_DELETE, IPC_METHOD_MCP_SERVER_SYNC,
    IPC_METHOD_MCP_SERVER_UPSERT, IPC_METHOD_MCP_SERVERS_LIST, IPC_METHOD_MEMORY_ADD,
    IPC_METHOD_MEMORY_DELETE, IPC_METHOD_MEMORY_DREAM, IPC_METHOD_MEMORY_HISTORY,
    IPC_METHOD_MEMORY_ROLLBACK, IPC_METHOD_MEMORY_SEARCH, IPC_METHOD_MEMORY_UPDATE,
    IPC_METHOD_SESSION_APPROVE_PLAN, IPC_METHOD_SESSION_BACKGROUND, IPC_METHOD_SESSION_CLEAR,
    IPC_METHOD_SESSION_CONTEXT, IPC_METHOD_SESSION_DELETE, IPC_METHOD_SESSION_GET,
    IPC_METHOD_SESSION_QUEUE_CANCEL, IPC_METHOD_SESSION_QUEUE_LIST, IPC_METHOD_SESSION_STEER,
    IPC_METHOD_SESSIONS_LIST, IPC_METHOD_SESSIONS_SEARCH, IPC_METHOD_SKILL_APPROVE,
    IPC_METHOD_SKILL_GET, IPC_METHOD_SKILL_HISTORY, IPC_METHOD_SKILL_MANAGE,
    IPC_METHOD_SKILL_ROLLBACK, IPC_METHOD_SKILLS_LIST, IPC_METHOD_STATUS,
    IPC_METHOD_TASKS_DAG_CREATE, IPC_METHOD_TASKS_DAG_DELETE, IPC_METHOD_TASKS_DAG_LIST,
    IPC_METHOD_TASKS_DAG_UPDATE, IPC_METHOD_TRIGGERS_LIST, IPC_WAIT_TIMEOUT_SLACK_SECS,
    JOB_DEFAULT_READ_BYTES, JOB_DEFAULT_WAIT_SECS, LSP_SERVER_REGISTRY, LlmProvider, LogChannel,
    LogDirection, LogEntry, LogFilter, LogStatusTone, MEMORY_SCOPE_GLOBAL, McpServerConfig,
    McpTransport, OrchestratorBridge, PeerRole, PermissionMode, RewindMode, RewindRequest,
    RiskLevel, SUBAGENT_DEFAULT_WAIT_SECS, SUBAGENT_MAX_WAIT_SECS, SecretVault,
    SessionSearchFilter, SessionStatus, SessionTask, SessionTaskPatch, SessionTaskStatus,
    SkillAuthor, SkillStatus, SubagentSpawnRequest, SubagentWorkspaceMode, TaskKind,
    VAULT_KEY_TELEGRAM, VAULT_KEY_WEBUI_ADMIN, ZenkoConfig, find_provider, format_hono_entry,
    format_tokens, new_id,
    ExportFormat, ExportOptions, GOAL_LOOP_IPC_TIMEOUT_SECS, GOAL_LOOP_RUNS_DEFAULT_LIMIT,
    GoalLoopCreateRequest, GoalLoopPatch, GoalLoopStatus,
};
use zenko_gateways::verify_bot_token;
use zenko_memory::{
    SnapshotManager, SqliteStore, parse_since_window, project_scope_for_workspace, resolve_git_root,
};
use zenko_providers::UnifiedLlmProvider;
use zenko_runtime::{AgentRuntime, TurnOptions};

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
        if let Ok(resp) = ipc_call_timeout(&cfg, IPC_METHOD_ASK, ipc_params, timeout).await {
            let data = unwrap_ipc_result(resp)?;
            return emit_ask_output(&data, params.output);
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

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    loop {
        eprint!("\nzenko> ");
        let _ = std::io::stderr().flush();

        let mut line = String::new();
        let bytes = stdin.lock().read_line(&mut line)?;
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

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    loop {
        eprint!("\nzenko> ");
        let _ = std::io::stderr().flush();

        let mut line = String::new();
        let bytes = stdin.lock().read_line(&mut line)?;
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
                if let Some(new_sid) = data.get("session_id").and_then(Value::as_str) {
                    if sid.as_deref() != Some(new_sid) {
                        sid = Some(new_sid.to_string());
                        eprintln!("[session: {new_sid}]");
                    }
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
    if let Ok(resp) = ipc_call(&cfg, IPC_METHOD_STATUS, json!({})).await {
        if let Ok(t) = unwrap_ipc_result(resp) {
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

pub async fn run_sessions_list(config_path: &Path, limit: usize, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let list_val = match ipc_call(&cfg, IPC_METHOD_SESSIONS_LIST, json!({ "limit": limit })).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            serde_json::to_value(store.list_sessions(limit)?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&list_val)?),
        false => {
            let arr = list_val.as_array().cloned().unwrap_or_default();
            if arr.is_empty() {
                println!("No sessions found.");
            } else {
                for s in arr {
                    println!(
                        "{}  [{:<8}]  {:>6} tok  {}",
                        s["id"].as_str().unwrap_or("-"),
                        s["channel"].as_str().unwrap_or("-"),
                        s["total_tokens"].as_u64().unwrap_or(0),
                        s["title"].as_str().unwrap_or("-"),
                    );
                }
            }
        }
    }
    Ok(())
}

pub async fn run_session_show(config_path: &Path, id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_SESSION_GET, json!({ "session_id": id })).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let session = store
                .get_session(id)?
                .ok_or_else(|| anyhow::anyhow!("session {id} not found"))?;
            let events = store.get_active_branch(id)?;
            json!({ "session": session, "events": events })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            println!(
                "Session {} · {} ({})",
                data["session"]["id"].as_str().unwrap_or(id),
                data["session"]["title"].as_str().unwrap_or("-"),
                data["session"]["status"].as_str().unwrap_or("-"),
            );
            for ev in data["events"].as_array().into_iter().flatten() {
                let role = ev["role"].as_str().unwrap_or("?");
                let text = ev["content_json"]["text"]
                    .as_str()
                    .or_else(|| ev["content_json"]["content"].as_str())
                    .unwrap_or("");
                println!("\n[{role}] {text}");
            }
        }
    }
    Ok(())
}

pub async fn run_session_clear(config_path: &Path, id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let res = match ipc_call(&cfg, IPC_METHOD_SESSION_CLEAR, json!({ "session_id": id })).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            store.clear_session_events(id)?;
            json!({ "cleared": true })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&res)?),
        false => println!("Cleared session {id}."),
    }
    Ok(())
}

pub async fn run_session_delete(config_path: &Path, id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let res = match ipc_call(&cfg, IPC_METHOD_SESSION_DELETE, json!({ "session_id": id })).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            json!({ "deleted": store.delete_session(id)? })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&res)?),
        false => println!("Deleted session {id}."),
    }
    Ok(())
}

pub async fn run_approvals_list(config_path: &Path, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_APPROVALS_LIST, json!({})).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            serde_json::to_value(store.list_pending_approvals()?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let arr = data.as_array().cloned().unwrap_or_default();
            if arr.is_empty() {
                println!("No pending approvals.");
            } else {
                for a in arr {
                    println!(
                        "{}  [{}]  {}  {}",
                        a["id"].as_str().unwrap_or("-"),
                        a["risk"].as_str().unwrap_or("-"),
                        a["tool_name"].as_str().unwrap_or("-"),
                        a["args_preview"].as_str().unwrap_or(""),
                    );
                }
            }
        }
    }
    Ok(())
}

pub async fn run_approval_resolve(
    config_path: &Path,
    id: &str,
    decision: ApprovalDecision,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let dec_str = match decision {
        ApprovalDecision::Approve => "approve",
        ApprovalDecision::AlwaysAllow => "always_allow",
        ApprovalDecision::Deny => "deny",
    };
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_APPROVAL_RESOLVE,
        json!({ "id": id, "decision": dec_str }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let rec = store.resolve_approval(id, decision, "cli")?;
            json!({ "resolved": rec.is_some(), "approval": rec })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!("Approval {id}: {dec_str}"),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn run_sessions_search(
    config_path: &Path,
    query: &str,
    since: Option<&str>,
    channel: Option<&str>,
    workspace: &str,
    role: Option<&str>,
    limit: usize,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_SESSIONS_SEARCH,
        json!({
            "query": query,
            "since": since,
            "channel": channel,
            "workspace": workspace,
            "role": role,
            "limit": limit,
        }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let ws_dir = match workspace.trim() {
                "" | "all" => None,
                "current" => Some(
                    resolve_git_root(&cfg.resolved_workspace_dir())
                        .display()
                        .to_string(),
                ),
                other => Some(resolve_git_root(&PathBuf::from(other)).display().to_string()),
            };
            let filter = SessionSearchFilter {
                query: query.to_string(),
                since: since.and_then(parse_since_window),
                channel: channel.map(String::from),
                workspace_dir: ws_dir,
                session_id: None,
                exclude_session_id: None,
                role: role.map(String::from),
                limit: Some(limit),
            };
            serde_json::to_value(store.search_sessions_fts(&filter)?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let arr = data.as_array().cloned().unwrap_or_default();
            if arr.is_empty() {
                println!("No session transcript events matched `{query}`.");
            } else {
                for h in arr {
                    println!(
                        "{}  [{}|{}]  {}  — {}",
                        h["session_id"].as_str().unwrap_or("-"),
                        h["channel"].as_str().unwrap_or("-"),
                        h["role"].as_str().unwrap_or("-"),
                        h["session_title"].as_str().unwrap_or("-"),
                        h["snippet"].as_str().unwrap_or(""),
                    );
                }
            }
        }
    }
    Ok(())
}

pub async fn run_memory_search(
    config_path: &Path,
    query: &str,
    scope: &str,
    limit: usize,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_MEMORY_SEARCH,
        json!({ "query": query, "scope": scope, "limit": limit }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let ws = cfg.resolved_workspace_dir();
            let proj = project_scope_for_workspace(&ws);
            let (scope_opt, include_global) = match scope.trim().to_ascii_lowercase().as_str() {
                "global" => (None, false),
                "project" => (Some(proj.as_str()), false),
                "current" => (Some(proj.as_str()), true),
                _ => (None, true),
            };
            let hits = store.search_memory_hybrid(query, scope_opt, include_global, Some(&ws), limit)?;
            let entries: Vec<_> = hits.into_iter().map(|h| h.entry).collect();
            serde_json::to_value(entries)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let arr = data.as_array().cloned().unwrap_or_default();
            if arr.is_empty() {
                println!("No memory entries matched.");
            } else {
                for m in arr {
                    println!(
                        "{}  [{}|{}]  {}: {}",
                        m["id"].as_str().unwrap_or("-"),
                        m["scope"].as_str().unwrap_or("-"),
                        m["kind"].as_str().unwrap_or("-"),
                        m["title"].as_str().unwrap_or("-"),
                        m["content"].as_str().unwrap_or(""),
                    );
                }
            }
        }
    }
    Ok(())
}

pub async fn run_memory_add(
    config_path: &Path,
    title: &str,
    content: &str,
    kind: &str,
    scope: &str,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_MEMORY_ADD,
        json!({ "title": title, "content": content, "kind": kind, "scope": scope }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let resolved_scope = match scope.trim() {
                "project" => project_scope_for_workspace(&cfg.resolved_workspace_dir()),
                "" => MEMORY_SCOPE_GLOBAL.to_string(),
                other => other.to_string(),
            };
            let res = store.upsert_memory_dedup(
                &resolved_scope,
                kind,
                title,
                content,
                1.0,
                None,
                None,
                cfg.memory.dedup_enabled,
            )?;
            serde_json::to_value(res.entry)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Saved memory {} [{}] ({title}).",
            data["id"].as_str().unwrap_or("-"),
            data["scope"].as_str().unwrap_or(scope),
        ),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn run_memory_update(
    config_path: &Path,
    id: &str,
    title: Option<&str>,
    content: Option<&str>,
    kind: Option<&str>,
    scope: Option<&str>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let resolved_scope = scope.map(|sc| match sc.trim() {
        "project" => project_scope_for_workspace(&cfg.resolved_workspace_dir()),
        "" => MEMORY_SCOPE_GLOBAL.to_string(),
        other => other.to_string(),
    });
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_MEMORY_UPDATE,
        json!({
            "id": id,
            "title": title,
            "content": content,
            "kind": kind,
            "scope": resolved_scope,
        }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let entry = store
                .update_memory(id, title, content, kind, None, resolved_scope.as_deref())?
                .ok_or_else(|| anyhow::anyhow!("memory `{id}` not found"))?;
            serde_json::to_value(entry)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!("Updated memory {id}."),
    }
    Ok(())
}

pub async fn run_memory_delete(config_path: &Path, id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_MEMORY_DELETE, json!({ "id": id })).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            json!({ "deleted": store.delete_memory(id)? })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!("Deleted memory {id}."),
    }
    Ok(())
}

pub async fn run_memory_dream(config_path: &Path, force: bool, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call_timeout(
        &cfg,
        IPC_METHOD_MEMORY_DREAM,
        json!({ "force": force, "trigger": "cli" }),
        Some(Duration::from_secs(180)),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _logs) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let rec = rt.run_dream_now("cli", force).await?;
            serde_json::to_value(rec)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Dream [{}] model={} sessions={} added={} merged={} archived={} docs={} ({})",
            data["status"].as_str().unwrap_or("-"),
            data["model"].as_str().unwrap_or("none"),
            data["sessions_scanned"].as_u64().unwrap_or(0),
            data["memories_added"].as_u64().unwrap_or(0),
            data["memories_merged"].as_u64().unwrap_or(0),
            data["memories_archived"].as_u64().unwrap_or(0),
            data["docs_patched"].as_u64().unwrap_or(0),
            data["detail"].as_str().unwrap_or(""),
        ),
    }
    Ok(())
}

pub async fn run_memory_history(
    config_path: &Path,
    doc: Option<&str>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_MEMORY_HISTORY, json!({ "doc": doc })).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let loader = SnapshotManager::new(cfg.resolved_data_dir(), cfg.resolved_workspace_dir());
            serde_json::to_value(loader.list_doc_history(doc)?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let arr = data.as_array().cloned().unwrap_or_default();
            if arr.is_empty() {
                println!("No memory doc snapshots in .history/.");
            } else {
                for s in arr {
                    println!(
                        "{}  {}  [{}]  {} bytes  {}",
                        s["id"].as_str().unwrap_or("-"),
                        s["doc_name"].as_str().unwrap_or("-"),
                        s["reason"].as_str().unwrap_or("-"),
                        s["bytes"].as_u64().unwrap_or(0),
                        s["created_at"].as_str().unwrap_or("-"),
                    );
                }
            }
        }
    }
    Ok(())
}

pub async fn run_memory_rollback(
    config_path: &Path,
    snapshot_id: &str,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_MEMORY_ROLLBACK,
        json!({ "snapshot_id": snapshot_id }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let loader = SnapshotManager::new(cfg.resolved_data_dir(), cfg.resolved_workspace_dir());
            let (doc, diff) = loader.rollback_doc(snapshot_id)?;
            json!({ "doc": doc, "diff": diff })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            println!(
                "Restored {} from `{snapshot_id}`.",
                data["doc"].as_str().unwrap_or("doc")
            );
            if let Some(d) = data["diff"].as_str().filter(|s| !s.is_empty()) {
                println!("{d}");
            }
        }
    }
    Ok(())
}

pub async fn run_cron_list(config_path: &Path, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_CRON_LIST, json!({})).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            serde_json::to_value(store.list_cron_jobs()?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let arr = data.as_array().cloned().unwrap_or_default();
            if arr.is_empty() {
                println!("No scheduled cron jobs.");
            } else {
                for j in arr {
                    let target = j["script_path"]
                        .as_str()
                        .map(|s| format!("script:{s}"))
                        .unwrap_or_else(|| j["prompt"].as_str().unwrap_or("").to_string());
                    let state_tag = match j["enabled"].as_bool().unwrap_or(true) {
                        true => "on ",
                        false => "off",
                    };
                    println!(
                        "{}  [{:<9}] [{}]  `{}`  {}  -> {}",
                        j["id"].as_str().unwrap_or("-"),
                        j["kind"].as_str().unwrap_or("-"),
                        state_tag,
                        j["cron_expr"].as_str().unwrap_or("-"),
                        j["name"].as_str().unwrap_or("-"),
                        target,
                    );
                }
            }
        }
    }
    Ok(())
}

pub async fn run_cron_add(
    config_path: &Path,
    name: &str,
    expr: &str,
    prompt: Option<&str>,
    script: Option<&str>,
    kind: &str,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let kind_enum = CronKind::from_str_loose(kind);
    let script_path = script.map(str::trim).filter(|s| !s.is_empty());
    let prompt_str = prompt.unwrap_or("");
    if kind_enum == CronKind::Script && script_path.is_none() {
        anyhow::bail!("--script <filename> inside ~/zenko_hub/scripts/ is required for --kind script");
    }
    if kind_enum != CronKind::Script && prompt_str.trim().is_empty() {
        anyhow::bail!("--prompt is required for cron/heartbeat jobs");
    }
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_CRON_CREATE,
        json!({
            "name": name,
            "cron_expr": expr,
            "prompt": prompt_str,
            "script_path": script_path,
            "kind": kind_enum.as_str(),
        }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let parsed = Cron::new(expr)
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid cron expression: {e}"))?;
            let store = SqliteStore::open(cfg.db_path())?;
            let job = CronJob {
                id: new_id(),
                name: name.to_string(),
                cron_expr: expr.to_string(),
                kind: kind_enum,
                prompt: prompt_str.to_string(),
                script_path: script_path.map(String::from),
                target_channel: "cli".into(),
                target_chat_id: None,
                target_thread_id: None,
                enabled: true,
                next_run_at: parsed.find_next_occurrence(&Utc::now(), false).ok(),
                last_run_at: None,
                last_status: None,
            };
            store.upsert_cron_job(&job)?;
            serde_json::to_value(job)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Scheduled {} '{}' (id={}).",
            kind_enum.as_str(),
            name,
            data["id"].as_str().unwrap_or("-")
        ),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn run_cron_update(
    config_path: &Path,
    id: &str,
    name: Option<&str>,
    expr: Option<&str>,
    prompt: Option<&str>,
    script: Option<&str>,
    enabled: Option<bool>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_CRON_UPDATE,
        json!({
            "id": id,
            "name": name,
            "cron_expr": expr,
            "prompt": prompt,
            "script_path": script,
            "enabled": enabled,
        }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let mut job = store
                .get_cron_job(id)?
                .ok_or_else(|| anyhow::anyhow!("cron job `{id}` not found"))?;
            if let Some(n) = name {
                job.name = n.to_string();
            }
            if let Some(e) = expr {
                let parsed = Cron::new(e)
                    .parse()
                    .map_err(|err| anyhow::anyhow!("invalid cron expression: {err}"))?;
                job.cron_expr = e.to_string();
                job.next_run_at = parsed.find_next_occurrence(&Utc::now(), false).ok();
            }
            if let Some(p) = prompt {
                job.prompt = p.to_string();
            }
            if let Some(s) = script {
                job.script_path = Some(s.to_string());
            }
            if let Some(en) = enabled {
                job.enabled = en;
            }
            store.upsert_cron_job(&job)?;
            serde_json::to_value(job)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Updated cron job {} (enabled={}).",
            id,
            data["enabled"].as_bool().unwrap_or(true)
        ),
    }
    Ok(())
}

pub async fn run_cron_run_now(config_path: &Path, id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let resp = ipc_call(&cfg, IPC_METHOD_CRON_RUN_NOW, json!({ "id": id }))
        .await
        .context("triggering a cron job immediately requires a running daemon (`zenko web`)")?;
    let data = unwrap_ipc_result(resp)?;
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Dispatched cron job {id} (mode={}).",
            data["dispatch"]["mode"].as_str().unwrap_or("-")
        ),
    }
    Ok(())
}

pub async fn run_cron_delete(config_path: &Path, id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_CRON_DELETE, json!({ "id": id })).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            json!({ "deleted": store.delete_cron_job(id)? })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!("Deleted cron job {id}."),
    }
    Ok(())
}

pub struct LogsParams {
    pub limit: usize,
    pub category: Option<String>,
    pub channel: Option<String>,
    pub level: Option<String>,
    pub session: Option<String>,
    pub search: Option<String>,
    pub follow: bool,
    pub json_out: bool,
}

fn print_log_entry_line(entry: &LogEntry, json_out: bool) {
    if json_out {
        if let Ok(line) = serde_json::to_string(entry) {
            println!("{line}");
        }
        return;
    }
    let ts = entry.timestamp.format("%H:%M:%S");
    let sid_tag = entry
        .session_id
        .as_deref()
        .map(|s| format!(" [{}]", zenko_core::truncate_utf8_prefix(s, 8)))
        .unwrap_or_default();
    let cols: Vec<&str> = entry.message.split('\t').collect();
    match cols.len() {
        5 => println!(
            "{ts} {:<7} {:>3} {:<7}{} {} {} {}",
            entry.category,
            cols[0].trim(),
            cols[1],
            sid_tag,
            cols[2],
            cols[3],
            cols[4]
        ),
        _ => println!(
            "{ts} {:<7} [{}]{} {}: {}",
            entry.category, entry.level, sid_tag, entry.target, entry.message
        ),
    }
}

pub async fn run_logs(config_path: &Path, params: LogsParams) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let filter = LogFilter {
        category: params.category,
        channel: params.channel,
        level: params.level,
        session_id: params.session,
        search: params.search,
        limit: Some(params.limit),
    };

    if params.follow {
        return ipc_follow_logs(&cfg, &filter, |entry| {
            print_log_entry_line(&entry, params.json_out);
        })
        .await
        .context("live log follow (`-f`) requires a running daemon (`zenko web`)");
    }

    let entries: Vec<LogEntry> = match ipc_call(
        &cfg,
        IPC_METHOD_LOGS_QUERY,
        serde_json::to_value(&filter)?,
    )
    .await
    {
        Ok(resp) => {
            let val = unwrap_ipc_result(resp)?;
            serde_json::from_value(val).unwrap_or_default()
        }
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            store.query_logs(&filter)?
        }
    };

    match params.json_out {
        true => println!("{}", serde_json::to_string_pretty(&entries)?),
        false => {
            if entries.is_empty() {
                println!("No persisted log entries matched.");
            } else {
                for entry in &entries {
                    print_log_entry_line(entry, false);
                }
            }
        }
    }
    Ok(())
}

pub async fn run_jobs_list(
    config_path: &Path,
    session: Option<&str>,
    kind: Option<&str>,
    limit: usize,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let kind_filter = kind.and_then(TaskKind::parse_filter);
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_JOBS_LIST,
        json!({
            "session_id": session,
            "kind": kind_filter.map(TaskKind::as_str),
            "limit": limit
        }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            serde_json::to_value(store.list_jobs(session, kind_filter, limit)?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let arr = data.as_array().cloned().unwrap_or_default();
            if arr.is_empty() {
                println!("No background tasks found.");
            } else {
                for j in arr {
                    let exit_tag = j["exit_code"]
                        .as_i64()
                        .map(|c| format!(" exit={c}"))
                        .unwrap_or_default();
                    let pid_tag = j["pid"]
                        .as_u64()
                        .map(|p| format!(" pid={p}"))
                        .unwrap_or_default();
                    let label_tag = j["label"]
                        .as_str()
                        .map(|l| format!(" ({l})"))
                        .unwrap_or_default();
                    println!(
                        "{}  [{:<8}][{:<11}]{}{}{}  {:>7}B  {}",
                        j["id"].as_str().unwrap_or("-"),
                        j["kind"].as_str().unwrap_or("shell"),
                        j["status"].as_str().unwrap_or("-"),
                        label_tag,
                        pid_tag,
                        exit_tag,
                        j["bytes_written"].as_u64().unwrap_or(0),
                        j["command"].as_str().unwrap_or("-"),
                    );
                }
            }
        }
    }
    Ok(())
}

pub async fn run_jobs_start(
    config_path: &Path,
    command: &[String],
    kind: Option<&str>,
    label: Option<&str>,
    workspace: Option<PathBuf>,
    json_out: bool,
) -> Result<()> {
    let cmd_str = command.join(" ");
    if cmd_str.trim().is_empty() {
        anyhow::bail!("provide a command, e.g. `zenko tasks start --kind dev npm run dev`");
    }
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let resp = ipc_call(
        &cfg,
        IPC_METHOD_JOBS_START,
        json!({
            "command": cmd_str,
            "kind": kind,
            "label": label,
            "workspace": workspace.map(|p| p.display().to_string()),
        }),
    )
    .await
    .context("starting a managed background task requires a running daemon (`zenko web`)")?;
    let data = unwrap_ipc_result(resp)?;
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Started task {} [{}] (pid={}) -> {}",
            data["id"].as_str().unwrap_or("-"),
            data["kind"].as_str().unwrap_or("shell"),
            data["pid"].as_u64().map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
            data["log_path"].as_str().unwrap_or("-"),
        ),
    }
    Ok(())
}

pub async fn run_jobs_update(
    config_path: &Path,
    job_id: &str,
    label: Option<&str>,
    kind: Option<&str>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let kind_enum = kind.and_then(TaskKind::parse_filter);
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_JOBS_UPDATE,
        json!({ "job_id": job_id, "label": label, "kind": kind_enum.map(TaskKind::as_str) }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let rec = store
                .update_job_meta(job_id, label.map(String::from), kind_enum)?
                .ok_or_else(|| anyhow::anyhow!("task `{job_id}` not found"))?;
            serde_json::to_value(rec)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Updated task {} [{}] label={:?}.",
            job_id,
            data["kind"].as_str().unwrap_or("shell"),
            data["label"].as_str()
        ),
    }
    Ok(())
}

pub async fn run_jobs_restart(config_path: &Path, job_id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let resp = ipc_call(&cfg, IPC_METHOD_JOBS_RESTART, json!({ "job_id": job_id }))
        .await
        .context("restarting a task requires a running daemon (`zenko web`)")?;
    let data = unwrap_ipc_result(resp)?;
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Restarted task {job_id} as {} [{}].",
            data["id"].as_str().unwrap_or("-"),
            data["kind"].as_str().unwrap_or("shell")
        ),
    }
    Ok(())
}

pub async fn run_jobs_delete(config_path: &Path, job_id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_JOBS_DELETE, json!({ "job_id": job_id })).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let removed = store.delete_job(job_id)?;
            if let Some(ref rec) = removed {
                let _ = std::fs::remove_file(&rec.log_path);
            }
            json!({ "deleted": removed.is_some(), "job_id": job_id })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!("Deleted task {job_id}."),
    }
    Ok(())
}

pub async fn run_jobs_prune(
    config_path: &Path,
    kind: Option<&str>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let kind_filter = kind.and_then(TaskKind::parse_filter);
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_JOBS_PRUNE,
        json!({ "kind": kind_filter.map(TaskKind::as_str) }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let removed = store.prune_finished_jobs(kind_filter)?;
            for rec in &removed {
                let _ = std::fs::remove_file(&rec.log_path);
            }
            json!({ "pruned": removed.len() })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!("Pruned {} finished task(s).", data["pruned"].as_u64().unwrap_or(0)),
    }
    Ok(())
}

pub async fn run_hub_list(
    config_path: &Path,
    category: Option<&str>,
    limit: usize,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let hub_root = cfg.ensure_hub_layout()?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_HUB_LIST,
        json!({ "category": category, "limit": limit }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let entries = cfg.list_hub_entries(category, limit)?;
            json!({
                "hub_dir": hub_root.display().to_string(),
                "categories": HUB_CATEGORIES,
                "entries": entries,
            })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            println!("Zenko Hub: {}", data["hub_dir"].as_str().unwrap_or("-"));
            let entries = data["entries"].as_array().cloned().unwrap_or_default();
            if entries.is_empty() {
                println!("  (empty — standalone projects, scripts, media, and artifacts will be stored here)");
            } else {
                for e in entries {
                    let kind_tag = match e["is_dir"].as_bool().unwrap_or(false) {
                        true => "dir ",
                        false => "file",
                    };
                    println!(
                        "  [{:<9}] {kind_tag}  {:>8}B  {}",
                        e["category"].as_str().unwrap_or("-"),
                        e["size_bytes"].as_u64().unwrap_or(0),
                        e["rel_path"].as_str().unwrap_or("-"),
                    );
                }
            }
        }
    }
    Ok(())
}

pub fn run_hub_path(config_path: &Path, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let hub_root = cfg.ensure_hub_layout()?;
    match json_out {
        true => println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "hub_dir": hub_root.display().to_string(),
                "categories": HUB_CATEGORIES,
            }))?
        ),
        false => println!("{}", hub_root.display()),
    }
    Ok(())
}

pub async fn run_queue_list(
    config_path: &Path,
    session_id: &str,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_SESSION_QUEUE_LIST,
        json!({ "session_id": session_id }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            serde_json::to_value(store.list_queued_followups(session_id)?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let arr = data.as_array().cloned().unwrap_or_default();
            if arr.is_empty() {
                println!("No queued follow-up prompts for session {session_id}.");
            } else {
                for (i, q) in arr.iter().enumerate() {
                    println!(
                        "#{}  {}  {}",
                        i + 1,
                        q["id"].as_str().unwrap_or("-"),
                        q["text"].as_str().unwrap_or("")
                    );
                }
            }
        }
    }
    Ok(())
}

pub async fn run_queue_cancel(
    config_path: &Path,
    session_id: &str,
    queue_id: &str,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_SESSION_QUEUE_CANCEL,
        json!({ "session_id": session_id, "queue_id": queue_id }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let ok = store.cancel_queued_followup(session_id, queue_id)?;
            json!({ "cancelled": ok })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!("Cancelled queued prompt {queue_id}."),
    }
    Ok(())
}

pub async fn run_jobs_read(
    config_path: &Path,
    job_id: &str,
    offset: Option<u64>,
    max_bytes: usize,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_JOBS_READ,
        json!({ "job_id": job_id, "offset": offset, "max_bytes": max_bytes }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let job = store
                .get_job(job_id)?
                .ok_or_else(|| anyhow::anyhow!("job `{job_id}` not found"))?;
            serde_json::to_value(zenko_runtime::read_job_log_slice_sync(
                job, offset, max_bytes,
            )?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            println!(
                "Job {} [{}] · bytes {}..{} of {}",
                data["job"]["id"].as_str().unwrap_or(job_id),
                data["job"]["status"].as_str().unwrap_or("-"),
                data["offset"].as_u64().unwrap_or(0),
                data["next_offset"].as_u64().unwrap_or(0),
                data["total_bytes"].as_u64().unwrap_or(0),
            );
            print!("{}", data["content"].as_str().unwrap_or(""));
        }
    }
    Ok(())
}

pub async fn run_jobs_input(
    config_path: &Path,
    job_id: &str,
    input: &str,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let resp = ipc_call(
        &cfg,
        IPC_METHOD_JOBS_INPUT,
        json!({ "job_id": job_id, "input": input }),
    )
    .await
    .context("sending stdin to a live background job requires a running daemon")?;
    let data = unwrap_ipc_result(resp)?;
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!("Sent {} bytes to stdin of job {job_id}.", input.len()),
    }
    Ok(())
}

pub async fn run_jobs_stop(config_path: &Path, job_id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let resp = ipc_call(&cfg, IPC_METHOD_JOBS_STOP, json!({ "job_id": job_id }))
        .await
        .context("stopping a live background job requires a running daemon")?;
    let data = unwrap_ipc_result(resp)?;
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!("Stopped job {job_id}."),
    }
    Ok(())
}

pub async fn run_jobs_wait(
    config_path: &Path,
    job_id: &str,
    timeout_secs: Option<u64>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let wait_s = timeout_secs.unwrap_or(JOB_DEFAULT_WAIT_SECS);
    let data = match ipc_call_timeout(
        &cfg,
        IPC_METHOD_JOBS_WAIT,
        json!({ "job_id": job_id, "timeout_secs": wait_s }),
        Some(Duration::from_secs(
            wait_s.saturating_add(IPC_WAIT_TIMEOUT_SLACK_SECS),
        )),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let job = store
                .get_job(job_id)?
                .ok_or_else(|| anyhow::anyhow!("job `{job_id}` not found"))?;
            let output = zenko_runtime::read_job_log_slice_sync(
                job.clone(),
                None,
                JOB_DEFAULT_READ_BYTES,
            )
            .ok();
            json!({ "job": job, "output": output })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let exit_tag = data["job"]["exit_code"]
                .as_i64()
                .map(|c| format!(" exit={c}"))
                .unwrap_or_default();
            println!(
                "Job {} [{}]{exit_tag} · {}B",
                data["job"]["id"].as_str().unwrap_or(job_id),
                data["job"]["status"].as_str().unwrap_or("-"),
                data["job"]["bytes_written"].as_u64().unwrap_or(0),
            );
            if let Some(content) = data["output"]["content"].as_str() {
                print!("{content}");
            }
        }
    }
    Ok(())
}

pub async fn run_jobs_follow(config_path: &Path, job_id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let mut saw_done = false;

    let stream_res = ipc_follow_job(&cfg, job_id, 0, |frame| {
        if json_out {
            if let Ok(line) = serde_json::to_string(&frame) {
                println!("{line}");
            }
            if frame.get("kind").and_then(Value::as_str) == Some(IPC_FRAME_DONE) {
                saw_done = true;
            }
            return;
        }
        match frame.get("kind").and_then(Value::as_str) {
            Some(IPC_FRAME_CHUNK) => {
                if let Some(content) = frame["data"]["content"].as_str() {
                    let mut out = std::io::stdout();
                    let _ = write!(out, "{content}");
                    let _ = out.flush();
                }
            }
            Some(IPC_FRAME_DONE) => {
                saw_done = true;
                let status = frame["job"]["status"].as_str().unwrap_or("completed");
                let exit_tag = frame["job"]["exit_code"]
                    .as_i64()
                    .map(|c| format!(" exit={c}"))
                    .unwrap_or_default();
                eprintln!("\n[job {job_id} finished: {status}{exit_tag}]");
            }
            _ => {}
        }
    })
    .await;

    match stream_res {
        Ok(()) => {
            if !saw_done && !json_out {
                eprintln!("\n[detached from job {job_id} — still running]");
            }
            Ok(())
        }
        Err(_) => {
            // Offline fallback when daemon is stopped: print full persisted log slice once.
            let store = SqliteStore::open(cfg.db_path())?;
            let job = store
                .get_job(job_id)?
                .ok_or_else(|| anyhow::anyhow!("job `{job_id}` not found"))?;
            let chunk = zenko_runtime::read_job_log_slice_sync(job, Some(0), JOB_DEFAULT_READ_BYTES)?;
            match json_out {
                true => println!(
                    "{}",
                    serde_json::to_string(&json!({
                        "ok": true,
                        "kind": IPC_FRAME_CHUNK,
                        "data": chunk,
                    }))?
                ),
                false => print!("{}", chunk.content),
            }
            Ok(())
        }
    }
}

pub async fn run_session_background(
    config_path: &Path,
    session_id: &str,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let resp = ipc_call(
        &cfg,
        IPC_METHOD_SESSION_BACKGROUND,
        json!({ "session_id": session_id }),
    )
    .await
    .context("demoting a live foreground command requires a running daemon (`zenko web`)")?;
    let data = unwrap_ipc_result(resp)?;
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => match data["demoted"].as_bool().unwrap_or(false) {
            true => println!("Signaled active foreground command in session {session_id} to move to background."),
            false => println!("Session {session_id} has no active foreground turn to demote."),
        },
    }
    Ok(())
}

pub async fn run_attach(
    config_path: &Path,
    session_id: &str,
    replay: usize,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    if !json_out {
        eprintln!(
            "Attached to session `{session_id}` on `zenko web` daemon (Ctrl+C detaches safely without stopping)."
        );
    }
    ipc_attach_session(&cfg, session_id, replay, |frame| {
        if json_out {
            if let Ok(line) = serde_json::to_string(&frame) {
                println!("{line}");
            }
            return;
        }
        match frame.get("kind").and_then(Value::as_str) {
            Some(IPC_FRAME_SNAPSHOT) => {
                let title = frame["session"]["title"].as_str().unwrap_or("-");
                let status = frame["session"]["status"].as_str().unwrap_or("-");
                let queued = frame["queued_prompts"]
                    .as_array()
                    .map(|a| a.len())
                    .unwrap_or(0);
                let active_jobs = frame["jobs"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter(|j| {
                                j["status"]
                                    .as_str()
                                    .map(|s| !BackgroundJobStatus::from_str_loose(s).is_terminal())
                                    .unwrap_or(false)
                            })
                            .count()
                    })
                    .unwrap_or(0);
                eprintln!("Session: {title} [{status}] · queued={queued} · active_jobs={active_jobs}");
                for ev in frame["events"].as_array().into_iter().flatten() {
                    let role = ev["role"].as_str().unwrap_or("?");
                    let text = ev["content_json"]["text"]
                        .as_str()
                        .or_else(|| ev["content_json"]["content"].as_str())
                        .unwrap_or("");
                    if !text.trim().is_empty() {
                        println!("[{role}] {text}");
                    }
                }
            }
            Some(IPC_FRAME_EVENT) => {
                let data = &frame["data"];
                match data.get("event").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(t) = data.get("text").and_then(Value::as_str) {
                            print!("{t}");
                            let _ = std::io::stdout().flush();
                        }
                    }
                    Some("tool_call_started") => {
                        eprintln!(
                            "\n  --> tool {}  {}",
                            data["tool_name"].as_str().unwrap_or("?"),
                            data["args_preview"].as_str().unwrap_or("")
                        );
                    }
                    Some("tool_call_finished") => {
                        let tag = match data["is_error"].as_bool().unwrap_or(false) {
                            true => "err",
                            false => "ok",
                        };
                        eprintln!(
                            "  <-- tool {} {tag} {}ms",
                            data["tool_name"].as_str().unwrap_or("?"),
                            data["duration_ms"].as_u64().unwrap_or(0)
                        );
                    }
                    Some("job_updated") => {
                        eprintln!(
                            "  ··· job {} [{}]  {}",
                            data["job"]["id"].as_str().unwrap_or("?"),
                            data["job"]["status"].as_str().unwrap_or("?"),
                            data["job"]["command"].as_str().unwrap_or("")
                        );
                    }
                    Some("steering_accepted") => {
                        eprintln!("  [steer] {}", data["preview"].as_str().unwrap_or(""));
                    }
                    Some("queue_updated") => {
                        eprintln!(
                            "  [queue] {} pending follow-up(s)",
                            data["queued_count"].as_u64().unwrap_or(0)
                        );
                    }
                    Some("orchestrator_updated") => {
                        eprintln!(
                            "  [orch:{}] {} · {}",
                            data["tier"].as_str().unwrap_or("standard"),
                            data["stage"].as_str().unwrap_or("-"),
                            data["detail"].as_str().unwrap_or("")
                        );
                    }
                    Some("brainstorm_recorded") => {
                        eprintln!(
                            "  [plan] brainstorm selected `{}`",
                            data["spec"]["selected_option_id"].as_str().unwrap_or("-")
                        );
                    }
                    Some("plan_updated") => {
                        let steps = data["plan"]["steps"].as_array();
                        let total = steps.map(|a| a.len()).unwrap_or(0);
                        let done = steps
                            .map(|a| {
                                a.iter()
                                    .filter(|s| s["status"].as_str() == Some("completed"))
                                    .count()
                            })
                            .unwrap_or(0);
                        eprintln!(
                            "  [plan] steps {done}/{total} · {}",
                            data["plan"]["goal"].as_str().unwrap_or("-")
                        );
                    }
                    Some("clarification_requested") => {
                        eprintln!(
                            "  [clarify] {} (id={})",
                            data["clarification"]["question"].as_str().unwrap_or("?"),
                            data["clarification"]["id"].as_str().unwrap_or("-")
                        );
                    }
                    Some("turn_completed") => println!(),
                    Some("turn_error") => {
                        eprintln!("\n  [error] {}", data["message"].as_str().unwrap_or(""));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    })
    .await
    .context("attaching to a session requires a running `zenko web` daemon")?;

    if !json_out {
        eprintln!("\nDetached from session `{session_id}`.");
    }
    Ok(())
}

pub async fn run_session_steer(
    config_path: &Path,
    session_id: &str,
    message: &[String],
    json_out: bool,
) -> Result<()> {
    let text = message.join(" ");
    if text.trim().is_empty() {
        anyhow::bail!("provide steering text, e.g. `zenko steer <session_id> \"focus on tests\"`");
    }
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let resp = ipc_call(
        &cfg,
        IPC_METHOD_SESSION_STEER,
        json!({ "session_id": session_id, "text": text }),
    )
    .await
    .context("steering an active turn requires a running daemon")?;
    let data = unwrap_ipc_result(resp)?;
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => match data["steered"].as_bool().unwrap_or(false) {
            true => println!("Steering guidance queued for active session {session_id}."),
            false => println!(
                "Session {session_id} is currently idle — use `zenko ask -s {session_id} ...` to start a turn."
            ),
        },
    }
    Ok(())
}

pub async fn run_clarifications_list(
    config_path: &Path,
    session: Option<&str>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let resp = ipc_call(
        &cfg,
        IPC_METHOD_CLARIFICATIONS_LIST,
        json!({ "session_id": session }),
    )
    .await
    .context("listing live clarifications requires a running daemon (`zenko web`)")?;
    let data = unwrap_ipc_result(resp)?;
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let arr = data.as_array().cloned().unwrap_or_default();
            if arr.is_empty() {
                println!("No pending clarifications.");
            } else {
                for c in arr {
                    println!(
                        "{}  [rec={}]  {}",
                        c["id"].as_str().unwrap_or("-"),
                        c["recommended_option_id"].as_str().unwrap_or("-"),
                        c["question"].as_str().unwrap_or("")
                    );
                }
            }
        }
    }
    Ok(())
}

pub async fn run_clarification_answer(
    config_path: &Path,
    id: &str,
    option_id: &str,
    custom: Option<&str>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let resp = ipc_call(
        &cfg,
        IPC_METHOD_CLARIFICATION_RESOLVE,
        json!({
            "id": id,
            "selected_option_id": option_id,
            "custom_answer": custom,
            "resolved_by": "cli",
        }),
    )
    .await
    .context("answering a clarification requires a running daemon (`zenko web`)")?;
    let data = unwrap_ipc_result(resp)?;
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!("Clarification {id} answered with `{option_id}`."),
    }
    Ok(())
}

pub fn default_job_read_bytes() -> usize {
    JOB_DEFAULT_READ_BYTES
}

pub async fn run_gateway_pair(config_path: &Path, role: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let store = SqliteStore::open(cfg.db_path())?;
    let role_enum = PeerRole::from_str_loose(role);
    let pair = store.create_pairing_code("telegram", role_enum, DEFAULT_PAIRING_TTL_MINS)?;
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&pair)?),
        false => {
            println!(
                "Telegram Pairing Code ({} — expires in {DEFAULT_PAIRING_TTL_MINS} min):",
                pair.role.as_str()
            );
            println!("\n  /pair {}\n", pair.code);
            println!("Send the command above to your Telegram bot.");
        }
    }
    Ok(())
}

pub async fn run_gateway_peers(config_path: &Path, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let store = SqliteStore::open(cfg.db_path())?;
    let peers = store.list_peers(Some("telegram"))?;
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&peers)?),
        false => {
            if peers.is_empty() {
                println!("No paired Telegram accounts yet. Run `zenko gateway pair`.");
            } else {
                for p in peers {
                    println!(
                        "- {} (@{}) id={} role={}",
                        p.display_name.as_deref().unwrap_or("User"),
                        p.username.as_deref().unwrap_or("-"),
                        p.peer_id,
                        p.role.as_str()
                    );
                }
            }
        }
    }
    Ok(())
}

pub fn run_web_info(config_path: &Path, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let vault = SecretVault::open(cfg.vault_path(), cfg.vault_key_path());
    let url = format!("http://{}:{}", cfg.webui.bind, cfg.webui.port);
    let token = match cfg.webui.require_auth {
        true => vault.get(VAULT_KEY_WEBUI_ADMIN)?.map(|z| z.to_string()),
        false => None,
    };
    match json_out {
        true => println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "url": url,
                "require_auth": cfg.webui.require_auth,
                "admin_token": token,
            }))?
        ),
        false => {
            println!("Zenko Pure-Rust WebUI: {url}");
            match (cfg.webui.require_auth, token) {
                (true, Some(tok)) => println!("Admin Token: {tok}"),
                (true, None) => println!("No admin token set yet — run `zenko setup` or start `zenko web`."),
                (false, _) => println!("Auth: disabled (local bind only)"),
            }
        }
    }
    Ok(())
}

pub async fn run_session_context(
    config_path: &Path,
    session_id: &str,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_SESSION_CONTEXT,
        json!({ "session_id": session_id }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            serde_json::to_value(rt.inspect_session_context(session_id).await?)?
        }
    };

    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let total = data["total_tokens"].as_u64().unwrap_or(0);
            let window = data["context_window"].as_u64().unwrap_or(0);
            let pct = data["utilization_pct"].as_f64().unwrap_or(0.0);
            let model = data["model"].as_str().unwrap_or("-");
            let t1 = data["tier1_tokens"].as_u64().unwrap_or(0);
            let t2 = data["tier2_tokens"].as_u64().unwrap_or(0);
            let t1_hash = data["tier1_hash"].as_str().unwrap_or("-");
            let tools_tok = data["tool_schema_tokens"].as_u64().unwrap_or(0);
            let eager = data["eager_tools_count"].as_u64().unwrap_or(0);
            let def_act = data["deferred_active_count"].as_u64().unwrap_or(0);
            let def_avail = data["deferred_available_count"].as_u64().unwrap_or(0);
            let hist_tok = data["history_tokens"].as_u64().unwrap_or(0);
            let mc_cnt = data["microcompacted_tools_count"].as_u64().unwrap_or(0);
            let mc_saved = data["microcompacted_tokens_saved"].as_u64().unwrap_or(0);
            let ckpts = data["checkpoints_count"].as_u64().unwrap_or(0);
            let ckpt_id = data["active_checkpoint_id"].as_str().unwrap_or("none");
            let artifacts = data["spilled_artifacts_count"].as_u64().unwrap_or(0);
            let cache_pct = data["cache_hit_pct"].as_f64().unwrap_or(0.0);
            let rules: Vec<&str> = data["repo_rules_loaded"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();

            println!("Zenko Context Breakdown · {session_id}");
            println!(
                "  Model:        {model} ({} / {} · {pct:.1}%)",
                format_tokens(total),
                format_tokens(window)
            );
            println!(
                "  Tier 1 (KV):  {} tok (hash={})",
                format_tokens(t1),
                zenko_core::truncate_utf8_prefix(&t1_hash, 12)
            );
            println!("  Tier 2:       {} tok", format_tokens(t2));
            println!(
                "  Repo Rules:   {}",
                match rules.is_empty() {
                    true => "none".into(),
                    false => rules.join(", "),
                }
            );
            println!(
                "  Tool Schemas: {} tok ({eager} eager, {def_act}/{def_avail} deferred active)",
                format_tokens(tools_tok)
            );
            println!(
                "  History:      {} tok (microcompacted {mc_cnt} tool(s), saved ~{} tok)",
                format_tokens(hist_tok),
                format_tokens(mc_saved)
            );
            println!("  Checkpoints:  {ckpts} (active={ckpt_id})");
            println!("  Artifacts:    {artifacts} spilled file(s)");
            println!("  KV Cache Hit: {cache_pct:.1}%");
        }
    }
    Ok(())
}

// ─── Phase 5: Async Subagents, Git Worktrees & Mailbox CLI ────────────────────

pub async fn run_agents_list(
    config_path: &Path,
    parent: Option<&str>,
    limit: usize,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_AGENTS_LIST,
        json!({ "parent_session_id": parent, "limit": limit }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            serde_json::to_value(store.list_subagents(parent, limit)?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let arr = data.as_array().cloned().unwrap_or_default();
            if arr.is_empty() {
                println!("No subagents found.");
            } else {
                for a in arr {
                    let branch = a["worktree_branch"].as_str().unwrap_or("-");
                    let files = a["changed_files"]
                        .as_array()
                        .map(|v| v.len())
                        .unwrap_or(0);
                    println!(
                        "{}  [{:<9}][{:<9}] d={} {:>6} tok  branch={} ({} files)  {}",
                        a["id"].as_str().unwrap_or("-"),
                        a["role"].as_str().unwrap_or("-"),
                        a["status"].as_str().unwrap_or("-"),
                        a["depth"].as_u64().unwrap_or(1),
                        format_tokens(a["tokens_used"].as_u64().unwrap_or(0)),
                        branch,
                        files,
                        a["goal"].as_str().unwrap_or("-"),
                    );
                }
            }
        }
    }
    Ok(())
}

pub async fn run_agents_show(config_path: &Path, id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_AGENTS_GET, json!({ "agent_id": id })).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let rec = store
                .get_subagent(id)?
                .ok_or_else(|| anyhow::anyhow!("subagent `{id}` not found"))?;
            let messages = store.list_agent_messages(id, 40).unwrap_or_default();
            json!({ "agent": rec, "messages": messages })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let a = &data["agent"];
            println!(
                "Subagent {} [{}] · status={} · depth={}",
                a["id"].as_str().unwrap_or(id),
                a["role"].as_str().unwrap_or("-"),
                a["status"].as_str().unwrap_or("-"),
                a["depth"].as_u64().unwrap_or(1),
            );
            println!("  Parent:    {}", a["parent_session_id"].as_str().unwrap_or("-"));
            println!("  Model:     {}", a["model"].as_str().unwrap_or("-"));
            println!(
                "  Workspace: {} (branch={})",
                a["workspace_dir"].as_str().unwrap_or("-"),
                a["worktree_branch"].as_str().unwrap_or("inherit"),
            );
            println!("  Goal:      {}", a["goal"].as_str().unwrap_or("-"));
            if let Some(st) = a.get("structured_output").filter(|v| !v.is_null()) {
                println!("  Structured JSON:\n{}", serde_json::to_string_pretty(st)?);
            } else if let Some(s) = a["summary"].as_str() {
                println!("  Summary:\n{s}");
            }
            if let Some(msgs) = data["messages"].as_array().filter(|m| !m.is_empty()) {
                println!("  Mailbox ({} message(s)):", msgs.len());
                for m in msgs {
                    println!(
                        "    {} -> {}: {}",
                        m["from_session_id"].as_str().unwrap_or("-"),
                        m["to_session_id"].as_str().unwrap_or("-"),
                        m["body"].as_str().unwrap_or(""),
                    );
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn run_agents_spawn(
    config_path: &Path,
    role: &str,
    goal: &str,
    parent: Option<&str>,
    workspace: Option<&str>,
    model: Option<&str>,
    run_in_background: bool,
    fork_context: bool,
    include_uncommitted: bool,
    max_duration_secs: Option<u64>,
    max_cost_usd: Option<f64>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call_timeout(
        &cfg,
        IPC_METHOD_AGENTS_SPAWN,
        json!({
            "parent_session_id": parent.unwrap_or("cli"),
            "role": role,
            "goal": goal,
            "workspace": workspace,
            "model": model,
            "run_in_background": run_in_background,
            "fork_context": fork_context,
            "include_uncommitted": include_uncommitted,
            "max_duration_secs": max_duration_secs,
            "max_cost_usd": max_cost_usd,
        }),
        Some(Duration::from_secs(SUBAGENT_MAX_WAIT_SECS + IPC_WAIT_TIMEOUT_SLACK_SECS)),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let rec = rt
                .subagents
                .spawn_agent(
                    &rt,
                    SubagentSpawnRequest {
                        parent_session_id: parent.unwrap_or("cli").to_string(),
                        role: role.to_string(),
                        goal: goal.to_string(),
                        context: None,
                        run_in_background: false,
                        workspace_mode: workspace.map(SubagentWorkspaceMode::from_str_loose),
                        output_schema: None,
                        model_override: model.map(String::from),
                        fork_context,
                        include_uncommitted,
                        max_duration_secs,
                        max_cost_usd,
                    },
                )
                .await?;
            serde_json::to_value(rec)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            println!(
                "Subagent {} [{}] status={} branch={}",
                data["id"].as_str().unwrap_or("-"),
                data["role"].as_str().unwrap_or(role),
                data["status"].as_str().unwrap_or("-"),
                data["worktree_branch"].as_str().unwrap_or("inherit"),
            );
            if let Some(s) = data["summary"].as_str() {
                println!("{s}");
            }
        }
    }
    Ok(())
}

pub async fn run_agents_wait(
    config_path: &Path,
    id: &str,
    timeout: Option<u64>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let secs = timeout
        .unwrap_or(SUBAGENT_DEFAULT_WAIT_SECS)
        .clamp(1, SUBAGENT_MAX_WAIT_SECS);
    let resp = ipc_call_timeout(
        &cfg,
        IPC_METHOD_AGENTS_WAIT,
        json!({ "agent_ids": [id], "wait_all": true, "timeout_secs": secs }),
        Some(Duration::from_secs(secs + IPC_WAIT_TIMEOUT_SLACK_SECS)),
    )
    .await
    .context("waiting on a running subagent requires a running daemon (`zenko web`)")?;
    let data = unwrap_ipc_result(resp)?;
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            for a in data.as_array().cloned().unwrap_or_default() {
                println!(
                    "{} [{}] status={}",
                    a["id"].as_str().unwrap_or(id),
                    a["role"].as_str().unwrap_or("-"),
                    a["status"].as_str().unwrap_or("-"),
                );
                if let Some(s) = a["summary"].as_str() {
                    println!("{s}");
                }
            }
        }
    }
    Ok(())
}

pub async fn run_agents_message(
    config_path: &Path,
    id: &str,
    message: &str,
    resume: bool,
    interrupt: bool,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_AGENTS_MESSAGE,
        json!({
            "from_session_id": "cli",
            "agent_id": id,
            "message": message,
            "resume": resume,
            "interrupt": interrupt,
        }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            serde_json::to_value(store.insert_agent_message("cli", id, message)?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Sent mailbox message {} to {} (action={}).",
            data["id"].as_str().unwrap_or("-"),
            id,
            data["delivery_action"].as_str().unwrap_or("queued")
        ),
    }
    Ok(())
}

pub async fn run_agents_diff(config_path: &Path, id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_AGENTS_DIFF,
        json!({ "caller_session_id": "cli", "agent_id": id }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let rep = rt
                .subagents
                .inspect_worktree_diff(&rt, "cli", id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("no git worktree for subagent `{id}`"))?;
            serde_json::to_value(rep)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            println!(
                "Worktree branch `{}` (changes={}):",
                data["branch"].as_str().unwrap_or("-"),
                data["has_changes"].as_bool().unwrap_or(false),
            );
            if let Some(stat) = data["diff_stat"].as_str().filter(|s| !s.is_empty()) {
                println!("{stat}");
            }
            if let Some(patch) = data["patch_preview"].as_str().filter(|s| !s.is_empty()) {
                println!("{patch}");
            }
        }
    }
    Ok(())
}

pub async fn run_agents_merge(
    config_path: &Path,
    id: &str,
    cleanup: bool,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_AGENTS_MERGE,
        json!({ "caller_session_id": "cli", "agent_id": id, "cleanup": cleanup }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            serde_json::to_value(rt.subagents.merge_worktree(&rt, "cli", id, cleanup).await?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Branch `{}` merged={} cleaned_up={}.",
            data["branch"].as_str().unwrap_or("-"),
            data["merged"].as_bool().unwrap_or(false),
            data["cleaned_up"].as_bool().unwrap_or(false),
        ),
    }
    Ok(())
}

pub async fn run_agents_discard(config_path: &Path, id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_AGENTS_DISCARD,
        json!({ "caller_session_id": "cli", "agent_id": id }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            serde_json::to_value(rt.subagents.discard_worktree(&rt, "cli", id).await?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Discarded worktree branch `{}` (cleaned_up={}).",
            data["branch"].as_str().unwrap_or("-"),
            data["cleaned_up"].as_bool().unwrap_or(false),
        ),
    }
    Ok(())
}

pub async fn run_agents_prune(config_path: &Path, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_AGENTS_PRUNE, json!({})).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            serde_json::to_value(rt.subagents.prune_orphan_worktrees(&rt).await?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Pruned {} orphan worktree director(ies).",
            data["removed_count"].as_u64().unwrap_or(0),
        ),
    }
    Ok(())
}

pub async fn run_agents_kill(
    config_path: &Path,
    id: &str,
    remove_worktree: bool,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_AGENTS_KILL,
        json!({
            "caller_session_id": "cli",
            "agent_id": id,
            "remove_worktree": remove_worktree
        }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let rec = rt
                .subagents
                .kill_agent(&rt, "cli", id, remove_worktree)
                .await?;
            json!({ "killed": rec.is_some(), "agent": rec })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!("Killed subagent {id} (worktree_removed={remove_worktree})."),
    }
    Ok(())
}

// ─── Phase 6 CLI: Session Task DAG, Plan Approval & Workspace Checkpoints / Undo ──

pub async fn run_dag_list(
    config_path: &Path,
    session_id: &str,
    ready_only: bool,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_TASKS_DAG_LIST,
        json!({ "session_id": session_id, "ready_only": ready_only }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let tasks = match ready_only {
                true => rt.clarifications.ready_session_tasks(session_id)?,
                false => rt.clarifications.list_session_tasks(session_id)?,
            };
            serde_json::to_value(tasks)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let empty = Vec::new();
            let list = data.as_array().unwrap_or(&empty);
            if list.is_empty() {
                println!("No session tasks found.");
            } else {
                for t in list {
                    let owner = t["owner_agent"]
                        .as_str()
                        .map(|o| format!(" owner={o}"))
                        .unwrap_or_default();
                    println!(
                        "  {} [{}] (p={}): {}{owner}",
                        t["id"].as_str().unwrap_or("-"),
                        t["status"].as_str().unwrap_or("-"),
                        t["priority"].as_i64().unwrap_or(0),
                        t["title"].as_str().unwrap_or(""),
                    );
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn run_dag_create(
    config_path: &Path,
    session_id: &str,
    title: &str,
    id: Option<&str>,
    depends_on: Vec<String>,
    verification_cmd: Option<&str>,
    priority: i32,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_TASKS_DAG_CREATE,
        json!({
            "session_id": session_id,
            "id": id,
            "title": title,
            "depends_on": depends_on,
            "verification_cmd": verification_cmd,
            "priority": priority,
        }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let now = Utc::now();
            let task = SessionTask {
                id: id
                    .map(String::from)
                    .unwrap_or_else(zenko_core::new_session_task_id),
                session_id: session_id.to_string(),
                branch_event_id: None,
                title: title.to_string(),
                description: String::new(),
                active_label: None,
                status: SessionTaskStatus::Pending,
                depends_on,
                owner_agent: None,
                verification_cmd: verification_cmd.map(String::from),
                verification_evidence: None,
                notes: None,
                priority,
                created_at: now,
                updated_at: now,
            };
            serde_json::to_value(rt.clarifications.create_session_task(session_id, task)?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Created task `{}` [{}]: {}",
            data["id"].as_str().unwrap_or("-"),
            data["status"].as_str().unwrap_or("-"),
            data["title"].as_str().unwrap_or(""),
        ),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn run_dag_update(
    config_path: &Path,
    session_id: &str,
    task_id: &str,
    status: Option<&str>,
    owner: Option<&str>,
    evidence: Option<&str>,
    notes: Option<&str>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let mut payload = json!({ "session_id": session_id, "task_id": task_id });
    if let Some(s) = status {
        payload["status"] = json!(s);
    }
    if let Some(o) = owner {
        payload["owner_agent"] = json!(o);
    }
    if let Some(e) = evidence {
        payload["verification_evidence"] = json!(e);
    }
    if let Some(n) = notes {
        payload["notes"] = json!(n);
    }
    let data = match ipc_call(&cfg, IPC_METHOD_TASKS_DAG_UPDATE, payload).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let patch = SessionTaskPatch {
                status: status.map(SessionTaskStatus::from_str_loose),
                owner_agent: owner.map(|s| Some(s.to_string())),
                verification_evidence: evidence.map(|s| Some(s.to_string())),
                notes: notes.map(|s| Some(s.to_string())),
                ..SessionTaskPatch::default()
            };
            serde_json::to_value(
                rt.clarifications
                    .update_session_task(session_id, task_id, patch)?,
            )?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Updated task `{}` [{}]: {}",
            data["id"].as_str().unwrap_or("-"),
            data["status"].as_str().unwrap_or("-"),
            data["title"].as_str().unwrap_or(""),
        ),
    }
    Ok(())
}

pub async fn run_dag_claim(
    config_path: &Path,
    session_id: &str,
    task_id: &str,
    owner: &str,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_TASKS_DAG_UPDATE,
        json!({ "session_id": session_id, "task_id": task_id, "claim_owner": owner }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            serde_json::to_value(
                rt.clarifications
                    .claim_session_task(session_id, task_id, owner)?,
            )?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Claimed task `{}` for `{}`.",
            data["id"].as_str().unwrap_or(task_id),
            data["owner_agent"].as_str().unwrap_or(owner),
        ),
    }
    Ok(())
}

pub async fn run_dag_delete(
    config_path: &Path,
    session_id: &str,
    task_id: Option<&str>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_TASKS_DAG_DELETE,
        json!({
            "session_id": session_id,
            "task_id": task_id,
            "clear_all": task_id.is_none(),
        }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            match task_id {
                Some(tid) => json!({ "deleted": rt.clarifications.delete_session_task(session_id, tid)? }),
                None => json!({ "cleared": rt.clarifications.clear_session_tasks(session_id)? }),
            }
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!("Session task DAG updated: {data}"),
    }
    Ok(())
}

pub async fn run_plan_approve(
    config_path: &Path,
    session_id: &str,
    mode: &str,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_SESSION_APPROVE_PLAN,
        json!({ "session_id": session_id, "mode": mode }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let (sess, task_count) =
                rt.approve_session_plan(session_id, PermissionMode::from_str_loose(mode))?;
            json!({ "session": sess, "task_count": task_count })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Approved plan for `{session_id}` -> permission_mode={} ({} tasks).",
            data["session"]["permission_mode"].as_str().unwrap_or(mode),
            data["task_count"].as_u64().unwrap_or(0),
        ),
    }
    Ok(())
}

pub async fn run_checkpoints_list(
    config_path: &Path,
    session_id: Option<&str>,
    limit: usize,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_CHECKPOINTS_LIST,
        json!({ "session_id": session_id, "limit": limit }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            serde_json::to_value(store.list_checkpoints(session_id, limit)?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let empty = Vec::new();
            let list = data.as_array().unwrap_or(&empty);
            if list.is_empty() {
                println!("No workspace checkpoints found.");
            } else {
                for cp in list {
                    let safety = match cp["is_safety_backup"].as_bool().unwrap_or(false) {
                        true => " [safety]",
                        false => "",
                    };
                    let files = cp["changed_files"].as_array().map(|a| a.len()).unwrap_or(0);
                    println!(
                        "  {} [{}]{safety} ({files} files) · {}",
                        cp["id"].as_str().unwrap_or("-"),
                        cp["backend"].as_str().unwrap_or("-"),
                        cp["label"].as_str().unwrap_or(""),
                    );
                }
            }
        }
    }
    Ok(())
}

pub async fn run_checkpoints_diff(
    config_path: &Path,
    checkpoint_id: &str,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_CHECKPOINTS_DIFF,
        json!({ "checkpoint_id": checkpoint_id }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let cp_cfg = cfg.checkpoints.clone();
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            serde_json::to_value(rt.checkpoints.diff_checkpoint(&cp_cfg, checkpoint_id).await?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            println!("Checkpoint `{checkpoint_id}` diff:");
            if let Some(conflicts) = data["manual_edit_conflicts"].as_array() {
                if !conflicts.is_empty() {
                    println!(
                        "  Manual hand-edits (preserved unless --force): {}",
                        conflicts
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
            if let Some(stat) = data["diff_stat"].as_str().filter(|s| !s.is_empty()) {
                println!("{stat}");
            }
            if let Some(patch) = data["patch_preview"].as_str().filter(|s| !s.is_empty()) {
                println!("{patch}");
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn run_checkpoints_rewind(
    config_path: &Path,
    checkpoint_id: &str,
    mode: &str,
    files: Vec<String>,
    overwrite_manual_edits: bool,
    summarize_abandoned: bool,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_CHECKPOINTS_REWIND,
        json!({
            "checkpoint_id": checkpoint_id,
            "mode": mode,
            "files": files,
            "overwrite_manual_edits": overwrite_manual_edits,
            "summarize_abandoned": summarize_abandoned,
        }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let cp_cfg = cfg.checkpoints.clone();
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            serde_json::to_value(
                rt.checkpoints
                    .rewind(
                        &cp_cfg,
                        &RewindRequest {
                            checkpoint_id: checkpoint_id.to_string(),
                            mode: RewindMode::from_str_loose(mode),
                            files,
                            overwrite_manual_edits,
                            summarize_abandoned,
                        },
                    )
                    .await?,
            )?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            println!(
                "Rewound to `{checkpoint_id}` (mode={}, restored={}, deleted={}, preserved_hand_edits={}).",
                data["mode"].as_str().unwrap_or(mode),
                data["restored_files"].as_array().map(|a| a.len()).unwrap_or(0),
                data["deleted_files"].as_array().map(|a| a.len()).unwrap_or(0),
                data["preserved_manual_edits"].as_array().map(|a| a.len()).unwrap_or(0),
            );
            if let Some(safety) = data["safety_checkpoint_id"].as_str() {
                println!("  Undo-of-undo safety checkpoint: {safety}");
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn run_undo(
    config_path: &Path,
    session_id: &str,
    checkpoint_id: Option<&str>,
    mode: &str,
    files: Vec<String>,
    overwrite_manual_edits: bool,
    summarize_abandoned: bool,
    json_out: bool,
) -> Result<()> {
    let target_cp = match checkpoint_id {
        Some(explicit) => explicit.to_string(),
        None => {
            let cfg = ZenkoConfig::load_or_default(config_path)?;
            let store = SqliteStore::open(cfg.db_path())?;
            store
                .list_checkpoints(Some(session_id), 20)?
                .into_iter()
                .find(|c| !c.is_safety_backup)
                .map(|c| c.id)
                .ok_or_else(|| {
                    anyhow::anyhow!("no workspace checkpoints found for session `{session_id}`")
                })?
        }
    };
    run_checkpoints_rewind(
        config_path,
        &target_cp,
        mode,
        files,
        overwrite_manual_edits,
        summarize_abandoned,
        json_out,
    )
    .await
}

pub async fn run_checkpoints_prune(config_path: &Path, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_CHECKPOINTS_PRUNE, json!({})).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let cp_cfg = cfg.checkpoints.clone();
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            serde_json::to_value(rt.checkpoints.prune_checkpoints(&cp_cfg).await?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Pruned {} expired checkpoint(s) (reclaimed {} bytes).",
            data["removed_checkpoints"].as_u64().unwrap_or(0),
            data["reclaimed_bytes"].as_u64().unwrap_or(0),
        ),
    }
    Ok(())
}

pub async fn run_doctor(config_path: &Path, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let db_res = SqliteStore::open(cfg.db_path()).map(|s| s.db_size_bytes());
    let vault = SecretVault::open(cfg.vault_path(), cfg.vault_key_path());
    let provider_desc = find_provider(&cfg.provider.kind);
    let has_provider_key = vault.has(provider_desc.vault_secret_key).unwrap_or(false);
    let has_tg = vault.has(VAULT_KEY_TELEGRAM).unwrap_or(false);

    let http = reqwest::Client::builder().build()?;
    let provider_check = match vault.get(provider_desc.vault_secret_key).ok().flatten() {
        Some(key) => {
            let p = UnifiedLlmProvider::for_provider(
                http.clone(),
                &cfg.provider.kind,
                cfg.provider.base_url.clone(),
                Some(key),
            );
            match p.verify_key().await {
                Ok(()) => Some(Ok(())),
                Err(e) => Some(Err(e.to_string())),
            }
        }
        None => None,
    };
    let tg_check = match vault.get(VAULT_KEY_TELEGRAM).ok().flatten() {
        Some(tok) => match verify_bot_token(&http, &tok).await {
            Ok(info) => Some(Ok(info.username.unwrap_or(info.first_name))),
            Err(e) => Some(Err(e.to_string())),
        },
        None => None,
    };

    match json_out {
        true => println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "config": config_path.display().to_string(),
                "config_exists": config_path.exists(),
                "data_dir": cfg.resolved_data_dir().display().to_string(),
                "workspace_dir": cfg.resolved_workspace_dir().display().to_string(),
                "sqlite_bytes": db_res.as_ref().ok().copied(),
                "sqlite_error": db_res.as_ref().err().map(|e| e.to_string()),
                "provider": provider_desc.id,
                "provider_key_set": has_provider_key,
                "provider_ok": provider_check.as_ref().map(|r| r.is_ok()),
                "telegram_key_set": has_tg,
                "telegram_ok": tg_check.as_ref().map(|r| r.is_ok()),
            }))?
        ),
        false => {
            println!("Zenko Doctor Diagnostics");
            println!(
                "  Config:      {} ({})",
                config_path.display(),
                match config_path.exists() {
                    true => "found",
                    false => "default",
                }
            );
            println!("  Data Dir:    {}", cfg.resolved_data_dir().display());
            println!("  Workspace:   {}", cfg.resolved_workspace_dir().display());
            match db_res {
                Ok(bytes) => println!("  SQLite WAL:  OK ({:.1} KB)", bytes as f64 / 1024.0),
                Err(e) => println!("  SQLite WAL:  ERROR ({e})"),
            }
            println!(
                "  Vault Keys:  {}={has_provider_key} Telegram={has_tg}",
                provider_desc.label
            );
            if let Some(res) = provider_check {
                match res {
                    Ok(()) => println!(
                        "  Provider:    CONNECTED ({} · {})",
                        provider_desc.label, cfg.provider.lead_model
                    ),
                    Err(e) => println!("  Provider:    FAILED ({e})"),
                }
            }
            if let Some(res) = tg_check {
                match res {
                    Ok(name) => println!("  Telegram:    CONNECTED (@{name})"),
                    Err(e) => println!("  Telegram:    FAILED ({e})"),
                }
            }
        }
    }
    Ok(())
}

// ─── Phase 7: External MCP Client & Pooled LSP CLI Commands ──────────────────

pub struct McpClientAddParams {
    pub name: String,
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    pub url: String,
    pub auth_secret: Option<String>,
    pub env_secrets: Vec<String>,
    pub enabled_tools: Vec<String>,
    pub disabled_tools: Vec<String>,
    pub risk: Option<String>,
    pub parallel: bool,
    pub allow_private_url: bool,
    pub sync: bool,
}

pub async fn run_mcp_client_list(config_path: &Path, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_MCP_SERVERS_LIST, json!({})).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let servers = rt.mcp_clients.list_status().await;
            let configs = rt.mcp_clients.configs();
            json!({ "servers": servers, "configs": configs })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let servers = data["servers"].as_array().cloned().unwrap_or_default();
            if servers.is_empty() {
                println!("No external MCP servers configured. Add one via `zenko mcp-client add` or WebUI Setup.");
                return Ok(());
            }
            for s in servers {
                let state = match (
                    s["enabled"].as_bool().unwrap_or(false),
                    s["connected"].as_bool().unwrap_or(false),
                ) {
                    (false, _) => "disabled",
                    (true, true) => "connected",
                    (true, false) => "idle/cached",
                };
                println!(
                    "{:<18} {:<6} {:<12} {} tool(s)",
                    s["name"].as_str().unwrap_or(""),
                    s["transport"].as_str().unwrap_or("stdio"),
                    state,
                    s["tool_count"].as_u64().unwrap_or(0),
                );
            }
        }
    }
    Ok(())
}

pub async fn run_mcp_client_add(
    config_path: &Path,
    params: McpClientAddParams,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let transport = match params.transport.trim().to_ascii_lowercase().as_str() {
        "http" | "streamable-http" | "sse" => McpTransport::Http,
        _ => McpTransport::Stdio,
    };
    let mut env_secrets = std::collections::BTreeMap::new();
    for pair in params.env_secrets {
        if let Some((k, v)) = pair.split_once('=') {
            let ek = k.trim();
            let vk = v.trim();
            if !ek.is_empty() && !vk.is_empty() {
                env_secrets.insert(ek.to_string(), vk.to_string());
            }
        }
    }
    let risk_override = params
        .risk
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| match s.to_ascii_lowercase().as_str() {
            "safe" => RiskLevel::Safe,
            "dangerous" => RiskLevel::Dangerous,
            _ => RiskLevel::Moderate,
        });

    let srv = McpServerConfig {
        name: params.name.trim().to_string(),
        enabled: true,
        transport,
        command: params.command,
        args: params.args,
        env_secrets,
        url: params.url,
        auth_header_secret: params.auth_secret,
        enabled_tools: params.enabled_tools,
        disabled_tools: params.disabled_tools,
        risk_override,
        supports_parallel: params.parallel,
        allow_private_url: params.allow_private_url,
        ..McpServerConfig::default()
    };

    let data = match ipc_call(
        &cfg,
        IPC_METHOD_MCP_SERVER_UPSERT,
        json!({ "server": &srv, "sync": params.sync }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let (servers, sync_error) = rt.upsert_mcp_server(srv.clone(), params.sync).await?;
            rt.shutdown_external_tools().await;
            json!({ "servers": servers, "sync_error": sync_error })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            println!("Saved MCP server `{}` ({}).", srv.name, srv.transport.as_str());
            if let Some(err) = data["sync_error"].as_str() {
                println!("Sync warning: {err}");
            }
        }
    }
    Ok(())
}

pub async fn run_mcp_client_remove(config_path: &Path, name: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_MCP_SERVER_DELETE, json!({ "name": name })).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let deleted = rt.delete_mcp_server(name).await?;
            json!({ "deleted": deleted })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "{}",
            match data["deleted"].as_bool().unwrap_or(false) {
                true => format!("Removed MCP server `{name}`."),
                false => format!("MCP server `{name}` was not configured."),
            }
        ),
    }
    Ok(())
}

pub async fn run_mcp_client_toggle(
    config_path: &Path,
    name: &str,
    explicit: Option<bool>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let mut target = cfg
        .mcp_servers
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name.trim()))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("MCP server `{name}` not configured"))?;
    target.enabled = explicit.unwrap_or(!target.enabled);

    let data = match ipc_call(
        &cfg,
        IPC_METHOD_MCP_SERVER_UPSERT,
        json!({ "server": &target, "sync": target.enabled }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let (servers, sync_error) = rt.upsert_mcp_server(target.clone(), false).await?;
            json!({ "servers": servers, "sync_error": sync_error })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "MCP server `{}` is now {}.",
            target.name,
            match target.enabled {
                true => "enabled",
                false => "disabled",
            }
        ),
    }
    Ok(())
}

pub async fn run_mcp_client_sync(config_path: &Path, name: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_MCP_SERVER_SYNC, json!({ "name": name })).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let status = rt.sync_mcp_server(name).await?;
            rt.shutdown_external_tools().await;
            serde_json::to_value(status)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Synced MCP server `{}`: {} deferred tool(s) registered.",
            data["name"].as_str().unwrap_or(name),
            data["tool_count"].as_u64().unwrap_or(0),
        ),
    }
    Ok(())
}

pub async fn run_lsp_status(config_path: &Path, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_LSP_STATUS, json!({})).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => json!({
            "enabled": cfg.lsp.enabled,
            "idle_timeout_secs": cfg.lsp.idle_timeout_secs,
            "active": [],
        }),
    };
    match json_out {
        true => println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "enabled": data["enabled"],
                "idle_timeout_secs": data["idle_timeout_secs"],
                "registry": LSP_SERVER_REGISTRY,
                "active": data["active"],
            }))?
        ),
        false => {
            println!(
                "LSP Pool: {} (idle eviction: {}s)",
                match data["enabled"].as_bool().unwrap_or(true) {
                    true => "enabled",
                    false => "disabled",
                },
                data["idle_timeout_secs"].as_u64().unwrap_or(600),
            );
            let active = data["active"].as_array().cloned().unwrap_or_default();
            if active.is_empty() {
                println!("  Active servers: none (servers spawn lazily on first `lsp_tool` call)");
            } else {
                for s in active {
                    println!(
                        "  {} ({}) · root={} · open_docs={}",
                        s["server_id"].as_str().unwrap_or(""),
                        s["language_id"].as_str().unwrap_or(""),
                        s["workspace_root"].as_str().unwrap_or(""),
                        s["open_documents"].as_u64().unwrap_or(0),
                    );
                }
            }
        }
    }
    Ok(())
}

pub async fn run_lsp_stop(config_path: &Path, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_LSP_STOP, json!({})).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => json!({ "stopped": 0 }),
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Stopped {} pooled LSP server(s).",
            data["stopped"].as_u64().unwrap_or(0),
        ),
    }
    Ok(())
}

// ─── Phase 8: Procedural Skills, 24/7 Goal Loops, Event Triggers & Trajectory Export ──

pub async fn run_skills_list(config_path: &Path, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_SKILLS_LIST, json!({})).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let mgr = SnapshotManager::new(cfg.resolved_data_dir(), cfg.resolved_workspace_dir())
                .with_hub_dir(cfg.resolved_hub_dir());
            serde_json::to_value(mgr.list_skills()?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let skills = data.as_array().cloned().unwrap_or_default();
            if skills.is_empty() {
                println!("No skills installed.");
                return Ok(());
            }
            for s in skills {
                println!(
                    "{:<24} [{:<6}] {:<8} ({})  ~{} tok — {}",
                    s["slug"].as_str().unwrap_or(""),
                    s["status"].as_str().unwrap_or("active"),
                    match s["enabled"].as_bool().unwrap_or(true) {
                        true => "enabled",
                        false => "disabled",
                    },
                    s["author"].as_str().unwrap_or("user"),
                    s["token_estimate"].as_u64().unwrap_or(0),
                    s["description"].as_str().unwrap_or(""),
                );
            }
        }
    }
    Ok(())
}

pub async fn run_skill_show(
    config_path: &Path,
    slug: &str,
    file_path: Option<&str>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_SKILL_GET,
        json!({ "slug": slug, "file_path": file_path }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let mgr = SnapshotManager::new(cfg.resolved_data_dir(), cfg.resolved_workspace_dir())
                .with_hub_dir(cfg.resolved_hub_dir());
            let (skill, sub) = mgr.read_skill(slug, file_path)?;
            json!({ "skill": skill, "subfile": sub })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            if let Some(sub) = data["subfile"].as_str() {
                println!("{sub}");
            } else {
                println!("{}", data["skill"]["content"].as_str().unwrap_or(""));
            }
        }
    }
    Ok(())
}

pub async fn run_skill_create(
    config_path: &Path,
    slug: &str,
    name: Option<&str>,
    description: &str,
    content: &str,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_SKILL_MANAGE,
        json!({
            "action": "create",
            "slug": slug,
            "name": name,
            "description": description,
            "content": content,
            "enabled": true,
        }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let mgr = SnapshotManager::new(cfg.resolved_data_dir(), cfg.resolved_workspace_dir())
                .with_hub_dir(cfg.resolved_hub_dir());
            let vault = SecretVault::open(cfg.vault_path(), cfg.vault_key_path());
            let (doc, snap) = mgr.create_skill_structured(
                slug,
                name,
                description,
                content,
                true,
                SkillStatus::Active,
                SkillAuthor::User,
                &[],
                &[],
                "cli",
                &vault.all_values(),
                cfg.skills.max_history_snapshots,
            )?;
            json!({ "skill": doc, "snapshot": snap })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!("Saved skill `{}`.", data["skill"]["slug"].as_str().unwrap_or(slug)),
    }
    Ok(())
}

pub async fn run_skill_toggle(
    config_path: &Path,
    slug: &str,
    enabled: bool,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_SKILL_MANAGE,
        json!({ "action": "toggle", "slug": slug, "enabled": enabled }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let mgr = SnapshotManager::new(cfg.resolved_data_dir(), cfg.resolved_workspace_dir())
                .with_hub_dir(cfg.resolved_hub_dir());
            let (doc, snap) = mgr.toggle_skill(slug, enabled, cfg.skills.max_history_snapshots)?;
            json!({ "skill": doc, "snapshot": snap })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!("Set skill `{slug}` enabled={enabled}."),
    }
    Ok(())
}

pub async fn run_skill_approve(config_path: &Path, slug: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_SKILL_APPROVE, json!({ "slug": slug })).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let mgr = SnapshotManager::new(cfg.resolved_data_dir(), cfg.resolved_workspace_dir())
                .with_hub_dir(cfg.resolved_hub_dir());
            let (doc, snap) = mgr.approve_skill(slug, cfg.skills.max_history_snapshots)?;
            json!({ "skill": doc, "snapshot": snap })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!("Approved and activated skill `{slug}`."),
    }
    Ok(())
}

pub async fn run_skill_history(
    config_path: &Path,
    slug: Option<&str>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_SKILL_HISTORY, json!({ "slug": slug })).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let mgr = SnapshotManager::new(cfg.resolved_data_dir(), cfg.resolved_workspace_dir())
                .with_hub_dir(cfg.resolved_hub_dir());
            serde_json::to_value(mgr.list_skill_history(slug)?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let items = data.as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                println!("No skill history snapshots found.");
                return Ok(());
            }
            for h in items {
                println!(
                    "{}  slug={:<20} reason={:<12} {}B",
                    h["id"].as_str().unwrap_or(""),
                    h["slug"].as_str().unwrap_or(""),
                    h["reason"].as_str().unwrap_or(""),
                    h["bytes"].as_u64().unwrap_or(0),
                );
            }
        }
    }
    Ok(())
}

pub async fn run_skill_rollback(
    config_path: &Path,
    snapshot_id: &str,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_SKILL_ROLLBACK,
        json!({ "snapshot_id": snapshot_id }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let mgr = SnapshotManager::new(cfg.resolved_data_dir(), cfg.resolved_workspace_dir())
                .with_hub_dir(cfg.resolved_hub_dir());
            let (doc, diff) = mgr.rollback_skill(snapshot_id, cfg.skills.max_history_snapshots)?;
            json!({ "skill": doc, "diff": diff })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Rolled back skill `{}` from snapshot `{snapshot_id}`.",
            data["skill"]["slug"].as_str().unwrap_or("")
        ),
    }
    Ok(())
}

pub async fn run_skill_delete(config_path: &Path, slug: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_SKILL_MANAGE,
        json!({ "action": "delete", "slug": slug }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let mgr = SnapshotManager::new(cfg.resolved_data_dir(), cfg.resolved_workspace_dir())
                .with_hub_dir(cfg.resolved_hub_dir());
            json!({ "deleted": mgr.delete_skill(slug)? })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Deleted skill `{slug}`: {}",
            data["deleted"].as_bool().unwrap_or(false)
        ),
    }
    Ok(())
}

pub async fn run_goals_list(config_path: &Path, limit: usize, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_GOALS_LIST, json!({ "limit": limit })).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            serde_json::to_value(store.list_goal_loops(limit)?)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let goals = data.as_array().cloned().unwrap_or_default();
            if goals.is_empty() {
                println!("No autonomous goal loops configured.");
                return Ok(());
            }
            for g in goals {
                println!(
                    "{:<16} [{:<16}] iter {}/{}  ${:.4}/${:.2}  {}  verify=`{}`",
                    g["id"].as_str().unwrap_or(""),
                    g["status"].as_str().unwrap_or(""),
                    g["iterations_done"].as_u64().unwrap_or(0),
                    g["max_iterations"].as_u64().unwrap_or(0),
                    g["cost_spent_usd"].as_f64().unwrap_or(0.0),
                    g["max_budget_usd"].as_f64().unwrap_or(0.0),
                    g["name"].as_str().unwrap_or(""),
                    g["verify_cmd"]
                        .as_array()
                        .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
                        .unwrap_or_default(),
                );
            }
        }
    }
    Ok(())
}

pub async fn run_goal_show(config_path: &Path, id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_GOAL_GET, json!({ "id": id })).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let goal = store
                .get_goal_loop(id)?
                .ok_or_else(|| anyhow::anyhow!("goal loop `{id}` not found"))?;
            let runs = store.list_goal_loop_runs(&goal.id, GOAL_LOOP_RUNS_DEFAULT_LIMIT)?;
            json!({ "goal": goal, "runs": runs })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let g = &data["goal"];
            println!(
                "Goal `{}` — {} [{}]\n  Session: {} · Workspace: {}\n  Progress: {}/{} iters · ${:.4}/${:.2} · stalls {}/{}\n  Last verify exit: {:?}",
                g["id"].as_str().unwrap_or(id),
                g["name"].as_str().unwrap_or(""),
                g["status"].as_str().unwrap_or(""),
                g["session_id"].as_str().unwrap_or(""),
                g["workspace_dir"].as_str().unwrap_or(""),
                g["iterations_done"].as_u64().unwrap_or(0),
                g["max_iterations"].as_u64().unwrap_or(0),
                g["cost_spent_usd"].as_f64().unwrap_or(0.0),
                g["max_budget_usd"].as_f64().unwrap_or(0.0),
                g["consecutive_stalls"].as_u64().unwrap_or(0),
                g["max_consecutive_stalls"].as_u64().unwrap_or(0),
                g["last_verify_exit"].as_i64(),
            );
        }
    }
    Ok(())
}

pub async fn run_goal_create(
    config_path: &Path,
    req: GoalLoopCreateRequest,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let payload = serde_json::to_value(&req)?;
    let data = match ipc_call(&cfg, IPC_METHOD_GOAL_CREATE, payload).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let goal = rt.create_goal_loop(req)?;
            serde_json::to_value(goal)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Created Goal Loop `{}` (`{}`).",
            data["id"].as_str().unwrap_or(""),
            data["name"].as_str().unwrap_or(""),
        ),
    }
    Ok(())
}

pub async fn run_goal_set_status(
    config_path: &Path,
    id: &str,
    status: GoalLoopStatus,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let patch = GoalLoopPatch {
        status: Some(status),
        reset_stalls: Some(status == GoalLoopStatus::Active),
        ..GoalLoopPatch::default()
    };
    let data = match ipc_call(
        &cfg,
        IPC_METHOD_GOAL_UPDATE,
        json!({ "id": id, "patch": patch }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let updated = rt
                .update_goal_loop(id, patch)?
                .ok_or_else(|| anyhow::anyhow!("goal loop `{id}` not found"))?;
            serde_json::to_value(updated)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!("Updated Goal Loop `{id}` -> `{}`.", status.as_str()),
    }
    Ok(())
}

pub async fn run_goal_run_now(config_path: &Path, id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call_timeout(
        &cfg,
        IPC_METHOD_GOAL_RUN_NOW,
        json!({ "id": id }),
        Some(Duration::from_secs(GOAL_LOOP_IPC_TIMEOUT_SECS)),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let updated =
                zenko_runtime::execute_goal_iteration(&rt, id, CancellationToken::new()).await?;
            serde_json::to_value(updated)?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Ran Goal Loop `{id}`: status=`{}`, iter={}, last_verify_exit={:?}.",
            data["status"].as_str().unwrap_or(""),
            data["iterations_done"].as_u64().unwrap_or(0),
            data["last_verify_exit"].as_i64(),
        ),
    }
    Ok(())
}

pub async fn run_goal_delete(config_path: &Path, id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_GOAL_DELETE, json!({ "id": id })).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            json!({ "deleted": store.delete_goal_loop(id)? })
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => println!(
            "Deleted Goal Loop `{id}`: {}",
            data["deleted"].as_bool().unwrap_or(false)
        ),
    }
    Ok(())
}

pub async fn run_triggers_list(config_path: &Path, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data = match ipc_call(&cfg, IPC_METHOD_TRIGGERS_LIST, json!({})).await {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            serde_json::to_value(rt.list_triggers())?
        }
    };
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&data)?),
        false => {
            let list = data.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                println!("No webhook rules or workspace watchers configured.");
                return Ok(());
            }
            for t in list {
                println!(
                    "{:<18} [{:<12}] {:<8} {}/{} hr -> {}  ({})",
                    t["id"].as_str().unwrap_or(""),
                    t["kind"].as_str().unwrap_or(""),
                    match t["enabled"].as_bool().unwrap_or(false) {
                        true => "enabled",
                        false => "disabled",
                    },
                    t["fires_last_hour"].as_u64().unwrap_or(0),
                    t["max_per_hour"].as_u64().unwrap_or(0),
                    t["target"].as_str().unwrap_or(""),
                    t["detail"].as_str().unwrap_or(""),
                );
            }
        }
    }
    Ok(())
}

pub struct ExportCliParams {
    pub session_id: Option<String>,
    pub all: bool,
    pub format: String,
    pub since: Option<String>,
    pub channel: Option<String>,
    pub verified_only: bool,
    pub min_turns: usize,
    pub include_thinking: bool,
    pub anonymize_paths: bool,
    pub all_branches: bool,
    pub output: Option<PathBuf>,
}

pub async fn run_export(
    config_path: &Path,
    params: ExportCliParams,
    json_out: bool,
) -> Result<()> {
    if params.session_id.is_none() && !params.all {
        anyhow::bail!("provide `<session_id>` or pass `--all` for batch trajectory export");
    }
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let fmt = ExportFormat::from_str_loose(&params.format);
    let since_dt = params.since.as_deref().and_then(parse_since_window);
    let opts = ExportOptions {
        format: fmt,
        include_thinking: params.include_thinking,
        anonymize_paths: params.anonymize_paths,
        all_branches: params.all_branches,
        verified_only: params.verified_only,
        min_turns: params.min_turns,
        since: since_dt,
        channel: params.channel,
        limit: None,
    };

    let data = match ipc_call(
        &cfg,
        IPC_METHOD_EXPORT_SESSION,
        json!({
            "session_id": params.session_id,
            "options": opts,
        }),
    )
    .await
    {
        Ok(resp) => unwrap_ipc_result(resp)?,
        Err(_) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let vault = SecretVault::open(cfg.vault_path(), cfg.vault_key_path());
            let secrets = vault.all_values();
            let home = zenko_core::expand_tilde("~");
            let rep = match params.session_id.as_deref() {
                Some(sid) => {
                    zenko_runtime::export_session(&store, sid, &opts, &secrets, Some(&home))?
                }
                None => zenko_runtime::export_batch(&store, &opts, &secrets, Some(&home))?,
            };
            serde_json::to_value(rep)?
        }
    };

    let content = data["content"].as_str().unwrap_or("");
    if let Some(out_path) = params.output {
        if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out_path, content)?;
        match json_out {
            true => println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "path": out_path.display().to_string(),
                    "format": data["format"],
                    "sessions_exported": data["sessions_exported"],
                    "events_exported": data["events_exported"],
                    "bytes": data["bytes"],
                }))?
            ),
            false => println!(
                "Exported {} session(s) ({} events, {} bytes, format={}) -> {}",
                data["sessions_exported"].as_u64().unwrap_or(0),
                data["events_exported"].as_u64().unwrap_or(0),
                data["bytes"].as_u64().unwrap_or(0),
                data["format"].as_str().unwrap_or(fmt.as_str()),
                out_path.display(),
            ),
        }
    } else if json_out {
        println!("{}", serde_json::to_string_pretty(&data)?);
    } else {
        print!("{content}");
    }
    Ok(())
}
