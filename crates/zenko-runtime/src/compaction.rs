//! Phase 3 Two-Stage Context Compaction Engine (Oh My Pi + Claude Code + Codex synthesis).
//!
//! - **Stage A (`microcompact_messages`):** Zero-cost, zero-LLM selective collapse of bulky
//!   ephemeral tool results (`MICROCOMPACTABLE_TOOLS`) and stale `reasoning_content` outside the
//!   recent exchange tail, while strictly protecting planning/memory tools
//!   (`COMPACTION_PROTECTED_TOOLS`) and preserving `artifact:` handles.
//! - **Stage B (`slice_from_latest_checkpoint` + `plan_checkpoint_split`):** Persisted SQLite
//!   `EventKind::CompactionCheckpoint` with `first_kept_event_id`, iterative-merge structured
//!   summarization prompt, anti-thrash cooldown (`COMPACTION_MIN_EVENTS_BETWEEN_CHECKPOINTS`),
//!   and zero-LLM deterministic fallback.
//! - **Emergency Valve (`compact_messages_to_budget`):** In-memory pair-preserving hard ceiling.
//!
//! Zero regex, zero hardcoded tool names, strictly dynamic across all model context windows.

use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
use zenko_core::{
    ARTIFACT_PATH_PREFIX, COMPACTION_MIN_EVENTS_BETWEEN_CHECKPOINTS, COMPACTION_PROTECTED_TOOLS,
    ChatMessage, CompactionCheckpointPayload, DEFAULT_COMPACTION_KEEP_RECENT,
    DEFAULT_COMPACTION_SNIPPET_CHARS, DEFAULT_COMPACTION_TRIGGER_RATIO, EventKind, EventRole,
    MICROCOMPACT_MIN_TOOL_CHARS, MICROCOMPACT_PROTECT_RECENT_EXCHANGES, MICROCOMPACTABLE_TOOLS,
    SessionEvent, ToolSpec, compact_preview,
};

/// Approximate token count for plain UTF-8 text (conservative ~4 bytes/token).
pub fn estimate_text_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

pub fn estimate_message_tokens(msg: &ChatMessage) -> usize {
    let base = msg.content.len()
        + msg.reasoning_content.as_deref().map_or(0, str::len)
        + msg
            .tool_calls
            .iter()
            .map(|tc| tc.name.len() + tc.arguments.to_string().len())
            .sum::<usize>();
    base.div_ceil(4) + 8
}

pub fn estimate_messages_tokens(messages: &[ChatMessage]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

pub fn estimate_tool_specs_tokens(specs: &[ToolSpec]) -> usize {
    specs
        .iter()
        .map(|s| {
            let bytes = s.name.len() + s.description.len() + s.parameters.to_string().len();
            bytes.div_ceil(4) + 12
        })
        .sum()
}

fn estimate_event_tokens(ev: &SessionEvent) -> usize {
    match ev.role {
        EventRole::System => 0,
        _ => ev.content_json.to_string().len().div_ceil(4) + 8,
    }
}

struct ExchangeUnit {
    messages: Vec<ChatMessage>,
    est_tokens: usize,
    summary_line: String,
}

/// Groups a flat chat history into atomic exchanges so tool-call / tool-result sequences
/// stay indivisible during microcompaction and budget windowing.
fn group_atomic_exchanges(messages: Vec<ChatMessage>) -> Vec<ExchangeUnit> {
    let mut units: Vec<ExchangeUnit> = Vec::with_capacity(messages.len());
    let mut iter = messages.into_iter().peekable();

    while let Some(msg) = iter.next() {
        let expects_tools = msg.role == EventRole::Assistant && !msg.tool_calls.is_empty();
        let mut group = vec![msg];

        if expects_tools {
            while let Some(next) = iter.peek() {
                match next.role {
                    EventRole::Tool => {
                        if let Some(tool_msg) = iter.next() {
                            group.push(tool_msg);
                        }
                    }
                    _ => break,
                }
            }
        }

        let est_tokens = group.iter().map(estimate_message_tokens).sum();
        let summary_line = summarize_exchange(&group);
        units.push(ExchangeUnit {
            messages: group,
            est_tokens,
            summary_line,
        });
    }

    units
}

fn summarize_exchange(group: &[ChatMessage]) -> String {
    let Some(first) = group.first() else {
        return String::new();
    };
    match first.role {
        EventRole::User => format!(
            "- User: {}",
            compact_preview(&first.content, DEFAULT_COMPACTION_SNIPPET_CHARS)
        ),
        EventRole::Assistant => {
            let tools: Vec<&str> = first.tool_calls.iter().map(|tc| tc.name.as_str()).collect();
            match (tools.is_empty(), first.content.trim().is_empty()) {
                (true, _) => format!(
                    "- Assistant: {}",
                    compact_preview(&first.content, DEFAULT_COMPACTION_SNIPPET_CHARS)
                ),
                (false, true) => format!("- Assistant ran tools [{}]", tools.join(", ")),
                (false, false) => format!(
                    "- Assistant [{}]: {}",
                    tools.join(", "),
                    compact_preview(&first.content, DEFAULT_COMPACTION_SNIPPET_CHARS)
                ),
            }
        }
        EventRole::Tool => format!(
            "- Tool ({}): {}",
            first.name.as_deref().unwrap_or("tool"),
            compact_preview(&first.content, DEFAULT_COMPACTION_SNIPPET_CHARS)
        ),
        EventRole::System => format!(
            "- Context: {}",
            compact_preview(&first.content, DEFAULT_COMPACTION_SNIPPET_CHARS)
        ),
    }
}

// ─── Stage A: Zero-Cost Selective Microcompaction ─────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MicrocompactStats {
    pub compacted_tools: usize,
    pub stripped_reasoning_blocks: usize,
    pub tokens_saved: usize,
}

