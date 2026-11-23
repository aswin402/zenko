//! Pure-Rust Model Context Protocol (MCP) JSON-RPC 2.0 Stdio Server (`zenko mcp`).
//! Enables ANY external AI agent (Antigravity, Claude Code, Codex, Cursor, scripts) to invoke
//! Zenko's 24/7 autonomous runtime, persistent SQLite FTS5 memory, sessions, and cron scheduler.

use crate::ipc::{ipc_call, is_daemon_offline, unwrap_ipc_result};
use anyhow::Result;
use chrono::Utc;
use croner::Cron;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use zenko_core::{
    CronJob, CronKind, ExportFormat, ExportOptions, GOAL_LOOP_LIST_DEFAULT_LIMIT,
    GoalLoopCreateRequest, HUB_CATEGORIES, IPC_METHOD_ASK, IPC_METHOD_CHECKPOINTS_LIST,
    IPC_METHOD_CHECKPOINTS_REWIND, IPC_METHOD_CLARIFICATION_RESOLVE,
    IPC_METHOD_CLARIFICATIONS_LIST, IPC_METHOD_CRON_CREATE, IPC_METHOD_CRON_DELETE,
    IPC_METHOD_CRON_LIST, IPC_METHOD_CRON_RUN_NOW, IPC_METHOD_CRON_UPDATE,
    IPC_METHOD_EXPORT_SESSION, IPC_METHOD_GOAL_CREATE, IPC_METHOD_GOALS_LIST, IPC_METHOD_HUB_LIST,
    IPC_METHOD_JOBS_DELETE, IPC_METHOD_JOBS_INPUT, IPC_METHOD_JOBS_LIST, IPC_METHOD_JOBS_PRUNE,
    IPC_METHOD_JOBS_READ, IPC_METHOD_JOBS_RESTART, IPC_METHOD_JOBS_START, IPC_METHOD_JOBS_STOP,
    IPC_METHOD_JOBS_UPDATE, IPC_METHOD_JOBS_WAIT, IPC_METHOD_LOGS_QUERY, IPC_METHOD_LSP_STATUS,
    IPC_METHOD_MCP_SERVERS_LIST, IPC_METHOD_MEMORY_ADD, IPC_METHOD_MEMORY_SEARCH,
    IPC_METHOD_ORCHESTRATOR_TRIAGE, IPC_METHOD_SESSION_APPROVE_PLAN, IPC_METHOD_SESSION_BACKGROUND,
    IPC_METHOD_SESSION_STEER, IPC_METHOD_SESSIONS_LIST, IPC_METHOD_SKILL_APPROVE,
    IPC_METHOD_SKILLS_LIST, IPC_METHOD_STATUS, IPC_METHOD_TASKS_DAG_LIST, JOB_DEFAULT_LIST_LIMIT,
    JOB_DEFAULT_READ_BYTES, JOB_DEFAULT_WAIT_SECS, LSP_SERVER_REGISTRY, LogFilter,
    MCP_PROTOCOL_VERSION, MCP_SERVER_NAME, MCP_SERVER_VERSION, PermissionMode, RewindMode,
    RewindRequest, SecretVault, TaskKind, ZenkoConfig, new_id,
};
use zenko_memory::{SnapshotManager, SqliteStore};
use zenko_runtime::{AgentRuntime, TurnOptions};

struct McpToolSpec {
    name: &'static str,
    description: &'static str,
    schema_fn: fn() -> Value,
}

