//! Two-Stage Context Compaction & Overflow-Recovery Engine
//! (Oh My Pi + Claude Code + Codex + Hermes + Prime Agent synthesis).
//!
//! - **Calibrated Token Budgeting:** Subtracts capped output-token reservation from `context_window`
//!   and supports per-session EMA bytes-per-token calibration from real provider `usage.prompt_tokens`.
//! - **Stage A (`microcompact_messages`):** Zero-cost, zero-LLM selective collapse of bulky
//!   ephemeral tool results (unwrapping `invoke_tool` & `batch_tools`!) using **quantized epoch
//!   boundaries** (`MICROCOMPACT_EPOCH_STEP`) so provider KV caches stay warm across turns instead
//!   of shifting every iteration.
//! - **Stage B (`slice_from_latest_checkpoint` + `plan_checkpoint_split`):** Persisted SQLite
//!   `EventKind::CompactionCheckpoint` carrying a **Hermes-style verbatim Turn-1 goal anchor**,
//!   an **Oh My Pi-style deterministic cumulative file & artifact ledger**, **newest-preserving**
//!   transcript packing, anti-thrash cooldown (bypassable on reactive overflow), and zero-LLM fallback.
//! - **Emergency Valve (`compact_messages_to_budget_with_ratio`):** Honors active `ZMode`/config
//!   ratios, shrinks oversized single-wave tool payloads even when `units.len() <= 2`, and pairs with
//!   `sanitize_message_tool_pairs` to guarantee 100% protocol-valid wire requests.
//! - **Abandoned-Branch Summarizer (`build_abandoned_branch_summary`):** Zero-LLM distillation of
//!   abandoned fork lessons on tree regeneration/retry.
//!
//! Zero regex, zero `if/else` ladders, zero hardcoded strings, zero `.unwrap()` in production paths.

use chrono::Utc;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use zenko_core::{
    ARTIFACT_PATH_PREFIX, BRANCH_SUMMARY_MAX_CHARS, BRANCH_SUMMARY_MAX_EVENTS,
    CODEX_PATCH_ADD_FILE, CODEX_PATCH_DELETE_FILE, CODEX_PATCH_UPDATE_FILE,
    COMPACTION_ANCHOR_GOAL_MAX_CHARS, COMPACTION_EMERGENCY_SINGLE_MSG_MAX_CHARS,
    COMPACTION_LEDGER_MAX_ARTIFACTS, COMPACTION_LEDGER_MAX_FILES_PER_KIND,
    COMPACTION_MAX_DETERMINISTIC_CHARS, COMPACTION_MAX_OUTPUT_RESERVE_DEN,
    COMPACTION_MAX_OUTPUT_RESERVE_NUM, COMPACTION_MAX_TRANSCRIPT_CHARS,
    COMPACTION_MIN_EVENTS_BETWEEN_CHECKPOINTS, COMPACTION_MIN_USABLE_INPUT_TOKENS,
    COMPACTION_PROTECTED_TOOLS, ChatMessage, CompactionCheckpointPayload, CompactionFileLedger,
    DEFAULT_COMPACTION_SNIPPET_CHARS, DEFAULT_COMPACTION_TRIGGER_RATIO, EventKind, EventRole,
    MICROCOMPACT_EPOCH_STEP, MICROCOMPACT_MIN_MARGINAL_TOKENS, MICROCOMPACT_MIN_TOOL_CHARS,
    MICROCOMPACT_MIN_WRITE_ARG_CHARS, MICROCOMPACT_PROTECT_RECENT_EXCHANGES,
    MICROCOMPACTABLE_TOOLS, PROVIDER_CONTEXT_OVERFLOW_SIGNATURES, SessionEvent,
    TOKEN_CALIBRATION_DEFAULT_BYTES_PER_TOK, TOKEN_CALIBRATION_EMA_ALPHA,
    TOKEN_CALIBRATION_MAX_BYTES_PER_TOK, TOKEN_CALIBRATION_MIN_BYTES_PER_TOK,
    TOKEN_CALIBRATION_MIN_SAMPLE_BYTES, TOOL_APPLY_PATCH, TOOL_BATCH_TOOLS, TOOL_CODE_REFACTOR,
    TOOL_HASH_EDIT, TOOL_INVOKE_TOOL, TOOL_KNOWLEDGE_CATALOG, TOOL_READ_FILE, TOOL_WRITE_FILE,
    ToolSpec, UNIFIED_DIFF_NEW_PREFIX, ZenkoError, compact_preview,
};

// ─── Calibrated Token Math & Usable Input Window ──────────────────────────────

#[inline]
pub fn clamp_bytes_per_token(bpt: f64) -> f64 {
    match bpt.is_finite() {
        true => bpt.clamp(
            TOKEN_CALIBRATION_MIN_BYTES_PER_TOK,
            TOKEN_CALIBRATION_MAX_BYTES_PER_TOK,
        ),
        false => TOKEN_CALIBRATION_DEFAULT_BYTES_PER_TOK,
    }
}

#[inline]
pub fn bytes_to_tokens(bytes: usize, bytes_per_tok: f64) -> usize {
    if bytes == 0 {
        return 0;
    }
    let denom = clamp_bytes_per_token(bytes_per_tok);
    ((bytes as f64) / denom).ceil() as usize
}

/// Computes the usable input token ceiling after reserving output-generation headroom
/// (capped at `30%` of `context_window` so small windows never starve input).
pub fn usable_input_window(context_window: u64, reserve_output_tokens: u32) -> usize {
    let max_reserve = (context_window
        .saturating_mul(COMPACTION_MAX_OUTPUT_RESERVE_NUM))
        / COMPACTION_MAX_OUTPUT_RESERVE_DEN;
    let effective_reserve = (reserve_output_tokens as u64).min(max_reserve);
    (context_window.saturating_sub(effective_reserve) as usize)
        .max(COMPACTION_MIN_USABLE_INPUT_TOKENS.min(context_window as usize))
}

/// Computes the proactive compaction window by clamping ultra-large model windows (e.g. 1M–2M)
/// to the active `ZMode` soft quality/economy ceiling while never exceeding `raw_context_window`.
/// Physical hard-overflow recovery still uses `raw_context_window`.
#[inline]
pub fn effective_compaction_window(raw_context_window: u64, soft_cap: u64) -> u64 {
    match soft_cap > 0 {
        true => raw_context_window.min(soft_cap),
        false => raw_context_window,
    }
}

/// Updates a session's EMA bytes-per-token calibration from live provider `usage.prompt_tokens`.
pub fn calibrate_bytes_per_token(
    current_bpt: f64,
    total_request_bytes: usize,
    actual_prompt_tokens: u64,
) -> f64 {
    let cur = clamp_bytes_per_token(current_bpt);
    if total_request_bytes < TOKEN_CALIBRATION_MIN_SAMPLE_BYTES || actual_prompt_tokens == 0 {
        return cur;
    }
    let observed = clamp_bytes_per_token(total_request_bytes as f64 / actual_prompt_tokens as f64);
    clamp_bytes_per_token(
        cur * (1.0 - TOKEN_CALIBRATION_EMA_ALPHA) + observed * TOKEN_CALIBRATION_EMA_ALPHA,
    )
}

/// Returns `true` when a provider error string matches known context-window overflow signatures
/// (pure case-insensitive table match, zero regex).
pub fn is_provider_context_overflow_error(err: &ZenkoError) -> bool {
    let lower = err.to_string().to_ascii_lowercase();
    PROVIDER_CONTEXT_OVERFLOW_SIGNATURES
        .iter()
        .any(|sig| lower.contains(sig))
}

/// Approximate token count for plain UTF-8 text using calibrated default density (`3.5 B/tok`).
pub fn estimate_text_tokens(text: &str) -> usize {
    bytes_to_tokens(text.len(), TOKEN_CALIBRATION_DEFAULT_BYTES_PER_TOK)
}

pub fn estimate_text_tokens_calibrated(text: &str, bytes_per_tok: f64) -> usize {
    bytes_to_tokens(text.len(), bytes_per_tok)
}

pub fn message_raw_bytes(msg: &ChatMessage) -> usize {
    msg.content.len()
        + msg.reasoning_content.as_deref().map_or(0, str::len)
        + msg
            .tool_calls
            .iter()
            .map(|tc| tc.name.len() + tc.arguments.to_string().len())
            .sum::<usize>()
}

pub fn estimate_message_tokens(msg: &ChatMessage) -> usize {
    estimate_message_tokens_calibrated(msg, TOKEN_CALIBRATION_DEFAULT_BYTES_PER_TOK)
}

pub fn estimate_message_tokens_calibrated(msg: &ChatMessage, bytes_per_tok: f64) -> usize {
    bytes_to_tokens(message_raw_bytes(msg), bytes_per_tok) + 8
}

pub fn estimate_messages_tokens(messages: &[ChatMessage]) -> usize {
    estimate_messages_tokens_calibrated(messages, TOKEN_CALIBRATION_DEFAULT_BYTES_PER_TOK)
}

pub fn estimate_messages_tokens_calibrated(messages: &[ChatMessage], bytes_per_tok: f64) -> usize {
    messages
        .iter()
        .map(|m| estimate_message_tokens_calibrated(m, bytes_per_tok))
        .sum()
}

