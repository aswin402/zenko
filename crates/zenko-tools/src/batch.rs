//! Wave-Aware Composable Dataflow Batch Tool (`batch_tools`) & KV-Cache-Safe Universal
//! Deferred Dispatcher (`invoke_tool`).
//!
//! - `batch_tools`: Pure-Rust, zero-Python programmatic tool chaining — executes 1..=12
//!   heterogeneous tool calls with parallel wave concurrency (`stop_on_error: false`),
//!   sequential short-circuit pipelines (`stop_on_error: true`), OR stateful variable-bound
//!   dataflow pipelines (`as`, `quiet`, `extract`, `${var}` / `${var.first_line}` / `${var.line[i]}`).
//! - `invoke_tool`: Ultra-lean eager proxy allowing the LLM to invoke ANY permitted deferred
//!   or external MCP tool discovered via `tool_search` while keeping the top-level provider
//!   `tools[]` JSON array 100% byte-frozen across the entire session (zero KV prompt cache busts!).
//!
//! Every inner call passes through the runtime's full role scope, Plan Mode, `ExecPolicy`,
//! argument coercion, hooks, `SessionFileTracker`, and warm LSP Diagnostics-on-Edit.

use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    BATCH_EXTRACTS, BATCH_MAX_VAR_BYTES, BATCH_TOOLS_DEFAULT_STEP_CHARS, BATCH_TOOLS_MAX_CALLS,
    BATCH_TOOLS_MAX_STEP_CHARS, BATCH_TOOLS_MIN_STEP_CHARS, BatchToolCallItem, BatchToolRequest,
    Result, RiskLevel, TOOL_BATCH_TOOLS, TOOL_INVOKE_TOOL, Tool, ToolBatchBridge, ToolConcurrency,
    ToolContext, ToolExposure, ToolOutput, ToolSet, ToolSpec, ZenkoError,
};

// ─── Zero-Regex Dataflow Projection & Template Interpolation ──────────────────

/// Returns `true` when any step in `calls` binds a variable (`as`) or references a `${...}` template.
pub fn batch_uses_dataflow(calls: &[BatchToolCallItem]) -> bool {
    calls
        .iter()
        .any(|c| c.bind_as.is_some() || json_contains_template(&c.args))
}

fn json_contains_template(val: &Value) -> bool {
    match val {
        Value::String(s) => s.contains("${"),
        Value::Array(arr) => arr.iter().any(json_contains_template),
        Value::Object(map) => map.values().any(json_contains_template),
        _ => false,
    }
}

/// Pure-Rust zero-regex deterministic output projection for dataflow `batch_tools` steps.
/// Supports `raw`, `lines`, `first_line`, `paths`, `contains:<needle>`, `head:<n>`, and `tail:<n>`.
pub fn extract_step_output(raw: &str, expr: Option<&str>) -> String {
    let Some(spec) = expr.map(str::trim).filter(|s| !s.is_empty()) else {
        return cap_var_bytes(raw);
    };
    let lower = spec.to_ascii_lowercase();
    let result = match lower.as_str() {
        "raw" => raw.to_string(),
        "first_line" => raw
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("")
            .to_string(),
        "lines" => raw
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        "paths" => extract_unique_paths(raw).join("\n"),
        _ => match spec.split_once(':') {
            Some((op, arg)) => match op.trim().to_ascii_lowercase().as_str() {
                "contains" => {
                    let needle = arg.to_ascii_lowercase();
                    raw.lines()
                        .filter(|l| l.to_ascii_lowercase().contains(&needle))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
                "head" => {
                    let n = arg.trim().parse::<usize>().unwrap_or(20).clamp(1, 500);
                    raw.lines().take(n).collect::<Vec<_>>().join("\n")
                }
                "tail" => {
                    let n = arg.trim().parse::<usize>().unwrap_or(20).clamp(1, 500);
                    let all: Vec<&str> = raw.lines().collect();
                    let start = all.len().saturating_sub(n);
                    all[start..].join("\n")
                }
                _ => raw.to_string(),
            },
            None => raw.to_string(),
        },
    };
    cap_var_bytes(&result)
}

fn cap_var_bytes(s: &str) -> String {
    match s.len() <= BATCH_MAX_VAR_BYTES {
        true => s.to_string(),
        false => s[..s.floor_char_boundary(BATCH_MAX_VAR_BYTES)].to_string(),
    }
}

fn extract_unique_paths(raw: &str) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("---")
            || trimmed.starts_with("Matches")
            || trimmed.starts_with("Matched ")
            || trimmed.starts_with("Found ")
            || trimmed.starts_with("No ")
            || trimmed.ends_with(':')
        {
            continue;
        }
        let without_bullet = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .unwrap_or(trimmed)
            .trim();
        // Strip `path:line:...` search suffix while preserving `hub:path` prefix cleanly.
        let candidate = match without_bullet.strip_prefix("hub:") {
            Some(rest) => {
                let path_part = rest.split(':').next().unwrap_or(rest).trim();
                format!("hub:{path_part}")
            }
            None => without_bullet
                .split([':', ' ', '\t'])
                .next()
                .unwrap_or("")
                .trim()
                .to_string(),
        };
        let clean = candidate.trim_matches(['`', '"', '\'']).to_string();
        if !clean.is_empty()
            && (clean.contains('/') || clean.contains('.') || clean.starts_with("hub:"))
            && seen.insert(clean.clone())
        {
            out.push(clean);
        }
    }
    out
}

