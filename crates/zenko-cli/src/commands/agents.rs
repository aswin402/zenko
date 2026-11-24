use super::*;

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
        Err(e) if is_daemon_offline(&e) => {
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
                        custom_directive: None,
                        cognitive_tier: None,
                        read_only: None,
                        tool_subset: Vec::new(),
                    },
                )
                .await?;
            serde_json::to_value(rec)?
        }
        Err(e) => return Err(e),
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

