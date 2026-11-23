//! Phase 8 Bounded 24/7 Autonomous Goal Loops ("Ralph Loop" + Prime Verifier + Shadow-Git).
//!
//! Runs zero-CPU via `tokio::time::sleep_until` + `runtime.goal_notify`:
//! 1. Enforces hard bounds (`max_iterations`, `max_budget_usd`, `max_consecutive_stalls`).
//! 2. Captures a pre-iteration Shadow-Git / `ContentBlobs` workspace checkpoint when enabled.
//! 3. Executes an agent turn with concise `<goal_loop_context>` (previous verifier failure tail & headroom).
//! 4. Runs the deterministic argv-only `verify_cmd` (ExecPolicy-checked, sanitized env, zero `/bin/sh`).
//! 5. Transitions deterministically on exit code (`0` -> `Succeeded`, repeated failure hash -> `Stalled` + optional auto-rewind).

use crate::policy::evaluate_verify_command_safety;
use crate::runtime::{AgentRuntime, TurnOptions};
use chrono::{Duration as ChronoDuration, Utc};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use zenko_core::{
    AgentEvent, ExecutionTier, GOAL_LOOP_FEEDBACK_TAIL_BYTES, GOAL_LOOP_HASH_BYTES,
    GOAL_LOOP_LABEL_PREVIEW_CHARS, GOAL_LOOP_LIST_DEFAULT_LIMIT, GOAL_LOOP_MIN_BACKOFF_SECS,
    GOAL_LOOP_MIN_REMAINING_BUDGET_USD, GOAL_LOOP_VERIFY_CAPTURE_CAP_BYTES,
    GOAL_LOOP_VERIFY_MAX_OUTPUT_BYTES, GOAL_LOOP_VERIFY_TIMEOUT_EXIT_CODE,
    GOAL_LOOP_VERIFY_TIMEOUT_SECS, GoalLoopRecord, GoalLoopRun, GoalLoopStatus, LogChannel,
    LogDirection, LogStatusTone, ORCH_STAGE_GOAL_LOOP, Result, RewindMode, RewindRequest,
    SAFE_SHELL_ENV_KEYS, ZenkoError, compact_preview, emit_hono_line, new_id, scrub_secrets,
};

#[derive(Debug, Clone)]
pub struct VerifyOutcome {
    pub exit_code: i32,
    pub output_excerpt: String,
    pub output_hash: String,
    pub timed_out: bool,
}

/// Zero-CPU event-driven Goal Loop scheduler waking strictly on next due iteration or `goal_notify`.
pub async fn run_goal_loop_scheduler(runtime: Arc<AgentRuntime>, cancel: CancellationToken) {
    info!("autonomous 24/7 goal loop scheduler started (zero-polling sleep_until)");
    loop {
        if cancel.is_cancelled() {
            break;
        }
        let goals = runtime
            .store
            .list_goal_loops(GOAL_LOOP_LIST_DEFAULT_LIMIT.max(200))
            .unwrap_or_default();
        let now = Utc::now();
        let mut next_due: Option<(chrono::DateTime<Utc>, GoalLoopRecord)> = None;

        for goal in goals
            .into_iter()
            .filter(|g| g.status == GoalLoopStatus::Active)
        {
            let due = goal.next_run_at.unwrap_or(now);
            if next_due.as_ref().map(|(t, _)| due < *t).unwrap_or(true) {
                next_due = Some((due, goal));
            }
        }

        match next_due {
            None => {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => break,
                    _ = runtime.goal_notify.notified() => continue,
                }
            }
            Some((due, goal)) => {
                let wait_ms = (due - now).num_milliseconds();
                if wait_ms > 0 {
                    let sleep = tokio::time::sleep(Duration::from_millis(wait_ms as u64));
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => break,
                        _ = runtime.goal_notify.notified() => continue,
                        _ = sleep => {}
                    }
                }
                if let Err(e) = execute_goal_iteration(&runtime, &goal.id, cancel.clone()).await {
                    warn!(goal_id = %goal.id, error = %e, "goal loop iteration failed");
                    if let Ok(Some(mut cur)) = runtime.store.get_goal_loop(&goal.id)
                        && cur.status == GoalLoopStatus::Active {
                            let backoff = cur.interval_secs.max(GOAL_LOOP_MIN_BACKOFF_SECS * 2);
                            cur.next_run_at = Some(Utc::now() + ChronoDuration::seconds(backoff as i64));
                            cur.updated_at = Utc::now();
                            let _ = runtime.store.upsert_goal_loop(&cur);
                        }
                }
            }
        }
    }
}