const MCP_TOOLS: &[McpToolSpec] = &[
    McpToolSpec {
        name: "zenko_ask",
        description: "Delegate a task or prompt to the 24/7 autonomous Zenko AI agent (uses live daemon when running, otherwise standalone). Returns session_id, reply, model, and usage.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Task or question for Zenko" },
                    "session_id": { "type": "string", "description": "Optional existing session ID to continue" },
                    "workspace": { "type": "string", "description": "Optional workspace directory override" },
                    "model": { "type": "string", "description": "Optional model ID override" },
                    "mode": { "type": "string", "enum": ["plan", "strict", "balanced", "autonomous"], "description": "Tool execution policy for this turn" }
                },
                "required": ["prompt"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_status",
        description: "Check Zenko daemon telemetry (running state, RAM RSS, uptime, daily spend, cache hit rate, active sessions).",
        schema_fn: || json!({ "type": "object", "properties": {} }),
    },
    McpToolSpec {
        name: "zenko_sessions_list",
        description: "List recent Zenko sessions with IDs, titles, channel, status, and token totals.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max sessions to return (default 20)" }
                }
            })
        },
    },
    McpToolSpec {
        name: "zenko_memory_search",
        description: "Search Zenko's persistent SQLite FTS5 long-term memory.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query (empty returns recent entries)" },
                    "limit": { "type": "integer", "description": "Max results (default 15)" }
                }
            })
        },
    },
    McpToolSpec {
        name: "zenko_memory_add",
        description: "Persist a fact, lesson, or preference into Zenko's long-term SQLite FTS5 memory.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "content": { "type": "string" },
                    "kind": { "type": "string", "enum": ["fact", "preference", "lesson", "episode"] }
                },
                "required": ["title", "content"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_cron_list",
        description: "List all 24/7 recurring cron jobs and autonomous heartbeats.",
        schema_fn: || json!({ "type": "object", "properties": {} }),
    },
    McpToolSpec {
        name: "zenko_cron_create",
        description: "Schedule a 24/7 recurring cron job, autonomous heartbeat, or zero-token script job (`kind: script` running from ~/zenko_hub/scripts/).",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "cron_expr": { "type": "string", "description": "Standard 5-field cron expression e.g. '0 9 * * *'" },
                    "prompt": { "type": "string", "description": "Autonomous prompt to execute on schedule" },
                    "script_path": { "type": "string", "description": "Script inside ~/zenko_hub/scripts/ when kind=script" },
                    "kind": { "type": "string", "enum": ["cron", "heartbeat", "script"] }
                },
                "required": ["name", "cron_expr"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_cron_update",
        description: "Update, pause, or resume a scheduled cron/heartbeat/script job.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "name": { "type": "string" },
                    "cron_expr": { "type": "string" },
                    "prompt": { "type": "string" },
                    "script_path": { "type": "string" },
                    "enabled": { "type": "boolean" }
                },
                "required": ["id"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_cron_run_now",
        description: "Trigger a scheduled cron/script job immediately.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_cron_delete",
        description: "Delete a scheduled 24/7 cron job by ID.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" }
                },
                "required": ["id"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_logs_query",
        description: "Query Zenko's categorized persistent logs (session, gateway, webui, tui, cron, system) by category, level, session_id, or text search.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "category": { "type": "string", "enum": ["session", "gateway", "webui", "tui", "cron", "system"] },
                    "level": { "type": "string", "enum": ["INFO", "WARN", "ERROR"] },
                    "session_id": { "type": "string" },
                    "search": { "type": "string" },
                    "limit": { "type": "integer", "description": "Max entries to return (default 50)" }
                }
            })
        },
    },
    McpToolSpec {
        name: "zenko_jobs_list",
        description: "List running and recent ZenkoTasks (filterable by category `kind` or `session_id`).",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "kind": { "type": "string", "enum": ["all", "shell", "build", "dev", "debug", "script", "cron", "browser", "subagent"] },
                    "limit": { "type": "integer", "description": "Max tasks to return (default 25)" }
                }
            })
        },
    },
    McpToolSpec {
        name: "zenko_jobs_start",
        description: "Start a categorized background ZenkoTask (shell, build, dev, debug, script, etc.).",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "kind": { "type": "string", "enum": ["shell", "build", "dev", "debug", "script", "cron", "browser", "subagent"] },
                    "label": { "type": "string" },
                    "workspace": { "type": "string" }
                },
                "required": ["command"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_jobs_input",
        description: "Write stdin text to a running background ZenkoTask.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string" },
                    "input": { "type": "string" }
                },
                "required": ["job_id", "input"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_jobs_update",
        description: "Update a ZenkoTask's human label or category kind.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string" },
                    "label": { "type": "string" },
                    "kind": { "type": "string" }
                },
                "required": ["job_id"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_jobs_restart",
        description: "Restart an existing ZenkoTask with the same command, workspace, kind, and label.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": { "job_id": { "type": "string" } },
                "required": ["job_id"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_jobs_delete",
        description: "Stop (if running) and delete a ZenkoTask record along with its categorized log file.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": { "job_id": { "type": "string" } },
                "required": ["job_id"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_jobs_prune",
        description: "Prune finished ZenkoTasks and remove their log files (optionally filtered by kind).",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": { "kind": { "type": "string" } }
            })
        },
    },
    McpToolSpec {
        name: "zenko_hub_list",
        description: "List categorized creations stored in ~/zenko_hub/ (projects, scripts, media, artifacts).",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "category": { "type": "string", "enum": ["all", "projects", "scripts", "media", "artifacts"] },
                    "limit": { "type": "integer" }
                }
            })
        },
    },
    McpToolSpec {
        name: "zenko_jobs_read",
        description: "Read streamed stdout/stderr bytes from a background job log.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string" },
                    "offset": { "type": "integer" },
                    "max_bytes": { "type": "integer" }
                },
                "required": ["job_id"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_jobs_stop",
        description: "Stop a running background job and its process group.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string" }
                },
                "required": ["job_id"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_jobs_wait",
        description: "Zero-CPU event-driven wait for a background job to finish (or return its latest output tail after timeout_secs).",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string" },
                    "timeout_secs": { "type": "integer" }
                },
                "required": ["job_id"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_session_steer",
        description: "Inject non-destructive mid-flight steering guidance into an actively running Zenko session turn.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "text": { "type": "string" }
                },
                "required": ["session_id", "text"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_session_background",
        description: "Live-demote an active foreground shell command in a session to an async background job without killing it or losing output.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" }
                },
                "required": ["session_id"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_orchestrator_triage",
        description: "Deterministically classify a prompt into ExecutionTier (instant | standard | orchestrated) with complexity/ambiguity score and recommended model role.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string" },
                    "mode": { "type": "string", "enum": ["plan", "strict", "balanced", "autonomous"] }
                },
                "required": ["prompt"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_clarifications_list",
        description: "List pending interactive clarification decision cards.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": { "session_id": { "type": "string" } }
            })
        },
    },
    McpToolSpec {
        name: "zenko_clarification_resolve",
        description: "Resolve a pending clarification decision card with a chosen option ID.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "selected_option_id": { "type": "string" },
                    "custom_answer": { "type": "string" }
                },
                "required": ["id", "selected_option_id"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_session_tasks_list",
        description: "List durable SQLite Task DAG items (or only topologically ready tasks) for a session.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "ready_only": { "type": "boolean" }
                },
                "required": ["session_id"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_checkpoints_list",
        description: "List shadow-git workspace checkpoints recorded for a session.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "limit": { "type": "integer" }
                }
            })
        },
    },
    McpToolSpec {
        name: "zenko_session_rewind",
        description: "Safely rewind workspace files and/or conversation history to a checkpoint (preserving manual hand-edits by default).",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "checkpoint_id": { "type": "string" },
                    "mode": { "type": "string", "enum": ["code_and_conversation", "code_only", "conversation_only"] },
                    "files": { "type": "array", "items": { "type": "string" } },
                    "overwrite_manual_edits": { "type": "boolean" },
                    "summarize_abandoned": { "type": "boolean" }
                },
                "required": ["checkpoint_id"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_plan_approve",
        description: "Approve a session's Read-Only Plan Mode blueprint and promote its permission mode.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "mode": { "type": "string", "enum": ["balanced", "autonomous", "strict"] }
                },
                "required": ["session_id"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_mcp_servers_list",
        description: "List configured external MCP client servers, live connection state, and discovered deferred tools.",
        schema_fn: || json!({ "type": "object", "properties": {} }),
    },
    McpToolSpec {
        name: "zenko_lsp_status",
        description: "Inspect Zenko's pooled Language Server Protocol (LSP) status, supported languages, and active server processes.",
        schema_fn: || json!({ "type": "object", "properties": {} }),
    },
    McpToolSpec {
        name: "zenko_skills_list",
        description: "List Zenko's installed global & workspace procedural skills (including draft skills awaiting approval).",
        schema_fn: || json!({ "type": "object", "properties": {} }),
    },
    McpToolSpec {
        name: "zenko_skill_approve",
        description: "Approve and activate a pending draft procedural skill by slug.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": { "slug": { "type": "string" } },
                "required": ["slug"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_goals_list",
        description: "List Zenko's persistent 24/7 bounded autonomous Goal Loops with iteration/budget/verification status.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": { "limit": { "type": "integer" } }
            })
        },
    },
    McpToolSpec {
        name: "zenko_goal_create",
        description: "Create a persistent 24/7 verify-until-green autonomous Goal Loop.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "goal_text": { "type": "string" },
                    "verify_cmd": { "type": "array", "items": { "type": "string" } },
                    "max_iterations": { "type": "integer" },
                    "max_budget_usd": { "type": "number" }
                },
                "required": ["name", "goal_text", "verify_cmd"]
            })
        },
    },
    McpToolSpec {
        name: "zenko_export_session",
        description: "Export a secret-scrubbed Zenko session trajectory in `jsonl`, `sharegpt`, or `markdown` format.",
        schema_fn: || {
            json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "format": { "type": "string", "enum": ["jsonl", "sharegpt", "markdown"] },
                    "anonymize_paths": { "type": "boolean" },
                    "include_thinking": { "type": "boolean" }
                },
                "required": ["session_id"]
            })
        },
    },
];

