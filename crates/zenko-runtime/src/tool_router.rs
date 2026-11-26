//! Modular Codex-inspired `ToolRouter` dispatch pipeline.
//! Separates tool lookup, fuzzy suggestions, availability gating, schema-guided coercion,
//! lifecycle hooks, ExecPolicy, HITL approval, execution, warm LSP diagnostics-on-edit,
//! and oversized artifact spilling out of the main turn loop into focused composable stages.

use crate::artifacts;
use crate::policy::{PolicyVerdict, evaluate_policy, run_hooks};
use crate::read_less;
use crate::runtime::{ActiveSessionScopeGuard, ActiveToolGuard, AgentRuntime, ExecutedToolCall};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    ARTIFACT_PATH_PREFIX, AgentEvent, ApprovalDecision, DOX_MAX_SCOPED_RULE_BYTES, HookEvent,
    MUTATING_FILE_TOOLS, ORCH_STAGE_READ_LESS, ORCH_STAGE_TOOL_ACTIVATED, PermissionMode,
    REPO_RULE_FILES, Result, ScriptBlockAction, SkillManageAction, TOOL_APPLY_PATCH,
    TOOL_CODE_SEARCH, TOOL_GIT, TOOL_HASH_EDIT, TOOL_INVOKE_TOOL, TOOL_LSP, TOOL_READ_FILE,
    TOOL_SCRIPT_BLOCK, TOOL_SEARCH_FUZZY_MAX_SUGGESTIONS, TOOL_SHELL_EXEC, TOOL_SKILL_MANAGE,
    TOOL_TOOL_SEARCH, TOOL_WRITE_FILE, Tool, ToolCall, ToolConcurrency, ToolContext, ToolOutput,
    ToolSpec, ZenkoConfig, ZenkoError, is_git_tool_args_mutating, is_lsp_tool_args_mutating,
};
use zenko_tools::coerce::{coerce_and_repair_args, format_compact_schema_hint};

/// Classifies the wave-scheduler concurrency lock for a tool invocation, escalating any
/// call requiring HITL approval or policy denial to `ToolConcurrency::Global`.
pub(crate) fn classify_tool_call(
    rt: &AgentRuntime,
    cfg: &ZenkoConfig,
    effective_permission: PermissionMode,
    tc: &ToolCall,
) -> ToolConcurrency {
    // Unwrap `invoke_tool` so parallel `Shared` / disjoint `Keyed` deferred calls run concurrently!
    if tc.name == TOOL_INVOKE_TOOL
        && let Some(target_name) = tc
            .arguments
            .get("tool")
            .or_else(|| tc.arguments.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != TOOL_INVOKE_TOOL)
    {
        let inner = ToolCall {
            id: tc.id.clone(),
            name: target_name.to_string(),
            arguments: tc
                .arguments
                .get("args")
                .or_else(|| tc.arguments.get("arguments"))
                .cloned()
                .unwrap_or(Value::Null),
        };
        return classify_tool_call(rt, cfg, effective_permission, &inner);
    }
    let Some(tool) = rt.tools.get(&tc.name) else {
        return ToolConcurrency::Global;
    };
    let base = tool.concurrency_for_args(&tc.arguments);
    if base == ToolConcurrency::Global {
        return ToolConcurrency::Global;
    }
    let allowed_set = rt.snapshot_always_allowed();
    match evaluate_policy(
        effective_permission,
        &cfg.policy,
        &tool.spec(),
        &tc.arguments,
        &allowed_set,
    ) {
        PolicyVerdict::Allow => base,
        PolicyVerdict::AskUser | PolicyVerdict::Deny(_) => ToolConcurrency::Global,
    }
}

