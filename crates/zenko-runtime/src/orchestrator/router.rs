//! Adaptive Multi-Role Model & Effort Router with mid-turn circuit-breaker escalation.
//! Zero hardcoded model IDs, zero `if/else` ladders — pure `match` & declarative precedence.

use zenko_core::{ModelRoutingRole, ReasoningEffort, ZenkoConfig};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRouteDecision {
    pub model_id: String,
    pub source: &'static str,
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|v| !v.is_empty())
}

/// Resolves the target model for a turn in strict precedence order:
/// 1. Explicit per-call override (`--model` / API parameter)
/// 2. Explicit per-session override (`session.model_override`)
/// 3. Adaptive role slot (`fast_model` / `plan_model` / `advisor_model`) when enabled
/// 4. Configured `provider.lead_model`
/// 5. Live-discovered fallback model (if configured model is blank)
pub fn select_turn_model(
    cfg: &ZenkoConfig,
    opt_override: Option<&str>,
    session_override: Option<&str>,
    role: ModelRoutingRole,
    discovered_fallback: Option<&str>,
) -> Option<ModelRouteDecision> {
    if let Some(m) = non_empty(opt_override) {
        return Some(ModelRouteDecision {
            model_id: m.to_string(),
            source: "turn_override",
        });
    }
    if let Some(m) = non_empty(session_override) {
        return Some(ModelRouteDecision {
            model_id: m.to_string(),
            source: "session_override",
        });
    }

    if cfg.orchestrator.enabled && cfg.orchestrator.adaptive_model_routing {
        let adaptive = match role {
            ModelRoutingRole::Fast => {
                non_empty(Some(&cfg.provider.fast_model)).map(|m| (m, "adaptive_fast"))
            }
            ModelRoutingRole::Plan => {
                non_empty(Some(&cfg.provider.plan_model)).map(|m| (m, "adaptive_plan"))
            }
            ModelRoutingRole::Advisor => {
                non_empty(Some(&cfg.provider.advisor_model)).map(|m| (m, "adaptive_advisor"))
            }
            ModelRoutingRole::Lead => None,
        };
        if let Some((model_id, source)) = adaptive {
            return Some(ModelRouteDecision {
                model_id: model_id.to_string(),
                source,
            });
        }
    }

    if let Some(lead) = non_empty(Some(&cfg.provider.lead_model)) {
        return Some(ModelRouteDecision {
            model_id: lead.to_string(),
            source: "lead_model",
        });
    }

    non_empty(discovered_fallback).map(|m| ModelRouteDecision {
        model_id: m.to_string(),
        source: "discovered",
    })
}

/// Computes deterministic ascending capability rank across configured model slots
/// (`Fast = 0 < Lead = 1 < Plan = 2 < Advisor = 3`) so escalation NEVER silently downgrades.
pub fn model_slot_rank(cfg: &ZenkoConfig, model_id: &str) -> u8 {
    let cur = model_id.trim();
    let advisor = cfg.provider.advisor_model.trim();
    let plan = cfg.provider.plan_model.trim();
    let lead = cfg.provider.lead_model.trim();
    let fast = cfg.provider.fast_model.trim();
    match () {
        _ if !advisor.is_empty() && cur == advisor => 3,
        _ if !plan.is_empty() && cur == plan => 2,
        _ if !lead.is_empty() && cur == lead => 1,
        _ if !fast.is_empty() && cur == fast => 0,
        _ => 1,
    }
}

