//! Developer Experience & Debugging Toolkit for Zenko:
//! - `zenko trace <session_id>` — turn-by-turn execution waterfall (tokens, cache, tools, diffs, logs)
//! - `zenko debug tool <name> [JSON]` — invoke any tool directly without spending LLM tokens
//! - `zenko debug policy "<cmd>"` — inspect POSIX shell lexer tokens & ExecPolicy verdict
//! - `zenko debug prompt` — inspect frozen system prompt snapshot, hash, and token breakdown
//! - `zenko debug logs-dir` — inspect categorized JSONL files, SQLite counts, and wire debug status
//! - `zenko debug bundle` — generate a secret-scrubbed JSON diagnostic bundle for bug reports

use crate::ipc::{ipc_call, unwrap_ipc_result};
use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    EventKind, EventRole, IPC_METHOD_STATUS, JobSupervisor, LOG_ALL_FILE, LOG_CATEGORIES,
    LOG_CRASH_FILE, LOG_DIR_NAME, LOG_SESSIONS_SUBDIR, LOG_WIRE_SUBDIR, LogFilter,
    NOTE_KIND_BRAINSTORM_SPEC, NOTE_KIND_CLARIFICATION_DECISION, NOTE_KIND_DEFERRED_TOOLS,
    NOTE_KIND_EXECUTION_PLAN, PermissionMode, SecretVault, ToolContext, ZenkoConfig,
    compact_preview, diff_line_stats, format_latency, format_tokens, scrub_json, scrub_secrets,
};
use zenko_memory::{SnapshotManager, SqliteStore};
use zenko_runtime::{
    ClarificationHub, LocalJobSupervisor, inspect_shell_policy, orchestrator::triage,
    wire_debug_enabled,
};
use zenko_tools::ToolRegistry;

