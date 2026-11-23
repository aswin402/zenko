//! Event-driven Hono-style activity logger for Zenko's autonomous runtime.
//! Subscribes to `AgentRuntime::events` and renders concise, aligned single-line
//! telemetry for user input, LLM thinking/streaming, tool calls, diffs, approvals,
//! completion metrics, errors, and gateway pairings.

use crate::AgentRuntime;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    AgentEvent, ApprovalDecision, LOG_PREVIEW_APPROVAL_ARGS_CHARS, LOG_PREVIEW_ARGS_CHARS,
    LOG_PREVIEW_ERR_CHARS, LOG_PREVIEW_INPUT_CHARS, LogChannel, LogDirection, LogStatusTone,
    SessionStatus, compact_preview, compact_tool_args, diff_line_stats, emit_hono_line,
    format_latency, format_tokens, short_model_name,
};

/// Pure rendered representation of one agent activity line plus structured correlation IDs.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedActivity {
    pub dir: LogDirection,
    pub channel: LogChannel,
    pub subject: String,
    pub status: Option<(String, LogStatusTone)>,
    pub meta: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub call_id: Option<String>,
    pub payload: Option<Value>,
}

/// Stateful debouncer & latency tracker for streaming agent events.
#[derive(Default)]
pub struct ActivityTracker {
    step_starts: HashMap<String, Instant>,
    logged_thinking: HashSet<String>,
    logged_streaming: HashSet<String>,
    approval_tools: HashMap<String, String>,
}

