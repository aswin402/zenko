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
    BRAINSTORM_STAGES, BrainstormOption, BrainstormSpec, BrainstormStage, CONCISE_MODES,
    ClarificationOption, ConciseMode, ExecutionPlan, MODEL_ROUTING_ROLES, ModelRoutingRole,
    ORCH_BRAINSTORM_MAX_OPTIONS, ORCH_DEFAULT_OPTION_SCORE, ORCH_PLAN_MAX_STEPS,
    OrchestrateControlRequest, OrchestratorBridge, PlanStep, PlanStepStatus, REASONING_EFFORTS,
    ReasoningEffort, ResourceLockKey, Result, RiskLevel, SESSION_TASK_ACTIONS,
    SESSION_TASK_DAG_LOCK_KEY, SESSION_TASK_STATUSES, SessionTask, SessionTaskAction,
    SessionTaskPatch, SessionTaskStatus, TOOL_ASK_CLARIFICATION, TOOL_ORCHESTRATE_CONTROL,
    TOOL_PLAN_UPDATE, TOOL_RECORD_BRAINSTORM, TOOL_SESSION_TASK, TURN_POSTURES, Tool,
    ToolConcurrency, ToolContext, ToolOutput, ToolSpec, TurnPosture, ZenkoError, new_id,
    new_session_task_id,
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

pub struct AskClarificationTool {
    bridge: Arc<dyn OrchestratorBridge>,
}

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
        ToolSpec {
            name: TOOL_ORCHESTRATE_CONTROL.into(),
            description: "Exercise sovereign control over your own turn orchestration (zero /commands): declare or transition your turn posture, dynamically shift cognitive gear (configured model tier & reasoning effort), adjust live English concise mode (`off`, `lite`, `full`, `ultra`), or refresh live 5-way parallel workspace/git/memory/DAG recon on demand.".into(),
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
                    "concise_mode": {
                        "type": "string",
                        "enum": concise_enum,
                        "description": "Live English output conciseness tier for this session (`off`, `lite`, `full`, `ultra`)."
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
        if let Some(cm) = receipt.concise_mode {
            parts.push(format!("concise_mode={}", cm.as_str()));
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

impl AskClarificationTool {
    pub fn new(bridge: Arc<dyn OrchestratorBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for AskClarificationTool {
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
        ToolSpec {
            name: TOOL_ASK_CLARIFICATION.into(),
            description: "Ask a high-impact multiple-choice design clarification question with a recommended default. Surfaces an interactive card in WebUI/CLI/Telegram and auto-resolves safely with the recommended option in headless 24/7 runs.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "Concise high-impact design question" },
                    "context": { "type": "string", "description": "Brief tradeoff context" },
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
                            "required": ["id", "label"]
                        }
                    }
                },
                "required": ["question", "options"]
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
            .ok_or_else(|| ZenkoError::Tool("missing `question`".into()))?;
        let context = args
            .get("context")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let raw_opts = args
            .get("options")
            .and_then(Value::as_array)
            .ok_or_else(|| ZenkoError::Tool("missing `options` array".into()))?;

        let options: Vec<ClarificationOption> = raw_opts
            .iter()
            .take(ORCH_BRAINSTORM_MAX_OPTIONS)
            .enumerate()
            .filter_map(|(idx, v)| {
                let label = v.get("label").and_then(Value::as_str)?.trim().to_string();
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
            .collect();

        if options.is_empty() {
            return Ok(ToolOutput::err(
                "provide at least 1 clarification option with `id` and `label`",
            ));
        }

        let answer = self
            .bridge
            .request_clarification(
                &ctx.session_id,
                question,
                context,
                options,
                ctx.headless_no_wait,
                cancel,
            )
            .await?;

        let custom_suffix = answer
            .custom_answer
            .as_deref()
            .map(|s| format!(" Note: {s}"))
            .unwrap_or_default();
        Ok(ToolOutput {
            content: format!(
                "Clarification resolved by `{}` (auto_resolved={}): selected option `{}`.{custom_suffix}",
                answer.resolved_by, answer.auto_resolved, answer.selected_option_id
            ),
            is_error: false,
            diff: None,
            metadata: Some(json!({
                "question": question,
                "context": context,
                "selected_option_id": answer.selected_option_id,
                "custom_answer": answer.custom_answer,
                "resolved_by": answer.resolved_by,
                "auto_resolved": answer.auto_resolved,
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
