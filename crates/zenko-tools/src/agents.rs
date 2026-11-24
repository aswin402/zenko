//! Phase 5 first-class async subagent orchestration tools (`spawn_agent`, `agent_list`,
//! `agent_message`, `agent_wait`, `agent_kill`) — zero `/commands` dependency, declarative roles.

use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    AgentDeliveryAction, AgentMessageOptions, MODEL_ROUTING_ROLES, ModelRoutingRole,
    ResourceLockKey, Result, RiskLevel, SUBAGENT_DEFAULT_WAIT_SECS, SUBAGENT_LIST_DEFAULT_LIMIT,
    SUBAGENT_MAX_WAIT_SECS, SUBAGENT_ROLE_REGISTRY, SubagentBridge, SubagentSpawnRequest,
    SubagentWorkspaceMode, TOOL_AGENT_KILL, TOOL_AGENT_LIST, TOOL_AGENT_MESSAGE, TOOL_AGENT_WAIT,
    TOOL_AGENT_WORKTREE, TOOL_SPAWN_AGENT, Tool, ToolConcurrency, ToolContext, ToolExposure,
    ToolOutput, ToolSpec, WORKTREE_ACTIONS, WorktreeAction, ZenkoError,
};

pub struct SpawnAgentTool {
    bridge: Arc<dyn SubagentBridge>,
}

impl SpawnAgentTool {
    pub fn new(bridge: Arc<dyn SubagentBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for SpawnAgentTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Subagents
    }

