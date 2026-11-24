use async_trait::async_trait;
use chrono::Utc;
use croner::Cron;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    CronJob, CronKind, MEMORY_DEFAULT_SEARCH_LIMIT, MEMORY_SCOPE_GLOBAL, MemoryKind,
    PROJECT_MEMORY_DOC_NAME, Result, RiskLevel, SESSION_CONTEXT_DEFAULT_WINDOW,
    SESSION_CONTEXT_MAX_WINDOW, SESSION_SEARCH_DEFAULT_LIMIT, SESSION_SEARCH_MAX_LIMIT,
    SecretVault, SessionSearchFilter, TOOL_SESSION_SEARCH, Tool, ToolConcurrency, ToolContext,
    ToolExposure, ToolOutput, ToolSpec, ZenkoError, new_id,
};
use zenko_memory::{
    SnapshotManager, SqliteStore, parse_since_window, project_scope_for_workspace, resolve_git_root,
};

pub struct MemoryTool {
    store: SqliteStore,
    snapshots: Arc<SnapshotManager>,
    vault: Option<Arc<SecretVault>>,
}

impl MemoryTool {
    pub fn new(store: SqliteStore, snapshots: Arc<SnapshotManager>) -> Self {
        Self {
            store,
            snapshots,
            vault: None,
        }
    }

    pub fn new_with_vault(
        store: SqliteStore,
        snapshots: Arc<SnapshotManager>,
        vault: Arc<SecretVault>,
    ) -> Self {
        Self {
            store,
            snapshots,
            vault: Some(vault),
        }
    }

    fn known_secrets(&self) -> Vec<String> {
        self.vault
            .as_ref()
            .map(|v| v.all_values())
            .unwrap_or_default()
    }

    fn scoped_snapshots(&self, ctx: &ToolContext) -> SnapshotManager {
        let _ = &self.snapshots;
        SnapshotManager::new(&ctx.data_dir, &ctx.workspace_dir).with_hub_dir(&ctx.hub_dir)
    }
}

fn resolve_target_doc(action: &str, args: &Value) -> Result<&'static str> {
    let raw = match action {
        "append_user" => "USER.md",
        "append_memory" => "MEMORY.md",
        _ => args
            .get("doc")
            .and_then(Value::as_str)
            .unwrap_or("MEMORY.md"),
    };
    match raw.trim() {
        "USER.md" | "user" | "USER" => Ok("USER.md"),
        "MEMORY.md" | "memory" | "MEMORY" => Ok("MEMORY.md"),
        PROJECT_MEMORY_DOC_NAME | "project_memory" | "PROJECT_MEMORY" | "project" => {
            Ok(PROJECT_MEMORY_DOC_NAME)
        }
        "ZENKO.md" | "zenko" | "ZENKO" => Ok("ZENKO.md"),
        other => Err(ZenkoError::Tool(format!(
            "unsupported memory doc `{other}` (expected USER.md, MEMORY.md, PROJECT_MEMORY.md, or ZENKO.md)"
        ))),
    }
}

