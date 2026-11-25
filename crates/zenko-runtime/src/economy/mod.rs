//! Native Rust Token Economy Engine (inspired by Microsoft TokenEconomizer).
//! Coordinates cross-session repository Micro-Skills, experience-driven soft Tool Pruning
//! (demoting unused non-core tools to deferred while preserving 100% reachability via `invoke_tool`),
//! empirical `1.25 x P90` turn-envelope loop baselines, and the honest Net ROI Ledger.

pub mod micro_skills;

use micro_skills::{
    ShellTraceEntry, detect_contradicted_micro_skills, detect_hit_micro_skills,
    micro_skill_signature, mine_turn_shell_micro_skills,
};
use std::collections::BTreeSet;
use zenko_core::{
    ECONOMY_PROTECTED_CORE_TOOLS, EconomyConfig, EconomyLedgerKind, MicroSkillStatus,
    RepoMicroSkill, Result, TokenEconomyReport,
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
    pub loop_guard_interventions: usize,
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
        let preview: String = output.chars().take(280).collect();
        self.shell_traces.push(ShellTraceEntry {
            command: command.to_string(),
            is_error,
            output_preview: preview,
        });
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

    // 4. Net Economy Ledger accounting (Read-Less + Tool-Prune schema savings + Loop-Guard saves)
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

    update
}

/// Builds a complete `TokenEconomyReport` for WebUI, CLI, and the deferred `token_economy` tool.
pub fn build_economy_report(
    store: &SqliteStore,
    cfg: &EconomyConfig,
    repo_scope: &str,
) -> Result<TokenEconomyReport> {
    let summary = store.summarize_economy_ledger(repo_scope, cfg.p90_multiplier)?;
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
