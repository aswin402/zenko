//! First-class Autonomous Orchestrator tools (zero hardcoding, zero `/commands`):
//! - `record_brainstorm`: persists an evidence-weighted multi-option `BrainstormSpec` artifact.
//! - `plan_update`: validates a cycle-free DAG `ExecutionPlan` checklist (Kahn's topological sort)
//!   and streams live step progress to WebUI/CLI.
//! - `ask_clarification`: surfaces a typed multiple-choice decision card in WebUI/CLI/Telegram,
//!   or deterministically auto-selects the recommended option in headless 24/7 mode.

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    ASK_USER_KINDS, ASK_USER_MAX_FIELDS, ASK_USER_MAX_OPTIONS, ASK_USER_MAX_PROS_CONS,
    ASK_USER_MAX_STEPS, ASK_USER_OPTION_ACCEPT, ASK_USER_SELECTION_MODES, AskUserField,
    AskUserKind, AskUserRecommendation, AskUserRequestSpec, AskUserSelectionMode, AskUserStep,
    BRAINSTORM_STAGES, BrainstormOption, BrainstormSpec, BrainstormStage, CODE_MINIMALISM_MODES,
    CONCISE_MODES, ClarificationOption, CodeMinimalismMode, ConciseMode, ExecutionPlan,
    MODEL_ROUTING_ROLES, ModelRoutingRole, ORCH_BRAINSTORM_MAX_OPTIONS, ORCH_DEFAULT_OPTION_SCORE,
    ORCH_PLAN_MAX_STEPS, OrchestrateControlRequest, OrchestratorBridge, PlanStep, PlanStepStatus,
    REASONING_EFFORTS, ReasoningEffort, ResourceLockKey, Result, RiskLevel, SESSION_TASK_ACTIONS,
    SESSION_TASK_DAG_LOCK_KEY, SESSION_TASK_STATUSES, SessionTask, SessionTaskAction,
    SessionTaskPatch, SessionTaskStatus, TOOL_ASK_USER, TOOL_ORCHESTRATE_CONTROL,
    TOOL_PLAN_UPDATE, TOOL_RECORD_BRAINSTORM, TOOL_SESSION_TASK, TURN_POSTURES, Tool,
    ToolConcurrency, ToolContext, ToolOutput, ToolSpec, TurnPosture, ZenkoError, new_id,
    new_session_task_id, scrub_secrets,
};

pub struct RecordBrainstormTool {
    bridge: Arc<dyn OrchestratorBridge>,
}

impl RecordBrainstormTool {
    pub fn new(bridge: Arc<dyn OrchestratorBridge>) -> Self {
        Self { bridge }
    }
}