pub async fn run_trace(config_path: &Path, session_id: &str, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let store = SqliteStore::open(cfg.db_path())?;
    let session = store
        .get_session(session_id)?
        .ok_or_else(|| anyhow::anyhow!("session `{session_id}` not found"))?;
    let events = store.get_active_branch(session_id)?;
    let logs = store.query_logs(&LogFilter {
        session_id: Some(session_id.to_string()),
        limit: Some(250),
        ..Default::default()
    })?;

    let session_log_file = cfg
        .resolved_data_dir()
        .join(LOG_DIR_NAME)
        .join(LOG_SESSIONS_SUBDIR)
        .join(format!("{session_id}.jsonl"));

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "session": session,
                "events": events,
                "logs": logs,
                "session_log_file": session_log_file.display().to_string(),
                "session_log_exists": session_log_file.exists(),
            }))?
        );
        return Ok(());
    }

    println!(
        "Zenko Execution Trace · {} ({})",
        session.id, session.title
    );
    println!(
        "  Channel: {} · Status: {} · Policy: {} · Tokens: {} · Cost: ${:.4}",
        session.channel,
        session.status.as_str(),
        session.permission_mode.as_str(),
        format_tokens(session.total_tokens),
        session.total_cost_usd
    );
    if let Some(ref hash) = session.frozen_system_hash {
        println!("  Prompt Hash: {hash}");
    }
    println!(
        "  Session Log: {} ({})",
        session_log_file.display(),
        match session_log_file.exists() {
            true => "present",
            false => "not yet written",
        }
    );
    println!();

    for (idx, ev) in events.iter().enumerate() {
        let ts = ev.created_at.format("%H:%M:%S");
        match ev.role {
            EventRole::User => {
                let text = ev
                    .content_json
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                println!(
                    "#{:02} {ts}  <-- USER   \"{}\"",
                    idx + 1,
                    compact_preview(text, 96)
                );
            }
            EventRole::Assistant => {
                let model = ev
                    .content_json
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or("-");
                let thinking_len = ev
                    .content_json
                    .get("thinking")
                    .and_then(Value::as_str)
                    .map(|s| s.chars().count())
                    .unwrap_or(0);
                let text = ev
                    .content_json
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let tool_calls = ev
                    .content_json
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .map(|a| a.len())
                    .unwrap_or(0);
                let cache_pct = match (ev.tokens_in > 0, ev.cache_read_tokens > 0) {
                    (true, true) => {
                        format!(
                            " ({}% cache)",
                            ev.cache_read_tokens.saturating_mul(100) / ev.tokens_in
                        )
                    }
                    _ => String::new(),
                };
                println!(
                    "#{:02} {ts}  --> LLM    {model} · in={} out={}{cache_pct} · ${:.4} · tools={tool_calls} · think={thinking_len}ch",
                    idx + 1,
                    format_tokens(ev.tokens_in),
                    format_tokens(ev.tokens_out),
                    ev.cost_usd
                );
                if !text.trim().is_empty() {
                    println!("             reply: \"{}\"", compact_preview(text, 108));
                }
            }
            EventRole::Tool => {
                let name = ev
                    .content_json
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let is_err = ev
                    .content_json
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let dur = ev
                    .content_json
                    .get("duration_ms")
                    .and_then(Value::as_u64)
                    .map(|ms| format_latency(Duration::from_millis(ms)))
                    .unwrap_or_else(|| "-".into());
                let diff_tag = ev
                    .content_json
                    .get("diff")
                    .and_then(Value::as_str)
                    .map(|d| {
                        let (a, r) = diff_line_stats(d);
                        format!(" (+{a}/-{r})")
                    })
                    .unwrap_or_default();
                let status = match is_err {
                    true => "ERR",
                    false => "OK ",
                };
                let preview = ev
                    .content_json
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|s| compact_preview(s, 84))
                    .unwrap_or_default();
                println!(
                    "#{:02} {ts}  ··· TOOL   {name:<16} {status}  {dur}{diff_tag}  {preview}",
                    idx + 1
                );
            }
            EventRole::System => match ev.kind {
                EventKind::CompactionCheckpoint => {
                    let cnt = ev.content_json["compacted_events_count"]
                        .as_u64()
                        .unwrap_or(0);
                    let before = ev.content_json["estimated_tokens_before"]
                        .as_u64()
                        .unwrap_or(0);
                    let after = ev.content_json["estimated_tokens_after"]
                        .as_u64()
                        .unwrap_or(0);
                    let model = ev.content_json["model"].as_str().unwrap_or("-");
                    println!(
                        "#{:02} {ts}  ··· COMPACT {cnt} events ({} -> {} tok, model={model})",
                        idx + 1,
                        format_tokens(before),
                        format_tokens(after)
                    );
                }
                _ => match ev.content_json.get("note_kind").and_then(Value::as_str) {
                    Some(NOTE_KIND_BRAINSTORM_SPEC) => {
                        let spec = &ev.content_json["spec"];
                        let problem = spec["problem"].as_str().unwrap_or("-");
                        let selected = spec["selected_option_id"].as_str().unwrap_or("-");
                        let opt_count = spec["options"].as_array().map(|a| a.len()).unwrap_or(0);
                        println!(
                            "#{:02} {ts}  ··· PLAN   brainstorm ({opt_count} options -> {selected}) · {}",
                            idx + 1,
                            compact_preview(problem, 80)
                        );
                    }
                    Some(NOTE_KIND_EXECUTION_PLAN) => {
                        let plan = &ev.content_json["plan"];
                        let goal = plan["goal"].as_str().unwrap_or("-");
                        let steps = plan["steps"].as_array();
                        let total = steps.map(|a| a.len()).unwrap_or(0);
                        let done = steps
                            .map(|a| {
                                a.iter()
                                    .filter(|s| s["status"].as_str() == Some("completed"))
                                    .count()
                            })
                            .unwrap_or(0);
                        println!(
                            "#{:02} {ts}  ··· PLAN   steps ({done}/{total} done) · {}",
                            idx + 1,
                            compact_preview(goal, 80)
                        );
                    }
                    Some(NOTE_KIND_CLARIFICATION_DECISION) => {
                        let dec = &ev.content_json["decision"];
                        let q = dec["question"].as_str().unwrap_or("-");
                        let sel = dec["selected_option_id"].as_str().unwrap_or("-");
                        let by = dec["resolved_by"].as_str().unwrap_or("-");
                        println!(
                            "#{:02} {ts}  ··· DECIDE clarify ({by} -> {sel}) · {}",
                            idx + 1,
                            compact_preview(q, 76)
                        );
                    }
                    Some(NOTE_KIND_DEFERRED_TOOLS) => {
                        let names: Vec<&str> = ev.content_json["tools"]
                            .as_array()
                            .map(|a| a.iter().filter_map(Value::as_str).collect())
                            .unwrap_or_default();
                        println!(
                            "#{:02} {ts}  ··· TOOLS  activated deferred [{}]",
                            idx + 1,
                            names.join(", ")
                        );
                    }
                    _ => {
                        println!("#{:02} {ts}  ··· SYSTEM {}", idx + 1, ev.content_json);
                    }
                },
            },
        }
    }

    if !logs.is_empty() {
        println!("\nCorrelated Session Logs ({}):", logs.len());
        for entry in logs.iter().rev().take(20).rev() {
            println!(
                "  {} [{}] {}",
                entry.timestamp.format("%H:%M:%S"),
                entry.target,
                entry.message.replace('\t', "  ")
            );
        }
    }

    Ok(())
}