pub async fn run_mcp_stdio(config_path: PathBuf) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(&config_path)?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // Dedicated blocking reader for stdin so Tokio workers never stall.
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines().map_while(std::result::Result::ok) {
            let trimmed = line.trim().to_string();
            if !trimmed.is_empty() && tx.send(trimmed).is_err() {
                break;
            }
        }
    });

    let mut lazy_runtime: Option<Arc<AgentRuntime>> = None;
    let stdout = std::io::stdout();

    while let Some(raw_line) = rx.recv().await {
        let req: Value = match serde_json::from_str(&raw_line) {
            Ok(v) => v,
            Err(e) => {
                let err_resp = jsonrpc_error(Value::Null, -32700, &format!("parse error: {e}"));
                write_json_line(&stdout, &err_resp)?;
                continue;
            }
        };

        let id = req.get("id").cloned();
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

        // JSON-RPC notifications have no `id` and expect no response.
        if id.is_none() {
            continue;
        }
        let req_id = id.unwrap_or(Value::Null);

        let response = dispatch_mcp_request(
            &config_path,
            &cfg,
            &mut lazy_runtime,
            req_id,
            method,
            params,
        )
        .await;
        write_json_line(&stdout, &response)?;
    }

    Ok(())
}

fn write_json_line(stdout: &std::io::Stdout, val: &Value) -> Result<()> {
    let mut lock = stdout.lock();
    let encoded = serde_json::to_string(val)?;
    writeln!(lock, "{encoded}")?;
    lock.flush()?;
    Ok(())
}

