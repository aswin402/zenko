//! Modular Codex-inspired `ToolRouter` dispatch pipeline.
//! Separates tool lookup, fuzzy suggestions, availability gating, schema-guided coercion,
//! lifecycle hooks, ExecPolicy, HITL approval, execution, warm LSP diagnostics-on-edit,
//! and oversized artifact spilling out of the main turn loop into focused composable stages.

use crate::artifacts;
use crate::policy::{PolicyVerdict, evaluate_policy, run_hooks};
use crate::read_less;
use crate::runtime::{ActiveSessionScopeGuard, ActiveToolGuard, AgentRuntime, ExecutedToolCall};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    AgentEvent, ApprovalDecision, HookEvent, MUTATING_FILE_TOOLS, ORCH_STAGE_READ_LESS,
    ORCH_STAGE_TOOL_ACTIVATED, PermissionMode, Result, ScriptBlockAction, SkillManageAction,
    TOOL_APPLY_PATCH, TOOL_GIT, TOOL_HASH_EDIT, TOOL_INVOKE_TOOL, TOOL_LSP, TOOL_SCRIPT_BLOCK,
    TOOL_SEARCH_FUZZY_MAX_SUGGESTIONS, TOOL_SHELL_EXEC, TOOL_SKILL_MANAGE, TOOL_TOOL_SEARCH,
    TOOL_WRITE_FILE, Tool, ToolCall, ToolConcurrency, ToolContext, ToolOutput, ToolSpec,
    ZenkoConfig, ZenkoError, is_git_tool_args_mutating, is_lsp_tool_args_mutating,
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

    if cfg.context.read_less
        && let Some(rl) = read_less::try_compress_tool_output(
            &tc.name,
            &tool_output.content,
            cfg.context.read_less_min_bytes,
        )
    {
        let raw_ccr = artifacts::persist_raw_ccr_artifact(
            &tool_ctx.data_dir,
            session_id,
            &tc.id,
            &tool_output.content,
        )
        .await;
        let ccr_footer = match &raw_ccr {
            Some(rec) => format!(
                "\n\n[read-less: {}, {} -> {} bytes (-{:.1}%, ~{} tok saved) · raw saved at `{}`; inspect slices via `read_file` or `code_search`]",
                rl.shape.as_str(),
                rl.original_bytes,
                rl.compressed_bytes,
                rl.savings_pct,
                rl.tokens_saved,
                rec.handle,
            ),
            None => format!(
                "\n\n[read-less: {}, {} -> {} bytes (-{:.1}%, ~{} tok saved)]",
                rl.shape.as_str(),
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
                "Read-Less `{}` ({}): {} -> {} B (-{:.1}%, ~{} tok saved)",
                tc.name,
                rl.shape.as_str(),
                rl.original_bytes,
                rl.compressed_bytes,
                rl.savings_pct,
                rl.tokens_saved,
            ),
            model: None,
        });
        let rl_meta = json!({
            "shape": rl.shape.as_str(),
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