/// Recursively substitutes `${var}`, `${var.first_line}`, `${var.last_line}`, and `${var.line[i]}`
/// placeholders inside a step's JSON arguments using previously bound variables (zero regex).
pub fn interpolate_args_templates(val: Value, vars: &HashMap<String, String>) -> Value {
    if vars.is_empty() {
        return val;
    }
    match val {
        Value::String(s) => Value::String(interpolate_string_templates(&s, vars)),
        Value::Array(arr) => Value::Array(
            arr.into_iter()
                .map(|v| interpolate_args_templates(v, vars))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, interpolate_args_templates(v, vars)))
                .collect(),
        ),
        other => other,
    }
}

fn interpolate_string_templates(input: &str, vars: &HashMap<String, String>) -> String {
    if !input.contains("${") {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(open) = rest.find("${") {
        out.push_str(&rest[..open]);
        let after_open = &rest[open + 2..];
        let Some(close) = after_open.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let expr = after_open[..close].trim();
        match resolve_template_expr(expr, vars) {
            Some(val) => out.push_str(&val),
            None => {
                out.push_str("${");
                out.push_str(expr);
                out.push('}');
            }
        }
        rest = &after_open[close + 1..];
    }
    out.push_str(rest);
    out
}

fn resolve_template_expr(expr: &str, vars: &HashMap<String, String>) -> Option<String> {
    if let Some(exact) = vars.get(expr) {
        return Some(exact.clone());
    }
    let (var_name, selector) = expr.split_once('.')?;
    let val = vars.get(var_name.trim())?;
    let sel = selector.trim();
    match sel {
        "first_line" => Some(
            val.lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .to_string(),
        ),
        "last_line" => Some(
            val.lines()
                .rev()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .to_string(),
        ),
        "count" => Some(
            val.lines()
                .filter(|l| !l.trim().is_empty())
                .count()
                .to_string(),
        ),
        _ => {
            let idx_str = sel
                .strip_prefix("line[")
                .or_else(|| sel.strip_prefix("paths["))
                .and_then(|s| s.strip_suffix(']'))?;
            let idx = idx_str.trim().parse::<usize>().ok()?;
            Some(
                val.lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .nth(idx)
                    .unwrap_or("")
                    .to_string(),
            )
        }
    }
}

// ─── batch_tools ──────────────────────────────────────────────────────────────

pub struct BatchToolsTool {
    bridge: Arc<dyn ToolBatchBridge>,
}

impl BatchToolsTool {
    pub fn new(bridge: Arc<dyn ToolBatchBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for BatchToolsTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::CoreFs
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Global
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Eager
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_BATCH_TOOLS.into(),
            description: "Execute 1..=12 heterogeneous tool calls in one turn with wave-parallel concurrency, short-circuiting sequential pipelines (`stop_on_error: true`), OR zero-Python dataflow variable chaining (`as` + `extract` + `quiet: true` + `${var}` / `${var.first_line}` / `${var.line[0]}` in downstream args so large intermediate outputs stay off-context). Every inner call enforces full role scope, Plan Mode, ExecPolicy, and LSP diagnostics.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "calls": {
                        "type": "array",
                        "description": "Ordered list of tool invocations (1..=12). Cannot recursively call `batch_tools` or `invoke_tool`.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "tool": { "type": "string", "description": "Registered tool name (eager, deferred, or `mcp__*`)" },
                                "args": { "type": "object", "description": "JSON arguments object (supports `${var}`, `${var.first_line}`, `${var.last_line}`, `${var.line[i]}`, `${var.count}` templates from earlier `as` bindings)" },
                                "as": { "type": "string", "description": "Optional variable name binding this step's extracted output for downstream `${var}` templates" },
                                "extract": { "type": "string", "description": format!("Optional output filter before binding/display: {}", BATCH_EXTRACTS.join(", ")) },
                                "quiet": { "type": "boolean", "description": "Suppress returning bulky raw step output to context while keeping its `as` binding live for later steps (default false)" }
                            },
                            "required": ["tool"]
                        }
                    },
                    "stop_on_error": {
                        "type": "boolean",
                        "description": "When true, runs sequentially and halts remaining calls on the first error (default false; automatically sequential when dataflow `as`/`${...}` bindings are used)."
                    },
                    "max_chars_per_call": {
                        "type": "integer",
                        "description": "Maximum characters kept per inner call output (default 4000, max 16000) to prevent context bloat."
                    }
                },
                "required": ["calls"]
            }),
            risk: RiskLevel::Safe,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let raw_calls = args
            .get("calls")
            .and_then(Value::as_array)
            .ok_or_else(|| ZenkoError::Tool("missing `calls` array".into()))?;
        if raw_calls.is_empty() {
            return Ok(ToolOutput::err(
                "`calls` must contain at least one tool invocation.",
            ));
        }
        if raw_calls.len() > BATCH_TOOLS_MAX_CALLS {
            return Ok(ToolOutput::err(format!(
                "`calls` exceeds maximum batch size of {BATCH_TOOLS_MAX_CALLS} (got {}). Split into smaller batches.",
                raw_calls.len()
            )));
        }

        let mut calls = Vec::with_capacity(raw_calls.len());
        for (idx, item) in raw_calls.iter().enumerate() {
            let tool = item
                .get("tool")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    ZenkoError::Tool(format!("calls[{idx}] is missing non-empty `tool` name"))
                })?;
            if tool == TOOL_BATCH_TOOLS || tool == TOOL_INVOKE_TOOL {
                return Ok(ToolOutput::err(format!(
                    "Recursive `{tool}` inside `{TOOL_BATCH_TOOLS}` is forbidden."
                )));
            }
            let call_args = item
                .get("args")
                .or_else(|| item.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let bind_as = item
                .get("as")
                .or_else(|| item.get("bind_as"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from);
            let quiet = item
                .get("quiet")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let extract = item
                .get("extract")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from);
            calls.push(BatchToolCallItem {
                tool: tool.to_string(),
                args: call_args,
                bind_as,
                quiet,
                extract,
            });
        }

        let stop_on_error = args
            .get("stop_on_error")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let max_chars_per_call = args
            .get("max_chars_per_call")
            .and_then(Value::as_u64)
            .unwrap_or(BATCH_TOOLS_DEFAULT_STEP_CHARS as u64)
            .clamp(
                BATCH_TOOLS_MIN_STEP_CHARS as u64,
                BATCH_TOOLS_MAX_STEP_CHARS as u64,
            ) as usize;

        let req = BatchToolRequest {
            calls,
            stop_on_error,
            max_chars_per_call,
        };
        let outcomes = self.bridge.execute_batch(ctx, req, cancel).await?;

        let total = outcomes.len();
        let failed = outcomes.iter().filter(|o| o.is_error && !o.skipped).count();
        let skipped = outcomes.iter().filter(|o| o.skipped).count();
        let succeeded = total.saturating_sub(failed + skipped);

        let mut rendered = format!(
            "Batch completed ({succeeded}/{total} ok, {failed} error(s), {skipped} skipped):\n"
        );
        let mut combined_diffs = Vec::new();
        let mut step_meta = Vec::with_capacity(total);

        for step in &outcomes {
            let status_tag = match (step.skipped, step.is_error) {
                (true, _) => "SKIPPED",
                (false, true) => "ERROR",
                (false, false) => "OK",
            };
            match step.quiet && !step.is_error && !step.skipped {
                true => {
                    let var_note = match step.bound_var.as_deref() {
                        Some(v) => format!("bound `${v}` = {} chars", step.content.len()),
                        None => format!("{} chars suppressed", step.content.len()),
                    };
                    rendered.push_str(&format!(
                        "\n--- [{}/{}] `{}` ({status_tag}, quiet, {}ms; {var_note}) ---\n",
                        step.index + 1,
                        total,
                        step.tool,
                        step.duration_ms
                    ));
                }
                false => {
                    let body = truncate_chars_bounded(&step.content, max_chars_per_call);
                    rendered.push_str(&format!(
                        "\n--- [{}/{}] `{}` ({status_tag}, {}ms) ---\n{}\n",
                        step.index + 1,
                        total,
                        step.tool,
                        step.duration_ms,
                        body
                    ));
                }
            }
            if let Some(ref d) = step.diff
                && !d.is_empty()
            {
                combined_diffs.push(d.clone());
            }
            let mut step_obj = json!({
                "index": step.index,
                "tool": step.tool,
                "status": status_tag.to_ascii_lowercase(),
                "duration_ms": step.duration_ms,
                "quiet": step.quiet,
            });
            if let Some(obj) = step_obj.as_object_mut() {
                if let Some(ref cmd) = step.shell_command {
                    obj.insert("shell_command".into(), Value::String(cmd.clone()));
                }
                if let Some(ref preview) = step.raw_output_preview {
                    obj.insert("output_preview".into(), Value::String(preview.clone()));
                }
                if step.read_less_tokens_saved > 0 {
                    obj.insert(
                        "read_less_tokens_saved".into(),
                        Value::Number(step.read_less_tokens_saved.into()),
                    );
                }
                if step.output_dedup_tokens_saved > 0 {
                    obj.insert(
                        "output_dedup_tokens_saved".into(),
                        Value::Number(step.output_dedup_tokens_saved.into()),
                    );
                }
                if step.shunt_tokens_saved > 0 {
                    obj.insert(
                        zenko_core::SHUNT_BATCH_STEP_TOKENS_SAVED_KEY.into(),
                        Value::Number(step.shunt_tokens_saved.into()),
                    );
                }
                if step.ccr_recovery {
                    obj.insert("ccr_recovery".into(), Value::Bool(true));
                }
            }
            step_meta.push(step_obj);
        }

        let diff = match combined_diffs.is_empty() {
            true => None,
            false => Some(combined_diffs.join("\n")),
        };
        let all_failed = succeeded == 0 && failed > 0;
        Ok(ToolOutput {
            content: rendered,
            is_error: all_failed,
            diff,
            metadata: Some(json!({
                "total": total,
                "succeeded": succeeded,
                "failed": failed,
                "skipped": skipped,
                "steps": step_meta,
            })),
        })
    }
}

