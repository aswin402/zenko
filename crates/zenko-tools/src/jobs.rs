//! Async background job inspection & lifecycle tools (`job_list`, `job_read`, `job_wait`, `job_input`, `job_stop`).

use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    JOB_DEFAULT_LIST_LIMIT, JOB_DEFAULT_READ_BYTES, JOB_DEFAULT_WAIT_SECS, JOB_MAX_HISTORY,
    JOB_MAX_READ_BYTES, JOB_MAX_WAIT_SECS, JOB_MIN_READ_BYTES, JobSupervisor, ResourceLockKey,
    Result, RiskLevel, TOOL_JOB_INPUT, TOOL_JOB_LIST, TOOL_JOB_READ, TOOL_JOB_STOP, TOOL_JOB_WAIT,
    TOOL_TASK_MANAGE, TaskKind, Tool, ToolConcurrency, ToolContext, ToolExposure, ToolOutput,
    ToolSpec, ZenkoError,
};

fn job_lock_key(args: &Value) -> ToolConcurrency {
    match args.get("job_id").and_then(Value::as_str).map(str::trim) {
        Some(id) if !id.is_empty() => ToolConcurrency::Keyed(ResourceLockKey::Job(id.to_string())),
        _ => ToolConcurrency::Global,
    }
}

pub struct JobListTool {
    jobs: Arc<dyn JobSupervisor>,
}

impl JobListTool {
    pub fn new(jobs: Arc<dyn JobSupervisor>) -> Self {
        Self { jobs }
    }
}

#[async_trait]
impl Tool for JobListTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Execution
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_JOB_LIST.into(),
            description: "List running and recent asynchronous ZenkoTasks (filterable by category `kind` or `all_sessions`).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["all", "shell", "build", "dev", "debug", "script", "cron", "browser", "subagent"], "description": "Optional category filter" },
                    "all_sessions": { "type": "boolean", "description": "If true, list tasks across all sessions" },
                    "limit": { "type": "integer", "description": "Maximum tasks to return (default 25)" }
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
        let all_sessions = args
            .get("all_sessions")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let kind_filter = args
            .get("kind")
            .and_then(Value::as_str)
            .and_then(TaskKind::parse_filter);
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(JOB_DEFAULT_LIST_LIMIT as u64)
            .clamp(1, JOB_MAX_HISTORY as u64) as usize;
        let filter_sid = match all_sessions {
            true => None,
            false => Some(ctx.session_id.as_str()),
        };
        let list = self.jobs.list_jobs(filter_sid, kind_filter, limit)?;
        if list.is_empty() {
            return Ok(ToolOutput::ok("No background tasks found."));
        }
        let lines = list
            .iter()
            .map(|j| {
                let code = j
                    .exit_code
                    .map(|c| format!(" exit={c}"))
                    .unwrap_or_default();
                let pid = j.pid.map(|p| format!(" pid={p}")).unwrap_or_default();
                let lbl = j
                    .label
                    .as_deref()
                    .map(|l| format!(" ({l})"))
                    .unwrap_or_default();
                format!(
                    "- {} [{}][{}]{}{}{} ({} B)  `{}`",
                    j.id,
                    j.kind.as_str(),
                    j.status.as_str(),
                    lbl,
                    pid,
                    code,
                    j.bytes_written,
                    j.command
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(ToolOutput {
            content: lines,
            is_error: false,
            diff: None,
            metadata: Some(serde_json::to_value(&list).unwrap_or(Value::Null)),
        })
    }
}

pub struct JobReadTool {
    jobs: Arc<dyn JobSupervisor>,
}

impl JobReadTool {
    pub fn new(jobs: Arc<dyn JobSupervisor>) -> Self {
        Self { jobs }
    }
}

#[async_trait]
impl Tool for JobReadTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Execution
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_JOB_READ.into(),
            description: "Read streamed stdout/stderr output from a background job log file by byte offset (defaults to latest tail).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string", "description": "Background job ID" },
                    "offset": { "type": "integer", "description": "Optional byte offset to resume from (omit for latest tail)" },
                    "max_bytes": { "type": "integer", "description": "Max bytes to read (default 8192)" }
                },
                "required": ["job_id"]
            }),
            risk: RiskLevel::Safe,
        }
    }

    async fn execute(
        &self,
        _ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let job_id = args
            .get("job_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `job_id`".into()))?;
        let offset = args.get("offset").and_then(Value::as_u64);
        let max_bytes = args
            .get("max_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(JOB_DEFAULT_READ_BYTES as u64)
            .clamp(JOB_MIN_READ_BYTES as u64, JOB_MAX_READ_BYTES as u64) as usize;

        let chunk = self
            .jobs
            .read_job_output(job_id, offset, max_bytes)
            .await?;
        let exit_str = chunk
            .job
            .exit_code
            .map(|c| format!(" exit={c}"))
            .unwrap_or_default();
        let header = format!(
            "Job `{}` [{}]{exit_str} · bytes {}..{} of {}\n",
            chunk.job.id,
            chunk.job.status.as_str(),
            chunk.offset,
            chunk.next_offset,
            chunk.total_bytes
        );
        Ok(ToolOutput {
            content: format!("{header}{}", chunk.content),
            is_error: false,
            diff: None,
            metadata: Some(serde_json::to_value(&chunk).unwrap_or(Value::Null)),
        })
    }
}