/// Executes a single bounded iteration of `goal_id`, serialized per-goal via `runtime.goal_lock`,
/// returning the updated `GoalLoopRecord`.
pub async fn execute_goal_iteration(
    runtime: &Arc<AgentRuntime>,
    goal_id: &str,
    cancel: CancellationToken,
) -> Result<GoalLoopRecord> {
    let goal_lock = runtime.goal_lock(goal_id);
    let _goal_guard = goal_lock.lock().await;

    let mut goal = runtime
        .store
        .get_goal_loop(goal_id)?
        .ok_or_else(|| ZenkoError::NotFound(format!("goal loop `{goal_id}` not found")))?;

    if goal.status.is_terminal() || goal.status == GoalLoopStatus::Paused {
        return Ok(goal);
    }

    // Hard pre-flight budget & iteration guards
    if goal.iterations_done >= goal.max_iterations {
        goal.status = GoalLoopStatus::Failed;
        goal.next_run_at = None;
        goal.updated_at = Utc::now();
        runtime.store.upsert_goal_loop(&goal)?;
        runtime.emit(AgentEvent::GoalLoopUpdated { goal: goal.clone() });
        return Ok(goal);
    }
    if goal.max_budget_usd > 0.0 && goal.cost_spent_usd >= goal.max_budget_usd {
        goal.status = GoalLoopStatus::BudgetExhausted;
        goal.next_run_at = None;
        goal.updated_at = Utc::now();
        runtime.store.upsert_goal_loop(&goal)?;
        runtime.emit(AgentEvent::GoalLoopUpdated { goal: goal.clone() });
        return Ok(goal);
    }

    let cfg = runtime.config();
    evaluate_verify_command_safety(&cfg.policy, &goal.verify_cmd)
        .map_err(ZenkoError::PolicyDenied)?;

    let iter_num = goal.iterations_done + 1;
    let started = Instant::now();
    let ws_dir = PathBuf::from(&goal.workspace_dir);

    // 1. Optional pre-iteration Shadow-Git / ContentBlobs checkpoint
    let checkpoint_id = match goal.checkpoint_each_iter && cfg.checkpoints.enabled && ws_dir.exists()
    {
        true => runtime
            .checkpoints
            .capture_checkpoint(
                &cfg.checkpoints,
                &goal.session_id,
                &format!("goal_{}_{}", goal.id, iter_num),
                None,
                &ws_dir,
                "goal_loop",
                format!(
                    "Goal `{}` iter {iter_num}",
                    compact_preview(&goal.name, GOAL_LOOP_LABEL_PREVIEW_CHARS)
                ),
                false,
            )
            .await
            .ok()
            .map(|cp| cp.id),
        false => None,
    };

    // 2. Execute bounded agent turn with concise previous verifier feedback
    let remaining_budget = match goal.max_budget_usd > 0.0 {
        true => Some(
            (goal.max_budget_usd - goal.cost_spent_usd).max(GOAL_LOOP_MIN_REMAINING_BUDGET_USD),
        ),
        false => None,
    };
    let turn_prompt = build_goal_iteration_prompt(&goal, iter_num);
    let turn_res = runtime
        .run_turn_with_options(
            &goal.session_id,
            &turn_prompt,
            TurnOptions {
                workspace_override: Some(ws_dir.clone()),
                model_override: goal.model.clone(),
                headless_no_wait: true,
                max_cost_usd_override: remaining_budget,
                external_cancel: Some(cancel.clone()),
                ..TurnOptions::default()
            },
        )
        .await;

    let (tokens_used, iter_cost, turn_err) = match turn_res {
        Ok(res) => (
            res.usage.prompt_tokens + res.usage.completion_tokens,
            res.usage.cost_usd,
            None,
        ),
        Err(e) => (0, 0.0, Some(e.to_string())),
    };

    if let Some(ref cp_id) = checkpoint_id {
        let _ = runtime
            .checkpoints
            .finalize_checkpoint(&cfg.checkpoints, cp_id)
            .await;
    }

    // 3. Deterministic argv-only verification command (never trusts LLM self-claims!)
    let known_secrets = runtime.vault.all_values();
    let verify = run_verify_argv(
        &goal.verify_cmd,
        &ws_dir,
        Duration::from_secs(GOAL_LOOP_VERIFY_TIMEOUT_SECS),
        &known_secrets,
    )
    .await;

    // 4. Deterministic state transition & circuit breakers
    let elapsed_ms = started.elapsed().as_millis() as u64;
    goal.iterations_done = iter_num;
    goal.cost_spent_usd += iter_cost;
    goal.last_verify_exit = Some(verify.exit_code);
    let excerpt = match turn_err {
        Some(ref err) if verify.output_excerpt.is_empty() => {
            format!("turn error: {err}")
        }
        _ => verify.output_excerpt.clone(),
    };
    goal.last_verify_output = Some(excerpt.clone());

    let (outcome_label, reward) = apply_verify_transition(
        &mut goal,
        verify.exit_code,
        &verify.output_hash,
    );

    // Optional auto-rewind on stall restores the clean pre-iteration workspace
    if goal.status == GoalLoopStatus::Stalled && goal.auto_rewind_on_stall
        && let Some(ref cp_id) = checkpoint_id {
            let _ = runtime
                .checkpoints
                .rewind(
                    &cfg.checkpoints,
                    &RewindRequest {
                        checkpoint_id: cp_id.clone(),
                        mode: RewindMode::CodeOnly,
                        files: Vec::new(),
                        overwrite_manual_edits: true,
                        summarize_abandoned: true,
                    },
                )
                .await;
        }

    goal.updated_at = Utc::now();
    runtime.store.upsert_goal_loop(&goal)?;

    let run = GoalLoopRun {
        id: new_id(),
        goal_id: goal.id.clone(),
        iteration: iter_num,
        checkpoint_id,
        verify_exit_code: verify.exit_code,
        verify_output_excerpt: excerpt,
        verify_hash: verify.output_hash,
        tokens_used,
        cost_usd: iter_cost,
        duration_ms: elapsed_ms,
        outcome: outcome_label.to_string(),
        reward,
        created_at: Utc::now(),
    };
    let _ = runtime.store.insert_goal_loop_run(&run);

    let tone = match goal.status {
        GoalLoopStatus::Succeeded => LogStatusTone::Ok,
        GoalLoopStatus::Active => LogStatusTone::Info,
        _ => LogStatusTone::Err,
    };
    let log_entry = emit_hono_line(
        LogDirection::Work,
        LogChannel::Cron,
        &format!(
            "goal/{} iter {}/{} -> {} (exit={})",
            goal.name,
            goal.iterations_done,
            goal.max_iterations,
            goal.status.as_str(),
            verify.exit_code
        ),
        Some((goal.status.as_str(), tone)),
        Some(&format!("{elapsed_ms}ms")),
    )
    .with_session(goal.session_id.clone());
    runtime.logs.push(log_entry);

    runtime.emit(AgentEvent::GoalLoopUpdated { goal: goal.clone() });
    runtime.emit(AgentEvent::OrchestratorUpdated {
        session_id: goal.session_id.clone(),
        stage: ORCH_STAGE_GOAL_LOOP.into(),
        tier: ExecutionTier::Standard,
        detail: format!(
            "Goal `{}` iter {}/{}: {} (verify exit {})",
            goal.name,
            goal.iterations_done,
            goal.max_iterations,
            goal.status.as_str(),
            verify.exit_code
        ),
        model: goal.model.clone(),
    });

    if goal.status == GoalLoopStatus::Succeeded {
        // Wake Dream consolidation so a multi-step victory can become a reusable draft skill!
        runtime.dream_notify.notify_one();
    }

    Ok(goal)
}