/// Stage 1: Resolve tool by name with scoped Damerau-Levenshtein fuzzy suggestions and
/// dynamic `Tool::is_available` environment gating.
fn resolve_tool_for_session(
    rt: &AgentRuntime,
    session_id: &str,
    tc: &ToolCall,
) -> std::result::Result<Arc<dyn Tool>, ToolOutput> {
    let scope = rt.effective_session_scope(session_id);
    let Some(tool) = rt.tools.get(&tc.name) else {
        let suggestions = rt.tools.suggest_permitted_tools(
            &tc.name,
            TOOL_SEARCH_FUZZY_MAX_SUGGESTIONS,
            &scope.availability,
            |spec| scope.permits_spec(spec),
        );
        let hint = match suggestions.is_empty() {
            true => format!(
                "Unknown tool: `{}`. Use `{TOOL_TOOL_SEARCH}(action=\"list_toolsets\")` to inspect available tools.",
                tc.name
            ),
            false => format!(
                "Unknown tool: `{}`. Closest available tools: {}.",
                tc.name,
                suggestions
                    .iter()
                    .map(|s| format!("`{s}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        return Err(ToolOutput::err(hint));
    };

    if !tool.is_available(&scope.availability) {
        return Err(ToolOutput::err(format!(
            "Tool `{}` is not available in the current session environment or scope.",
            tc.name
        )));
    }
    Ok(tool)
}

fn format_tool_error_with_schema(spec: &ToolSpec, err: ZenkoError) -> ToolOutput {
    let base = format!("Tool error: {err}");
    match err {
        ZenkoError::Tool(_) => {
            let schema_hint = format_compact_schema_hint(spec);
            match schema_hint.is_empty() {
                true => ToolOutput::err(base),
                false => ToolOutput::err(format!("{base}\n{schema_hint}")),
            }
        }
        _ => ToolOutput::err(base),
    }
}

fn activate_deferred_if_needed(rt: &AgentRuntime, session_id: &str, tool_name: &str) {
    if rt.tools.is_deferred(tool_name) {
        let newly = rt.activate_deferred_tools(session_id, &[tool_name.to_string()]);
        if !newly.is_empty() {
            rt.emit(AgentEvent::OrchestratorUpdated {
                session_id: session_id.to_string(),
                stage: ORCH_STAGE_TOOL_ACTIVATED.into(),
                tier: zenko_core::ExecutionTier::Standard,
                detail: format!("Auto-activated deferred tool `{tool_name}` on direct call"),
                model: None,
            });
        }
    }
}

/// Stage 3 + 4: Execute `PreToolUse` hook, `ExecPolicy`, optional HITL approval, post-approval
/// deferred activation, and the tool implementation itself.
#[allow(clippy::too_many_arguments)]
async fn authorize_and_invoke(
    rt: &AgentRuntime,
    cfg: &ZenkoConfig,
    session_id: &str,
    effective_permission: PermissionMode,
    tool_ctx: &ToolContext,
    headless_no_wait: bool,
    tool: &Arc<dyn Tool>,
    spec: &ToolSpec,
    tc: &ToolCall,
    args_preview: &str,
    cancel: &CancellationToken,
) -> Result<ToolOutput> {
    let hook_res = run_hooks(
        &cfg.hooks,
        HookEvent::PreToolUse,
        Some(&tc.name),
        json!({ "session_id": session_id, "args": tc.arguments }),
    )
    .await;

    if let Err(blocked) = hook_res {
        return Ok(ToolOutput::err(format!(
            "Blocked by PreToolUse hook: {blocked}"
        )));
    }

    let allowed_set = rt.snapshot_always_allowed();
    let verdict = evaluate_policy(
        effective_permission,
        &cfg.policy,
        spec,
        &tc.arguments,
        &allowed_set,
    );

    let approved = match verdict {
        PolicyVerdict::Deny(reason) => {
            return Ok(ToolOutput::err(format!("Denied by execpolicy: {reason}")));
        }
        PolicyVerdict::AskUser if headless_no_wait => {
            return Ok(ToolOutput::err(format!(
                "Tool `{}` requires approval in `{}` mode (headless run did not block). Re-run with `--mode autonomous` or approve via daemon/WebUI.",
                tc.name,
                effective_permission.as_str()
            )));
        }
        PolicyVerdict::AskUser => {
            let _ = run_hooks(
                &cfg.hooks,
                HookEvent::PermissionRequest,
                Some(&tc.name),
                json!({ "session_id": session_id, "args": tc.arguments }),
            )
            .await;
            match rt
                .request_hitl_approval(
                    session_id,
                    &tc.id,
                    &tc.name,
                    args_preview,
                    spec.risk,
                    cancel,
                )
                .await?
            {
                ApprovalDecision::Approve | ApprovalDecision::AlwaysAllow => true,
                ApprovalDecision::Deny => false,
            }
        }
        PolicyVerdict::Allow => true,
    };

    if !approved {
        return Ok(ToolOutput::err(
            "Tool execution skipped (user denied or steered the turn with updated instructions).",
        ));
    }

    activate_deferred_if_needed(rt, session_id, &tc.name);
    match tool
        .execute(tool_ctx, tc.arguments.clone(), cancel.clone())
        .await
    {
        Ok(out) => Ok(out),
        Err(ZenkoError::Cancelled) => Err(ZenkoError::Cancelled),
        Err(err) => Ok(format_tool_error_with_schema(spec, err)),
    }
}

/// Deterministically extracts workspace-relative paths modified by `write_file`, `hash_edit`,
/// `apply_patch` (single-file, structured `files[]`, Codex `patch`), or `lsp_tool(rename)`
/// for warm LSP Diagnostics-on-Edit (zero regex).
pub(crate) fn extract_edited_rel_paths(
    tool_name: &str,
    args: &Value,
    metadata: Option<&Value>,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut push_unique = |raw: &str| {
        let trimmed = raw.trim();
        if !trimmed.is_empty()
            && !trimmed.starts_with("hub:")
            && !out.iter().any(|existing| existing == trimmed)
        {
            out.push(trimmed.to_string());
        }
    };

    if let Some(arr) = metadata
        .and_then(|m| m.get("edited_files"))
        .and_then(Value::as_array)
    {
        for item in arr {
            if let Some(p) = item.as_str() {
                push_unique(p);
            }
        }
    }

    match tool_name {
        TOOL_WRITE_FILE | TOOL_HASH_EDIT | TOOL_APPLY_PATCH => {
            if let Some(p) = args.get("path").and_then(Value::as_str) {
                push_unique(p);
            }
            if tool_name == TOOL_APPLY_PATCH {
                if let Some(files) = args.get("files").and_then(Value::as_array) {
                    for item in files {
                        if let Some(p) = item
                            .get("move_to")
                            .or_else(|| item.get("path"))
                            .and_then(Value::as_str)
                        {
                            push_unique(p);
                        }
                    }
                }
                if let Some(patch) = args.get("patch").and_then(Value::as_str) {
                    for line in patch.lines() {
                        let trimmed = line.trim();
                        let rel = trimmed
                            .strip_prefix("*** Update File:")
                            .or_else(|| trimmed.strip_prefix("*** Add File:"))
                            .or_else(|| trimmed.strip_prefix("*** Move to:"));
                        if let Some(p) = rel {
                            push_unique(p);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    out
}

/// Extracts workspace-relative file paths read or edited by `read_file`, `write_file`,
/// `hash_edit`, or `apply_patch` (strictly skipping `artifact:` and `hub:` handles)
/// so JIT hierarchical DOX / `AGENTS.md` ancestor rules can be injected on first access.
pub(crate) fn extract_touched_rel_paths(
    tool_name: &str,
    args: &Value,
    metadata: Option<&Value>,
) -> Vec<String> {
    match tool_name {
        TOOL_READ_FILE => args
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|p| {
                !p.is_empty()
                    && !p.starts_with(ARTIFACT_PATH_PREFIX)
                    && !p.starts_with("hub:")
            })
            .map(|p| vec![p.to_string()])
            .unwrap_or_default(),
        TOOL_WRITE_FILE | TOOL_HASH_EDIT | TOOL_APPLY_PATCH => {
            extract_edited_rel_paths(tool_name, args, metadata)
        }
        _ => Vec::new(),
    }
}

/// Walks ancestor directories of `touched_rel_paths` strictly below `workspace_dir` (never
/// re-injecting root docs already frozen in Tier 1), discovering nested `REPO_RULE_FILES`
/// ordered outer-to-inner (`parent/AGENTS.md` before `parent/child/AGENTS.md`) and skipping
/// self-reads when the caller directly opened that rule file.
pub(crate) fn discover_ancestor_scoped_rules(
    workspace_dir: &Path,
    touched_rel_paths: &[String],
) -> Vec<(String, PathBuf)> {
    let canon_ws = std::fs::canonicalize(workspace_dir).unwrap_or_else(|_| workspace_dir.to_path_buf());
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    let mut seen_rel: BTreeSet<String> = BTreeSet::new();

    for raw_rel in touched_rel_paths {
        let candidate = match Path::new(raw_rel).is_absolute() {
            true => PathBuf::from(raw_rel),
            false => workspace_dir.join(raw_rel),
        };
        let parent_dir = match candidate.is_dir() {
            true => candidate.clone(),
            false => match candidate.parent() {
                Some(p) => p.to_path_buf(),
                None => continue,
            },
        };
        let canon_dir = std::fs::canonicalize(&parent_dir).unwrap_or(parent_dir);
        if !canon_dir.starts_with(&canon_ws) || canon_dir == canon_ws {
            continue;
        }
        let touched_norm = std::fs::canonicalize(&candidate)
            .ok()
            .and_then(|c| c.strip_prefix(&canon_ws).ok().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from(raw_rel))
            .to_string_lossy()
            .replace('\\', "/");

        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut cursor = canon_dir.as_path();
        while cursor != canon_ws && cursor.starts_with(&canon_ws) {
            dirs.push(cursor.to_path_buf());
            let Some(next) = cursor.parent() else {
                break;
            };
            cursor = next;
        }
        dirs.reverse();

        for dir in dirs {
            for filename in REPO_RULE_FILES {
                let doc_path = dir.join(filename);
                if !doc_path.is_file() {
                    continue;
                }
                let Ok(rel) = doc_path.strip_prefix(&canon_ws) else {
                    continue;
                };
                let rel_posix = rel.to_string_lossy().replace('\\', "/");
                if rel_posix == touched_norm {
                    continue;
                }
                if seen_rel.insert(rel_posix.clone()) {
                    found.push((rel_posix, doc_path));
                }
            }
        }
    }

    found
}

fn read_capped_scoped_rule(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(match trimmed.len() <= DOX_MAX_SCOPED_RULE_BYTES {
        true => trimmed.to_string(),
        false => {
            let cut = trimmed.floor_char_boundary(DOX_MAX_SCOPED_RULE_BYTES);
            format!(
                "{}\n... [truncated at {} KiB scoped-rule cap]",
                &trimmed[..cut],
                DOX_MAX_SCOPED_RULE_BYTES / 1024
            )
        }
    })
}

/// Full 5-stage `ToolRouter` dispatch pipeline shared by top-level turns and `batch_tools`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_tool_call(
    rt: &AgentRuntime,
    cfg: &ZenkoConfig,
    session_id: &str,
    effective_permission: PermissionMode,
    tool_ctx: &ToolContext,
    headless_no_wait: bool,
    mut tc: ToolCall,
    cancel: &CancellationToken,
) -> Result<ExecutedToolCall> {
    if cancel.is_cancelled() {
        return Err(ZenkoError::Cancelled);
    }

    let start_ts = Instant::now();

    // Stage 1: Resolve tool & dynamic availability gate.
    let tool = match resolve_tool_for_session(rt, session_id, &tc) {
        Ok(t) => t,
        Err(output) => {
            return Ok(ExecutedToolCall {
                tool_call_id: tc.id,
                tool_name: tc.name,
                arguments: tc.arguments,
                output,
                duration_ms: 0,
            });
        }
    };

    // Stage 2: Schema-guided zero-regex argument coercion & preview.
    let spec = tool.spec();
    tc.arguments = coerce_and_repair_args(tc.arguments, &spec.parameters);
    let args_preview = truncate_preview(&tc.arguments.to_string(), 320);

    let _active_tool_guard =
        ActiveToolGuard::enter(rt.session_control(session_id), tc.name.clone());
    // Ensure composite proxies (`invoke_tool`, `batch_tools`) inherit the strictest effective
    // permission even when `dispatch_tool_call` is invoked directly outside a full turn wrapper.
    let current_scope = rt.effective_session_scope(session_id);
    let strictest_perm = PermissionMode::strictest(current_scope.permission, effective_permission);
    let _tightened_scope = match strictest_perm != current_scope.permission {
        true => {
            let mut next_scope = current_scope;
            next_scope.permission = strictest_perm;
            if strictest_perm == PermissionMode::Plan {
                next_scope.availability.read_only = true;
            }
            Some(ActiveSessionScopeGuard::enter(
                rt.session_control(session_id),
                next_scope,
            ))
        }
        false => None,
    };
    rt.emit(AgentEvent::ToolCallStarted {
        session_id: session_id.to_string(),
        call_id: tc.id.clone(),
        tool_name: tc.name.clone(),
        args_preview: args_preview.clone(),
        risk: spec.risk,
    });

    // Stage 3 & 4: PreToolUse hook, ExecPolicy, HITL approval, deferred activation, execution.
    let mut tool_output = authorize_and_invoke(
        rt,
        cfg,
        session_id,
        effective_permission,
        tool_ctx,
        headless_no_wait,
        &tool,
        &spec,
        &tc,
        &args_preview,
        cancel,
    )
    .await?;

    // Stage 5: Post-execution enrichers (mutation watermark, warm LSP diagnostics, Read-Less + CCR, artifact spill, telemetry).
    let mutated_workspace = !tool_output.is_error
        && (MUTATING_FILE_TOOLS.contains(&tc.name.as_str())
            || (tc.name == TOOL_GIT && is_git_tool_args_mutating(&tc.arguments))
            || (tc.name == TOOL_LSP && is_lsp_tool_args_mutating(&tc.arguments))
            || (tc.name == TOOL_SCRIPT_BLOCK
                && tc
                    .arguments
                    .get("action")
                    .and_then(Value::as_str)
                    .and_then(ScriptBlockAction::parse)
                    == Some(ScriptBlockAction::Run))
            || tc.name == TOOL_SHELL_EXEC);
    if mutated_workspace {
        rt.record_workspace_mutation();
    }

    let triggers_lsp_diag = !tool_output.is_error
        && cfg.lsp.enabled
        && cfg.lsp.auto_diagnostics_on_edit
        && (MUTATING_FILE_TOOLS.contains(&tc.name.as_str())
            || (tc.name == TOOL_LSP && is_lsp_tool_args_mutating(&tc.arguments)));
    if triggers_lsp_diag {
        for edited_rel in
            extract_edited_rel_paths(&tc.name, &tc.arguments, tool_output.metadata.as_ref())
        {
            let candidate = tool_ctx.workspace_dir.join(&edited_rel);
            if let Some(diag_block) = rt
                .lsp
                .warm_diagnostics_for_edited_file(
                    &tool_ctx.workspace_dir,
                    &candidate,
                    cancel.clone(),
                )
                .await
            {
                tool_output.content.push_str(&diag_block);
            }
        }
    }

    // Backfire / CCR recovery telemetry: track whenever `read_file` or `code_search` explicitly
    // inspects an `artifact:` handle created by Read-Less compression or oversized spilling.
    let is_ccr_recovery = !tool_output.is_error
        && matches!(tc.name.as_str(), TOOL_READ_FILE | TOOL_CODE_SEARCH)
        && tc
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|p| p.trim().starts_with(ARTIFACT_PATH_PREFIX));
    if is_ccr_recovery {
        tool_output.metadata = match tool_output.metadata {
            Some(Value::Object(mut map)) => {
                map.insert("ccr_recovery".into(), Value::Bool(true));
                Some(Value::Object(map))
            }
            Some(other) => Some(json!({ "data": other, "ccr_recovery": true })),
            None => Some(json!({ "ccr_recovery": true })),
        };
    }

    // Tier 1 (Session Output Dedup) -> Tier 2 (Custom Filters + Command-Aware Read-Less + Failure Tee) -> Tier 3 (Error-Salvaging Spill)
    let raw_bypass = tc.name == TOOL_SHELL_EXEC
        && tc
            .arguments
            .get("raw")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let (cmd_family, _) = read_less::command_shapers::classify_tool_command(&tc.name, &tc.arguments);
    let default_family_label =
        read_less::command_shapers::command_family_label(&tc.name, cmd_family, false);

    if raw_bypass {
        let bypass_meta = json!({
            "raw_bypass": true,
            "command_family": default_family_label,
        });
        tool_output.metadata = match tool_output.metadata {
            Some(Value::Object(mut map)) => {
                map.insert("raw_bypass".into(), Value::Bool(true));
                map.insert(
                    "command_family".into(),
                    Value::String(default_family_label.to_string()),
                );
                Some(Value::Object(map))
            }
            Some(other) => Some(json!({ "data": other, "raw_bypass": true, "command_family": default_family_label })),
            None => Some(bypass_meta),
        };
    }

    let raw_for_dedup_handle = match !raw_bypass
        && cfg.context.read_less
        && !tool_output.is_error
        && read_less::is_read_less_eligible(&tc.name)
    {
        true => Some(tool_output.content.clone()),
        false => None,
    };

    let mut deduped_this_call = false;
    if !raw_bypass
        && cfg.context.read_less
        && let Some(hit) = rt.check_or_record_output_dedup(
            session_id,
            &tc.name,
            &tool_output.content,
            tool_output.is_error,
        )
    {
        deduped_this_call = true;
        tool_output.content = hit.replacement;
        rt.record_read_less_savings(session_id, hit.bytes_saved as u64, hit.tokens_saved);
        rt.emit(AgentEvent::OrchestratorUpdated {
            session_id: session_id.to_string(),
            stage: ORCH_STAGE_READ_LESS.into(),
            tier: zenko_core::ExecutionTier::Standard,
            detail: format!(
                "Dedup `{}` ({}) (-{} calls): {} -> {} B (~{} tok saved)",
                tc.name,
                default_family_label,
                hit.calls_ago,
                hit.original_bytes,
                hit.compressed_bytes,
                hit.tokens_saved,
            ),
            model: None,
        });
        let dedup_meta = json!({
            "calls_ago": hit.calls_ago,
            "command_family": default_family_label,
            "original_bytes": hit.original_bytes,
            "compressed_bytes": hit.compressed_bytes,
            "bytes_saved": hit.bytes_saved,
            "tokens_saved": hit.tokens_saved,
            "prior_handle": hit.prior_handle,
        });
        let rl_meta = json!({
            "shape": "dedup",
            "command_family": default_family_label,
            "original_bytes": hit.original_bytes,
            "compressed_bytes": hit.compressed_bytes,
            "bytes_saved": hit.bytes_saved,
            "tokens_saved": hit.tokens_saved,
        });
        tool_output.metadata = match tool_output.metadata {
            Some(Value::Object(mut map)) => {
                map.insert("output_dedup".into(), dedup_meta);
                map.insert("read_less".into(), rl_meta);
                Some(Value::Object(map))
            }
            Some(other) => {
                Some(json!({ "data": other, "output_dedup": dedup_meta, "read_less": rl_meta }))
            }
            None => Some(json!({ "output_dedup": dedup_meta, "read_less": rl_meta })),
        };
    }

    let mut shaped_this_call = false;
    if !raw_bypass && !deduped_this_call && cfg.context.read_less {
        let active_zmode = rt.effective_zmode(session_id, cfg, &[]);
        let repo_scope = rt.resolve_repo_scope(Some(&tool_ctx.workspace_dir));
        let custom_ctx = read_less::CustomFilterContext {
            store: &rt.store,
            repo_scope: &repo_scope,
            data_dir: &tool_ctx.data_dir,
            workspace_dir: &tool_ctx.workspace_dir,
        };
        if let Some(rl) = read_less::try_compress_tool_output_with_context(
            &tc.name,
            &tc.arguments,
            &tool_output.content,
            cfg.context.read_less_min_bytes,
            active_zmode,
            tool_output.is_error,
            Some(&custom_ctx),
        ) {
            shaped_this_call = true;
            let raw_ccr = artifacts::persist_raw_ccr_artifact(
                &tool_ctx.data_dir,
                session_id,
                &tc.id,
                &tool_output.content,
            )
            .await;
            if let (Some(rec), Some(raw_src)) = (&raw_ccr, raw_for_dedup_handle.as_deref()) {
                rt.attach_output_dedup_handle(session_id, &tc.name, raw_src, rec.handle.clone());
            }
            let tag_prefix = match tool_output.is_error {
                true => "failure-tee",
                false => "read-less",
            };
            let shape_label = match rl.custom_filter_name.as_deref() {
                Some(fname) => format!("{} ({fname})", rl.shape.as_str()),
                None => format!("{} ({})", rl.shape.as_str(), rl.command_family),
            };
            let ccr_footer = match &raw_ccr {
                Some(rec) => format!(
                    "\n\n[{tag_prefix}: {shape_label}, {} -> {} bytes (-{:.1}%, ~{} tok saved) · raw saved at `{}`; inspect slices via `read_file` or `code_search`]",
                    rl.original_bytes,
                    rl.compressed_bytes,
                    rl.savings_pct,
                    rl.tokens_saved,
                    rec.handle,
                ),
                None => format!(
                    "\n\n[{tag_prefix}: {shape_label}, {} -> {} bytes (-{:.1}%, ~{} tok saved)]",
                    rl.original_bytes,
                    rl.compressed_bytes,
                    rl.savings_pct,
                    rl.tokens_saved,
                ),
            };
            tool_output.content = format!("{}{ccr_footer}", rl.compressed);
            rt.record_read_less_savings(session_id, rl.bytes_saved as u64, rl.tokens_saved);
            rt.emit(AgentEvent::OrchestratorUpdated {
                session_id: session_id.to_string(),
                stage: ORCH_STAGE_READ_LESS.into(),
                tier: zenko_core::ExecutionTier::Standard,
                detail: format!(
                    "{} `{}` ({shape_label}): {} -> {} B (-{:.1}%, ~{} tok saved)",
                    match tool_output.is_error {
                        true => "Failure-Tee",
                        false => "Read-Less",
                    },
                    tc.name,
                    rl.original_bytes,
                    rl.compressed_bytes,
                    rl.savings_pct,
                    rl.tokens_saved,
                ),
                model: None,
            });
            let rl_meta = json!({
                "shape": rl.shape.as_str(),
                "command_family": rl.command_family,
                "custom_filter": rl.custom_filter_name,
                "failure_tee": tool_output.is_error,
                "original_bytes": rl.original_bytes,
                "compressed_bytes": rl.compressed_bytes,
                "bytes_saved": rl.bytes_saved,
                "tokens_saved": rl.tokens_saved,
                "savings_pct": rl.savings_pct,
                "raw_handle": raw_ccr.as_ref().map(|r| r.handle.as_str()),
            });
            tool_output.metadata = match tool_output.metadata {
                Some(Value::Object(mut map)) => {
                    map.insert("read_less".into(), rl_meta);
                    if let Some(rec) = raw_ccr {
                        map.insert(
                            "artifact".into(),
                            serde_json::to_value(&rec).unwrap_or(Value::Null),
                        );
                    }
                    Some(Value::Object(map))
                }
                Some(other) => Some(json!({ "data": other, "read_less": rl_meta })),
                None => Some(json!({ "read_less": rl_meta })),
            };
        }
    }

    // Native Failure Tee fallback: persist full raw error output even if it stayed uncompressed
    // and below `max_output_bytes`, guaranteeing zero hidden error lines across retries.
    if !shaped_this_call
        && !deduped_this_call
        && tool_output.is_error
        && cfg.context.artifact_spilling
        && read_less::is_read_less_eligible(&tc.name)
        && tool_output.content.len() >= zenko_core::READ_LESS_FAILURE_TEE_MIN_BYTES
        && tool_output.content.len() <= tool_ctx.max_output_bytes
        && !tool_output.content.contains(ARTIFACT_PATH_PREFIX)
        && let Some(rec) = artifacts::persist_raw_ccr_artifact(
            &tool_ctx.data_dir,
            session_id,
            &tc.id,
            &tool_output.content,
        )
        .await
    {
        tool_output.content.push_str(&format!(
            "\n\n[failure-tee: full raw error output ({} bytes) archived at `{}`]",
            rec.total_bytes, rec.handle
        ));
        let art_val = serde_json::to_value(&rec).unwrap_or(Value::Null);
        tool_output.metadata = match tool_output.metadata {
            Some(Value::Object(mut map)) => {
                map.insert("artifact".into(), art_val);
                Some(Value::Object(map))
            }
            Some(other) => Some(json!({ "data": other, "artifact": art_val })),
            None => Some(json!({ "artifact": art_val })),
        };
    }

    if cfg.context.artifact_spilling && tool_output.content.len() > tool_ctx.max_output_bytes {
        let (spilled_content, spill_rec) = artifacts::spill_tool_output_if_oversized(
            &tool_ctx.data_dir,
            session_id,
            &tc.id,
            &tc.name,
            tool_output.content,
            tool_ctx.max_output_bytes,
        )
        .await;
        tool_output.content = spilled_content;
        if let Some(rec) = spill_rec {
            if let Some(raw_src) = raw_for_dedup_handle.as_deref() {
                rt.attach_output_dedup_handle(session_id, &tc.name, raw_src, rec.handle.clone());
            }
            let art_val = serde_json::to_value(&rec).unwrap_or(Value::Null);
            tool_output.metadata = match tool_output.metadata {
                Some(Value::Object(mut map)) => {
                    map.insert("artifact".into(), art_val);
                    Some(Value::Object(map))
                }
                Some(other) => Some(json!({ "data": other, "artifact": art_val })),
                None => Some(json!({ "artifact": art_val })),
            };
        }
    }

    // Just-In-Time (JIT) hierarchical DOX / nested `AGENTS.md` ancestor auto-injection:
    // runs strictly AFTER artifact spilling so injected `<scoped_repo_rules>` blocks are never
    // truncated or hidden inside an oversized spill artifact (zero Tier-1 KV invalidation).
    if !tool_output.is_error && cfg.context.auto_repo_rules && !is_ccr_recovery {
        let touched = extract_touched_rel_paths(&tc.name, &tc.arguments, tool_output.metadata.as_ref());
        if !touched.is_empty() {
            let candidates = discover_ancestor_scoped_rules(&tool_ctx.workspace_dir, &touched);
            let mut injected_paths: Vec<String> = Vec::new();
            for (rel_doc, full_doc) in candidates {
                if !rt.claim_unloaded_scoped_rule(session_id, &rel_doc) {
                    continue;
                }
                if let Some(body) = read_capped_scoped_rule(&full_doc) {
                    tool_output.content.push_str(&format!(
                        "\n\n<scoped_repo_rules path=\"{rel_doc}\">\n{body}\n</scoped_repo_rules>"
                    ));
                    injected_paths.push(rel_doc);
                }
            }
            if !injected_paths.is_empty() {
                let val = Value::Array(injected_paths.into_iter().map(Value::String).collect());
                tool_output.metadata = match tool_output.metadata {
                    Some(Value::Object(mut map)) => {
                        map.insert("scoped_rules_injected".into(), val);
                        Some(Value::Object(map))
                    }
                    Some(other) => Some(json!({ "data": other, "scoped_rules_injected": val })),
                    None => Some(json!({ "scoped_rules_injected": val })),
                };
            }
        }
    }

    let duration_ms = start_ts.elapsed().as_millis() as u64;

    let _ = run_hooks(
        &cfg.hooks,
        HookEvent::PostToolUse,
        Some(&tc.name),
        json!({
            "session_id": session_id,
            "is_error": tool_output.is_error,
            "duration_ms": duration_ms
        }),
    )
    .await;

    rt.emit(AgentEvent::ToolCallFinished {
        session_id: session_id.to_string(),
        call_id: tc.id.clone(),
        tool_name: tc.name.clone(),
        is_error: tool_output.is_error,
        duration_ms,
        diff: tool_output.diff.clone(),
        output_preview: truncate_preview(&tool_output.content, 600),
    });

    if tc.name == TOOL_SKILL_MANAGE
        && !tool_output.is_error
        && let Some(action_raw) = tc.arguments.get("action").and_then(Value::as_str)
        && let Some(action) = SkillManageAction::parse(action_raw)
        && action.is_mutating()
    {
        let meta_skill = tool_output.metadata.as_ref().and_then(|m| m.get("skill"));
        let slug = meta_skill
            .and_then(|s| s.get("slug"))
            .and_then(Value::as_str)
            .or_else(|| tc.arguments.get("slug").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        let status = meta_skill
            .and_then(|s| s.get("status"))
            .and_then(Value::as_str)
            .unwrap_or("active")
            .to_string();
        if !slug.is_empty() {
            rt.emit(AgentEvent::SkillUpdated {
                slug,
                action: action_raw.to_string(),
                status,
            });
        }
    }

    Ok(ExecutedToolCall {
        tool_call_id: tc.id,
        tool_name: tc.name,
        arguments: tc.arguments,
        output: tool_output,
        duration_ms,
    })
}

fn truncate_preview(s: &str, max: usize) -> String {
    match s.len() <= max {
        true => s.to_string(),
        false => format!("{}…", &s[..s.floor_char_boundary(max)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jit_ancestor_scoped_rule_discovery_orders_outer_to_inner_and_skips_root_and_self() {
        let ws = std::env::temp_dir().join(format!("zenko_jit_dox_{}", zenko_core::new_id()));
        std::fs::create_dir_all(ws.join("crates/api/src/handlers")).expect("mkdir");
        std::fs::write(ws.join("AGENTS.md"), "# Root rules").expect("root");
        std::fs::write(ws.join("crates/AGENTS.md"), "# Crates rules").expect("crates");
        std::fs::write(ws.join("crates/api/ZENKO.md"), "# API rules").expect("api");
        std::fs::write(ws.join("crates/api/src/handlers/auth.rs"), "pub fn ok() {}").expect("auth");

        let touched = extract_touched_rel_paths(
            TOOL_READ_FILE,
            &json!({ "path": "crates/api/src/handlers/auth.rs" }),
            None,
        );
        let found: Vec<String> = discover_ancestor_scoped_rules(&ws, &touched)
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        assert_eq!(found, vec!["crates/AGENTS.md", "crates/api/ZENKO.md"]);

        // Direct self-read of `crates/api/ZENKO.md` must not duplicate itself, while still seeing outer parent.
        let self_touched = extract_touched_rel_paths(
            TOOL_READ_FILE,
            &json!({ "path": "crates/api/ZENKO.md" }),
            None,
        );
        let self_found: Vec<String> = discover_ancestor_scoped_rules(&ws, &self_touched)
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        assert_eq!(self_found, vec!["crates/AGENTS.md"]);

        // Artifact and hub paths must never trigger workspace ancestor injection.
        assert!(
            extract_touched_rel_paths(
                TOOL_READ_FILE,
                &json!({ "path": "artifact:s1/c1.txt" }),
                None
            )
            .is_empty()
        );

        let _ = std::fs::remove_dir_all(ws);
    }
}
