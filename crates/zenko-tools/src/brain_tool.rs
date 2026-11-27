//! Deferred `project_brain` Tool — Correct-by-Construction Repo-Native Project Brain
//! (interoperable with `mindmuxai/brain.md` open standard; zero regex, zero hardcoding, pure `match`).

use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    BRAIN_ACTIONS, BRAIN_DEFAULT_SEARCH_LIMIT, BRAIN_MAX_SEARCH_LIMIT, BRAIN_PAGE_CATEGORIES,
    BRAIN_PAGE_STATUSES, BRAIN_ROOT_SLOT_SLUGS, BRAIN_SCOPES, BRAIN_TIMELINE_KINDS, BrainAction,
    BrainPageCategory, BrainPageStatus, BrainRootSlot, BrainScope, BrainTimelineKind, Result,
    RiskLevel, SecretVault, TOOL_PROJECT_BRAIN, Tool, ToolConcurrency, ToolContext, ToolExposure,
    ToolOutput, ToolSet, ToolSpec, ZenkoError, is_brain_tool_args_mutating,
};
use zenko_memory::brain::{
    append_brain_timeline_scoped, archive_brain_page_scoped, compute_brain_impact_graph_scoped,
    create_brain_page_scoped, ingest_raw_source, init_brain_scoped, lint_brain,
    list_brain_pages_scoped, read_brain_page_scoped, read_brain_root_slot_scoped,
    search_brain_pages_scoped, update_brain_page_truth_scoped, update_brain_root_slot_scoped,
};

pub struct ProjectBrainTool {
    vault: Option<Arc<SecretVault>>,
}

impl ProjectBrainTool {
    pub fn new() -> Self {
        Self { vault: None }
    }

    pub fn new_with_vault(vault: Arc<SecretVault>) -> Self {
        Self { vault: Some(vault) }
    }

    fn known_secrets(&self) -> Vec<String> {
        self.vault
            .as_ref()
            .map(|v| v.all_values())
            .unwrap_or_default()
    }
}

impl Default for ProjectBrainTool {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_string_array(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn ok_meta(content: String, meta: Value) -> ToolOutput {
    ToolOutput {
        content,
        is_error: false,
        diff: None,
        metadata: Some(meta),
    }
}

#[async_trait]
impl Tool for ProjectBrainTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::Memory
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        match is_brain_tool_args_mutating(args) {
            false => ToolConcurrency::Shared,
            true => ToolConcurrency::Global,
        }
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_PROJECT_BRAIN.into(),
            description: "Manage Git-tracked Repo Project Brain (`scope=\"workspace\"`, `BRAIN.md` + `brain/`) or Personal Compounding Hub Wiki (`scope=\"hub\"`, `hub:wiki/`): Karpathy LLM Wiki layers (`raw/` immutable source drop-box, `pages/` pairing rewritable `compiled_truth` with append-only `Timeline`, auto-maintained `index.md` & `log.md`), root slots, search, BFS `impact`, `ingest_raw`, and deterministic `lint`.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "enum": BRAIN_SCOPES,
                        "description": "Target wiki scope: `workspace` (default, repo-native `brain/`) or `hub` (personal compounding `hub:wiki/`)."
                    },
                    "action": {
                        "type": "string",
                        "enum": BRAIN_ACTIONS,
                        "description": "Read-only (`list_pages`, `read_page`, `read_root`, `search`, `impact`, `lint`) or atomic mutating (`init`, `create_page`, `update_truth`, `append_timeline`, `archive_page`, `update_root`, `ingest_raw`)."
                    },
                    "id": { "type": "string", "description": "Kebab-case page ID (e.g. `auth-session-strategy`)." },
                    "title": { "type": "string", "description": "One-line page title (for `create_page`)." },
                    "category": { "type": "string", "enum": BRAIN_PAGE_CATEGORIES },
                    "status": { "type": "string", "enum": BRAIN_PAGE_STATUSES },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "compiled_truth": { "type": "string", "description": "Current synthesized truth body (rewritable as a whole on `create_page` / `update_truth`)." },
                    "kind": { "type": "string", "enum": BRAIN_TIMELINE_KINDS, "description": "Timeline entry kind (`decision`, `evidence`, `update`, `reversal`, `archived`)." },
                    "summary": { "type": "string", "description": "One-line timeline summary explaining what changed or why a decision was reversed." },
                    "source": { "type": "string", "description": "Origin reference (file path, audit, turn, PR, etc.)." },
                    "affects": { "type": "array", "items": { "type": "string" }, "description": "Related page IDs impacted by this entry (builds the BFS impact graph)." },
                    "is_reversal": { "type": "boolean", "description": "For `archive_page`: mark timeline entry as `reversal` instead of `archived` (default: false)." },
                    "slot": { "type": "string", "enum": BRAIN_ROOT_SLOT_SLUGS, "description": "Fixed blueprint root slot (for `read_root` / `update_root`)." },
                    "content": { "type": "string", "description": "Markdown body for `update_root`." },
                    "query": { "type": "string", "description": "Search query (for `search`)." },
                    "include_archived": { "type": "boolean", "description": "Include archived/reversed pages in `search` (default: true)." },
                    "limit": { "type": "integer", "description": "Max search results (default: 8, cap: 24)." }
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
        let raw_action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `action`".into()))?;
        let Some(action) = BrainAction::parse(raw_action) else {
            return Err(ZenkoError::Tool(format!(
                "invalid project_brain action `{raw_action}` (expected one of: {})",
                BRAIN_ACTIONS.join(", ")
            )));
        };

