//! Autonomous Orchestrator & Structured Brainstorming Engine for Zenko.
//! Drives Phase-0 Intent Gate -> Token-Boundary Triage -> 5-Way Parallel Read-Only Recon ->
//! Risk-Scaled Deliberate Brainstorming (Pre-Mortem + Momus Rubric) & Kahn's DAG Planning ->
//! Adaptive Role/Effort Routing -> Mid-Turn Budget & Doom-Loop Guards -> Progress-Aware Completion Gate
//! with zero `/commands` dependency.

pub mod brainstorm;
pub mod guard;
pub mod plan;
pub mod recon;
pub mod router;
pub mod triage;

use crate::jobs::LocalJobSupervisor;
use brainstorm::ClarificationHub;
use guard::ExecutionGuard;
use recon::{format_recon_context, run_parallel_recon};
use router::{ModelRouteDecision, select_turn_model};
use std::path::PathBuf;
use std::sync::Arc;
use triage::{TriageInput, classify_turn};
use zenko_core::{
    DeliberationDepth, ExecutionTier, IntentKind, ModelRoutingRole, PermissionMode,
    ReasoningEffort, ReconSummary, TriageReport, ZenkoConfig,
};
use zenko_memory::SqliteStore;

pub use brainstorm::{
    audit_brainstorm_rubric, finalize_brainstorm_spec, resolve_headless_clarification,
    score_brainstorm_option, select_optimal_option,
};
pub use guard::{
    GuardVerdict, PhaseGateDecision, ToolCategory, classify_tool_category, detect_text_doom_loop,
    hash_canonical_json, is_read_only_shell_command, is_verification_command,
};
pub use plan::{
    audit_plan_rubric, compute_ready_session_tasks, execution_plan_from_session_tasks,
    format_plan_summary, format_session_tasks_prompt_block, incomplete_steps,
    load_latest_active_plan, plan_progress, validate_and_normalize_plan,
    validate_and_normalize_session_tasks,
};
pub use router::escalate_on_pivot;

pub struct OrchestratorTurnPlan {
    pub triage: TriageReport,
    pub route: Option<ModelRouteDecision>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub has_explicit_model_override: bool,
    pub recon: Option<ReconSummary>,
    pub guidance_block: Option<String>,
    pub guard: ExecutionGuard,
}