    fn spec(&self) -> ToolSpec {
        let role_enum: Vec<Value> = SUBAGENT_ROLE_REGISTRY
            .iter()
            .map(|r| Value::String(r.id.to_string()))
            .collect();
        let role_help: Vec<String> = SUBAGENT_ROLE_REGISTRY
            .iter()
            .map(|r| format!("`{}` ({})", r.id, r.description))
            .collect();
        let tier_enum: Vec<Value> = MODEL_ROUTING_ROLES
            .iter()
            .map(|&r| Value::String(r.to_string()))
            .collect();
        ToolSpec {
            name: TOOL_SPAWN_AGENT.into(),
            description: format!(
                "Delegate a focused subtask to an isolated child subagent with its own clean context window. Roles: {}. \
                 Set `run_in_background: true` to fan out parallel workers and collect results via `agent_wait`. \
                 Use `role: \"custom\"` (with `custom_directive`, `cognitive_tier`, `read_only`, and optional `tool_subset`) to compose bespoke specialists dynamically; `coder` defaults to an isolated git worktree.",
                role_help.join("; ")
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "role": {
                        "type": "string",
                        "enum": role_enum,
                        "description": "Declarative role profile (`explorer`, `architect`, `coder`, `verifier`, `critic`, `researcher`, or dynamic `custom`)."
                    },
                    "goal": {
                        "type": "string",
                        "description": "Self-contained, concrete objective for the child agent."
                    },
                    "context": {
                        "type": "string",
                        "description": "Optional focused handoff notes (exact file paths, constraints, prior findings)."
                    },
                    "custom_directive": {
                        "type": "string",
                        "description": "Optional specialist persona/instructions authored by you (used with `role: \"custom\"` or to tailor any role)."
                    },
                    "cognitive_tier": {
                        "type": "string",
                        "enum": tier_enum,
                        "description": "Optional category/intent model slot override (`fast`, `lead`, `plan`, `advisor`) without hardcoding model IDs."
                    },
                    "read_only": {
                        "type": "boolean",
                        "description": "When true, strictly clamps the child specialist to read-only tools."
                    },
                    "tool_subset": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional narrowed allowlist of tool names for surgical specialist focus."
                    },
                    "run_in_background": {
                        "type": "boolean",
                        "description": "When true, returns immediately with the child agent ID so you can fan out parallel workers."
                    },
                    "workspace": {
                        "type": "string",
                        "enum": ["inherit", "worktree"],
                        "description": "Workspace isolation mode (`worktree` creates an isolated git worktree branch)."
                    },
                    "output_schema": {
                        "type": "object",
                        "description": "Optional JSON Schema enforcing structured JSON output from the child."
                    },
                    "model": {
                        "type": "string",
                        "description": "Optional explicit model ID override (normally routed dynamically by role or `cognitive_tier`)."
                    },
                    "fork_context": {
                        "type": "boolean",
                        "description": "When true, injects a bounded digest of the parent's active branch into the child (subject to `[subagents].allow_fork_context`)."
                    },
                    "include_uncommitted": {
                        "type": "boolean",
                        "description": "When workspace is `worktree`, snapshots uncommitted parent changes non-invasively into the worktree."
                    },
                    "max_duration_secs": {
                        "type": "integer",
                        "description": "Optional wall-clock timeout ceiling in seconds (clamped to `[subagents].max_duration_secs`)."
                    },
                    "max_cost_usd": {
                        "type": "number",
                        "description": "Optional USD spend ceiling for this subagent (clamped to `[subagents].max_cost_usd`)."
                    }
                },
                "required": ["role", "goal"]
            }),
            risk: RiskLevel::Safe,
        }
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Eager
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let role = args
            .get("role")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `role`".into()))?
            .to_string();
        let goal = args
            .get("goal")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `goal`".into()))?
            .to_string();
        let context = args
            .get("context")
            .and_then(Value::as_str)
            .map(String::from);
        let run_in_background = args
            .get("run_in_background")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let workspace_mode = args
            .get("workspace")
            .and_then(Value::as_str)
            .map(SubagentWorkspaceMode::from_str_loose);
        let output_schema = args.get("output_schema").filter(|v| v.is_object()).cloned();
        let model_override = args.get("model").and_then(Value::as_str).map(String::from);
        let fork_context = args
            .get("fork_context")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let include_uncommitted = args
            .get("include_uncommitted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let max_duration_secs = args.get("max_duration_secs").and_then(Value::as_u64);
        let max_cost_usd = args.get("max_cost_usd").and_then(Value::as_f64);
        let custom_directive = args
            .get("custom_directive")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let cognitive_tier = args
            .get("cognitive_tier")
            .and_then(Value::as_str)
            .and_then(ModelRoutingRole::parse);
        let read_only = args.get("read_only").and_then(Value::as_bool);
        let tool_subset: Vec<String> = args
            .get("tool_subset")
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

        let rec = self
            .bridge
            .spawn_agent(SubagentSpawnRequest {
                parent_session_id: ctx.session_id.clone(),
                role,
                goal,
                context,
                run_in_background,
                workspace_mode,
                output_schema,
                model_override,
                fork_context,
                include_uncommitted,
                max_duration_secs,
                max_cost_usd,
                custom_directive,
                cognitive_tier,
                read_only,
                tool_subset,
            })
            .await?;

        let content = match run_in_background {
            true => format!(
                "Spawned background `{}` subagent `{}` (workspace={}, branch={}). Use `agent_wait` or `agent_message` to coordinate.",
                rec.role,
                rec.id,
                rec.workspace_mode.as_str(),
                rec.worktree_branch.as_deref().unwrap_or("inherit")
            ),
            false => {
                let mut lines = vec![format!(
                    "Subagent `{}` (role={}, status={}, tokens={}, cost=${:.4})",
                    rec.id,
                    rec.role,
                    rec.status.as_str(),
                    rec.tokens_used,
                    rec.cost_usd
                )];
                if let Some(ref br) = rec.worktree_branch {
                    lines.push(format!(
                        "Worktree branch: `{br}` ({} changed file(s))",
                        rec.changed_files.len()
                    ));
                }
                if let Some(ref stat) = rec.diff_stat {
                    lines.push(format!("Diff stat:\n{stat}"));
                }
                if let Some(ref structured) = rec.structured_output {
                    lines.push(format!(
                        "Structured JSON:\n{}",
                        serde_json::to_string_pretty(structured).unwrap_or_default()
                    ));
                } else if let Some(ref sum) = rec.summary {
                    lines.push(format!("Summary:\n{sum}"));
                }
                lines.join("\n")
            }
        };

        let mut out = ToolOutput::ok(content);
        out.metadata = serde_json::to_value(&rec).ok();
        Ok(out)
    }
}