/// Deterministic state machine computing the next `GoalLoopStatus`, stall counters, and `next_run_at`.
pub fn apply_verify_transition(
    goal: &mut GoalLoopRecord,
    verify_exit_code: i32,
    verify_hash: &str,
) -> (&'static str, f64) {
    if verify_exit_code == 0 {
        goal.status = GoalLoopStatus::Succeeded;
        goal.consecutive_stalls = 0;
        goal.last_verify_hash = Some(verify_hash.to_string());
        goal.next_run_at = None;
        return ("succeeded", 1.0);
    }

    let repeated_failure = goal
        .last_verify_hash
        .as_deref()
        .is_some_and(|prev| !prev.is_empty() && prev == verify_hash);
    goal.consecutive_stalls = match repeated_failure {
        true => goal.consecutive_stalls + 1,
        false => 1,
    };
    goal.last_verify_hash = Some(verify_hash.to_string());

    if goal.max_budget_usd > 0.0 && goal.cost_spent_usd >= goal.max_budget_usd {
        goal.status = GoalLoopStatus::BudgetExhausted;
        goal.next_run_at = None;
        return ("budget_exhausted", 0.0);
    }
    if goal.consecutive_stalls >= goal.max_consecutive_stalls.max(1) {
        goal.status = GoalLoopStatus::Stalled;
        goal.next_run_at = None;
        return ("stalled", 0.0);
    }
    if goal.iterations_done >= goal.max_iterations.max(1) {
        goal.status = GoalLoopStatus::Failed;
        goal.next_run_at = None;
        return ("max_iterations", 0.0);
    }

    let delay = goal.interval_secs.max(GOAL_LOOP_MIN_BACKOFF_SECS);
    goal.status = GoalLoopStatus::Active;
    goal.next_run_at = Some(Utc::now() + ChronoDuration::seconds(delay as i64));
    ("retry", 0.0)
}

