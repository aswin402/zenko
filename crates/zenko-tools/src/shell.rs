use crate::jail::resolve_workspace_or_hub_path;
use crate::pty::PtySessionManager;
use crate::sandbox::{ShellSandboxPolicy, install_sandbox_on_command};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    ARTIFACT_HEAD_RATIO_DEN, ARTIFACT_HEAD_RATIO_NUM, BackgroundJobRecord, BackgroundJobStatus,
    JOB_MAX_AUTO_BACKGROUND_SECS, JobSupervisor, PTY_ACTIONS, PTY_SEND_DEFAULT_WAIT_MS,
    PTY_SEND_MAX_WAIT_MS, PtyAction, Result, RiskLevel, SAFE_SHELL_ENV_KEYS, SHELL_BIN_UNIX,
    SHELL_MAX_TIMEOUT_SECS, SHELL_MIN_OUTPUT_BYTES, TOOL_SHELL_EXEC, TaskKind, Tool, ToolContext,
    ToolOutput, ToolSet, ToolSpec, ZenkoError,
};

pub struct ShellExecTool {
    jobs: Option<Arc<dyn JobSupervisor>>,
    pty: Arc<PtySessionManager>,
    os_sandbox: Arc<AtomicBool>,
}

impl ShellExecTool {
    pub fn new(jobs: Option<Arc<dyn JobSupervisor>>) -> Self {
        Self {
            jobs,
            pty: PtySessionManager::new(),
            os_sandbox: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn new_with_runtime(
        jobs: Option<Arc<dyn JobSupervisor>>,
        pty: Arc<PtySessionManager>,
        os_sandbox: Arc<AtomicBool>,
    ) -> Self {
        Self {
            jobs,
            pty,
            os_sandbox,
        }
    }
}

async fn format_completed_job(
    supervisor: &Arc<dyn JobSupervisor>,
    rec: &BackgroundJobRecord,
    max_output_bytes: usize,
) -> Result<ToolOutput> {
    let limit = max_output_bytes.max(SHELL_MIN_OUTPUT_BYTES);
    let head_chunk = supervisor
        .read_job_output(&rec.id, Some(0), limit)
        .await?;
    let code = rec.exit_code.unwrap_or(-1);
    let body = match head_chunk.total_bytes > head_chunk.next_offset {
        false => format!("exit_code: {code}\n{}", head_chunk.content),
        true => {
            // Codex-style Head (35%) + Tail (65%) preservation so compiler errors & test summaries
            // at the bottom of long logs are never chopped off!
            let usable = limit.saturating_sub(180).max(256);
            let head_bytes = (usable * ARTIFACT_HEAD_RATIO_NUM) / ARTIFACT_HEAD_RATIO_DEN;
            let tail_bytes = usable.saturating_sub(head_bytes);
            let head_cut = head_chunk
                .content
                .floor_char_boundary(head_bytes.min(head_chunk.content.len()));
            let head_part = &head_chunk.content[..head_cut];
            let tail_offset = head_chunk.total_bytes.saturating_sub(tail_bytes as u64);
            let tail_part = supervisor
                .read_job_output(&rec.id, Some(tail_offset), tail_bytes + 64)
                .await
                .map(|c| c.content)
                .unwrap_or_default();
            let omitted = head_chunk
                .total_bytes
                .saturating_sub((head_part.len() + tail_part.len()) as u64);
            format!(
                "exit_code: {code}\n{head_part}\n\n... [{omitted} bytes omitted; full log ({} bytes) at {} — use `job_read` to inspect] ...\n\n{tail_part}",
                head_chunk.total_bytes, rec.log_path
            )
        }
    };
    Ok(match code == 0 {
        true => ToolOutput::ok(body),
        false => ToolOutput::err(body),
    })
}

#[async_trait]
impl Tool for ShellExecTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::Execution
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_SHELL_EXEC.into(),
            description: "Execute a shell command inside the workspace (or jailed `cwd`) with OS-native Landlock write sandboxing, sanitized env vars (zero secrets), timeout, output bounds, live background demotion, and optional persistent Unix PTY terminal sessions (`session` + `pty_action` preserving `cd`, `export`, virtualenvs, and TTY REPLs across turns).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Shell command to run (required for stateless exec and `pty_action=\"exec\"`)" },
                    "cwd": { "type": "string", "description": "Optional working directory relative to workspace (or `hub:...`), strictly jailed inside workspace/hub" },
                    "session": { "type": "string", "description": "Optional persistent PTY session name (e.g. `dev`, `py`) to preserve `cd`, `export`, virtualenvs, and TTY state across turns" },
                    "pty_action": { "type": "string", "enum": PTY_ACTIONS, "description": "Persistent PTY operation when `session` is used: `exec` (default), `send` (raw interactive stdin), `close`, or `list`" },
                    "input": { "type": "string", "description": "Raw stdin text for `pty_action=\"send\"` (include `\\n` if pressing Enter)" },
                    "wait_ms": { "type": "integer", "description": "Milliseconds to collect PTY output after `pty_action=\"send\"` (default 350, max 10000)" },
                    "kind": { "type": "string", "enum": ["shell", "build", "dev", "debug", "script", "cron", "browser", "subagent"], "description": "Optional ZenkoTasks category (auto-inferred from command if omitted)" },
                    "label": { "type": "string", "description": "Optional human-readable task label" },
                    "timeout_secs": { "type": "integer", "description": "Optional foreground timeout override in seconds" },
                    "run_in_background": { "type": "boolean", "description": "Run asynchronously as a managed ZenkoTask; streams output to ~/.zenko/tasks/<kind>/ and returns task_id immediately" },
                    "auto_background_after_secs": { "type": "integer", "description": "Optional seconds to wait before seamlessly handing off a long-running command to ZenkoTasks instead of killing it" }
                }
            }),
            risk: RiskLevel::Dangerous,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let exec_dir = match args
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(rel_cwd) => {
                let resolved = resolve_workspace_or_hub_path(
                    &ctx.workspace_dir,
                    &ctx.hub_dir,
                    rel_cwd,
                    true,
                )?;
                match tokio::fs::metadata(&resolved).await {
                    Ok(meta) if meta.is_dir() => resolved,
                    Ok(_) => {
                        return Ok(ToolOutput::err(format!(
                            "`cwd` `{rel_cwd}` exists but is not a directory."
                        )));
                    }
                    Err(e) => {
                        return Ok(ToolOutput::err(format!("Invalid `cwd` `{rel_cwd}`: {e}")));
                    }
                }
            }
            None => ctx.workspace_dir.clone(),
        };
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(ctx.shell_timeout_secs)
            .clamp(1, SHELL_MAX_TIMEOUT_SECS);
        let os_sandbox_enabled = self.os_sandbox.load(Ordering::Relaxed);

        // Persistent PTY terminal routing when `session` or `pty_action` is specified.
        let pty_session = args
            .get("session")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let pty_action = match args.get("pty_action").and_then(Value::as_str) {
            Some(raw) => Some(PtyAction::parse(raw).ok_or_else(|| {
                ZenkoError::Tool(format!(
                    "Invalid `pty_action` `{raw}`. Valid actions: {}.",
                    PTY_ACTIONS.join(", ")
                ))
            })?),
            None => pty_session.map(|_| PtyAction::Exec),
        };

        if let Some(action) = pty_action {
            match action {
                PtyAction::List => {
                    let active = self.pty.list_for_chat(&ctx.session_id).await;
                    let text = match active.is_empty() {
                        true => "No active persistent PTY sessions for this conversation.".into(),
                        false => active
                            .iter()
                            .map(|s| {
                                format!(
                                    "- `{}` (pid={}, cwd={}, idle={}s)",
                                    s.name, s.pid, s.initial_cwd, s.idle_secs
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                    };
                    return Ok(ToolOutput::ok(text));
                }
                PtyAction::Close => {
                    let name = pty_session.ok_or_else(|| {
                        ZenkoError::Tool("`session` name is required for `pty_action=\"close\"`".into())
                    })?;
                    let closed = self.pty.close_session(&ctx.session_id, name).await;
                    return Ok(match closed {
                        true => ToolOutput::ok(format!("Closed persistent PTY session `{name}`.")),
                        false => ToolOutput::ok(format!("PTY session `{name}` was not active.")),
                    });
                }
                PtyAction::Send => {
                    let name = pty_session.ok_or_else(|| {
                        ZenkoError::Tool("`session` name is required for `pty_action=\"send\"`".into())
                    })?;
                    let input = args
                        .get("input")
                        .and_then(Value::as_str)
                        .or_else(|| args.get("command").and_then(Value::as_str))
                        .ok_or_else(|| {
                            ZenkoError::Tool("`input` is required for `pty_action=\"send\"`".into())
                        })?;
                    let wait_ms = args
                        .get("wait_ms")
                        .and_then(Value::as_u64)
                        .unwrap_or(PTY_SEND_DEFAULT_WAIT_MS)
                        .clamp(20, PTY_SEND_MAX_WAIT_MS);
                    let output = self
                        .pty
                        .send_input(
                            &ctx.session_id,
                            name,
                            input,
                            Duration::from_millis(wait_ms),
                            &cancel,
                        )
                        .await?;
                    return Ok(ToolOutput::ok(format!(
                        "[pty:{name}]\n{output}"
                    )));
                }
                PtyAction::Exec => {
                    let name = pty_session.ok_or_else(|| {
                        ZenkoError::Tool("`session` name is required for `pty_action=\"exec\"`".into())
                    })?;
                    let cmd_str = args
                        .get("command")
                        .and_then(Value::as_str)
                        .ok_or_else(|| ZenkoError::Tool("missing `command`".into()))?;
                    let outcome = self
                        .pty
                        .exec(
                            &ctx.session_id,
                            name,
                            cmd_str,
                            &exec_dir,
                            &ctx.hub_dir,
                            os_sandbox_enabled,
                            Duration::from_secs(timeout_secs),
                            &cancel,
                        )
                        .await?;
                    let limit = ctx.max_output_bytes.max(SHELL_MIN_OUTPUT_BYTES);
                    let mut body = match outcome.timed_out {
                        true => format!(
                            "exit_code: {} [pty:{name} timed out after {timeout_secs}s; sent Ctrl-C]\n{}",
                            outcome.exit_code, outcome.output
                        ),
                        false => format!(
                            "exit_code: {} [pty:{name}]\n{}",
                            outcome.exit_code, outcome.output
                        ),
                    };
                    if body.len() > limit {
                        let cut = body.floor_char_boundary(limit);
                        body = format!(
                            "{}\n\n[pty output truncated at {limit} bytes; full length {} bytes]",
                            &body[..cut],
                            body.len()
                        );
                    }
                    return Ok(match outcome.exit_code == 0 && !outcome.timed_out {
                        true => ToolOutput::ok(body),
                        false => ToolOutput::err(body),
                    });
                }
            }
        }

        let cmd_str = args
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `command`".into()))?;
        let run_in_background = args
            .get("run_in_background")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let task_kind = args
            .get("kind")
            .and_then(Value::as_str)
            .map(TaskKind::from_str_loose)
            .unwrap_or_else(|| TaskKind::infer_from_command(cmd_str));
        let task_label = args
            .get("label")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let effective_auto_bg = args
            .get("auto_background_after_secs")
            .and_then(Value::as_u64)
            .or_else(|| {
                (ctx.auto_background_after_secs > 0).then_some(ctx.auto_background_after_secs)
            })
            .map(|s| s.clamp(1, JOB_MAX_AUTO_BACKGROUND_SECS));

        match (run_in_background, self.jobs.as_ref()) {
            (true, None) => {
                return Ok(ToolOutput::err(
                    "Background job supervisor is not attached in this runtime context.",
                ));
            }
            (true, Some(supervisor)) => {
                let job = supervisor
                    .spawn_shell(
                        &ctx.session_id,
                        cmd_str,
                        &exec_dir,
                        task_kind,
                        task_label,
                        true,
                    )
                    .await?;
                return Ok(ToolOutput {
                    content: format!(
                        "Started background task `{}` [{}] (pid={:?}, status={}). Log: {}\nUse `job_wait`, `job_read`, `job_input`, `job_stop`, or `task_manage` to inspect or control it.",
                        job.id,
                        job.kind.as_str(),
                        job.pid,
                        job.status.as_str(),
                        job.log_path
                    ),
                    is_error: false,
                    diff: None,
                    metadata: Some(serde_json::to_value(&job).unwrap_or(Value::Null)),
                });
            }
            (false, Some(supervisor)) => {
                // Unified supervised foreground path: zero lost output if auto-backgrounded
                // or demoted live via API / CLI / WebUI while running.
                let job = supervisor
                    .spawn_shell(
                        &ctx.session_id,
                        cmd_str,
                        &exec_dir,
                        task_kind,
                        task_label,
                        false,
                    )
                    .await?;

                let (wait_secs, is_auto_bg_deadline) = match effective_auto_bg {
                    Some(auto_s) if auto_s < timeout_secs => (auto_s, true),
                    _ => (timeout_secs, false),
                };

                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        let _ = supervisor.stop_job(&job.id).await;
                        return Err(ZenkoError::Cancelled);
                    }
                    _ = ctx.demote_bg.notified() => {
                        supervisor.enable_completion_notify(&job.id);
                        let latest = supervisor.get_job(&job.id)?.unwrap_or(job);
                        return match latest.status {
                            BackgroundJobStatus::Running => Ok(ToolOutput {
                                content: format!(
                                    "Foreground command demoted to background job `{}` (pid={:?}). Log: {}\nIt continues running asynchronously; you will be notified when it finishes, or use `job_wait` / `job_read` / `job_stop`.",
                                    latest.id, latest.pid, latest.log_path
                                ),
                                is_error: false,
                                diff: None,
                                metadata: Some(serde_json::to_value(&latest).unwrap_or(Value::Null)),
                            }),
                            _ => format_completed_job(supervisor, &latest, ctx.max_output_bytes).await,
                        };
                    }
                    res = supervisor.wait_job(&job.id, Duration::from_secs(wait_secs)) => {
                        let latest = res?.unwrap_or(job);
                        return match (latest.status, is_auto_bg_deadline) {
                            (BackgroundJobStatus::Running, true) => {
                                supervisor.enable_completion_notify(&latest.id);
                                Ok(ToolOutput {
                                    content: format!(
                                        "Command still running after {wait_secs}s — auto-backgrounded as job `{}` (pid={:?}). Log: {}\nYou will be notified when it finishes, or use `job_wait` / `job_read` / `job_stop`.",
                                        latest.id, latest.pid, latest.log_path
                                    ),
                                    is_error: false,
                                    diff: None,
                                    metadata: Some(serde_json::to_value(&latest).unwrap_or(Value::Null)),
                                })
                            }
                            (BackgroundJobStatus::Running, false) => {
                                let _ = supervisor.stop_job(&latest.id).await;
                                Ok(ToolOutput::err(format!("Command timed out after {timeout_secs}s")))
                            }
                            _ => format_completed_job(supervisor, &latest, ctx.max_output_bytes).await,
                        };
                    }
                }
            }
            (false, None) => {}
        }

        let mut cmd = Command::new(SHELL_BIN_UNIX);
        cmd.arg("-c")
            .arg(cmd_str)
            .current_dir(&exec_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        #[cfg(unix)]
        cmd.process_group(0);

        cmd.env_clear();
        for key in SAFE_SHELL_ENV_KEYS {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }
        let sandbox_policy = ShellSandboxPolicy::for_workspace(
            os_sandbox_enabled,
            &ctx.workspace_dir,
            Some(&ctx.hub_dir),
            &[],
        );
        install_sandbox_on_command(&mut cmd, &sandbox_policy);

        let child = cmd
            .spawn()
            .map_err(|e| ZenkoError::Tool(format!("failed to spawn shell: {e}")))?;

        let wait_fut = child.wait_with_output();
        let output = tokio::select! {
            _ = cancel.cancelled() => {
                return Err(ZenkoError::Cancelled);
            }
            res = tokio::time::timeout(Duration::from_secs(timeout_secs), wait_fut) => {
                match res {
                    Ok(Ok(out)) => out,
                    Ok(Err(e)) => return Err(ZenkoError::Tool(format!("shell error: {e}"))),
                    Err(_) => return Ok(ToolOutput::err(format!("Command timed out after {timeout_secs}s"))),
                }
            }
        };

        let code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut combined = format!("exit_code: {code}\n");
        if !stdout.is_empty() {
            combined.push_str("--- stdout ---\n");
            combined.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !combined.ends_with('\n') {
                combined.push('\n');
            }
            combined.push_str("--- stderr ---\n");
            combined.push_str(&stderr);
        }

        let limit = ctx.max_output_bytes.max(SHELL_MIN_OUTPUT_BYTES);
        if combined.len() > limit {
            let truncated = &combined[..combined.floor_char_boundary(limit)];
            combined = format!(
                "{truncated}\n\n[output truncated at {limit} bytes; full length {} bytes]",
                combined.len()
            );
        }

        match code == 0 {
            true => Ok(ToolOutput::ok(combined)),
            false => Ok(ToolOutput::err(combined)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shell_exec_honors_jailed_cwd_and_blocks_escape() {
        let dir = std::env::temp_dir().join(format!("zenko_shell_cwd_{}", zenko_core::new_id()));
        let sub = dir.join("subcrate");
        std::fs::create_dir_all(&sub).expect("mkdir");
        let ctx = ToolContext {
            session_id: "s1".into(),
            workspace_dir: dir.clone(),
            data_dir: dir.clone(),
            hub_dir: dir.clone(),
            shell_timeout_secs: 10,
            auto_background_after_secs: 0,
            max_output_bytes: 8192,
            headless_no_wait: true,
            demote_bg: Arc::new(tokio::sync::Notify::new()),
            file_state: None,
        };
        let tool = ShellExecTool::new(None);
        let ok = tool
            .execute(
                &ctx,
                json!({ "command": "pwd", "cwd": "subcrate" }),
                CancellationToken::new(),
            )
            .await
            .expect("pwd");
        assert!(!ok.is_error, "err={}", ok.content);
        assert!(ok.content.contains("subcrate"), "out={}", ok.content);

        let escaped = tool
            .execute(
                &ctx,
                json!({ "command": "pwd", "cwd": "../../.." }),
                CancellationToken::new(),
            )
            .await;
        assert!(escaped.is_err(), "cwd escape must be rejected by jail");

        let _ = std::fs::remove_dir_all(dir);
    }
}
