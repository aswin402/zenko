use super::*;

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
                data["idle_timeout_secs"]
                    .as_u64()
                    .unwrap_or(zenko_core::LSP_IDLE_TIMEOUT_SECS),
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