/// Executes deterministic argv-only verification command with sanitized environment, timeout,
/// bounded output capture, and Vault secret redaction.
pub async fn run_verify_argv(
    argv: &[String],
    workspace_dir: &Path,
    timeout: Duration,
    known_secrets: &[String],
) -> VerifyOutcome {
    let Some((program, args)) = argv.split_first() else {
        return VerifyOutcome {
            exit_code: -1,
            output_excerpt: "empty verify_cmd".into(),
            output_hash: hash_failure("empty verify_cmd"),
            timed_out: false,
        };
    };

    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(workspace_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env_clear();
    for key in SAFE_SHELL_ENV_KEYS {
        if let Ok(val) = std::env::var(key) {
            cmd.env(key, val);
        }
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("failed to spawn verify command `{program}`: {e}");
            return VerifyOutcome {
                exit_code: -1,
                output_hash: hash_failure(&msg),
                output_excerpt: msg,
                timed_out: false,
            };
        }
    };

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let wait_fut = async {
        let out_fut = async {
            match stdout_pipe {
                Some(r) => read_bounded_tail(r, GOAL_LOOP_VERIFY_CAPTURE_CAP_BYTES).await,
                None => Vec::new(),
            }
        };
        let err_fut = async {
            match stderr_pipe {
                Some(r) => read_bounded_tail(r, GOAL_LOOP_VERIFY_CAPTURE_CAP_BYTES).await,
                None => Vec::new(),
            }
        };
        let (stdout_bytes, stderr_bytes, status_res) =
            tokio::join!(out_fut, err_fut, child.wait());
        status_res.map(|status| (status, stdout_bytes, stderr_bytes))
    };

    match tokio::time::timeout(timeout, wait_fut).await {
        Ok(Ok((status, stdout_bytes, stderr_bytes))) => {
            let exit_code = status.code().unwrap_or(-1);
            let mut combined = String::new();
            let stdout = String::from_utf8_lossy(&stdout_bytes);
            let stderr = String::from_utf8_lossy(&stderr_bytes);
            if !stdout.trim().is_empty() {
                combined.push_str(stdout.trim());
            }
            if !stderr.trim().is_empty() {
                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str(stderr.trim());
            }
            let scrubbed = scrub_secrets(&combined, known_secrets);
            let bounded = bound_tail(&scrubbed, GOAL_LOOP_VERIFY_MAX_OUTPUT_BYTES);
            let output_hash = hash_failure(&bounded);
            VerifyOutcome {
                exit_code,
                output_excerpt: bounded,
                output_hash,
                timed_out: false,
            }
        }
        Ok(Err(e)) => {
            let msg = format!("verify command I/O error: {e}");
            VerifyOutcome {
                exit_code: -1,
                output_hash: hash_failure(&msg),
                output_excerpt: msg,
                timed_out: false,
            }
        }
        Err(_) => {
            let _ = child.kill().await;
            let msg = format!("verify command timed out after {}s", timeout.as_secs());
            VerifyOutcome {
                exit_code: GOAL_LOOP_VERIFY_TIMEOUT_EXIT_CODE,
                output_hash: hash_failure(&msg),
                output_excerpt: msg,
                timed_out: true,
            }
        }
    }
}