fn parse_string_vec(val: Option<&Value>) -> Vec<String> {
    val.and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

#[async_trait]
impl Tool for RecordBrainstormTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Orchestration
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Global
    }

    fn exposure(&self) -> zenko_core::ToolExposure {
        zenko_core::ToolExposure::Deferred
    }

    fn spec(&self) -> ToolSpec {
        let stage_enum: Vec<Value> = BRAINSTORM_STAGES
            .iter()
            .map(|&s| Value::String(s.to_string()))
            .collect();
        ToolSpec {
            name: TOOL_RECORD_BRAINSTORM.into(),
            description: "Record or iteratively evolve an LLM-sovereign architectural brainstorm (`exploring` -> `refined` -> `committed`) with assumptions, blind spots, decision criteria, candidate options, adversarial self-critique, and optional verification plan. Leave `selected_option_id` empty while still exploring or awaiting user input.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "stage": {
                        "type": "string",
                        "enum": stage_enum,
                        "description": "Brainstorm lifecycle stage: `exploring` (diverging/open), `refined` (incorporating feedback/evidence), or `committed` (chosen direction)."
                    },
                    "parent_spec_id": { "type": "string", "description": "Optional ID of an earlier brainstorm being refined or superseded." },
                    "problem": { "type": "string", "description": "Concise problem framing grounded in inspected context." },
                    "invariants": { "type": "array", "items": { "type": "string" }, "description": "Hard constraints and invariants that must hold." },
                    "assumptions": { "type": "array", "items": { "type": "string" }, "description": "Explicit premises being assumed (surface so they can be challenged)." },
                    "blind_spots": { "type": "array", "items": { "type": "string" }, "description": "Identified unknowns, edge cases, or missing requirements." },
                    "decision_criteria": { "type": "array", "items": { "type": "string" }, "description": "Problem-specific criteria guiding option selection." },
                    "options": {
                        "type": "array",
                        "description": "Candidate approaches with tradeoffs and structural evidence.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "title": { "type": "string" },
                                "summary": { "type": "string" },
                                "pros": { "type": "array", "items": { "type": "string" } },
                                "cons": { "type": "array", "items": { "type": "string" } },
                                "blast_radius": { "type": "string", "enum": ["low", "medium", "high"] },
                                "reversibility": { "type": "string", "enum": ["high", "medium", "low"] },
                                "safety_score": { "type": "integer", "minimum": 1, "maximum": 10 },
                                "testability_score": { "type": "integer", "minimum": 1, "maximum": 10 },
                                "touched_files": { "type": "array", "items": { "type": "string" } },
                                "rollback_steps": { "type": "array", "items": { "type": "string" } },
                                "risks": { "type": "array", "items": { "type": "string" } },
                                "recommended": { "type": "boolean" }
                            },
                            "required": ["id", "title", "summary"]
                        }
                    },
                    "selected_option_id": { "type": "string", "description": "Option ID chosen by you (optional while `stage` is `exploring`)." },
                    "selection_rationale": { "type": "string", "description": "Why this direction fits the problem and criteria." },
                    "critique": { "type": "array", "items": { "type": "string" }, "description": "Adversarial counterarguments or sharpest objections against the leading option." },
                    "open_questions": { "type": "array", "items": { "type": "string" } },
                    "pre_mortem": { "type": "array", "items": { "type": "string" }, "description": "Concrete failure modes and mitigations if this path goes wrong." },
                    "verification_plan": { "type": "array", "items": { "type": "string" }, "description": "Checks or experiments that will confirm success." }
                },
                "required": ["problem", "options"]
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
        let problem = args
            .get("problem")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ZenkoError::Tool("missing `problem`".into()))?
            .to_string();

        let raw_opts = args
            .get("options")
            .and_then(Value::as_array)
            .ok_or_else(|| ZenkoError::Tool("missing `options` array".into()))?;

        let options: Vec<BrainstormOption> = raw_opts
            .iter()
            .take(ORCH_BRAINSTORM_MAX_OPTIONS)
            .enumerate()
            .filter_map(|(idx, v)| {
                let title = v.get("title").and_then(Value::as_str)?.trim().to_string();
                let summary = v
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let id = v
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .unwrap_or_else(|| format!("opt_{}", idx + 1));
                Some(BrainstormOption {
                    id,
                    title,
                    summary,
                    pros: parse_string_vec(v.get("pros")),
                    cons: parse_string_vec(v.get("cons")),
                    blast_radius: v
                        .get("blast_radius")
                        .and_then(Value::as_str)
                        .unwrap_or("medium")
                        .to_string(),
                    reversibility: v
                        .get("reversibility")
                        .and_then(Value::as_str)
                        .unwrap_or("high")
                        .to_string(),
                    safety_score: v
                        .get("safety_score")
                        .and_then(Value::as_u64)
                        .unwrap_or(u64::from(ORCH_DEFAULT_OPTION_SCORE))
                        .clamp(1, 10) as u8,
                    testability_score: v
                        .get("testability_score")
                        .and_then(Value::as_u64)
                        .unwrap_or(u64::from(ORCH_DEFAULT_OPTION_SCORE))
                        .clamp(1, 10) as u8,
                    touched_files: parse_string_vec(v.get("touched_files")),
                    rollback_steps: parse_string_vec(v.get("rollback_steps")),
                    risks: parse_string_vec(v.get("risks")),
                    recommended: v
                        .get("recommended")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
            })
            .collect();

        if options.is_empty() {
            return Ok(ToolOutput::err(
                "provide at least 1 valid option with `id`, `title`, and `summary`",
            ));
        }

        let selected_option_id = args
            .get("selected_option_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let stage = args
            .get("stage")
            .and_then(Value::as_str)
            .and_then(BrainstormStage::parse)
            .unwrap_or_else(|| match selected_option_id.is_empty() {
                true => BrainstormStage::Exploring,
                false => BrainstormStage::Committed,
            });
        let parent_spec_id = args
            .get("parent_spec_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);

        let spec = BrainstormSpec {
            id: new_id(),
            stage,
            parent_spec_id,
            problem,
            invariants: parse_string_vec(args.get("invariants")),
            assumptions: parse_string_vec(args.get("assumptions")),
            blind_spots: parse_string_vec(args.get("blind_spots")),
            decision_criteria: parse_string_vec(args.get("decision_criteria")),
            options,
            selected_option_id,
            selection_rationale: args
                .get("selection_rationale")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string(),
            critique: parse_string_vec(args.get("critique")),
            open_questions: parse_string_vec(args.get("open_questions")),
            pre_mortem: parse_string_vec(args.get("pre_mortem")),
            verification_plan: parse_string_vec(args.get("verification_plan")),
            rubric_warnings: Vec::new(),
            created_at: Utc::now(),
        };

        let finalized = self.bridge.record_brainstorm(&ctx.session_id, &spec)?;
        let selected_label = match finalized.selected_option_id.is_empty() {
            true => "open".to_string(),
            false => format!("`{}`", finalized.selected_option_id),
        };
        let rubric_note = match finalized.rubric_warnings.is_empty() {
            true => String::new(),
            false => format!(
                "\nAdvisory reflections: {}",
                finalized.rubric_warnings.join(" | ")
            ),
        };
        Ok(ToolOutput {
            content: format!(
                "Recorded brainstorm `{}` [stage={}, {} options, selected={selected_label}].{rubric_note}",
                finalized.id,
                finalized.stage.as_str(),
                finalized.options.len(),
            ),
            is_error: false,
            diff: None,
            metadata: Some(serde_json::to_value(&finalized).unwrap_or(Value::Null)),
        })
    }
}

pub struct PlanUpdateTool {
    bridge: Arc<dyn OrchestratorBridge>,
}

impl PlanUpdateTool {
    pub fn new(bridge: Arc<dyn OrchestratorBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for PlanUpdateTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Orchestration
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Global
    }

    fn exposure(&self) -> zenko_core::ToolExposure {
        zenko_core::ToolExposure::Deferred
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_PLAN_UPDATE.into(),
            description: "Create or update the live DAG execution step checklist for multi-step work (validates acyclic dependencies, renders live progress in WebUI/CLI, and tracks completion).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "goal": { "type": "string", "description": "Concise objective for the execution plan" },
                    "steps": {
                        "type": "array",
                        "description": "Ordered list of atomic steps with status and optional depends_on step IDs",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "title": { "type": "string" },
                                "status": { "type": "string", "enum": ["pending", "in_progress", "completed", "blocked"] },
                                "depends_on": { "type": "array", "items": { "type": "string" } },
                                "verification": { "type": "string" }
                            },
                            "required": ["id", "title"]
                        }
                    }
                },
                "required": ["goal", "steps"]
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
        let goal = args
            .get("goal")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ZenkoError::Tool("missing `goal`".into()))?
            .to_string();

        let raw_steps = args
            .get("steps")
            .and_then(Value::as_array)
            .ok_or_else(|| ZenkoError::Tool("missing `steps` array".into()))?;

        let steps: Vec<PlanStep> = raw_steps
            .iter()
            .take(ORCH_PLAN_MAX_STEPS)
            .enumerate()
            .filter_map(|(idx, v)| {
                let title = v.get("title").and_then(Value::as_str)?.trim().to_string();
                if title.is_empty() {
                    return None;
                }
                let id = v
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .unwrap_or_else(|| format!("step_{}", idx + 1));
                let status = v
                    .get("status")
                    .and_then(Value::as_str)
                    .map(PlanStepStatus::from_str_loose)
                    .unwrap_or(PlanStepStatus::Pending);
                let verification = v
                    .get("verification")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                Some(PlanStep {
                    id,
                    title,
                    status,
                    depends_on: parse_string_vec(v.get("depends_on")),
                    verification,
                })
            })
            .collect();

        if steps.is_empty() {
            return Ok(ToolOutput::err(
                "provide at least 1 valid plan step with `id` and `title`",
            ));
        }

        let plan = ExecutionPlan {
            id: new_id(),
            goal,
            steps,
            rubric_warnings: Vec::new(),
            updated_at: Utc::now(),
        };

        let validated = match self.bridge.update_plan(&ctx.session_id, &plan) {
            Ok(p) => p,
            Err(e) => return Ok(ToolOutput::err(e.to_string())),
        };
        let done = validated
            .steps
            .iter()
            .filter(|s| s.status == PlanStepStatus::Completed)
            .count();
        let rubric_note = match validated.rubric_warnings.is_empty() {
            true => String::new(),
            false => format!(
                "\nRubric checks: {}",
                validated.rubric_warnings.join(" | ")
            ),
        };
        Ok(ToolOutput {
            content: format!(
                "Updated DAG execution plan `{}` ({done}/{} steps completed).{rubric_note}",
                validated.id,
                validated.steps.len()
            ),
            is_error: false,
            diff: None,
            metadata: Some(serde_json::to_value(&validated).unwrap_or(Value::Null)),
        })
    }
}

