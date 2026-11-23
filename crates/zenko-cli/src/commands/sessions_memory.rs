use super::*;

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
        Err(e) if is_daemon_offline(&e) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let vault = SecretVault::open(cfg.vault_path(), cfg.vault_key_path());
            let vault_secrets = vault.all_values();
            let resolved_scope = match scope.trim() {
                "project" => project_scope_for_workspace(&cfg.resolved_workspace_dir()),
                "" => MEMORY_SCOPE_GLOBAL.to_string(),
                other => other.to_string(),
            };
            let res = store.upsert_memory_dedup_with_secrets(
                &resolved_scope,
                kind,
                title,
                content,
                1.0,
                None,
                None,
                cfg.memory.dedup_enabled,
                &vault_secrets,
            )?;
            serde_json::to_value(res.entry)?
        }
        Err(e) => return Err(e),
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
        Err(e) if is_daemon_offline(&e) => {
            let store = SqliteStore::open(cfg.db_path())?;
            let vault = SecretVault::open(cfg.vault_path(), cfg.vault_key_path());
            let vault_secrets = vault.all_values();
            let entry = store
                .update_memory_with_secrets(
                    id,
                    title,
                    content,
                    kind,
                    None,
                    resolved_scope.as_deref(),
                    &vault_secrets,
                )?
                .ok_or_else(|| anyhow::anyhow!("memory `{id}` not found"))?;
            serde_json::to_value(entry)?
        }
        Err(e) => return Err(e),
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
        Err(e) if is_daemon_offline(&e) => {
            let (rt, _logs) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let rec = rt.run_dream_now("cli", force).await?;
            serde_json::to_value(rec)?
        }
        Err(e) => return Err(e),
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

pub(super) fn print_log_entry_line(entry: &LogEntry, json_out: bool) {
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

