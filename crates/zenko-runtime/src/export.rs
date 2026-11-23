//! Phase 8 Scrubbed Multi-Format Trajectory Exporter (`zenko export` + HTTP `/api/sessions/{id}/export`).
//!
//! Exports SQLite `session_events` joined with `goal_loop_runs` verification reward metadata into:
//! - `jsonl`: Prime Agent `verifiers` / OpenAI rollout format (`schema_version: zenko.trajectory.v1`)
//! - `sharegpt`: Hermes / Axolotl / Unsloth `conversations` format (`human`/`gpt`/`function_call`/`observation`)
//! - `markdown`: Codex / Oh My Pi human-readable chronological audit transcript
//!
//! Every exported string is unconditionally passed through `scrub_secrets` (Vault values + token
//! prefixes), with optional deterministic `<HOME>` / `<WORKSPACE>` path anonymization.

use serde_json::{Value, json};
use std::path::Path;
use zenko_core::{
    EXPORT_ANON_HOME, EXPORT_ANON_WORKSPACE, EXPORT_DEFAULT_BATCH_LIMIT, EXPORT_MAX_BATCH_LIMIT,
    EventKind, EventRole, ExportFormat, ExportOptions, ExportReport, GoalLoopRun, Result, Session,
    SessionEvent, TRAJECTORY_SCHEMA_VERSION, ZenkoError, scrub_secrets,
};
use zenko_memory::SqliteStore;

/// Exports a single session trajectory into the requested format, scrubbed of all Vault secrets.
pub fn export_session(
    store: &SqliteStore,
    session_id: &str,
    opts: &ExportOptions,
    known_secrets: &[String],
    home_dir: Option<&Path>,
) -> Result<ExportReport> {
    let session = store
        .get_session(session_id)?
        .ok_or_else(|| ZenkoError::NotFound(format!("session `{session_id}` not found")))?;
    let events = match opts.all_branches {
        true => store.get_all_session_events(&session.id)?,
        false => store.get_active_branch(&session.id)?,
    };
    let verification = store.latest_goal_run_for_session(&session.id).ok().flatten();
    let mut content = render_single_session(
        &session,
        &events,
        verification.as_ref(),
        opts,
        known_secrets,
        home_dir,
    );
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    Ok(ExportReport {
        format: opts.format.as_str().to_string(),
        sessions_exported: 1,
        events_exported: events.len(),
        bytes: content.len(),
        content,
    })
}

/// Batch-exports filtered sessions (`--all`, `--since`, `--channel`, `--verified-only`, `--min-turns`)
/// into streaming JSONL (`jsonl` / `sharegpt`) or multi-session Markdown.
pub fn export_batch(
    store: &SqliteStore,
    opts: &ExportOptions,
    known_secrets: &[String],
    home_dir: Option<&Path>,
) -> Result<ExportReport> {
    let limit = opts
        .limit
        .unwrap_or(EXPORT_DEFAULT_BATCH_LIMIT)
        .clamp(1, EXPORT_MAX_BATCH_LIMIT);
    let sessions = store.list_sessions_for_export(opts.since, opts.channel.as_deref(), limit)?;
    let mut chunks = Vec::new();
    let mut sessions_exported = 0usize;
    let mut events_exported = 0usize;

    for session in sessions {
        let verification = store.latest_goal_run_for_session(&session.id).ok().flatten();
        if opts.verified_only {
            let passed = verification
                .as_ref()
                .is_some_and(|v| v.verify_exit_code == 0 && v.reward >= 1.0);
            if !passed {
                continue;
            }
        }
        let events = match opts.all_branches {
            true => store.get_all_session_events(&session.id)?,
            false => store.get_active_branch(&session.id)?,
        };
        let user_turns = events
            .iter()
            .filter(|e| e.role == EventRole::User && e.kind == EventKind::Message)
            .count();
        if opts.min_turns > 0 && user_turns < opts.min_turns {
            continue;
        }
        if events.is_empty() {
            continue;
        }
        chunks.push(render_single_session(
            &session,
            &events,
            verification.as_ref(),
            opts,
            known_secrets,
            home_dir,
        ));
        sessions_exported += 1;
        events_exported += events.len();
    }

    let separator = match opts.format {
        ExportFormat::Markdown => "\n\n---\n\n",
        ExportFormat::Jsonl | ExportFormat::ShareGpt => "\n",
    };
    let mut content = chunks.join(separator);
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }

    Ok(ExportReport {
        format: opts.format.as_str().to_string(),
        sessions_exported,
        events_exported,
        bytes: content.len(),
        content,
    })
}