fn extract_artifact_handle(content: &str) -> Option<&str> {
    let idx = content.find(ARTIFACT_PATH_PREFIX)?;
    let tail = &content[idx..];
    let end = tail
        .find(|c: char| {
            c.is_whitespace() || matches!(c, '`' | '\'' | '"' | ')' | ']' | ',' | ';' | '}')
        })
        .unwrap_or(tail.len());
    let candidate = &tail[..end];
    match candidate.len() > ARTIFACT_PATH_PREFIX.len() {
        true => Some(candidate),
        false => None,
    }
}

fn extract_scoped_repo_rules_blocks(content: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel_start) = content[cursor..].find(zenko_core::DOX_SCOPED_RULES_TAG_OPEN) {
        let start = cursor + rel_start;
        let Some(rel_end) = content[start..].find(zenko_core::DOX_SCOPED_RULES_TAG_CLOSE) else {
            break;
        };
        let end = start + rel_end + zenko_core::DOX_SCOPED_RULES_TAG_CLOSE.len();
        blocks.push(&content[start..end]);
        cursor = end;
    }
    blocks
}

fn extract_tool_target_hint(args: Option<&Value>) -> String {
    let Some(val) = args else {
        return String::new();
    };
    const HINT_KEYS: &[&str] = &["path", "query", "command", "url", "job_id", "slug"];
    for key in HINT_KEYS {
        if let Some(s) = val.get(*key).and_then(Value::as_str).map(str::trim)
            && !s.is_empty() {
                return compact_preview(s, 72);
            }
    }
    String::new()
}

