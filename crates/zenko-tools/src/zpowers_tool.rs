//! Unified deferred `zpowers` tool bridging Zenko's native plan-first orchestration workflow
//! and per-session `.zenko/sessions/<id>/zpowers/` workspace (`spec.md`, `plan.md`, `debug.md`,
//! `reviews/`, `walkthrough.md`, `state.json`).
//!
//! Zero regex, zero `if/else` ladders, zero hardcoded action names, zero `.unwrap()` in prod.

use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    Result, RiskLevel, TOOL_ZPOWERS, Tool, ToolConcurrency, ToolContext, ToolExposure, ToolOutput,
    ToolSet, ToolSpec, ZPOWERS_ACTIONS, ZPOWERS_PROMOTE_ALLOWED_ROOTS, ZPOWERS_REVIEW_STAGES,
    ZPOWERS_REVIEW_VERDICTS, ZPowersAction, ZPowersActionRequest, ZPowersBridge,
    ZPowersReviewStage, ZPowersReviewVerdict, ZenkoError,
};

pub struct ZPowersTool {
    bridge: Arc<dyn ZPowersBridge>,
}

impl ZPowersTool {
    pub fn new(bridge: Arc<dyn ZPowersBridge>) -> Self {
        Self { bridge }
    }
}

fn parse_string_list(args: &Value, key: &str) -> Vec<String> {
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

fn parse_opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

#[async_trait]
impl Tool for ZPowersTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::Orchestration
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Global
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .and_then(ZPowersAction::parse);
        match action {
            Some(ZPowersAction::Status) => ToolConcurrency::Shared,
            _ => ToolConcurrency::Global,
        }
    }

    fn spec(&self) -> ToolSpec {
        let actions: Vec<Value> = ZPOWERS_ACTIONS
            .iter()
            .map(|&s| Value::String(s.to_string()))
            .collect();
        let review_stages: Vec<Value> = ZPOWERS_REVIEW_STAGES
            .iter()
            .map(|&s| Value::String(s.to_string()))
            .collect();
        let review_verdicts: Vec<Value> = ZPOWERS_REVIEW_VERDICTS
            .iter()
            .map(|&s| Value::String(s.to_string()))
            .collect();
        ToolSpec {
            name: TOOL_ZPOWERS.into(),
            description: format!(
                "Manage Zenko's native ZPowers plan-first session workspace (`.zenko/sessions/<id>/zpowers/`): inspect status, write `spec.md`/`plan.md`/`walkthrough.md`, log systematic debugging strikes in `debug.md`, record two-stage (`spec` then `quality`) reviews under `reviews/`, declare an auditable `fast_track` for trivial 1-line edits, or `promote` a doc into tracked `{}` folders.",
                ZPOWERS_PROMOTE_ALLOWED_ROOTS.join("`/`")
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": actions,
                        "description": "ZPowers action to execute."
                    },
                    "title": {
                        "type": "string",
                        "description": "Optional heading for `write_spec`, `write_plan`, or `write_walkthrough`."
                    },
                    "content": {
                        "type": "string",
                        "description": "Markdown body for `write_spec` or `write_plan` (also accepted as fallback summary for `write_walkthrough`)."
                    },
                    "hypothesis": {
                        "type": "string",
                        "description": "Single falsifiable root-cause hypothesis for `log_debug`."
                    },
                    "evidence": {
                        "type": "string",
                        "description": "Observed error, repro output, or inspection evidence for `log_debug`."
                    },
                    "outcome": {
                        "type": "string",
                        "description": "Experiment outcome or next adjustment for `log_debug`."
                    },
                    "stage": {
                        "type": "string",
                        "enum": review_stages,
                        "description": "Two-stage review phase for `record_review`: `spec` (compliance) or `quality` (architecture & minimalism)."
                    },
                    "verdict": {
                        "type": "string",
                        "enum": review_verdicts,
                        "description": "Review verdict for `record_review`."
                    },
                    "summary": {
                        "type": "string",
                        "description": "Concise summary for `record_review` or `write_walkthrough`."
                    },
                    "findings": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Actionable review findings for `record_review`."
                    },
                    "files_changed": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Changed file paths for `write_walkthrough`."
                    },
                    "verification": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Fresh post-edit build/test commands & results for `write_walkthrough`."
                    },
                    "next_steps": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional caveats or follow-ups for `write_walkthrough`."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Auditable 1-line rationale for `fast_track` bypassing the plan-first gate on tiny surgical edits."
                    },
                    "doc_name": {
                        "type": "string",
                        "description": "Source doc for `promote` (`spec.md`, `plan.md`, `debug.md`, `walkthrough.md`, or `reviews/<file>`)."
                    },
                    "dest_path": {
                        "type": "string",
                        "description": "Relative destination under `docs/` or `knowledge/` for `promote`."
                    },
                    "include_content": {
                        "type": "boolean",
                        "description": "When true on `status`, include full markdown bodies for existing ZPowers docs."
                    }
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
            .ok_or_else(|| ZenkoError::Tool("`action` is required for `zpowers`".into()))?;
        let action = ZPowersAction::parse(raw_action).ok_or_else(|| {
            ZenkoError::Tool(format!(
                "Invalid `zpowers` action `{raw_action}`. Expected one of: {}",
                ZPOWERS_ACTIONS.join(", ")
            ))
        })?;

        let stage = match parse_opt_str(&args, "stage") {
            Some(s) => Some(ZPowersReviewStage::parse(&s).ok_or_else(|| {
                ZenkoError::Tool(format!(
                    "Invalid `stage` `{s}`. Expected one of: {}",
                    ZPOWERS_REVIEW_STAGES.join(", ")
                ))
            })?),
            None => None,
        };
        let verdict = match parse_opt_str(&args, "verdict") {
            Some(v) => Some(ZPowersReviewVerdict::parse(&v).ok_or_else(|| {
                ZenkoError::Tool(format!(
                    "Invalid `verdict` `{v}`. Expected one of: {}",
                    ZPOWERS_REVIEW_VERDICTS.join(", ")
                ))
            })?),
            None => None,
        };

        let req = ZPowersActionRequest {
            action,
            title: parse_opt_str(&args, "title"),
            content: parse_opt_str(&args, "content"),
            hypothesis: parse_opt_str(&args, "hypothesis"),
            evidence: parse_opt_str(&args, "evidence"),
            outcome: parse_opt_str(&args, "outcome"),
            stage,
            verdict,
            summary: parse_opt_str(&args, "summary"),
            findings: parse_string_list(&args, "findings"),
            files_changed: parse_string_list(&args, "files_changed"),
            verification: parse_string_list(&args, "verification"),
            next_steps: parse_string_list(&args, "next_steps"),
            reason: parse_opt_str(&args, "reason"),
            doc_name: parse_opt_str(&args, "doc_name"),
            dest_path: parse_opt_str(&args, "dest_path"),
            include_content: args
                .get("include_content")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };

        self.bridge.execute_zpowers(ctx, req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tokio::sync::Notify;
    use zenko_core::{ZPowersMode, ZPowersSessionView, ZPowersState};

    struct StubBridge;

    #[async_trait]
    impl ZPowersBridge for StubBridge {
        async fn execute_zpowers(
            &self,
            _ctx: &ToolContext,
            req: ZPowersActionRequest,
        ) -> Result<ToolOutput> {
            Ok(ToolOutput::ok(format!("ok:{:?}", req.action)))
        }

        fn inspect_session_zpowers(
            &self,
            session_id: &str,
            _include_content: bool,
        ) -> Result<ZPowersSessionView> {
            Ok(ZPowersSessionView {
                session_id: session_id.into(),
                workspace_rel_dir: format!(".zenko/sessions/{session_id}/zpowers"),
                materialized: false,
                state: ZPowersState::new(session_id, ZPowersMode::Guided),
                docs: vec![],
            })
        }
    }

    #[tokio::test]
    async fn zpowers_tool_is_deferred_parses_actions_and_rejects_invalid_enums() {
        let tool = ZPowersTool::new(Arc::new(StubBridge));
        assert_eq!(tool.exposure(), ToolExposure::Deferred);
        assert_eq!(tool.toolset(), ToolSet::Orchestration);
        assert_eq!(
            tool.concurrency_for_args(&json!({"action": "status"})),
            ToolConcurrency::Shared
        );
        assert_eq!(
            tool.concurrency_for_args(&json!({"action": "write_plan"})),
            ToolConcurrency::Global
        );

        let ctx = ToolContext {
            session_id: "s1".into(),
            workspace_dir: PathBuf::from("/tmp"),
            data_dir: PathBuf::from("/tmp"),
            hub_dir: PathBuf::from("/tmp"),
            shell_timeout_secs: 10,
            auto_background_after_secs: 30,
            max_output_bytes: 4096,
            headless_no_wait: true,
            demote_bg: Arc::new(Notify::new()),
            file_state: None,
        };
        let ok = tool
            .execute(
                &ctx,
                json!({"action": "record_review", "stage": "spec", "verdict": "approved", "summary": "LGTM"}),
                CancellationToken::new(),
            )
            .await
            .expect("valid action");
        assert!(!ok.is_error);
        assert!(ok.content.contains("RecordReview"));

        let bad = tool
            .execute(&ctx, json!({"action": "nope"}), CancellationToken::new())
            .await;
        assert!(bad.is_err());
    }
}
