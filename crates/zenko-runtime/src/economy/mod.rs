//! Native Rust Token Economy Engine (inspired by Microsoft TokenEconomizer).
//! Coordinates cross-session repository Micro-Skills, experience-driven soft Tool Pruning
//! (demoting unused non-core tools to deferred while preserving 100% reachability via `invoke_tool`),
//! empirical `1.25 x P90` turn-envelope loop baselines, and the honest Net ROI Ledger.

pub mod micro_skills;

use micro_skills::{
    ShellTraceEntry, detect_contradicted_micro_skills, detect_hit_micro_skills,
    micro_skill_signature, mine_turn_shell_micro_skills,
};
use serde_json::Value;
use std::collections::BTreeSet;
use zenko_core::{
    CodeMinimalismMode, ECONOMY_PROTECTED_CORE_TOOLS, ECONOMY_SHELL_OUTPUT_PREVIEW_CHARS,
    EconomyConfig, EconomyLedgerKind, MicroSkillStatus, RepoMicroSkill, Result, TOOL_BATCH_TOOLS,
    TOOL_CODE_MINIMALISM, TOOL_INVOKE_TOOL, TOOL_SHELL_EXEC, TokenEconomyReport, ToolOutput,
    find_minimalism_mode,
};
use zenko_memory::SqliteStore;

/// Lightweight per-turn trace accumulator (bounded RAM, zero extra network/LLM calls).
#[derive(Debug, Default)]
pub struct TurnEconomyTrace {
    pub repo_scope: String,
    pub used_tools: BTreeSet<String>,
    pub shell_traces: Vec<ShellTraceEntry>,
    pub injected_skills: Vec<RepoMicroSkill>,
    pub pruned_tokens_saved_per_iter: usize,
    pub iters_executed: usize,
    pub read_less_tokens_saved: usize,
    pub output_dedup_hits: usize,
    pub output_dedup_tokens_saved: usize,
    pub ccr_recoveries: usize,
    pub loop_guard_interventions: usize,
    pub lines_added: u64,
    pub lines_deleted: u64,
    pub new_files_created: usize,
    pub bloat_hints_issued: usize,
    pub bloat_hint_emitted: bool,
}

impl TurnEconomyTrace {
    pub fn new(repo_scope: String) -> Self {
        Self {
            repo_scope,
            ..Self::default()
        }
    }

    pub fn record_tool_use(&mut self, tool_name: &str) {
        self.used_tools.insert(tool_name.to_string());
    }

    pub fn record_shell_outcome(&mut self, command: &str, is_error: bool, output: &str) {
        if self.shell_traces.len() >= 48 {
            return;
        }
        let preview: String = output.chars().take(ECONOMY_SHELL_OUTPUT_PREVIEW_CHARS).collect();
        self.shell_traces.push(ShellTraceEntry {
            command: command.to_string(),
            is_error,
            output_preview: preview,
        });
    }
}