/// Runs pre-execution orchestration for a turn:
/// 1. Phase-0 Intent Gate + Token-boundary complexity, ambiguity & risk triage
/// 2. Adaptive multi-role model & reasoning-effort routing
/// 3. Automatic 5-way parallel read-only recon (`tokio::join!`) for `Orchestrated` runs
/// 4. Non-cache-breaking volatile `<orchestrator_plan>` tailored by `IntentKind` & `DeliberationDepth`
#[allow(clippy::too_many_arguments)]
pub async fn prepare_turn(
    cfg: &ZenkoConfig,
    session_id: &str,
    prompt: &str,
    workspace_dir: PathBuf,
    hub_dir: PathBuf,
    permission_mode: PermissionMode,
    is_first_turn: bool,
    prior_turn_errors: usize,
    opt_model_override: Option<&str>,
    session_model_override: Option<&str>,
    headless_no_wait: bool,
    store: SqliteStore,
    jobs: Arc<LocalJobSupervisor>,
) -> OrchestratorTurnPlan {
    let has_fast_model = !cfg.provider.fast_model.trim().is_empty();
    let has_plan_model = !cfg.provider.plan_model.trim().is_empty();
    let has_explicit_model_override = opt_model_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some()
        || session_model_override
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_some();

    let triage = match cfg.orchestrator.enabled {
        true => classify_turn(&TriageInput {
            prompt,
            prior_turn_errors,
            is_first_turn,
            permission_mode,
            has_fast_model,
            has_plan_model,
        }),
        false => TriageReport {
            tier: ExecutionTier::Standard,
            intent: IntentKind::Implement,
            deliberation: DeliberationDepth::Standard,
            complexity_score: 2,
            ambiguity_score: 0,
            risk_score: 0,
            matched_signals: vec!["orchestrator:disabled".into()],
            recommended_model_role: ModelRoutingRole::Lead,
            recommended_effort: ReasoningEffort::Medium,
            needs_recon: false,
            needs_brainstorm: false,
            reason: "Orchestrator disabled in config.".into(),
        },
    };

    let route = select_turn_model(
        cfg,
        opt_model_override,
        session_model_override,
        triage.recommended_model_role,
        None,
    );
    let reasoning_effort = match cfg.orchestrator.enabled && cfg.orchestrator.adaptive_effort {
        true => Some(triage.recommended_effort),
        false => None,
    };

    let recon = match cfg.orchestrator.enabled && cfg.orchestrator.auto_recon && triage.needs_recon
    {
        true => Some(
            run_parallel_recon(
                workspace_dir,
                hub_dir,
                session_id.to_string(),
                prompt,
                store,
                jobs,
            )
            .await,
        ),
        false => None,
    };

    let recon_txt_opt = recon
        .as_ref()
        .map(format_recon_context)
        .filter(|s| !s.is_empty());

    let guidance_block = match (
        cfg.orchestrator.enabled && triage.tier == ExecutionTier::Orchestrated,
        cfg.orchestrator.auto_brainstorm,
        recon_txt_opt,
    ) {
        (true, true, recon_opt) => {
            let recon_txt = recon_opt.unwrap_or_else(|| "No workspace markers detected yet.".into());
            let protocol = match (triage.intent, triage.deliberation, headless_no_wait) {
                (IntentKind::Research, _, _) => "Execution protocol (read-only research — focused budget):\n\
                 1. Inspect ONLY the directly referenced files in 1-3 focused parallel waves (`read_file`, `code_search`, `list_dir`); avoid wandering into unrelated files.\n\
                 2. Do NOT mutate workspace files on this read-only investigation turn.\n\
                 3. Once core files are read, immediately synthesize a concise, accurate answer with exact file/line evidence.",
                (IntentKind::Advisory, _, false) => "Execution protocol (read-only design advisory — focused budget):\n\
                 1. Inspect the directly relevant files in 1-3 focused parallel waves (`read_file`, `code_search`, `list_dir`) — do NOT explore unrelated subsystems.\n\
                 2. Call `record_brainstorm` with 2-3 evidence-backed options (`touched_files`, `rollback_steps`, `risks`, `pre_mortem`, `verification_plan`).\n\
                 3. Present a clear recommendation without mutating files until the user confirms.",
                (IntentKind::Advisory, _, true) => "Execution protocol (headless advisory — focused budget):\n\
                 1. Inspect the directly relevant files in 1-3 focused parallel waves — avoid wandering into unrelated subsystems.\n\
                 2. Call `record_brainstorm` with 2-3 evidence-backed options (`touched_files`, `rollback_steps`, `risks`, `pre_mortem`, `verification_plan`) and present the optimal reversible path without mutating files.",
                (_, DeliberationDepth::Deliberate, true) => "Execution protocol (HIGH-RISK DELIBERATE — headless, focused execution):\n\
                 1. Inspect affected files in 1-2 focused parallel waves BEFORE editing; prefer existing dependencies/stdlib unless new crates are explicitly requested, and avoid creating scratch `/tmp` probe projects.\n\
                 2. Call `record_brainstorm` (>=2 options, `touched_files`, `rollback_steps`, >=2 `pre_mortem`, executable `verification_plan`) AND `plan_update` right after recon.\n\
                 3. Implement the edits cleanly, update `plan_update` to `completed`, and verify with real build/test commands before concluding.",
                (_, DeliberationDepth::Deliberate, false) => "Execution protocol (HIGH-RISK DELIBERATE — phase-gated, focused execution):\n\
                 1. Inspect affected files in 1-2 focused parallel waves BEFORE editing; prefer existing dependencies/stdlib unless new crates are explicitly requested, and avoid scratch `/tmp` probe projects.\n\
                 2. Call `record_brainstorm` (>=2 options, `touched_files`, `rollback_steps`, >=2 `pre_mortem`, executable `verification_plan`) AND `plan_update`; call `ask_clarification` ONLY for genuine high-impact design forks.\n\
                 3. Close every step via `plan_update` and verify modified code with real build/test commands before concluding.",
                (_, DeliberationDepth::Standard, true) => "Execution protocol (phase-gated — headless, focused execution):\n\
                 1. Inspect relevant files in 1-2 focused parallel waves before editing.\n\
                 2. Record 2-3 reversible options via `record_brainstorm`, track concrete steps via `plan_update`, and proceed cleanly without tangential probing.\n\
                 3. Verify modified code with real build/test commands before reporting completion.",
                (_, DeliberationDepth::Standard, false) => "Execution protocol (phase-gated — focused execution):\n\
                 1. Inspect relevant files in 1-2 focused parallel waves before editing.\n\
                 2. Use `record_brainstorm` (with `touched_files`, `rollback_steps`, `risks`) and `plan_update` (DAG step list) before heavy mutations; call `ask_clarification` ONLY if a high-impact decision genuinely requires user input.\n\
                 3. Update step statuses via `plan_update` and verify modified code with real build/test commands before reporting completion.",
            };
            Some(format!(
                "<orchestrator_plan tier=\"orchestrated\" intent=\"{}\" depth=\"{}\" complexity=\"{}\" ambiguity=\"{}\" risk=\"{}\" effort=\"{}\">\n\
                 {}\n\n\
                 {}\n\
                 </orchestrator_plan>",
                triage.intent.as_str(),
                triage.deliberation.as_str(),
                triage.complexity_score,
                triage.ambiguity_score,
                triage.risk_score,
                triage.recommended_effort.as_str(),
                recon_txt,
                protocol
            ))
        }
        (true, false, Some(recon_txt)) => Some(format!(
            "<recon_context intent=\"{}\" effort=\"{}\">\n{}\n</recon_context>",
            triage.intent.as_str(),
            triage.recommended_effort.as_str(),
            recon_txt
        )),
        _ => None,
    };

    OrchestratorTurnPlan {
        guard: ExecutionGuard::new(triage.tier, triage.intent, triage.deliberation),
        triage,
        route,
        reasoning_effort,
        has_explicit_model_override,
        recon,
        guidance_block,
    }
}

pub type SharedClarificationHub = Arc<ClarificationHub>;