/// Stage-A lossless microcompaction:
/// When estimated prompt + history tokens exceed `context_window * trigger_ratio`, collapses
/// older bulky outputs of `MICROCOMPACTABLE_TOOLS` (and stale `reasoning_content`) outside the
/// protected recent tail into 1-line stubs preserving target hints and `artifact:` handles.
/// Strictly exempts `COMPACTION_PROTECTED_TOOLS` to prevent working-memory amnesia.
pub fn microcompact_messages(
    messages: Vec<ChatMessage>,
    context_window: u64,
    prefix_tokens: usize,
    trigger_ratio: f64,
) -> (Vec<ChatMessage>, MicrocompactStats) {
    let ratio = trigger_ratio.clamp(0.25, 0.90);
    let trigger_budget = ((context_window as f64) * ratio) as usize;
    let mut units = group_atomic_exchanges(messages);
    let history_tokens: usize = units.iter().map(|u| u.est_tokens).sum();

    if prefix_tokens + history_tokens <= trigger_budget
        || units.len() <= MICROCOMPACT_PROTECT_RECENT_EXCHANGES
    {
        return (
            units.into_iter().flat_map(|u| u.messages).collect(),
            MicrocompactStats::default(),
        );
    }

    let protected_start = units.len().saturating_sub(MICROCOMPACT_PROTECT_RECENT_EXCHANGES);
    let mut stats = MicrocompactStats::default();

    for unit in units.iter_mut().take(protected_start) {
        let mut call_meta: HashMap<String, (String, Value)> = HashMap::new();
        for msg in &mut unit.messages {
            match msg.role {
                EventRole::Assistant => {
                    for tc in &msg.tool_calls {
                        call_meta.insert(tc.id.clone(), (tc.name.clone(), tc.arguments.clone()));
                    }
                    if let Some(reasoning) = msg.reasoning_content.take() {
                        stats.stripped_reasoning_blocks += 1;
                        stats.tokens_saved += reasoning.len().div_ceil(4);
                    }
                }
                EventRole::Tool => {
                    let meta = msg
                        .tool_call_id
                        .as_deref()
                        .and_then(|id| call_meta.get(id));
                    let tool_name = msg
                        .name
                        .as_deref()
                        .or_else(|| meta.map(|(n, _)| n.as_str()))
                        .unwrap_or("");
                    if COMPACTION_PROTECTED_TOOLS.contains(&tool_name)
                        || !MICROCOMPACTABLE_TOOLS.contains(&tool_name)
                        || msg.content.len() < MICROCOMPACT_MIN_TOOL_CHARS
                    {
                        continue;
                    }
                    let orig_bytes = msg.content.len();
                    let orig_lines = msg.content.lines().count();
                    let target = extract_tool_target_hint(meta.map(|(_, a)| a));
                    let target_part = match target.is_empty() {
                        true => String::new(),
                        false => format!(" `{target}`"),
                    };
                    let preserved_rules: Vec<String> = extract_scoped_repo_rules_blocks(&msg.content)
                        .into_iter()
                        .map(String::from)
                        .collect();
                    let base_stub = match extract_artifact_handle(&msg.content) {
                        Some(handle) => format!(
                            "[microcompacted `{tool_name}`{target_part} ({orig_lines} lines, {orig_bytes}B) — inspect full output with `read_file` at `{handle}`]"
                        ),
                        None => format!(
                            "[microcompacted `{tool_name}`{target_part} ({orig_lines} lines, {orig_bytes}B) — re-read if needed]"
                        ),
                    };
                    let stub = match preserved_rules.is_empty() {
                        true => base_stub,
                        false => format!("{base_stub}\n\n{}", preserved_rules.join("\n\n")),
                    };
                    if stub.len() >= orig_bytes {
                        continue;
                    }
                    let saved = orig_bytes.saturating_sub(stub.len()) / 4;
                    msg.content = stub;
                    stats.compacted_tools += 1;
                    stats.tokens_saved += saved;
                }
                EventRole::User | EventRole::System => {}
            }
        }
    }

    let out = units.into_iter().flat_map(|u| u.messages).collect();
    (out, stats)
}

// ─── Stage B: Persisted Checkpoint Slicing & Iterative-Merge Planner ──────────

pub struct CheckpointSlice<'a> {
    pub checkpoint_event_id: Option<String>,
    pub checkpoint: Option<CompactionCheckpointPayload>,
    /// Chronological events starting from `first_kept_event_id` (excluding checkpoint markers).
    pub kept_events: Vec<&'a SessionEvent>,
    /// Non-system events appended AFTER the latest checkpoint (drives anti-thrash cooldown).
    pub events_since_checkpoint: usize,
    pub checkpoints_count: usize,
}

/// Locates the latest persisted `EventKind::CompactionCheckpoint` on the active branch and
/// slices events starting at its `first_kept_event_id`, guaranteeing lossless chronological order.
pub fn slice_from_latest_checkpoint<'a>(events: &'a [SessionEvent]) -> CheckpointSlice<'a> {
    let mut checkpoints_count = 0usize;
    let mut latest_ckpt: Option<(usize, &SessionEvent, CompactionCheckpointPayload)> = None;

    for (idx, ev) in events.iter().enumerate() {
        if ev.kind == EventKind::CompactionCheckpoint {
            checkpoints_count += 1;
            if let Ok(payload) =
                serde_json::from_value::<CompactionCheckpointPayload>(ev.content_json.clone())
            {
                latest_ckpt = Some((idx, ev, payload));
            }
        }
    }

    let Some((ckpt_idx, ckpt_ev, payload)) = latest_ckpt else {
        return CheckpointSlice {
            checkpoint_event_id: None,
            checkpoint: None,
            kept_events: events
                .iter()
                .filter(|e| e.kind != EventKind::CompactionCheckpoint)
                .collect(),
            events_since_checkpoint: events
                .iter()
                .filter(|e| e.role != EventRole::System)
                .count(),
            checkpoints_count,
        };
    };

    let start_idx = events[..ckpt_idx]
        .iter()
        .position(|e| e.id == payload.first_kept_event_id)
        .unwrap_or(ckpt_idx + 1);

    let kept_events: Vec<&SessionEvent> = events[start_idx..]
        .iter()
        .filter(|e| e.kind != EventKind::CompactionCheckpoint)
        .collect();
    let events_since_checkpoint = events[(ckpt_idx + 1)..]
        .iter()
        .filter(|e| e.role != EventRole::System)
        .count();

    CheckpointSlice {
        checkpoint_event_id: Some(ckpt_ev.id.clone()),
        checkpoint: Some(payload),
        kept_events,
        events_since_checkpoint,
        checkpoints_count,
    }
}