        let scope = match args.get("scope").and_then(Value::as_str) {
            Some(raw_scope) => BrainScope::parse(raw_scope).ok_or_else(|| {
                ZenkoError::Tool(format!(
                    "invalid project_brain scope `{raw_scope}` (expected one of: {})",
                    BRAIN_SCOPES.join(", ")
                ))
            })?,
            None => BrainScope::Workspace,
        };
        let scope_label = match scope {
            BrainScope::Workspace => "Project Brain",
            BrainScope::Hub => "Personal Hub Wiki",
        };
        let ws = ctx.workspace_dir.clone();
        let hub = ctx.hub_dir.clone();
        let secrets = self.known_secrets();

        match action {
            BrainAction::Init => {
                let created =
                    tokio::task::spawn_blocking(move || init_brain_scoped(&ws, &hub, scope))
                        .await
                        .map_err(|e| ZenkoError::Tool(format!("project_brain init join error: {e}")))??;
                let msg = match created.is_empty() {
                    true => format!("{scope_label} already initialized."),
                    false => format!(
                        "Initialized {scope_label} ({} file(s) created): {}",
                        created.len(),
                        created.join(", ")
                    ),
                };
                Ok(ok_meta(msg, json!({ "scope": scope.as_str(), "created": created })))
            }
            BrainAction::ListPages => {
                let status_f = args
                    .get("status")
                    .and_then(Value::as_str)
                    .and_then(BrainPageStatus::parse);
                let cat_f = args
                    .get("category")
                    .and_then(Value::as_str)
                    .and_then(BrainPageCategory::parse);
                let pages = tokio::task::spawn_blocking(move || {
                    list_brain_pages_scoped(&ws, &hub, scope, status_f, cat_f)
                })
                .await
                .map_err(|e| ZenkoError::Tool(format!("project_brain list join error: {e}")))?;

                let content = match pages.is_empty() {
                    true => format!("No {scope_label} pages found."),
                    false => {
                        let mut lines = vec![format!("{scope_label} Pages ({}):", pages.len())];
                        for p in &pages {
                            let rev = match p.has_reversals {
                                true => " [reversal]",
                                false => "",
                            };
                            lines.push(format!(
                                "- `{}` [{}][{}]{rev} {} — {}",
                                p.id,
                                p.category.as_str(),
                                p.status.as_str(),
                                p.title,
                                p.truth_preview
                            ));
                        }
                        lines.join("\n")
                    }
                };
                Ok(ok_meta(
                    content,
                    serde_json::to_value(&pages).unwrap_or(Value::Null),
                ))
            }
            BrainAction::ReadPage => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`read_page` requires `id`".into()))?
                    .to_string();
                let (page, impact) = tokio::task::spawn_blocking(move || -> Result<_> {
                    let page = read_brain_page_scoped(&ws, &hub, scope, &id)?;
                    let impact = compute_brain_impact_graph_scoped(&ws, &hub, scope, &page.id)?;
                    Ok((page, impact))
                })
                .await
                .map_err(|e| ZenkoError::Tool(format!("project_brain read join error: {e}")))??;

                let mut out = vec![format!(
                    "# {} (`{}` · {} · {} · scope={})\nupdated: {} · tags: [{}]",
                    page.title,
                    page.id,
                    page.category.as_str(),
                    page.status.as_str(),
                    scope.as_str(),
                    page.updated,
                    page.tags.join(", ")
                )];
                out.push(format!("\n## Compiled Truth\n{}", page.compiled_truth));
                if !page.timeline.is_empty() {
                    out.push("\n## Timeline".to_string());
                    for ev in &page.timeline {
                        let aff = match ev.affects.is_empty() {
                            true => String::new(),
                            false => format!(" · affects=[{}]", ev.affects.join(", ")),
                        };
                        let src = match ev.source.is_empty() {
                            true => String::new(),
                            false => format!(" (src: {})", ev.source),
                        };
                        out.push(format!(
                            "- [{}] ({}) {}{src}{aff}",
                            ev.time,
                            ev.kind.as_str(),
                            ev.summary
                        ));
                    }
                }
                if !impact.referenced_by.is_empty() || !impact.direct_affects.is_empty() {
                    out.push(format!(
                        "\n## Impact Links\n- affects: [{}]\n- referenced_by: [{}]",
                        impact.direct_affects.join(", "),
                        impact.referenced_by.join(", ")
                    ));
                }
                Ok(ok_meta(
                    out.join("\n"),
                    json!({ "scope": scope.as_str(), "page": page, "impact": impact }),
                ))
            }
            BrainAction::CreatePage => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`create_page` requires `id`".into()))?
                    .to_string();
                let title = args
                    .get("title")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`create_page` requires `title`".into()))?
                    .to_string();
                let truth = args
                    .get("compiled_truth")
                    .or_else(|| args.get("content"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ZenkoError::Tool("`create_page` requires `compiled_truth`".into())
                    })?
                    .to_string();
                let category = args
                    .get("category")
                    .and_then(Value::as_str)
                    .and_then(BrainPageCategory::parse)
                    .unwrap_or(BrainPageCategory::Decision);
                let status = args
                    .get("status")
                    .and_then(Value::as_str)
                    .and_then(BrainPageStatus::parse)
                    .unwrap_or(BrainPageStatus::Active);
                let tags = parse_string_array(&args, "tags");
                let summary = args
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let source = args
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("zenko")
                    .to_string();
                let affects = parse_string_array(&args, "affects");