pub struct AskUserTool {
    bridge: Arc<dyn OrchestratorBridge>,
}

/// Backwards-compatible alias for `AskUserTool`.
pub type AskClarificationTool = AskUserTool;

/// LLM-sovereign live orchestration tool (`orchestrate_control`): lets Zenko declare/transition
/// turn posture, dynamically shift cognitive gear (model slot + reasoning effort) mid-turn,
/// or refresh 5-way parallel workspace/git/memory/DAG recon on demand with zero `/commands`.
pub struct OrchestrateControlTool {
    bridge: Arc<dyn OrchestratorBridge>,
}

impl OrchestrateControlTool {
    pub fn new(bridge: Arc<dyn OrchestratorBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for OrchestrateControlTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Orchestration
    }

    fn is_available(&self, ctx: &zenko_core::ToolAvailabilityContext) -> bool {
        !ctx.is_subagent
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Global
    }

    fn exposure(&self) -> zenko_core::ToolExposure {
        zenko_core::ToolExposure::Deferred
    }

    fn spec(&self) -> ToolSpec {
        let posture_enum: Vec<Value> = TURN_POSTURES
            .iter()
            .map(|&p| Value::String(p.to_string()))
            .collect();
        let tier_enum: Vec<Value> = MODEL_ROUTING_ROLES
            .iter()
            .map(|&r| Value::String(r.to_string()))
            .collect();
        let effort_enum: Vec<Value> = REASONING_EFFORTS
            .iter()
            .map(|&e| Value::String(e.to_string()))
            .collect();
        let concise_enum: Vec<Value> = CONCISE_MODES
            .iter()
            .map(|&m| Value::String(m.to_string()))
            .collect();
        let minimalism_enum: Vec<Value> = CODE_MINIMALISM_MODES
            .iter()
            .map(|&m| Value::String(m.to_string()))
            .collect();
        let zmode_enum: Vec<Value> = zenko_core::ZMODES
            .iter()
            .map(|&m| Value::String(m.to_string()))
            .collect();
        ToolSpec {
            name: TOOL_ORCHESTRATE_CONTROL.into(),
            description: "Exercise sovereign control over your own turn orchestration (zero /commands): declare or transition your turn posture, dynamically shift cognitive gear (configured model tier & reasoning effort), switch unified ZMode (`lite`, `balanced`, `high`, `ultra`), adjust live English concise mode or 7-rung code minimalism mode, or refresh live 5-way parallel workspace/git/memory/DAG recon on demand.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "posture": {
                        "type": "string",
                        "enum": posture_enum,
                        "description": "Self-declared turn posture streamed live to WebUI/CLI."
                    },
                    "cognitive_tier": {
                        "type": "string",
                        "enum": tier_enum,
                        "description": "Configured model slot for subsequent iterations (`fast`, `lead`, `plan`, `advisor`; respects explicit user model locks)."
                    },
                    "reasoning_effort": {
                        "type": "string",
                        "enum": effort_enum,
                        "description": "Reasoning depth for subsequent iterations (`low`, `medium`, `high`)."
                    },
                    "zmode": {
                        "type": "string",
                        "enum": zmode_enum,
                        "description": "Unified Zenko context & efficiency profile (`lite`, `balanced`, `high`, `ultra`) synchronizing prose conciseness, 7-rung code minimalism, and compression."
                    },
                    "concise_mode": {
                        "type": "string",
                        "enum": concise_enum,
                        "description": "Live English output conciseness tier for this session (`off`, `lite`, `full`, `ultra`)."
                    },
                    "minimalism_mode": {
                        "type": "string",
                        "enum": minimalism_enum,
                        "description": "Live 7-rung Code Minimalism tier for this session (`off`, `lite`, `full`, `ultra`)."
                    },
                    "refresh_recon": {
                        "type": "boolean",
                        "description": "When true, runs a fresh parallel workspace/git/memory/task recon snapshot and returns it."
                    },
                    "recon_query": {
                        "type": "string",
                        "description": "Optional focused search terms for memory/session recall when `refresh_recon` is true."
                    },
                    "rationale": {
                        "type": "string",
                        "description": "Concise 1-line rationale recorded in session trace."
                    }
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
        let posture = match args.get("posture").and_then(Value::as_str) {
            Some(raw) => Some(TurnPosture::parse(raw).ok_or_else(|| {
                ZenkoError::Tool(format!(
                    "Unknown `posture` `{raw}`. Valid values: {}.",
                    TURN_POSTURES.join(", ")
                ))
            })?),
            None => None,
        };
        let cognitive_tier = match args.get("cognitive_tier").and_then(Value::as_str) {
            Some(raw) => Some(ModelRoutingRole::parse(raw).ok_or_else(|| {
                ZenkoError::Tool(format!(
                    "Unknown `cognitive_tier` `{raw}`. Valid values: {}.",
                    MODEL_ROUTING_ROLES.join(", ")
                ))
            })?),
            None => None,
        };
        let reasoning_effort = match args.get("reasoning_effort").and_then(Value::as_str) {
            Some(raw) => Some(ReasoningEffort::parse(raw).ok_or_else(|| {
                ZenkoError::Tool(format!(
                    "Unknown `reasoning_effort` `{raw}`. Valid values: {}.",
                    REASONING_EFFORTS.join(", ")
                ))
            })?),
            None => None,
        };
        let concise_mode = match args.get("concise_mode").and_then(Value::as_str) {
            Some(raw) => Some(ConciseMode::parse(raw).ok_or_else(|| {
                ZenkoError::Tool(format!(
                    "Unknown `concise_mode` `{raw}`. Valid values: {}.",
                    CONCISE_MODES.join(", ")
                ))
            })?),
            None => None,
        };
        let minimalism_mode = match args.get("minimalism_mode").and_then(Value::as_str) {
            Some(raw) => Some(CodeMinimalismMode::parse(raw).ok_or_else(|| {
                ZenkoError::Tool(format!(
                    "Unknown `minimalism_mode` `{raw}`. Valid values: {}.",
                    CODE_MINIMALISM_MODES.join(", ")
                ))
            })?),
            None => None,
        };
        let zmode = match args.get("zmode").and_then(Value::as_str) {
            Some(raw) => Some(zenko_core::ZMode::parse(raw).ok_or_else(|| {
                ZenkoError::Tool(format!(
                    "Unknown `zmode` `{raw}`. Valid values: {}.",
                    zenko_core::ZMODES.join(", ")
                ))
            })?),
            None => None,
        };
        let refresh_recon = args
            .get("refresh_recon")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let recon_query = args
            .get("recon_query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let rationale = args
            .get("rationale")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);

        let receipt = self
            .bridge
            .apply_orchestrate_control(
                &ctx.session_id,
                &ctx.workspace_dir,
                OrchestrateControlRequest {
                    posture,
                    cognitive_tier,
                    reasoning_effort,
                    concise_mode,
                    minimalism_mode,
                    zmode,
                    refresh_recon,
                    recon_query,
                    rationale,
                },
            )
            .await?;

        let mut parts: Vec<String> = Vec::new();
        if let Some(p) = receipt.posture {
            parts.push(format!("posture={}", p.as_str()));
        }
        if let Some(t) = receipt.cognitive_tier {
            let model_note = match (receipt.model_locked_by_override, receipt.resolved_model.as_deref()) {
                (true, _) => " (model locked by explicit override)".to_string(),
                (false, Some(m)) => format!(" -> `{m}`"),
                (false, None) => String::new(),
            };
            parts.push(format!("tier={}{model_note}", t.as_str()));
        }
        if let Some(e) = receipt.reasoning_effort {
            parts.push(format!("effort={}", e.as_str()));
        }
        if let Some(zm) = receipt.zmode {
            parts.push(format!("zmode={}", zm.as_str()));
        }
        if let Some(cm) = receipt.concise_mode {
            parts.push(format!("concise_mode={}", cm.as_str()));
        }
        if let Some(mm) = receipt.minimalism_mode {
            parts.push(format!("minimalism_mode={}", mm.as_str()));
        }
        let header = match parts.is_empty() {
            true => "Orchestration state inspected.".to_string(),
            false => format!("Orchestration updated: {}.", parts.join(", ")),
        };
        let content = match receipt.recon_summary.as_deref().filter(|s| !s.is_empty()) {
            Some(recon) => format!("{header}\n\n<situational_awareness>\n{recon}\n</situational_awareness>"),
            None => header,
        };
        Ok(ToolOutput {
            content,
            is_error: false,
            diff: None,
            metadata: serde_json::to_value(&receipt).ok(),
        })
    }
}

