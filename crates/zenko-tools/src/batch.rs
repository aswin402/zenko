//! Wave-Aware Composable Batch Execution Tool (`batch_tools`).
//!
//! Pure-Rust, policy-safe alternative to Python REPL/RPC wrappers: lets any LLM (including
//! single-call/fast/local models) execute up to `BATCH_TOOLS_MAX_CALLS` heterogeneous tool calls
//! in a single turn with parallel wave concurrency (`stop_on_error: false`) or sequential
//! short-circuit pipelines (`stop_on_error: true`).
//!
//! Every inner call passes through the runtime's full role scope, Plan Mode, `ExecPolicy`,
//! argument coercion, hooks, and LSP diagnostics-on-edit (zero safety bypasses).

use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    BATCH_TOOLS_DEFAULT_STEP_CHARS, BATCH_TOOLS_MAX_CALLS, BATCH_TOOLS_MAX_STEP_CHARS,
    BATCH_TOOLS_MIN_STEP_CHARS, BatchToolCallItem, BatchToolRequest, Result, RiskLevel,
    TOOL_BATCH_TOOLS, Tool, ToolBatchBridge, ToolConcurrency, ToolContext, ToolExposure,
    ToolOutput, ToolSet, ToolSpec, ZenkoError,
};

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
        // Outer batch is a wave boundary because its inner calls run through the 3-state scheduler.
        ToolConcurrency::Global
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Eager
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_BATCH_TOOLS.into(),
            description: "Execute multiple independent or chained tool calls in a single turn with wave-parallel concurrency, bounded per-step output, and full policy/role enforcement. Ideal for fanning out multi-file reads, outlines, searches, or running short-circuiting edit/verify pipelines.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "calls": {
                        "type": "array",
                        "description": "Ordered list of tool invocations (1..=12). Cannot recursively call `batch_tools`.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "tool": { "type": "string", "description": "Registered tool name (e.g. `read_file`, `code_search`, `glob_find`, `hash_edit`, `git_tool`)" },
                                "args": { "type": "object", "description": "JSON arguments object for that tool" }
                            },
                            "required": ["tool"]
                        }
                    },
                    "stop_on_error": {
                        "type": "boolean",
                        "description": "When true, runs sequentially and halts remaining calls on the first error (pipeline mode). Default false (wave-parallel mode)."
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
            if tool == TOOL_BATCH_TOOLS {
                return Ok(ToolOutput::err(
                    "Recursive `batch_tools` inside `batch_tools` is forbidden.",
                ));
            }
            let call_args = item
                .get("args")
                .or_else(|| item.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            calls.push(BatchToolCallItem {
                tool: tool.to_string(),
                args: call_args,
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
            let body = truncate_chars_bounded(&step.content, max_chars_per_call);
            rendered.push_str(&format!(
                "\n--- [{}/{}] `{}` ({status_tag}, {}ms) ---\n{}\n",
                step.index + 1,
                total,
                step.tool,
                step.duration_ms,
                body
            ));
            if let Some(ref d) = step.diff
                && !d.is_empty()
            {
                combined_diffs.push(d.clone());
            }
            step_meta.push(json!({
                "index": step.index,
                "tool": step.tool,
                "status": status_tag.to_ascii_lowercase(),
                "duration_ms": step.duration_ms,
            }));
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
                    duration_ms: 1,
                    content: format!("ran {}", item.args),
                    diff: None,
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
        }
    }

    #[tokio::test]
    async fn batch_tools_rejects_recursion_and_runs_valid_batches() {
        let tool = BatchToolsTool::new(Arc::new(DummyBatchBridge));
        let rec = tool
            .execute(
                &dummy_ctx(),
                json!({ "calls": [{ "tool": "batch_tools", "args": {} }] }),
                CancellationToken::new(),
            )
            .await
            .expect("exec");
        assert!(rec.is_error);
        assert!(rec.content.contains("Recursive"));

        let ok = tool
            .execute(
                &dummy_ctx(),
                json!({
                    "calls": [
                        { "tool": "read_file", "args": { "path": "a.rs" } },
                        { "tool": "glob_find", "args": { "pattern": "*.rs" } }
                    ]
                }),
                CancellationToken::new(),
            )
            .await
            .expect("exec ok");
        assert!(!ok.is_error);
        assert!(ok.content.contains("2/2 ok"));
        assert!(ok.content.contains("read_file"));
        assert!(ok.content.contains("glob_find"));
    }
}