/// Injects a persisted checkpoint summary at the head of `messages[]` without mutating the
/// Tier-1/Tier-2 system prompt and without creating adjacent `user, user` turns.
pub fn prepend_checkpoint_to_messages(
    messages: &mut Vec<ChatMessage>,
    checkpoint: &CompactionCheckpointPayload,
) {
    let block = format!(
        "<compacted_checkpoint compacted_events=\"{}\" model=\"{}\">\n{}\n</compacted_checkpoint>",
        checkpoint.compacted_events_count, checkpoint.model, checkpoint.summary
    );
    match messages.first_mut() {
        Some(first) if first.role == EventRole::User => {
            first.content = format!("{block}\n\n{}", first.content);
        }
        _ => {
            messages.insert(
                0,
                ChatMessage {
                    role: EventRole::User,
                    content: block,
                    reasoning_content: None,
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    name: None,
                },
            );
        }
    }
}

struct EventUnit<'a> {
    events: Vec<&'a SessionEvent>,
    est_tokens: usize,
    first_non_system_id: Option<String>,
}

fn group_atomic_event_units<'a>(events: &[&'a SessionEvent]) -> Vec<EventUnit<'a>> {
    let mut units: Vec<EventUnit<'a>> = Vec::with_capacity(events.len());
    let mut idx = 0usize;

    while idx < events.len() {
        let ev = events[idx];
        let mut group = vec![ev];
        idx += 1;

        let expects_tools = ev.role == EventRole::Assistant
            && ev
                .content_json
                .get("tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty());

        if expects_tools {
            while idx < events.len() {
                match events[idx].role {
                    EventRole::Tool | EventRole::System => {
                        group.push(events[idx]);
                        idx += 1;
                    }
                    _ => break,
                }
            }
        }

        let est_tokens = group.iter().map(|e| estimate_event_tokens(e)).sum();
        let first_non_system_id = group
            .iter()
            .find(|e| e.role != EventRole::System)
            .map(|e| e.id.clone());

        units.push(EventUnit {
            events: group,
            est_tokens,
            first_non_system_id,
        });
    }

    units
}

pub struct CheckpointPlan<'a> {
    pub events_to_compact: Vec<&'a SessionEvent>,
    pub first_kept_event_id: String,
    pub compacted_events_count: usize,
    pub estimated_tokens_before: usize,
    pub estimated_kept_tokens: usize,
}

/// Decides whether Stage-B persisted compaction should run and computes a pair-safe split boundary
/// while enforcing anti-thrash cooldown (`COMPACTION_MIN_EVENTS_BETWEEN_CHECKPOINTS`).
pub fn plan_checkpoint_split<'a>(
    slice: &CheckpointSlice<'a>,
    context_window: u64,
    prefix_tokens: usize,
    trigger_ratio: f64,
) -> Option<CheckpointPlan<'a>> {
    if slice.checkpoint.is_some()
        && slice.events_since_checkpoint < COMPACTION_MIN_EVENTS_BETWEEN_CHECKPOINTS
    {
        return None;
    }

    let ratio = trigger_ratio.clamp(0.45, 0.94);
    let total_budget = ((context_window as f64) * ratio) as usize;
    let units = group_atomic_event_units(&slice.kept_events);
    if units.len() <= 3 {
        return None;
    }

    let prior_ckpt_tokens = slice
        .checkpoint
        .as_ref()
        .map_or(0, |c| estimate_text_tokens(&c.summary));
    let active_tokens: usize = units.iter().map(|u| u.est_tokens).sum::<usize>() + prior_ckpt_tokens;
    if prefix_tokens + active_tokens <= total_budget {
        return None;
    }

    // Keep roughly 35% of available history budget as verbatim recent tail (minimum 2 units)
    let available = total_budget.saturating_sub(prefix_tokens).max(2_048);
    let tail_budget = ((available * 35) / 100).max(1_024);

    let mut kept_tokens = 0usize;
    let mut keep_from = units.len();
    for idx in (0..units.len()).rev() {
        let kept_units = units.len() - idx;
        let next_tokens = kept_tokens + units[idx].est_tokens;
        if kept_units > 2 && next_tokens > tail_budget {
            break;
        }
        kept_tokens = next_tokens;
        keep_from = idx;
    }

    // Ensure at least one unit is compacted and the kept boundary has a real event ID
    if keep_from == 0 || keep_from >= units.len() {
        return None;
    }

    let first_kept_event_id = units[keep_from..]
        .iter()
        .find_map(|u| u.first_non_system_id.clone())?;

    let events_to_compact: Vec<&SessionEvent> = units[..keep_from]
        .iter()
        .flat_map(|u| u.events.iter().copied())
        .filter(|e| e.role != EventRole::System)
        .collect();
    if events_to_compact.is_empty() {
        return None;
    }

    let compacted_events_count = events_to_compact.len()
        + slice
            .checkpoint
            .as_ref()
            .map_or(0, |c| c.compacted_events_count);

    Some(CheckpointPlan {
        events_to_compact,
        first_kept_event_id,
        compacted_events_count,
        estimated_tokens_before: prefix_tokens + active_tokens,
        estimated_kept_tokens: kept_tokens,
    })
}