pub async fn run_debug_tool(
    config_path: &Path,
    tool_name: &str,
    args_raw: Option<&str>,
    workspace: Option<PathBuf>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data_dir = cfg.resolved_data_dir();
    let hub_dir = cfg.ensure_hub_layout().unwrap_or_else(|_| cfg.resolved_hub_dir());
    let workspace_dir = workspace.unwrap_or_else(|| cfg.resolved_workspace_dir());
    let store = SqliteStore::open(cfg.db_path())?;
    let snapshots = Arc::new(SnapshotManager::new(&data_dir, &workspace_dir).with_hub_dir(&hub_dir));
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()?;
    let (ev_tx, _) = tokio::sync::broadcast::channel(16);
    let jobs = LocalJobSupervisor::new(&data_dir, store.clone(), ev_tx.clone(), None);
    let clarifications = ClarificationHub::new(store.clone(), ev_tx);
    let registry = ToolRegistry::default_suite(
        http,
        store,
        snapshots,
        Arc::new(Notify::new()),
        Some(jobs as Arc<dyn JobSupervisor>),
        Some(clarifications as Arc<dyn zenko_core::OrchestratorBridge>),
    );

    if tool_name == "list" {
        let specs = registry.specs();
        match json_out {
            true => println!("{}", serde_json::to_string_pretty(&specs)?),
            false => {
                println!("Available Zenko Tools ({}):", specs.len());
                for s in specs {
                    println!("  {:<20} [{}]  {}", s.name, s.risk.as_str(), s.description);
                }
            }
        }
        return Ok(());
    }

    let Some(tool) = registry.get(tool_name) else {
        let names: Vec<String> = registry.specs().into_iter().map(|s| s.name).collect();
        anyhow::bail!("unknown tool `{tool_name}` (available: {})", names.join(", "));
    };

    let args: Value = match args_raw {
        Some(raw) if !raw.trim().is_empty() => {
            serde_json::from_str(raw).context("tool arguments must be valid JSON")?
        }
        _ => json!({}),
    };

    let ctx = ToolContext {
        session_id: "debug-cli".into(),
        workspace_dir,
        data_dir,
        hub_dir,
        shell_timeout_secs: cfg.policy.shell_timeout_secs,
        auto_background_after_secs: cfg.policy.auto_background_after_secs,
        max_output_bytes: cfg.policy.max_output_bytes,
        headless_no_wait: true,
        demote_bg: Arc::new(Notify::new()),
        file_state: None,
    };

    let started = Instant::now();
    let output = tool
        .execute(&ctx, args, CancellationToken::new())
        .await?;
    let elapsed = started.elapsed();

    match json_out {
        true => println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "tool": tool_name,
                "duration_ms": elapsed.as_millis() as u64,
                "is_error": output.is_error,
                "content": output.content,
                "diff": output.diff,
                "metadata": output.metadata,
            }))?
        ),
        false => {
            println!(
                "Tool `{tool_name}` · {} · {}",
                match output.is_error {
                    true => "ERROR",
                    false => "OK",
                },
                format_latency(elapsed)
            );
            println!("{}", output.content);
            if let Some(diff) = output.diff {
                println!("\n--- diff ---\n{diff}");
            }
        }
    }
    Ok(())
}