                let page = tokio::task::spawn_blocking(move || {
                    create_brain_page_scoped(
                        &ws,
                        &hub,
                        scope,
                        &id,
                        &title,
                        category,
                        status,
                        &tags,
                        &truth,
                        &summary,
                        &source,
                        &affects,
                        &secrets,
                    )
                })
                .await
                .map_err(|e| ZenkoError::Tool(format!("project_brain create join error: {e}")))??;

                Ok(ok_meta(
                    format!(
                        "Created {scope_label} page `{}` [{} · {}] ({} timeline entry; index.md & log.md synced).",
                        page.id,
                        page.category.as_str(),
                        page.status.as_str(),
                        page.timeline.len()
                    ),
                    serde_json::to_value(&page).unwrap_or(Value::Null),
                ))
            }
            BrainAction::UpdateTruth => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`update_truth` requires `id`".into()))?
                    .to_string();
                let truth = args
                    .get("compiled_truth")
                    .or_else(|| args.get("content"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ZenkoError::Tool("`update_truth` requires `compiled_truth`".into())
                    })?
                    .to_string();
                let summary = args
                    .get("summary")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ZenkoError::Tool("`update_truth` requires timeline `summary`".into())
                    })?
                    .to_string();
                let kind = args
                    .get("kind")
                    .and_then(Value::as_str)
                    .and_then(BrainTimelineKind::parse)
                    .unwrap_or(BrainTimelineKind::Update);
                let source = args
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("zenko")
                    .to_string();
                let affects = parse_string_array(&args, "affects");
                let tags_override = args.get("tags").map(|_| parse_string_array(&args, "tags"));
                let status_override = args
                    .get("status")
                    .and_then(Value::as_str)
                    .and_then(BrainPageStatus::parse);

                let (page, impact) = tokio::task::spawn_blocking(move || -> Result<_> {
                    let page = update_brain_page_truth_scoped(
                        &ws,
                        &hub,
                        scope,
                        &id,
                        &truth,
                        kind,
                        &summary,
                        &source,
                        &affects,
                        tags_override.as_deref(),
                        status_override,
                        &secrets,
                    )?;
                    let impact = compute_brain_impact_graph_scoped(&ws, &hub, scope, &page.id)?;
                    Ok((page, impact))
                })
                .await
                .map_err(|e| ZenkoError::Tool(format!("project_brain update join error: {e}")))??;

                let impact_note = match impact.referenced_by.is_empty() && impact.direct_affects.is_empty() {
                    true => String::new(),
                    false => format!(
                        " (affects=[{}], referenced_by=[{}])",
                        impact.direct_affects.join(", "),
                        impact.referenced_by.join(", ")
                    ),
                };
                Ok(ok_meta(
                    format!(
                        "Updated `{}` ({}) compiled_truth + appended `{}` timeline entry (#{}){impact_note}.",
                        page.id,
                        scope.as_str(),
                        kind.as_str(),
                        page.timeline.len()
                    ),
                    json!({ "scope": scope.as_str(), "page": page, "impact": impact }),
                ))
            }
            BrainAction::AppendTimeline => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`append_timeline` requires `id`".into()))?
                    .to_string();
                let summary = args
                    .get("summary")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ZenkoError::Tool("`append_timeline` requires `summary`".into())
                    })?
                    .to_string();
                let kind = args
                    .get("kind")
                    .and_then(Value::as_str)
                    .and_then(BrainTimelineKind::parse)
                    .unwrap_or(BrainTimelineKind::Evidence);
                let source = args
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("zenko")
                    .to_string();
                let affects = parse_string_array(&args, "affects");

                let page = tokio::task::spawn_blocking(move || {
                    append_brain_timeline_scoped(
                        &ws, &hub, scope, &id, kind, &summary, &source, &affects, &secrets,
                    )
                })
                .await
                .map_err(|e| ZenkoError::Tool(format!("project_brain timeline join error: {e}")))??;

                Ok(ok_meta(
                    format!(
                        "Appended `{}` timeline entry (#{}) to `{}` ({}).",
                        kind.as_str(),
                        page.timeline.len(),
                        page.id,
                        scope.as_str()
                    ),
                    serde_json::to_value(&page).unwrap_or(Value::Null),
                ))
            }
            BrainAction::ArchivePage => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`archive_page` requires `id`".into()))?
                    .to_string();
                let summary = args
                    .get("summary")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ZenkoError::Tool("`archive_page` requires `summary`".into())
                    })?
                    .to_string();
                let source = args
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("zenko")
                    .to_string();
                let affects = parse_string_array(&args, "affects");
                let is_reversal = args
                    .get("is_reversal")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);

                let page = tokio::task::spawn_blocking(move || {
                    archive_brain_page_scoped(
                        &ws,
                        &hub,
                        scope,
                        &id,
                        &summary,
                        &source,
                        &affects,
                        is_reversal,
                        &secrets,
                    )
                })
                .await
                .map_err(|e| ZenkoError::Tool(format!("project_brain archive join error: {e}")))??;

                Ok(ok_meta(
                    format!(
                        "Archived `{}` ({}, {} timeline entries).",
                        page.id,
                        scope.as_str(),
                        page.timeline.len()
                    ),
                    serde_json::to_value(&page).unwrap_or(Value::Null),
                ))
            }
            BrainAction::ReadRoot => {
                let raw_slot = args
                    .get("slot")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`read_root` requires `slot`".into()))?;
                let Some(slot) = BrainRootSlot::parse(raw_slot) else {
                    return Err(ZenkoError::Tool(format!(
                        "invalid root slot `{raw_slot}` (expected one of: {})",
                        BRAIN_ROOT_SLOT_SLUGS.join(", ")
                    )));
                };
                let body = tokio::task::spawn_blocking(move || {
                    read_brain_root_slot_scoped(&ws, &hub, scope, slot)
                })
                .await
                .map_err(|e| ZenkoError::Tool(format!("project_brain read_root error: {e}")))??;
                Ok(ok_meta(body, json!({ "scope": scope.as_str(), "slot": slot.as_str() })))
            }
            BrainAction::UpdateRoot => {
                let raw_slot = args
                    .get("slot")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`update_root` requires `slot`".into()))?;
                let Some(slot) = BrainRootSlot::parse(raw_slot) else {
                    return Err(ZenkoError::Tool(format!(
                        "invalid root slot `{raw_slot}` (expected one of: {})",
                        BRAIN_ROOT_SLOT_SLUGS.join(", ")
                    )));
                };
                let content = args
                    .get("content")
                    .or_else(|| args.get("compiled_truth"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`update_root` requires `content`".into()))?
                    .to_string();
                let written = tokio::task::spawn_blocking(move || {
                    update_brain_root_slot_scoped(&ws, &hub, scope, slot, &content, &secrets)
                })
                .await
                .map_err(|e| ZenkoError::Tool(format!("project_brain update_root error: {e}")))??;
                Ok(ok_meta(
                    format!(
                        "Updated {scope_label} root slot `{}` ({} bytes).",
                        slot.filename(),
                        written.len()
                    ),
                    json!({ "scope": scope.as_str(), "slot": slot.as_str(), "bytes": written.len() }),
                ))
            }
            BrainAction::Search => {
                let query = args
                    .get("query")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`search` requires `query`".into()))?
                    .to_string();
                let include_archived = args
                    .get("include_archived")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(BRAIN_DEFAULT_SEARCH_LIMIT as u64)
                    .clamp(1, BRAIN_MAX_SEARCH_LIMIT as u64) as usize;
                let hits = tokio::task::spawn_blocking(move || {
                    search_brain_pages_scoped(&ws, &hub, scope, &query, include_archived, limit)
                })
                .await
                .map_err(|e| ZenkoError::Tool(format!("project_brain search error: {e}")))?;

                let content = match hits.is_empty() {
                    true => format!("No matching {scope_label} pages found."),
                    false => {
                        let mut lines = vec![format!("{scope_label} Search ({} hit(s)):", hits.len())];
                        for h in &hits {
                            let rev = match h.summary.has_reversals {
                                true => " [reversal]",
                                false => "",
                            };
                            let aff = match h.affects.is_empty() {
                                true => String::new(),
                                false => format!(" · affects=[{}]", h.affects.join(", ")),
                            };
                            lines.push(format!(
                                "- `{}` [{}][{}]{rev} ({:.1}) {} — {}{aff}",
                                h.summary.id,
                                h.summary.category.as_str(),
                                h.summary.status.as_str(),
                                h.score,
                                h.summary.title,
                                h.summary.truth_preview
                            ));
                        }
                        lines.join("\n")
                    }
                };
                Ok(ok_meta(
                    content,
                    serde_json::to_value(&hits).unwrap_or(Value::Null),
                ))
            }
            BrainAction::Impact => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`impact` requires `id`".into()))?
                    .to_string();
                let graph = tokio::task::spawn_blocking(move || {
                    compute_brain_impact_graph_scoped(&ws, &hub, scope, &id)
                })
                .await
                .map_err(|e| ZenkoError::Tool(format!("project_brain impact error: {e}")))??;

                let content = format!(
                    "{scope_label} Impact Graph for `{}`:\n- direct_affects: [{}]\n- transitive_affects: [{}]\n- referenced_by: [{}]",
                    graph.page_id,
                    graph.direct_affects.join(", "),
                    graph.transitive_affects.join(", "),
                    graph.referenced_by.join(", "),
                );
                Ok(ok_meta(
                    content,
                    serde_json::to_value(&graph).unwrap_or(Value::Null),
                ))
            }
            BrainAction::IngestRaw => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`ingest_raw` requires `id`".into()))?
                    .to_string();
                let title = args
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let source_url = args
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let content_body = args
                    .get("content")
                    .or_else(|| args.get("compiled_truth"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`ingest_raw` requires `content`".into()))?
                    .to_string();
                let rel_path = tokio::task::spawn_blocking(move || {
                    ingest_raw_source(
                        &ws,
                        &hub,
                        scope,
                        &id,
                        &title,
                        &source_url,
                        &content_body,
                        &secrets,
                    )
                })
                .await
                .map_err(|e| ZenkoError::Tool(format!("project_brain ingest_raw error: {e}")))??;
                Ok(ok_meta(
                    format!(
                        "Saved immutable raw source `{rel_path}` in {scope_label} (`index.md` & `log.md` updated). Compile into wiki pages with `create_page`/`update_truth` citing `source=\"{rel_path}\"`."
                    ),
                    json!({ "scope": scope.as_str(), "raw_path": rel_path }),
                ))
            }
            BrainAction::Lint => {
                let report = tokio::task::spawn_blocking(move || lint_brain(&ws, &hub, scope))
                    .await
                    .map_err(|e| ZenkoError::Tool(format!("project_brain lint error: {e}")))?;
                let mut lines = vec![format!(
                    "{scope_label} Lint Report (pages={}/{} active · raw={} · issues={}):",
                    report.active_pages,
                    report.total_pages,
                    report.raw_sources_count,
                    report.issues.len()
                )];
                if report.issues.is_empty() {
                    lines.push("- Healthy: zero orphans, zero broken links, zero stale pages, and zero uncompiled raw sources.".into());
                } else {
                    for issue in &report.issues {
                        lines.push(format!("- [{}] {}: {}", issue.kind.as_str(), issue.target, issue.detail));
                    }
                }
                Ok(ok_meta(
                    lines.join("\n"),
                    serde_json::to_value(&report).unwrap_or(Value::Null),
                ))
            }
        }
    }
}