pub fn tool_specs_raw_bytes(specs: &[ToolSpec]) -> usize {
    specs
        .iter()
        .map(|s| s.name.len() + s.description.len() + s.parameters.to_string().len())
        .sum()
}

pub fn estimate_tool_specs_tokens(specs: &[ToolSpec]) -> usize {
    estimate_tool_specs_tokens_calibrated(specs, TOKEN_CALIBRATION_DEFAULT_BYTES_PER_TOK)
}

pub fn estimate_tool_specs_tokens_calibrated(specs: &[ToolSpec], bytes_per_tok: f64) -> usize {
    specs
        .iter()
        .map(|s| {
            let bytes = s.name.len() + s.description.len() + s.parameters.to_string().len();
            bytes_to_tokens(bytes, bytes_per_tok) + 12
        })
        .sum()
}

/// Estimates tokens for a persisted `SessionEvent` using only wire-visible fields (never
/// double-counting `duration_ms`, `read_less` metadata, or JSON wrapper overhead).
fn estimate_event_tokens_calibrated(ev: &SessionEvent, bytes_per_tok: f64) -> usize {
    let bytes = match ev.role {
        EventRole::System => return 0,
        EventRole::User => ev
            .content_json
            .get("text")
            .and_then(Value::as_str)
            .map_or(0, str::len),
        EventRole::Assistant => {
            let text_len = ev
                .content_json
                .get("text")
                .and_then(Value::as_str)
                .map_or(0, str::len);
            let calls_len: usize = ev
                .content_json
                .get("tool_calls")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .map(|tc| {
                            let n = tc.get("name").and_then(Value::as_str).map_or(0, str::len);
                            let a = tc.get("arguments").map_or(0, |v| v.to_string().len());
                            n + a
                        })
                        .sum()
                })
                .unwrap_or(0);
            text_len + calls_len
        }
        EventRole::Tool => ev
            .content_json
            .get("content")
            .and_then(Value::as_str)
            .map_or(0, str::len),
    };
    bytes_to_tokens(bytes, bytes_per_tok) + 8
}

// ─── Wire Protocol-Pair Sanitizer (Codex / Hermes Safety Guarantee) ───────────

/// Guarantees that an outbound `messages[]` array strictly satisfies OpenAI/Anthropic/MiniMax
/// tool-call pairing invariants:
/// 1. Drops any orphaned `EventRole::Tool` messages not preceded by a matching `Assistant(tool_calls)`.
/// 2. Synthesizes recovery `EventRole::Tool` replies for any `Assistant(tool_calls)` ID missing a result.
pub fn sanitize_message_tool_pairs(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let mut out: Vec<ChatMessage> = Vec::with_capacity(messages.len());
    let mut pending_calls: Vec<(String, String)> = Vec::new();
    let mut answered_ids: HashSet<String> = HashSet::new();

    let flush_missing = |out: &mut Vec<ChatMessage>,
                         pending: &mut Vec<(String, String)>,
                         answered: &mut HashSet<String>| {
        for (call_id, tool_name) in pending.drain(..) {
            if !answered.contains(&call_id) {
                out.push(ChatMessage {
                    role: EventRole::Tool,
                    content: format!(
                        "[recovered missing `{tool_name}` tool result after branch/compaction boundary]"
                    ),
                    reasoning_content: None,
                    tool_calls: Vec::new(),
                    tool_call_id: Some(call_id),
                    name: Some(tool_name),
                });
            }
        }
        answered.clear();
    };

    for msg in messages {
        match msg.role {
            EventRole::Tool => {
                let valid = msg
                    .tool_call_id
                    .as_deref()
                    .is_some_and(|cid| {
                        pending_calls.iter().any(|(pid, _)| pid == cid)
                            && !answered_ids.contains(cid)
                    });
                if valid {
                    if let Some(ref cid) = msg.tool_call_id {
                        answered_ids.insert(cid.clone());
                    }
                    out.push(msg);
                }
            }
            EventRole::Assistant => {
                flush_missing(&mut out, &mut pending_calls, &mut answered_ids);
                if !msg.tool_calls.is_empty() {
                    for tc in &msg.tool_calls {
                        pending_calls.push((tc.id.clone(), tc.name.clone()));
                    }
                }
                out.push(msg);
            }
            EventRole::User | EventRole::System => {
                flush_missing(&mut out, &mut pending_calls, &mut answered_ids);
                out.push(msg);
            }
        }
    }
    flush_missing(&mut out, &mut pending_calls, &mut answered_ids);
    out
}

// ─── Atomic Exchange Grouping ─────────────────────────────────────────────────

struct ExchangeUnit {
    messages: Vec<ChatMessage>,
    est_tokens: usize,
    summary_line: String,
}

