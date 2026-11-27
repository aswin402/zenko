//! LMCache-Inspired Parent-Chained Prompt-Prefix Hasher & Content-Addressed Chunk-Reference Cache.
//!
//! Guarantees (zero hardcoding, zero regex, zero `if/else` chains, zero `.unwrap()`):
//! 1. **Parent-Chained SHA-256 Chunk Tree:** Splits every outbound `ChatRequest` into semantic
//!    chunks `[SystemBase, ToolCatalog, HistoryMessage_0..N]` and chains
//!    `H_k = sha256(H_{k-1} || kind_tag || canonical_bytes)`.
//! 2. **Exact Prefix-Drift Diagnostics:** Diffs consecutive turns in `O(n)` to identify exact
//!    reusable vs. busted prefix tokens and root cause (`FullHit`, `SystemMutated`,
//!    `ToolCatalogChanged`, `HistoryCompacted`, `HistoryDiverged`, `FirstTurn`).
//! 3. **Deterministic Sticky `prompt_cache_key`:** Anchored strictly on `(SystemBase, ToolCatalog)`
//!    for OpenAI / vLLM / SGLang + LMCache prefix-aware worker routing.
//! 4. **Cross-Turn File-Stat Chunk Reference Dedup:** Prevents re-stuffing unchanged multi-KB
//!    `read_file` / `knowledge_catalog` payloads already sitting in the live prompt prefix,
//!    auto-invalidating on file `mtime`/`size` changes, file edits, or context compaction.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;
use zenko_core::{
    ChatMessage, EventRole, PREFIX_CACHE_CHUNK_REF_MAX_ENTRIES, PREFIX_CACHE_CHUNK_REF_MIN_BYTES,
    PREFIX_CACHE_KEY_HEX_LEN, PREFIX_CACHE_KEY_PREFIX, PREFIX_CACHE_MAX_SESSIONS,
    PrefixCacheEvaluation, PrefixDriftReason, PromptChunkKind, TOOL_KNOWLEDGE_CATALOG,
    TOOL_READ_FILE, ToolSpec,
};
use zenko_memory::estimate_tokens;

#[derive(Debug, Clone)]
struct ChainedPromptChunk {
    kind: PromptChunkKind,
    chained_hash: [u8; 32],
    approx_tokens: usize,
}

#[derive(Debug, Clone)]
struct CachedFileChunk {
    mtime_ns: u128,
    byte_len: u64,
    content_sha256: String,
    seen_iter: usize,
}

#[derive(Debug, Default)]
struct SessionPrefixState {
    last_chain: Vec<ChainedPromptChunk>,
    chunk_refs: HashMap<(PathBuf, String), CachedFileChunk>,
    chunk_lru: VecDeque<(PathBuf, String)>,
}

impl SessionPrefixState {
    fn clear_chunk_refs(&mut self) {
        self.chunk_refs.clear();
        self.chunk_lru.clear();
    }

    fn touch_chunk(&mut self, key: &(PathBuf, String)) {
        if let Some(pos) = self.chunk_lru.iter().position(|k| k == key) {
            self.chunk_lru.remove(pos);
        }
        self.chunk_lru.push_back(key.clone());
    }

    fn insert_chunk(&mut self, key: (PathBuf, String), entry: CachedFileChunk) {
        self.chunk_refs.insert(key.clone(), entry);
        self.touch_chunk(&key);
        while self.chunk_refs.len() > PREFIX_CACHE_CHUNK_REF_MAX_ENTRIES {
            let Some(oldest) = self.chunk_lru.pop_front() else {
                break;
            };
            self.chunk_refs.remove(&oldest);
        }
    }

    fn invalidate_path(&mut self, path: &Path) {
        self.chunk_refs.retain(|(p, _), _| p != path);
        self.chunk_lru.retain(|(p, _)| p != path);
    }
}

#[derive(Debug, Default)]
struct TrackerInner {
    sessions: HashMap<String, SessionPrefixState>,
    lru: VecDeque<String>,
}

impl TrackerInner {
    fn touch(&mut self, session_id: &str) {
        if let Some(pos) = self.lru.iter().position(|s| s == session_id) {
            self.lru.remove(pos);
        }
        self.lru.push_back(session_id.to_string());
    }

    fn evict_if_needed(&mut self) {
        while self.sessions.len() > PREFIX_CACHE_MAX_SESSIONS {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            self.sessions.remove(&oldest);
        }
    }
}