pub struct JobWaitTool {
    jobs: Arc<dyn JobSupervisor>,
}

impl JobWaitTool {
    pub fn new(jobs: Arc<dyn JobSupervisor>) -> Self {
        Self { jobs }
    }
}

#[async_trait]
impl Tool for JobWaitTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Execution
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_JOB_WAIT.into(),
            description: "Zero-CPU event-driven wait for a background job to finish (or return its latest status & output tail after `timeout_secs`). Never poll with `sleep`.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string", "description": "Background job ID to await" },
                    "timeout_secs": { "type": "integer", "description": "Max seconds to wait before returning current progress (default 30, max 120)" },
                    "tail_bytes": { "type": "integer", "description": "Optional output tail bytes to return (default 8192)" }
                },
                "required": ["job_id"]
            }),
            risk: RiskLevel::Safe,
        }
    }

    async fn execute(
        &self,
        _ctx: &ToolContext,
        args: Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let job_id = args
            .get("job_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `job_id`".into()))?;
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(JOB_DEFAULT_WAIT_SECS)
            .clamp(1, JOB_MAX_WAIT_SECS);
        let tail_bytes = args
            .get("tail_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(JOB_DEFAULT_READ_BYTES as u64)
            .clamp(JOB_MIN_READ_BYTES as u64, JOB_MAX_READ_BYTES as u64) as usize;

        let waited = tokio::select! {
            _ = cancel.cancelled() => return Err(ZenkoError::Cancelled),
            res = self.jobs.wait_job(job_id, Duration::from_secs(timeout_secs)) => res?,
        };
        let Some(rec) = waited else {
            return Ok(ToolOutput::err(format!("Job `{job_id}` not found.")));
        };
        let chunk = self
            .jobs
            .read_job_output(&rec.id, None, tail_bytes)
            .await?;
        let exit_str = rec
            .exit_code
            .map(|c| format!(" exit={c}"))
            .unwrap_or_default();
        let state_note = match rec.status.is_terminal() {
            true => "finished",
            false => "still running after wait window",
        };
        let header = format!(
            "Job `{}` [{}]{exit_str} ({state_note}) · tail bytes {}..{} of {}\n",
            rec.id,
            rec.status.as_str(),
            chunk.offset,
            chunk.next_offset,
            chunk.total_bytes
        );
        Ok(ToolOutput {
            content: format!("{header}{}", chunk.content),
            is_error: false,
            diff: None,
            metadata: Some(serde_json::to_value(&chunk).unwrap_or(Value::Null)),
        })
    }
}

pub struct JobInputTool {
    jobs: Arc<dyn JobSupervisor>,
}

impl JobInputTool {
    pub fn new(jobs: Arc<dyn JobSupervisor>) -> Self {
        Self { jobs }
    }
}

#[async_trait]
impl Tool for JobInputTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Execution
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        job_lock_key(args)
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_JOB_INPUT.into(),
            description: "Send stdin text to a running background job (e.g. answering prompts or interactive commands).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string", "description": "Running background job ID" },
                    "input": { "type": "string", "description": "Text to write to stdin (include '\\n' if Enter is needed)" }
                },
                "required": ["job_id", "input"]
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
        let job_id = args
            .get("job_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `job_id`".into()))?;
        let input = args
            .get("input")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `input`".into()))?;

        self.jobs.send_input(job_id, input).await?;
        Ok(ToolOutput::ok(format!(
            "Delivered {} bytes to stdin of job `{job_id}`.",
            input.len()
        )))
    }
}

pub struct JobStopTool {
    jobs: Arc<dyn JobSupervisor>,
}