impl ActivityTracker {
    pub fn render_event<F>(
        &mut self,
        ev: &AgentEvent,
        resolve_session: F,
    ) -> Option<RenderedActivity>
    where
        F: Fn(&str) -> (String, String),
    {
        match ev {
            AgentEvent::UserMessageAccepted {
                session_id,
                event_id,
                text,
            } => {
                self.step_starts.insert(session_id.clone(), Instant::now());
                let (channel, _) = resolve_session(session_id);
                let preview = compact_preview(text, LOG_PREVIEW_INPUT_CHARS);
                Some(RenderedActivity {
                    dir: LogDirection::In,
                    channel: LogChannel::Input,
                    subject: format!("[{channel}] \"{preview}\""),
                    status: None,
                    meta: None,
                    session_id: Some(session_id.clone()),
                    turn_id: None,
                    call_id: None,
                    payload: Some(json!({
                        "event_id": event_id,
                        "source_channel": channel,
                        "chars": text.chars().count(),
                    })),
                })
            }

            AgentEvent::ThinkingDelta {
                session_id,
                turn_id,
                ..
            } => match self.logged_thinking.insert(turn_id.clone()) {
                false => None,
                true => {
                    let (_, model) = resolve_session(session_id);
                    Some(RenderedActivity {
                        dir: LogDirection::Work,
                        channel: LogChannel::Agent,
                        subject: format!("{} · thinking...", short_model_name(&model)),
                        status: None,
                        meta: None,
                        session_id: Some(session_id.clone()),
                        turn_id: Some(turn_id.clone()),
                        call_id: None,
                        payload: Some(json!({ "phase": "thinking", "model": model })),
                    })
                }
            },

            AgentEvent::TextDelta {
                session_id,
                turn_id,
                ..
            } => match self.logged_streaming.insert(turn_id.clone()) {
                false => None,
                true => {
                    let (_, model) = resolve_session(session_id);
                    Some(RenderedActivity {
                        dir: LogDirection::Work,
                        channel: LogChannel::Agent,
                        subject: format!("{} · responding...", short_model_name(&model)),
                        status: None,
                        meta: None,
                        session_id: Some(session_id.clone()),
                        turn_id: Some(turn_id.clone()),
                        call_id: None,
                        payload: Some(json!({ "phase": "responding", "model": model })),
                    })
                }
            },

            AgentEvent::ToolCallStarted {
                session_id,
                call_id,
                tool_name,
                args_preview,
                risk,
            } => {
                let args = compact_tool_args(args_preview, LOG_PREVIEW_ARGS_CHARS);
                let subject = match args.is_empty() {
                    true => tool_name.clone(),
                    false => format!("{tool_name}  {args}"),
                };
                Some(RenderedActivity {
                    dir: LogDirection::Work,
                    channel: LogChannel::Tool,
                    subject,
                    status: None,
                    meta: None,
                    session_id: Some(session_id.clone()),
                    turn_id: None,
                    call_id: Some(call_id.clone()),
                    payload: Some(json!({
                        "tool": tool_name,
                        "risk": risk.as_str(),
                        "args_preview": args_preview,
                    })),
                })
            }

            AgentEvent::ToolApprovalRequested { approval } => {
                self.approval_tools
                    .insert(approval.id.clone(), approval.tool_name.clone());
                let args =
                    compact_tool_args(&approval.args_preview, LOG_PREVIEW_APPROVAL_ARGS_CHARS);
                let subject = match args.is_empty() {
                    true => approval.tool_name.clone(),
                    false => format!("{}  {args}", approval.tool_name),
                };
                Some(RenderedActivity {
                    dir: LogDirection::Warn,
                    channel: LogChannel::Approve,
                    subject,
                    status: Some((approval.risk.as_str().to_string(), LogStatusTone::Warn)),
                    meta: Some("waiting".to_string()),
                    session_id: Some(approval.session_id.clone()),
                    turn_id: None,
                    call_id: Some(approval.call_id.clone()),
                    payload: Some(json!({
                        "approval_id": approval.id,
                        "tool": approval.tool_name,
                        "risk": approval.risk.as_str(),
                        "args_preview": approval.args_preview,
                    })),
                })
            }

            AgentEvent::ToolApprovalResolved {
                approval_id,
                session_id,
                decision,
                resolved_by,
            } => {
                let tool_name = self
                    .approval_tools
                    .remove(approval_id)
                    .unwrap_or_else(|| "tool".to_string());
                let (label, tone) = match decision {
                    ApprovalDecision::Approve => ("approved", LogStatusTone::Ok),
                    ApprovalDecision::AlwaysAllow => ("always", LogStatusTone::Ok),
                    ApprovalDecision::Deny => ("denied", LogStatusTone::Err),
                };
                Some(RenderedActivity {
                    dir: LogDirection::Out,
                    channel: LogChannel::Approve,
                    subject: tool_name.clone(),
                    status: Some((label.to_string(), tone)),
                    meta: Some(resolved_by.clone()),
                    session_id: Some(session_id.clone()),
                    turn_id: None,
                    call_id: None,
                    payload: Some(json!({
                        "approval_id": approval_id,
                        "tool": tool_name,
                        "decision": label,
                        "resolved_by": resolved_by,
                    })),
                })
            }

            AgentEvent::ToolCallFinished {
                session_id,
                call_id,
                tool_name,
                is_error,
                duration_ms,
                diff,
                output_preview,
            } => {
                self.step_starts.insert(session_id.clone(), Instant::now());
                let elapsed = format_latency(Duration::from_millis(*duration_ms));
                let diff_stats = diff.as_deref().map(diff_line_stats);
                let payload = Some(json!({
                    "tool": tool_name,
                    "is_error": is_error,
                    "duration_ms": duration_ms,
                    "diff_added": diff_stats.map(|(a, _)| a),
                    "diff_removed": diff_stats.map(|(_, d)| d),
                    "output_preview": output_preview,
                }));
                match is_error {
                    true => {
                        let err_hint = compact_preview(output_preview, LOG_PREVIEW_ERR_CHARS);
                        let subject = match err_hint.is_empty() {
                            true => tool_name.clone(),
                            false => format!("{tool_name} · {err_hint}"),
                        };
                        Some(RenderedActivity {
                            dir: LogDirection::Out,
                            channel: LogChannel::Tool,
                            subject,
                            status: Some(("err".to_string(), LogStatusTone::Err)),
                            meta: Some(elapsed),
                            session_id: Some(session_id.clone()),
                            turn_id: None,
                            call_id: Some(call_id.clone()),
                            payload,
                        })
                    }
                    false => {
                        let meta = match diff_stats {
                            Some((add, del)) => format!("{elapsed} (+{add}/-{del})"),
                            None => elapsed,
                        };
                        Some(RenderedActivity {
                            dir: LogDirection::Out,
                            channel: LogChannel::Tool,
                            subject: tool_name.clone(),
                            status: Some(("ok".to_string(), LogStatusTone::Ok)),
                            meta: Some(meta),
                            session_id: Some(session_id.clone()),
                            turn_id: None,
                            call_id: Some(call_id.clone()),
                            payload,
                        })
                    }
                }
            }

            AgentEvent::TurnCompleted {
                session_id,
                turn_id,
                event_id,
                usage,
            } => {
                self.logged_thinking.remove(turn_id);
                self.logged_streaming.remove(turn_id);
                let elapsed_dur = self
                    .step_starts
                    .insert(session_id.clone(), Instant::now())
                    .map(|started| started.elapsed());
                let elapsed = elapsed_dur.map(format_latency);
                let (_, model) = resolve_session(session_id);

                let mut meta_parts: Vec<String> = Vec::with_capacity(3);
                if let Some(lat) = elapsed {
                    meta_parts.push(lat);
                }

                let total_tokens = usage.prompt_tokens.saturating_add(usage.completion_tokens);
                if total_tokens > 0 {
                    let cache_suffix = match (usage.prompt_tokens > 0, usage.cache_read_tokens > 0) {
                        (true, true) => {
                            let pct = (usage.cache_read_tokens.saturating_mul(100))
                                / usage.prompt_tokens;
                            format!(" ({pct}% cache)")
                        }
                        _ => String::new(),
                    };
                    meta_parts.push(format!("{} tok{cache_suffix}", format_tokens(total_tokens)));
                }

                if usage.cost_usd > 0.0 {
                    meta_parts.push(format!("${:.4}", usage.cost_usd));
                }

                let meta = match meta_parts.is_empty() {
                    true => None,
                    false => Some(meta_parts.join(" · ")),
                };

                Some(RenderedActivity {
                    dir: LogDirection::Out,
                    channel: LogChannel::Agent,
                    subject: short_model_name(&model).to_string(),
                    status: Some(("done".to_string(), LogStatusTone::Ok)),
                    meta,
                    session_id: Some(session_id.clone()),
                    turn_id: Some(turn_id.clone()),
                    call_id: None,
                    payload: Some(json!({
                        "event_id": event_id,
                        "model": model,
                        "latency_ms": elapsed_dur.map(|d| d.as_millis() as u64),
                        "prompt_tokens": usage.prompt_tokens,
                        "completion_tokens": usage.completion_tokens,
                        "cache_read_tokens": usage.cache_read_tokens,
                        "cost_usd": usage.cost_usd,
                    })),
                })
            }

            AgentEvent::TurnError {
                session_id,
                message,
            } => {
                self.step_starts.remove(session_id);
                Some(RenderedActivity {
                    dir: LogDirection::Err,
                    channel: LogChannel::Agent,
                    subject: compact_preview(message, LOG_PREVIEW_ERR_CHARS),
                    status: Some(("err".to_string(), LogStatusTone::Err)),
                    meta: None,
                    session_id: Some(session_id.clone()),
                    turn_id: None,
                    call_id: None,
                    payload: Some(json!({ "error": message })),
                })
            }

            AgentEvent::GatewayPaired { peer } => {
                let handle = peer.username.as_deref().unwrap_or(&peer.peer_id);
                Some(RenderedActivity {
                    dir: LogDirection::Out,
                    channel: LogChannel::Gateway,
                    subject: format!("{} paired @{handle}", peer.channel),
                    status: Some((peer.role.as_str().to_string(), LogStatusTone::Ok)),
                    meta: None,
                    session_id: None,
                    turn_id: None,
                    call_id: None,
                    payload: Some(json!({
                        "channel": peer.channel,
                        "peer_id": peer.peer_id,
                        "username": peer.username,
                        "role": peer.role.as_str(),
                    })),
                })
            }

            AgentEvent::JobUpdated { job } => {
                let (dir, tone) = match job.status {
                    zenko_core::BackgroundJobStatus::Running => (LogDirection::Work, LogStatusTone::Info),
                    zenko_core::BackgroundJobStatus::Completed => (LogDirection::Out, LogStatusTone::Ok),
                    zenko_core::BackgroundJobStatus::Stopped => (LogDirection::Warn, LogStatusTone::Neutral),
                    zenko_core::BackgroundJobStatus::Failed
                    | zenko_core::BackgroundJobStatus::Interrupted => {
                        (LogDirection::Err, LogStatusTone::Err)
                    }
                };
                let cmd_preview = compact_preview(&job.command, LOG_PREVIEW_ARGS_CHARS);
                let meta = match job.exit_code {
                    Some(c) => Some(format!("exit={c} · {}B", job.bytes_written)),
                    None => job.pid.map(|p| format!("pid={p}")),
                };
                Some(RenderedActivity {
                    dir,
                    channel: LogChannel::Tool,
                    subject: format!("job {}  {cmd_preview}", &job.id[..job.id.len().min(12)]),
                    status: Some((job.status.as_str().to_string(), tone)),
                    meta,
                    session_id: Some(job.session_id.clone()),
                    turn_id: None,
                    call_id: Some(job.id.clone()),
                    payload: Some(json!({
                        "job_id": job.id,
                        "status": job.status.as_str(),
                        "pid": job.pid,
                        "exit_code": job.exit_code,
                        "bytes_written": job.bytes_written,
                        "log_path": job.log_path,
                    })),
                })
            }

            AgentEvent::JobOutputDelta { .. } => None,

            AgentEvent::SteeringAccepted {
                session_id,
                preview,
            } => Some(RenderedActivity {
                dir: LogDirection::In,
                channel: LogChannel::Input,
                subject: format!("[steer] \"{}\"", compact_preview(preview, LOG_PREVIEW_INPUT_CHARS)),
                status: Some(("steer".to_string(), LogStatusTone::Info)),
                meta: None,
                session_id: Some(session_id.clone()),
                turn_id: None,
                call_id: None,
                payload: Some(json!({ "steering": true, "preview": preview })),
            }),

            AgentEvent::OrchestratorUpdated {
                session_id,
                stage,
                tier,
                detail,
                model,
            } => Some(RenderedActivity {
                dir: LogDirection::Work,
                channel: LogChannel::Agent,
                subject: format!(
                    "orch [{}:{}] {}",
                    tier.as_str(),
                    stage,
                    compact_preview(detail, LOG_PREVIEW_ARGS_CHARS)
                ),
                status: Some((stage.clone(), LogStatusTone::Info)),
                meta: model.as_deref().map(|m| short_model_name(m).to_string()),
                session_id: Some(session_id.clone()),
                turn_id: None,
                call_id: None,
                payload: Some(json!({
                    "stage": stage,
                    "tier": tier.as_str(),
                    "detail": detail,
                    "model": model,
                })),
            }),

            AgentEvent::BrainstormRecorded { session_id, spec } => Some(RenderedActivity {
                dir: LogDirection::Out,
                channel: LogChannel::Agent,
                subject: format!(
                    "brainstorm ({} options -> {})",
                    spec.options.len(),
                    spec.selected_option_id
                ),
                status: Some(("plan".to_string(), LogStatusTone::Ok)),
                meta: Some(format!("{} checks", spec.verification_plan.len())),
                session_id: Some(session_id.clone()),
                turn_id: None,
                call_id: Some(spec.id.clone()),
                payload: Some(json!({
                    "brainstorm_id": spec.id,
                    "selected_option_id": spec.selected_option_id,
                    "options_count": spec.options.len(),
                })),
            }),

            AgentEvent::PlanUpdated { session_id, plan } => {
                let done = plan
                    .steps
                    .iter()
                    .filter(|s| s.status == zenko_core::PlanStepStatus::Completed)
                    .count();
                Some(RenderedActivity {
                    dir: LogDirection::Work,
                    channel: LogChannel::Agent,
                    subject: format!(
                        "plan ({done}/{}) · {}",
                        plan.steps.len(),
                        compact_preview(&plan.goal, LOG_PREVIEW_ARGS_CHARS)
                    ),
                    status: Some(("plan".to_string(), LogStatusTone::Info)),
                    meta: Some(format!("{} steps", plan.steps.len())),
                    session_id: Some(session_id.clone()),
                    turn_id: None,
                    call_id: Some(plan.id.clone()),
                    payload: Some(json!({
                        "plan_id": plan.id,
                        "goal": plan.goal,
                        "completed": done,
                        "total": plan.steps.len(),
                    })),
                })
            }

            AgentEvent::ClarificationRequested { clarification } => Some(RenderedActivity {
                dir: LogDirection::Warn,
                channel: LogChannel::Approve,
                subject: format!(
                    "clarify: {}",
                    compact_preview(&clarification.question, LOG_PREVIEW_ARGS_CHARS)
                ),
                status: Some(("clarify".to_string(), LogStatusTone::Warn)),
                meta: Some(format!("{} opts", clarification.options.len())),
                session_id: Some(clarification.session_id.clone()),
                turn_id: None,
                call_id: Some(clarification.id.clone()),
                payload: Some(json!({
                    "clarification_id": clarification.id,
                    "question": clarification.question,
                    "recommended": clarification.recommended_option_id,
                })),
            }),

            AgentEvent::ClarificationResolved {
                clarification_id,
                session_id,
                selected_option_id,
                resolved_by,
                ..
            } => Some(RenderedActivity {
                dir: LogDirection::Out,
                channel: LogChannel::Approve,
                subject: format!("clarify -> {selected_option_id}"),
                status: Some(("answered".to_string(), LogStatusTone::Ok)),
                meta: Some(resolved_by.clone()),
                session_id: Some(session_id.clone()),
                turn_id: None,
                call_id: Some(clarification_id.clone()),
                payload: Some(json!({
                    "clarification_id": clarification_id,
                    "selected_option_id": selected_option_id,
                    "resolved_by": resolved_by,
                })),
            }),

            AgentEvent::SubagentSpawned { agent } => Some(RenderedActivity {
                dir: LogDirection::Work,
                channel: LogChannel::Agent,
                subject: format!(
                    "subagent[{}] {} · {}",
                    agent.role,
                    &agent.id[..agent.id.len().min(10)],
                    compact_preview(&agent.goal, LOG_PREVIEW_ARGS_CHARS)
                ),
                status: Some(("spawn".to_string(), LogStatusTone::Info)),
                meta: agent
                    .worktree_branch
                    .clone()
                    .or_else(|| agent.model.as_deref().map(|m| short_model_name(m).to_string())),
                session_id: Some(agent.parent_session_id.clone()),
                turn_id: None,
                call_id: Some(agent.id.clone()),
                payload: Some(json!({
                    "agent_id": agent.id,
                    "parent_session_id": agent.parent_session_id,
                    "role": agent.role,
                    "workspace_mode": agent.workspace_mode.as_str(),
                    "worktree_branch": agent.worktree_branch,
                })),
            }),

            AgentEvent::SubagentCompleted { agent } => {
                let (dir, tone) = match agent.status {
                    zenko_core::SubagentStatus::Completed => (LogDirection::Out, LogStatusTone::Ok),
                    zenko_core::SubagentStatus::Killed
                    | zenko_core::SubagentStatus::Interrupted => {
                        (LogDirection::Warn, LogStatusTone::Neutral)
                    }
                    zenko_core::SubagentStatus::Failed => (LogDirection::Err, LogStatusTone::Err),
                    zenko_core::SubagentStatus::Running => (LogDirection::Work, LogStatusTone::Info),
                };
                let meta = match (agent.tokens_used > 0, !agent.changed_files.is_empty()) {
                    (true, true) => Some(format!(
                        "{} tok · {} files",
                        format_tokens(agent.tokens_used),
                        agent.changed_files.len()
                    )),
                    (true, false) => Some(format!("{} tok", format_tokens(agent.tokens_used))),
                    (false, true) => Some(format!("{} files", agent.changed_files.len())),
                    (false, false) => None,
                };
                Some(RenderedActivity {
                    dir,
                    channel: LogChannel::Agent,
                    subject: format!(
                        "subagent[{}] {}",
                        agent.role,
                        &agent.id[..agent.id.len().min(10)]
                    ),
                    status: Some((agent.status.as_str().to_string(), tone)),
                    meta,
                    session_id: Some(agent.parent_session_id.clone()),
                    turn_id: None,
                    call_id: Some(agent.id.clone()),
                    payload: Some(json!({
                        "agent_id": agent.id,
                        "role": agent.role,
                        "status": agent.status.as_str(),
                        "tokens_used": agent.tokens_used,
                        "cost_usd": agent.cost_usd,
                        "changed_files": agent.changed_files,
                    })),
                })
            }

            AgentEvent::SubagentMessage { message } => Some(RenderedActivity {
                dir: LogDirection::In,
                channel: LogChannel::Agent,
                subject: format!(
                    "mail {} -> {} · {}",
                    &message.from_session_id[..message.from_session_id.len().min(10)],
                    &message.to_session_id[..message.to_session_id.len().min(10)],
                    compact_preview(&message.body, LOG_PREVIEW_ARGS_CHARS)
                ),
                status: Some(("mail".to_string(), LogStatusTone::Info)),
                meta: None,
                session_id: Some(message.to_session_id.clone()),
                turn_id: None,
                call_id: Some(message.id.clone()),
                payload: Some(json!({
                    "message_id": message.id,
                    "from": message.from_session_id,
                    "to": message.to_session_id,
                })),
            }),

            AgentEvent::SessionTasksUpdated { session_id, tasks } => {
                let done = tasks
                    .iter()
                    .filter(|t| t.status == zenko_core::SessionTaskStatus::Done)
                    .count();
                Some(RenderedActivity {
                    dir: LogDirection::Work,
                    channel: LogChannel::Agent,
                    subject: format!("task dag updated ({done}/{} done)", tasks.len()),
                    status: Some(("dag".to_string(), LogStatusTone::Info)),
                    meta: Some(format!("{} tasks", tasks.len())),
                    session_id: Some(session_id.clone()),
                    turn_id: None,
                    call_id: None,
                    payload: Some(json!({
                        "completed": done,
                        "total": tasks.len(),
                    })),
                })
            }

            AgentEvent::CheckpointCreated { checkpoint } => Some(RenderedActivity {
                dir: LogDirection::Work,
                channel: LogChannel::Agent,
                subject: format!(
                    "checkpoint {} · {}",
                    &checkpoint.id[..checkpoint.id.len().min(8)],
                    compact_preview(&checkpoint.label, LOG_PREVIEW_ARGS_CHARS)
                ),
                status: Some((checkpoint.backend.as_str().to_string(), LogStatusTone::Info)),
                meta: Some(checkpoint.trigger_tool.clone()),
                session_id: Some(checkpoint.session_id.clone()),
                turn_id: Some(checkpoint.turn_id.clone()),
                call_id: Some(checkpoint.id.clone()),
                payload: Some(json!({
                    "checkpoint_id": checkpoint.id,
                    "backend": checkpoint.backend.as_str(),
                    "trigger_tool": checkpoint.trigger_tool,
                    "is_safety_backup": checkpoint.is_safety_backup,
                })),
            }),

            AgentEvent::SessionRewound { report } => Some(RenderedActivity {
                dir: LogDirection::Out,
                channel: LogChannel::Agent,
                subject: format!(
                    "rewind {} ({} restored, {} deleted)",
                    &report.checkpoint_id[..report.checkpoint_id.len().min(8)],
                    report.restored_files.len(),
                    report.deleted_files.len()
                ),
                status: Some((report.mode.as_str().to_string(), LogStatusTone::Ok)),
                meta: match report.preserved_manual_edits.is_empty() {
                    true => None,
                    false => Some(format!(
                        "{} hand-edits kept",
                        report.preserved_manual_edits.len()
                    )),
                },
                session_id: Some(report.session_id.clone()),
                turn_id: None,
                call_id: Some(report.checkpoint_id.clone()),
                payload: Some(json!({
                    "checkpoint_id": report.checkpoint_id,
                    "mode": report.mode.as_str(),
                    "restored_files": report.restored_files,
                    "deleted_files": report.deleted_files,
                    "preserved_manual_edits": report.preserved_manual_edits,
                    "safety_checkpoint_id": report.safety_checkpoint_id,
                })),
            }),

            AgentEvent::PlanApproved {
                session_id,
                new_mode,
                task_count,
            } => Some(RenderedActivity {
                dir: LogDirection::Out,
                channel: LogChannel::Agent,
                subject: format!("plan approved -> mode={}", new_mode.as_str()),
                status: Some(("approved".to_string(), LogStatusTone::Ok)),
                meta: Some(format!("{task_count} tasks")),
                session_id: Some(session_id.clone()),
                turn_id: None,
                call_id: None,
                payload: Some(json!({
                    "new_mode": new_mode.as_str(),
                    "task_count": task_count,
                })),
            }),

            AgentEvent::McpServersUpdated { servers } => {
                let connected = servers.iter().filter(|s| s.connected).count();
                let total_tools: usize = servers.iter().map(|s| s.tool_count).sum();
                Some(RenderedActivity {
                    dir: LogDirection::Out,
                    channel: LogChannel::System,
                    subject: format!(
                        "mcp catalog synced ({connected}/{} connected)",
                        servers.len()
                    ),
                    status: Some(("mcp".to_string(), LogStatusTone::Info)),
                    meta: Some(format!("{total_tools} tools")),
                    session_id: None,
                    turn_id: None,
                    call_id: None,
                    payload: Some(json!({
                        "server_count": servers.len(),
                        "connected": connected,
                        "total_tools": total_tools,
                    })),
                })
            }

            AgentEvent::SkillUpdated {
                slug,
                action,
                status,
            } => Some(RenderedActivity {
                dir: LogDirection::Work,
                channel: LogChannel::System,
                subject: format!("skill/{slug} {action} ({status})"),
                status: Some((status.clone(), LogStatusTone::Info)),
                meta: None,
                session_id: None,
                turn_id: None,
                call_id: None,
                payload: Some(json!({
                    "slug": slug,
                    "action": action,
                    "status": status,
                })),
            }),

            AgentEvent::GoalLoopUpdated { goal } => {
                let tone = match goal.status {
                    zenko_core::GoalLoopStatus::Succeeded => LogStatusTone::Ok,
                    zenko_core::GoalLoopStatus::Active | zenko_core::GoalLoopStatus::Paused => {
                        LogStatusTone::Info
                    }
                    _ => LogStatusTone::Err,
                };
                Some(RenderedActivity {
                    dir: LogDirection::Work,
                    channel: LogChannel::Cron,
                    subject: format!(
                        "goal/{} iter {}/{} ({})",
                        goal.name,
                        goal.iterations_done,
                        goal.max_iterations,
                        goal.status.as_str()
                    ),
                    status: Some((goal.status.as_str().to_string(), tone)),
                    meta: Some(format!("${:.4}", goal.cost_spent_usd)),
                    session_id: Some(goal.session_id.clone()),
                    turn_id: None,
                    call_id: None,
                    payload: Some(json!({
                        "goal_id": goal.id,
                        "status": goal.status.as_str(),
                        "iterations_done": goal.iterations_done,
                        "last_verify_exit": goal.last_verify_exit,
                    })),
                })
            }

            AgentEvent::SessionUpdated {
                session_id,
                status: SessionStatus::Idle | SessionStatus::Error,
                ..
            } => {
                self.step_starts.remove(session_id);
                None
            }

            // Explicitly ignored to avoid feedback loops and noise.
            AgentEvent::SessionUpdated { .. }
            | AgentEvent::QueueUpdated { .. }
            | AgentEvent::Telemetry(_)
            | AgentEvent::Log(_) => None,
        }
    }
}