/// Thread-safe, bounded-LRU tracker maintaining parent-chained prompt hashes and live-prefix
/// file-stat chunk references across active sessions.
#[derive(Debug, Default)]
pub struct SessionPrefixTracker {
    inner: Mutex<TrackerInner>,
}

fn canonicalize_json(val: &Value) -> Value {
    match val {
        Value::Object(map) => {
            let sorted: BTreeMap<&str, Value> = map
                .iter()
                .map(|(k, v)| (k.as_str(), canonicalize_json(v)))
                .collect();
            let mut out = serde_json::Map::with_capacity(sorted.len());
            for (k, v) in sorted {
                out.insert(k.to_string(), v);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        other => other.clone(),
    }
}

fn chain_hash(prev: &[u8; 32], kind: PromptChunkKind, payload: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(prev);
    hasher.update([kind.tag_byte()]);
    hasher.update((payload.len() as u64).to_le_bytes());
    hasher.update(payload);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn role_tag(role: EventRole) -> u8 {
    match role {
        EventRole::System => 1,
        EventRole::User => 2,
        EventRole::Assistant => 3,
        EventRole::Tool => 4,
    }
}

fn serialize_message_canonical(msg: &ChatMessage) -> Vec<u8> {
    let mut buf = Vec::with_capacity(msg.content.len() + 64);
    buf.push(role_tag(msg.role));
    buf.push(0);
    if let Some(ref name) = msg.name {
        buf.extend_from_slice(name.as_bytes());
    }
    buf.push(0);
    if let Some(ref id) = msg.tool_call_id {
        buf.extend_from_slice(id.as_bytes());
    }
    buf.push(0);
    buf.extend_from_slice(msg.content.as_bytes());
    buf.push(0);
    if let Some(ref rc) = msg.reasoning_content {
        buf.extend_from_slice(rc.as_bytes());
    }
    for tc in &msg.tool_calls {
        buf.push(0x1F);
        buf.extend_from_slice(tc.id.as_bytes());
        buf.push(0x1E);
        buf.extend_from_slice(tc.name.as_bytes());
        buf.push(0x1E);
        buf.extend_from_slice(canonicalize_json(&tc.arguments).to_string().as_bytes());
    }
    buf
}

fn build_chained_prompt_chunks(
    system_prompt: &str,
    tools: &[ToolSpec],
    messages: &[ChatMessage],
) -> (String, Vec<ChainedPromptChunk>) {
    let mut chunks = Vec::with_capacity(messages.len() + 2);
    let zero_seed = [0u8; 32];

    // Chunk 0: Tier-1 System Base
    let sys_hash = chain_hash(&zero_seed, PromptChunkKind::SystemBase, system_prompt.as_bytes());
    chunks.push(ChainedPromptChunk {
        kind: PromptChunkKind::SystemBase,
        chained_hash: sys_hash,
        approx_tokens: estimate_tokens(system_prompt),
    });

    // Chunk 1: Sorted & canonicalized Tool Catalog (identical canonical order to dialect.rs)
    let mut sorted_tools: Vec<&ToolSpec> = tools.iter().collect();
    sorted_tools.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.description.cmp(&b.description)));
    let mut tool_bytes = Vec::new();
    for spec in &sorted_tools {
        tool_bytes.extend_from_slice(spec.name.as_bytes());
        tool_bytes.push(0);
        tool_bytes.extend_from_slice(spec.description.as_bytes());
        tool_bytes.push(0);
        tool_bytes.extend_from_slice(canonicalize_json(&spec.parameters).to_string().as_bytes());
        tool_bytes.push(0x1D);
    }
    let tools_hash = chain_hash(&sys_hash, PromptChunkKind::ToolCatalog, &tool_bytes);
    let tools_tokens = tool_bytes.len().div_ceil(4);
    chunks.push(ChainedPromptChunk {
        kind: PromptChunkKind::ToolCatalog,
        chained_hash: tools_hash,
        approx_tokens: tools_tokens,
    });

    // Anchor sticky `prompt_cache_key` strictly on (SystemBase, ToolCatalog)
    let key_hex = hex::encode(tools_hash);
    let prompt_cache_key = format!(
        "{PREFIX_CACHE_KEY_PREFIX}{}",
        key_hex
            .get(..PREFIX_CACHE_KEY_HEX_LEN)
            .unwrap_or(&key_hex)
    );

    // Chunks 2..N: History messages
    let mut running = tools_hash;
    for msg in messages {
        let payload = serialize_message_canonical(msg);
        running = chain_hash(&running, PromptChunkKind::HistoryMessage, &payload);
        let approx_tokens = payload.len().div_ceil(4).max(1);
        chunks.push(ChainedPromptChunk {
            kind: PromptChunkKind::HistoryMessage,
            chained_hash: running,
            approx_tokens,
        });
    }

    (prompt_cache_key, chunks)
}

fn sha256_short_hex(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let hex_str = hex::encode(hasher.finalize());
    hex_str.get(..12).unwrap_or(&hex_str).to_string()
}

fn file_mtime_and_len(path: &Path) -> Option<(u128, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Some((mtime_ns, meta.len()))
}

fn extract_read_chunk_target(
    workspace_dir: &Path,
    tool_name: &str,
    args: &Value,
) -> Option<(PathBuf, String, String)> {
    match tool_name {
        TOOL_READ_FILE => {
            if args.get("mode").and_then(Value::as_str) == Some("outline") {
                return None;
            }
            let raw_path = args.get("path").and_then(Value::as_str)?.trim();
            if raw_path.is_empty() || raw_path.contains("..") {
                return None;
            }
            let candidate = match Path::new(raw_path).is_absolute() {
                true => PathBuf::from(raw_path),
                false => workspace_dir.join(raw_path),
            };
            let canon = std::fs::canonicalize(&candidate).ok()?;
            let range_key = match (
                args.get("start_line").and_then(Value::as_u64),
                args.get("end_line").and_then(Value::as_u64),
                args.get("ranges").and_then(Value::as_array),
                args.get("symbols").and_then(Value::as_array),
            ) {
                (_, _, Some(ranges), _) if !ranges.is_empty() => {
                    format!("ranges:{}", canonicalize_json(&Value::Array(ranges.clone())))
                }
                (_, _, _, Some(syms)) if !syms.is_empty() => {
                    format!("symbols:{}", canonicalize_json(&Value::Array(syms.clone())))
                }
                (Some(s), Some(e), _, _) => format!("lines:{s}..{e}"),
                (Some(s), None, _, _) => format!("lines:{s}.."),
                (None, Some(e), _, _) => format!("lines:1..{e}"),
                (None, None, _, _) => "full".to_string(),
            };
            Some((canon, raw_path.to_string(), range_key))
        }
        TOOL_KNOWLEDGE_CATALOG => {
            if args.get("action").and_then(Value::as_str) != Some("read") {
                return None;
            }
            let id = args.get("id").and_then(Value::as_str)?.trim();
            if id.is_empty() {
                return None;
            }
            let concept = zenko_memory::okf::read_okf_concept(workspace_dir, id)?;
            let canon = std::fs::canonicalize(workspace_dir.join(&concept.rel_path)).ok()?;
            Some((canon, concept.rel_path, "okf_read".to_string()))
        }
        _ => None,
    }
}

impl SessionPrefixTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Evaluates parent-chained prefix overlap against the session's previous LLM call,
    /// records the new chain, and automatically invalidates file-chunk refs if the prefix
    /// was compacted or diverged.
    pub fn evaluate_and_record(
        &self,
        session_id: &str,
        system_prompt: &str,
        tools: &[ToolSpec],
        messages: &[ChatMessage],
        compacted_this_iter: bool,
    ) -> PrefixCacheEvaluation {
        let (prompt_cache_key, current_chunks) =
            build_chained_prompt_chunks(system_prompt, tools, messages);
        let total_chunks = current_chunks.len();

        let Ok(mut guard) = self.inner.lock() else {
            return PrefixCacheEvaluation {
                prompt_cache_key,
                total_chunks,
                ..Default::default()
            };
        };

        guard.touch(session_id);
        let state = guard.sessions.entry(session_id.to_string()).or_default();

        let eval = match state.last_chain.is_empty() {
            true => PrefixCacheEvaluation {
                prompt_cache_key: prompt_cache_key.clone(),
                total_chunks,
                matched_chunks: 0,
                reusable_tokens: 0,
                busted_tokens: 0,
                hit_ratio_pct: 0,
                drift_reason: PrefixDriftReason::FirstTurn,
            },
            false => {
                let prev = &state.last_chain;
                let shared_len = prev.len().min(current_chunks.len());
                let first_mismatch = (0..shared_len).find(|&i| {
                    prev[i].kind != current_chunks[i].kind
                        || prev[i].chained_hash != current_chunks[i].chained_hash
                });
                let matched_chunks = match first_mismatch {
                    Some(idx) => idx,
                    None => shared_len,
                };
                let prev_total_tokens: usize = prev.iter().map(|c| c.approx_tokens).sum();
                let reusable_tokens: usize = current_chunks[..matched_chunks]
                    .iter()
                    .map(|c| c.approx_tokens)
                    .sum();
                let busted_tokens: usize = prev[matched_chunks..]
                    .iter()
                    .map(|c| c.approx_tokens)
                    .sum();
                let hit_ratio_pct = match prev_total_tokens > 0 {
                    true => ((reusable_tokens.saturating_mul(100)) / prev_total_tokens).min(100) as u8,
                    false => 100,
                };
                let drift_reason = match (first_mismatch, compacted_this_iter, current_chunks.len() < prev.len()) {
                    (Some(0), _, _) => PrefixDriftReason::SystemMutated,
                    (Some(1), _, _) => PrefixDriftReason::ToolCatalogChanged,
                    (Some(_), true, _) | (None, true, _) => PrefixDriftReason::HistoryCompacted,
                    (Some(_), false, _) => PrefixDriftReason::HistoryDiverged,
                    (None, false, true) => PrefixDriftReason::HistoryDiverged,
                    (None, false, false) => PrefixDriftReason::FullHit,
                };
                if !matches!(drift_reason, PrefixDriftReason::FullHit) {
                    state.clear_chunk_refs();
                }
                PrefixCacheEvaluation {
                    prompt_cache_key: prompt_cache_key.clone(),
                    total_chunks,
                    matched_chunks,
                    reusable_tokens,
                    busted_tokens,
                    hit_ratio_pct,
                    drift_reason,
                }
            }
        };

        state.last_chain = current_chunks;
        guard.evict_if_needed();
        eval
    }

    /// Checks whether a large read-only file/OKF output is already verbatim inside the active
    /// prompt prefix with identical `(mtime_ns, byte_len, content_sha256)`.
    /// Returns `Some((compact_stub, tokens_saved, short_hash))` when safe to replace with a ref.
    pub fn check_or_record_read_chunk(
        &self,
        session_id: &str,
        workspace_dir: &Path,
        tool_name: &str,
        args: &Value,
        raw_output: &str,
        iter_idx: usize,
    ) -> Option<(String, usize, String)> {
        if raw_output.len() < PREFIX_CACHE_CHUNK_REF_MIN_BYTES {
            return None;
        }
        let (canon_path, display_path, range_key) =
            extract_read_chunk_target(workspace_dir, tool_name, args)?;
        let (mtime_ns, byte_len) = file_mtime_and_len(&canon_path)?;
        let content_sha256 = sha256_short_hex(raw_output);
        let key = (canon_path, range_key);

        let Ok(mut guard) = self.inner.lock() else {
            return None;
        };
        guard.touch(session_id);
        let state = guard.sessions.entry(session_id.to_string()).or_default();

        if let Some(prev) = state.chunk_refs.get(&key)
            && prev.mtime_ns == mtime_ns
            && prev.byte_len == byte_len
            && prev.content_sha256 == content_sha256
            && iter_idx > prev.seen_iter
        {
            let seen_step = prev.seen_iter + 1;
            let stub = format!(
                "[zenko_chunk_ref: unchanged `{display_path}` (sha256 #{content_sha256}, {}B already in active prefix from step {seen_step}); file unmodified on disk — reuse existing prefix context or request a narrower line range]",
                raw_output.len()
            );
            let saved_bytes = raw_output.len().saturating_sub(stub.len());
            let saved_tokens = saved_bytes / 4;
            state.touch_chunk(&key);
            if saved_tokens > 0 {
                return Some((stub, saved_tokens, content_sha256));
            }
            return None;
        }

        state.insert_chunk(
            key,
            CachedFileChunk {
                mtime_ns,
                byte_len,
                content_sha256,
                seen_iter: iter_idx,
            },
        );
        guard.evict_if_needed();
        None
    }

    /// Invalidates any cached chunk references for a mutated file path.
    pub fn invalidate_file(&self, session_id: &str, workspace_dir: &Path, raw_path: &str) {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return;
        }
        let candidate = match Path::new(trimmed).is_absolute() {
            true => PathBuf::from(trimmed),
            false => workspace_dir.join(trimmed),
        };
        let target = std::fs::canonicalize(&candidate).unwrap_or(candidate);
        if let Ok(mut guard) = self.inner.lock()
            && let Some(state) = guard.sessions.get_mut(session_id)
        {
            state.invalidate_path(&target);
        }
    }

    /// Clears all chunk refs for a session (e.g., on rewind, compaction, or session reset).
    pub fn clear_session(&self, session_id: &str) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.sessions.remove(session_id);
            if let Some(pos) = guard.lru.iter().position(|s| s == session_id) {
                guard.lru.remove(pos);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenko_core::RiskLevel;

    fn sample_tool(name: &str) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: format!("tool {name}"),
            parameters: serde_json::json!({ "type": "object", "properties": { "b": {}, "a": {} } }),
            risk: RiskLevel::Safe,
        }
    }

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: EventRole::User,
            content: text.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }
    }

    #[test]
    fn chained_prefix_detects_full_hit_tool_change_system_mutation_and_compaction() {
        let tracker = SessionPrefixTracker::new();
        let tools_a = vec![sample_tool("read_file"), sample_tool("code_search")];
        let tools_a_reordered = vec![sample_tool("code_search"), sample_tool("read_file")];
        let tools_b = vec![sample_tool("read_file"), sample_tool("write_file")];

        let e1 = tracker.evaluate_and_record(
            "s1",
            "System prompt v1",
            &tools_a,
            &[user_msg("hello")],
            false,
        );
        assert_eq!(e1.drift_reason, PrefixDriftReason::FirstTurn);
        assert!(e1.prompt_cache_key.starts_with(PREFIX_CACHE_KEY_PREFIX));

        // Append message + reordered identical tool specs => exact FullHit & identical prompt_cache_key!
        let e2 = tracker.evaluate_and_record(
            "s1",
            "System prompt v1",
            &tools_a_reordered,
            &[user_msg("hello"), user_msg("follow up")],
            false,
        );
        assert_eq!(e2.drift_reason, PrefixDriftReason::FullHit);
        assert_eq!(e2.prompt_cache_key, e1.prompt_cache_key);
        assert_eq!(e2.hit_ratio_pct, 100);
        assert_eq!(e2.busted_tokens, 0);

        // Changing tool catalog busts at chunk 1 (SystemBase still matched!)
        let e3 = tracker.evaluate_and_record(
            "s1",
            "System prompt v1",
            &tools_b,
            &[user_msg("hello"), user_msg("follow up")],
            false,
        );
        assert_eq!(e3.drift_reason, PrefixDriftReason::ToolCatalogChanged);
        assert_eq!(e3.matched_chunks, 1);
        assert_ne!(e3.prompt_cache_key, e1.prompt_cache_key);

        // Mutating system prompt busts at chunk 0
        let e4 = tracker.evaluate_and_record(
            "s1",
            "System prompt v2 mutated",
            &tools_b,
            &[user_msg("hello"), user_msg("follow up")],
            false,
        );
        assert_eq!(e4.drift_reason, PrefixDriftReason::SystemMutated);
        assert_eq!(e4.matched_chunks, 0);

        // Compaction flag classifies history shrinkage cleanly
        let e5 = tracker.evaluate_and_record(
            "s1",
            "System prompt v2 mutated",
            &tools_b,
            &[user_msg("compacted summary")],
            true,
        );
        assert_eq!(e5.drift_reason, PrefixDriftReason::HistoryCompacted);
        assert_eq!(e5.matched_chunks, 2);
    }

    #[test]
    fn chunk_ref_cache_dedups_unchanged_file_and_invalidates_on_edit() {
        let base = std::env::temp_dir().join(format!("zenko_pfx_ref_{}", zenko_core::new_id()));
        let _ = std::fs::create_dir_all(&base);
        let file = base.join("big.rs");
        let content = "pub fn demo() { let x = 42; }\n".repeat(120);
        let _ = std::fs::write(&file, &content);

        let tracker = SessionPrefixTracker::new();
        let args = serde_json::json!({ "path": "big.rs" });

        // First read records chunk ref
        assert!(tracker
            .check_or_record_read_chunk("s1", &base, TOOL_READ_FILE, &args, &content, 0)
            .is_none());

        // Second read at later iteration with unchanged file returns compact chunk ref!
        let hit = tracker
            .check_or_record_read_chunk("s1", &base, TOOL_READ_FILE, &args, &content, 1)
            .expect("chunk ref hit");
        assert!(hit.0.contains("zenko_chunk_ref"));
        assert!(hit.1 > 400);

        // Invalidate after file mutation -> next read serves full fresh output
        tracker.invalidate_file("s1", &base, "big.rs");
        assert!(tracker
            .check_or_record_read_chunk("s1", &base, TOOL_READ_FILE, &args, &content, 2)
            .is_none());

        let _ = std::fs::remove_dir_all(&base);
    }
}