pub struct AgentListTool {
    bridge: Arc<dyn SubagentBridge>,
}

impl AgentListTool {
    pub fn new(bridge: Arc<dyn SubagentBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for AgentListTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Subagents
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_AGENT_LIST.into(),
            description: "List subagents spawned by the current session (or all subagents) with status, role, tokens, and git worktree branches.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "all": { "type": "boolean", "description": "When true, lists subagents across all sessions." },
                    "limit": { "type": "integer", "description": "Max records to return." }
                }
            }),
            risk: RiskLevel::Safe,
        }
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(SUBAGENT_LIST_DEFAULT_LIMIT as u64) as usize;
        let parent_filter = match all {
            true => None,
            false => Some(ctx.session_id.as_str()),
        };
        let list = self.bridge.list_agents(parent_filter, limit)?;
        let content = match list.is_empty() {
            true => "No subagents found.".to_string(),
            false => list
                .iter()
                .map(|r| {
                    format!(
                        "- {} [{}] status={} depth={} ws={} branch={} tokens={}",
                        r.id,
                        r.role,
                        r.status.as_str(),
                        r.depth,
                        r.workspace_mode.as_str(),
                        r.worktree_branch.as_deref().unwrap_or("-"),
                        r.tokens_used
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        };
        let mut out = ToolOutput::ok(content);
        out.metadata = serde_json::to_value(&list).ok();
        Ok(out)
    }
}

pub struct AgentMessageTool {
    bridge: Arc<dyn SubagentBridge>,
}

impl AgentMessageTool {
    pub fn new(bridge: Arc<dyn SubagentBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for AgentMessageTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Subagents
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_AGENT_MESSAGE.into(),
            description: "Send a targeted mailbox message to a direct child, direct parent, or same-parent sibling agent (nuclear-family isolated). Live-steers active children, optionally interrupts & restarts them (`interrupt: true`), or automatically resumes idle/completed children in their existing SQLite history & git worktree (`resume: true`, default).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "to": { "type": "string", "description": "Target agent/session ID." },
                    "message": { "type": "string", "description": "Concise instruction, steering update, or follow-up task." },
                    "resume": { "type": "boolean", "description": "When true (default), messaging a completed/failed child resumes its session & worktree." },
                    "interrupt": { "type": "boolean", "description": "When true and target child is running, interrupts its current step and restarts with this message." }
                },
                "required": ["to", "message"]
            }),
            risk: RiskLevel::Safe,
        }
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        let to = args.get("to").and_then(Value::as_str).unwrap_or("default");
        ToolConcurrency::Keyed(ResourceLockKey::Agent(to.to_string()))
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let to = args
            .get("to")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `to`".into()))?;
        let message = args
            .get("message")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `message`".into()))?;
        let resume = args.get("resume").and_then(Value::as_bool).unwrap_or(true);
        let interrupt = args
            .get("interrupt")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mail = self
            .bridge
            .send_message(
                &ctx.session_id,
                to,
                message,
                AgentMessageOptions { resume, interrupt },
            )
            .await?;
        let action = mail
            .delivery_action
            .map(AgentDeliveryAction::as_str)
            .unwrap_or("queued");
        let mut out = ToolOutput::ok(format!(
            "Delivered message `{}` to `{}` (action={action}).",
            mail.id, mail.to_session_id
        ));
        out.metadata = serde_json::to_value(&mail).ok();
        Ok(out)
    }
}

pub struct AgentWaitTool {
    bridge: Arc<dyn SubagentBridge>,
}