fn render_single_session(
    session: &Session,
    events: &[SessionEvent],
    verification: Option<&GoalLoopRun>,
    opts: &ExportOptions,
    known_secrets: &[String],
    home_dir: Option<&Path>,
) -> String {
    let ws = session.workspace_dir.as_deref();
    let clean = |raw: &str| sanitize_export_text(raw, known_secrets, opts.anonymize_paths, home_dir, ws);

    match opts.format {
        ExportFormat::Jsonl => render_openai_jsonl(session, events, verification, opts, &clean),
        ExportFormat::ShareGpt => render_sharegpt(session, events, verification, opts, &clean),
        ExportFormat::Markdown => render_markdown(session, events, verification, opts, &clean),
    }
}

fn render_openai_jsonl(
    session: &Session,
    events: &[SessionEvent],
    verification: Option<&GoalLoopRun>,
    opts: &ExportOptions,
    clean: &impl Fn(&str) -> String,
) -> String {
    let mut messages = Vec::new();
    let mut tool_calls_total = 0usize;

    for ev in events {
        match (ev.role, ev.kind) {
            (EventRole::User, EventKind::Message) => {
                let text = ev.content_json.get("text").and_then(Value::as_str).unwrap_or("");
                messages.push(json!({
                    "role": "user",
                    "content": clean(text),
                    "created_at": ev.created_at.to_rfc3339(),
                }));
            }
            (EventRole::Assistant, EventKind::Message) => {
                let text = ev.content_json.get("text").and_then(Value::as_str).unwrap_or("");
                let mut msg = json!({
                    "role": "assistant",
                    "content": clean(text),
                    "created_at": ev.created_at.to_rfc3339(),
                });
                if opts.include_thinking {
                    if let Some(thinking) = ev
                        .content_json
                        .get("reasoning_content")
                        .or_else(|| ev.content_json.get("thinking"))
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                    {
                        msg["reasoning_content"] = json!(clean(thinking));
                    }
                }
                if let Some(calls) = ev.content_json.get("tool_calls").and_then(Value::as_array) {
                    if !calls.is_empty() {
                        tool_calls_total += calls.len();
                        let scrubbed_calls: Vec<Value> = calls
                            .iter()
                            .map(|c| {
                                let name = c.get("name").and_then(Value::as_str).unwrap_or("");
                                let id = c.get("id").and_then(Value::as_str).unwrap_or("");
                                let args = c
                                    .get("arguments")
                                    .map(|a| clean(&a.to_string()))
                                    .unwrap_or_else(|| "{}".into());
                                json!({
                                    "id": id,
                                    "type": "function",
                                    "function": { "name": name, "arguments": args }
                                })
                            })
                            .collect();
                        msg["tool_calls"] = Value::Array(scrubbed_calls);
                    }
                }
                messages.push(msg);
            }
            (EventRole::Assistant, EventKind::Thinking) if opts.include_thinking => {
                if let Some(th) = extract_thinking_text(&ev.content_json) {
                    messages.push(json!({
                        "role": "assistant",
                        "content": "",
                        "reasoning_content": clean(th),
                        "created_at": ev.created_at.to_rfc3339(),
                    }));
                }
            }
            (EventRole::Assistant, EventKind::ToolCall) => {
                tool_calls_total += 1;
                let name = ev.content_json.get("name").and_then(Value::as_str).unwrap_or("");
                let call_id = ev
                    .content_json
                    .get("call_id")
                    .or_else(|| ev.content_json.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or(&ev.id);
                let args = ev
                    .content_json
                    .get("arguments")
                    .map(|a| clean(&a.to_string()))
                    .unwrap_or_else(|| "{}".into());
                messages.push(json!({
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "id": call_id,
                        "type": "function",
                        "function": { "name": name, "arguments": args }
                    }],
                    "created_at": ev.created_at.to_rfc3339(),
                }));
            }
            (EventRole::Tool, EventKind::ToolResult) => {
                let name = ev.content_json.get("name").and_then(Value::as_str).unwrap_or("");
                let call_id = ev
                    .content_json
                    .get("call_id")
                    .or_else(|| ev.content_json.get("tool_call_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let output = ev
                    .content_json
                    .get("output")
                    .or_else(|| ev.content_json.get("content"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "name": name,
                    "content": clean(output),
                    "is_error": ev.content_json.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                    "created_at": ev.created_at.to_rfc3339(),
                }));
            }
            (EventRole::System, EventKind::CompactionCheckpoint) => {
                if let Some(summary) = ev.content_json.get("summary").and_then(Value::as_str) {
                    messages.push(json!({
                        "role": "system",
                        "kind": "compaction_checkpoint",
                        "content": clean(summary),
                        "created_at": ev.created_at.to_rfc3339(),
                    }));
                }
            }
            _ => {}
        }
    }

    let verif_json = match verification {
        Some(v) => json!({
            "verified": v.verify_exit_code == 0,
            "exit_code": v.verify_exit_code,
            "reward": v.reward,
            "goal_id": v.goal_id,
            "iteration": v.iteration,
            "outcome": v.outcome,
        }),
        None => Value::Null,
    };

    let record = json!({
        "schema_version": TRAJECTORY_SCHEMA_VERSION,
        "session_id": session.id,
        "title": clean(&session.title),
        "channel": session.channel,
        "model": session.model_override,
        "workspace": session.workspace_dir.as_deref().map(clean),
        "messages": messages,
        "usage": {
            "total_tokens": session.total_tokens,
            "total_cost_usd": session.total_cost_usd,
            "tool_calls": tool_calls_total,
        },
        "verification": verif_json,
        "created_at": session.created_at.to_rfc3339(),
        "updated_at": session.updated_at.to_rfc3339(),
    });
    serde_json::to_string(&record).unwrap_or_else(|_| "{}".into())
}