fn parse_ask_options(raw: Option<&Value>, max_items: usize) -> Vec<ClarificationOption> {
    raw.and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .take(max_items)
                .enumerate()
                .filter_map(|(idx, v)| {
                    let label = v.get("label").and_then(Value::as_str)?.trim().to_string();
                    if label.is_empty() {
                        return None;
                    }
                    let id = v
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .unwrap_or_else(|| format!("choice_{}", idx + 1));
                    let description = v
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let recommended = v
                        .get("recommended")
                        .and_then(Value::as_bool)
                        .unwrap_or(idx == 0);
                    Some(ClarificationOption {
                        id,
                        label,
                        description,
                        recommended,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_ask_steps(raw: Option<&Value>) -> Vec<AskUserStep> {
    raw.and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .take(ASK_USER_MAX_STEPS)
                .enumerate()
                .filter_map(|(idx, v)| {
                    let question = v
                        .get("question")
                        .and_then(Value::as_str)?
                        .trim()
                        .to_string();
                    if question.is_empty() {
                        return None;
                    }
                    let options = parse_ask_options(v.get("options"), ASK_USER_MAX_OPTIONS);
                    if options.is_empty() {
                        return None;
                    }
                    let id = v
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .unwrap_or_else(|| format!("step_{}", idx + 1));
                    let description = v
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let selection_mode = v
                        .get("selection_mode")
                        .and_then(Value::as_str)
                        .and_then(AskUserSelectionMode::parse)
                        .unwrap_or(AskUserSelectionMode::Single);
                    let min_selections = v
                        .get("min_selections")
                        .and_then(Value::as_u64)
                        .map(|n| n as usize);
                    let max_selections = v
                        .get("max_selections")
                        .and_then(Value::as_u64)
                        .map(|n| n as usize);
                    Some(AskUserStep {
                        id,
                        question,
                        description,
                        selection_mode,
                        options,
                        min_selections,
                        max_selections,
                        allow_custom: v
                            .get("allow_custom")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_ask_fields(raw: Option<&Value>) -> Vec<AskUserField> {
    raw.and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .take(ASK_USER_MAX_FIELDS)
                .enumerate()
                .filter_map(|(idx, v)| {
                    let label = v.get("label").and_then(Value::as_str)?.trim().to_string();
                    if label.is_empty() {
                        return None;
                    }
                    let id = v
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .unwrap_or_else(|| format!("field_{}", idx + 1));
                    Some(AskUserField {
                        id,
                        label,
                        description: v
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .trim()
                            .to_string(),
                        placeholder: v
                            .get("placeholder")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .trim()
                            .to_string(),
                        default_value: v
                            .get("default_value")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(String::from),
                        required: v.get("required").and_then(Value::as_bool).unwrap_or(true),
                        secret: v.get("secret").and_then(Value::as_bool).unwrap_or(false),
                        multiline: v.get("multiline").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_ask_recommendation(
    raw: Option<&Value>,
    fallback_title: &str,
    fallback_summary: &str,
) -> Option<AskUserRecommendation> {
    raw.and_then(Value::as_object).map(|obj| {
        let mut pros = parse_string_vec(obj.get("pros"));
        pros.truncate(ASK_USER_MAX_PROS_CONS);
        let mut cons = parse_string_vec(obj.get("cons"));
        cons.truncate(ASK_USER_MAX_PROS_CONS);
        let title = obj
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(fallback_title)
            .to_string();
        let summary = obj
            .get("summary")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(fallback_summary)
            .to_string();
        let rationale = obj
            .get("rationale")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let confidence = obj
            .get("confidence")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("recommended")
            .to_string();
        let accept_label = obj
            .get("accept_label")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Accept")
            .to_string();
        let alternatives_label = obj
            .get("alternatives_label")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Alternatives")
            .to_string();
        AskUserRecommendation {
            title,
            summary,
            rationale,
            confidence,
            pros,
            cons,
            accept_label,
            alternatives_label,
        }
    })
}

fn format_ask_user_answer_text(spec: &AskUserRequestSpec, ans: &zenko_core::ClarificationAnswer) -> String {
    let mut parts: Vec<String> = vec![format!(
        "User prompt ({}) resolved by `{}` (auto_resolved={}, needs_user_input={}):",
        spec.kind.as_str(),
        ans.resolved_by,
        ans.auto_resolved,
        ans.needs_user_input
    )];
    match (
        !ans.selected_option_ids.is_empty(),
        !ans.selected_option_id.trim().is_empty(),
    ) {
        (true, _) => parts.push(format!("selected=[{}]", ans.selected_option_ids.join(", "))),
        (false, true) => parts.push(format!("selected=`{}`", ans.selected_option_id)),
        (false, false) => {}
    }
    if !ans.step_answers.is_empty() {
        let steps_str: Vec<String> = ans
            .step_answers
            .iter()
            .map(|(k, v)| format!("{k}=[{}]", v.join(", ")))
            .collect();
        parts.push(format!("steps={{ {} }}", steps_str.join("; ")));
    }
    if !ans.field_values.is_empty() {
        let fields_str: Vec<String> = ans
            .field_values
            .iter()
            .map(|(k, v)| format!("{k}={v:?}"))
            .collect();
        parts.push(format!("fields={{ {} }}", fields_str.join(", ")));
    }
    if let Some(ref rating) = ans.rating
        && !rating.trim().is_empty()
    {
        parts.push(format!("rating=`{}`", rating.trim()));
    }
    if let Some(ref note) = ans.custom_answer
        && !note.trim().is_empty()
    {
        parts.push(format!("note={:?}", note.trim()));
    }
    parts.join(" ")
}

impl AskUserTool {
    pub fn new(bridge: Arc<dyn OrchestratorBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for AskUserTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Orchestration
    }

    fn is_available(&self, ctx: &zenko_core::ToolAvailabilityContext) -> bool {
        !ctx.is_subagent
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Global
    }

    fn spec(&self) -> ToolSpec {
        let kind_enum: Vec<Value> = ASK_USER_KINDS
            .iter()
            .map(|&k| Value::String(k.to_string()))
            .collect();
        let mode_enum: Vec<Value> = ASK_USER_SELECTION_MODES
            .iter()
            .map(|&m| Value::String(m.to_string()))
            .collect();
        ToolSpec {
            name: TOOL_ASK_USER.into(),
            description: "Ask the user directly whenever you lack confidence, face meaningful ambiguity, or need missing data (URLs, paths, target environment, credentials/config, preferences, or sign-off). Available at any point in a task (start, mid-turn, or wrap-up). Supports five interactive WebUI elements: `option_list` (single/multi pick), `question_flow` (multi-step guided questions), `recommendation` (opinionated proposal with Accept/Alternatives), `data_input` (structured fields or freeform data), and `feedback` (wrap-up check-in/rating). Resolved answers persist into `.zenko/sessions/<id>/zpowers/decisions.md`.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": kind_enum,
                        "description": "UI surface kind: `option_list`, `question_flow`, `recommendation`, `data_input`, or `feedback` (inferred automatically from supplied fields if omitted)."
                    },
                    "title": { "type": "string", "description": "Optional short card heading." },
                    "question": { "type": "string", "description": "Clear, concise question or request for the user." },
                    "context": { "type": "string", "description": "Brief rationale, tradeoff summary, or why this input is needed." },
                    "selection_mode": {
                        "type": "string",
                        "enum": mode_enum,
                        "description": "For `option_list`: `single` (commits on click) or `multiple` (checkboxes + Confirm)."
                    },
                    "options": {
                        "type": "array",
                        "description": "Choices for `option_list`, fallback alternatives for `recommendation`, or quick-pick reasons for `feedback`.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "label": { "type": "string" },
                                "description": { "type": "string" },
                                "recommended": { "type": "boolean" }
                            },
                            "required": ["label"]
                        }
                    },
                    "steps": {
                        "type": "array",
                        "description": "Sequential steps for `question_flow` (up to 5 steps).",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "question": { "type": "string" },
                                "description": { "type": "string" },
                                "selection_mode": { "type": "string", "enum": mode_enum },
                                "min_selections": { "type": "integer", "minimum": 0 },
                                "max_selections": { "type": "integer", "minimum": 1 },
                                "options": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "id": { "type": "string" },
                                            "label": { "type": "string" },
                                            "description": { "type": "string" },
                                            "recommended": { "type": "boolean" }
                                        },
                                        "required": ["label"]
                                    }
                                }
                            },
                            "required": ["question", "options"]
                        }
                    },
                    "fields": {
                        "type": "array",
                        "description": "Structured input fields for `data_input` (e.g. target URL, DB host, region, token name).",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "label": { "type": "string" },
                                "placeholder": { "type": "string" },
                                "default_value": { "type": "string" },
                                "required": { "type": "boolean" },
                                "secret": { "type": "boolean" },
                                "multiline": { "type": "boolean" }
                            },
                            "required": ["label"]
                        }
                    },
                    "recommendation": {
                        "type": "object",
                        "description": "Structured opinionated proposal for `recommendation`.",
                        "properties": {
                            "title": { "type": "string" },
                            "summary": { "type": "string" },
                            "rationale": { "type": "string" },
                            "confidence": { "type": "string" },
                            "pros": { "type": "array", "items": { "type": "string" } },
                            "cons": { "type": "array", "items": { "type": "string" } },
                            "accept_label": { "type": "string" },
                            "alternatives_label": { "type": "string" }
                        }
                    },
                    "allow_custom": { "type": "boolean", "description": "Show a custom freeform note/override input alongside structured choices." },
                    "allow_auto_default": { "type": "boolean", "description": "Whether headless 24/7 mode may auto-apply defaults (set false for missing required facts/data)." }
                },
                "required": ["question"]
            }),
            risk: RiskLevel::Safe,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let question = args
            .get("question")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ZenkoError::Tool("missing `question`".into()))?
            .to_string();
        let title = args
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let context = args
            .get("context")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let selection_mode = args
            .get("selection_mode")
            .and_then(Value::as_str)
            .and_then(AskUserSelectionMode::parse)
            .unwrap_or(AskUserSelectionMode::Single);
        let mut options = parse_ask_options(args.get("options"), ASK_USER_MAX_OPTIONS);
        let steps = parse_ask_steps(args.get("steps"));
        let fields = parse_ask_fields(args.get("fields"));
        let mut recommendation = parse_ask_recommendation(
            args.get("recommendation"),
            match title.is_empty() {
                true => &question,
                false => &title,
            },
            &context,
        );

        let kind = match args.get("kind").and_then(Value::as_str) {
            Some(raw) => AskUserKind::parse(raw).ok_or_else(|| {
                ZenkoError::Tool(format!(
                    "Unknown `kind` `{raw}`. Valid values: {}.",
                    ASK_USER_KINDS.join(", ")
                ))
            })?,
            None => match (
                !steps.is_empty(),
                !fields.is_empty(),
                recommendation.is_some(),
                !options.is_empty(),
            ) {
                (true, _, _, _) => AskUserKind::QuestionFlow,
                (false, true, _, _) => AskUserKind::DataInput,
                (false, false, true, _) => AskUserKind::Recommendation,
                (false, false, false, true) => AskUserKind::OptionList,
                (false, false, false, false) => AskUserKind::DataInput,
            },
        };

        match kind {
            AskUserKind::OptionList if options.is_empty() => {
                return Ok(ToolOutput::err(
                    "`option_list` requires at least 1 option with `label` (or use `kind=\"data_input\"` for freeform input)",
                ));
            }
            AskUserKind::QuestionFlow if steps.is_empty() => {
                return Ok(ToolOutput::err(
                    "`question_flow` requires at least 1 step with `question` and `options`",
                ));
            }
            AskUserKind::Recommendation => {
                if recommendation.is_none() {
                    recommendation = Some(AskUserRecommendation {
                        title: match title.is_empty() {
                            true => question.clone(),
                            false => title.clone(),
                        },
                        summary: context.clone(),
                        rationale: String::new(),
                        confidence: "recommended".into(),
                        pros: Vec::new(),
                        cons: Vec::new(),
                        accept_label: "Accept".into(),
                        alternatives_label: "Alternatives".into(),
                    });
                }
                if options.is_empty() {
                    options.push(ClarificationOption {
                        id: ASK_USER_OPTION_ACCEPT.into(),
                        label: recommendation
                            .as_ref()
                            .map(|r| r.accept_label.clone())
                            .unwrap_or_else(|| "Accept".into()),
                        description: "Proceed with the recommended approach".into(),
                        recommended: true,
                    });
                }
            }
            _ => {}
        }

        let allow_custom = args
            .get("allow_custom")
            .and_then(Value::as_bool)
            .unwrap_or(matches!(kind, AskUserKind::DataInput | AskUserKind::Feedback));
        let allow_auto_default = args
            .get("allow_auto_default")
            .and_then(Value::as_bool)
            .unwrap_or(!matches!(kind, AskUserKind::DataInput));

        let spec = AskUserRequestSpec {
            kind,
            title: title.clone(),
            question: question.clone(),
            context: context.clone(),
            selection_mode,
            options,
            steps,
            fields,
            recommendation,
            allow_custom,
            allow_auto_default,
        };

        let answer = self
            .bridge
            .request_ask_user(&ctx.session_id, spec.clone(), ctx.headless_no_wait, cancel)
            .await?;

        let secret_ids: std::collections::HashSet<&str> = spec
            .fields
            .iter()
            .filter(|f| f.secret)
            .map(|f| f.id.as_str())
            .collect();
        let safe_field_values: std::collections::BTreeMap<String, String> = answer
            .field_values
            .iter()
            .map(|(k, v)| {
                let sanitized = match secret_ids.contains(k.as_str()) {
                    true => "[provided]".to_string(),
                    false => scrub_secrets(v, &[]),
                };
                (k.clone(), sanitized)
            })
            .collect();
        let safe_custom = answer.custom_answer.as_deref().map(|s| scrub_secrets(s, &[]));

        Ok(ToolOutput {
            content: format_ask_user_answer_text(&spec, &answer),
            is_error: false,
            diff: None,
            metadata: Some(json!({
                "kind": spec.kind.as_str(),
                "title": spec.title,
                "question": spec.question,
                "context": spec.context,
                "options": spec.options,
                "steps": spec.steps,
                "recommendation": spec.recommendation,
                "selected_option_id": answer.selected_option_id,
                "selected_option_ids": answer.selected_option_ids,
                "step_answers": answer.step_answers,
                "field_values": safe_field_values,
                "custom_answer": safe_custom,
                "rating": answer.rating,
                "resolved_by": answer.resolved_by,
                "auto_resolved": answer.auto_resolved,
                "needs_user_input": answer.needs_user_input,
            })),
        })
    }
}

/// Phase 6 durable SQLite Task DAG tool (`session_task`):
/// supports incremental `create`, `update`, atomic `claim`, `list`, topological `ready`, `delete`,
/// and `clear` across compaction, daemon restarts, and parallel subagent workers.
pub struct SessionTaskTool {
    bridge: Arc<dyn OrchestratorBridge>,
}

impl SessionTaskTool {
    pub fn new(bridge: Arc<dyn OrchestratorBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for SessionTaskTool {
    fn toolset(&self) -> zenko_core::ToolSet {
        zenko_core::ToolSet::Orchestration
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        let parsed = args
            .get("action")
            .and_then(Value::as_str)
            .and_then(SessionTaskAction::parse);
        match parsed {
            Some(SessionTaskAction::List | SessionTaskAction::Ready) => ToolConcurrency::Shared,
            Some(SessionTaskAction::Clear) => ToolConcurrency::Global,
            _ => ToolConcurrency::Keyed(ResourceLockKey::Task(
                SESSION_TASK_DAG_LOCK_KEY.to_string(),
            )),
        }
    }

    fn spec(&self) -> ToolSpec {
        let action_enum: Vec<Value> = SESSION_TASK_ACTIONS
            .iter()
            .map(|&a| Value::String(a.to_string()))
            .collect();
        let status_enum: Vec<Value> = SESSION_TASK_STATUSES
            .iter()
            .map(|&s| Value::String(s.to_string()))
            .collect();
        ToolSpec {
            name: TOOL_SESSION_TASK.into(),
            description: "Manage the durable SQLite Session Task DAG (survives compaction, daemon restarts, and coordinates parallel subagents via atomic `claim`). Actions: `create`, `update`, `claim`, `list`, `ready` (unblocked pending queue), `delete`, `clear`.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": action_enum,
                        "description": "Task DAG operation."
                    },
                    "id": { "type": "string", "description": "Task ID (optional on create; required on update/claim/delete)." },
                    "title": { "type": "string", "description": "Concise task title." },
                    "description": { "type": "string", "description": "Optional detailed specification." },
                    "active_label": { "type": "string", "description": "Optional live progress spinner label." },
                    "status": {
                        "type": "string",
                        "enum": status_enum
                    },
                    "depends_on": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Upstream task IDs that must reach `done` before this task becomes ready."
                    },
                    "add_depends_on": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Additional upstream task IDs to append on update."
                    },
                    "owner_agent": { "type": "string", "description": "Assigned subagent or worker ID." },
                    "verification_cmd": { "type": "string", "description": "Executable build/test check proving completion." },
                    "verification_evidence": { "type": "string", "description": "Concise pass/fail output recorded upon verification." },
                    "notes": { "type": "string", "description": "Optional execution or handoff notes." },
                    "priority": { "type": "integer", "description": "Higher values sort first in the ready queue." }
                },
                "required": ["action"]
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
        let raw_action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `action`".into()))?;
        let action = SessionTaskAction::parse(raw_action).ok_or_else(|| {
            ZenkoError::Tool(format!(
                "Unknown `session_task` action `{raw_action}`. Valid actions: {}.",
                SESSION_TASK_ACTIONS.join(", ")
            ))
        })?;

        let opt_str = |key: &str| -> Option<String> {
            args.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
        };
        let task_id_opt = || opt_str("id").or_else(|| opt_str("task_id"));

        match action {
            SessionTaskAction::List => {
                let tasks = self.bridge.list_session_tasks(&ctx.session_id)?;
                let mut out = ToolOutput::ok(format_task_list("Session Task DAG", &tasks));
                out.metadata = serde_json::to_value(&tasks).ok();
                Ok(out)
            }
            SessionTaskAction::Ready => {
                let tasks = self.bridge.ready_session_tasks(&ctx.session_id)?;
                let mut out = ToolOutput::ok(format_task_list("Ready Tasks (Unblocked)", &tasks));
                out.metadata = serde_json::to_value(&tasks).ok();
                Ok(out)
            }
            SessionTaskAction::Create => {
                let title = opt_str("title")
                    .ok_or_else(|| ZenkoError::Tool("`title` is required for `create`".into()))?;
                let now = Utc::now();
                let task = SessionTask {
                    id: task_id_opt().unwrap_or_else(new_session_task_id),
                    session_id: ctx.session_id.clone(),
                    branch_event_id: None,
                    title,
                    description: opt_str("description").unwrap_or_default(),
                    active_label: opt_str("active_label"),
                    status: args
                        .get("status")
                        .and_then(Value::as_str)
                        .map(SessionTaskStatus::from_str_loose)
                        .unwrap_or(SessionTaskStatus::Pending),
                    depends_on: parse_string_vec(args.get("depends_on")),
                    owner_agent: opt_str("owner_agent"),
                    verification_cmd: opt_str("verification_cmd"),
                    verification_evidence: opt_str("verification_evidence"),
                    notes: opt_str("notes"),
                    priority: args.get("priority").and_then(Value::as_i64).unwrap_or(0) as i32,
                    created_at: now,
                    updated_at: now,
                };
                let saved = self.bridge.create_session_task(&ctx.session_id, task)?;
                let mut out = ToolOutput::ok(format!(
                    "Created task `{}` [{}]: {}",
                    saved.id,
                    saved.status.as_str(),
                    saved.title
                ));
                out.metadata = serde_json::to_value(&saved).ok();
                Ok(out)
            }
            SessionTaskAction::Update => {
                let id = task_id_opt()
                    .ok_or_else(|| ZenkoError::Tool("`id` is required for `update`".into()))?;
                let patch = SessionTaskPatch {
                    title: opt_str("title"),
                    description: args
                        .get("description")
                        .and_then(Value::as_str)
                        .map(|s| s.trim().to_string()),
                    active_label: args
                        .get("active_label")
                        .map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)),
                    status: args
                        .get("status")
                        .and_then(Value::as_str)
                        .map(SessionTaskStatus::from_str_loose),
                    depends_on: args.get("depends_on").map(|v| parse_string_vec(Some(v))),
                    add_depends_on: parse_string_vec(args.get("add_depends_on")),
                    owner_agent: args
                        .get("owner_agent")
                        .map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)),
                    verification_cmd: args
                        .get("verification_cmd")
                        .map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)),
                    verification_evidence: args
                        .get("verification_evidence")
                        .map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)),
                    notes: args
                        .get("notes")
                        .map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)),
                    priority: args.get("priority").and_then(Value::as_i64).map(|v| v as i32),
                };
                let updated = self.bridge.update_session_task(&ctx.session_id, &id, patch)?;
                let mut out = ToolOutput::ok(format!(
                    "Updated task `{}` [{}]: {}",
                    updated.id,
                    updated.status.as_str(),
                    updated.title
                ));
                out.metadata = serde_json::to_value(&updated).ok();
                Ok(out)
            }
            SessionTaskAction::Claim => {
                let id = task_id_opt()
                    .ok_or_else(|| ZenkoError::Tool("`id` is required for `claim`".into()))?;
                let owner = opt_str("owner_agent").unwrap_or_else(|| ctx.session_id.clone());
                let claimed = self.bridge.claim_session_task(&ctx.session_id, &id, &owner)?;
                let mut out = ToolOutput::ok(format!(
                    "Claimed task `{}` for `{}` (status={}).",
                    claimed.id,
                    claimed.owner_agent.as_deref().unwrap_or(&owner),
                    claimed.status.as_str()
                ));
                out.metadata = serde_json::to_value(&claimed).ok();
                Ok(out)
            }
            SessionTaskAction::Delete => {
                let id = task_id_opt()
                    .ok_or_else(|| ZenkoError::Tool("`id` is required for `delete`".into()))?;
                let deleted = self.bridge.delete_session_task(&ctx.session_id, &id)?;
                Ok(ToolOutput::ok(match deleted {
                    true => format!("Deleted task `{id}`."),
                    false => format!("Task `{id}` was not found."),
                }))
            }
            SessionTaskAction::Clear => {
                let n = self.bridge.clear_session_tasks(&ctx.session_id)?;
                Ok(ToolOutput::ok(format!("Cleared {n} session task(s).")))
            }
        }
    }
}