impl AgentWaitTool {
    pub fn new(bridge: Arc<dyn SubagentBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for AgentWaitTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Subagents
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_AGENT_WAIT.into(),
            description: "Event-driven zero-CPU await for one or more background subagents to complete and return their distilled summaries and diff stats.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "agent_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Specific subagent IDs to await (empty = all running children of this session)."
                    },
                    "wait_all": {
                        "type": "boolean",
                        "description": "When true (default), waits for all listed agents; when false, returns as soon as any finishes."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Maximum seconds to wait."
                    }
                }
            }),
            risk: RiskLevel::Safe,
        }
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let agent_ids: Vec<String> = args
            .get("agent_ids")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        let wait_all = args
            .get("wait_all")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(SUBAGENT_DEFAULT_WAIT_SECS)
            .clamp(1, SUBAGENT_MAX_WAIT_SECS);

        let records = self
            .bridge
            .wait_agents(
                &ctx.session_id,
                &agent_ids,
                wait_all,
                Duration::from_secs(timeout_secs),
            )
            .await?;

        let content = match records.is_empty() {
            true => "No matching subagents to await.".to_string(),
            false => records
                .iter()
                .map(|r| {
                    let body = match (&r.structured_output, &r.summary) {
                        (Some(v), _) => serde_json::to_string_pretty(v).unwrap_or_default(),
                        (None, Some(s)) => s.clone(),
                        (None, None) => "(still running)".to_string(),
                    };
                    let wt_info = match &r.worktree_branch {
                        Some(br) => format!(" branch={br} changed={}", r.changed_files.len()),
                        None => String::new(),
                    };
                    format!(
                        "### {} [{}] status={}{wt_info}\n{}",
                        r.id,
                        r.role,
                        r.status.as_str(),
                        body
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
        };

        let mut out = ToolOutput::ok(content);
        out.metadata = serde_json::to_value(&records).ok();
        Ok(out)
    }
}

pub struct AgentKillTool {
    bridge: Arc<dyn SubagentBridge>,
}

impl AgentKillTool {
    pub fn new(bridge: Arc<dyn SubagentBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for AgentKillTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Subagents
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_AGENT_KILL.into(),
            description: "Cascade-cancel a subagent and its descendants, terminate child background shell jobs, and optionally remove its git worktree.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Subagent ID to terminate." },
                    "remove_worktree": { "type": "boolean", "description": "Remove the child's temporary git worktree & branch (default true)." }
                },
                "required": ["agent_id"]
            }),
            risk: RiskLevel::Safe,
        }
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        let id = args
            .get("agent_id")
            .and_then(Value::as_str)
            .unwrap_or("default");
        ToolConcurrency::Keyed(ResourceLockKey::Agent(id.to_string()))
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let agent_id = args
            .get("agent_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `agent_id`".into()))?;
        let remove_wt = args
            .get("remove_worktree")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        match self
            .bridge
            .kill_agent(&ctx.session_id, agent_id, remove_wt)
            .await?
        {
            Some(rec) => {
                let mut out = ToolOutput::ok(format!(
                    "Terminated subagent `{}` (status={}, worktree_removed={remove_wt}).",
                    rec.id,
                    rec.status.as_str()
                ));
                out.metadata = serde_json::to_value(&rec).ok();
                Ok(out)
            }
            None => Ok(ToolOutput::err(format!("Subagent `{agent_id}` not found."))),
        }
    }
}

pub struct AgentWorktreeTool {
    bridge: Arc<dyn SubagentBridge>,
}

impl AgentWorktreeTool {
    pub fn new(bridge: Arc<dyn SubagentBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for AgentWorktreeTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Subagents
    }

