//! Phase 8 `goal_loop` Tool — create, inspect, pause, resume, trigger, or delete
//! bounded 24/7 autonomous verify-until-green Goal Loops (Ralph + Prime Verifier pattern).

use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    GOAL_LOOP_LIST_DEFAULT_LIMIT, GoalLoopAction, GoalLoopBridge, GoalLoopCreateRequest,
    GoalLoopPatch, GoalLoopStatus, ResourceLockKey, Result, RiskLevel, TOOL_GOAL_LOOP, Tool,
    ToolConcurrency, ToolContext, ToolExposure, ToolOutput, ToolSpec, ZenkoError,
};

pub struct GoalLoopTool {
    bridge: Arc<dyn GoalLoopBridge>,
}

impl GoalLoopTool {
    pub fn new(bridge: Arc<dyn GoalLoopBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for GoalLoopTool {
    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .and_then(GoalLoopAction::parse);
        match action {
            Some(a) if !a.is_mutating() => ToolConcurrency::Shared,
            _ => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("new")
                    .trim()
                    .to_string();
                ToolConcurrency::Keyed(ResourceLockKey::Goal(id))
            }
        }
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_GOAL_LOOP.into(),
            description: "Manage bounded 24/7 autonomous verify-until-green Goal Loops (`list`, `create`, `status`, `pause`, `resume`, `run_now`, `delete`). Each iteration runs a Shadow-Git checkpoint, executes an agent turn, and checks a deterministic `verify_cmd` argv until exit 0, budget cap, max iterations, or stall breaker hits.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "create", "status", "pause", "resume", "run_now", "delete"],
                        "description": "Goal loop operation"
                    },
                    "id": { "type": "string", "description": "Goal loop ID (required for status, pause, resume, run_now, delete)" },
                    "name": { "type": "string", "description": "Short goal loop title (for `create`)" },
                    "goal_text": { "type": "string", "description": "Objective instructions executed each iteration (for `create`)" },
                    "verify_cmd": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Deterministic argv command that exits 0 when the goal succeeds (e.g. [\"cargo\", \"test\", \"--workspace\"])"
                    },
                    "interval_secs": { "type": "integer", "description": "Seconds between retries (0 = immediate backoff loop)" },
                    "max_iterations": { "type": "integer", "description": "Hard iteration ceiling (default 10, cap 100)" },
                    "max_budget_usd": { "type": "number", "description": "Hard USD spend ceiling (default 2.0)" },
                    "max_consecutive_stalls": { "type": "integer", "description": "Consecutive identical verification failures before stalling (default 3)" },
                    "checkpoint_each_iter": { "type": "boolean", "description": "Capture a Shadow-Git/Blob workspace checkpoint before each iteration (default true)" },
                    "auto_rewind_on_stall": { "type": "boolean", "description": "Automatically rewind workspace to last clean checkpoint when stalled (default false)" },
                    "model": { "type": "string", "description": "Optional model ID override" }
                },
                "required": ["action"]
            }),
            risk: RiskLevel::Moderate,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let action_raw = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `action` for `goal_loop`".into()))?;
        let action = GoalLoopAction::parse(action_raw).ok_or_else(|| {
            ZenkoError::Tool(format!(
                "unknown `goal_loop` action `{action_raw}` (use list, create, status, pause, resume, run_now, delete)"
            ))
        })?;

        match action {
            GoalLoopAction::List => {
                let goals = self.bridge.list_goals(GOAL_LOOP_LIST_DEFAULT_LIMIT)?;
                if goals.is_empty() {
                    return Ok(ToolOutput::ok("No autonomous goal loops configured."));
                }
                let lines = goals
                    .iter()
                    .map(|g| {
                        format!(
                            "- `{}` **{}** [{}] iter={}/{} cost=${:.4}/${:.2} verify=`{}`",
                            g.id,
                            g.name,
                            g.status.as_str(),
                            g.iterations_done,
                            g.max_iterations,
                            g.cost_spent_usd,
                            g.max_budget_usd,
                            g.verify_cmd.join(" ")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(ToolOutput {
                    content: lines,
                    is_error: false,
                    diff: None,
                    metadata: Some(json!({ "goals": goals })),
                })
            }
            GoalLoopAction::Create => {
                let name = args
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| ZenkoError::Tool("missing `name` for `goal_loop(create)`".into()))?;
                let goal_text = args
                    .get("goal_text")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        ZenkoError::Tool("missing `goal_text` for `goal_loop(create)`".into())
                    })?;
                let verify_cmd: Vec<String> = args
                    .get("verify_cmd")
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
                if verify_cmd.is_empty() {
                    return Err(ZenkoError::Tool(
                        "`verify_cmd` must be a non-empty argv array (e.g. [\"cargo\", \"test\"])"
                            .into(),
                    ));
                }
                let req = GoalLoopCreateRequest {
                    name: name.to_string(),
                    goal_text: goal_text.to_string(),
                    verify_cmd,
                    workspace_dir: Some(ctx.workspace_dir.display().to_string()),
                    session_id: None,
                    interval_secs: args.get("interval_secs").and_then(Value::as_u64),
                    max_iterations: args
                        .get("max_iterations")
                        .and_then(Value::as_u64)
                        .map(|n| n as usize),
                    max_budget_usd: args.get("max_budget_usd").and_then(Value::as_f64),
                    max_consecutive_stalls: args
                        .get("max_consecutive_stalls")
                        .and_then(Value::as_u64)
                        .map(|n| n as usize),
                    checkpoint_each_iter: args.get("checkpoint_each_iter").and_then(Value::as_bool),
                    auto_rewind_on_stall: args.get("auto_rewind_on_stall").and_then(Value::as_bool),
                    model: args.get("model").and_then(Value::as_str).map(String::from),
                    notify_channel: None,
                };
                let created = self.bridge.create_goal(req)?;
                Ok(ToolOutput {
                    content: format!(
                        "Created autonomous Goal Loop `{}` (`{}`) with verify `{}` (max_iter={}, budget=${:.2}).",
                        created.id,
                        created.name,
                        created.verify_cmd.join(" "),
                        created.max_iterations,
                        created.max_budget_usd
                    ),
                    is_error: false,
                    diff: None,
                    metadata: Some(json!({ "goal": created })),
                })
            }
            GoalLoopAction::Status => {
                let id = req_goal_id(&args)?;
                let Some((goal, runs)) = self.bridge.get_goal(id)? else {
                    return Err(ZenkoError::NotFound(format!("goal loop `{id}` not found")));
                };
                let last_exit = goal
                    .last_verify_exit
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "none".into());
                let summary = format!(
                    "Goal Loop `{}` — {} [{}]\n\
                     - Session: `{}` · Workspace: `{}`\n\
                     - Progress: iter {}/{} · stalls {}/{} · cost ${:.4}/${:.2}\n\
                     - Verify: `{}` (last_exit={last_exit})\n\
                     - Runs recorded: {}",
                    goal.id,
                    goal.name,
                    goal.status.as_str(),
                    goal.session_id,
                    goal.workspace_dir,
                    goal.iterations_done,
                    goal.max_iterations,
                    goal.consecutive_stalls,
                    goal.max_consecutive_stalls,
                    goal.cost_spent_usd,
                    goal.max_budget_usd,
                    goal.verify_cmd.join(" "),
                    runs.len()
                );
                Ok(ToolOutput {
                    content: summary,
                    is_error: false,
                    diff: None,
                    metadata: Some(json!({ "goal": goal, "runs": runs })),
                })
            }
            GoalLoopAction::Pause => {
                let id = req_goal_id(&args)?;
                let updated = self
                    .bridge
                    .update_goal(
                        id,
                        GoalLoopPatch {
                            status: Some(GoalLoopStatus::Paused),
                            ..GoalLoopPatch::default()
                        },
                    )?
                    .ok_or_else(|| ZenkoError::NotFound(format!("goal loop `{id}` not found")))?;
                Ok(ToolOutput {
                    content: format!("Paused Goal Loop `{}`.", updated.id),
                    is_error: false,
                    diff: None,
                    metadata: Some(json!({ "goal": updated })),
                })
            }
            GoalLoopAction::Resume => {
                let id = req_goal_id(&args)?;
                let updated = self
                    .bridge
                    .update_goal(
                        id,
                        GoalLoopPatch {
                            status: Some(GoalLoopStatus::Active),
                            reset_stalls: Some(true),
                            ..GoalLoopPatch::default()
                        },
                    )?
                    .ok_or_else(|| ZenkoError::NotFound(format!("goal loop `{id}` not found")))?;
                Ok(ToolOutput {
                    content: format!("Resumed Goal Loop `{}` (status=active).", updated.id),
                    is_error: false,
                    diff: None,
                    metadata: Some(json!({ "goal": updated })),
                })
            }
            GoalLoopAction::RunNow => {
                let id = req_goal_id(&args)?;
                let updated = self.bridge.trigger_goal_now(id).await?;
                Ok(ToolOutput {
                    content: format!(
                        "Ran Goal Loop `{}` iteration {}: status=`{}`, last_verify_exit={:?}.",
                        updated.id, updated.iterations_done, updated.status.as_str(), updated.last_verify_exit
                    ),
                    is_error: false,
                    diff: None,
                    metadata: Some(json!({ "goal": updated })),
                })
            }
            GoalLoopAction::Delete => {
                let id = req_goal_id(&args)?;
                let deleted = self.bridge.delete_goal(id)?;
                Ok(ToolOutput::ok(match deleted {
                    true => format!("Deleted Goal Loop `{id}`."),
                    false => format!("Goal Loop `{id}` not found."),
                }))
            }
        }
    }
}

fn req_goal_id(args: &Value) -> Result<&str> {
    args.get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ZenkoError::Tool("missing `id` for `goal_loop`".into()))
}