fn format_task_list(header: &str, tasks: &[SessionTask]) -> String {
    if tasks.is_empty() {
        return format!("{header}: (none)");
    }
    let lines: Vec<String> = tasks
        .iter()
        .map(|t| {
            let deps = match t.depends_on.is_empty() {
                true => String::new(),
                false => format!(" deps=[{}]", t.depends_on.join(",")),
            };
            let owner = t
                .owner_agent
                .as_deref()
                .map(|o| format!(" owner={o}"))
                .unwrap_or_default();
            format!(
                "- `{}` [{}] (p={}): {}{owner}{deps}",
                t.id,
                t.status.as_str(),
                t.priority,
                t.title
            )
        })
        .collect();
    format!("{header} ({}):\n{}", tasks.len(), lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use zenko_core::{
        ClarificationAnswer, ClarificationRecord, TOOL_ASK_CLARIFICATION, TOOL_ASK_USER,
        ToolAvailabilityContext,
    };

    struct MockAskBridge {
        seen: std::sync::Mutex<Vec<AskUserRequestSpec>>,
    }

    impl MockAskBridge {
        fn new() -> Self {
            Self {
                seen: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl OrchestratorBridge for MockAskBridge {
        fn record_brainstorm(&self, _: &str, spec: &BrainstormSpec) -> Result<BrainstormSpec> {
            Ok(spec.clone())
        }
        fn update_plan(&self, _: &str, plan: &ExecutionPlan) -> Result<ExecutionPlan> {
            Ok(plan.clone())
        }
        async fn request_ask_user(
            &self,
            _: &str,
            spec: AskUserRequestSpec,
            _: bool,
            _: CancellationToken,
        ) -> Result<ClarificationAnswer> {
            if let Ok(mut list) = self.seen.lock() {
                list.push(spec.clone());
            }
            let mut field_values = std::collections::BTreeMap::new();
            for f in &spec.fields {
                field_values.insert(
                    f.id.clone(),
                    match f.secret {
                        true => "sk-live-secret-999".into(),
                        false => "https://staging.example.com".into(),
                    },
                );
            }
            Ok(ClarificationAnswer {
                selected_option_id: spec
                    .options
                    .first()
                    .map(|o| o.id.clone())
                    .unwrap_or_else(|| "data_submitted".into()),
                selected_option_ids: spec.options.first().map(|o| vec![o.id.clone()]).unwrap_or_default(),
                step_answers: std::collections::BTreeMap::new(),
                field_values,
                custom_answer: Some("user note".into()),
                rating: None,
                resolved_by: "webui".into(),
                auto_resolved: false,
                needs_user_input: false,
            })
        }
        fn list_pending_clarifications(&self, _: Option<&str>) -> Vec<ClarificationRecord> {
            Vec::new()
        }
        fn list_session_tasks(&self, _: &str) -> Result<Vec<SessionTask>> {
            Ok(Vec::new())
        }
        fn ready_session_tasks(&self, _: &str) -> Result<Vec<SessionTask>> {
            Ok(Vec::new())
        }
        fn create_session_task(&self, _: &str, task: SessionTask) -> Result<SessionTask> {
            Ok(task)
        }
        fn update_session_task(&self, _: &str, _: &str, _: SessionTaskPatch) -> Result<SessionTask> {
            Err(ZenkoError::Tool("unused".into()))
        }
        fn claim_session_task(&self, _: &str, _: &str, _: &str) -> Result<SessionTask> {
            Err(ZenkoError::Tool("unused".into()))
        }
        fn delete_session_task(&self, _: &str, _: &str) -> Result<bool> {
            Ok(false)
        }
        fn clear_session_tasks(&self, _: &str) -> Result<usize> {
            Ok(0)
        }
        async fn apply_orchestrate_control(
            &self,
            _: &str,
            _: &Path,
            _: OrchestrateControlRequest,
        ) -> Result<zenko_core::OrchestrateControlReceipt> {
            Err(ZenkoError::Tool("unused".into()))
        }
    }

    #[tokio::test]
    async fn ask_user_tool_infers_kinds_masks_secret_metadata_and_resolves_alias() {
        let bridge = Arc::new(MockAskBridge::new());
        let reg = crate::ToolRegistry::empty();
        reg.register(Arc::new(AskUserTool::new(bridge.clone())));

        // Canonical name and legacy alias both resolve to the exact same tool.
        let canonical = reg.get(TOOL_ASK_USER).expect("ask_user registered");
        let aliased = reg
            .get(TOOL_ASK_CLARIFICATION)
            .expect("ask_clarification alias resolves");
        assert_eq!(canonical.spec().name, TOOL_ASK_USER);
        assert_eq!(aliased.spec().name, TOOL_ASK_USER);
        assert!(canonical.is_available(&ToolAvailabilityContext {
            is_subagent: false,
            ..Default::default()
        }));
        assert!(!canonical.is_available(&ToolAvailabilityContext {
            is_subagent: true,
            ..Default::default()
        }));

        let ctx = ToolContext {
            session_id: "s1".into(),
            workspace_dir: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            hub_dir: std::env::temp_dir(),
            shell_timeout_secs: 30,
            auto_background_after_secs: 15,
            max_output_bytes: 65_536,
            headless_no_wait: false,
            demote_bg: Arc::new(tokio::sync::Notify::new()),
            file_state: None,
        };

        // 1. DataInput inferred from `fields`, secret field masked in persisted metadata.
        let out = canonical
            .execute(
                &ctx,
                json!({
                    "question": "Provide target environment details",
                    "fields": [
                        { "id": "url", "label": "Target URL" },
                        { "id": "token", "label": "API Token", "secret": true }
                    ]
                }),
                CancellationToken::new(),
            )
            .await
            .expect("data_input execute");
        assert!(!out.is_error);
        let meta = out.metadata.expect("metadata");
        assert_eq!(meta["kind"], "data_input");
        assert_eq!(meta["field_values"]["url"], "https://staging.example.com");
        assert_eq!(meta["field_values"]["token"], "[provided]");

        // 2. QuestionFlow inferred from `steps`.
        let out_flow = canonical
            .execute(
                &ctx,
                json!({
                    "question": "Configure new service",
                    "steps": [
                        {
                            "id": "runtime",
                            "question": "Runtime?",
                            "options": [{ "id": "rust", "label": "Rust" }]
                        }
                    ]
                }),
                CancellationToken::new(),
            )
            .await
            .expect("flow execute");
        assert_eq!(out_flow.metadata.expect("meta")["kind"], "question_flow");

        // 3. Recommendation inferred from `recommendation` object.
        let out_rec = canonical
            .execute(
                &ctx,
                json!({
                    "question": "Proceed with SQLite WAL?",
                    "recommendation": {
                        "title": "Use SQLite WAL",
                        "summary": "Zero external daemon, ACID transactions",
                        "pros": ["Single file"],
                        "cons": ["Single writer"]
                    }
                }),
                CancellationToken::new(),
            )
            .await
            .expect("rec execute");
        assert_eq!(out_rec.metadata.expect("meta")["kind"], "recommendation");
    }
}