pub fn run_debug_triage(
    config_path: &Path,
    prompt: &str,
    mode_override: Option<&str>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let mode = mode_override
        .map(PermissionMode::from_str_loose)
        .unwrap_or(cfg.policy.mode);
    let report = triage::classify_turn(&triage::TriageInput {
        prompt,
        prior_turn_errors: 0,
        is_first_turn: true,
        permission_mode: mode,
        has_fast_model: !cfg.provider.fast_model.trim().is_empty(),
        has_plan_model: !cfg.provider.plan_model.trim().is_empty(),
    });
    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&report)?),
        false => {
            println!("Zenko Orchestrator Triage ({})", report.tier.as_str().to_uppercase());
            println!("  Intent:       {}", report.intent.as_str());
            println!("  Deliberation: {} (risk={})", report.deliberation.as_str(), report.risk_score);
            println!("  Complexity:   {}", report.complexity_score);
            println!("  Ambiguity:    {}", report.ambiguity_score);
            println!("  Model Role:   {}", report.recommended_model_role.as_str());
            println!("  Effort:       {}", report.recommended_effort.as_str());
            println!("  Auto Recon:   {}", report.needs_recon);
            println!("  Brainstorm:   {}", report.needs_brainstorm);
            println!("  Signals:      {}", report.matched_signals.join(", "));
            println!("  Reason:       {}", report.reason);
        }
    }
    Ok(())
}

pub fn run_debug_policy(
    config_path: &Path,
    command: &str,
    mode_override: Option<&str>,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let mode = mode_override
        .map(PermissionMode::from_str_loose)
        .unwrap_or(cfg.policy.mode);
    let report = inspect_shell_policy(mode, &cfg.policy, command);

    match json_out {
        true => println!("{}", serde_json::to_string_pretty(&report)?),
        false => {
            println!("ExecPolicy Inspection ({})", report.mode);
            println!("  Command:     {:?}", report.command);
            println!("  Tokens:      {:?}", report.tokens);
            println!("  Control Ops: {}", report.has_control_ops);
            println!("  Safe Prefix: {}", report.matched_safe_prefix);
            println!("  Verdict:     {}", report.verdict.to_uppercase());
            if let Some(reason) = report.reason {
                println!("  Reason:      {reason}");
            }
        }
    }
    Ok(())
}