impl JobStopTool {
    pub fn new(jobs: Arc<dyn JobSupervisor>) -> Self {
        Self { jobs }
    }
}

#[async_trait]
impl Tool for JobStopTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Execution
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        job_lock_key(args)
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_JOB_STOP.into(),
            description: "Gracefully stop a running background job and its entire child process group.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string", "description": "Background job ID to terminate" }
                },
                "required": ["job_id"]
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
        let job_id = args
            .get("job_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `job_id`".into()))?;
        match self.jobs.stop_job(job_id).await? {
            Some(rec) => Ok(ToolOutput {
                content: format!(
                    "Stopped job `{}` (status={}, bytes={}).",
                    rec.id,
                    rec.status.as_str(),
                    rec.bytes_written
                ),
                is_error: false,
                diff: None,
                metadata: Some(serde_json::to_value(&rec).unwrap_or(Value::Null)),
            }),
            None => Ok(ToolOutput::err(format!("Job `{job_id}` not found."))),
        }
    }
}

/// Full CRUD lifecycle management tool for ZenkoTasks (`update`, `restart`, `delete`, `prune`).
pub struct TaskManageTool {
    jobs: Arc<dyn JobSupervisor>,
}

impl TaskManageTool {
    pub fn new(jobs: Arc<dyn JobSupervisor>) -> Self {
        Self { jobs }
    }
}

#[async_trait]
impl Tool for TaskManageTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Execution
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        match args.get("action").and_then(Value::as_str) {
            Some("update" | "restart" | "delete") => job_lock_key(args),
            _ => ToolConcurrency::Global,
        }
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_TASK_MANAGE.into(),
            description: "Full CRUD management for ZenkoTasks: update task label/kind, restart a previous task, delete a task + its log file, or prune finished tasks by category.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["update", "restart", "delete", "prune"] },
                    "job_id": { "type": "string", "description": "Target task ID (required for update, restart, delete)" },
                    "label": { "type": "string", "description": "Optional updated label (for update)" },
                    "kind": { "type": "string", "enum": ["all", "shell", "build", "dev", "debug", "script", "cron", "browser", "subagent"], "description": "Category for update or prune filter" }
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
            "update" => {
                let job_id = args
                    .get("job_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("missing `job_id`".into()))?;
                let label = args.get("label").and_then(Value::as_str).map(String::from);
                let kind = args
                    .get("kind")
                    .and_then(Value::as_str)
                    .and_then(TaskKind::parse_filter);
                match self.jobs.update_job_meta(job_id, label, kind)? {
                    Some(rec) => Ok(ToolOutput {
                        content: format!(
                            "Updated task `{}` (kind={}, label={:?}).",
                            rec.id,
                            rec.kind.as_str(),
                            rec.label
                        ),
                        is_error: false,
                        diff: None,
                        metadata: Some(serde_json::to_value(&rec).unwrap_or(Value::Null)),
                    }),
                    None => Ok(ToolOutput::err(format!("Task `{job_id}` not found."))),
                }
            }
            "restart" => {
                let job_id = args
                    .get("job_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("missing `job_id`".into()))?;
                let spawned = self.jobs.restart_job(job_id).await?;
                Ok(ToolOutput {
                    content: format!(
                        "Restarted task `{job_id}` as `{}` [{}] (pid={:?}). Log: {}",
                        spawned.id,
                        spawned.kind.as_str(),
                        spawned.pid,
                        spawned.log_path
                    ),
                    is_error: false,
                    diff: None,
                    metadata: Some(serde_json::to_value(&spawned).unwrap_or(Value::Null)),
                })
            }
            "delete" => {
                let job_id = args
                    .get("job_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("missing `job_id`".into()))?;
                let deleted = self.jobs.delete_job(job_id).await?;
                Ok(match deleted {
                    true => ToolOutput::ok(format!("Deleted task `{job_id}` and its log file.")),
                    false => ToolOutput::err(format!("Task `{job_id}` not found.")),
                })
            }
            "prune" => {
                let kind = args
                    .get("kind")
                    .and_then(Value::as_str)
                    .and_then(TaskKind::parse_filter);
                let count = self.jobs.prune_jobs(kind).await?;
                Ok(ToolOutput::ok(format!(
                    "Pruned {count} finished task(s) and cleaned up their logs."
                )))
            }
            other => Ok(ToolOutput::err(format!("Unknown task_manage action: {other}"))),
        }
    }
}