fn format_event_for_compactor(ev: &SessionEvent) -> String {
    match ev.role {
        EventRole::User => format!(
            "[USER] {}",
            compact_preview(
                ev.content_json
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                480
            )
        ),
        EventRole::Assistant => {
            let text = ev
                .content_json
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("");
            let calls: Vec<&str> = ev
                .content_json
                .get("tool_calls")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.get("name").and_then(Value::as_str))
                        .collect()
                })
                .unwrap_or_default();
            match (calls.is_empty(), text.trim().is_empty()) {
                (true, _) => format!("[ASSISTANT] {}", compact_preview(text, 480)),
                (false, true) => format!("[ASSISTANT tools={}]", calls.join(",")),
                (false, false) => format!(
                    "[ASSISTANT tools={}] {}",
                    calls.join(","),
                    compact_preview(text, 360)
                ),
            }
        }
        EventRole::Tool => {
            let name = ev
                .content_json
                .get("tool_name")
                .and_then(Value::as_str)
                .unwrap_or("tool");
            let is_err = ev
                .content_json
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let content = ev
                .content_json
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("");
            let status = match is_err {
                true => "ERR",
                false => "OK",
            };
            let artifact_suffix = match extract_artifact_handle(content) {
                Some(h) => format!(" (artifact={h})"),
                None => String::new(),
            };
            format!(
                "[TOOL {name} {status}{artifact_suffix}] {}",
                compact_preview(content, 360)
            )
        }
        EventRole::System => String::new(),
    }
}

/// Builds the iterative-merge compaction prompt (System + User transcript) for `compaction_model`.
pub fn build_compaction_prompt(
    plan: &CheckpointPlan<'_>,
    prior_checkpoint: Option<&CompactionCheckpointPayload>,
    active_plan_summary: Option<&str>,
) -> (String, String) {
    let sys = "You are Zenko's lossless context compactor. Merge the prior checkpoint (if any) and the new transcript segment into ONE dense, factual, structured handoff checkpoint.\n\
               STRICT RULES:\n\
               - Never invent file paths, symbol names, or test results.\n\
               - Preserve exact file paths, function/struct names, error messages, and any `artifact:...` handles verbatim.\n\
               - Keep active constraints, user preferences, decisions, and unfinished plan steps intact.\n\
               - Output ONLY the following concise Markdown sections (target <= 900 words):\n\
                 ## Goal & Scope\n\
                 ## Completed Work\n\
                 ## Key Files, Symbols & Artifacts\n\
                 ## Active Constraints & Decisions\n\
                 ## Open Issues & Blockers\n\
                 ## Immediate Next Steps"
        .to_string();

    let mut body = String::with_capacity(4096);
    if let Some(prev) = prior_checkpoint {
        body.push_str("<prior_checkpoint>\n");
        body.push_str(&prev.summary);
        body.push_str("\n</prior_checkpoint>\n\n");
    }
    if let Some(plan_txt) = active_plan_summary {
        body.push_str("<active_execution_plan>\n");
        body.push_str(plan_txt);
        body.push_str("\n</active_execution_plan>\n\n");
    }
    body.push_str("<transcript_to_merge>\n");
    const MAX_TRANSCRIPT_CHARS: usize = 28_000;
    for ev in &plan.events_to_compact {
        let line = format_event_for_compactor(ev);
        if line.is_empty() {
            continue;
        }
        if body.len() + line.len() + 48 > MAX_TRANSCRIPT_CHARS {
            body.push_str("... (additional middle entries condensed)\n");
            break;
        }
        body.push_str(&line);
        body.push('\n');
    }
    body.push_str("</transcript_to_merge>");
    (sys, body)
}