/// Streams a child pipe into a bounded ring tail buffer capped strictly at `cap` bytes in RAM
/// so runaway compiler/test output can never cause an OOM spike.
async fn read_bounded_tail<R: AsyncRead + Unpin>(mut reader: R, cap: usize) -> Vec<u8> {
    if cap == 0 {
        return Vec::new();
    }
    let mut ring: VecDeque<u8> = VecDeque::with_capacity(cap.min(8_192));
    let mut buf = [0u8; 4_096];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let chunk = &buf[..n];
                if chunk.len() >= cap {
                    ring.clear();
                    ring.extend(&chunk[chunk.len() - cap..]);
                } else {
                    let overflow = (ring.len() + chunk.len()).saturating_sub(cap);
                    if overflow > 0 {
                        ring.drain(..overflow);
                    }
                    ring.extend(chunk);
                }
            }
        }
    }
    ring.into_iter().collect()
}

fn build_goal_iteration_prompt(goal: &GoalLoopRecord, iter_num: usize) -> String {
    let remaining_budget = (goal.max_budget_usd - goal.cost_spent_usd).max(0.0);
    let prev_feedback = match (goal.last_verify_exit, goal.last_verify_output.as_deref()) {
        (Some(code), Some(out)) if !out.trim().is_empty() => format!(
            "\nPrevious verification (`{}`) exited with code {code}:\n```\n{}\n```",
            goal.verify_cmd.join(" "),
            bound_tail(out, GOAL_LOOP_FEEDBACK_TAIL_BYTES)
        ),
        (Some(code), _) => format!(
            "\nPrevious verification (`{}`) exited with code {code} (no output).",
            goal.verify_cmd.join(" ")
        ),
        _ => "\nThis is the first iteration; inspect the workspace and work directly toward the goal.".into(),
    };
    format!(
        "<goal_loop_context>\n\
         goal_id: {}\n\
         name: {}\n\
         iteration: {} / {}\n\
         budget_remaining_usd: {:.4}\n\
         verify_cmd: `{}`\n\
         </goal_loop_context>\n\n\
         Objective:\n{}\n\
         {prev_feedback}\n\n\
         Make concrete progress so `{}` exits with status 0. Keep changes minimal, clean, and verified.",
        goal.id,
        goal.name,
        iter_num,
        goal.max_iterations,
        remaining_budget,
        goal.verify_cmd.join(" "),
        goal.goal_text,
        goal.verify_cmd.join(" "),
    )
}

fn bound_tail(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let start = text.ceil_char_boundary(text.len().saturating_sub(max_bytes));
    format!("...[truncated]...\n{}", &text[start..])
}

