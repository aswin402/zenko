//! Phase 3 Context Engineering Tools:
//! - `skill_view`: 3-level progressive disclosure (`slug` list -> full `SKILL.md` -> nested subfiles)
//! - `tool_search`: Zero-regex deterministic BM25 deferred tool schema loader
//! - `context_inspect`: Live per-tier token budget, cache hit %, checkpoint & artifact report

use crate::coerce::format_compact_schema_hint;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::{Arc, RwLock};
use tokio_util::sync::CancellationToken;
use zenko_core::{
    ContextBridge, ResourceLockKey, Result, RiskLevel, SKILL_HISTORY_LIST_DEFAULT_LIMIT,
    SecretVault, SkillAuthor, SkillManageAction, SkillStatus, SkillsConfig, TOOL_CONTEXT_INSPECT,
    TOOL_INVOKE_TOOL, TOOL_SEARCH_DEFAULT_LIMIT, TOOL_SEARCH_MAX_LIMIT, TOOL_SKILL_MANAGE,
    TOOL_SKILL_VIEW, TOOL_TOOL_SEARCH, Tool, ToolAvailabilityContext, ToolConcurrency, ToolContext,
    ToolExposure, ToolOutput, ToolSet, ToolSpec, ZenkoError,
};
use zenko_memory::SnapshotManager;

pub struct SkillViewTool {
    snapshots: Arc<SnapshotManager>,
}

impl SkillViewTool {
    pub fn new(snapshots: Arc<SnapshotManager>) -> Self {
        Self { snapshots }
    }
}