// ─── invoke_tool (KV-Cache-Safe Universal Deferred/MCP Dispatcher) ────────────

pub struct InvokeToolProxy {
    bridge: Arc<dyn ToolBatchBridge>,
}

impl InvokeToolProxy {
    pub fn new(bridge: Arc<dyn ToolBatchBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for InvokeToolProxy {
    fn toolset(&self) -> ToolSet {
        ToolSet::CoreFs
    }

    fn concurrency(&self) -> ToolConcurrency {
        // Refined dynamically to target tool's concurrency in `tool_router::classify_tool_call`.
        ToolConcurrency::Global
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Eager
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_INVOKE_TOOL.into(),
            description: "Invoke any permitted deferred or external MCP tool (from `<deferred_tools>` or `tool_search`) by name without modifying the frozen top-level `tools[]` schema array (preserves 100% provider KV prompt cache hit rates). Enforces full role scope, Plan Mode, ExecPolicy, argument coercion, and LSP diagnostics.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "tool": { "type": "string", "description": "Target deferred or `mcp__*` tool name (e.g. `git_tool`, `lsp_tool`, `web_fetch`, `memory_tool`)" },
                    "args": { "type": "object", "description": "JSON arguments object for the target tool" }
                },
                "required": ["tool"]
            }),
            risk: RiskLevel::Safe,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let target = args
            .get("tool")
            .or_else(|| args.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ZenkoError::Tool("missing `tool` name".into()))?;
        if target == TOOL_INVOKE_TOOL || target == TOOL_BATCH_TOOLS {
            return Ok(ToolOutput::err(format!(
                "Recursive `{target}` inside `{TOOL_INVOKE_TOOL}` is forbidden."
            )));
        }
        let target_args = args
            .get("args")
            .or_else(|| args.get("arguments"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        self.bridge
            .invoke_single(ctx, target.to_string(), target_args, cancel)
            .await
    }
}

fn truncate_chars_bounded(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    match char_count <= max_chars {
        true => s.to_string(),
        false => {
            let head: String = s.chars().take(max_chars).collect();
            format!(
                "{head}\n... [batch step truncated: showed {max_chars}/{char_count} chars]"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Notify;
    use zenko_core::BatchToolStepOutcome;

    struct DummyBatchBridge;

    #[async_trait]
    impl ToolBatchBridge for DummyBatchBridge {
        async fn execute_batch(
            &self,
            _ctx: &ToolContext,
            req: BatchToolRequest,
            _cancel: CancellationToken,
        ) -> Result<Vec<BatchToolStepOutcome>> {
            Ok(req
                .calls
                .into_iter()
                .enumerate()
                .map(|(index, item)| BatchToolStepOutcome {
                    index,
                    tool: item.tool,
                    is_error: false,
                    skipped: false,
                    quiet: item.quiet,
                    bound_var: item.bind_as,
                    duration_ms: 1,
                    content: format!("ran {}", item.args),
                    diff: None,
                    shell_command: None,
                    raw_output_preview: None,
                    read_less_tokens_saved: 0,
                    output_dedup_tokens_saved: 0,
                    shunt_tokens_saved: 0,
                    ccr_recovery: false,
                })
                .collect())
        }
    }

    fn dummy_ctx() -> ToolContext {
        ToolContext {
            session_id: "s1".into(),
            workspace_dir: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            hub_dir: std::env::temp_dir(),
            shell_timeout_secs: 10,
            auto_background_after_secs: 0,
            max_output_bytes: 16_384,
            headless_no_wait: true,
            demote_bg: Arc::new(Notify::new()),
            file_state: None,
        }
    }

    #[test]
    fn dataflow_extract_and_template_interpolation_work_without_regex() {
        let search_out = "src/main.rs:12: fn main()\nsrc/lib.rs:4: pub mod app;\nsrc/main.rs:20: println!()\n";
        let paths = extract_step_output(search_out, Some("paths"));
        assert_eq!(paths, "src/main.rs\nsrc/lib.rs");
        assert_eq!(
            extract_step_output(search_out, Some("contains:pub mod")),
            "src/lib.rs:4: pub mod app;"
        );
        assert_eq!(extract_step_output(&paths, Some("first_line")), "src/main.rs");

        let mut vars = HashMap::new();
        vars.insert("hits".into(), paths);
        let args = interpolate_args_templates(
            json!({
                "path": "${hits.first_line}",
                "second": "${hits.line[1]}",
                "count": "${hits.count}",
                "unknown": "${missing}"
            }),
            &vars,
        );
        assert_eq!(args["path"], "src/main.rs");
        assert_eq!(args["second"], "src/lib.rs");
        assert_eq!(args["count"], "2");
        assert_eq!(args["unknown"], "${missing}");
    }

    #[tokio::test]
    async fn batch_tools_and_invoke_tool_reject_recursion_and_run_valid_calls() {
        let bridge = Arc::new(DummyBatchBridge);
        let batch = BatchToolsTool::new(bridge.clone());
        let rec = batch
            .execute(
                &dummy_ctx(),
                json!({ "calls": [{ "tool": "batch_tools", "args": {} }] }),
                CancellationToken::new(),
            )
            .await
            .expect("rec");
        assert!(rec.is_error);
        assert!(rec.content.contains("Recursive"));

        let ok = batch
            .execute(
                &dummy_ctx(),
                json!({
                    "calls": [
                        { "tool": "read_file", "args": { "path": "a.rs" }, "as": "a", "quiet": true },
                        { "tool": "glob_find", "args": { "pattern": "*.rs" } }
                    ]
                }),
                CancellationToken::new(),
            )
            .await
            .expect("ok");
        assert!(!ok.is_error);
        assert!(ok.content.contains("Batch completed (2/2 ok"));
        assert!(ok.content.contains("bound `$a`"));

        let proxy = InvokeToolProxy::new(bridge);
        let proxy_rec = proxy
            .execute(
                &dummy_ctx(),
                json!({ "tool": "invoke_tool", "args": {} }),
                CancellationToken::new(),
            )
            .await
            .expect("proxy_rec");
        assert!(proxy_rec.is_error);

        let proxy_ok = proxy
            .execute(
                &dummy_ctx(),
                json!({ "tool": "git_tool", "args": { "action": "status" } }),
                CancellationToken::new(),
            )
            .await
            .expect("proxy_ok");
        assert!(!proxy_ok.is_error);
        assert!(proxy_ok.content.contains("status"));
    }
}
