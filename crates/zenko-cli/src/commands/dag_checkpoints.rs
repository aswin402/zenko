use super::*;

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
            if let Some(conflicts) = data["manual_edit_conflicts"].as_array()
                && !conflicts.is_empty() {
                    println!(
                        "  Manual hand-edits (preserved unless --force): {}",
                        conflicts
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
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
        Err(e) if is_daemon_offline(&e) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            serde_json::to_value(
                rt.rewind_checkpoint(&RewindRequest {
                    checkpoint_id: checkpoint_id.to_string(),
                    mode: RewindMode::from_str_loose(mode),
                    files,
                    overwrite_manual_edits,
                    summarize_abandoned,
                })
                .await?,
            )?
        }
        Err(e) => return Err(e),
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
                .list_checkpoints(Some(session_id), zenko_core::CHECKPOINT_LIST_DEFAULT_LIMIT)?
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

    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()?;
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