fn render_sharegpt(
    session: &Session,
    events: &[SessionEvent],
    verification: Option<&GoalLoopRun>,
    opts: &ExportOptions,
    clean: &impl Fn(&str) -> String,
) -> String {
    let mut conversations = Vec::new();

    for ev in events {
        match (ev.role, ev.kind) {
            (EventRole::User, EventKind::Message) => {
                let text = ev.content_json.get("text").and_then(Value::as_str).unwrap_or("");
                conversations.push(json!({ "from": "human", "value": clean(text) }));
            }
            (EventRole::Assistant, EventKind::Message) => {
                let text = clean(ev.content_json.get("text").and_then(Value::as_str).unwrap_or(""));
                let value = match opts.include_thinking {
                    true => match ev
                        .content_json
                        .get("reasoning_content")
                        .or_else(|| ev.content_json.get("thinking"))
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                    {
                        Some(th) => format!("<think>\n{}\n</think>\n{text}", clean(th)),
                        None => text,
                    },
                    false => text,
                };
                if !value.trim().is_empty() {
                    conversations.push(json!({ "from": "gpt", "value": value }));
                }
                if let Some(calls) = ev.content_json.get("tool_calls").and_then(Value::as_array) {
                    for c in calls {
                        let name = c.get("name").and_then(Value::as_str).unwrap_or("");
                        let args = c.get("arguments").cloned().unwrap_or(json!({}));
                        let payload = clean(&json!({"name": name, "arguments": args}).to_string());
                        conversations.push(json!({ "from": "function_call", "value": payload }));
                    }
                }
            }
            (EventRole::Assistant, EventKind::Thinking) if opts.include_thinking => {
                if let Some(th) = extract_thinking_text(&ev.content_json) {
                    conversations.push(json!({
                        "from": "gpt",
                        "value": format!("<think>\n{}\n</think>", clean(th)),
                    }));
                }
            }
            (EventRole::Assistant, EventKind::ToolCall) => {
                let name = ev.content_json.get("name").and_then(Value::as_str).unwrap_or("");
                let args = ev.content_json.get("arguments").cloned().unwrap_or(json!({}));
                let payload = clean(&json!({"name": name, "arguments": args}).to_string());
                conversations.push(json!({ "from": "function_call", "value": payload }));
            }
            (EventRole::Tool, EventKind::ToolResult) => {
                let output = ev
                    .content_json
                    .get("output")
                    .or_else(|| ev.content_json.get("content"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                conversations.push(json!({ "from": "observation", "value": clean(output) }));
            }
            _ => {}
        }
    }

    let record = json!({
        "id": session.id,
        "conversations": conversations,
        "metadata": {
            "schema_version": TRAJECTORY_SCHEMA_VERSION,
            "channel": session.channel,
            "model": session.model_override,
            "total_tokens": session.total_tokens,
            "total_cost_usd": session.total_cost_usd,
            "reward": verification.map(|v| v.reward),
            "verified": verification.map(|v| v.verify_exit_code == 0),
        }
    });
    serde_json::to_string(&record).unwrap_or_else(|_| "{}".into())
}

fn render_markdown(
    session: &Session,
    events: &[SessionEvent],
    verification: Option<&GoalLoopRun>,
    opts: &ExportOptions,
    clean: &impl Fn(&str) -> String,
) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str(&format!("# Session `{}` — {}\n\n", session.id, clean(&session.title)));
    out.push_str(&format!(
        "- **Channel:** {} · **Tokens:** {} · **Cost:** ${:.4}\n",
        session.channel, session.total_tokens, session.total_cost_usd
    ));
    if let Some(v) = verification {
        out.push_str(&format!(
            "- **Verification:** {} (exit_code={}, reward={:.1}, goal=`{}`)\n",
            v.outcome, v.verify_exit_code, v.reward, v.goal_id
        ));
    }
    out.push('\n');

    for ev in events {
        match (ev.role, ev.kind) {
            (EventRole::User, EventKind::Message) => {
                let text = ev.content_json.get("text").and_then(Value::as_str).unwrap_or("");
                out.push_str(&format!("## User ({})\n\n{}\n\n", ev.created_at.format("%H:%M:%S UTC"), clean(text)));
            }
            (EventRole::Assistant, EventKind::Message) => {
                let text = ev.content_json.get("text").and_then(Value::as_str).unwrap_or("");
                out.push_str(&format!("## Assistant ({})\n\n", ev.created_at.format("%H:%M:%S UTC")));
                if opts.include_thinking {
                    if let Some(th) = ev
                        .content_json
                        .get("reasoning_content")
                        .or_else(|| ev.content_json.get("thinking"))
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                    {
                        out.push_str(&format!("<details><summary>Reasoning</summary>\n\n{}\n\n</details>\n\n", clean(th)));
                    }
                }
                if !text.trim().is_empty() {
                    out.push_str(&format!("{}\n\n", clean(text)));
                }
            }
            (EventRole::Assistant, EventKind::Thinking) if opts.include_thinking => {
                if let Some(th) = extract_thinking_text(&ev.content_json) {
                    out.push_str(&format!(
                        "<details><summary>Reasoning ({})</summary>\n\n{}\n\n</details>\n\n",
                        ev.created_at.format("%H:%M:%S UTC"),
                        clean(th)
                    ));
                }
            }
            (EventRole::Assistant, EventKind::ToolCall) => {
                let name = ev.content_json.get("name").and_then(Value::as_str).unwrap_or("tool");
                let args = ev
                    .content_json
                    .get("arguments")
                    .map(|a| clean(&serde_json::to_string_pretty(a).unwrap_or_else(|_| a.to_string())))
                    .unwrap_or_else(|| "{}".into());
                let fence = markdown_code_fence(&args);
                out.push_str(&format!("### Tool Call: `{name}`\n{fence}json\n{args}\n{fence}\n\n"));
            }
            (EventRole::Tool, EventKind::ToolResult) => {
                let name = ev.content_json.get("name").and_then(Value::as_str).unwrap_or("tool");
                let output = ev
                    .content_json
                    .get("output")
                    .or_else(|| ev.content_json.get("content"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let cleaned = clean(output);
                let fence = markdown_code_fence(&cleaned);
                out.push_str(&format!(
                    "<details><summary>Tool Output: `{name}`</summary>\n\n{fence}\n{cleaned}\n{fence}\n</details>\n\n",
                ));
            }
            _ => {}
        }
    }
    out
}

fn extract_thinking_text(content_json: &Value) -> Option<&str> {
    content_json
        .get("reasoning_content")
        .or_else(|| content_json.get("thinking"))
        .or_else(|| content_json.get("text"))
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

/// Deterministic zero-regex CommonMark fence selector: chooses one more backtick than
/// the longest consecutive backtick run in `text` (minimum 3).
fn markdown_code_fence(text: &str) -> String {
    let mut max_run = 0usize;
    let mut cur = 0usize;
    for b in text.bytes() {
        match b {
            b'`' => {
                cur += 1;
                if cur > max_run {
                    max_run = cur;
                }
            }
            _ => cur = 0,
        }
    }
    "`".repeat(max_run.max(2) + 1)
}

fn sanitize_export_text(
    raw: &str,
    known_secrets: &[String],
    anonymize_paths: bool,
    home_dir: Option<&Path>,
    workspace_dir: Option<&str>,
) -> String {
    let mut s = scrub_secrets(raw, known_secrets);
    if anonymize_paths {
        if let Some(ws) = workspace_dir
            .map(|w| w.trim().trim_end_matches('/'))
            .filter(|w| w.len() > 2)
        {
            s = s.replace(ws, EXPORT_ANON_WORKSPACE);
        }
        if let Some(home) = home_dir {
            let home_raw = home.display().to_string();
            let home_s = home_raw.trim().trim_end_matches('/');
            if home_s.len() > 2 {
                s = s.replace(home_s, EXPORT_ANON_HOME);
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use zenko_core::{GoalLoopRecord, GoalLoopStatus, PermissionMode, UsageStats, new_id};

    #[test]
    fn exports_jsonl_sharegpt_and_markdown_with_mandatory_redaction_and_anonymization() {
        let dir = std::env::temp_dir().join(format!("zenko_export_{}", new_id()));
        let store = SqliteStore::open(dir.join("zenko.db")).expect("open db");
        let mut sess = store
            .create_session("Fix secret bug", "cli", None, None, PermissionMode::Balanced)
            .expect("session");
        sess.workspace_dir = Some("/home/alice/projects/zenko".into());
        store
            .set_session_workspace(&sess.id, "/home/alice/projects/zenko")
            .expect("ws");

        let e1 = store
            .append_event(
                &sess.id,
                None,
                EventRole::User,
                EventKind::Message,
                json!({"text": "Check /home/alice/projects/zenko/src/main.rs using token my_vault_secret_42"}),
                &UsageStats::default(),
            )
            .expect("e1");
        let e2 = store
            .append_event(
                &sess.id,
                Some(&e1.id),
                EventRole::Assistant,
                EventKind::ToolCall,
                json!({"call_id": "c1", "name": "read_file", "arguments": {"path": "/home/alice/projects/zenko/src/main.rs"}}),
                &UsageStats::default(),
            )
            .expect("e2");
        let e3 = store
            .append_event(
                &sess.id,
                Some(&e2.id),
                EventRole::Tool,
                EventKind::ToolResult,
                json!({"call_id": "c1", "name": "read_file", "output": "API=sk-or-abcdef12345678901234"}),
                &UsageStats::default(),
            )
            .expect("e3");
        let _e4 = store
            .append_event(
                &sess.id,
                Some(&e3.id),
                EventRole::Assistant,
                EventKind::Message,
                json!({"text": "Fixed safely.", "reasoning_content": "Inspecting my_vault_secret_42"}),
                &UsageStats::default(),
            )
            .expect("e4");

        // Record a verified goal loop run linked to this session
        let now = Utc::now();
        store
            .upsert_goal_loop(&GoalLoopRecord {
                id: "goal_exp1".into(),
                name: "Export Test".into(),
                session_id: sess.id.clone(),
                workspace_dir: "/home/alice/projects/zenko".into(),
                goal_text: "Fix".into(),
                verify_cmd: vec!["true".into()],
                interval_secs: 0,
                max_iterations: 3,
                max_budget_usd: 1.0,
                max_consecutive_stalls: 2,
                checkpoint_each_iter: false,
                auto_rewind_on_stall: false,
                model: None,
                notify_channel: "webui".into(),
                status: GoalLoopStatus::Succeeded,
                iterations_done: 1,
                consecutive_stalls: 0,
                cost_spent_usd: 0.01,
                last_verify_exit: Some(0),
                last_verify_hash: Some("ok".into()),
                last_verify_output: Some("ok".into()),
                next_run_at: None,
                created_at: now,
                updated_at: now,
            })
            .expect("goal");
        store
            .insert_goal_loop_run(&GoalLoopRun {
                id: new_id(),
                goal_id: "goal_exp1".into(),
                iteration: 1,
                checkpoint_id: None,
                verify_exit_code: 0,
                verify_output_excerpt: "ok".into(),
                verify_hash: "ok".into(),
                tokens_used: 100,
                cost_usd: 0.01,
                duration_ms: 50,
                outcome: "succeeded".into(),
                reward: 1.0,
                created_at: now,
            })
            .expect("run");

        let secrets = vec!["my_vault_secret_42".to_string()];
        let home = Path::new("/home/alice");

        for fmt in [ExportFormat::Jsonl, ExportFormat::ShareGpt, ExportFormat::Markdown] {
            let opts = ExportOptions {
                format: fmt,
                include_thinking: true,
                anonymize_paths: true,
                ..ExportOptions::default()
            };
            let rep = export_session(&store, &sess.id, &opts, &secrets, Some(home)).expect("export");
            assert_eq!(rep.sessions_exported, 1);
            assert!(!rep.content.contains("my_vault_secret_42"), "vault secret leaked in {fmt:?}");
            assert!(!rep.content.contains("sk-or-abcdef12345678901234"), "token prefix leaked in {fmt:?}");
            assert!(!rep.content.contains("/home/alice"), "home path not anonymized in {fmt:?}");
            assert!(rep.content.contains("<WORKSPACE>"), "workspace not anonymized in {fmt:?}");
        }

        // Batch verified-only export finds our verified session
        let batch = export_batch(
            &store,
            &ExportOptions {
                format: ExportFormat::ShareGpt,
                verified_only: true,
                min_turns: 1,
                ..ExportOptions::default()
            },
            &secrets,
            Some(home),
        )
        .expect("batch");
        assert_eq!(batch.sessions_exported, 1);
        assert!(batch.content.contains("\"reward\":1.0"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn export_handles_standalone_thinking_dynamic_markdown_fences_and_trailing_slashes() {
        let dir = std::env::temp_dir().join(format!("zenko_export_edge_{}", new_id()));
        let store = SqliteStore::open(dir.join("zenko.db")).expect("open db");
        let sess = store
            .create_session("Fence Test", "cli", None, None, PermissionMode::Balanced)
            .expect("session");
        // Trailing slash in stored workspace_dir must still anonymize paths cleanly
        store
            .set_session_workspace(&sess.id, "/home/alice/repo/")
            .expect("ws");

        let e1 = store
            .append_event(
                &sess.id,
                None,
                EventRole::Assistant,
                EventKind::Thinking,
                json!({"text": "Standalone reasoning inspecting /home/alice/repo/lib.rs"}),
                &UsageStats::default(),
            )
            .expect("e1");
        let _e2 = store
            .append_event(
                &sess.id,
                Some(&e1.id),
                EventRole::Tool,
                EventKind::ToolResult,
                json!({"call_id": "c1", "name": "read_file", "output": "```rust\nfn main() {}\n```"}),
                &UsageStats::default(),
            )
            .expect("e2");

        let home = Path::new("/home/alice/");
        let jsonl = export_session(
            &store,
            &sess.id,
            &ExportOptions {
                format: ExportFormat::Jsonl,
                include_thinking: true,
                anonymize_paths: true,
                ..ExportOptions::default()
            },
            &[],
            Some(home),
        )
        .expect("jsonl");
        assert!(jsonl.content.ends_with('\n'), "single-session jsonl must end with newline");
        assert!(jsonl.content.contains("Standalone reasoning inspecting <WORKSPACE>/lib.rs"));

        let md = export_session(
            &store,
            &sess.id,
            &ExportOptions {
                format: ExportFormat::Markdown,
                include_thinking: true,
                anonymize_paths: true,
                ..ExportOptions::default()
            },
            &[],
            Some(home),
        )
        .expect("md");
        assert!(md.content.contains("Standalone reasoning inspecting <WORKSPACE>/lib.rs"));
        assert!(
            md.content.contains("````\n```rust\nfn main() {}\n```\n````"),
            "markdown fence must dynamically widen to 4 backticks around triple backticks: {}",
            md.content
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}