fn jsonrpc_ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn jsonrpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

fn mcp_tools_list_payload() -> Value {
    let tools: Vec<Value> = MCP_TOOLS
        .iter()
        .map(|spec| {
            json!({
                "name": spec.name,
                "description": spec.description,
                "inputSchema": (spec.schema_fn)(),
            })
        })
        .collect();
    json!({ "tools": tools })
}

async fn dispatch_mcp_request(
    config_path: &Path,
    cfg: &ZenkoConfig,
    lazy_runtime: &mut Option<Arc<AgentRuntime>>,
    id: Value,
    method: &str,
    params: Value,
) -> Value {
    match method {
        "initialize" => jsonrpc_ok(
            id,
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": MCP_SERVER_NAME,
                    "version": MCP_SERVER_VERSION,
                }
            }),
        ),
        "ping" => jsonrpc_ok(id, json!({})),
        "tools/list" => jsonrpc_ok(id, mcp_tools_list_payload()),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match execute_mcp_tool(config_path, cfg, lazy_runtime, name, args).await {
                Ok(data) => {
                    let text = serde_json::to_string_pretty(&data).unwrap_or_else(|_| data.to_string());
                    jsonrpc_ok(
                        id,
                        json!({
                            "content": [{ "type": "text", "text": text }],
                            "structuredContent": data,
                            "isError": false
                        }),
                    )
                }
                Err(e) => jsonrpc_ok(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": format!("Error: {e}") }],
                        "isError": true
                    }),
                ),
            }
        }
        other => jsonrpc_error(id, -32601, &format!("method not found: {other}")),
    }
}

fn get_or_build_runtime(
    lazy_runtime: &mut Option<Arc<AgentRuntime>>,
    config_path: &Path,
    cfg: &ZenkoConfig,
) -> Result<Arc<AgentRuntime>> {
    if let Some(existing) = lazy_runtime {
        return Ok(existing.clone());
    }
    let (built, _logs) = AgentRuntime::build(config_path.to_path_buf(), cfg.clone())?;
    *lazy_runtime = Some(built.clone());
    Ok(built)
}