#[async_trait]
impl Tool for MemoryTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Memory
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        match args.get("action").and_then(Value::as_str) {
            Some("search" | "read_snapshot" | "history") => ToolConcurrency::Shared,
            _ => ToolConcurrency::Global,
        }
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "memory_tool".into(),
            description: "Manage Zenko's scoped persistent memory: hybrid RRF search with staleness checks, deduplicated add/update/delete entries, surgical `patch_doc` / guarded `append_doc` over USER.md/MEMORY.md/ZENKO.md, and versioned `.history/` rollback.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": [
                            "search",
                            "add_entry",
                            "update_entry",
                            "delete_entry",
                            "patch_doc",
                            "append_doc",
                            "append_user",
                            "append_memory",
                            "read_snapshot",
                            "history",
                            "rollback_doc"
                        ]
                    },
                    "id": { "type": "string", "description": "Memory entry ID (for update_entry / delete_entry)" },
                    "query": { "type": "string", "description": "Search query (for search)" },
                    "scope": { "type": "string", "description": "project (default for project/fact) | global (default for user/feedback) | all (for search)" },
                    "kind": { "type": "string", "enum": ["user", "feedback", "project", "reference", "fact", "lesson"] },
                    "title": { "type": "string" },
                    "content": { "type": "string" },
                    "salience": { "type": "number", "description": "Importance weight 0.1..3.0 (default 1.0)" },
                    "doc": { "type": "string", "enum": ["USER.md", "MEMORY.md", "PROJECT_MEMORY.md", "ZENKO.md"] },
                    "old_text": { "type": "string", "description": "Exact unique substring to replace (for patch_doc)" },
                    "new_text": { "type": "string", "description": "Replacement text (for patch_doc)" },
                    "snapshot_id": { "type": "string", "description": "History snapshot ID (for rollback_doc)" },
                    "limit": { "type": "integer", "description": "Max entries to return (default 10)" }
                },
                "required": ["action"]
            }),
            risk: RiskLevel::Safe,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `action`".into()))?;
        let proj_scope = project_scope_for_workspace(&ctx.workspace_dir);

        match action {
            "search" => {
                let query = args.get("query").and_then(Value::as_str).unwrap_or("");
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(MEMORY_DEFAULT_SEARCH_LIMIT as u64)
                    .clamp(1, 30) as usize;
                let (scope_opt, include_global) = match args
                    .get("scope")
                    .and_then(Value::as_str)
                    .unwrap_or("current")
                    .trim()
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "global" => (None, false),
                    "project" => (Some(proj_scope.as_str()), false),
                    "all" => (None, true),
                    _ => (Some(proj_scope.as_str()), true),
                };
                let hits = self.store.search_memory_hybrid(
                    query,
                    scope_opt,
                    include_global,
                    Some(&ctx.workspace_dir),
                    limit,
                )?;
                if hits.is_empty() {
                    return Ok(ToolOutput::ok("No matching memory entries found."));
                }
                let text = hits
                    .iter()
                    .map(|h| {
                        let m = &h.entry;
                        let stale_note = match h.stale_paths.is_empty() {
                            true => String::new(),
                            false => format!(
                                " [stale-path: {} missing — verify before trusting]",
                                h.stale_paths.join(", ")
                            ),
                        };
                        let cite = m
                            .source_session_id
                            .as_deref()
                            .map(|s| format!(" (session={})", zenko_core::truncate_utf8_prefix(s, 12)))
                            .unwrap_or_default();
                        format!(
                            "- [id={}] [{}][{}] {}: {}{}{}",
                            m.id, m.scope, m.kind, m.title, m.content, cite, stale_note
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(ToolOutput {
                    content: text,
                    is_error: false,
                    diff: None,
                    metadata: Some(json!({ "hits": hits })),
                })
            }
            "add_entry" => {
                let title = args.get("title").and_then(Value::as_str).unwrap_or("Note");
                let content = args
                    .get("content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("missing `content`".into()))?;
                let kind_raw = args.get("kind").and_then(Value::as_str).unwrap_or("project");
                let kind = MemoryKind::from_str_loose(kind_raw);
                let scope = match args.get("scope").and_then(Value::as_str).map(str::trim) {
                    Some("global") => MEMORY_SCOPE_GLOBAL.to_string(),
                    Some("project") => proj_scope,
                    Some(custom) if !custom.is_empty() => custom.to_string(),
                    _ => match kind {
                        MemoryKind::User | MemoryKind::Feedback => MEMORY_SCOPE_GLOBAL.to_string(),
                        _ => proj_scope,
                    },
                };
                let salience = args.get("salience").and_then(Value::as_f64).unwrap_or(1.0);
                let secrets = self.known_secrets();
                let res = self.store.upsert_memory_dedup_with_secrets(
                    &scope,
                    kind.as_str(),
                    title,
                    content,
                    salience,
                    Some(&ctx.session_id),
                    None,
                    true,
                    &secrets,
                )?;
                let verb = match res.merged {
                    true => format!(
                        "Merged with existing near-duplicate entry {} (similarity={:.0}%, scope={})",
                        res.entry.id,
                        res.similarity * 100.0,
                        res.entry.scope
                    ),
                    false => format!(
                        "Saved memory entry {} (scope={}, kind={})",
                        res.entry.id, res.entry.scope, res.entry.kind
                    ),
                };
                Ok(ToolOutput {
                    content: format!("{verb}: {}.", res.entry.title),
                    is_error: false,
                    diff: None,
                    metadata: Some(serde_json::to_value(&res).unwrap_or(Value::Null)),
                })
            }
            "update_entry" => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("missing `id`".into()))?;
                let scope_opt = args.get("scope").and_then(Value::as_str).map(|s| match s.trim() {
                    "project" => proj_scope.as_str(),
                    other => other,
                });
                let secrets = self.known_secrets();
                let updated = self.store.update_memory_with_secrets(
                    id,
                    args.get("title").and_then(Value::as_str),
                    args.get("content").and_then(Value::as_str),
                    args.get("kind").and_then(Value::as_str),
                    args.get("salience").and_then(Value::as_f64),
                    scope_opt,
                    &secrets,
                )?;
                match updated {
                    Some(entry) => Ok(ToolOutput {
                        content: format!("Updated memory entry {} ({}).", entry.id, entry.title),
                        is_error: false,
                        diff: None,
                        metadata: Some(serde_json::to_value(&entry).unwrap_or(Value::Null)),
                    }),
                    None => Ok(ToolOutput::err(format!("Memory entry `{id}` not found."))),
                }
            }
            "delete_entry" => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("missing `id`".into()))?;
                let deleted = self.store.delete_memory(id)?;
                Ok(match deleted {
                    true => ToolOutput::ok(format!("Deleted memory entry `{id}`.")),
                    false => ToolOutput::err(format!("Memory entry `{id}` not found.")),
                })
            }
            "patch_doc" => {
                let doc = resolve_target_doc(action, &args)?;
                let old_text = args
                    .get("old_text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("missing `old_text`".into()))?;
                let new_text = args
                    .get("new_text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("missing `new_text`".into()))?;
                let secrets = self.known_secrets();
                let mgr = self.scoped_snapshots(ctx);
                let (diff, snap) =
                    mgr.patch_doc_with_secrets(doc, old_text, new_text, "patch", &secrets)?;
                let snap_note = snap
                    .as_ref()
                    .map(|s| format!(" (rollback snapshot: `{}`)", s.id))
                    .unwrap_or_default();
                Ok(ToolOutput {
                    content: format!("Patched {doc}{snap_note}."),
                    is_error: false,
                    diff: Some(diff),
                    metadata: Some(json!({ "doc": doc, "snapshot": snap })),
                })
            }
            "append_doc" | "append_user" | "append_memory" => {
                let doc = resolve_target_doc(action, &args)?;
                let content = args
                    .get("content")
                    .or_else(|| args.get("text"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("missing `content`".into()))?;
                let secrets = self.known_secrets();
                let mgr = self.scoped_snapshots(ctx);
                let (diff, snap, added) =
                    mgr.append_doc_guarded_with_secrets(doc, content, "append", &secrets)?;
                match added {
                    true => {
                        let snap_note = snap
                            .as_ref()
                            .map(|s| format!(" (rollback snapshot: `{}`)", s.id))
                            .unwrap_or_default();
                        Ok(ToolOutput {
                            content: format!("Updated {doc}{snap_note}."),
                            is_error: false,
                            diff: Some(diff),
                            metadata: Some(json!({ "doc": doc, "snapshot": snap })),
                        })
                    }
                    false => Ok(ToolOutput::ok(format!(
                        "Skipped duplicate note already present in {doc}."
                    ))),
                }
            }
            "read_snapshot" => {
                let snap = self.scoped_snapshots(ctx).load_snapshot()?;
                Ok(ToolOutput::ok(format!(
                    "=== USER.md ===\n{}\n\n=== MEMORY.md ===\n{}\n\n=== PROJECT_MEMORY.md ===\n{}",
                    snap.user_md, snap.memory_md, snap.project_memory_md
                )))
            }
            "history" => {
                let doc_filter = args.get("doc").and_then(Value::as_str);
                let items = self.scoped_snapshots(ctx).list_doc_history(doc_filter)?;
                if items.is_empty() {
                    return Ok(ToolOutput::ok("No doc rollback snapshots recorded yet."));
                }
                let text = items
                    .iter()
                    .map(|s| {
                        format!(
                            "- `{}` · {} ({}) · {}B · {}",
                            s.id,
                            s.doc_name,
                            s.reason,
                            s.bytes,
                            s.created_at.format("%Y-%m-%d %H:%M:%S UTC")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(ToolOutput {
                    content: text,
                    is_error: false,
                    diff: None,
                    metadata: Some(json!({ "snapshots": items })),
                })
            }
            "rollback_doc" => {
                let sid = args
                    .get("snapshot_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("missing `snapshot_id`".into()))?;
                let (doc_name, diff) = self.scoped_snapshots(ctx).rollback_doc(sid)?;
                Ok(ToolOutput::ok_with_diff(
                    format!("Rolled back {doc_name} to snapshot `{sid}`."),
                    diff,
                ))
            }
            other => Ok(ToolOutput::err(format!("Unknown memory action: {other}"))),
        }
    }
}

/// Cross-session SQLite FTS5 BM25 recall tool (Hermes + Oh My Pi synthesis, zero LLM tokens).
pub struct SessionSearchTool {
    store: SqliteStore,
}

impl SessionSearchTool {
    pub fn new(store: SqliteStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for SessionSearchTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Memory
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_SESSION_SEARCH.into(),
            description: "Search across past Zenko conversation transcripts, tool results, brainstorms, plans, and compaction checkpoints using SQLite FTS5 BM25 (zero LLM tokens), or expand ±N surrounding turns around a matched `around_event_id`.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Keywords or symbols to recall from past sessions" },
                    "around_event_id": { "type": "string", "description": "Optional event_id from a prior hit to expand ±window surrounding turns" },
                    "window": { "type": "integer", "description": "Turns before/after `around_event_id` (default 3, max 8)" },
                    "since": { "type": "string", "description": "Optional time window: e.g. '24h', '7d', '30d', or '2026-11-01'" },
                    "channel": { "type": "string", "description": "Optional channel filter: webui | telegram | cli | cron" },
                    "workspace": { "type": "string", "enum": ["current", "all"], "description": "Scope to current git repo/workspace (default) or search all workspaces" },
                    "session_id": { "type": "string", "description": "Optional specific session ID to search inside" },
                    "role": { "type": "string", "enum": ["user", "assistant", "tool", "system"], "description": "Optional role filter" },
                    "limit": { "type": "integer", "description": "Max hits to return (default 8, max 25)" }
                }
            }),
            risk: RiskLevel::Safe,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        // Hermes-style lineage context expansion around a specific matched event_id
        if let Some(anchor_id) = args
            .get("around_event_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let window = (args
                .get("window")
                .and_then(Value::as_u64)
                .unwrap_or(SESSION_CONTEXT_DEFAULT_WINDOW as u64) as usize)
                .clamp(1, SESSION_CONTEXT_MAX_WINDOW);
            let Some(win) = self.store.get_session_event_context(anchor_id, window)? else {
                return Ok(ToolOutput::ok(format!(
                    "No session event found for `around_event_id={anchor_id}`."
                )));
            };
            let lines = win
                .events
                .iter()
                .map(|t| {
                    let marker = match t.is_anchor {
                        true => ">>",
                        false => "  ",
                    };
                    format!(
                        "{marker} [{}][{}] ({}) {}",
                        t.role,
                        t.kind,
                        t.created_at.format("%Y-%m-%d %H:%M"),
                        t.text.replace('\n', " ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            return Ok(ToolOutput {
                content: format!(
                    "Context window for session `{}` ({}) around `{}`:\n{}",
                    win.session_id, win.session_title, win.anchor_event_id, lines
                ),
                is_error: false,
                diff: None,
                metadata: Some(json!({ "context": win })),
            });
        }

        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ZenkoError::Tool("provide either `query` or `around_event_id`".into())
            })?;
        let since = args
            .get("since")
            .and_then(Value::as_str)
            .and_then(parse_since_window);
        let channel = args
            .get("channel")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let explicit_sid = args
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let role = args
            .get("role")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(SESSION_SEARCH_DEFAULT_LIMIT as u64)
            .clamp(1, SESSION_SEARCH_MAX_LIMIT as u64) as usize;
        let workspace_dir = match args
            .get("workspace")
            .and_then(Value::as_str)
            .unwrap_or("current")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "all" => None,
            _ => Some(resolve_git_root(&ctx.workspace_dir).display().to_string()),
        };

        let filter = SessionSearchFilter {
            query: query.to_string(),
            since,
            channel,
            workspace_dir,
            session_id: explicit_sid.clone(),
            exclude_session_id: match explicit_sid.is_some() {
                true => None,
                false => Some(ctx.session_id.clone()),
            },
            role,
            limit: Some(limit),
        };
        let hits = self.store.search_sessions_fts(&filter)?;
        if hits.is_empty() {
            return Ok(ToolOutput::ok(format!(
                "No past session events matched `{query}`."
            )));
        }
        let body = hits
            .iter()
            .map(|h| {
                format!(
                    "- event=`{}` session=`{}` ({}) [{}][{}] {} · {}",
                    h.event_id,
                    h.session_id,
                    h.session_title,
                    h.channel,
                    h.role,
                    h.created_at.format("%Y-%m-%d %H:%M"),
                    h.snippet.replace('\n', " ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(ToolOutput {
            content: format!("Matched {} past session event(s):\n{body}", hits.len()),
            is_error: false,
            diff: None,
            metadata: Some(json!({ "hits": hits })),
        })
    }
}

