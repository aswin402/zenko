use super::*;

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
            let scoped_rules: Vec<&str> = data["scoped_rules_loaded"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let nested_docs = data["nested_repo_docs_count"].as_u64().unwrap_or(0);
            let cache_warns: Vec<&str> = data["cache_stability_warnings"]
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
                "  Tier 1 (KV):  {} tok (hash={} · prefix={})",
                format_tokens(t1),
                zenko_core::truncate_utf8_prefix(t1_hash, 12),
                match cache_warns.is_empty() {
                    true => "stable".to_string(),
                    false => format!("{} volatile warning(s)", cache_warns.len()),
                }
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
                "  Scoped DOX:   {} loaded / {nested_docs} indexed{}",
                scoped_rules.len(),
                match scoped_rules.is_empty() {
                    true => String::new(),
                    false => format!(" [{}]", scoped_rules.join(", ")),
                }
            );
            for warn in &cache_warns {
                println!("  KV Warning:   {warn}");
            }
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

