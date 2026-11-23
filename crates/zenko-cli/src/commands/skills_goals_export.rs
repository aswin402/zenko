use super::*;

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
        Err(e) if is_daemon_offline(&e) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let goal = rt.create_goal_loop(req)?;
            serde_json::to_value(goal)?
        }
        Err(e) => return Err(e),
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
        Err(e) if is_daemon_offline(&e) => {
            let (rt, _) = AgentRuntime::build(config_path.to_path_buf(), cfg)?;
            let updated =
                zenko_runtime::execute_goal_iteration(&rt, id, CancellationToken::new()).await?;
            serde_json::to_value(updated)?
        }
        Err(e) => return Err(e),
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