fn hash_failure(text: &str) -> String {
    let normalized: String = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let mut h = Sha256::new();
    h.update(normalized.as_bytes());
    hex::encode(&h.finalize()[..GOAL_LOOP_HASH_BYTES])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_goal() -> GoalLoopRecord {
        let now = Utc::now();
        GoalLoopRecord {
            id: "goal_test01".into(),
            name: "Fix unit tests".into(),
            session_id: "sess_1".into(),
            workspace_dir: ".".into(),
            goal_text: "Make cargo test pass".into(),
            verify_cmd: vec!["true".into()],
            interval_secs: 0,
            max_iterations: 5,
            max_budget_usd: 1.0,
            max_consecutive_stalls: 2,
            checkpoint_each_iter: false,
            auto_rewind_on_stall: false,
            model: None,
            notify_channel: "webui".into(),
            status: GoalLoopStatus::Active,
            iterations_done: 1,
            consecutive_stalls: 0,
            cost_spent_usd: 0.10,
            last_verify_exit: None,
            last_verify_hash: None,
            last_verify_output: None,
            next_run_at: Some(now),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn goal_transition_handles_success_stall_and_budget_exhaustion() {
        // 1. Exit 0 -> Succeeded with reward 1.0
        let mut g1 = sample_goal();
        let (label, reward) = apply_verify_transition(&mut g1, 0, "ok");
        assert_eq!(label, "succeeded");
        assert_eq!(reward, 1.0);
        assert_eq!(g1.status, GoalLoopStatus::Succeeded);
        assert!(g1.next_run_at.is_none());

        // 2. Repeated identical failure hash trips Stalled circuit breaker
        let mut g2 = sample_goal();
        let (l1, _) = apply_verify_transition(&mut g2, 1, "hash_same");
        assert_eq!(l1, "retry");
        assert_eq!(g2.consecutive_stalls, 1);
        g2.iterations_done = 2;
        let (l2, r2) = apply_verify_transition(&mut g2, 1, "hash_same");
        assert_eq!(l2, "stalled");
        assert_eq!(r2, 0.0);
        assert_eq!(g2.status, GoalLoopStatus::Stalled);

        // 3. Budget exhaustion stops loop cleanly
        let mut g3 = sample_goal();
        g3.cost_spent_usd = 1.05;
        let (l3, _) = apply_verify_transition(&mut g3, 1, "hash_other");
        assert_eq!(l3, "budget_exhausted");
        assert_eq!(g3.status, GoalLoopStatus::BudgetExhausted);
    }

    #[tokio::test]
    async fn run_verify_argv_captures_exit_code_and_scrubs_secrets() {
        let dir = std::env::temp_dir();
        let ok = run_verify_argv(
            &["/bin/sh".into(), "-c".into(), "echo super_secret_tok".into()],
            &dir,
            Duration::from_secs(5),
            &["super_secret_tok".into()],
        )
        .await;
        assert_eq!(ok.exit_code, 0);
        assert!(!ok.output_excerpt.contains("super_secret_tok"));
        assert!(ok.output_excerpt.contains("[REDACTED]"));
    }

    #[tokio::test]
    async fn run_verify_argv_bounds_huge_stdout_and_enforces_timeout() {
        let dir = std::env::temp_dir();
        // Emit >128 KiB stdout ending with a distinct tail marker; streaming ring must keep tail bounded!
        let huge = run_verify_argv(
            &[
                "/bin/sh".into(),
                "-c".into(),
                "python3 -c 'import sys; sys.stdout.write(\"A\"*140000 + \"\\nFINAL_TAIL_MARKER\\n\")'".into(),
            ],
            &dir,
            Duration::from_secs(5),
            &[],
        )
        .await;
        assert_eq!(huge.exit_code, 0);
        assert!(huge.output_excerpt.contains("FINAL_TAIL_MARKER"));
        assert!(
            huge.output_excerpt.len() <= GOAL_LOOP_VERIFY_MAX_OUTPUT_BYTES + 64,
            "excerpt exceeded bound: {} bytes",
            huge.output_excerpt.len()
        );

        let timed = run_verify_argv(
            &["/bin/sh".into(), "-c".into(), "sleep 5".into()],
            &dir,
            Duration::from_millis(120),
            &[],
        )
        .await;
        assert!(timed.timed_out);
        assert_eq!(timed.exit_code, GOAL_LOOP_VERIFY_TIMEOUT_EXIT_CODE);
    }
}