pub fn run_debug_prompt(
    config_path: &Path,
    workspace: Option<PathBuf>,
    show_full: bool,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let data_dir = cfg.resolved_data_dir();
    let hub_dir = cfg.ensure_hub_layout().unwrap_or_else(|_| cfg.resolved_hub_dir());
    let workspace_dir = workspace.unwrap_or_else(|| cfg.resolved_workspace_dir());
    let snapshots = SnapshotManager::new(&data_dir, &workspace_dir).with_hub_dir(&hub_dir);
    snapshots.ensure_defaults()?;
    let snap = snapshots.load_snapshot()?;

    let enabled_skills: Vec<_> = snap.skills.iter().filter(|s| s.enabled).collect();
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "workspace_dir": workspace_dir.display().to_string(),
                "prompt_hash": snap.prompt_hash,
                "total_tokens_estimate": snap.total_tokens_estimate,
                "sections": {
                    "soul_chars": snap.soul_md.len(),
                    "user_chars": snap.user_md.len(),
                    "memory_chars": snap.memory_md.len(),
                    "zenko_chars": snap.zenko_md.len(),
                    "enabled_skills": enabled_skills.len(),
                },
                "system_prompt": match show_full {
                    true => Some(snap.system_prompt),
                    false => None,
                }
            }))?
        );
        return Ok(());
    }

    println!("Zenko Frozen Prompt Snapshot");
    println!("  Workspace:   {}", workspace_dir.display());
    println!("  Prompt Hash: {}", snap.prompt_hash);
    println!(
        "  Est. Tokens: ~{} ({} bytes)",
        format_tokens(snap.total_tokens_estimate as u64),
        snap.system_prompt.len()
    );
    println!(
        "  Docs:        SOUL={}B · USER={}B · MEMORY={}B · ZENKO={}B",
        snap.soul_md.len(),
        snap.user_md.len(),
        snap.memory_md.len(),
        snap.zenko_md.len()
    );
    println!("  Skills:      {} enabled", enabled_skills.len());
    for sk in enabled_skills {
        println!("    - {} (~{} tok)", sk.slug, sk.token_estimate);
    }
    if show_full {
        println!("\n--- Frozen System Prompt ---\n{}", snap.system_prompt);
    }
    Ok(())
}

pub fn run_debug_logs_dir(config_path: &Path, json_out: bool) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let logs_dir = cfg.resolved_data_dir().join(LOG_DIR_NAME);
    let store = SqliteStore::open(cfg.db_path()).ok();
    let counts = store
        .as_ref()
        .and_then(|s| s.log_category_counts().ok())
        .unwrap_or_default();

    let session_files = count_files_with_ext(&logs_dir.join(LOG_SESSIONS_SUBDIR), "jsonl");
    let wire_files = count_files_with_ext(&logs_dir.join(LOG_WIRE_SUBDIR), "json");
    let crash_path = logs_dir.join(LOG_CRASH_FILE);
    let all_bytes = file_size(&logs_dir.join(LOG_ALL_FILE));

    let categories_info: Vec<Value> = LOG_CATEGORIES
        .iter()
        .map(|meta| {
            let full = logs_dir.join(meta.file);
            json!({
                "id": meta.id,
                "label": meta.label,
                "file": full.display().to_string(),
                "bytes": file_size(&full),
                "sqlite_rows": counts.get(meta.id).copied().unwrap_or(0),
                "description": meta.description,
            })
        })
        .collect();

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "logs_dir": logs_dir.display().to_string(),
                "all_jsonl_bytes": all_bytes,
                "per_session_files": session_files,
                "wire_capture_files": wire_files,
                "wire_debug_enabled": wire_debug_enabled(),
                "crash_log_exists": crash_path.exists(),
                "categories": categories_info,
            }))?
        );
        return Ok(());
    }

    println!("Zenko Categorized Log Storage: {}", logs_dir.display());
    println!(
        "  Unified Firehose:  {} ({:.1} KB)",
        logs_dir.join(LOG_ALL_FILE).display(),
        all_bytes as f64 / 1024.0
    );
    println!(
        "  Per-Session Files: {} files in {}/",
        session_files,
        logs_dir.join(LOG_SESSIONS_SUBDIR).display()
    );
    println!(
        "  Wire Capture:      {} ({wire_files} files in {}/)",
        match wire_debug_enabled() {
            true => "ENABLED (ZENKO_DEBUG_WIRE=1)",
            false => "off (set ZENKO_DEBUG_WIRE=1 to capture)",
        },
        logs_dir.join(LOG_WIRE_SUBDIR).display()
    );
    println!(
        "  Crash Log:         {}",
        match crash_path.exists() {
            true => format!("PRESENT ({})", crash_path.display()),
            false => "clean (no panics recorded)".into(),
        }
    );
    println!("\nCategories:");
    for meta in LOG_CATEGORIES {
        let full = logs_dir.join(meta.file);
        let kb = file_size(&full) as f64 / 1024.0;
        let rows = counts.get(meta.id).copied().unwrap_or(0);
        println!(
            "  {:<9} {:>6} rows  {:>7.1} KB  {}",
            meta.id,
            rows,
            kb,
            full.display()
        );
    }
    Ok(())
}