async fn execute_mcp_tool(
    config_path: &Path,
    cfg: &ZenkoConfig,
    lazy_runtime: &mut Option<Arc<AgentRuntime>>,
    name: &str,
    args: Value,
) -> Result<Value> {
    match name {
        "zenko_status" => match ipc_call(cfg, IPC_METHOD_STATUS, json!({})).await {
            Ok(resp) => {
                let telemetry = unwrap_ipc_result(resp)?;
                Ok(json!({ "running": true, "telemetry": telemetry }))
            }
            Err(_) => Ok(json!({
                "running": false,
                "config": config_path.display().to_string(),
                "hint": "Start Zenko WebUI & 24/7 daemon with `zenko web`."
            })),
        },
        "zenko_sessions_list" => {
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20);
            match ipc_call(cfg, IPC_METHOD_SESSIONS_LIST, json!({ "limit": limit })).await {
                Ok(resp) => unwrap_ipc_result(resp),
                Err(_) => {
                    let store = SqliteStore::open(cfg.db_path())?;
                    Ok(serde_json::to_value(store.list_sessions(limit as usize)?)?)
                }
            }
        }
        "zenko_memory_search" => {
            let query = args.get("query").and_then(Value::as_str).unwrap_or("");
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(15);
            match ipc_call(
                cfg,
                IPC_METHOD_MEMORY_SEARCH,
                json!({ "query": query, "limit": limit }),
            )
            .await
            {
                Ok(resp) => unwrap_ipc_result(resp),
                Err(_) => {
                    let store = SqliteStore::open(cfg.db_path())?;
                    Ok(serde_json::to_value(
                        store.search_memory_fts(query, limit as usize)?,
                    )?)
                }
            }
        }
        "zenko_memory_add" => {
            let title = args.get("title").and_then(Value::as_str).unwrap_or("Note");
            let content = args
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing `content`"))?;
            let kind = args.get("kind").and_then(Value::as_str).unwrap_or("fact");
            match ipc_call(
                cfg,
                IPC_METHOD_MEMORY_ADD,
                json!({ "title": title, "content": content, "kind": kind }),
            )
            .await
            {
                Ok(resp) => unwrap_ipc_result(resp),
                Err(e) if is_daemon_offline(&e) => {
                    let store = SqliteStore::open(cfg.db_path())?;
                    let vault = SecretVault::open(cfg.vault_path(), cfg.vault_key_path());
                    let vault_secrets = vault.all_values();
                    let res = store.upsert_memory_dedup_with_secrets(
                        zenko_core::MEMORY_SCOPE_GLOBAL,
                        kind,
                        title,
                        content,
                        1.0,
                        None,
                        None,
                        cfg.memory.dedup_enabled,
                        &vault_secrets,
                    )?;
                    Ok(serde_json::to_value(res.entry)?)
                }
                Err(e) => Err(e),
            }
        }
        "zenko_cron_list" => match ipc_call(cfg, IPC_METHOD_CRON_LIST, json!({})).await {
            Ok(resp) => unwrap_ipc_result(resp),
            Err(_) => {
                let store = SqliteStore::open(cfg.db_path())?;
                Ok(serde_json::to_value(store.list_cron_jobs()?)?)
            }
        },
        "zenko_cron_create" => {
            let name_arg = args.get("name").and_then(Value::as_str).unwrap_or("Scheduled Task");
            let expr = args
                .get("cron_expr")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing `cron_expr`"))?;
            let prompt = args.get("prompt").and_then(Value::as_str).unwrap_or("");
            let script_path = args
                .get("script_path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from);
            let kind = args.get("kind").and_then(Value::as_str).unwrap_or("cron");
            match ipc_call(
                cfg,
                IPC_METHOD_CRON_CREATE,
                json!({
                    "name": name_arg,
                    "cron_expr": expr,
                    "prompt": prompt,
                    "script_path": script_path,
                    "kind": kind
                }),
            )
            .await
            {
                Ok(resp) => unwrap_ipc_result(resp),
                Err(_) => {
                    let parsed = Cron::new(expr)
                        .parse()
                        .map_err(|e| anyhow::anyhow!("invalid cron expression: {e}"))?;
                    let store = SqliteStore::open(cfg.db_path())?;
                    let job = CronJob {
                        id: new_id(),
                        name: name_arg.to_string(),
                        cron_expr: expr.to_string(),
                        kind: CronKind::from_str_loose(kind),
                        prompt: prompt.to_string(),
                        script_path,
                        target_channel: "mcp".into(),
                        target_chat_id: None,
                        target_thread_id: None,
                        enabled: true,
                        next_run_at: parsed.find_next_occurrence(&Utc::now(), false).ok(),
                        last_run_at: None,
                        last_status: None,
                    };
                    store.upsert_cron_job(&job)?;
                    Ok(serde_json::to_value(job)?)
                }
            }
        }
        "zenko_cron_update" => {
            let resp = ipc_call(cfg, IPC_METHOD_CRON_UPDATE, args).await?;
            unwrap_ipc_result(resp)
        }
        "zenko_cron_run_now" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing `id`"))?;
            let resp = ipc_call(cfg, IPC_METHOD_CRON_RUN_NOW, json!({ "id": id })).await?;
            unwrap_ipc_result(resp)
        }
        "zenko_cron_delete" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing `id`"))?;
            match ipc_call(cfg, IPC_METHOD_CRON_DELETE, json!({ "id": id })).await {
                Ok(resp) => unwrap_ipc_result(resp),
                Err(_) => {
                    let store = SqliteStore::open(cfg.db_path())?;
                    Ok(json!({ "deleted": store.delete_cron_job(id)? }))
                }
            }
        }
        "zenko_logs_query" => {
            let mut filter: LogFilter = serde_json::from_value(args).unwrap_or_default();
            if filter.limit.is_none() {
                filter.limit = Some(50);
            }
            match ipc_call(cfg, IPC_METHOD_LOGS_QUERY, serde_json::to_value(&filter)?).await {
                Ok(resp) => unwrap_ipc_result(resp),
                Err(_) => {
                    let store = SqliteStore::open(cfg.db_path())?;
                    Ok(serde_json::to_value(store.query_logs(&filter)?)?)
                }
            }
        }
        "zenko_jobs_list" => {
            let sid = args.get("session_id").and_then(Value::as_str);
            let kind = args
                .get("kind")
                .and_then(Value::as_str)
                .and_then(TaskKind::parse_filter);
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(JOB_DEFAULT_LIST_LIMIT as u64);
            match ipc_call(
                cfg,
                IPC_METHOD_JOBS_LIST,
                json!({ "session_id": sid, "kind": kind.map(TaskKind::as_str), "limit": limit }),
            )
            .await
            {
                Ok(resp) => unwrap_ipc_result(resp),
                Err(_) => {
                    let store = SqliteStore::open(cfg.db_path())?;
                    Ok(serde_json::to_value(store.list_jobs(sid, kind, limit as usize)?)?)
                }
            }
        }
        "zenko_jobs_start" => {
            let resp = ipc_call(cfg, IPC_METHOD_JOBS_START, args).await?;
            unwrap_ipc_result(resp)
        }
        "zenko_jobs_input" => {
            let resp = ipc_call(cfg, IPC_METHOD_JOBS_INPUT, args).await?;
            unwrap_ipc_result(resp)
        }
        "zenko_jobs_update" => {
            let resp = ipc_call(cfg, IPC_METHOD_JOBS_UPDATE, args).await?;
            unwrap_ipc_result(resp)
        }
        "zenko_jobs_restart" => {
            let resp = ipc_call(cfg, IPC_METHOD_JOBS_RESTART, args).await?;
            unwrap_ipc_result(resp)
        }
        "zenko_jobs_delete" => {
            let resp = ipc_call(cfg, IPC_METHOD_JOBS_DELETE, args).await?;
            unwrap_ipc_result(resp)
        }
        "zenko_jobs_prune" => {
            let resp = ipc_call(cfg, IPC_METHOD_JOBS_PRUNE, args).await?;
            unwrap_ipc_result(resp)
        }
        "zenko_hub_list" => {
            let cat = args
                .get("category")
                .and_then(Value::as_str)
                .map(String::from);
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
            match ipc_call(cfg, IPC_METHOD_HUB_LIST, args).await {
                Ok(resp) => unwrap_ipc_result(resp),
                Err(_) => {
                    let root = cfg.ensure_hub_layout()?;
                    let entries = cfg.list_hub_entries(cat.as_deref(), limit)?;
                    Ok(json!({
                        "hub_dir": root.display().to_string(),
                        "categories": HUB_CATEGORIES,
                        "entries": entries,
                    }))
                }
            }
        }
        "zenko_jobs_read" => {
            let job_id = args
                .get("job_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing `job_id`"))?;
            let offset = args.get("offset").and_then(Value::as_u64);
            let max_bytes = args
                .get("max_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(JOB_DEFAULT_READ_BYTES as u64);
            match ipc_call(
                cfg,
                IPC_METHOD_JOBS_READ,
                json!({ "job_id": job_id, "offset": offset, "max_bytes": max_bytes }),
            )
            .await
            {
                Ok(resp) => unwrap_ipc_result(resp),
                Err(_) => {
                    let store = SqliteStore::open(cfg.db_path())?;
                    let job = store
                        .get_job(job_id)?
                        .ok_or_else(|| anyhow::anyhow!("job `{job_id}` not found"))?;
                    Ok(serde_json::to_value(
                        zenko_runtime::read_job_log_slice_sync(job, offset, max_bytes as usize)?,
                    )?)
                }
            }
        }
        "zenko_jobs_stop" => {
            let job_id = args
                .get("job_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing `job_id`"))?;
            let resp = ipc_call(cfg, IPC_METHOD_JOBS_STOP, json!({ "job_id": job_id })).await?;
            unwrap_ipc_result(resp)
        }
        "zenko_jobs_wait" => {
            let job_id = args
                .get("job_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing `job_id`"))?;
            let timeout_secs = args
                .get("timeout_secs")
                .and_then(Value::as_u64)
                .unwrap_or(JOB_DEFAULT_WAIT_SECS);
            let resp = ipc_call(
                cfg,
                IPC_METHOD_JOBS_WAIT,
                json!({ "job_id": job_id, "timeout_secs": timeout_secs }),
            )
            .await?;
            unwrap_ipc_result(resp)
        }
        "zenko_session_steer" => {
            let sid = args
                .get("session_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing `session_id`"))?;
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing `text`"))?;
            let resp = ipc_call(
                cfg,
                IPC_METHOD_SESSION_STEER,
                json!({ "session_id": sid, "text": text }),
            )
            .await?;
            unwrap_ipc_result(resp)
        }
        "zenko_session_background" => {
            let sid = args
                .get("session_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing `session_id`"))?;
            let resp = ipc_call(
                cfg,
                IPC_METHOD_SESSION_BACKGROUND,
                json!({ "session_id": sid }),
            )
            .await?;
            unwrap_ipc_result(resp)
        }
        "zenko_orchestrator_triage" => {
            let prompt = args
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let mode = args.get("mode").and_then(Value::as_str).map(String::from);
            match ipc_call(cfg, IPC_METHOD_ORCHESTRATOR_TRIAGE, args).await {
                Ok(resp) => unwrap_ipc_result(resp),
                Err(_) => {
                    let report = zenko_runtime::orchestrator::triage::classify_turn(
                        &zenko_runtime::orchestrator::triage::TriageInput {
                            prompt: &prompt,
                            prior_turn_errors: 0,
                            is_first_turn: true,
                            permission_mode: mode
                                .as_deref()
                                .map(PermissionMode::from_str_loose)
                                .unwrap_or(cfg.policy.mode),
                            has_fast_model: !cfg.provider.fast_model.trim().is_empty(),
                            has_plan_model: !cfg.provider.plan_model.trim().is_empty(),
                        },
                    );
                    Ok(serde_json::to_value(report)?)
                }
            }
        }
        "zenko_clarifications_list" => {
            let resp = ipc_call(cfg, IPC_METHOD_CLARIFICATIONS_LIST, args).await?;
            unwrap_ipc_result(resp)
        }
        "zenko_clarification_resolve" => {
            let resp = ipc_call(cfg, IPC_METHOD_CLARIFICATION_RESOLVE, args).await?;
            unwrap_ipc_result(resp)
        }
        "zenko_session_tasks_list" => {
            let sid = args
                .get("session_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing `session_id`"))?
                .to_string();
            let ready_only = args
                .get("ready_only")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            match ipc_call(cfg, IPC_METHOD_TASKS_DAG_LIST, args).await {
                Ok(resp) => unwrap_ipc_result(resp),
                Err(_) => {
                    let store = SqliteStore::open(cfg.db_path())?;
                    let tasks = store.list_session_tasks(&sid)?;
                    let out = match ready_only {
                        true => zenko_runtime::orchestrator::compute_ready_session_tasks(&tasks),
                        false => tasks,
                    };
                    Ok(serde_json::to_value(out)?)
                }
            }
        }
        "zenko_checkpoints_list" => {
            let sid = args
                .get("session_id")
                .and_then(Value::as_str)
                .map(String::from);
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(zenko_core::CHECKPOINT_LIST_DEFAULT_LIMIT as u64)
                as usize;
            match ipc_call(cfg, IPC_METHOD_CHECKPOINTS_LIST, args).await {
                Ok(resp) => unwrap_ipc_result(resp),
                Err(_) => {
                    let store = SqliteStore::open(cfg.db_path())?;
                    Ok(serde_json::to_value(
                        store.list_checkpoints(sid.as_deref(), limit)?,
                    )?)
                }
            }
        }
        "zenko_session_rewind" => {
            let cp_id = args
                .get("checkpoint_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing `checkpoint_id`"))?
                .to_string();
            match ipc_call(cfg, IPC_METHOD_CHECKPOINTS_REWIND, args.clone()).await {
                Ok(resp) => unwrap_ipc_result(resp),
                Err(e) if is_daemon_offline(&e) => {
                    let rt = get_or_build_runtime(lazy_runtime, config_path, cfg)?;
                    let mode = args
                        .get("mode")
                        .and_then(Value::as_str)
                        .map(RewindMode::from_str_loose)
                        .unwrap_or(RewindMode::CodeAndConversation);
                    let files = args
                        .get("files")
                        .and_then(Value::as_array)
                        .map(|arr| {
                            arr.iter()
                                .filter_map(Value::as_str)
                                .map(String::from)
                                .collect()
                        })
                        .unwrap_or_default();
                    let overwrite_manual_edits = args
                        .get("overwrite_manual_edits")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let summarize_abandoned = args
                        .get("summarize_abandoned")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    let cp_cfg = rt.config().checkpoints;
                    let rep = rt
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
                        .await?;
                    Ok(serde_json::to_value(rep)?)
                }
                Err(e) => Err(e),
            }
        }
        "zenko_plan_approve" => {
            let sid = args
                .get("session_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing `session_id`"))?
                .to_string();
            let mode = args
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("autonomous")
                .to_string();
            match ipc_call(cfg, IPC_METHOD_SESSION_APPROVE_PLAN, args).await {
                Ok(resp) => unwrap_ipc_result(resp),
                Err(_) => {
                    let rt = get_or_build_runtime(lazy_runtime, config_path, cfg)?;
                    let (sess, task_count) =
                        rt.approve_session_plan(&sid, PermissionMode::from_str_loose(&mode))?;
                    Ok(json!({ "session": sess, "task_count": task_count }))
                }
            }
        }
        "zenko_mcp_servers_list" => match ipc_call(cfg, IPC_METHOD_MCP_SERVERS_LIST, json!({})).await {
            Ok(resp) => unwrap_ipc_result(resp),
            Err(_) => {
                let rt = get_or_build_runtime(lazy_runtime, config_path, cfg)?;
                Ok(json!({
                    "servers": rt.mcp_clients.list_status().await,
                    "configs": rt.mcp_clients.configs(),
                }))
            }
        },
        "zenko_lsp_status" => match ipc_call(cfg, IPC_METHOD_LSP_STATUS, json!({})).await {
            Ok(resp) => {
                let mut val = unwrap_ipc_result(resp)?;
                if let Some(obj) = val.as_object_mut() {
                    obj.insert("registry".into(), serde_json::to_value(LSP_SERVER_REGISTRY)?);
                }
                Ok(val)
            }
            Err(_) => Ok(json!({
                "enabled": cfg.lsp.enabled,
                "idle_timeout_secs": cfg.lsp.idle_timeout_secs,
                "registry": LSP_SERVER_REGISTRY,
                "active": [],
            })),
        },
        "zenko_ask" => {
            let prompt = args
                .get("prompt")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing `prompt`"))?;
            let session_id = args.get("session_id").and_then(Value::as_str);
            let workspace = args.get("workspace").and_then(Value::as_str);
            let model = args.get("model").and_then(Value::as_str);
            let mode = args.get("mode").and_then(Value::as_str);

            match ipc_call(
                cfg,
                IPC_METHOD_ASK,
                json!({
                    "prompt": prompt,
                    "session_id": session_id,
                    "workspace": workspace,
                    "model": model,
                    "mode": mode,
                    "headless_no_wait": true,
                }),
            )
            .await
            {
                Ok(resp) => return unwrap_ipc_result(resp),
                Err(e) if is_daemon_offline(&e) => {}
                Err(e) => return Err(e),
            }

            let rt = get_or_build_runtime(lazy_runtime, config_path, cfg)?;
            let perm = mode.map(PermissionMode::from_str_loose);
            let sid = match session_id {
                Some(id) => id.to_string(),
                None => rt
                    .store
                    .create_session("MCP Session", "mcp", None, None, perm.unwrap_or(cfg.policy.mode))?
                    .id,
            };
            let turn = rt
                .run_turn_with_options(
                    &sid,
                    prompt,
                    TurnOptions {
                        parent_event_override: None,
                        workspace_override: workspace.map(PathBuf::from),
                        model_override: model.map(String::from),
                        permission_override: perm,
                        headless_no_wait: true,
                        ..TurnOptions::default()
                    },
                )
                .await?;
            Ok(serde_json::to_value(turn)?)
        }
        "zenko_skills_list" => {
            if let Ok(resp) = ipc_call(cfg, IPC_METHOD_SKILLS_LIST, json!({})).await {
                return unwrap_ipc_result(resp);
            }
            let mgr = SnapshotManager::new(cfg.resolved_data_dir(), cfg.resolved_workspace_dir())
                .with_hub_dir(cfg.resolved_hub_dir());
            Ok(serde_json::to_value(mgr.list_skills()?)?)
        }
        "zenko_skill_approve" => {
            let slug = args
                .get("slug")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing `slug`"))?;
            if let Ok(resp) = ipc_call(cfg, IPC_METHOD_SKILL_APPROVE, json!({ "slug": slug })).await
            {
                return unwrap_ipc_result(resp);
            }
            let mgr = SnapshotManager::new(cfg.resolved_data_dir(), cfg.resolved_workspace_dir())
                .with_hub_dir(cfg.resolved_hub_dir());
            let (doc, snap) = mgr.approve_skill(slug, cfg.skills.max_history_snapshots)?;
            Ok(json!({ "skill": doc, "snapshot": snap }))
        }
        "zenko_goals_list" => {
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(GOAL_LOOP_LIST_DEFAULT_LIMIT as u64) as usize;
            if let Ok(resp) = ipc_call(cfg, IPC_METHOD_GOALS_LIST, json!({ "limit": limit })).await
            {
                return unwrap_ipc_result(resp);
            }
            let store = SqliteStore::open(cfg.db_path())?;
            Ok(serde_json::to_value(store.list_goal_loops(limit)?)?)
        }
        "zenko_goal_create" => {
            let req: GoalLoopCreateRequest = serde_json::from_value(args)?;
            match ipc_call(cfg, IPC_METHOD_GOAL_CREATE, serde_json::to_value(&req)?).await {
                Ok(resp) => return unwrap_ipc_result(resp),
                Err(e) if is_daemon_offline(&e) => {}
                Err(e) => return Err(e),
            }
            let rt = get_or_build_runtime(lazy_runtime, config_path, cfg)?;
            Ok(serde_json::to_value(rt.create_goal_loop(req)?)?)
        }
        "zenko_export_session" => {
            let sid = args
                .get("session_id")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| anyhow::anyhow!("missing `session_id`"))?;
            let opts = ExportOptions {
                format: ExportFormat::from_str_loose(
                    args.get("format").and_then(Value::as_str).unwrap_or("jsonl"),
                ),
                anonymize_paths: args
                    .get("anonymize_paths")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                include_thinking: args
                    .get("include_thinking")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                ..ExportOptions::default()
            };
            if let Ok(resp) = ipc_call(
                cfg,
                IPC_METHOD_EXPORT_SESSION,
                json!({ "session_id": sid, "options": opts }),
            )
            .await
            {
                return unwrap_ipc_result(resp);
            }
            let store = SqliteStore::open(cfg.db_path())?;
            let vault = SecretVault::open(cfg.vault_path(), cfg.vault_key_path());
            let home = zenko_core::expand_tilde("~");
            let rep = zenko_runtime::export_session(
                &store,
                sid,
                &opts,
                &vault.all_values(),
                Some(&home),
            )?;
            Ok(serde_json::to_value(rep)?)
        }
        other => anyhow::bail!("unknown MCP tool: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mcp_protocol_handles_initialize_ping_and_tools_list() {
        let dir = std::env::temp_dir().join(format!("zenko_mcp_test_{}", new_id()));
        let cfg_path = dir.join("zenko.toml");
        let cfg = ZenkoConfig::default();
        let mut rt = None;

        let init = dispatch_mcp_request(&cfg_path, &cfg, &mut rt, json!(1), "initialize", json!({}))
            .await;
        assert_eq!(init["result"]["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert_eq!(init["result"]["serverInfo"]["name"], MCP_SERVER_NAME);

        let list = dispatch_mcp_request(&cfg_path, &cfg, &mut rt, json!(2), "tools/list", json!({}))
            .await;
        let tools = list["result"]["tools"].as_array().expect("tools array");
        assert!(tools.iter().any(|t| t["name"] == "zenko_ask"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_memory_search"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_cron_create"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_logs_query"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_jobs_list"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_jobs_start"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_jobs_delete"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_jobs_wait"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_cron_update"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_hub_list"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_session_steer"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_session_background"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_orchestrator_triage"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_clarifications_list"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_clarification_resolve"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_session_tasks_list"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_checkpoints_list"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_session_rewind"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_plan_approve"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_mcp_servers_list"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_lsp_status"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_skills_list"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_skill_approve"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_goals_list"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_goal_create"));
        assert!(tools.iter().any(|t| t["name"] == "zenko_export_session"));

        let ping = dispatch_mcp_request(&cfg_path, &cfg, &mut rt, json!(3), "ping", json!({})).await;
        assert_eq!(ping["id"], json!(3));
        assert!(ping.get("error").is_none());
    }
}