/// Deterministically extracts tool utilization, shell fail->fix traces, and Read-Less savings
/// from any completed tool call (including unwrapped `invoke_tool` calls and `batch_tools` inner steps).
pub fn record_tool_outcome_in_economy_trace(
    trace: &mut TurnEconomyTrace,
    eff_tool_name: &str,
    eff_tool_args: &Value,
    output: &ToolOutput,
) {
    trace.record_tool_use(eff_tool_name);
    if !output.is_error
        && let Some(ref diff) = output.diff
    {
        let (added, deleted, new_files) =
            zenko_tools::minimalism::parse_unified_diff_counts(diff);
        trace.lines_added = trace.lines_added.saturating_add(added);
        trace.lines_deleted = trace.lines_deleted.saturating_add(deleted);
        trace.new_files_created = trace.new_files_created.saturating_add(new_files);
    }
    if eff_tool_name == TOOL_BATCH_TOOLS {
        if let Some(steps) = output
            .metadata
            .as_ref()
            .and_then(|m| m.get("steps"))
            .and_then(Value::as_array)
        {
            for step in steps {
                let status = step.get("status").and_then(Value::as_str).unwrap_or("");
                if status == "skipped" {
                    continue;
                }
                if let Some(inner_tool) = step.get("tool").and_then(Value::as_str) {
                    trace.record_tool_use(inner_tool);
                    if inner_tool == TOOL_SHELL_EXEC
                        && let Some(cmd) = step.get("shell_command").and_then(Value::as_str)
                    {
                        let preview = step
                            .get("output_preview")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        trace.record_shell_outcome(cmd, status == "error", preview);
                    }
                }
                if let Some(saved_tok) = step.get("read_less_tokens_saved").and_then(Value::as_u64)
                {
                    trace.read_less_tokens_saved =
                        trace.read_less_tokens_saved.saturating_add(saved_tok as usize);
                }
            }
        }
        return;
    }
    if eff_tool_name == TOOL_SHELL_EXEC
        && let Some(cmd) = eff_tool_args.get("command").and_then(Value::as_str)
    {
        trace.record_shell_outcome(cmd, output.is_error, &output.content);
    }
    if output
        .metadata
        .as_ref()
        .and_then(|m| m.get("ccr_recovery"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        trace.ccr_recoveries = trace.ccr_recoveries.saturating_add(1);
    }
    if let Some(dedup_tok) = output
        .metadata
        .as_ref()
        .and_then(|m| m.get("output_dedup"))
        .and_then(|od| od.get("tokens_saved"))
        .and_then(Value::as_u64)
    {
        trace.output_dedup_hits = trace.output_dedup_hits.saturating_add(1);
        trace.output_dedup_tokens_saved = trace
            .output_dedup_tokens_saved
            .saturating_add(dedup_tok as usize);
    } else if let Some(saved_tok) = output
        .metadata
        .as_ref()
        .and_then(|m| m.get("read_less"))
        .and_then(|rl| rl.get("tokens_saved"))
        .and_then(Value::as_u64)
    {
        trace.read_less_tokens_saved =
            trace.read_less_tokens_saved.saturating_add(saved_tok as usize);
    }
}

/// Deterministic 1-shot advisory minimalism guard driven purely by `CODE_MINIMALISM_REGISTRY`.
/// Fires at most once per turn when additive code churn or new-file creation crosses the active
/// mode's declared threshold, nudging the orchestrator to run `code_minimalism(action="audit_diff")`.
pub fn maybe_emit_bloat_hint(
    mode: CodeMinimalismMode,
    trace: &mut TurnEconomyTrace,
) -> Option<String> {
    if trace.bloat_hint_emitted || !mode.is_active() {
        return None;
    }
    let desc = find_minimalism_mode(mode);
    let net_loc = trace.lines_added.saturating_sub(trace.lines_deleted);
    let heavy_net_addition = desc.bloat_hint_net_loc > 0
        && net_loc >= desc.bloat_hint_net_loc
        && trace.lines_deleted.saturating_mul(4) < trace.lines_added;
    let heavy_new_files =
        desc.bloat_hint_new_files > 0 && trace.new_files_created >= desc.bloat_hint_new_files;

    match heavy_net_addition || heavy_new_files {
        false => None,
        true => {
            trace.bloat_hint_emitted = true;
            trace.bloat_hints_issued = trace.bloat_hints_issued.saturating_add(1);
            Some(format!(
                "Code Minimalism ({}) check: this turn added +{} / -{} LOC ({} new file(s)). Before finalizing, run `{TOOL_INVOKE_TOOL}(tool=\"{TOOL_CODE_MINIMALISM}\", args={{\"action\":\"audit_diff\"}})` (or review against the 7-rung Simplicity Ladder) to confirm no helper was duplicated, no speculative wrapper was added, and all safety checks remain intact.",
                desc.id, trace.lines_added, trace.lines_deleted, trace.new_files_created
            ))
        }
    }
}

/// Summary of deterministic post-turn economy updates emitted as telemetry when non-empty.
#[derive(Debug, Default)]
pub struct EconomyTurnUpdate {
    pub newly_pruned_tools: Vec<String>,
    pub reaudited_tools: usize,
    pub mined_micro_skills: usize,
    pub retired_micro_skills: usize,
    pub net_tokens_delta: i64,
    pub lines_added: u64,
    pub lines_deleted: u64,
}

impl EconomyTurnUpdate {
    pub fn summary_line(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.mined_micro_skills > 0 {
            parts.push(format!("+{} micro-skill evidence", self.mined_micro_skills));
        }
        if self.retired_micro_skills > 0 {
            parts.push(format!("{} retired", self.retired_micro_skills));
        }
        if !self.newly_pruned_tools.is_empty() {
            parts.push(format!(
                "soft-pruned {}",
                self.newly_pruned_tools.join(", ")
            ));
        }
        if self.reaudited_tools > 0 {
            parts.push(format!("re-audited {} tool(s)", self.reaudited_tools));
        }
        if self.net_tokens_delta > 0 {
            parts.push(format!("~{} tok saved", self.net_tokens_delta));
        }
        if self.lines_added > 0 || self.lines_deleted > 0 {
            parts.push(format!("+{} / -{} LOC", self.lines_added, self.lines_deleted));
        }
        match parts.is_empty() {
            true => None,
            false => Some(parts.join(" · ")),
        }
    }
}

/// Executes all zero-LLM post-turn Token Economy updates in SQLite (fails open on storage errors).
#[allow(clippy::too_many_arguments)]
pub fn after_turn_update(
    store: &SqliteStore,
    cfg: &EconomyConfig,
    session_id: &str,
    eager_tools: &[(String, u32)],
    trace: &TurnEconomyTrace,
    tool_calls_count: usize,
    succeeded: bool,
) -> EconomyTurnUpdate {
    if !cfg.enabled {
        return EconomyTurnUpdate::default();
    }
    let scope = &trace.repo_scope;
    let mut update = EconomyTurnUpdate::default();

    // 1. Micro-skill contradiction retirement, hit-rate tracking, and zero-LLM fail->fix mining
    if cfg.auto_micro_skills {
        let contradicted = detect_contradicted_micro_skills(&trace.injected_skills, &trace.shell_traces);
        for id in contradicted {
            if store.record_micro_skill_contradiction(&id).unwrap_or(false) {
                update.retired_micro_skills += 1;
            }
        }
        let injected_ids: Vec<String> = trace.injected_skills.iter().map(|s| s.id.clone()).collect();
        let hits = detect_hit_micro_skills(&trace.injected_skills, &trace.shell_traces);
        if let Ok(retired_low_rate) = store.record_micro_skill_exposures(&injected_ids, &hits) {
            update.retired_micro_skills += retired_low_rate;
        }
        if !injected_ids.is_empty() {
            let _ = store.record_economy_ledger_event(
                scope,
                EconomyLedgerKind::MicroSkill,
                0,
                0.0,
                &format!("injected {} active micro-skill(s)", injected_ids.len()),
            );
        }

        let mined = mine_turn_shell_micro_skills(&trace.shell_traces);
        for item in mined {
            if store
                .upsert_micro_skill_evidence(
                    scope,
                    &item.signature,
                    &item.fact,
                    &item.failed_cmd,
                    &item.fixed_cmd,
                    session_id,
                    cfg.micro_skill_min_sessions,
                )
                .is_ok()
            {
                update.mined_micro_skills += 1;
            }
        }
    }

    // 2. Per-repo tool utilization tracking + soft-demotion pruning
    if let Ok(newly_pruned) =
        store.record_turn_tool_usage(scope, session_id, eager_tools, &trace.used_tools, cfg)
    {
        update.newly_pruned_tools = newly_pruned;
    }

    // 3. Turn-length baseline tracking (for empirical 1.25x P90 loop guard) + periodic tool re-audit
    if let Ok(total_tasks) = store.record_turn_baseline(
        scope,
        tool_calls_count,
        trace.iters_executed.max(1),
        succeeded,
    ) {
        let cadence = u64::from(cfg.tool_prune_reaudit_tasks.max(1));
        if cfg.auto_tool_prune && total_tasks > 0 && total_tasks.is_multiple_of(cadence) {
            update.reaudited_tools = store.reaudit_pruned_tools(scope).unwrap_or(0);
        }
    }

    // 4. Net Economy Ledger accounting (Read-Less + Output-Dedup + Tool-Prune + Loop-Guard + CCR Backfire)
    if trace.read_less_tokens_saved > 0 {
        let _ = store.record_economy_ledger_event(
            scope,
            EconomyLedgerKind::ReadLess,
            trace.read_less_tokens_saved as i64,
            0.0,
            "Read-Less tool output compression",
        );
        update.net_tokens_delta += trace.read_less_tokens_saved as i64;
    }
    if trace.output_dedup_hits > 0 {
        for idx in 0..trace.output_dedup_hits {
            let delta = match idx == 0 {
                true => trace.output_dedup_tokens_saved as i64,
                false => 0,
            };
            let _ = store.record_economy_ledger_event(
                scope,
                EconomyLedgerKind::OutputDedup,
                delta,
                0.0,
                "Session tool output deduplication",
            );
        }
        update.net_tokens_delta += trace.output_dedup_tokens_saved as i64;
    }
    if trace.ccr_recoveries > 0 {
        for _ in 0..trace.ccr_recoveries {
            let _ = store.record_economy_ledger_event(
                scope,
                EconomyLedgerKind::CcrRecovery,
                0,
                0.0,
                "CCR artifact recovery read",
            );
        }
    }
    let total_prune_saved = trace
        .pruned_tokens_saved_per_iter
        .saturating_mul(trace.iters_executed.max(1));
    if total_prune_saved > 0 {
        let _ = store.record_economy_ledger_event(
            scope,
            EconomyLedgerKind::ToolPrune,
            total_prune_saved as i64,
            0.0,
            &format!(
                "Soft-pruned schemas across {} turn iteration(s)",
                trace.iters_executed.max(1)
            ),
        );
        update.net_tokens_delta += total_prune_saved as i64;
    }
    if trace.loop_guard_interventions > 0 {
        let _ = store.record_economy_ledger_event(
            scope,
            EconomyLedgerKind::LoopGuard,
            0,
            0.0,
            &format!(
                "{} P90 turn-envelope loop redirect(s)",
                trace.loop_guard_interventions
            ),
        );
    }

    // 5. Real per-repository code churn & minimalism metrics (zero synthetic estimates)
    if trace.lines_added > 0
        || trace.lines_deleted > 0
        || trace.new_files_created > 0
        || trace.bloat_hints_issued > 0
    {
        let _ = store.record_turn_code_metrics(
            scope,
            trace.lines_added,
            trace.lines_deleted,
            trace.new_files_created,
            trace.bloat_hints_issued,
        );
        update.lines_added = trace.lines_added;
        update.lines_deleted = trace.lines_deleted;
    }

    update
}

/// Builds a complete `TokenEconomyReport` for WebUI, CLI, and the deferred `token_economy` tool.
pub fn build_economy_report(
    store: &SqliteStore,
    cfg: &EconomyConfig,
    repo_scope: &str,
) -> Result<TokenEconomyReport> {
    let summary = store.summarize_economy_ledger(repo_scope, cfg.p90_multiplier)?;
    let code_metrics = store.get_repo_code_metrics(repo_scope)?;
    let micro_skills = store.list_repo_micro_skills(repo_scope, true)?;
    let tool_usage = store.list_repo_tool_usage(repo_scope)?;
    let pruned_tools: Vec<String> = tool_usage
        .iter()
        .filter(|t| t.pruned_to_deferred)
        .map(|t| t.tool_name.clone())
        .collect();
    let near_threshold = cfg.tool_prune_min_prompts.saturating_sub(3).max(1);
    let prune_candidates: Vec<String> = tool_usage
        .iter()
        .filter(|t| {
            !t.pruned_to_deferred
                && !t.manually_pinned
                && !ECONOMY_PROTECTED_CORE_TOOLS.contains(&t.tool_name.as_str())
                && t.prompts_since_used >= near_threshold
        })
        .map(|t| t.tool_name.clone())
        .collect();

    Ok(TokenEconomyReport {
        repo_scope: repo_scope.to_string(),
        config: cfg.clone(),
        summary,
        code_metrics,
        micro_skills,
        tool_usage,
        pruned_tools,
        prune_candidates,
    })
}

/// Helper for manual micro-skill creation using the shared deterministic FNV-1a signature.
pub fn add_manual_micro_skill(
    store: &SqliteStore,
    repo_scope: &str,
    fact: &str,
    failed_cmd: Option<&str>,
    fixed_cmd: Option<&str>,
) -> Result<RepoMicroSkill> {
    let failed = failed_cmd.unwrap_or("").trim();
    let fixed = fixed_cmd.unwrap_or("").trim();
    let sig_input = match (failed.is_empty(), fixed.is_empty()) {
        (false, false) => format!("{failed}=>{fixed}"),
        _ => fact.trim().to_string(),
    };
    let sig = micro_skill_signature(&sig_input);
    store.insert_manual_micro_skill(repo_scope, &sig, fact, failed, fixed)
}

/// Returns active corroborated micro-skills ready for Tier-2 injection.
pub fn active_micro_skills_for_repo(
    store: &SqliteStore,
    repo_scope: &str,
) -> Vec<RepoMicroSkill> {
    store
        .list_repo_micro_skills(repo_scope, false)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.status == MicroSkillStatus::Active)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn record_tool_outcome_unwraps_batch_tools_and_shell_fail_fix() {
        let mut trace = TurnEconomyTrace::new("project:test".into());
        let batch_out = ToolOutput {
            content: "Batch completed".into(),
            is_error: false,
            diff: None,
            metadata: Some(json!({
                "total": 4,
                "steps": [
                    {
                        "index": 0,
                        "tool": "web_search",
                        "status": "ok",
                        "duration_ms": 12,
                        "quiet": false
                    },
                    {
                        "index": 1,
                        "tool": "shell_exec",
                        "status": "error",
                        "duration_ms": 8,
                        "quiet": true,
                        "shell_command": "pytest -q",
                        "output_preview": "bash: pytest: command not found (exit_code: 127)"
                    },
                    {
                        "index": 2,
                        "tool": "shell_exec",
                        "status": "ok",
                        "duration_ms": 42,
                        "quiet": true,
                        "shell_command": "uv run pytest -q",
                        "output_preview": "12 passed in 0.4s",
                        "read_less_tokens_saved": 350
                    },
                    {
                        "index": 3,
                        "tool": "git_tool",
                        "status": "skipped",
                        "duration_ms": 0,
                        "quiet": false
                    }
                ]
            })),
        };

        record_tool_outcome_in_economy_trace(&mut trace, TOOL_BATCH_TOOLS, &json!({}), &batch_out);

        assert!(trace.used_tools.contains("web_search"));
        assert!(trace.used_tools.contains("shell_exec"));
        assert!(!trace.used_tools.contains("git_tool"), "skipped step must not count as used");
        assert_eq!(trace.read_less_tokens_saved, 350);
        assert_eq!(trace.shell_traces.len(), 2);

        let mined = mine_turn_shell_micro_skills(&trace.shell_traces);
        assert_eq!(mined.len(), 1);
        assert!(mined[0].fact.contains("uv run pytest"));
    }

    #[test]
    fn diff_loc_tracking_and_one_shot_bloat_hint_respect_minimalism_mode() {
        let mut trace = TurnEconomyTrace::new("project:test".into());
        let mut big_diff = String::from("diff --git a/src/a.rs b/src/a.rs\n--- /dev/null\n+++ b/src/a.rs\n@@ -0,0 +1,80 @@\n");
        for i in 0..80 {
            big_diff.push_str(&format!("+let line_{i} = {i};\n"));
        }
        let edit_out = ToolOutput {
            content: "wrote src/a.rs".into(),
            is_error: false,
            diff: Some(big_diff),
            metadata: None,
        };
        record_tool_outcome_in_economy_trace(&mut trace, "write_file", &json!({}), &edit_out);
        assert_eq!(trace.lines_added, 80);
        assert_eq!(trace.lines_deleted, 0);
        assert_eq!(trace.new_files_created, 1);

        // Off and Full (threshold 140) stay silent at +80 LOC; Ultra (threshold 70) fires once!
        assert!(maybe_emit_bloat_hint(CodeMinimalismMode::Off, &mut trace).is_none());
        assert!(maybe_emit_bloat_hint(CodeMinimalismMode::Full, &mut trace).is_none());
        let first = maybe_emit_bloat_hint(CodeMinimalismMode::Ultra, &mut trace).expect("ultra hint");
        assert!(first.contains(TOOL_CODE_MINIMALISM));
        assert!(first.contains("+80 / -0 LOC"));
        assert_eq!(trace.bloat_hints_issued, 1);
        assert!(
            maybe_emit_bloat_hint(CodeMinimalismMode::Ultra, &mut trace).is_none(),
            "bloat hint must be strictly 1-shot per turn"
        );
    }

    #[test]
    fn output_dedup_and_ccr_recovery_telemetry_persists_in_sqlite_ledger() {
        let db_path = std::env::temp_dir().join(format!("zenko_econ_chisle_{}.db", zenko_core::new_id()));
        let store = SqliteStore::open(&db_path).expect("open db");
        let cfg = EconomyConfig::default();
        let mut trace = TurnEconomyTrace::new("project:chisle".into());

        let dedup_out = ToolOutput {
            content: "[dedup: identical]".into(),
            is_error: false,
            diff: None,
            metadata: Some(json!({
                "output_dedup": { "calls_ago": 2, "tokens_saved": 420 },
                "read_less": { "shape": "dedup", "tokens_saved": 420 }
            })),
        };
        record_tool_outcome_in_economy_trace(&mut trace, TOOL_SHELL_EXEC, &json!({}), &dedup_out);

        let ccr_out = ToolOutput {
            content: "1:aaaa| raw artifact slice".into(),
            is_error: false,
            diff: None,
            metadata: Some(json!({ "ccr_recovery": true })),
        };
        record_tool_outcome_in_economy_trace(&mut trace, "read_file", &json!({}), &ccr_out);

        assert_eq!(trace.output_dedup_hits, 1);
        assert_eq!(trace.output_dedup_tokens_saved, 420);
        assert_eq!(trace.read_less_tokens_saved, 0, "must not double-count dedup into read_less");
        assert_eq!(trace.ccr_recoveries, 1);

        let upd = after_turn_update(&store, &cfg, "sess_c1", &[], &trace, 2, true);
        assert_eq!(upd.net_tokens_delta, 420);

        let sum = store
            .summarize_economy_ledger("project:chisle", cfg.p90_multiplier)
            .expect("summary");
        assert_eq!(sum.output_dedup_hits, 1);
        assert_eq!(sum.output_dedup_tokens_saved, 420);
        assert_eq!(sum.ccr_recoveries, 1);
        assert_eq!(sum.net_tokens_saved, 420);

        let _ = std::fs::remove_file(&db_path);
    }
}