pub async fn run_debug_bundle(
    config_path: &Path,
    output_path: Option<PathBuf>,
    limit: usize,
    json_out: bool,
) -> Result<()> {
    let cfg = ZenkoConfig::load_or_default(config_path)?;
    let vault = SecretVault::open(cfg.vault_path(), cfg.vault_key_path());
    let secrets = vault.all_values();
    let store = SqliteStore::open(cfg.db_path())?;
    let logs_dir = cfg.resolved_data_dir().join(LOG_DIR_NAME);

    let daemon_status = match ipc_call(&cfg, IPC_METHOD_STATUS, json!({})).await {
        Ok(resp) => unwrap_ipc_result(resp).ok(),
        Err(_) => None,
    };

    let mut logs_by_cat = Map::new();
    for meta in LOG_CATEGORIES {
        let rows = store
            .query_logs(&LogFilter {
                category: Some(meta.id.to_string()),
                limit: Some(limit),
                ..Default::default()
            })
            .unwrap_or_default();
        logs_by_cat.insert(meta.id.to_string(), serde_json::to_value(rows)?);
    }

    let crash_tail = std::fs::read_to_string(logs_dir.join(LOG_CRASH_FILE))
        .ok()
        .map(|raw| {
            let cut = raw.len().saturating_sub(8_192);
            let start = raw.ceil_char_boundary(cut);
            scrub_secrets(&raw[start..], &secrets)
        });

    let raw_bundle = json!({
        "generated_at": Utc::now().to_rfc3339(),
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "config": cfg,
        "daemon_running": daemon_status.is_some(),
        "daemon_telemetry": daemon_status,
        "logs_dir": logs_dir.display().to_string(),
        "log_category_counts": store.log_category_counts().unwrap_or_default(),
        "wire_debug_enabled": wire_debug_enabled(),
        "crash_log_tail": crash_tail,
        "recent_sessions": store.list_sessions(25).unwrap_or_default(),
        "cron_jobs": store.list_cron_jobs().unwrap_or_default(),
        "logs_by_category": logs_by_cat,
    });
    let safe_bundle = scrub_json(raw_bundle, &secrets);

    let out_file = output_path.unwrap_or_else(|| {
        PathBuf::from(format!(
            "zenko-debug-bundle-{}.json",
            Utc::now().format("%Y%m%dT%H%M%SZ")
        ))
    });
    std::fs::write(&out_file, serde_json::to_vec_pretty(&safe_bundle)?)
        .with_context(|| format!("failed to write debug bundle to {}", out_file.display()))?;

    match json_out {
        true => println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": true,
                "bundle_path": out_file.display().to_string(),
                "bytes": file_size(&out_file),
                "scrubbed_secrets_count": secrets.len(),
            }))?
        ),
        false => {
            println!("Scrubbed diagnostic bundle written to {}", out_file.display());
            println!("  Size: {:.1} KB (all vault secrets & bearer tokens redacted)", file_size(&out_file) as f64 / 1024.0);
        }
    }
    Ok(())
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn count_files_with_ext(dir: &Path, ext: &str) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    rd.flatten()
        .filter(|e| {
            let p = e.path();
            p.extension().and_then(|x| x.to_str()) == Some(ext)
                && p.file_name().and_then(|n| n.to_str()) != Some("index.jsonl")
        })
        .count()
}