/// Zero-LLM deterministic fallback checkpoint builder used if the compaction LLM call fails.
pub fn build_deterministic_checkpoint(
    plan: &CheckpointPlan<'_>,
    prior_checkpoint: Option<&CompactionCheckpointPayload>,
    active_plan_summary: Option<&str>,
    prefix_tokens: usize,
) -> CompactionCheckpointPayload {
    let mut summary = String::new();
    if let Some(prev) = prior_checkpoint {
        summary.push_str("## Prior Checkpoint\n");
        summary.push_str(&compact_preview(&prev.summary, 2_400));
        summary.push_str("\n\n");
    }
    if let Some(plan_txt) = active_plan_summary {
        summary.push_str("## Active Execution Plan\n");
        summary.push_str(plan_txt);
        summary.push_str("\n\n");
    }
    summary.push_str("## Compacted Transcript\n");
    const MAX_DETERMINISTIC_CHARS: usize = 4_800;
    for ev in &plan.events_to_compact {
        let line = format_event_for_compactor(ev);
        if line.is_empty() {
            continue;
        }
        if summary.len() + line.len() + 32 > MAX_DETERMINISTIC_CHARS {
            summary.push_str("- ... (earlier entries omitted)\n");
            break;
        }
        summary.push_str("- ");
        summary.push_str(&line);
        summary.push('\n');
    }
    let after = prefix_tokens + plan.estimated_kept_tokens + estimate_text_tokens(&summary);
    CompactionCheckpointPayload {
        version: 1,
        summary,
        first_kept_event_id: plan.first_kept_event_id.clone(),
        compacted_events_count: plan.compacted_events_count,
        estimated_tokens_before: plan.estimated_tokens_before,
        estimated_tokens_after: after,
        model: "deterministic".into(),
        created_at: Utc::now(),
    }
}

// ─── Emergency Hard-Ceiling Pair-Preserving Budget Valve ──────────────────────