pub struct CronTool {
    store: SqliteStore,
    cron_notify: Arc<Notify>,
}

impl CronTool {
    pub fn new(store: SqliteStore, cron_notify: Arc<Notify>) -> Self {
        Self { store, cron_notify }
    }
}

#[async_trait]
impl Tool for CronTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Automation
    }

    fn is_available(&self, ctx: &zenko_core::ToolAvailabilityContext) -> bool {
        !ctx.is_subagent
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        match args.get("action").and_then(Value::as_str) {
            Some("list") => ToolConcurrency::Shared,
            _ => ToolConcurrency::Global,
        }
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "cron_tool".into(),
            description: "Full CRUD for 24/7 recurring cron jobs, heartbeats, and zero-token No-Agent script jobs (scripts run from `~/zenko_hub/scripts/`).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "create", "update", "toggle", "delete"] },
                    "id": { "type": "string", "description": "Cron job ID (for update, toggle, delete)" },
                    "name": { "type": "string" },
                    "cron_expr": { "type": "string", "description": "Standard 5-field cron expression e.g. '0 8 * * *'" },
                    "kind": { "type": "string", "enum": ["cron", "heartbeat", "script"], "description": "`script` runs a file in ~/zenko_hub/scripts/ directly with zero LLM tokens" },
                    "prompt": { "type": "string", "description": "Task prompt (for cron/heartbeat) or description (for script)" },
                    "script_path": { "type": "string", "description": "Script filename/subpath inside ~/zenko_hub/scripts/ (required when kind=script)" },
                    "enabled": { "type": "boolean", "description": "Enable/pause toggle for update or toggle" },
                    "target_channel": { "type": "string", "description": "webui | telegram | cli" }
                },
                "required": ["action"]
            }),
            risk: RiskLevel::Moderate,
        }
    }

    async fn execute(
        &self,
        _ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `action`".into()))?;

        match action {
            "list" => {
                let jobs = self.store.list_cron_jobs()?;
                if jobs.is_empty() {
                    return Ok(ToolOutput::ok("No scheduled jobs."));
                }
                let lines = jobs
                    .into_iter()
                    .map(|j| {
                        let script_tag = j
                            .script_path
                            .as_deref()
                            .map(|s| format!(" script=`{s}`"))
                            .unwrap_or_default();
                        format!(
                            "- [{}] {} ({}) expr=`{}`{} channel={} enabled={}",
                            j.id,
                            j.name,
                            j.kind.as_str(),
                            j.cron_expr,
                            script_tag,
                            j.target_channel,
                            j.enabled
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(ToolOutput::ok(lines))
            }
            "create" => {
                let name = args.get("name").and_then(Value::as_str).unwrap_or("Scheduled Task");
                let cron_expr = args
                    .get("cron_expr")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("missing `cron_expr`".into()))?;
                let kind = CronKind::from_str_loose(
                    args.get("kind").and_then(Value::as_str).unwrap_or("cron"),
                );
                let script_path = args
                    .get("script_path")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                let prompt = args
                    .get("prompt")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if kind == CronKind::Script && script_path.is_none() {
                    return Ok(ToolOutput::err(
                        "kind=`script` requires `script_path` pointing to a script inside ~/zenko_hub/scripts/",
                    ));
                }
                if kind != CronKind::Script && prompt.trim().is_empty() {
                    return Ok(ToolOutput::err("missing `prompt`"));
                }
                let target_channel = args
                    .get("target_channel")
                    .and_then(Value::as_str)
                    .unwrap_or("webui");

                let parsed = Cron::new(cron_expr)
                    .parse()
                    .map_err(|e| ZenkoError::Tool(format!("invalid cron expression: {e}")))?;
                let next = parsed.find_next_occurrence(&Utc::now(), false).ok();

                let job = CronJob {
                    id: new_id(),
                    name: name.to_string(),
                    cron_expr: cron_expr.to_string(),
                    kind,
                    prompt,
                    script_path,
                    target_channel: target_channel.to_string(),
                    target_chat_id: None,
                    target_thread_id: None,
                    enabled: args.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                    next_run_at: next,
                    last_run_at: None,
                    last_status: None,
                };
                self.store.upsert_cron_job(&job)?;
                self.cron_notify.notify_one();
                Ok(ToolOutput::ok(format!(
                    "Scheduled {} job '{}' (id={}) next run: {:?}",
                    job.kind.as_str(),
                    job.name,
                    job.id,
                    job.next_run_at
                )))
            }
            "update" | "toggle" => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("missing `id`".into()))?;
                let Some(mut job) = self.store.get_cron_job(id)? else {
                    return Ok(ToolOutput::err(format!("Cron job `{id}` not found.")));
                };
                if let Some(n) = args.get("name").and_then(Value::as_str)
                    && !n.trim().is_empty() {
                        job.name = n.trim().to_string();
                    }
                if let Some(expr) = args.get("cron_expr").and_then(Value::as_str) {
                    let parsed = Cron::new(expr)
                        .parse()
                        .map_err(|e| ZenkoError::Tool(format!("invalid cron expression: {e}")))?;
                    job.cron_expr = expr.to_string();
                    job.next_run_at = parsed.find_next_occurrence(&Utc::now(), false).ok();
                }
                if let Some(k) = args.get("kind").and_then(Value::as_str) {
                    job.kind = CronKind::from_str_loose(k);
                }
                if let Some(p) = args.get("prompt").and_then(Value::as_str) {
                    job.prompt = p.to_string();
                }
                if let Some(sp) = args.get("script_path").and_then(Value::as_str) {
                    let trimmed = sp.trim();
                    job.script_path = match trimmed.is_empty() {
                        true => None,
                        false => Some(trimmed.to_string()),
                    };
                }
                if let Some(en) = args.get("enabled").and_then(Value::as_bool) {
                    job.enabled = en;
                } else if action == "toggle" {
                    job.enabled = !job.enabled;
                }
                self.store.upsert_cron_job(&job)?;
                self.cron_notify.notify_one();
                Ok(ToolOutput::ok(format!(
                    "Updated cron job `{}` ({}) enabled={}.",
                    job.id, job.name, job.enabled
                )))
            }
            "delete" => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("missing `id`".into()))?;
                let removed = self.store.delete_cron_job(id)?;
                if removed {
                    self.cron_notify.notify_one();
                }
                Ok(ToolOutput::ok(match removed {
                    true => format!("Deleted job {id}."),
                    false => format!("Job {id} not found."),
                }))
            }
            other => Ok(ToolOutput::err(format!("Unknown cron action: {other}"))),
        }
    }
}