/// Runs the zero-polling background task translating runtime `AgentEvent`s
/// into minimal Hono-style stdout lines and categorized persistent log entries.
pub async fn run_activity_logger(runtime: Arc<AgentRuntime>, cancel: CancellationToken) {
    let mut rx = runtime.subscribe();
    let mut tracker = ActivityTracker::default();

    loop {
        let ev = tokio::select! {
            _ = cancel.cancelled() => break,
            recv = rx.recv() => match recv {
                Ok(event) => event,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
        };

        let rendered = tracker.render_event(&ev, |session_id| {
            let cfg_model = runtime.config().provider.lead_model;
            match runtime.store.get_session(session_id).ok().flatten() {
                Some(s) => (s.channel, s.model_override.unwrap_or(cfg_model)),
                None => ("webui".to_string(), cfg_model),
            }
        });

        if let Some(act) = rendered {
            let status_ref = act.status.as_ref().map(|(s, t)| (s.as_str(), *t));
            let mut entry = emit_hono_line(
                act.dir,
                act.channel,
                &act.subject,
                status_ref,
                act.meta.as_deref(),
            );
            entry.session_id = act.session_id;
            entry.turn_id = act.turn_id;
            entry.call_id = act.call_id;
            entry.payload = act.payload;
            runtime.logs.push(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use zenko_core::{ApprovalRecord, ApprovalStatus, RiskLevel, UsageStats};

    fn dummy_resolver(_: &str) -> (String, String) {
        ("webui".to_string(), "anthropic/claude-sonnet-4".to_string())
    }

    #[test]
    fn renders_full_agent_lifecycle_and_debounces_deltas() {
        let mut tracker = ActivityTracker::default();

        // 1. User input
        let input = tracker
            .render_event(
                &AgentEvent::UserMessageAccepted {
                    session_id: "s1".into(),
                    event_id: "e1".into(),
                    text: "fix auth bug\nand run tests".into(),
                },
                dummy_resolver,
            )
            .expect("input rendered");
        assert_eq!(input.dir, LogDirection::In);
        assert_eq!(input.channel, LogChannel::Input);
        assert_eq!(input.subject, r#"[webui] "fix auth bug and run tests""#);

        // 2. Repeated thinking deltas collapse to one line
        let t1 = tracker.render_event(
            &AgentEvent::ThinkingDelta {
                session_id: "s1".into(),
                turn_id: "turn1".into(),
                text: "Let me".into(),
            },
            dummy_resolver,
        );
        let t2 = tracker.render_event(
            &AgentEvent::ThinkingDelta {
                session_id: "s1".into(),
                turn_id: "turn1".into(),
                text: " check".into(),
            },
            dummy_resolver,
        );
        assert_eq!(t1.expect("first thinking").subject, "claude-sonnet-4 · thinking...");
        assert!(t2.is_none());

        // 3. Tool call start & finish with diff stats
        let tc_start = tracker
            .render_event(
                &AgentEvent::ToolCallStarted {
                    session_id: "s1".into(),
                    call_id: "c1".into(),
                    tool_name: "edit_file".into(),
                    args_preview: r#"{"path":"src/auth.rs"}"#.into(),
                    risk: RiskLevel::Moderate,
                },
                dummy_resolver,
            )
            .expect("tool start");
        assert_eq!(tc_start.channel, LogChannel::Tool);
        assert_eq!(tc_start.subject, r#"edit_file  path="src/auth.rs""#);

        let tc_done = tracker
            .render_event(
                &AgentEvent::ToolCallFinished {
                    session_id: "s1".into(),
                    call_id: "c1".into(),
                    tool_name: "edit_file".into(),
                    is_error: false,
                    duration_ms: 12,
                    diff: Some("--- a/x\n+++ b/x\n-old\n+new1\n+new2\n".into()),
                    output_preview: "ok".into(),
                },
                dummy_resolver,
            )
            .expect("tool done");
        assert_eq!(tc_done.status, Some(("ok".to_string(), LogStatusTone::Ok)));
        assert_eq!(tc_done.meta.as_deref(), Some("12.0ms (+2/-1)"));

        // 4. Approval request + resolution carries tool name forward
        let apr_req = tracker
            .render_event(
                &AgentEvent::ToolApprovalRequested {
                    approval: ApprovalRecord {
                        id: "a1".into(),
                        session_id: "s1".into(),
                        call_id: "c2".into(),
                        tool_name: "shell".into(),
                        args_preview: r#"{"command":"cargo test"}"#.into(),
                        risk: RiskLevel::Dangerous,
                        status: ApprovalStatus::Pending,
                        resolved_by: None,
                        created_at: Utc::now(),
                        resolved_at: None,
                    },
                },
                dummy_resolver,
            )
            .expect("approval req");
        assert_eq!(apr_req.dir, LogDirection::Warn);
        assert_eq!(apr_req.status, Some(("dangerous".to_string(), LogStatusTone::Warn)));

        let apr_res = tracker
            .render_event(
                &AgentEvent::ToolApprovalResolved {
                    approval_id: "a1".into(),
                    session_id: "s1".into(),
                    decision: ApprovalDecision::Approve,
                    resolved_by: "webui".into(),
                },
                dummy_resolver,
            )
            .expect("approval resolved");
        assert_eq!(apr_res.subject, "shell");
        assert_eq!(apr_res.status, Some(("approved".to_string(), LogStatusTone::Ok)));

        // 5. Turn completion summarizes tokens, cache %, and cost
        let done = tracker
            .render_event(
                &AgentEvent::TurnCompleted {
                    session_id: "s1".into(),
                    turn_id: "turn1".into(),
                    event_id: "e2".into(),
                    usage: UsageStats {
                        prompt_tokens: 2_000,
                        completion_tokens: 500,
                        cache_read_tokens: 1_500,
                        cost_usd: 0.0042,
                    },
                },
                dummy_resolver,
            )
            .expect("turn completed");
        assert_eq!(done.subject, "claude-sonnet-4");
        assert_eq!(done.status, Some(("done".to_string(), LogStatusTone::Ok)));
        let meta = done.meta.expect("meta");
        assert!(meta.contains("2.5k tok (75% cache)"));
        assert!(meta.contains("$0.0042"));

        // 6. Log events are ignored to prevent feedback loops
        let ignored = tracker.render_event(
            &AgentEvent::Log(zenko_core::LogEntry {
                timestamp: Utc::now(),
                level: "INFO".into(),
                category: "session".into(),
                target: "agent".into(),
                session_id: None,
                turn_id: None,
                call_id: None,
                message: "loop test".into(),
                payload: None,
            }),
            dummy_resolver,
        );
        assert!(ignored.is_none());
    }
}