    fn spec(&self) -> ToolSpec {
        let action_enum: Vec<Value> = WORKTREE_ACTIONS
            .iter()
            .map(|&a| Value::String(a.to_string()))
            .collect();
        ToolSpec {
            name: TOOL_AGENT_WORKTREE.into(),
            description: "Inspect (`diff`), merge (`merge`), discard (`discard`), or garbage-collect (`prune`) isolated git worktrees owned by subagents. Always inspect `diff` before calling `merge`.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": action_enum,
                        "description": "Worktree lifecycle action."
                    },
                    "agent_id": {
                        "type": "string",
                        "description": "Target subagent ID (required for `diff`, `merge`, `discard`)."
                    },
                    "cleanup": {
                        "type": "boolean",
                        "description": "For `merge`: remove the temporary worktree & branch after a clean merge (default true)."
                    }
                },
                "required": ["action"]
            }),
            risk: RiskLevel::Moderate,
        }
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        let parsed = args
            .get("action")
            .and_then(Value::as_str)
            .and_then(WorktreeAction::parse);
        match parsed {
            Some(WorktreeAction::Diff | WorktreeAction::Prune) => ToolConcurrency::Shared,
            _ => {
                let id = args
                    .get("agent_id")
                    .and_then(Value::as_str)
                    .unwrap_or("default");
                ToolConcurrency::Keyed(ResourceLockKey::Agent(id.to_string()))
            }
        }
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
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
        let action = WorktreeAction::parse(raw_action).ok_or_else(|| {
            ZenkoError::Tool(format!(
                "Unknown `agent_worktree` action `{raw_action}`. Valid actions: {}.",
                WORKTREE_ACTIONS.join(", ")
            ))
        })?;

        match action {
            WorktreeAction::Diff => {
                let agent_id = args
                    .get("agent_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`agent_id` is required for `diff`".into()))?;
                match self
                    .bridge
                    .inspect_worktree_diff(&ctx.session_id, agent_id)
                    .await?
                {
                    Some(rep) => {
                        let content = format!(
                            "Worktree `{}` (branch=`{}`, dirty_baseline={}, changes={}):\n{}\n\n{}",
                            rep.agent_id,
                            rep.branch,
                            rep.dirty_baseline,
                            rep.has_changes,
                            rep.diff_stat,
                            rep.patch_preview
                        );
                        let mut out = ToolOutput::ok(content);
                        out.metadata = serde_json::to_value(&rep).ok();
                        Ok(out)
                    }
                    None => Ok(ToolOutput::err(format!(
                        "No isolated git worktree found for subagent `{agent_id}`."
                    ))),
                }
            }
            WorktreeAction::Merge => {
                let agent_id = args
                    .get("agent_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`agent_id` is required for `merge`".into()))?;
                let cleanup = args.get("cleanup").and_then(Value::as_bool).unwrap_or(true);
                let rep = self
                    .bridge
                    .merge_worktree(&ctx.session_id, agent_id, cleanup)
                    .await?;
                let mut out = ToolOutput::ok(format!(
                    "Merged worktree branch `{}` (merged={}, cleaned_up={}, files={}).",
                    rep.branch,
                    rep.merged,
                    rep.cleaned_up,
                    rep.changed_files.len()
                ));
                out.metadata = serde_json::to_value(&rep).ok();
                Ok(out)
            }
            WorktreeAction::Discard => {
                let agent_id = args
                    .get("agent_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ZenkoError::Tool("`agent_id` is required for `discard`".into())
                    })?;
                let rep = self
                    .bridge
                    .discard_worktree(&ctx.session_id, agent_id)
                    .await?;
                let mut out = ToolOutput::ok(format!(
                    "Discarded worktree branch `{}` without merging (cleaned_up={}).",
                    rep.branch, rep.cleaned_up
                ));
                out.metadata = serde_json::to_value(&rep).ok();
                Ok(out)
            }
            WorktreeAction::Prune => {
                let rep = self.bridge.prune_orphan_worktrees().await?;
                let mut out = ToolOutput::ok(format!(
                    "Pruned {} orphaned worktree director(ies).",
                    rep.removed_count
                ));
                out.metadata = serde_json::to_value(&rep).ok();
                Ok(out)
            }
        }
    }
}