#[async_trait]
impl Tool for SkillViewTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::Memory
    }

    fn is_available(&self, ctx: &ToolAvailabilityContext) -> bool {
        ctx.has_skills
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Eager
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_SKILL_VIEW.into(),
            description: "Progressive 3-level skill loader: omit `slug` to list skills, pass `slug` to read full SKILL.md instructions on demand, or pass `slug` + `file_path` to load a reference subfile inside that skill's folder.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "slug": { "type": "string", "description": "Skill slug from <available_skills> (omit to list all available skills)" },
                    "file_path": { "type": "string", "description": "Optional Level-2 relative reference file path inside the skill directory (e.g. 'references/guide.md')" }
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
        let slug = args
            .get("slug")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let file_path = args
            .get("file_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);

        let mgr = match ctx.workspace_dir != self.snapshots.workspace_dir() {
            true => Arc::new(
                SnapshotManager::new(&ctx.data_dir, &ctx.workspace_dir)
                    .with_hub_dir(&ctx.hub_dir),
            ),
            false => self.snapshots.clone(),
        };

        tokio::task::spawn_blocking(move || -> Result<ToolOutput> {
            let Some(slug_str) = slug else {
                let skills = mgr.list_skills()?;
                if skills.is_empty() {
                    return Ok(ToolOutput::ok("No skills installed."));
                }
                let lines = skills
                    .iter()
                    .map(|s| {
                        let state = match (s.status, s.enabled) {
                            (SkillStatus::Draft, _) => "draft",
                            (SkillStatus::Active, true) => "enabled",
                            (SkillStatus::Active, false) => "disabled",
                        };
                        let refs = match s.subfiles.is_empty() {
                            true => String::new(),
                            false => format!(" · subfiles=[{}]", s.subfiles.join(", ")),
                        };
                        format!(
                            "- `{}` ({}, {}, author={}, ~{} tok): {}{refs}",
                            s.slug,
                            s.name,
                            state,
                            s.author.as_str(),
                            s.token_estimate,
                            s.description
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                return Ok(ToolOutput::ok(lines));
            };

            let (skill, subfile_content) = mgr.read_skill(&slug_str, file_path.as_deref())?;
            match (file_path, subfile_content) {
                (Some(rel), Some(body)) => Ok(ToolOutput::ok(format!(
                    "# Skill `{}` · `{rel}`\n\n{body}",
                    skill.slug
                ))),
                _ => {
                    let sub_footer = match skill.subfiles.is_empty() {
                        true => String::new(),
                        false => format!(
                            "\n\nAvailable reference subfiles (load via `{TOOL_SKILL_VIEW}` with `file_path`):\n- {}",
                            skill.subfiles.join("\n- ")
                        ),
                    };
                    Ok(ToolOutput::ok(format!(
                        "# Skill: {} (`{}`)\n\n{}{sub_footer}",
                        skill.name, skill.slug, skill.content
                    )))
                }
            }
        })
        .await
        .map_err(|e| ZenkoError::Tool(format!("skill_view worker failed: {e}")))?
    }
}

pub struct ToolSearchTool {
    bridge: Arc<dyn ContextBridge>,
}

impl ToolSearchTool {
    pub fn new(bridge: Arc<dyn ContextBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for ToolSearchTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::Memory
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Eager
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_TOOL_SEARCH.into(),
            description: "Discover and activate deferred tools on demand (respecting active role & permission scope). Search by `query` (BM25), activate an entire `toolset` domain (e.g. `git`, `subagents`, `web`, `memory`, `code_intel`, `execution`, `automation`, `mcp`, or `all`), activate exact `names`, or pass `action=\"list_toolsets\"` to inspect all permitted domains.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["search", "list_toolsets"], "description": "`search` (default) to find/activate deferred tools, or `list_toolsets` to inspect all permitted ToolSets" },
                    "query": { "type": "string", "description": "Keywords describing the capability or tool name needed (e.g. 'git diff', 'lsp definition', 'web search')" },
                    "toolset": { "type": "string", "description": "Declarative ToolSet domain to activate (`core_fs`, `code_intel`, `execution`, `git`, `web`, `memory`, `orchestration`, `subagents`, `automation`, `mcp`, or `all`)" },
                    "names": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Exact deferred tool name(s) to activate in one call"
                    },
                    "limit": { "type": "integer", "description": "Max tools to activate for keyword search (default 5, max 12)" }
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
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("search")
            .trim()
            .to_ascii_lowercase();

        if action == "list_toolsets" {
            let sets = self.bridge.list_permitted_toolsets(&ctx.session_id);
            if sets.is_empty() {
                return Ok(ToolOutput::ok("No permitted toolsets available in this scope."));
            }
            let lines: Vec<String> = sets
                .iter()
                .map(|s| {
                    let eager = match s.eager_tools.is_empty() {
                        true => "none".to_string(),
                        false => s.eager_tools.join(", "),
                    };
                    let deferred = match s.deferred_tools.is_empty() {
                        true => "none".to_string(),
                        false => s.deferred_tools.join(", "),
                    };
                    format!(
                        "- `{}` ({}): {}\n  eager: {eager} | deferred: {deferred}",
                        s.id, s.label, s.description
                    )
                })
                .collect();
            return Ok(ToolOutput {
                content: format!(
                    "Permitted ToolSets ({}):\n{}",
                    sets.len(),
                    lines.join("\n")
                ),
                is_error: false,
                diff: None,
                metadata: Some(json!({ "toolsets": sets })),
            });
        }

        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let raw_toolset = args
            .get("toolset")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let activate_all = raw_toolset.is_some_and(|s| s.eq_ignore_ascii_case("all"));
        let parsed_toolset = match (raw_toolset, activate_all) {
            (Some(_), true) | (None, _) => None,
            (Some(raw), false) => match ToolSet::parse(raw) {
                Some(ts) => Some(ts),
                None => {
                    let valid: Vec<&str> = ToolSet::ALL.iter().map(|t| t.as_str()).collect();
                    return Ok(ToolOutput::err(format!(
                        "Unknown toolset `{raw}`. Valid toolsets: {}, or `all`.",
                        valid.join(", ")
                    )));
                }
            },
        };
        let names: Vec<String> = args
            .get("names")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();

        if query.is_none() && parsed_toolset.is_none() && !activate_all && names.is_empty() {
            return Err(ZenkoError::Tool(
                "provide `query`, `toolset`, `names`, or `action=\"list_toolsets\"`".into(),
            ));
        }

        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(TOOL_SEARCH_DEFAULT_LIMIT as u64)
            .clamp(1, TOOL_SEARCH_MAX_LIMIT as u64) as usize;

        let matched = self.bridge.activate_deferred_by_scope(
            &ctx.session_id,
            parsed_toolset,
            activate_all,
            &names,
            query,
            limit,
        );
        if matched.is_empty() {
            return Ok(ToolOutput::ok(
                "No permitted deferred tools matched that request. Use `tool_search(action=\"list_toolsets\")` to inspect available domains in your current scope.",
            ));
        }

        let activated_names: Vec<String> = matched.iter().map(|t| t.name.clone()).collect();
        let details = matched
            .iter()
            .map(|t| {
                let hint = format_compact_schema_hint(t);
                match hint.is_empty() {
                    true => format!("- `{}` [{}]: {}", t.name, t.risk.as_str(), t.description),
                    false => format!(
                        "- `{}` [{}]: {}\n  {hint}",
                        t.name,
                        t.risk.as_str(),
                        t.description
                    ),
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ToolOutput {
            content: format!(
                "Discovered {} deferred tool(s) (call via `{TOOL_INVOKE_TOOL}(tool, args)` or `batch_tools` to keep the KV prompt cache 100% frozen):\n{details}",
                matched.len()
            ),
            is_error: false,
            diff: None,
            metadata: Some(json!({ "activated_tools": activated_names })),
        })
    }
}

pub struct ContextInspectTool {
    bridge: Arc<dyn ContextBridge>,
}

impl ContextInspectTool {
    pub fn new(bridge: Arc<dyn ContextBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for ContextInspectTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::Memory
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_CONTEXT_INSPECT.into(),
            description: "Inspect live token budget breakdown (Tier 1/2 prompt sizes, KV cache hit %, checkpoints, artifacts) OR pass `query` to run unified multi-source ranked context retrieval across Repo Project Brain, Personal Hub Wiki, OKF Knowledge Catalog, SQLite Memories, and Skills (separating instruction precedence from relevance scoring).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Optional topic/task query: when set, ranks relevant context across Project Brain, Hub Wiki, OKF, Memories & Skills with exact just-in-time load hints."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max ranked context hits when `query` is provided (default: 8, max: 24)."
                    }
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
        if let Some(query) = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|q| !q.is_empty())
        {
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(zenko_core::CONTEXT_RETRIEVAL_DEFAULT_LIMIT as u64)
                .clamp(1, zenko_core::CONTEXT_RETRIEVAL_MAX_LIMIT as u64) as usize;
            let report = self
                .bridge
                .retrieve_ranked_context(&ctx.session_id, query, limit)
                .await?;
            let content = match report.items.is_empty() {
                true => format!(
                    "Context Retrieval for `{}`: 0 matching items across Project Brain, Hub Wiki, OKF, Memories, and Skills.",
                    report.query
                ),
                false => {
                    let mut lines = vec![format!(
                        "Ranked Context Retrieval for `{}` ({} shown / {} candidates · precedence != relevance):",
                        report.query,
                        report.items.len(),
                        report.total_candidates
                    )];
                    for (idx, item) in report.items.iter().enumerate() {
                        lines.push(format!(
                            "{}. [score={:.2} · prec=P{}:{} · {}] `{}` — {} | {} | load: `{}`",
                            idx + 1,
                            item.score,
                            item.precedence.rank(),
                            item.precedence.as_str(),
                            item.source.as_str(),
                            item.id,
                            item.title,
                            item.summary,
                            item.load_hint
                        ));
                    }
                    lines.join("\n")
                }
            };
            return Ok(ToolOutput {
                content,
                is_error: false,
                diff: None,
                metadata: serde_json::to_value(&report).ok(),
            });
        }

        let info = self.bridge.inspect_session_context(&ctx.session_id).await?;
        let rules = match info.repo_rules_loaded.is_empty() {
            true => "none".to_string(),
            false => info.repo_rules_loaded.join(", "),
        };
        let scoped_rules = match info.scoped_rules_loaded.is_empty() {
            true => format!("0 active / {} nested indexed", info.nested_repo_docs_count),
            false => format!(
                "{} active / {} nested indexed [{}]",
                info.scoped_rules_loaded.len(),
                info.nested_repo_docs_count,
                info.scoped_rules_loaded.join(", ")
            ),
        };
        let kv_prefix_status = match info.cache_stability_warnings.is_empty() {
            true => "stable".to_string(),
            false => format!(
                "{} warning(s): {}",
                info.cache_stability_warnings.len(),
                info.cache_stability_warnings[0]
            ),
        };
        let ckpt = info
            .active_checkpoint_id
            .as_deref()
            .unwrap_or("none");
        let summary = format!(
            "Context Budget (`{}` · model=`{}`)\n\
             - Window: {} / {} tok ({:.1}% used · KV cache hit {:.1}%)\n\
             - Tier 1 (frozen prefix): {} tok (hash={} · repo_rules=[{rules}] · kv_prefix={kv_prefix_status})\n\
             - Tier 2 (session stable): {} tok (scoped_dox={scoped_rules} · project_brain={} active · hub_wiki={} active · okf={} concepts)\n\
             - Prefix Engine: drift=`{}` ({}% reused · calibrated {:.2} B/tok · session_vars={})\n\
             - Tool schemas: {} tok (eager={}, deferred_active={}/{})\n\
             - History: {} tok (microcompacted={} tools, ~{} tok saved)\n\
             - Concise Mode: `{}` (auto_clarity={}) · Read-Less: {} ({} outputs compressed, {} B / ~{} tok saved)\n\
             - Code Minimalism: `{}` (repo churn +{} / -{} LOC)\n\
             - Checkpoints: {} (active={ckpt}) · Spilled artifacts: {}",
            info.session_id,
            info.model,
            info.total_tokens,
            info.context_window,
            info.utilization_pct,
            info.cache_hit_pct,
            info.tier1_tokens,
            &info.tier1_hash[..info.tier1_hash.len().min(12)],
            info.tier2_tokens,
            info.project_brain_pages_count,
            info.hub_wiki_pages_count,
            info.okf_concepts_count,
            info.prefix_drift_reason.as_str(),
            info.prefix_hit_ratio_pct,
            info.calibrated_bytes_per_token,
            info.session_vars_count,
            info.tool_schema_tokens,
            info.eager_tools_count,
            info.deferred_active_count,
            info.deferred_available_count,
            info.history_tokens,
            info.microcompacted_tools_count,
            info.microcompacted_tokens_saved,
            info.concise_mode.as_str(),
            info.auto_clarity,
            match info.read_less_enabled {
                true => "on",
                false => "off",
            },
            info.read_less_calls_compressed,
            info.read_less_bytes_saved,
            info.read_less_tokens_saved,
            info.minimalism_mode.as_str(),
            info.economy_lines_added,
            info.economy_lines_deleted,
            info.checkpoints_count,
            info.spilled_artifacts_count,
        );
        Ok(ToolOutput {
            content: summary,
            is_error: false,
            diff: None,
            metadata: Some(serde_json::to_value(&info).unwrap_or(Value::Null)),
        })
    }
}

/// Phase 8 Procedural Skill Authoring tool (`skill_manage`) with atomic `.history/` versioning,
/// Hermes-style draft approval gate, surgical patching, subfile management, and rollback.
pub struct SkillManageTool {
    snapshots: Arc<SnapshotManager>,
    vault: Arc<SecretVault>,
    skills_cfg: Arc<RwLock<SkillsConfig>>,
}

impl SkillManageTool {
    pub fn new(
        snapshots: Arc<SnapshotManager>,
        vault: Arc<SecretVault>,
        skills_cfg: Arc<RwLock<SkillsConfig>>,
    ) -> Self {
        Self {
            snapshots,
            vault,
            skills_cfg,
        }
    }
}

#[async_trait]
impl Tool for SkillManageTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::Memory
    }

    fn is_available(&self, ctx: &zenko_core::ToolAvailabilityContext) -> bool {
        !ctx.is_subagent
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .and_then(SkillManageAction::parse);
        match action {
            Some(a) if !a.is_mutating() => ToolConcurrency::Shared,
            _ => {
                let slug = args
                    .get("slug")
                    .and_then(Value::as_str)
                    .unwrap_or("global")
                    .trim()
                    .to_ascii_lowercase();
                ToolConcurrency::Keyed(ResourceLockKey::Skill(slug))
            }
        }
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_SKILL_MANAGE.into(),
            description: "Author, patch, version, toggle, and roll back reusable procedural skills under `~/.zenko/skills/` (Agent Skills standard). Every edit writes an atomic `.history/` backup and strips secrets/injection tags. When `[skills].require_approval = true`, agent-created skills save as `draft` until approved.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "create", "patch", "write_subfile", "remove_subfile", "toggle", "history", "rollback", "delete"],
                        "description": "Skill operation to perform"
                    },
                    "slug": { "type": "string", "description": "Skill slug (lowercase alphanumeric/hyphens, e.g. 'rust-sqlx-migrations')" },
                    "name": { "type": "string", "description": "Human title for `create`" },
                    "description": { "type": "string", "description": "Concise 1-line trigger description for `create`" },
                    "content": { "type": "string", "description": "Markdown skill instructions body for `create` or subfile contents for `write_subfile`" },
                    "old_text": { "type": "string", "description": "Exact unique substring to replace in `patch`" },
                    "new_text": { "type": "string", "description": "Replacement text in `patch`" },
                    "file_path": { "type": "string", "description": "Relative subfile path inside the skill folder (e.g. 'references/checklist.md') for `write_subfile`/`remove_subfile`" },
                    "enabled": { "type": "boolean", "description": "Target enabled flag for `toggle`" },
                    "triggers": { "type": "array", "items": { "type": "string" }, "description": "Optional trigger keywords for `create`" },
                    "allowed_tools": { "type": "array", "items": { "type": "string" }, "description": "Optional recommended tool names for `create`" },
                    "snapshot_id": { "type": "string", "description": "Version snapshot ID from `history` for `rollback`" },
                    "reason": { "type": "string", "description": "Short reason tag recorded in `.history/` filename" }
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
        let action_raw = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `action` for `skill_manage`".into()))?;
        let action = SkillManageAction::parse(action_raw).ok_or_else(|| {
            ZenkoError::Tool(format!(
                "unknown `skill_manage` action `{action_raw}` (use list, create, patch, write_subfile, remove_subfile, toggle, history, rollback, delete)"
            ))
        })?;
        let cfg = self
            .skills_cfg
            .read()
            .map(|g| g.clone())
            .unwrap_or_default();
        let snapshots = self.snapshots.clone();
        let known_secrets = self.vault.all_values();

        tokio::task::spawn_blocking(move || -> Result<ToolOutput> {
            match action {
                SkillManageAction::List => {
                    let skills = snapshots.list_skills()?;
                    if skills.is_empty() {
                        return Ok(ToolOutput::ok("No skills installed."));
                    }
                    let lines = skills
                        .iter()
                        .map(|s| {
                            format!(
                                "- `{}` ({}) [status={}, enabled={}, author={}, ~{} tok]: {}",
                                s.slug,
                                s.name,
                                s.status.as_str(),
                                s.enabled,
                                s.author.as_str(),
                                s.token_estimate,
                                s.description
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    Ok(ToolOutput {
                        content: lines,
                        is_error: false,
                        diff: None,
                        metadata: Some(json!({ "skills": skills })),
                    })
                }
                SkillManageAction::Create => {
                    let slug = req_str(&args, "slug")?;
                    if snapshots.get_global_skill(slug)?.is_some() {
                        return Err(ZenkoError::PolicyDenied(format!(
                            "Skill `{slug}` already exists; use `skill_manage(action=\"patch\")` for surgical updates or choose a new slug."
                        )));
                    }
                    let body = req_str(&args, "content")?;
                    let name = opt_str(&args, "name");
                    let desc = opt_str(&args, "description").unwrap_or("");
                    let reason = opt_str(&args, "reason").unwrap_or("agent_create");
                    let triggers = str_array(&args, "triggers");
                    let allowed_tools = str_array(&args, "allowed_tools");
                    let (status, enabled) = match cfg.require_approval {
                        true => (SkillStatus::Draft, false),
                        false => (SkillStatus::Active, true),
                    };
                    let (doc, snap) = snapshots.create_skill_structured(
                        slug,
                        name,
                        desc,
                        body,
                        enabled,
                        status,
                        SkillAuthor::Agent,
                        &triggers,
                        &allowed_tools,
                        reason,
                        &known_secrets,
                        cfg.max_history_snapshots,
                    )?;
                    let gate_note = match doc.status {
                        SkillStatus::Draft => format!(
                            " Saved as `draft` (awaiting human approval via `zenko skills approve {}` or WebUI Memory -> Skills).",
                            doc.slug
                        ),
                        SkillStatus::Active => " Active immediately.".into(),
                    };
                    Ok(ToolOutput {
                        content: format!(
                            "Saved procedural skill `{}` ({} · ~{} tok).{gate_note}",
                            doc.slug, doc.name, doc.token_estimate
                        ),
                        is_error: false,
                        diff: None,
                        metadata: Some(json!({
                            "skill": doc,
                            "backup_snapshot": snap,
                        })),
                    })
                }
                SkillManageAction::Patch => {
                    let slug = req_str(&args, "slug")?;
                    let old_text = req_str(&args, "old_text")?;
                    let new_text = req_str(&args, "new_text")?;
                    let reason = opt_str(&args, "reason").unwrap_or("agent_patch");
                    let (doc, diff, snap) = snapshots.patch_skill_guarded(
                        slug,
                        old_text,
                        new_text,
                        reason,
                        &known_secrets,
                        cfg.max_history_snapshots,
                        cfg.require_approval,
                    )?;
                    Ok(ToolOutput {
                        content: format!("Patched skill `{}` (~{} tok).", doc.slug, doc.token_estimate),
                        is_error: false,
                        diff: Some(diff),
                        metadata: Some(json!({
                            "skill": doc,
                            "backup_snapshot": snap,
                        })),
                    })
                }
                SkillManageAction::WriteSubfile => {
                    let slug = req_str(&args, "slug")?;
                    let file_path = req_str(&args, "file_path")?;
                    let content = req_str(&args, "content")?;
                    let doc = snapshots.write_skill_subfile_with_secrets(
                        slug,
                        file_path,
                        content,
                        &known_secrets,
                    )?;
                    Ok(ToolOutput {
                        content: format!(
                            "Wrote subfile `{file_path}` inside skill `{}` (total subfiles: {}).",
                            doc.slug,
                            doc.subfiles.len()
                        ),
                        is_error: false,
                        diff: None,
                        metadata: Some(json!({ "skill": doc })),
                    })
                }
                SkillManageAction::RemoveSubfile => {
                    let slug = req_str(&args, "slug")?;
                    let file_path = req_str(&args, "file_path")?;
                    let removed = snapshots.remove_skill_subfile(slug, file_path)?;
                    Ok(ToolOutput::ok(match removed {
                        true => format!("Removed subfile `{file_path}` from skill `{slug}`."),
                        false => format!("Subfile `{file_path}` not found in skill `{slug}`."),
                    }))
                }
                SkillManageAction::Toggle => {
                    let slug = req_str(&args, "slug")?;
                    let enabled = args
                        .get("enabled")
                        .and_then(Value::as_bool)
                        .ok_or_else(|| ZenkoError::Tool("missing boolean `enabled` for `toggle`".into()))?;
                    if enabled
                        && let Some(existing) = snapshots.get_global_skill(slug)?
                            && existing.status == SkillStatus::Draft {
                                return Err(ZenkoError::PolicyDenied(format!(
                                    "Cannot enable draft skill `{}` before operator approval (`zenko skills approve {}` or WebUI Approve).",
                                    existing.slug, existing.slug
                                )));
                            }
                    let (doc, snap) =
                        snapshots.toggle_skill(slug, enabled, cfg.max_history_snapshots)?;
                    Ok(ToolOutput {
                        content: format!("Set skill `{}` enabled={}.", doc.slug, doc.enabled),
                        is_error: false,
                        diff: None,
                        metadata: Some(json!({ "skill": doc, "backup_snapshot": snap })),
                    })
                }
                SkillManageAction::Approve => Err(ZenkoError::PolicyDenied(
                    "Draft skill approval requires an operator action (`zenko skills approve <slug>` or WebUI Approve button).".into(),
                )),
                SkillManageAction::History => {
                    let slug_filter = opt_str(&args, "slug");
                    let items = snapshots.list_skill_history(slug_filter)?;
                    if items.is_empty() {
                        return Ok(ToolOutput::ok("No skill history snapshots found."));
                    }
                    let lines = items
                        .iter()
                        .take(SKILL_HISTORY_LIST_DEFAULT_LIMIT)
                        .map(|h| {
                            format!(
                                "- `{}` (slug=`{}`, reason={}, {}B, {})",
                                h.id,
                                h.slug,
                                h.reason,
                                h.bytes,
                                h.created_at.format("%Y-%m-%d %H:%M UTC")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    Ok(ToolOutput {
                        content: lines,
                        is_error: false,
                        diff: None,
                        metadata: Some(json!({ "snapshots": items })),
                    })
                }
                SkillManageAction::Rollback => {
                    let snapshot_id = req_str(&args, "snapshot_id")?;
                    let (doc, diff) = snapshots.rollback_skill_guarded(
                        snapshot_id,
                        cfg.max_history_snapshots,
                        cfg.require_approval,
                    )?;
                    Ok(ToolOutput {
                        content: format!(
                            "Rolled back skill `{}` to snapshot `{snapshot_id}` (status={}, enabled={}).",
                            doc.slug,
                            doc.status.as_str(),
                            doc.enabled
                        ),
                        is_error: false,
                        diff: Some(diff),
                        metadata: Some(json!({ "skill": doc })),
                    })
                }
                SkillManageAction::Delete => {
                    let slug = req_str(&args, "slug")?;
                    let deleted = snapshots.delete_skill(slug)?;
                    Ok(ToolOutput::ok(match deleted {
                        true => format!("Deleted skill `{slug}` (backup saved in `.history/`)."),
                        false => format!("Skill `{slug}` not found."),
                    }))
                }
            }
        })
        .await
        .map_err(|e| ZenkoError::Tool(format!("skill_manage worker failed: {e}")))?
    }
}

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ZenkoError::Tool(format!("missing `{key}` for `skill_manage`")))
}

fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn str_array(args: &Value, key: &str) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use zenko_core::new_id;

    #[tokio::test]
    async fn skill_manage_tool_enforces_draft_approval_gate_against_all_bypass_vectors() {
        let base = std::env::temp_dir().join(format!("zenko_sk_tool_{}", new_id()));
        let snaps = Arc::new(SnapshotManager::new(base.join("data"), base.join("ws")));
        snaps.ensure_defaults().expect("defaults");
        let vault = Arc::new(SecretVault::open(base.join("vault.enc"), base.join("vault.key")));
        let cfg = Arc::new(RwLock::new(SkillsConfig {
            require_approval: true,
            ..SkillsConfig::default()
        }));
        let tool = SkillManageTool::new(snaps.clone(), vault, cfg);
        let ctx = ToolContext {
            session_id: "sess_1".into(),
            workspace_dir: base.join("ws"),
            data_dir: base.join("data"),
            hub_dir: base.join("hub"),
            shell_timeout_secs: 10,
            auto_background_after_secs: 0,
            max_output_bytes: 8192,
            headless_no_wait: true,
            demote_bg: Arc::new(tokio::sync::Notify::new()),
            file_state: None,
        };
        let cancel = CancellationToken::new();

        // 1. Create stages draft when require_approval=true
        let out = tool
            .execute(
                &ctx,
                json!({
                    "action": "create",
                    "slug": "deploy-guide",
                    "description": "Safe deploy",
                    "content": "1. Run tests"
                }),
                cancel.clone(),
            )
            .await
            .expect("create draft");
        assert!(out.content.contains("draft"));

        // 2. Duplicate create cannot overwrite existing skill
        assert!(
            tool.execute(
                &ctx,
                json!({
                    "action": "create",
                    "slug": "deploy-guide",
                    "content": "Overwritten"
                }),
                cancel.clone(),
            )
            .await
            .is_err()
        );

        // 3. Patch cannot promote status: draft -> active or enable it
        assert!(
            tool.execute(
                &ctx,
                json!({
                    "action": "patch",
                    "slug": "deploy-guide",
                    "old_text": "status: draft",
                    "new_text": "status: active"
                }),
                cancel.clone(),
            )
            .await
            .is_err()
        );
        assert!(
            tool.execute(
                &ctx,
                json!({
                    "action": "patch",
                    "slug": "deploy-guide",
                    "old_text": "enabled: false",
                    "new_text": "enabled: true"
                }),
                cancel.clone(),
            )
            .await
            .is_err()
        );

        // 4. Toggle cannot enable a draft skill
        assert!(
            tool.execute(
                &ctx,
                json!({
                    "action": "toggle",
                    "slug": "deploy-guide",
                    "enabled": true
                }),
                cancel.clone(),
            )
            .await
            .is_err()
        );

        // 5. Approve via agent tool is denied
        assert!(
            tool.execute(
                &ctx,
                json!({ "action": "approve", "slug": "deploy-guide" }),
                cancel.clone(),
            )
            .await
            .is_err()
        );

        // 6. Operator approves -> active -> patch creates active snapshot -> delete -> agent rollback stays draft!
        let _ = snaps.approve_skill("deploy-guide", 10).expect("op approve");
        let _ = tool
            .execute(
                &ctx,
                json!({
                    "action": "patch",
                    "slug": "deploy-guide",
                    "old_text": "1. Run tests",
                    "new_text": "1. Run cargo test"
                }),
                cancel.clone(),
            )
            .await
            .expect("patch active ok");
        let hist = snaps.list_skill_history(Some("deploy-guide")).expect("hist");
        let active_snap = hist
            .iter()
            .find(|h| h.reason == "agent_patch")
            .expect("active snap");
        let _ = tool
            .execute(
                &ctx,
                json!({ "action": "delete", "slug": "deploy-guide" }),
                cancel.clone(),
            )
            .await
            .expect("delete");
        let rb_out = tool
            .execute(
                &ctx,
                json!({ "action": "rollback", "snapshot_id": active_snap.id }),
                cancel,
            )
            .await
            .expect("rollback");
        assert!(rb_out.content.contains("status=draft"));
        let restored = snaps
            .get_global_skill("deploy-guide")
            .expect("get")
            .expect("exists");
        assert_eq!(restored.status, SkillStatus::Draft);
        assert!(!restored.enabled);

        let _ = std::fs::remove_dir_all(base);
    }
}