fn group_atomic_exchanges(messages: Vec<ChatMessage>, bytes_per_tok: f64) -> Vec<ExchangeUnit> {
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

        let est_tokens = group
            .iter()
            .map(|m| estimate_message_tokens_calibrated(m, bytes_per_tok))
            .sum();
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

// ─── Stage A: Quantized, Wrapper-Aware Selective Microcompaction ──────────────

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MicrocompactStats {
    pub compacted_tools: usize,
    pub stripped_reasoning_blocks: usize,
    pub tokens_saved: usize,
    pub cut_units: usize,
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
    const HINT_KEYS: &[&str] = &["path", "id", "query", "command", "url", "job_id", "slug"];
    for key in HINT_KEYS {
        if let Some(s) = val.get(*key).and_then(Value::as_str).map(str::trim)
            && !s.is_empty()
        {
            return compact_preview(s, 72);
        }
    }
    String::new()
}

/// Unwraps `invoke_tool` and inspects `batch_tools` so deferred bulky calls (`git`, `web_fetch`,
/// `code_refactor`, `knowledge_catalog`, multi-file read batches) actually microcompact while
/// strictly shielding `COMPACTION_PROTECTED_TOOLS` (`plan_update`, `memory_tool`, `project_brain`, ...).
fn classify_microcompact_target<'a>(
    outer_name: &'a str,
    outer_args: Option<&'a Value>,
) -> Option<(String, String)> {
    match outer_name {
        TOOL_INVOKE_TOOL => {
            let args = outer_args?;
            let inner_name = args
                .get("tool")
                .or_else(|| args.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())?;
            if COMPACTION_PROTECTED_TOOLS.contains(&inner_name) {
                return None;
            }
            let inner_args = args.get("args").or_else(|| args.get("arguments"));
            // Allow all declared microcompactable tools AND dynamic `mcp__*` read payloads
            let eligible = MICROCOMPACTABLE_TOOLS.contains(&inner_name)
                || inner_name.starts_with(zenko_core::MCP_TOOL_PREFIX);
            match eligible {
                true => Some((inner_name.to_string(), extract_tool_target_hint(inner_args))),
                false => None,
            }
        }
        TOOL_BATCH_TOOLS => {
            let calls = outer_args
                .and_then(|a| a.get("calls"))
                .and_then(Value::as_array)?;
            if calls.is_empty() {
                return None;
            }
            let mut inner_names: Vec<&str> = Vec::with_capacity(calls.len());
            for step in calls {
                let raw = step
                    .get("tool")
                    .or_else(|| step.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let eff = match raw {
                    TOOL_INVOKE_TOOL => step
                        .get("args")
                        .or_else(|| step.get("arguments"))
                        .and_then(|a| a.get("tool").or_else(|| a.get("name")))
                        .and_then(Value::as_str)
                        .unwrap_or(raw),
                    other => other,
                };
                if COMPACTION_PROTECTED_TOOLS.contains(&eff) {
                    return None;
                }
                inner_names.push(eff);
            }
            let preview_names: Vec<&str> = inner_names.iter().copied().take(3).collect();
            let hint = format!("{} calls: {}", calls.len(), preview_names.join(", "));
            Some((TOOL_BATCH_TOOLS.to_string(), compact_preview(&hint, 72)))
        }
        other => {
            if COMPACTION_PROTECTED_TOOLS.contains(&other)
                || !MICROCOMPACTABLE_TOOLS.contains(&other)
            {
                return None;
            }
            Some((other.to_string(), extract_tool_target_hint(outer_args)))
        }
    }
}

/// Stage-A lossless microcompaction (backwards-compatible wrapper using default calibration).
pub fn microcompact_messages(
    messages: Vec<ChatMessage>,
    context_window: u64,
    prefix_tokens: usize,
    trigger_ratio: f64,
) -> (Vec<ChatMessage>, MicrocompactStats) {
    microcompact_messages_calibrated(
        messages,
        context_window,
        0,
        prefix_tokens,
        trigger_ratio,
        TOKEN_CALIBRATION_DEFAULT_BYTES_PER_TOK,
    )
}

/// Collapses oversized historical `write_file` / `apply_patch` / `hash_edit` assistant tool-call
/// body payloads (`content`, `patch`, `new_str`) once they age behind `protected_start`, so
/// writing 50KB–80KB files does not stay permanently glued inside prompt history.
fn compact_historical_write_call(tc: &mut zenko_core::ToolCall, bytes_per_tok: f64) -> usize {
    let (eff_name, target_obj) = match tc.name.as_str() {
        TOOL_INVOKE_TOOL => {
            let inner = tc
                .arguments
                .get("tool")
                .or_else(|| tc.arguments.get("name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("")
                .to_string();
            let args_mut = match tc.arguments.get_mut("args").is_some() {
                true => tc.arguments.get_mut("args"),
                false => tc.arguments.get_mut("arguments"),
            };
            (inner, args_mut.and_then(Value::as_object_mut))
        }
        other => (other.to_string(), tc.arguments.as_object_mut()),
    };
    let Some(map) = target_obj else {
        return 0;
    };
    let field_key = match eff_name.as_str() {
        TOOL_WRITE_FILE => "content",
        TOOL_APPLY_PATCH => "patch",
        TOOL_HASH_EDIT => "new_str",
        _ => return 0,
    };
    let Some(orig_len) = map
        .get(field_key)
        .and_then(Value::as_str)
        .filter(|s| s.len() >= MICROCOMPACT_MIN_WRITE_ARG_CHARS && !s.starts_with("[microcompacted "))
        .map(str::len)
    else {
        return 0;
    };
    let orig_lines = map
        .get(field_key)
        .and_then(Value::as_str)
        .map(|s| s.lines().count())
        .unwrap_or(0);
    let path_label = map
        .get("path")
        .or_else(|| map.get("file_path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("file")
        .to_string();
    let stub = format!(
        "[microcompacted `{eff_name}` payload for `{path_label}` ({orig_lines} lines, {orig_len}B) — use `read_file` if needed]"
    );
    if stub.len() >= orig_len {
        return 0;
    }
    let saved = bytes_to_tokens(orig_len.saturating_sub(stub.len()), bytes_per_tok);
    map.insert(field_key.to_string(), Value::String(stub));
    saved
}

fn estimate_unit_compactable_tokens(unit: &ExchangeUnit, bytes_per_tok: f64) -> usize {
    let mut call_meta: HashMap<&str, (&str, &Value)> = HashMap::new();
    let mut saved = 0usize;
    for msg in &unit.messages {
        match msg.role {
            EventRole::Assistant => {
                for tc in &msg.tool_calls {
                    call_meta.insert(tc.id.as_str(), (tc.name.as_str(), &tc.arguments));
                    let mut tc_clone = tc.clone();
                    saved += compact_historical_write_call(&mut tc_clone, bytes_per_tok);
                }
                if let Some(ref reasoning) = msg.reasoning_content {
                    saved += bytes_to_tokens(reasoning.len(), bytes_per_tok);
                }
            }
            EventRole::Tool => {
                if msg.content.len() < MICROCOMPACT_MIN_TOOL_CHARS {
                    continue;
                }
                let meta = msg
                    .tool_call_id
                    .as_deref()
                    .and_then(|id| call_meta.get(id).copied());
                let outer_name = msg
                    .name
                    .as_deref()
                    .or_else(|| meta.map(|(n, _)| n))
                    .unwrap_or("");
                if classify_microcompact_target(outer_name, meta.map(|(_, a)| a)).is_some() {
                    let est_stub = 140usize;
                    saved += bytes_to_tokens(msg.content.len().saturating_sub(est_stub), bytes_per_tok);
                }
            }
            EventRole::User | EventRole::System => {}
        }
    }
    saved
}

/// Stage-A lossless, KV-cache-friendly quantized microcompaction (stateless entrypoint).
pub fn microcompact_messages_calibrated(
    messages: Vec<ChatMessage>,
    context_window: u64,
    reserve_output_tokens: u32,
    prefix_tokens: usize,
    trigger_ratio: f64,
    bytes_per_tok: f64,
) -> (Vec<ChatMessage>, MicrocompactStats) {
    microcompact_messages_guarded(
        messages,
        context_window,
        reserve_output_tokens,
        prefix_tokens,
        trigger_ratio,
        DEFAULT_COMPACTION_TRIGGER_RATIO,
        bytes_per_tok,
        0,
        false,
    )
}

/// Cache-aware Stage-A microcompaction with KV-prefix ROI protection & sticky cut retention:
/// - Never oscillates (once `prev_cut_units > 0`, keeps that prefix compacted across subsequent turns).
/// - When `enforce_cache_roi` is true, refuses to advance `prev_cut_units -> candidate_cut` if doing
///   so would bust a large warm KV-cache tail (`> 2x` marginal savings) while still safely below
///   Stage-B compaction pressure.
/// - Compacts both bulky historical `EventRole::Tool` results AND oversized historical
///   `write_file`/`apply_patch`/`hash_edit` assistant tool-call payloads.
#[allow(clippy::too_many_arguments)]
pub fn microcompact_messages_guarded(
    messages: Vec<ChatMessage>,
    context_window: u64,
    reserve_output_tokens: u32,
    prefix_tokens: usize,
    trigger_ratio: f64,
    stage_b_ratio: f64,
    bytes_per_tok: f64,
    prev_cut_units: usize,
    enforce_cache_roi: bool,
) -> (Vec<ChatMessage>, MicrocompactStats) {
    let ratio = trigger_ratio.clamp(0.25, 0.90);
    let usable = usable_input_window(context_window, reserve_output_tokens);
    let trigger_budget = ((usable as f64) * ratio) as usize;
    let stage_b_budget = ((usable as f64) * stage_b_ratio.clamp(0.45, 0.94)) as usize;
    let mut units = group_atomic_exchanges(messages, bytes_per_tok);
    let history_tokens: usize = units.iter().map(|u| u.est_tokens).sum();

    if units.len() <= MICROCOMPACT_PROTECT_RECENT_EXCHANGES {
        return (
            units.into_iter().flat_map(|u| u.messages).collect(),
            MicrocompactStats::default(),
        );
    }

    let raw_protected_start = units.len().saturating_sub(MICROCOMPACT_PROTECT_RECENT_EXCHANGES);
    let held_cut = prev_cut_units.min(raw_protected_start);

    if prefix_tokens + history_tokens <= trigger_budget && held_cut == 0 {
        return (
            units.into_iter().flat_map(|u| u.messages).collect(),
            MicrocompactStats::default(),
        );
    }

    // Quantize cut point to epoch multiples so KV prefix remains invariant across turns:
    // full `MICROCOMPACT_EPOCH_STEP` (8) once past the first epoch, half-step (4) before that
    // so early turns (raw 4..7) also stay pinned instead of drifting by +1 each iteration.
    let half_step = (MICROCOMPACT_EPOCH_STEP / 2).max(1);
    let quantized_start = match raw_protected_start >= MICROCOMPACT_EPOCH_STEP {
        true => (raw_protected_start / MICROCOMPACT_EPOCH_STEP) * MICROCOMPACT_EPOCH_STEP,
        false => (raw_protected_start / half_step) * half_step,
    };
    let candidate_cut = match quantized_start > 0 {
        true => quantized_start,
        false => raw_protected_start,
    };

    let protected_start = match (enforce_cache_roi, held_cut > 0, candidate_cut > held_cut) {
        (true, true, true) => {
            let saved_at_held: usize = units
                .iter()
                .take(held_cut)
                .map(|u| estimate_unit_compactable_tokens(u, bytes_per_tok))
                .sum();
            let total_if_held =
                prefix_tokens + history_tokens.saturating_sub(saved_at_held);
            let near_stage_b = total_if_held.saturating_mul(100) >= stage_b_budget.saturating_mul(88);
            let marginal_savings: usize = units
                .iter()
                .skip(held_cut)
                .take(candidate_cut.saturating_sub(held_cut))
                .map(|u| estimate_unit_compactable_tokens(u, bytes_per_tok))
                .sum();
            let busted_tail_tokens: usize = units
                .iter()
                .skip(held_cut)
                .map(|u| u.est_tokens)
                .sum();
            let good_roi = marginal_savings >= MICROCOMPACT_MIN_MARGINAL_TOKENS
                && marginal_savings.saturating_mul(2) >= busted_tail_tokens;
            match near_stage_b || good_roi {
                true => candidate_cut,
                false => held_cut,
            }
        }
        (_, true, false) => held_cut,
        _ => candidate_cut,
    };

    let mut stats = MicrocompactStats {
        cut_units: protected_start,
        ..Default::default()
    };

    for unit in units.iter_mut().take(protected_start) {
        let mut call_meta: HashMap<String, (String, Value)> = HashMap::new();
        for msg in &mut unit.messages {
            match msg.role {
                EventRole::Assistant => {
                    for tc in &mut msg.tool_calls {
                        call_meta.insert(tc.id.clone(), (tc.name.clone(), tc.arguments.clone()));
                        let arg_saved = compact_historical_write_call(tc, bytes_per_tok);
                        if arg_saved > 0 {
                            stats.compacted_tools += 1;
                            stats.tokens_saved += arg_saved;
                        }
                    }
                    if let Some(reasoning) = msg.reasoning_content.take() {
                        stats.stripped_reasoning_blocks += 1;
                        stats.tokens_saved += bytes_to_tokens(reasoning.len(), bytes_per_tok);
                    }
                }
                EventRole::Tool => {
                    let meta = msg
                        .tool_call_id
                        .as_deref()
                        .and_then(|id| call_meta.get(id));
                    let outer_name = msg
                        .name
                        .as_deref()
                        .or_else(|| meta.map(|(n, _)| n.as_str()))
                        .unwrap_or("");
                    if msg.content.len() < MICROCOMPACT_MIN_TOOL_CHARS {
                        continue;
                    }
                    let Some((eff_label, target)) =
                        classify_microcompact_target(outer_name, meta.map(|(_, a)| a))
                    else {
                        continue;
                    };
                    let orig_bytes = msg.content.len();
                    let orig_lines = msg.content.lines().count();
                    let target_part = match target.is_empty() {
                        true => String::new(),
                        false => format!(" `{target}`"),
                    };
                    let preserved_rules: Vec<String> =
                        extract_scoped_repo_rules_blocks(&msg.content)
                            .into_iter()
                            .map(String::from)
                            .collect();
                    let base_stub = match extract_artifact_handle(&msg.content) {
                        Some(handle) => format!(
                            "[microcompacted `{eff_label}`{target_part} ({orig_lines} lines, {orig_bytes}B) — inspect full output with `read_file` at `{handle}`]"
                        ),
                        None => format!(
                            "[microcompacted `{eff_label}`{target_part} ({orig_lines} lines, {orig_bytes}B) — re-read if needed]"
                        ),
                    };
                    let stub = match preserved_rules.is_empty() {
                        true => base_stub,
                        false => format!("{base_stub}\n\n{}", preserved_rules.join("\n\n")),
                    };
                    if stub.len() >= orig_bytes {
                        continue;
                    }
                    let saved = bytes_to_tokens(orig_bytes.saturating_sub(stub.len()), bytes_per_tok);
                    msg.content = stub;
                    stats.compacted_tools += 1;
                    stats.tokens_saved += saved;
                }
                EventRole::User | EventRole::System => {}
            }
        }
    }

    if stats.compacted_tools == 0 && stats.stripped_reasoning_blocks == 0 {
        stats.cut_units = held_cut;
    }

    let out = units.into_iter().flat_map(|u| u.messages).collect();
    (out, stats)
}

// ─── Oh My Pi Cumulative File-Op Ledger & Hermes Turn-1 Goal Anchor ──────────

fn push_capped(set: &mut BTreeSet<String>, path: &str, cap: usize) {
    let cleaned = path.trim();
    if !cleaned.is_empty() && set.len() < cap {
        set.insert(cleaned.to_string());
    }
}

fn record_tool_call_in_ledger(
    tool_name: &str,
    args: &Value,
    read_set: &mut BTreeSet<String>,
    mod_set: &mut BTreeSet<String>,
    created_set: &mut BTreeSet<String>,
    deleted_set: &mut BTreeSet<String>,
) {
    match tool_name {
        TOOL_INVOKE_TOOL => {
            if let Some(inner) = args
                .get("tool")
                .or_else(|| args.get("name"))
                .and_then(Value::as_str)
            {
                let inner_args = args
                    .get("args")
                    .or_else(|| args.get("arguments"))
                    .unwrap_or(&Value::Null);
                record_tool_call_in_ledger(
                    inner,
                    inner_args,
                    read_set,
                    mod_set,
                    created_set,
                    deleted_set,
                );
            }
        }
        TOOL_BATCH_TOOLS => {
            if let Some(calls) = args.get("calls").and_then(Value::as_array) {
                for step in calls {
                    if let Some(t) = step
                        .get("tool")
                        .or_else(|| step.get("name"))
                        .and_then(Value::as_str)
                    {
                        let a = step
                            .get("args")
                            .or_else(|| step.get("arguments"))
                            .unwrap_or(&Value::Null);
                        record_tool_call_in_ledger(
                            t,
                            a,
                            read_set,
                            mod_set,
                            created_set,
                            deleted_set,
                        );
                    }
                }
            }
        }
        TOOL_READ_FILE => {
            if let Some(p) = args.get("path").and_then(Value::as_str)
                && !p.starts_with(ARTIFACT_PATH_PREFIX)
            {
                push_capped(read_set, p, COMPACTION_LEDGER_MAX_FILES_PER_KIND);
            }
        }
        TOOL_KNOWLEDGE_CATALOG => {
            if args.get("action").and_then(Value::as_str) == Some("upsert")
                && let Some(id) = args.get("id").and_then(Value::as_str)
            {
                push_capped(
                    mod_set,
                    &format!("knowledge/{id}.md"),
                    COMPACTION_LEDGER_MAX_FILES_PER_KIND,
                );
            }
        }
        TOOL_WRITE_FILE => {
            if let Some(p) = args.get("path").and_then(Value::as_str) {
                match read_set.contains(p) || mod_set.contains(p) {
                    true => push_capped(mod_set, p, COMPACTION_LEDGER_MAX_FILES_PER_KIND),
                    false => push_capped(created_set, p, COMPACTION_LEDGER_MAX_FILES_PER_KIND),
                }
            }
        }
        TOOL_HASH_EDIT => {
            if let Some(p) = args.get("path").and_then(Value::as_str) {
                push_capped(mod_set, p, COMPACTION_LEDGER_MAX_FILES_PER_KIND);
            }
        }
        TOOL_CODE_REFACTOR => {
            if zenko_core::is_code_refactor_args_mutating(args)
                && let Some(p) = args
                    .get("path")
                    .or_else(|| args.get("from"))
                    .and_then(Value::as_str)
            {
                push_capped(mod_set, p, COMPACTION_LEDGER_MAX_FILES_PER_KIND);
            }
        }
        TOOL_APPLY_PATCH => {
            if let Some(p) = args.get("path").and_then(Value::as_str) {
                push_capped(mod_set, p, COMPACTION_LEDGER_MAX_FILES_PER_KIND);
            }
            if let Some(files) = args.get("files").and_then(Value::as_array) {
                for f in files {
                    let act = f.get("action").and_then(Value::as_str).unwrap_or("update");
                    if let Some(p) = f.get("path").and_then(Value::as_str) {
                        match act {
                            "create" => {
                                push_capped(created_set, p, COMPACTION_LEDGER_MAX_FILES_PER_KIND)
                            }
                            "delete" => {
                                push_capped(deleted_set, p, COMPACTION_LEDGER_MAX_FILES_PER_KIND)
                            }
                            _ => push_capped(mod_set, p, COMPACTION_LEDGER_MAX_FILES_PER_KIND),
                        }
                    }
                }
            }
            if let Some(patch) = args.get("patch").and_then(Value::as_str) {
                #[derive(Clone, Copy)]
                enum PatchLineOp {
                    Add,
                    Delete,
                    Modify,
                    UnifiedNew,
                }
                const PATCH_LINE_PREFIXES: &[(&str, PatchLineOp)] = &[
                    (CODEX_PATCH_ADD_FILE, PatchLineOp::Add),
                    (CODEX_PATCH_DELETE_FILE, PatchLineOp::Delete),
                    (CODEX_PATCH_UPDATE_FILE, PatchLineOp::Modify),
                    (UNIFIED_DIFF_NEW_PREFIX, PatchLineOp::UnifiedNew),
                ];
                for line in patch.lines() {
                    let t = line.trim();
                    let matched = PATCH_LINE_PREFIXES
                        .iter()
                        .find_map(|&(pfx, op)| t.strip_prefix(pfx).map(|rest| (op, rest)));
                    match matched {
                        Some((PatchLineOp::Add, rest)) => {
                            push_capped(created_set, rest, COMPACTION_LEDGER_MAX_FILES_PER_KIND)
                        }
                        Some((PatchLineOp::Delete, rest)) => {
                            push_capped(deleted_set, rest, COMPACTION_LEDGER_MAX_FILES_PER_KIND)
                        }
                        Some((PatchLineOp::Modify, rest)) => {
                            push_capped(mod_set, rest, COMPACTION_LEDGER_MAX_FILES_PER_KIND)
                        }
                        Some((PatchLineOp::UnifiedNew, rest)) => {
                            let clean = rest.trim().strip_prefix("b/").unwrap_or(rest.trim());
                            if clean != zenko_core::UNIFIED_DIFF_DEV_NULL {
                                push_capped(mod_set, clean, COMPACTION_LEDGER_MAX_FILES_PER_KIND);
                            }
                        }
                        None => {}
                    }
                }
            }
        }
        _ => {}
    }
}

/// Deterministically merges prior checkpoint file operations with newly compacted events
/// so edited/created/deleted/read files and artifact handles are never forgotten across compactions.
pub fn extract_compaction_file_ledger(
    events: &[&SessionEvent],
    prior: Option<&CompactionFileLedger>,
) -> CompactionFileLedger {
    let mut read_set: BTreeSet<String> = prior
        .map(|p| p.files_read.iter().cloned().collect())
        .unwrap_or_default();
    let mut mod_set: BTreeSet<String> = prior
        .map(|p| p.files_modified.iter().cloned().collect())
        .unwrap_or_default();
    let mut created_set: BTreeSet<String> = prior
        .map(|p| p.files_created.iter().cloned().collect())
        .unwrap_or_default();
    let mut deleted_set: BTreeSet<String> = prior
        .map(|p| p.files_deleted.iter().cloned().collect())
        .unwrap_or_default();
    let mut artifact_set: BTreeSet<String> = prior
        .map(|p| p.artifacts.iter().cloned().collect())
        .unwrap_or_default();

    for ev in events {
        match ev.role {
            EventRole::Assistant => {
                if let Some(calls) = ev.content_json.get("tool_calls").and_then(Value::as_array) {
                    for tc in calls {
                        if let Some(name) = tc.get("name").and_then(Value::as_str) {
                            let args = tc.get("arguments").unwrap_or(&Value::Null);
                            record_tool_call_in_ledger(
                                name,
                                args,
                                &mut read_set,
                                &mut mod_set,
                                &mut created_set,
                                &mut deleted_set,
                            );
                        }
                    }
                }
            }
            EventRole::Tool => {
                if let Some(content) = ev.content_json.get("content").and_then(Value::as_str)
                    && let Some(handle) = extract_artifact_handle(content)
                {
                    push_capped(&mut artifact_set, handle, COMPACTION_LEDGER_MAX_ARTIFACTS);
                }
            }
            EventRole::User | EventRole::System => {}
        }
    }

    CompactionFileLedger {
        files_read: read_set
            .into_iter()
            .take(COMPACTION_LEDGER_MAX_FILES_PER_KIND)
            .collect(),
        files_modified: mod_set
            .into_iter()
            .take(COMPACTION_LEDGER_MAX_FILES_PER_KIND)
            .collect(),
        files_created: created_set
            .into_iter()
            .take(COMPACTION_LEDGER_MAX_FILES_PER_KIND)
            .collect(),
        files_deleted: deleted_set
            .into_iter()
            .take(COMPACTION_LEDGER_MAX_FILES_PER_KIND)
            .collect(),
        artifacts: artifact_set
            .into_iter()
            .take(COMPACTION_LEDGER_MAX_ARTIFACTS)
            .collect(),
    }
}

fn extract_original_goal(
    events: &[&SessionEvent],
    prior: Option<&CompactionCheckpointPayload>,
) -> Option<String> {
    if let Some(goal) = prior
        .and_then(|c| c.original_goal.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(goal.to_string());
    }
    events
        .iter()
        .find(|ev| ev.role == EventRole::User)
        .and_then(|ev| ev.content_json.get("text").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| compact_preview(s, COMPACTION_ANCHOR_GOAL_MAX_CHARS))
}

fn format_file_ledger_block(ledger: &CompactionFileLedger) -> Option<String> {
    if ledger.is_empty() {
        return None;
    }
    let mut parts = Vec::with_capacity(5);
    if !ledger.files_modified.is_empty() {
        parts.push(format!("modified: {}", ledger.files_modified.join(", ")));
    }
    if !ledger.files_created.is_empty() {
        parts.push(format!("created: {}", ledger.files_created.join(", ")));
    }
    if !ledger.files_deleted.is_empty() {
        parts.push(format!("deleted: {}", ledger.files_deleted.join(", ")));
    }
    if !ledger.files_read.is_empty() {
        parts.push(format!("read: {}", ledger.files_read.join(", ")));
    }
    if !ledger.artifacts.is_empty() {
        parts.push(format!("artifacts: {}", ledger.artifacts.join(", ")));
    }
    Some(format!("<file_ledger>\n{}\n</file_ledger>", parts.join("\n")))
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

/// Injects a persisted checkpoint summary (including Hermes Turn-1 goal anchor & Oh My Pi
/// cumulative file ledger) at the head of `messages[]` without mutating the Tier-1 system prompt.
pub fn prepend_checkpoint_to_messages(
    messages: &mut Vec<ChatMessage>,
    checkpoint: &CompactionCheckpointPayload,
) {
    let mut inner = String::with_capacity(checkpoint.summary.len() + 256);
    if let Some(ref goal) = checkpoint.original_goal
        && !goal.trim().is_empty()
    {
        inner.push_str("<original_goal>\n");
        inner.push_str(goal.trim());
        inner.push_str("\n</original_goal>\n\n");
    }
    if let Some(ledger_block) = format_file_ledger_block(&checkpoint.file_ledger) {
        inner.push_str(&ledger_block);
        inner.push_str("\n\n");
    }
    inner.push_str(&checkpoint.summary);

    let block = format!(
        "<compacted_checkpoint compacted_events=\"{}\" model=\"{}\">\n{inner}\n</compacted_checkpoint>",
        checkpoint.compacted_events_count, checkpoint.model
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

fn group_atomic_event_units<'a>(
    events: &[&'a SessionEvent],
    bytes_per_tok: f64,
) -> Vec<EventUnit<'a>> {
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

        let est_tokens = group
            .iter()
            .map(|e| estimate_event_tokens_calibrated(e, bytes_per_tok))
            .sum();
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
    pub original_goal: Option<String>,
    pub file_ledger: CompactionFileLedger,
}

/// Backwards-compatible Stage-B split planner using default calibration & cooldown.
pub fn plan_checkpoint_split<'a>(
    slice: &CheckpointSlice<'a>,
    context_window: u64,
    prefix_tokens: usize,
    trigger_ratio: f64,
) -> Option<CheckpointPlan<'a>> {
    plan_checkpoint_split_calibrated(
        slice,
        context_window,
        0,
        prefix_tokens,
        trigger_ratio,
        TOKEN_CALIBRATION_DEFAULT_BYTES_PER_TOK,
        false,
    )
}

/// Calibrated Stage-B split planner honoring output-token reservation, real-token EMA density,
/// optional reactive-overflow cooldown bypass (`force_overflow`), Turn-1 goal anchor, and
/// deterministic cumulative file-op ledger extraction.
#[allow(clippy::too_many_arguments)]
pub fn plan_checkpoint_split_calibrated<'a>(
    slice: &CheckpointSlice<'a>,
    context_window: u64,
    reserve_output_tokens: u32,
    prefix_tokens: usize,
    trigger_ratio: f64,
    bytes_per_tok: f64,
    force_overflow: bool,
) -> Option<CheckpointPlan<'a>> {
    if !force_overflow
        && slice.checkpoint.is_some()
        && slice.events_since_checkpoint < COMPACTION_MIN_EVENTS_BETWEEN_CHECKPOINTS
    {
        return None;
    }

    let ratio = trigger_ratio.clamp(0.45, 0.94);
    let usable = usable_input_window(context_window, reserve_output_tokens);
    let total_budget = ((usable as f64) * ratio) as usize;
    let units = group_atomic_event_units(&slice.kept_events, bytes_per_tok);
    let min_units = match force_overflow {
        true => 2,
        false => 3,
    };
    if units.len() <= min_units {
        return None;
    }

    let prior_ckpt_tokens = slice
        .checkpoint
        .as_ref()
        .map_or(0, |c| estimate_text_tokens_calibrated(&c.summary, bytes_per_tok));
    let active_tokens: usize = units.iter().map(|u| u.est_tokens).sum::<usize>() + prior_ckpt_tokens;
    if !force_overflow && prefix_tokens + active_tokens <= total_budget {
        return None;
    }

    let available = total_budget.saturating_sub(prefix_tokens).max(2_048);
    let tail_pct = match force_overflow {
        true => 25,
        false => 35,
    };
    let effective_pool = match force_overflow {
        true => available.min(active_tokens),
        false => available,
    };
    let tail_budget = ((effective_pool * tail_pct) / 100).max(512);
    let min_keep = match force_overflow {
        true => 1,
        false => 2,
    };

    let mut kept_tokens = 0usize;
    let mut keep_from = units.len();
    for idx in (0..units.len()).rev() {
        let kept_units = units.len() - idx;
        let next_tokens = kept_tokens + units[idx].est_tokens;
        if kept_units > min_keep && next_tokens > tail_budget {
            break;
        }
        kept_tokens = next_tokens;
        keep_from = idx;
    }

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

    let original_goal = extract_original_goal(&events_to_compact, slice.checkpoint.as_ref());
    let file_ledger = extract_compaction_file_ledger(
        &events_to_compact,
        slice.checkpoint.as_ref().map(|c| &c.file_ledger),
    );
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
        original_goal,
        file_ledger,
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

/// Packs formatted transcript lines newest-first within `char_budget`, then restores strict
/// chronological order so the most recent decisions/errors right before the cut are NEVER dropped.
fn pack_newest_chronological(lines: &[String], char_budget: usize, line_prefix: &str) -> (Vec<String>, usize) {
    let mut kept_rev: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut omitted = 0usize;
    for line in lines.iter().rev() {
        let needed = line_prefix.len() + line.len() + 1;
        match used + needed <= char_budget || kept_rev.is_empty() {
            true => {
                used += needed;
                kept_rev.push(format!("{line_prefix}{line}"));
            }
            false => {
                omitted += 1;
            }
        }
    }
    kept_rev.reverse();
    (kept_rev, omitted)
}

/// Builds the iterative-merge compaction prompt (System + User transcript) for `compaction_model`.
/// Preserves the Turn-1 goal anchor, cumulative file ledger, and **newest** compacted events.
pub fn build_compaction_prompt(
    plan: &CheckpointPlan<'_>,
    prior_checkpoint: Option<&CompactionCheckpointPayload>,
    active_plan_summary: Option<&str>,
) -> (String, String) {
    build_compaction_prompt_with_focus(plan, prior_checkpoint, active_plan_summary, None)
}

/// Builds the iterative-merge compaction prompt with an optional orchestrator focus directive.
pub fn build_compaction_prompt_with_focus(
    plan: &CheckpointPlan<'_>,
    prior_checkpoint: Option<&CompactionCheckpointPayload>,
    active_plan_summary: Option<&str>,
    focus: Option<&str>,
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
    if let Some(focus_text) = focus.map(str::trim).filter(|s| !s.is_empty()) {
        body.push_str("<compaction_focus>\n");
        body.push_str(&compact_preview(focus_text, 400));
        body.push_str("\n</compaction_focus>\n\n");
    }
    if let Some(ref goal) = plan.original_goal {
        body.push_str("<original_goal>\n");
        body.push_str(goal);
        body.push_str("\n</original_goal>\n\n");
    }
    if let Some(ledger_block) = format_file_ledger_block(&plan.file_ledger) {
        body.push_str(&ledger_block);
        body.push_str("\n\n");
    }
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

    let formatted: Vec<String> = plan
        .events_to_compact
        .iter()
        .map(|ev| format_event_for_compactor(ev))
        .filter(|l| !l.is_empty())
        .collect();
    let remaining_budget = COMPACTION_MAX_TRANSCRIPT_CHARS.saturating_sub(body.len() + 96);
    let (kept_lines, omitted) = pack_newest_chronological(&formatted, remaining_budget, "");
    if omitted > 0 {
        body.push_str(&format!("... ({omitted} earlier entries condensed)\n"));
    }
    for line in kept_lines {
        body.push_str(&line);
        body.push('\n');
    }
    body.push_str("</transcript_to_merge>");
    (sys, body)
}

/// Zero-LLM deterministic fallback checkpoint builder (newest-preserving + Turn-1 goal + file ledger).
pub fn build_deterministic_checkpoint(
    plan: &CheckpointPlan<'_>,
    prior_checkpoint: Option<&CompactionCheckpointPayload>,
    active_plan_summary: Option<&str>,
    prefix_tokens: usize,
) -> CompactionCheckpointPayload {
    let mut summary = String::new();
    if let Some(prev) = prior_checkpoint {
        summary.push_str("## Prior Checkpoint\n");
        summary.push_str(&compact_preview(&prev.summary, 2_000));
        summary.push_str("\n\n");
    }
    if let Some(plan_txt) = active_plan_summary {
        summary.push_str("## Active Execution Plan\n");
        summary.push_str(plan_txt);
        summary.push_str("\n\n");
    }
    summary.push_str("## Compacted Transcript\n");
    let formatted: Vec<String> = plan
        .events_to_compact
        .iter()
        .map(|ev| format_event_for_compactor(ev))
        .filter(|l| !l.is_empty())
        .collect();
    let remaining = COMPACTION_MAX_DETERMINISTIC_CHARS.saturating_sub(summary.len() + 64);
    let (kept_lines, omitted) = pack_newest_chronological(&formatted, remaining, "- ");
    if omitted > 0 {
        summary.push_str(&format!("- ... ({omitted} earlier entries omitted)\n"));
    }
    for line in kept_lines {
        summary.push_str(&line);
        summary.push('\n');
    }
    let after = prefix_tokens + plan.estimated_kept_tokens + estimate_text_tokens(&summary);
    CompactionCheckpointPayload {
        version: 2,
        summary,
        first_kept_event_id: plan.first_kept_event_id.clone(),
        compacted_events_count: plan.compacted_events_count,
        estimated_tokens_before: plan.estimated_tokens_before,
        estimated_tokens_after: after,
        model: "deterministic".into(),
        created_at: Utc::now(),
        original_goal: plan.original_goal.clone(),
        file_ledger: plan.file_ledger.clone(),
    }
}

// ─── Oh My Pi-Style Abandoned-Branch Summarizer (Zero-LLM) ────────────────────

/// Deterministically summarizes an abandoned branch when the user forks/regenerates from an
/// earlier event, preserving files touched, errors encountered, and dead-end lessons.
pub fn build_abandoned_branch_summary(abandoned: &[&SessionEvent]) -> Option<String> {
    let non_sys: Vec<&SessionEvent> = abandoned
        .iter()
        .copied()
        .filter(|e| e.role != EventRole::System)
        .collect();
    if non_sys.is_empty() {
        return None;
    }

    let ledger = extract_compaction_file_ledger(&non_sys, None);
    let recent: Vec<&SessionEvent> = non_sys
        .iter()
        .rev()
        .take(BRANCH_SUMMARY_MAX_EVENTS)
        .copied()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "Abandoned branch summary ({} event(s) replaced by this retry/fork — avoid repeating failed approaches):",
        non_sys.len()
    ));
    if let Some(ledger_block) = format_file_ledger_block(&ledger) {
        lines.push(ledger_block);
    }

    let mut errors: Vec<String> = Vec::new();
    let mut last_assistant: Option<String> = None;
    for ev in &recent {
        match ev.role {
            EventRole::Tool => {
                if ev
                    .content_json
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    let name = ev
                        .content_json
                        .get("tool_name")
                        .and_then(Value::as_str)
                        .unwrap_or("tool");
                    let body = ev
                        .content_json
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if errors.len() < 4 {
                        errors.push(format!("- [{name} ERR] {}", compact_preview(body, 140)));
                    }
                }
            }
            EventRole::Assistant => {
                let text = ev
                    .content_json
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if !text.is_empty() {
                    last_assistant = Some(compact_preview(text, 220));
                }
            }
            EventRole::User | EventRole::System => {}
        }
    }

    if !errors.is_empty() {
        lines.push(format!("Errors hit on prior branch:\n{}", errors.join("\n")));
    }
    if let Some(reply) = last_assistant {
        lines.push(format!("Last prior reply excerpt: {reply}"));
    }

    let joined = lines.join("\n");
    Some(compact_preview(&joined, BRANCH_SUMMARY_MAX_CHARS))
}

// ─── Emergency Hard-Ceiling Pair-Preserving Budget Valve ──────────────────────

/// Backwards-compatible emergency hard-ceiling valve using default trigger ratio & calibration.
pub fn compact_messages_to_budget(
    messages: Vec<ChatMessage>,
    context_window: u64,
    system_prompt_tokens: usize,
) -> Vec<ChatMessage> {
    compact_messages_to_budget_calibrated(
        messages,
        context_window,
        0,
        system_prompt_tokens,
        DEFAULT_COMPACTION_TRIGGER_RATIO,
        TOKEN_CALIBRATION_DEFAULT_BYTES_PER_TOK,
    )
}

fn shrink_oversized_tool_message(msg: &mut ChatMessage, char_cap: usize) {
    if msg.content.len() <= char_cap {
        return;
    }
    let orig_bytes = msg.content.len();
    let preserved_rules: Vec<String> = extract_scoped_repo_rules_blocks(&msg.content)
        .into_iter()
        .map(String::from)
        .collect();
    let handle_note = match extract_artifact_handle(&msg.content) {
        Some(h) => format!(" (full output at `{h}`)"),
        None => String::new(),
    };
    let head_cap = (char_cap / 2).max(256);
    let tail_cap = (char_cap / 3).max(256);
    let head_end = msg.content.floor_char_boundary(head_cap.min(orig_bytes));
    let tail_start = msg
        .content
        .ceil_char_boundary(orig_bytes.saturating_sub(tail_cap));
    let body = match head_end < tail_start {
        true => format!(
            "{}\n\n... [emergency-trimmed {orig_bytes}B tool output to fit context window{handle_note}] ...\n\n{}",
            &msg.content[..head_end],
            &msg.content[tail_start..]
        ),
        false => compact_preview(&msg.content, char_cap),
    };
    msg.content = match preserved_rules.is_empty() {
        true => body,
        false => format!("{body}\n\n{}", preserved_rules.join("\n\n")),
    };
}

/// Emergency pair-preserving hard ceiling honoring the active `ZMode`/config `trigger_ratio`,
/// output-token reservation, and calibrated density — AND safely shrinking oversized single-wave
/// tool results even when `units.len() <= 2` (eliminating unrecoverable HTTP 400 overflows).
pub fn compact_messages_to_budget_calibrated(
    messages: Vec<ChatMessage>,
    context_window: u64,
    reserve_output_tokens: u32,
    system_prompt_tokens: usize,
    trigger_ratio: f64,
    bytes_per_tok: f64,
) -> Vec<ChatMessage> {
    let ratio = trigger_ratio.clamp(0.45, 0.94);
    let usable = usable_input_window(context_window, reserve_output_tokens);
    let total_budget = ((usable as f64) * ratio) as usize;
    let available = total_budget.saturating_sub(system_prompt_tokens).max(2_048);

    let mut units = group_atomic_exchanges(messages, bytes_per_tok);
    let total_tokens: usize = units.iter().map(|u| u.est_tokens).sum();
    if total_tokens <= available {
        return units.into_iter().flat_map(|u| u.messages).collect();
    }

    // Giant single/two-unit edge case (e.g. massive parallel tool wave): shrink bulky tool outputs
    // in place so the payload fits without dropping the active turn!
    if units.len() <= 2 {
        let tool_count = units
            .iter()
            .flat_map(|u| u.messages.iter())
            .filter(|m| m.role == EventRole::Tool)
            .count()
            .max(1);
        let per_tool_chars = (((available as f64) * clamp_bytes_per_token(bytes_per_tok)) as usize
            / (tool_count + 1))
            .clamp(1_024, COMPACTION_EMERGENCY_SINGLE_MSG_MAX_CHARS);
        for unit in &mut units {
            for msg in &mut unit.messages {
                if msg.role == EventRole::Tool {
                    shrink_oversized_tool_message(msg, per_tool_chars);
                }
            }
        }
        return units.into_iter().flat_map(|u| u.messages).collect();
    }

    let recent_budget = (available * 3) / 4;
    let mut kept_tokens = 0usize;
    let mut keep_from = units.len();

    for idx in (0..units.len()).rev() {
        let next_tokens = kept_tokens + units[idx].est_tokens;
        let kept_count = units.len() - idx;
        let must_keep_tail = kept_count <= 2;
        if !must_keep_tail && next_tokens > recent_budget {
            break;
        }
        kept_tokens = next_tokens;
        keep_from = idx;
    }

    if keep_from == 0 {
        return units.into_iter().flat_map(|u| u.messages).collect();
    }

    // Pack dropped exchange summaries newest-first so recent decisions survive
    let summary_char_cap = (available / 4).saturating_mul(4).clamp(1_200, 12_000);
    let dropped_lines: Vec<String> = units[..keep_from]
        .iter()
        .map(|u| u.summary_line.clone())
        .collect();
    let header = "<compacted_history>\nEarlier conversation turns compacted to fit context budget:\n";
    let (kept_summaries, omitted) =
        pack_newest_chronological(&dropped_lines, summary_char_cap.saturating_sub(header.len() + 64), "");
    let mut summary = String::from(header);
    if omitted > 0 {
        summary.push_str(&format!("... ({omitted} older turns omitted)\n"));
    }
    for line in kept_summaries {
        summary.push_str(&line);
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
    use zenko_core::{TOOL_PLAN_UPDATE, TOOL_WEB_FETCH, ToolCall};

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
    fn stage_a_microcompaction_unwraps_invoke_tool_and_batch_tools_while_quantizing_boundary() {
        let mut msgs = Vec::new();
        // Unit 0: User prompt
        msgs.push(user_msg("inspect docs and web"));
        // Unit 1: invoke_tool wrapping bulky `web_fetch` -> MUST microcompact!
        msgs.push(ChatMessage {
            role: EventRole::Assistant,
            content: String::new(),
            reasoning_content: Some("stale thought ".repeat(80)),
            tool_calls: vec![ToolCall {
                id: "c_inv_web".into(),
                name: TOOL_INVOKE_TOOL.into(),
                arguments: json!({"tool": TOOL_WEB_FETCH, "args": {"url": "https://example.com"}}),
            }],
            tool_call_id: None,
            name: None,
        });
        msgs.push(ChatMessage {
            role: EventRole::Tool,
            content: "web body line\n".repeat(250),
            reasoning_content: None,
            tool_calls: Vec::new(),
            tool_call_id: Some("c_inv_web".into()),
            name: Some(TOOL_INVOKE_TOOL.into()),
        });
        // Unit 2: invoke_tool wrapping PROTECTED `plan_update` -> MUST NEVER microcompact!
        msgs.push(ChatMessage {
            role: EventRole::Assistant,
            content: String::new(),
            reasoning_content: None,
            tool_calls: vec![ToolCall {
                id: "c_inv_plan".into(),
                name: TOOL_INVOKE_TOOL.into(),
                arguments: json!({"tool": TOOL_PLAN_UPDATE, "args": {"goal": "keep safe"}}),
            }],
            tool_call_id: None,
            name: None,
        });
        let plan_body = format!("PLAN_KEEP {}", "detail ".repeat(300));
        msgs.push(ChatMessage {
            role: EventRole::Tool,
            content: plan_body.clone(),
            reasoning_content: None,
            tool_calls: Vec::new(),
            tool_call_id: Some("c_inv_plan".into()),
            name: Some(TOOL_INVOKE_TOOL.into()),
        });
        // Unit 3: batch_tools of non-protected reads -> MUST microcompact!
        msgs.push(ChatMessage {
            role: EventRole::Assistant,
            content: String::new(),
            reasoning_content: None,
            tool_calls: vec![ToolCall {
                id: "c_batch".into(),
                name: TOOL_BATCH_TOOLS.into(),
                arguments: json!({"calls": [{"tool": "read_file", "args": {"path": "a.rs"}}]}),
            }],
            tool_call_id: None,
            name: None,
        });
        msgs.push(ChatMessage {
            role: EventRole::Tool,
            content: "batch output line\n".repeat(250),
            reasoning_content: None,
            tool_calls: Vec::new(),
            tool_call_id: Some("c_batch".into()),
            name: Some(TOOL_BATCH_TOOLS.into()),
        });
        // Unit 4: Recent unit right on quantized epoch boundary (epoch_end = 4 for 8..11 units)
        msgs.push(ChatMessage {
            role: EventRole::Assistant,
            content: String::new(),
            reasoning_content: None,
            tool_calls: vec![ToolCall {
                id: "c_boundary".into(),
                name: TOOL_READ_FILE.into(),
                arguments: json!({"path": "boundary.rs"}),
            }],
            tool_call_id: None,
            name: None,
        });
        let boundary_body = "boundary code line\n".repeat(200);
        msgs.push(ChatMessage {
            role: EventRole::Tool,
            content: boundary_body.clone(),
            reasoning_content: None,
            tool_calls: Vec::new(),
            tool_call_id: Some("c_boundary".into()),
            name: Some(TOOL_READ_FILE.into()),
        });
        // Units 5..8: 4 recent turns (total = 9 units -> raw_protected_start = 5, quantized = 4!)
        for i in 0..MICROCOMPACT_PROTECT_RECENT_EXCHANGES {
            msgs.push(user_msg(&format!("recent turn {i}")));
        }

        let (out, stats) = microcompact_messages(msgs, 2_400, 600, 0.50);
        assert_eq!(stats.compacted_tools, 2, "invoke_tool(web_fetch) + batch_tools must compact");
        assert_eq!(stats.stripped_reasoning_blocks, 1);

        let web_res = out
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some("c_inv_web"))
            .expect("web kept");
        assert!(web_res.content.contains("[microcompacted `web_fetch`"));
        assert!(web_res.content.contains("https://example.com"));

        let plan_res = out
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some("c_inv_plan"))
            .expect("plan kept");
        assert_eq!(plan_res.content, plan_body, "protected invoke_tool(plan_update) must not compact");

        let batch_res = out
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some("c_batch"))
            .expect("batch kept");
        assert!(batch_res.content.contains("[microcompacted `batch_tools`"));

        // Unit 4 stays verbatim because quantized epoch boundary stays pinned at 4 (KV-cache stability!)
        let boundary_res = out
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some("c_boundary"))
            .expect("boundary kept");
        assert_eq!(boundary_res.content, boundary_body);
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
        assert!(compacted.content.contains(scoped_block));
        assert!(!compacted.content.contains("pub fn route() {}"));
    }

    #[test]
    fn stage_b_preserves_goal_anchor_cumulative_file_ledger_and_newest_events() {
        let mut events = Vec::new();
        events.push(mk_event(
            "u_0",
            EventRole::User,
            EventKind::Message,
            json!({"text": "ORIGINAL_ANCHOR_GOAL: Refactor payment retry state machine"}),
        ));
        events.push(mk_event(
            "a_0",
            EventRole::Assistant,
            EventKind::ToolCall,
            json!({
                "text": "",
                "tool_calls": [
                    {"id": "t1", "name": "read_file", "arguments": {"path": "src/payments.rs"}},
                    {"id": "t2", "name": "hash_edit", "arguments": {"path": "src/payments.rs"}}
                ]
            }),
        ));
        events.push(mk_event(
            "tr_1",
            EventRole::Tool,
            EventKind::ToolResult,
            json!({"tool_call_id": "t1", "tool_name": "read_file", "content": "ok -> artifact:s1/t1.txt", "is_error": false}),
        ));
        for i in 1..10 {
            events.push(mk_event(
                &format!("u_{i}"),
                EventRole::User,
                EventKind::Message,
                json!({"text": format!("Turn {i} filler: {}", "z".repeat(700))}),
            ));
            events.push(mk_event(
                &format!("a_{i}"),
                EventRole::Assistant,
                EventKind::Message,
                json!({"text": format!("Reply {i} CRITICAL_MARKER_{i}: {}", "w".repeat(700))}),
            ));
        }

        let slice = slice_from_latest_checkpoint(&events);
        let plan = plan_checkpoint_split_calibrated(
            &slice,
            6_000,
            1_000,
            500,
            0.75,
            TOKEN_CALIBRATION_DEFAULT_BYTES_PER_TOK,
            false,
        )
        .expect("plan created");

        assert!(plan
            .original_goal
            .as_deref()
            .is_some_and(|g| g.contains("ORIGINAL_ANCHOR_GOAL")));
        assert!(plan.file_ledger.files_read.contains(&"src/payments.rs".to_string()));
        assert!(plan.file_ledger.files_modified.contains(&"src/payments.rs".to_string()));
        assert!(plan.file_ledger.artifacts.contains(&"artifact:s1/t1.txt".to_string()));

        let ckpt = build_deterministic_checkpoint(&plan, None, Some("goal: refactor"), 500);
        // Verify newest compacted event survives truncation (not head-only!)
        let last_compacted_id = plan
            .events_to_compact
            .last()
            .and_then(|e| e.content_json.get("text").and_then(Value::as_str))
            .unwrap_or("");
        if let Some(marker) = last_compacted_id.split(':').next() {
            assert!(
                ckpt.summary.contains(marker),
                "newest compacted entry `{marker}` must be retained over oldest filler"
            );
        }

        let mut msgs = vec![user_msg("continue")];
        prepend_checkpoint_to_messages(&mut msgs, &ckpt);
        assert!(msgs[0].content.contains("<original_goal>"));
        assert!(msgs[0].content.contains("<file_ledger>"));
        assert!(msgs[0].content.contains("modified: src/payments.rs"));
    }

    #[test]
    fn emergency_valve_shrinks_giant_single_wave_and_sanitizer_repairs_broken_pairs() {
        let mut msgs = vec![
            user_msg("analyze huge dump"),
            ChatMessage {
                role: EventRole::Assistant,
                content: String::new(),
                reasoning_content: None,
                tool_calls: vec![ToolCall {
                    id: "c_big".into(),
                    name: "shell_exec".into(),
                    arguments: json!({"command": "cat big.log"}),
                }],
                tool_call_id: None,
                name: None,
            },
            ChatMessage {
                role: EventRole::Tool,
                content: "huge log line\n".repeat(4_000),
                reasoning_content: None,
                tool_calls: Vec::new(),
                tool_call_id: Some("c_big".into()),
                name: Some("shell_exec".into()),
            },
        ];

        // Even with <= 2 atomic units, emergency valve must shrink oversized tool output!
        let trimmed = compact_messages_to_budget_calibrated(msgs.clone(), 4_096, 512, 400, 0.80, 3.5);
        let big_out = trimmed.last().expect("tool msg");
        assert!(big_out.content.len() < 12_000);
        assert!(big_out.content.contains("emergency-trimmed"));

        // Sanitizer drops leading orphan tool and synthesizes missing tool reply
        msgs.insert(
            0,
            ChatMessage {
                role: EventRole::Tool,
                content: "orphan".into(),
                reasoning_content: None,
                tool_calls: Vec::new(),
                tool_call_id: Some("ghost".into()),
                name: Some("read_file".into()),
            },
        );
        msgs.push(ChatMessage {
            role: EventRole::Assistant,
            content: String::new(),
            reasoning_content: None,
            tool_calls: vec![ToolCall {
                id: "missing_reply".into(),
                name: "read_file".into(),
                arguments: json!({"path": "x.rs"}),
            }],
            tool_call_id: None,
            name: None,
        });
        let clean = sanitize_message_tool_pairs(msgs);
        assert_eq!(clean[0].role, EventRole::User);
        assert_eq!(clean.last().map(|m| m.role), Some(EventRole::Tool));
        assert_eq!(
            clean.last().and_then(|m| m.tool_call_id.as_deref()),
            Some("missing_reply")
        );
    }

    #[test]
    fn token_calibration_overflow_detection_and_branch_summary_work() {
        assert!(is_provider_context_overflow_error(&ZenkoError::Provider(
            "HTTP 400: maximum context length exceeded".into()
        )));
        assert!(!is_provider_context_overflow_error(&ZenkoError::Provider(
            "HTTP 429: rate limit exceeded".into()
        )));

        // Dense code: 20_000 bytes took 8_000 tokens (2.5 B/tok) -> EMA moves down from 3.5
        let next = calibrate_bytes_per_token(3.5, 20_000, 8_000);
        assert!(next < 3.4 && next > 2.5);

        let e1 = mk_event(
            "a1",
            EventRole::Assistant,
            EventKind::ToolCall,
            json!({"text": "Trying regex rewrite", "tool_calls": [{"id": "c1", "name": "hash_edit", "arguments": {"path": "src/app.rs"}}]}),
        );
        let e2 = mk_event(
            "t1",
            EventRole::Tool,
            EventKind::ToolResult,
            json!({"tool_call_id": "c1", "tool_name": "hash_edit", "content": "hash mismatch on line 42", "is_error": true}),
        );
        let sum = build_abandoned_branch_summary(&[&e1, &e2]).expect("branch summary");
        assert!(sum.contains("modified: src/app.rs"));
        assert!(sum.contains("hash_edit ERR"));
    }

    #[test]
    fn effective_compaction_window_clamps_million_token_models_without_shrinking_small_windows() {
        assert_eq!(
            effective_compaction_window(1_000_000, zenko_core::ZMODE_BALANCED_COMPACTION_WINDOW_CAP),
            128_000
        );
        assert_eq!(
            effective_compaction_window(64_000, zenko_core::ZMODE_BALANCED_COMPACTION_WINDOW_CAP),
            64_000
        );
        assert_eq!(effective_compaction_window(1_000_000, 0), 1_000_000);
    }

    #[test]
    fn microcompact_guarded_compacts_historical_write_file_and_protects_warm_kv_cache() {
        let mut msgs: Vec<ChatMessage> = vec![user_msg("Build portfolio website")];
        // Unit 1: huge write_file(index.html) payload (~20KB) with tiny ok tool output
        msgs.push(ChatMessage {
            role: EventRole::Assistant,
            content: String::new(),
            reasoning_content: None,
            tool_calls: vec![ToolCall {
                id: "w1".into(),
                name: TOOL_WRITE_FILE.into(),
                arguments: json!({
                    "path": "index.html",
                    "content": "<div>portfolio hero section</div>\n".repeat(600),
                }),
            }],
            tool_call_id: None,
            name: None,
        });
        msgs.push(ChatMessage {
            role: EventRole::Tool,
            content: "Wrote 20400 bytes to index.html".into(),
            reasoning_content: None,
            tool_calls: Vec::new(),
            tool_call_id: Some("w1".into()),
            name: Some(TOOL_WRITE_FILE.into()),
        });
        // Add 10 more small exchanges (total 12 units: 0..12, raw_protected_start = 8 -> cut = 8)
        for i in 0..10 {
            let cid = format!("r{i}");
            msgs.push(ChatMessage {
                role: EventRole::Assistant,
                content: String::new(),
                reasoning_content: None,
                tool_calls: vec![ToolCall {
                    id: cid.clone(),
                    name: TOOL_READ_FILE.into(),
                    arguments: json!({"path": format!("file_{i}.js")}),
                }],
                tool_call_id: None,
                name: None,
            });
            msgs.push(ChatMessage {
                role: EventRole::Tool,
                content: format!("const x{i} = {i};\n").repeat(80),
                reasoning_content: None,
                tool_calls: Vec::new(),
                tool_call_id: Some(cid),
                name: Some(TOOL_READ_FILE.into()),
            });
        }

        // Initial Stage-A triggers (trigger_ratio=0.35 on 16k window) -> cuts at 8 and compacts w1's content!
        let (compacted, stats1) = microcompact_messages_guarded(
            msgs.clone(),
            16_000,
            1_000,
            2_000,
            0.35,
            0.80,
            3.5,
            0,
            true,
        );
        assert_eq!(stats1.cut_units, 8);
        assert!(stats1.compacted_tools >= 1);
        assert!(stats1.tokens_saved > 4_000);
        let w1_content = compacted[1].tool_calls[0]
            .arguments
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("");
        assert!(
            w1_content.starts_with("[microcompacted `write_file` payload for `index.html`"),
            "expected write_file body stub, got: {w1_content}"
        );

        // Now grow by 8 tiny non-compactable units (total 20 units -> candidate_cut = 16 > held_cut = 8)
        // Because the new tail has large warm tokens (~10k) and near-zero marginal savings while
        // safely below Stage-B (128k window), ROI guard MUST hold cut_units at 8!
        for i in 0..8 {
            msgs.push(user_msg(&format!("follow-up note {i}: {}", "detail ".repeat(180))));
        }
        let (_held_msgs, stats2) = microcompact_messages_guarded(
            msgs,
            128_000,
            4_000,
            2_000,
            0.35,
            0.80,
            3.5,
            stats1.cut_units,
            true,
        );
        assert_eq!(
            stats2.cut_units, 8,
            "ROI guard must keep warm KV-cache cut frozen at 8 instead of advancing to 16"
        );
    }

    #[test]
    fn forced_orchestrator_compaction_splits_below_threshold_on_large_context_windows() {
        let mut events = Vec::new();
        for i in 0..8 {
            events.push(mk_event(
                &format!("u_{i}"),
                EventRole::User,
                EventKind::Message,
                json!({"text": format!("Turn {i}: {}", "architecture requirement detail ".repeat(40))}),
            ));
            events.push(mk_event(
                &format!("a_{i}"),
                EventRole::Assistant,
                EventKind::Message,
                json!({"text": format!("Completed step {i}: {}", "verified module summary ".repeat(40))}),
            ));
        }
        let slice = slice_from_latest_checkpoint(&events);
        // Active history (~5k tok) is well below 128k window, so automatic split returns None,
        // while explicit orchestrator `force_overflow = true` MUST still produce a valid split.
        assert!(
            plan_checkpoint_split_calibrated(&slice, 128_000, 16_384, 12_000, 0.78, 3.5, false)
                .is_none()
        );
        let plan = plan_checkpoint_split_calibrated(&slice, 128_000, 16_384, 12_000, 0.78, 3.5, true)
            .expect("forced orchestrator compaction must split multi-turn history on large window");
        assert!(plan.compacted_events_count >= 4);
        assert_ne!(plan.first_kept_event_id, "u_0");
    }
}