/// Computes an escalated `(model_id, effort, source)` mid-turn when the circuit breaker trips:
/// - Always escalates `ReasoningEffort` (`Low -> Medium -> High`) when `adaptive_effort` is on.
/// - Switches model ONLY strictly upward (`Fast -> Lead -> Plan -> Advisor`) when no explicit
///   override locked the turn; never downgrades `Plan`/`Advisor` back to `Lead` or `Fast`.
pub fn escalate_on_pivot(
    cfg: &ZenkoConfig,
    current_model: &str,
    current_effort: Option<ReasoningEffort>,
    has_explicit_override: bool,
) -> (String, Option<ReasoningEffort>, &'static str) {
    let next_effort = match (cfg.orchestrator.adaptive_effort, current_effort) {
        (true, Some(e)) => Some(e.escalated()),
        (true, None) => Some(ReasoningEffort::High),
        (false, existing) => existing,
    };

    if has_explicit_override || !cfg.orchestrator.adaptive_model_routing {
        return (current_model.to_string(), next_effort, "effort_escalated");
    }

    let cur_rank = model_slot_rank(cfg, current_model);
    let candidates: [(u8, &str, &'static str); 3] = [
        (3, cfg.provider.advisor_model.trim(), "escalated_advisor"),
        (2, cfg.provider.plan_model.trim(), "escalated_plan"),
        (1, cfg.provider.lead_model.trim(), "escalated_lead"),
    ];
    for (rank, cand, src) in candidates {
        if rank > cur_rank && !cand.is_empty() && cand != current_model {
            return (cand.to_string(), next_effort, src);
        }
    }

    if let Some(fb) = cfg
        .provider
        .fallbacks
        .iter()
        .map(|s| s.trim())
        .find(|s| !s.is_empty() && *s != current_model && model_slot_rank(cfg, s) >= cur_rank)
    {
        return (fb.to_string(), next_effort, "escalated_fallback");
    }

    (current_model.to_string(), next_effort, "effort_escalated")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_router_honors_overrides_roles_and_monotonic_pivot_escalation() {
        let mut cfg = ZenkoConfig::default();
        cfg.provider.lead_model = "anthropic/claude-sonnet-4".into();
        cfg.provider.fast_model = "google/gemini-2.5-flash".into();
        cfg.provider.plan_model = "anthropic/claude-opus-4".into();
        cfg.provider.advisor_model = "openai/o3".into();

        let fast = select_turn_model(&cfg, None, None, ModelRoutingRole::Fast, None)
            .expect("fast route");
        assert_eq!(fast.model_id, "google/gemini-2.5-flash");
        assert_eq!(fast.source, "adaptive_fast");

        let plan = select_turn_model(&cfg, None, None, ModelRoutingRole::Plan, None)
            .expect("plan route");
        assert_eq!(plan.model_id, "anthropic/claude-opus-4");
        assert_eq!(plan.source, "adaptive_plan");

        let lead = select_turn_model(&cfg, None, None, ModelRoutingRole::Lead, None)
            .expect("lead route");
        assert_eq!(lead.model_id, "anthropic/claude-sonnet-4");
        assert_eq!(lead.source, "lead_model");

        let (esc_model, esc_effort, esc_src) = escalate_on_pivot(
            &cfg,
            &fast.model_id,
            Some(ReasoningEffort::Low),
            false,
        );
        assert_eq!(esc_model, "openai/o3");
        assert_eq!(esc_effort, Some(ReasoningEffort::Medium));
        assert_eq!(esc_src, "escalated_advisor");

        // Already on Advisor -> MUST NOT downgrade back to Lead or Plan!
        let (stay_adv, adv_eff, adv_src) = escalate_on_pivot(
            &cfg,
            "openai/o3",
            Some(ReasoningEffort::Medium),
            false,
        );
        assert_eq!(stay_adv, "openai/o3");
        assert_eq!(adv_eff, Some(ReasoningEffort::High));
        assert_eq!(adv_src, "effort_escalated");

        // On Plan with no Advisor configured -> MUST NOT downgrade to Lead!
        let mut no_adv = cfg.clone();
        no_adv.provider.advisor_model.clear();
        let (stay_plan, plan_eff, plan_src) = escalate_on_pivot(
            &no_adv,
            "anthropic/claude-opus-4",
            Some(ReasoningEffort::Medium),
            false,
        );
        assert_eq!(stay_plan, "anthropic/claude-opus-4");
        assert_eq!(plan_eff, Some(ReasoningEffort::High));
        assert_eq!(plan_src, "effort_escalated");

        // On Lead with Plan configured (and no Advisor) -> escalates cleanly to Plan!
        let (lead_to_plan, _, ltp_src) = escalate_on_pivot(
            &no_adv,
            "anthropic/claude-sonnet-4",
            Some(ReasoningEffort::Medium),
            false,
        );
        assert_eq!(lead_to_plan, "anthropic/claude-opus-4");
        assert_eq!(ltp_src, "escalated_plan");

        let override_res = select_turn_model(
            &cfg,
            Some("custom/model-x"),
            Some("session/model-y"),
            ModelRoutingRole::Fast,
            None,
        )
        .expect("override");
        assert_eq!(override_res.model_id, "custom/model-x");
        assert_eq!(override_res.source, "turn_override");

        cfg.orchestrator.adaptive_model_routing = false;
        let disabled = select_turn_model(&cfg, None, None, ModelRoutingRole::Fast, None)
            .expect("disabled adaptive");
        assert_eq!(disabled.model_id, "anthropic/claude-sonnet-4");
        assert_eq!(disabled.source, "lead_model");
    }
}
