//! LLM-Sovereign Autonomous Orchestrator & Iterative Brainstorming Membrane for Zenko.
//! Supplies fast 5-way parallel read-only situational awareness (`recon`), durable Kahn's DAG
//! planning (`plan`), iterative Metis/Momus brainstorming (`brainstorm`), live cognitive-gear
//! routing (`router`), and purely mechanical safety guards (`guard`) — with zero hardcoded
//! keyword rules, zero `if/else` chains, zero regex, and zero `/commands`.

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

/// Runs lightweight, non-blocking pre-turn membrane preparation:
/// 1. Language-agnostic structural profiling (zero keyword tables, zero forced blocks)
/// 2. Initial model & effort selection (LLM can dynamically shift anytime via `orchestrate_control`)
/// 3. Fast 5-way parallel read-only recon (`tokio::join!`) when structurally useful
/// 4. Neutral, non-bossy Tier-3 `<situational_awareness>` snapshot preserving full LLM sovereignty
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
    _headless_no_wait: bool,
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
            complexity_score: 0,
            ambiguity_score: 0,
            risk_score: 0,
            matched_signals: vec!["orchestrator:disabled".into()],
            recommended_model_role: ModelRoutingRole::Lead,
            recommended_effort: ReasoningEffort::Medium,
            needs_recon: false,
            needs_brainstorm: false,
            reason: "Orchestrator membrane disabled in config.".into(),
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

    let guidance_block = recon
        .as_ref()
        .map(format_recon_context)
        .filter(|s| !s.is_empty())
        .map(|recon_txt| {
            format!("<situational_awareness>\n{recon_txt}\n</situational_awareness>")
        });

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
