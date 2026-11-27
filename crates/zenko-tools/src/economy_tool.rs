//! Deferred `token_economy` tool exposing Zenko's native Token Economy Engine to the orchestrator
//! with zero eager KV-cache schema bloat (invoked via `invoke_tool`).

use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    EconomyAction, EconomyBridge, Result, RiskLevel, TOOL_TOKEN_ECONOMY, Tool, ToolConcurrency,
    ToolContext, ToolExposure, ToolOutput, ToolSet, ToolSpec, ZenkoError,
};

fn ok_with_meta(content: String, meta: Value) -> ToolOutput {
    ToolOutput {
        content,
        is_error: false,
        diff: None,
        metadata: Some(meta),
    }
}

pub struct TokenEconomyTool {
    bridge: Arc<dyn EconomyBridge>,
}

impl TokenEconomyTool {
    pub fn new(bridge: Arc<dyn EconomyBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for TokenEconomyTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_TOKEN_ECONOMY.into(),
            description: "Inspect and manage Zenko's native Token Economy Engine for the current repository: verified cross-session micro-skills, soft-pruned tool schemas, empirical P90 turn baselines, and net token savings ledger.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": [
                            "status",
                            "micro_skills",
                            "add_micro_skill",
                            "retire_micro_skill",
                            "prune_report",
                            "apply_prune",
                            "restore_tool",
                            "reaudit",
                            "ledger",
                            "discover",
                            "verify_filters"
                        ],
                        "description": "Operation to perform on the current repository's Token Economy state."
                    },
                    "fact": {
                        "type": "string",
                        "description": "Verified 1-line repository/environment fact (for action=add_micro_skill)."
                    },
                    "failed_cmd": {
                        "type": "string",
                        "description": "Optional failing command replaced by this fact (for action=add_micro_skill)."
                    },
                    "fixed_cmd": {
                        "type": "string",
                        "description": "Optional working replacement command (for action=add_micro_skill)."
                    },
                    "id": {
                        "type": "string",
                        "description": "Micro-skill ID to retire (for action=retire_micro_skill)."
                    },
                    "tool_name": {
                        "type": "string",
                        "description": "Tool name to soft-prune or restore (for action=apply_prune|restore_tool)."
                    },
                    "pin": {
                        "type": "boolean",
                        "description": "When restoring a tool, pin it so auto-pruning never demotes it (default: true)."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max missed-savings opportunities to return (for action=discover, default: 10)."
                    }
                },
                "required": ["action"]
            }),
            risk: RiskLevel::Safe,
        }
    }

    fn toolset(&self) -> ToolSet {
        ToolSet::Memory
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
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
            .unwrap_or("status");
        let Some(action) = EconomyAction::parse(raw_action) else {
            return Err(ZenkoError::Tool(format!(
                "Invalid action `{raw_action}` (expected status, micro_skills, add_micro_skill, retire_micro_skill, prune_report, apply_prune, restore_tool, reaudit, ledger, discover, or verify_filters)"
            )));
        };

        match action {
            EconomyAction::Status => {
                let rep = self.bridge.economy_report(&ctx.workspace_dir)?;
                let s = &rep.summary;
                let cmd_summary = match rep.by_command.is_empty() {
                    true => "none yet".to_string(),
                    false => rep
                        .by_command
                        .iter()
                        .take(6)
                        .map(|c| format!("{} (+{} tok, {}x)", c.command_family, c.tokens_saved, c.calls))
                        .collect::<Vec<_>>()
                        .join(", "),
                };
                let filter_line = match rep.filter_status.project_file_exists {
                    true => format!(
                        "project `.zenko/filters.toml`: {} ({} rule(s)); user global: {} rule(s)",
                        match rep.filter_status.trusted {
                            true => "trusted",
                            false => "UNTRUSTED (run `zenko economy trust` or trust in WebUI)",
                        },
                        rep.filter_status.rule_count,
                        rep.filter_status.user_rule_count
                    ),
                    false => format!(
                        "no project `.zenko/filters.toml`; user global: {} rule(s)",
                        rep.filter_status.user_rule_count
                    ),
                };
                let text = format!(
                    "Token Economy ({scope}) · enabled={enabled}\n\
                     - Net Saved: {net} tok (~${usd:.4}) [Read-Less +{rl} · Output-Dedup +{od} ({od_hits} hits) · Prefix-ChunkRef +{pc} ({pc_hits} hits) · Shunt +{sh} ({sh_hits} hits) · Tool-Prune +{tp} · Overhead -{oh}]\n\
                     - Compression Health: {ccr} CCR recovery read(s) · {raw} raw bypass(es)\n\
                     - By Command Family: {cmd_summary}\n\
                     - Custom Filters: {filter_line}\n\
                     - Micro-Skills: {active} active, {cand} candidate, {ret} retired ({inj} injections)\n\
                     - Tool Pruning: {pruned} soft-demoted to deferred ({cands} candidates)\n\
                     - Loop Guard Baseline: P90={p90} calls, envelope={env} calls ({samples} samples, {lg} redirects)",
                    scope = rep.repo_scope,
                    enabled = rep.config.enabled,
                    net = s.net_tokens_saved,
                    usd = s.estimated_usd_saved,
                    rl = s.read_less_tokens_saved,
                    od = s.output_dedup_tokens_saved,
                    od_hits = s.output_dedup_hits,
                    pc = s.prefix_chunk_ref_tokens_saved,
                    pc_hits = s.prefix_chunk_ref_hits,
                    sh = s.shunt_tokens_saved,
                    sh_hits = s.shunt_hits,
                    tp = s.tool_prune_tokens_saved,
                    oh = s.overhead_tokens,
                    ccr = s.ccr_recoveries,
                    raw = s.raw_bypasses,
                    active = s.active_micro_skills,
                    cand = s.candidate_micro_skills,
                    ret = s.retired_micro_skills,
                    inj = s.micro_skill_injections,
                    pruned = s.pruned_tools_count,
                    cands = rep.prune_candidates.len(),
                    p90 = s.p90_turn_calls,
                    env = s.envelope_calls,
                    samples = s.baseline_samples,
                    lg = s.loop_guard_interventions,
                );
                Ok(ok_with_meta(
                    text,
                    serde_json::to_value(&rep).unwrap_or(Value::Null),
                ))
            }
            EconomyAction::MicroSkills => {
                let rep = self.bridge.economy_report(&ctx.workspace_dir)?;
                let lines: Vec<String> = rep
                    .micro_skills
                    .iter()
                    .map(|m| {
                        format!(
                            "- [{}] {} ({}x, hits={}/{}, contradictions={}): {}",
                            m.status.as_str(),
                            m.id,
                            m.corroborations,
                            m.need_hits,
                            m.exposures,
                            m.contradictions,
                            m.fact
                        )
                    })
                    .collect();
                let body = match lines.is_empty() {
                    true => format!("No repository micro-skills recorded yet for `{}`.", rep.repo_scope),
                    false => lines.join("\n"),
                };
                Ok(ok_with_meta(
                    body,
                    serde_json::to_value(&rep.micro_skills).unwrap_or(Value::Null),
                ))
            }
            EconomyAction::AddMicroSkill => {
                let fact = args
                    .get("fact")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        ZenkoError::Tool("`fact` is required for action=add_micro_skill".into())
                    })?;
                let failed_cmd = args.get("failed_cmd").and_then(Value::as_str);
                let fixed_cmd = args.get("fixed_cmd").and_then(Value::as_str);
                let saved = self.bridge.add_micro_skill(
                    &ctx.workspace_dir,
                    fact,
                    failed_cmd,
                    fixed_cmd,
                )?;
                Ok(ok_with_meta(
                    format!("Saved active micro-skill `{}`: {}", saved.id, saved.fact),
                    serde_json::to_value(&saved).unwrap_or(Value::Null),
                ))
            }
            EconomyAction::RetireMicroSkill => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        ZenkoError::Tool("`id` is required for action=retire_micro_skill".into())
                    })?;
                let retired = self.bridge.retire_micro_skill(id)?;
                match retired {
                    true => Ok(ToolOutput::ok(format!("Retired micro-skill `{id}`."))),
                    false => Ok(ToolOutput::err(format!("Micro-skill `{id}` not found."))),
                }
            }
            EconomyAction::PruneReport => {
                let rep = self.bridge.economy_report(&ctx.workspace_dir)?;
                let mut lines = vec![format!(
                    "Tool Pruning ({}): {} soft-pruned, {} approaching threshold",
                    rep.repo_scope,
                    rep.pruned_tools.len(),
                    rep.prune_candidates.len()
                )];
                for t in &rep.tool_usage {
                    let state = match (t.pruned_to_deferred, t.manually_pinned) {
                        (true, _) => "pruned->deferred",
                        (false, true) => "pinned",
                        (false, false) => "eager",
                    };
                    lines.push(format!(
                        "- `{}` [{}]: calls={}, unused_prompts={}, sessions={}, ~{} tok",
                        t.tool_name,
                        state,
                        t.call_count,
                        t.prompts_since_used,
                        t.sessions_observed,
                        t.estimated_schema_tokens
                    ));
                }
                Ok(ok_with_meta(
                    lines.join("\n"),
                    serde_json::to_value(&rep.tool_usage).unwrap_or(Value::Null),
                ))
            }
            EconomyAction::ApplyPrune => {
                let tool_name = args
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        ZenkoError::Tool("`tool_name` is required for action=apply_prune".into())
                    })?;
                let ok = self.bridge.apply_tool_prune(&ctx.workspace_dir, tool_name)?;
                match ok {
                    true => Ok(ToolOutput::ok(format!(
                        "Soft-pruned `{tool_name}` to deferred for this repository (still callable via `invoke_tool`)."
                    ))),
                    false => Ok(ToolOutput::err(format!(
                        "Cannot prune `{tool_name}` (protected core tool or invalid name)."
                    ))),
                }
            }
            EconomyAction::RestoreTool => {
                let tool_name = args
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        ZenkoError::Tool("`tool_name` is required for action=restore_tool".into())
                    })?;
                let pin = args.get("pin").and_then(Value::as_bool).unwrap_or(true);
                let _ = self.bridge.restore_tool(&ctx.workspace_dir, tool_name, pin)?;
                Ok(ToolOutput::ok(format!(
                    "Restored `{tool_name}` to eager exposure (pinned={pin})."
                )))
            }
            EconomyAction::Reaudit => {
                let count = self.bridge.reaudit_pruned_tools(&ctx.workspace_dir)?;
                Ok(ToolOutput::ok(format!(
                    "Re-audited repository tools: restored {count} pruned tool(s) for fresh evaluation."
                )))
            }
            EconomyAction::Ledger => {
                let rep = self.bridge.economy_report(&ctx.workspace_dir)?;
                let s = &rep.summary;
                let mut text = format!(
                    "Net Economy Ledger ({}):\n\
                     - Read-Less saved: {} tok\n\
                     - Output-Dedup saved: {} tok ({} hits)\n\
                     - Shunt large-read saved: {} tok ({} hits)\n\
                     - Tool-Prune saved: {} tok\n\
                     - CCR recoveries: {} | Raw bypasses: {}\n\
                     - Micro-Skill injections: {}\n\
                     - Loop-Guard redirects: {}\n\
                     - Background overhead (Dream/Judge): {} tok\n\
                     - Net tokens saved: {} tok (~${:.4})",
                    rep.repo_scope,
                    s.read_less_tokens_saved,
                    s.output_dedup_tokens_saved,
                    s.output_dedup_hits,
                    s.shunt_tokens_saved,
                    s.shunt_hits,
                    s.tool_prune_tokens_saved,
                    s.ccr_recoveries,
                    s.raw_bypasses,
                    s.micro_skill_injections,
                    s.loop_guard_interventions,
                    s.overhead_tokens,
                    s.net_tokens_saved,
                    s.estimated_usd_saved
                );
                if !rep.by_command.is_empty() {
                    text.push_str("\nBy Command Family:");
                    for c in &rep.by_command {
                        text.push_str(&format!(
                            "\n- {}: +{} tok ({} calls, {} raw bypasses)",
                            c.command_family, c.tokens_saved, c.calls, c.raw_bypasses
                        ));
                    }
                }
                Ok(ok_with_meta(
                    text,
                    serde_json::to_value(&rep).unwrap_or(Value::Null),
                ))
            }
            EconomyAction::Discover => {
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .map(|v| v as usize)
                    .unwrap_or(zenko_core::ECONOMY_DISCOVER_DEFAULT_LIMIT);
                let items = self.bridge.discover_missed_savings(&ctx.workspace_dir, limit)?;
                let text = match items.is_empty() {
                    true => "No large uncompressed shell/git command families found in recent sessions.".to_string(),
                    false => {
                        let mut lines = vec![format!(
                            "Discovered {} missed-savings command opportunit(y/ies) (add to `.zenko/filters.toml` and trust):",
                            items.len()
                        )];
                        for item in &items {
                            lines.push(format!(
                                "- `{}`: {} call(s), {} bytes raw -> ~{} missed tok",
                                item.command_prefix,
                                item.calls,
                                item.total_bytes,
                                item.estimated_missed_tokens
                            ));
                        }
                        lines.join("\n")
                    }
                };
                Ok(ok_with_meta(
                    text,
                    serde_json::to_value(&items).unwrap_or(Value::Null),
                ))
            }
            EconomyAction::VerifyFilters => {
                let rep = self.bridge.verify_custom_filters(&ctx.workspace_dir)?;
                let text = match rep.exists {
                    false => format!("No project filter file found at `{}`.", rep.file_path),
                    true => format!(
                        "Custom filters `{}`: valid={} · {} rule(s) · {} test(s) passed, {} failed{}",
                        rep.file_path,
                        rep.valid,
                        rep.rule_count,
                        rep.tests_passed,
                        rep.tests_failed,
                        match rep.errors.is_empty() {
                            true => String::new(),
                            false => format!("\nErrors:\n- {}", rep.errors.join("\n- ")),
                        }
                    ),
                };
                Ok(ok_with_meta(
                    text,
                    serde_json::to_value(&rep).unwrap_or(Value::Null),
                ))
            }
        }
    }
}