/// Compacts older exchanges into a rolling `<compacted_history>` summary when estimated tokens
/// still exceed `context_window * DEFAULT_COMPACTION_TRIGGER_RATIO` after Stage A/B (e.g., during
/// Stage-B anti-thrash cooldown), while keeping the most recent exchanges verbatim and never
/// orphaning tool-result messages.
pub fn compact_messages_to_budget(
    messages: Vec<ChatMessage>,
    context_window: u64,
    system_prompt_tokens: usize,
) -> Vec<ChatMessage> {
    let total_budget = ((context_window as f64) * DEFAULT_COMPACTION_TRIGGER_RATIO) as usize;
    let available = total_budget.saturating_sub(system_prompt_tokens).max(2_048);

    let units = group_atomic_exchanges(messages);
    let total_tokens: usize = units.iter().map(|u| u.est_tokens).sum();
    if total_tokens <= available || units.len() <= 2 {
        return units.into_iter().flat_map(|u| u.messages).collect();
    }

    let recent_budget = (available * 3) / 4;
    let mut kept_tokens = 0usize;
    let mut keep_from = units.len();

    for idx in (0..units.len()).rev() {
        let next_tokens = kept_tokens + units[idx].est_tokens;
        let kept_count = units.len() - idx;
        let must_keep_tail = kept_count <= 2;
        if !must_keep_tail
            && kept_count > DEFAULT_COMPACTION_KEEP_RECENT
            && next_tokens > recent_budget
        {
            break;
        }
        if !must_keep_tail && next_tokens > recent_budget {
            break;
        }
        kept_tokens = next_tokens;
        keep_from = idx;
    }

    if keep_from == 0 {
        return units.into_iter().flat_map(|u| u.messages).collect();
    }

    let summary_char_cap = (available / 4).saturating_mul(4).clamp(1_200, 12_000);
    let mut summary = String::from(
        "<compacted_history>\nEarlier conversation turns compacted to fit context budget:\n",
    );
    for unit in &units[..keep_from] {
        if summary.len() + unit.summary_line.len() + 24 > summary_char_cap {
            summary.push_str("... (older turns omitted)\n");
            break;
        }
        summary.push_str(&unit.summary_line);
        summary.push('\n');
    }
    summary.push_str("</compacted_history>");

    let mut out = Vec::with_capacity(units.len().saturating_sub(keep_from) * 2 + 1);
    let mut tail: Vec<ChatMessage> = units
        .into_iter()
        .skip(keep_from)
        .flat_map(|u| u.messages)
        .collect();
    match tail.first_mut() {
        Some(first) if first.role == EventRole::User => {
            first.content = format!("{summary}\n\n{}", first.content);
            out.extend(tail);
        }
        _ => {
            out.push(ChatMessage {
                role: EventRole::User,
                content: summary,
                reasoning_content: None,
                tool_calls: Vec::new(),
                tool_call_id: None,
                name: None,
            });
            out.extend(tail);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use zenko_core::{TOOL_PLAN_UPDATE, ToolCall};

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: EventRole::User,
            content: text.into(),
            reasoning_content: None,
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
        }
    }

    fn mk_event(id: &str, role: EventRole, kind: EventKind, content_json: Value) -> SessionEvent {
        SessionEvent {
            id: id.into(),
            session_id: "s1".into(),
            parent_id: None,
            branch_index: 0,
            role,
            kind,
            content_json,
            tokens_in: 0,
            tokens_out: 0,
            cache_read_tokens: 0,
            cost_usd: 0.0,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn compaction_preserves_tool_pairs_and_bounds_long_history() {
        let mut msgs = Vec::new();
        for i in 0..30 {
            msgs.push(user_msg(&format!("Turn {i}: {}", "x".repeat(400))));
            msgs.push(ChatMessage {
                role: EventRole::Assistant,
                content: String::new(),
                reasoning_content: None,
                tool_calls: vec![ToolCall {
                    id: format!("call_{i}"),
                    name: "read_file".into(),
                    arguments: json!({"path": "Cargo.toml"}),
                }],
                tool_call_id: None,
                name: None,
            });
            msgs.push(ChatMessage {
                role: EventRole::Tool,
                content: format!("file content {i} {}", "y".repeat(400)),
                reasoning_content: None,
                tool_calls: Vec::new(),
                tool_call_id: Some(format!("call_{i}")),
                name: Some("read_file".into()),
            });
        }

        let compacted = compact_messages_to_budget(msgs, 4_000, 400);
        assert!(compacted.len() < 40);
        assert_eq!(compacted[0].role, EventRole::User);
        assert!(compacted[0].content.contains("<compacted_history>"));

        for window in compacted.windows(2) {
            if window[1].role == EventRole::Tool {
                assert!(
                    window[0].role == EventRole::Tool
                        || (window[0].role == EventRole::Assistant
                            && !window[0].tool_calls.is_empty()),
                    "found orphaned tool message after {:?}",
                    window[0].role
                );
            }
        }
    }

    #[test]
    fn stage_a_microcompaction_collapses_old_reads_preserves_protected_tools_and_artifacts() {
        let mut msgs = Vec::new();
        // Old read_file with spilled artifact handle -> MUST microcompact & preserve handle
        msgs.push(user_msg("inspect big file"));
        msgs.push(ChatMessage {
            role: EventRole::Assistant,
            content: String::new(),
            reasoning_content: Some("long stale thinking ".repeat(100)),
            tool_calls: vec![ToolCall {
                id: "c_read".into(),
                name: "read_file".into(),
                arguments: json!({"path": "src/main.rs"}),
            }],
            tool_call_id: None,
            name: None,
        });
        msgs.push(ChatMessage {
            role: EventRole::Tool,
            content: format!(
                "{}\n[spilled full `read_file` output -> `artifact:s1/c_read.txt`]",
                "line\n".repeat(400)
            ),
            reasoning_content: None,
            tool_calls: Vec::new(),
            tool_call_id: Some("c_read".into()),
            name: Some("read_file".into()),
        });

        // Old protected plan_update -> MUST NEVER microcompact
        msgs.push(ChatMessage {
            role: EventRole::Assistant,
            content: String::new(),
            reasoning_content: None,
            tool_calls: vec![ToolCall {
                id: "c_plan".into(),
                name: TOOL_PLAN_UPDATE.into(),
                arguments: json!({"goal": "ship phase 3"}),
            }],
            tool_call_id: None,
            name: None,
        });
        let big_plan = format!("PLAN_PAYLOAD: {}", "step detail ".repeat(200));
        msgs.push(ChatMessage {
            role: EventRole::Tool,
            content: big_plan.clone(),
            reasoning_content: None,
            tool_calls: Vec::new(),
            tool_call_id: Some("c_plan".into()),
            name: Some(TOOL_PLAN_UPDATE.into()),
        });

        // 4 recent protected exchanges -> MUST remain untouched
        for i in 0..MICROCOMPACT_PROTECT_RECENT_EXCHANGES {
            msgs.push(user_msg(&format!("recent turn {i}")));
        }

        let (out, stats) = microcompact_messages(msgs, 2_000, 600, 0.50);
        assert_eq!(stats.compacted_tools, 1);
        assert_eq!(stats.stripped_reasoning_blocks, 1);
        assert!(stats.tokens_saved > 200);

        let read_res = out
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some("c_read"))
            .expect("c_read kept");
        assert!(read_res.content.contains("[microcompacted `read_file`"));
        assert!(read_res.content.contains("src/main.rs"));
        assert!(read_res.content.contains("artifact:s1/c_read.txt"));

        let plan_res = out
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some("c_plan"))
            .expect("c_plan kept");
        assert_eq!(plan_res.content, big_plan, "protected plan_update must never microcompact");
    }

    #[test]
    fn stage_a_microcompaction_preserves_scoped_repo_rules_blocks() {
        let mut msgs = Vec::new();
        msgs.push(user_msg("inspect api routes"));
        msgs.push(ChatMessage {
            role: EventRole::Assistant,
            content: String::new(),
            reasoning_content: None,
            tool_calls: vec![ToolCall {
                id: "c_scoped".into(),
                name: "read_file".into(),
                arguments: json!({"path": "crates/api/src/routes.rs"}),
            }],
            tool_call_id: None,
            name: None,
        });
        let scoped_block = "<scoped_repo_rules path=\"crates/api/AGENTS.md\">\n# API\nAlways validate auth header.\n</scoped_repo_rules>";
        let bulky_body = format!("{}\n\n{scoped_block}", "pub fn route() {}\n".repeat(120));
        msgs.push(ChatMessage {
            role: EventRole::Tool,
            content: bulky_body,
            reasoning_content: None,
            tool_calls: Vec::new(),
            tool_call_id: Some("c_scoped".into()),
            name: Some("read_file".into()),
        });

        for i in 0..MICROCOMPACT_PROTECT_RECENT_EXCHANGES {
            msgs.push(user_msg(&format!("later turn {i}")));
        }

        let (out, stats) = microcompact_messages(msgs, 1_800, 500, 0.50);
        assert_eq!(stats.compacted_tools, 1);
        let compacted = out
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some("c_scoped"))
            .expect("c_scoped kept");
        assert!(compacted.content.contains("[microcompacted `read_file`"));
        assert!(
            compacted.content.contains(scoped_block),
            "microcompaction must preserve JIT scoped rules: {}",
            compacted.content
        );
        assert!(!compacted.content.contains("pub fn route() {}"));
    }

    #[test]
    fn stage_b_checkpoint_slicing_and_antithrash_cooldown() {
        let mut events = Vec::new();
        for i in 0..8 {
            events.push(mk_event(
                &format!("u_{i}"),
                EventRole::User,
                EventKind::Message,
                json!({"text": format!("User turn {i}: {}", "z".repeat(800))}),
            ));
            events.push(mk_event(
                &format!("a_{i}"),
                EventRole::Assistant,
                EventKind::Message,
                json!({"text": format!("Reply {i}: {}", "w".repeat(800))}),
            ));
        }

        let slice = slice_from_latest_checkpoint(&events);
        assert!(slice.checkpoint.is_none());
        let plan = plan_checkpoint_split(&slice, 4_000, 500, 0.75).expect("should plan checkpoint");
        assert!(!plan.events_to_compact.is_empty());

        let ckpt = build_deterministic_checkpoint(&plan, None, Some("goal: test"), 500);
        events.push(mk_event(
            "ckpt_1",
            EventRole::System,
            EventKind::CompactionCheckpoint,
            serde_json::to_value(&ckpt).expect("serialize ckpt"),
        ));
        events.push(mk_event(
            "u_after",
            EventRole::User,
            EventKind::Message,
            json!({"text": "next step"}),
        ));

        let slice2 = slice_from_latest_checkpoint(&events);
        assert_eq!(slice2.checkpoints_count, 1);
        assert_eq!(slice2.checkpoint_event_id.as_deref(), Some("ckpt_1"));
        assert_eq!(slice2.events_since_checkpoint, 1);
        assert_eq!(slice2.kept_events.first().map(|e| e.id.as_str()), Some(ckpt.first_kept_event_id.as_str()));
        assert_eq!(slice2.kept_events.last().map(|e| e.id.as_str()), Some("u_after"));
        assert!(
            plan_checkpoint_split(&slice2, 1_000, 500, 0.50).is_none(),
            "anti-thrash cooldown must block immediate re-compaction"
        );
    }
}
