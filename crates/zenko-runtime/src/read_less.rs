//! Pure-Rust, zero-regex Read-Less Structural Tool-Output Compressor (CCR-backed).
//! Shrinks bulky ephemeral outputs (`shell_exec`, `job_read`, `git_tool`, `web_fetch`,
//! `script_block`, and external `mcp__*` blobs) BEFORE they consume LLM context tokens,
//! while strictly excluding `read_file` and code editors so line numbers and hash anchors
//! remain untouched. Uses deterministic structural algorithms:
//! - JSON schema/array sampler (`serde_json`)
//! - Test-runner pass collapser preserving failures & stack traces verbatim
//! - RFC-4180 quote-aware finite-state CSV/TSV column detector
//! - Drain-style FNV-1a digit/hex-normalized log-template clustering
//! Every compressed payload is paired with an atomic raw `artifact:` file so 100% of the
//! original bytes stay recoverable on demand.

use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use zenko_core::{
    ARTIFACT_PATH_PREFIX, FNV1A_OFFSET_BASIS, FNV1A_PRIME, MCP_TOOL_PREFIX,
    READ_LESS_BUILD_PROGRESS_PREFIXES, READ_LESS_CSV_MIN_COLS, READ_LESS_CSV_MIN_ROWS,
    READ_LESS_CSV_SAMPLE_SCAN_LINES, READ_LESS_DEDUP_MAX_CALL_GAP,
    READ_LESS_DEDUP_MAX_ENTRIES_PER_SESSION, READ_LESS_DEDUP_MIN_BYTES,
    READ_LESS_DEDUP_MIN_REPEATS, READ_LESS_ELIGIBLE_TOOLS, READ_LESS_FAIL_PREFIXES,
    READ_LESS_FAIL_SUBSTRS, READ_LESS_GLOB_MAX_PER_DIR, READ_LESS_GLOB_MIN_LINES,
    READ_LESS_JSON_FULL_DEPTH, READ_LESS_JSON_PRIORITY_KEYS, READ_LESS_JSON_SCHEMA_SCAN_ITEMS,
    READ_LESS_LOG_HEAD_KEEP, READ_LESS_LOG_MAX_KEPT_LINES, READ_LESS_LOG_MAX_MIDDLE_DIAGS,
    READ_LESS_LOG_MAX_REPEAT_SAMPLES, READ_LESS_MAX_JSON_ARRAY_SAMPLE, READ_LESS_MAX_JSON_KEYS,
    READ_LESS_MAX_SAMPLE_ROWS, READ_LESS_MAX_STRING_CHARS, READ_LESS_MAX_TAIL_LINES,
    READ_LESS_MIN_LINES, READ_LESS_MIN_SAVED_BYTES, READ_LESS_MIN_SAVINGS_PCT,
    READ_LESS_PASS_PREFIXES, READ_LESS_SCRUB_MIN_BYTES, READ_LESS_SCRUB_MIN_SAVED_BYTES,
    READ_LESS_SUMMARY_PREFIXES, READ_LESS_TAIL_ROWS, READ_LESS_TEST_MAX_KEPT_OTHER_LINES,
    READ_LESS_TEST_MIN_PASS_WITHOUT_SUMMARY, READ_LESS_TEST_MIN_PASS_WITH_SUMMARY, TOOL_GLOB_FIND,
};
use zenko_tools::pty::strip_ansi_escapes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadLessShape {
    Json,
    Csv,
    GlobList,
    TestOutput,
    BuildOrLog,
    Scrub,
}

impl ReadLessShape {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
            Self::GlobList => "glob_list",
            Self::TestOutput => "test_output",
            Self::BuildOrLog => "build_log",
            Self::Scrub => "scrub",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReadLessResult {
    pub compressed: String,
    pub shape: ReadLessShape,
    pub original_bytes: usize,
    pub compressed_bytes: usize,
    pub bytes_saved: usize,
    pub tokens_saved: usize,
    pub savings_pct: f64,
}

/// Exact-match fingerprint for bounded per-session tool-output deduplication (zero heap bloat).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OutputFingerprint {
    tool: String,
    fnv1a: u64,
    len: usize,
    head: [u8; 32],
    tail: [u8; 32],
}

impl OutputFingerprint {
    fn compute(tool_name: &str, bytes: &[u8]) -> Self {
        let mut h = FNV1A_OFFSET_BASIS;
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(FNV1A_PRIME);
        }
        let mut head = [0u8; 32];
        let head_n = bytes.len().min(32);
        head[..head_n].copy_from_slice(&bytes[..head_n]);
        let mut tail = [0u8; 32];
        let tail_start = bytes.len().saturating_sub(32);
        let tail_n = bytes.len() - tail_start;
        tail[..tail_n].copy_from_slice(&bytes[tail_start..]);
        Self {
            tool: tool_name.to_string(),
            fnv1a: h,
            len: bytes.len(),
            head,
            tail,
        }
    }
}

#[derive(Debug, Clone)]
struct DedupRecord {
    last_seq: u64,
    prior_handle: Option<String>,
}

/// Result returned when an eligible tool emits byte-identical output already active in context.
#[derive(Debug, Clone)]
pub struct OutputDedupHit {
    pub calls_ago: u64,
    pub original_bytes: usize,
    pub compressed_bytes: usize,
    pub bytes_saved: usize,
    pub tokens_saved: usize,
    pub prior_handle: Option<String>,
    pub replacement: String,
}

/// Bounded, compaction-aware per-session LRU deduplicator for allowlisted ephemeral tool outputs.
/// Strictly excludes `read_file`, `code_search`, `lsp_tool`, and all file mutation tools.
#[derive(Debug, Default)]
pub struct SessionOutputDedup {
    call_seq: u64,
    entries: HashMap<OutputFingerprint, DedupRecord>,
    lru: VecDeque<OutputFingerprint>,
}

impl SessionOutputDedup {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.lru.clear();
        self.call_seq = 0;
    }

    fn touch_lru(&mut self, key: &OutputFingerprint) {
        if let Some(pos) = self.lru.iter().position(|k| k == key) {
            self.lru.remove(pos);
        }
        self.lru.push_back(key.clone());
        while self.entries.len() > READ_LESS_DEDUP_MAX_ENTRIES_PER_SESSION {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    /// Checks whether `raw` matches a recent identical output from `tool_name` in this session.
    /// Records the fingerprint when unseen/stale; returns `Some(OutputDedupHit)` on an active hit.
    pub fn check_or_record(
        &mut self,
        tool_name: &str,
        raw: &str,
        is_error: bool,
    ) -> Option<OutputDedupHit> {
        self.call_seq = self.call_seq.saturating_add(1);
        if is_error
            || !is_read_less_eligible(tool_name)
            || raw.len() < READ_LESS_DEDUP_MIN_BYTES
            || raw.contains("[dedup:")
            || raw.contains("[read-less:")
            || raw.contains("[spilled full `")
        {
            return None;
        }
        let cleaned = strip_ansi_escapes(raw);
        let trimmed = cleaned.trim();
        if trimmed.len() < READ_LESS_DEDUP_MIN_BYTES {
            return None;
        }
        let key = OutputFingerprint::compute(tool_name, trimmed.as_bytes());
        let now_seq = self.call_seq;

        if let Some(existing) = self.entries.get_mut(&key) {
            let gap = now_seq.saturating_sub(existing.last_seq);
            existing.last_seq = now_seq;
            let prior_handle = existing.prior_handle.clone();
            self.touch_lru(&key);
            if gap > 0 && gap <= READ_LESS_DEDUP_MAX_CALL_GAP {
                let original_bytes = raw.len();
                let handle_note = match &prior_handle {
                    Some(h) => format!(" · raw at `{h}`"),
                    None => String::new(),
                };
                let draft_saved = original_bytes.saturating_sub(96);
                let replacement = format!(
                    "[dedup: identical to prior `{tool_name}` output from {gap} tool call(s) ago ({original_bytes} B, ~{} tok saved){handle_note}]",
                    draft_saved.div_ceil(4)
                );
                if replacement.len() < original_bytes {
                    let bytes_saved = original_bytes.saturating_sub(replacement.len());
                    let tokens_saved = bytes_saved.div_ceil(4);
                    return Some(OutputDedupHit {
                        calls_ago: gap,
                        original_bytes,
                        compressed_bytes: replacement.len(),
                        bytes_saved,
                        tokens_saved,
                        prior_handle,
                        replacement,
                    });
                }
            }
            return None;
        }

        self.entries.insert(
            key.clone(),
            DedupRecord {
                last_seq: now_seq,
                prior_handle: None,
            },
        );
        self.touch_lru(&key);
        None
    }

    /// Associates a persisted `artifact:` handle with an already-recorded raw output fingerprint.
    pub fn attach_artifact_handle(&mut self, tool_name: &str, raw: &str, handle: String) {
        if !is_read_less_eligible(tool_name) || raw.len() < READ_LESS_DEDUP_MIN_BYTES {
            return;
        }
        let cleaned = strip_ansi_escapes(raw);
        let trimmed = cleaned.trim();
        let key = OutputFingerprint::compute(tool_name, trimmed.as_bytes());
        if let Some(rec) = self.entries.get_mut(&key) {
            rec.prior_handle = Some(handle);
        }
    }
}

/// Returns `true` strictly for noisy/ephemeral tools eligible for Read-Less compression & dedup.
/// Never matches `read_file`, `hash_edit`, `apply_patch`, `write_file`, `code_search`, or `lsp_tool`.
pub fn is_read_less_eligible(tool_name: &str) -> bool {
    READ_LESS_ELIGIBLE_TOOLS.contains(&tool_name) || tool_name.starts_with(MCP_TOOL_PREFIX)
}

/// Attempts deterministic structural compression (or lossless `scrub` on medium outputs) of `raw`.
/// Returns `None` when the tool is ineligible, already compact/spilled, or savings are negligible.
pub fn try_compress_tool_output(
    tool_name: &str,
    raw: &str,
    min_bytes: usize,
) -> Option<ReadLessResult> {
    if !is_read_less_eligible(tool_name) {
        return None;
    }
    let original_bytes = raw.len();
    let line_count = raw.lines().count();
    if raw.contains("[read-less:")
        || raw.contains("[dedup:")
        || raw.contains("[spilled full `")
        || raw.contains(ARTIFACT_PATH_PREFIX)
    {
        return None;
    }

    let cleaned = strip_ansi_escapes(raw);
    let heavy_eligible = original_bytes >= min_bytes || line_count >= READ_LESS_MIN_LINES;
    let (shape, compressed) = match heavy_eligible {
        true => compress_by_detected_shape(tool_name, &cleaned)
            .or_else(|| try_lossless_scrub(&cleaned).map(|s| (ReadLessShape::Scrub, s)))?,
        false if original_bytes >= READ_LESS_SCRUB_MIN_BYTES => {
            (ReadLessShape::Scrub, try_lossless_scrub(&cleaned)?)
        }
        false => return None,
    };

    let compressed_bytes = compressed.len();
    if compressed_bytes >= original_bytes {
        return None;
    }
    let bytes_saved = original_bytes.saturating_sub(compressed_bytes);
    let pct_whole = (bytes_saved * 100) / original_bytes.max(1);
    let meets_threshold = match shape {
        ReadLessShape::Scrub => bytes_saved >= READ_LESS_SCRUB_MIN_SAVED_BYTES,
        _ => bytes_saved >= READ_LESS_MIN_SAVED_BYTES && pct_whole >= READ_LESS_MIN_SAVINGS_PCT,
    };
    if !meets_threshold {
        return None;
    }
    let tokens_saved = bytes_saved.div_ceil(4);
    let savings_pct = ((bytes_saved as f64 / original_bytes as f64) * 1000.0).round() / 10.0;

    Some(ReadLessResult {
        compressed,
        shape,
        original_bytes,
        compressed_bytes,
        bytes_saved,
        tokens_saved,
        savings_pct,
    })
}

fn compress_by_detected_shape(tool_name: &str, text: &str) -> Option<(ReadLessShape, String)> {
    if let Some(json_out) = try_compress_json_body(text) {
        return Some((ReadLessShape::Json, json_out));
    }
    if let Some(test_out) = try_compress_test_output(text) {
        return Some((ReadLessShape::TestOutput, test_out));
    }
    if let Some(csv_out) = try_compress_csv(text) {
        return Some((ReadLessShape::Csv, csv_out));
    }
    if tool_name == TOOL_GLOB_FIND
        && let Some(glob_out) = try_compress_glob_list(text)
    {
        return Some((ReadLessShape::GlobList, glob_out));
    }
    try_compress_build_or_log(text).map(|out| (ReadLessShape::BuildOrLog, out))
}

fn split_optional_exit_header(text: &str) -> (Option<&str>, &str) {
    match text.strip_prefix("exit_code:") {
        Some(_) => match text.find('\n') {
            Some(pos) => (Some(&text[..pos]), &text[pos + 1..]),
            None => (Some(text), ""),
        },
        None => (None, text),
    }
}

fn try_compress_json_body(text: &str) -> Option<String> {
    let (header, body) = split_optional_exit_header(text);
    let trimmed = body.trim();
    if !matches!(trimmed.as_bytes().first(), Some(b'{' | b'[')) {
        return None;
    }
    let parsed: Value = serde_json::from_str(trimmed).ok()?;
    let compacted = compact_json_value(&parsed, 0);
    let rendered = serde_json::to_string(&compacted).ok()?;
    match header {
        Some(h) => Some(format!("{h}\n{rendered}")),
        None => Some(rendered),
    }
}

fn compact_json_value(val: &Value, depth: usize) -> Value {
    match val {
        Value::String(s) => match s.len() > READ_LESS_MAX_STRING_CHARS {
            true => {
                let cut = s.floor_char_boundary(READ_LESS_MAX_STRING_CHARS);
                let omitted = s.len().saturating_sub(cut);
                Value::String(format!("{}... [+{omitted} chars]", &s[..cut]))
            }
            false => Value::String(s.clone()),
        },
        Value::Array(items) => {
            if items.len() <= READ_LESS_MAX_JSON_ARRAY_SAMPLE {
                return Value::Array(
                    items
                        .iter()
                        .map(|v| compact_json_value(v, depth + 1))
                        .collect(),
                );
            }
            let sample: Vec<Value> = items
                .iter()
                .take(READ_LESS_MAX_JSON_ARRAY_SAMPLE)
                .map(|v| compact_json_value(v, depth + 1))
                .collect();
            let mut schema_keys: BTreeSet<&str> = BTreeSet::new();
            for item in items.iter().take(READ_LESS_JSON_SCHEMA_SCAN_ITEMS) {
                if let Value::Object(map) = item {
                    for k in map.keys() {
                        if schema_keys.len() < READ_LESS_MAX_JSON_KEYS {
                            schema_keys.insert(k.as_str());
                        }
                    }
                }
            }
            let mut out = Map::new();
            out.insert("_total_items".into(), json!(items.len()));
            out.insert(
                "_omitted_items".into(),
                json!(items.len().saturating_sub(sample.len())),
            );
            if !schema_keys.is_empty() {
                out.insert(
                    "_item_keys".into(),
                    Value::Array(
                        schema_keys
                            .into_iter()
                            .map(|k| Value::String(k.to_string()))
                            .collect(),
                    ),
                );
            }
            out.insert("_sample".into(), Value::Array(sample));
            Value::Object(out)
        }
        Value::Object(map) => {
            let mut out = Map::new();
            let mut count = 0usize;
            for (k, v) in map {
                let is_priority = READ_LESS_JSON_PRIORITY_KEYS
                    .iter()
                    .any(|pk| k.eq_ignore_ascii_case(pk));
                if depth >= READ_LESS_JSON_FULL_DEPTH
                    && !is_priority
                    && count >= READ_LESS_MAX_JSON_KEYS
                {
                    continue;
                }
                out.insert(k.clone(), compact_json_value(v, depth + 1));
                count += 1;
            }
            let omitted = map.len().saturating_sub(out.len());
            if omitted > 0 {
                out.insert("_omitted_keys".into(), json!(omitted));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

fn is_passing_test_line(line: &str) -> bool {
    let t = line.trim();
    (t.starts_with("test ") && t.ends_with("... ok"))
        || t.ends_with(" PASSED")
        || READ_LESS_PASS_PREFIXES.iter().any(|p| t.starts_with(p))
}

fn is_failing_or_diag_line(line: &str) -> bool {
    let t = line.trim();
    if (t.starts_with("test ") && t.contains("FAILED"))
        || READ_LESS_FAIL_PREFIXES.iter().any(|p| t.starts_with(p))
    {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    READ_LESS_FAIL_SUBSTRS.iter().any(|s| lower.contains(s))
}

fn is_test_summary_line(line: &str) -> bool {
    let t = line.trim();
    READ_LESS_SUMMARY_PREFIXES.iter().any(|p| t.starts_with(p))
        || (t.starts_with("running ") && t.contains(" test"))
        || (t.starts_with("===") && (t.contains("passed") || t.contains("failed")))
}

fn is_build_progress_line(line: &str) -> bool {
    let t = line.trim_start();
    READ_LESS_BUILD_PROGRESS_PREFIXES
        .iter()
        .any(|p| t.starts_with(p))
}

fn try_compress_test_output(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let pass_count = lines.iter().filter(|l| is_passing_test_line(l)).count();
    let summary_count = lines.iter().filter(|l| is_test_summary_line(l)).count();
    if pass_count < READ_LESS_TEST_MIN_PASS_WITH_SUMMARY
        || (summary_count == 0 && pass_count < READ_LESS_TEST_MIN_PASS_WITHOUT_SUMMARY)
    {
        return None;
    }

    let mut out: Vec<String> = Vec::with_capacity(lines.len() / 3 + 8);
    let mut collapsed_pass = 0usize;
    let mut collapsed_progress = 0usize;
    let mut in_failures_section = false;

    let flush_collapsed = |out: &mut Vec<String>, pass: &mut usize, prog: &mut usize| {
        if *prog > 0 {
            out.push(format!("  [+{} build/progress lines collapsed]", *prog));
            *prog = 0;
        }
        if *pass > 0 {
            out.push(format!("  [+{} passing tests collapsed]", *pass));
            *pass = 0;
        }
    };

    for line in &lines {
        let trimmed = line.trim();
        if trimmed == "failures:" {
            flush_collapsed(&mut out, &mut collapsed_pass, &mut collapsed_progress);
            in_failures_section = true;
            out.push((*line).to_string());
            continue;
        }
        if in_failures_section {
            out.push((*line).to_string());
            if trimmed.starts_with("test result:") {
                in_failures_section = false;
            }
            continue;
        }
        if is_passing_test_line(line) {
            collapsed_pass += 1;
            continue;
        }
        if is_build_progress_line(line) {
            collapsed_progress += 1;
            continue;
        }
        let keep_line = is_failing_or_diag_line(line)
            || is_test_summary_line(line)
            || trimmed.starts_with("exit_code:")
            || trimmed.starts_with("Finished ")
            || trimmed.starts_with("Running ")
            || (!trimmed.is_empty() && out.len() < READ_LESS_TEST_MAX_KEPT_OTHER_LINES);
        if keep_line {
            flush_collapsed(&mut out, &mut collapsed_pass, &mut collapsed_progress);
            out.push((*line).to_string());
        }
    }
    flush_collapsed(&mut out, &mut collapsed_pass, &mut collapsed_progress);
    Some(out.join("\n"))
}

/// RFC-4180 quote-aware finite-state column counter (zero regex).
/// Accurately counts columns when quoted fields contain embedded delimiters or `""` escapes.
fn count_delimited_columns(line: &str, delim: u8) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut cols = 1usize;
    let mut in_quotes = false;
    let mut at_field_start = true;
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        match (in_quotes, b) {
            (true, b'"') => match bytes.get(i + 1) == Some(&b'"') {
                true => i += 2,
                false => {
                    in_quotes = false;
                    i += 1;
                }
            },
            (true, _) => i += 1,
            (false, b'"') if at_field_start => {
                in_quotes = true;
                at_field_start = false;
                i += 1;
            }
            (false, c) if c == delim => {
                cols += 1;
                at_field_start = true;
                i += 1;
            }
            (false, _) => {
                at_field_start = false;
                i += 1;
            }
        }
    }
    match in_quotes {
        true => None,
        false => Some(cols),
    }
}

fn detect_delimited_cols(lines: &[&str], delim: u8) -> Option<usize> {
    let sample = &lines[..lines.len().min(READ_LESS_CSV_SAMPLE_SCAN_LINES)];
    let first_cols = count_delimited_columns(sample.first()?, delim)?;
    if first_cols < READ_LESS_CSV_MIN_COLS {
        return None;
    }
    let consistent = sample
        .iter()
        .all(|l| count_delimited_columns(l, delim) == Some(first_cols));
    match consistent {
        true => Some(first_cols),
        false => None,
    }
}

fn try_compress_csv(text: &str) -> Option<String> {
    let (header_line, body) = split_optional_exit_header(text);
    let rows: Vec<&str> = body
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    if rows.len() < READ_LESS_CSV_MIN_ROWS {
        return None;
    }
    let cols =
        detect_delimited_cols(&rows, b',').or_else(|| detect_delimited_cols(&rows, b'\t'))?;
    let keep_head = 1 + READ_LESS_MAX_SAMPLE_ROWS;
    let keep_tail = READ_LESS_TAIL_ROWS;
    if rows.len() <= keep_head + keep_tail + 2 {
        return None;
    }

    let omitted = rows.len().saturating_sub(keep_head + keep_tail);
    let data_rows = rows.len().saturating_sub(1);
    let mut out = Vec::with_capacity(keep_head + keep_tail + 2);
    if let Some(h) = header_line {
        out.push(h.to_string());
    }
    for row in &rows[..keep_head] {
        out.push((*row).to_string());
    }
    out.push(format!(
        "... [{omitted} rows omitted; {data_rows} data rows x {cols} columns] ..."
    ));
    for row in &rows[rows.len() - keep_tail..] {
        out.push((*row).to_string());
    }
    Some(out.join("\n"))
}

#[inline]
fn fnv1a_byte(hash: &mut u64, b: u8) {
    *hash ^= u64::from(b);
    *hash = hash.wrapping_mul(FNV1A_PRIME);
}

fn is_hex_or_uuid_token(tok: &str) -> bool {
    let body = tok
        .strip_prefix("0x")
        .or_else(|| tok.strip_prefix("0X"))
        .unwrap_or(tok);
    let is_prefixed = body.len() != tok.len();
    let mut hex_count = 0usize;
    let mut has_digit = false;
    for b in body.bytes() {
        match b {
            b'0'..=b'9' => {
                hex_count += 1;
                has_digit = true;
            }
            b'a'..=b'f' | b'A'..=b'F' => hex_count += 1,
            b'-' if !is_prefixed => {}
            _ => return false,
        }
    }
    match is_prefixed {
        true => hex_count >= 2,
        false => hex_count >= 6 && has_digit,
    }
}

fn hash_log_atom(h: &mut u64, atom: &str) {
    let core = atom.trim_matches(|c: char| {
        matches!(c, '[' | ']' | '(' | ')' | ',' | ';' | '"' | '\'')
    });
    if is_hex_or_uuid_token(core) {
        fnv1a_byte(h, b'$');
        return;
    }
    let mut prev_digit = false;
    for b in atom.bytes() {
        match b.is_ascii_digit() {
            true if prev_digit => {}
            true => {
                fnv1a_byte(h, b'#');
                prev_digit = true;
            }
            false => {
                fnv1a_byte(h, b.to_ascii_lowercase());
                prev_digit = false;
            }
        }
    }
}

/// Drain-style deterministic 64-bit FNV-1a structural log template fingerprinter (zero regex).
/// Splits `key=val` pairs and collapses digit runs (`12ms`, `09:12:01`, `v1.2.3`) to `#` and
/// hex/UUID literals (`0xdead000a`, `550e8400-e29b-...`) to `$` while preserving the exact
/// alphabetic/punctuation skeleton so repeating log families cluster algorithmically.
fn structural_log_template_hash(line: &str) -> u64 {
    let mut h = FNV1A_OFFSET_BASIS;
    for tok in line.split_whitespace() {
        match tok.split_once('=') {
            Some((key, val)) if !key.is_empty() && !val.is_empty() => {
                hash_log_atom(&mut h, key);
                fnv1a_byte(&mut h, b'=');
                hash_log_atom(&mut h, val);
            }
            _ => hash_log_atom(&mut h, tok),
        }
        fnv1a_byte(&mut h, b' ');
    }
    h
}

fn try_compress_build_or_log(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < READ_LESS_MIN_LINES {
        return None;
    }

    let hashes: Vec<u64> = lines
        .iter()
        .map(|l| structural_log_template_hash(l.trim()))
        .collect();

    // Pass 1: Drain-style template clustering over consecutive runs + global template frequency cap.
    let mut deduped: Vec<String> = Vec::with_capacity(lines.len());
    let mut progress_count = 0usize;
    let mut template_freq: HashMap<u64, usize> = HashMap::new();
    let mut suppressed_by_freq = 0usize;
    let mut idx = 0usize;

    while idx < lines.len() {
        let cur = lines[idx];
        let trimmed = cur.trim();
        if is_build_progress_line(cur) {
            progress_count += 1;
            idx += 1;
            continue;
        }
        if progress_count > 0 {
            deduped.push(format!("[+{progress_count} build/progress lines collapsed]"));
            progress_count = 0;
        }
        if trimmed.is_empty() {
            if deduped.last().is_some_and(|prev| !prev.is_empty()) {
                deduped.push(String::new());
            }
            idx += 1;
            continue;
        }

        let cur_hash = hashes[idx];
        let is_diag = is_failing_or_diag_line(cur);
        let mut run = 1usize;
        if !is_diag {
            while idx + run < lines.len()
                && !is_failing_or_diag_line(lines[idx + run])
                && hashes[idx + run] == cur_hash
            {
                run += 1;
            }
        }

        if run >= READ_LESS_DEDUP_MIN_REPEATS {
            let exact_same = lines[idx..idx + run]
                .iter()
                .all(|l| l.trim() == trimmed);
            let tag = match exact_same {
                true => format!("[x{run}]"),
                false => format!("[~x{run} similar lines]"),
            };
            deduped.push(format!("{cur} {tag}"));
        } else {
            let seen = template_freq.entry(cur_hash).or_insert(0);
            *seen += 1;
            let in_tail = idx + READ_LESS_MAX_TAIL_LINES >= lines.len();
            let keep = *seen <= READ_LESS_LOG_MAX_REPEAT_SAMPLES || is_diag || in_tail;
            match keep {
                true => deduped.push(cur.to_string()),
                false => suppressed_by_freq += 1,
            }
        }
        idx += run;
    }
    if progress_count > 0 {
        deduped.push(format!("[+{progress_count} build/progress lines collapsed]"));
    }
    if suppressed_by_freq > 0 {
        deduped.push(format!(
            "[+{suppressed_by_freq} recurring template log lines collapsed]"
        ));
    }

    // Pass 2: if still verbose, keep head + all diagnostic lines + bottom tail.
    if deduped.len() <= READ_LESS_LOG_MAX_KEPT_LINES {
        return Some(deduped.join("\n"));
    }
    let tail_start = deduped.len().saturating_sub(READ_LESS_MAX_TAIL_LINES);
    let head_end = READ_LESS_LOG_HEAD_KEEP.min(tail_start);
    let mut out: Vec<String> = Vec::with_capacity(READ_LESS_LOG_MAX_KEPT_LINES + 8);
    for line in &deduped[..head_end] {
        out.push(line.clone());
    }
    let mut middle_omitted = 0usize;
    let mut middle_diags: Vec<&str> = Vec::new();
    for line in &deduped[head_end..tail_start] {
        if is_failing_or_diag_line(line) && middle_diags.len() < READ_LESS_LOG_MAX_MIDDLE_DIAGS {
            middle_diags.push(line.as_str());
        } else {
            middle_omitted += 1;
        }
    }
    for d in middle_diags {
        out.push(d.to_string());
    }
    if middle_omitted > 0 {
        out.push(format!(
            "... [{middle_omitted} repetitive log lines collapsed] ..."
        ));
    }
    for line in &deduped[tail_start..] {
        out.push(line.clone());
    }
    Some(out.join("\n"))
}

/// Lossless `scrub` tier (inspired by Chisle): trims trailing whitespace, collapses consecutive
/// blank runs to a single blank line, and folds `>= 3` exact consecutive identical lines to `[xN]`.
/// Preserves every unique line and every diagnostic verbatim (zero semantic loss).
fn try_lossless_scrub(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    if lines.is_empty() {
        return None;
    }
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut idx = 0usize;
    while idx < lines.len() {
        let cur = lines[idx];
        if cur.is_empty() {
            if out.last().is_some_and(|prev| !prev.is_empty()) {
                out.push(String::new());
            }
            idx += 1;
            continue;
        }
        let mut run = 1usize;
        while idx + run < lines.len() && lines[idx + run] == cur {
            run += 1;
        }
        match run >= READ_LESS_DEDUP_MIN_REPEATS {
            true => out.push(format!("{cur} [x{run}]")),
            false => {
                for _ in 0..run {
                    out.push(cur.to_string());
                }
            }
        }
        idx += run;
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    let scrubbed = out.join("\n");
    match scrubbed.len() < text.len() {
        true => Some(scrubbed),
        false => None,
    }
}

/// Groups repetitive `glob_find` path listings by parent directory, slashing repeated path
/// prefixes while preserving complete directory structure and sample filenames per folder.
fn try_compress_glob_list(text: &str) -> Option<String> {
    let mut header_lines: Vec<&str> = Vec::new();
    let mut grouped: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut path_count = 0usize;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("Found ") || line.starts_with('[') || line.contains(" file(s)") {
            header_lines.push(line);
            continue;
        }
        let candidate = line.split_whitespace().next().unwrap_or(line);
        if candidate.contains('/') {
            if let Some((dir, file)) = candidate.rsplit_once('/') {
                let dir_key = match dir.is_empty() {
                    true => "/",
                    false => dir,
                };
                grouped.entry(dir_key).or_default().push(file);
                path_count += 1;
            }
        } else if candidate.contains('.') {
            grouped.entry(".").or_default().push(candidate);
            path_count += 1;
        }
    }

    if path_count < READ_LESS_GLOB_MIN_LINES || grouped.is_empty() {
        return None;
    }

    let mut out: Vec<String> = Vec::with_capacity(grouped.len() + header_lines.len() + 2);
    for h in header_lines {
        out.push(h.to_string());
    }
    out.push(format!(
        "Grouped {path_count} paths across {} director(y/ies):",
        grouped.len()
    ));
    for (dir, files) in grouped {
        let shown = files.len().min(READ_LESS_GLOB_MAX_PER_DIR);
        let joined = files[..shown].join(", ");
        let extra = files.len().saturating_sub(shown);
        match extra > 0 {
            true => out.push(format!("  {dir}/ ({}): {joined}, [+{extra} more]", files.len())),
            false => out.push(format!("  {dir}/ ({}): {joined}", files.len())),
        }
    }
    Some(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenko_core::{TOOL_READ_FILE, TOOL_SHELL_EXEC};

    #[test]
    fn read_file_is_strictly_exempt_from_read_less() {
        assert!(!is_read_less_eligible(TOOL_READ_FILE));
        assert!(is_read_less_eligible(TOOL_SHELL_EXEC));
        assert!(is_read_less_eligible("mcp__github__list_issues"));
        let big = "line\n".repeat(200);
        assert!(try_compress_tool_output(TOOL_READ_FILE, &big, 128).is_none());
    }

    #[test]
    fn compresses_large_json_array_preserving_schema_and_sample() {
        let items: Vec<Value> = (0..60)
            .map(|i| {
                json!({
                    "id": i,
                    "name": format!("pkg-{i}"),
                    "status": "ok",
                    "payload": "x".repeat(40)
                })
            })
            .collect();
        let raw = serde_json::to_string_pretty(&Value::Array(items)).expect("json");
        let res = try_compress_tool_output("mcp__demo__list", &raw, 256).expect("compressed");
        assert_eq!(res.shape, ReadLessShape::Json);
        assert!(res.savings_pct >= 50.0, "pct={}", res.savings_pct);
        assert!(res.compressed.contains("\"_total_items\":60"));
        assert!(res.compressed.contains("\"_item_keys\""));
    }

    #[test]
    fn compresses_cargo_test_passing_noise_while_keeping_failures_verbatim() {
        let mut lines = vec![
            "exit_code: 101".to_string(),
            "   Compiling zenko-core v0.0.22".to_string(),
            "   Compiling zenko-tools v0.0.22".to_string(),
            "running 30 tests".to_string(),
        ];
        for i in 0..29 {
            lines.push(format!("test module::tests::case_{i} ... ok"));
        }
        lines.push("test module::tests::broken_case ... FAILED".to_string());
        lines.push("failures:".to_string());
        lines.push("---- module::tests::broken_case stdout ----".to_string());
        lines.push("thread 'broken_case' panicked at src/lib.rs:42: assertion left == right failed".to_string());
        lines.push("test result: FAILED. 29 passed; 1 failed; 0 ignored".to_string());
        let raw = lines.join("\n");

        let res = try_compress_tool_output(TOOL_SHELL_EXEC, &raw, 256).expect("compressed");
        assert_eq!(res.shape, ReadLessShape::TestOutput);
        assert!(res.compressed.contains("+29 passing tests collapsed"));
        assert!(res.compressed.contains("broken_case ... FAILED"));
        assert!(res.compressed.contains("panicked at src/lib.rs:42"));
        assert!(res.compressed.contains("test result: FAILED. 29 passed; 1 failed"));
    }

    #[test]
    fn compresses_rfc4180_quoted_csv_with_embedded_commas() {
        let mut lines = vec!["id,owner,service,latency_ms,status".to_string()];
        for i in 0..80 {
            lines.push(format!(
                "{i},\"Smith, Alice \"\"Dev\"\"\",api-gateway,{},ok",
                10 + (i % 50)
            ));
        }
        let raw = lines.join("\n");
        let res = try_compress_tool_output(TOOL_SHELL_EXEC, &raw, 256).expect("compressed");
        assert_eq!(res.shape, ReadLessShape::Csv);
        assert!(res.compressed.contains("id,owner,service,latency_ms,status"));
        assert!(res.compressed.contains("80 data rows x 5 columns"));
        assert!(res.compressed.contains("79,\"Smith, Alice"));
    }

    #[test]
    fn clusters_timestamped_repeating_logs_via_fnv1a_templates_preserving_errors() {
        let mut lines = vec!["exit_code: 0".to_string()];
        for i in 0..45 {
            lines.push(format!(
                "[09:12:{:02}.{:03}] GET /api/health 200 {}ms req=0x{:08x}",
                i % 60,
                (i * 17) % 1000,
                10 + (i % 9),
                0xdead_0000u32 + i as u32
            ));
        }
        lines.push("error: upstream socket reset on shard 3".to_string());
        for i in 45..70 {
            lines.push(format!(
                "[09:13:{:02}.{:03}] GET /api/health 200 {}ms req=0x{:08x}",
                i % 60,
                (i * 13) % 1000,
                12 + (i % 7),
                0xbeef_0000u32 + i as u32
            ));
        }
        let raw = lines.join("\n");
        let res = try_compress_tool_output(TOOL_SHELL_EXEC, &raw, 256).expect("compressed");
        assert_eq!(res.shape, ReadLessShape::BuildOrLog);
        assert!(res.compressed.contains("[~x45 similar lines]"), "out={}", res.compressed);
        assert!(res.compressed.contains("error: upstream socket reset on shard 3"));
        assert!(res.compressed.contains("[~x25 similar lines]"), "out={}", res.compressed);
    }

    #[test]
    fn session_output_dedup_collapses_exact_repeats_and_respects_clear_and_exemptions() {
        let mut dedup = SessionOutputDedup::default();
        let body = "M crates/zenko-core/src/types.rs\n".repeat(25);

        // Exempt tool (`read_file`) and error results are never deduped
        assert!(dedup.check_or_record(TOOL_READ_FILE, &body, false).is_none());
        assert!(dedup.check_or_record(TOOL_READ_FILE, &body, false).is_none());
        assert!(dedup.check_or_record(TOOL_SHELL_EXEC, &body, true).is_none());

        // First eligible call records fingerprint; attaching artifact links future dedup hits
        assert!(dedup.check_or_record(TOOL_SHELL_EXEC, &body, false).is_none());
        dedup.attach_artifact_handle(
            TOOL_SHELL_EXEC,
            &body,
            "artifact:s1/c1.txt".to_string(),
        );

        // Identical second call hits dedup cleanly and cites the raw artifact handle
        let hit = dedup
            .check_or_record(TOOL_SHELL_EXEC, &body, false)
            .expect("second identical call must dedup");
        assert_eq!(hit.calls_ago, 1);
        assert!(hit.bytes_saved > 500);
        assert!(hit.tokens_saved > 100);
        assert!(hit.replacement.contains("[dedup:"));
        assert!(hit.replacement.contains("artifact:s1/c1.txt"));

        // Compaction reset clears cache so subsequent call emits fresh output
        dedup.clear();
        assert!(dedup.check_or_record(TOOL_SHELL_EXEC, &body, false).is_none());
    }

    #[test]
    fn lossless_scrub_and_glob_directory_grouping_compress_cleanly() {
        // Medium output below heavy min_bytes=4096 with ANSI escapes, blank runs, and exact repeats
        let mut medium = String::new();
        for _ in 0..8 {
            medium.push_str("\x1b[32mstatus: healthy\x1b[0m   \n");
        }
        medium.push_str("\n\n\n\n");
        for i in 0..6 {
            medium.push_str(&format!("\x1b[33munique-line-{i}\x1b[0m     \n"));
        }
        let scrubbed =
            try_compress_tool_output(TOOL_SHELL_EXEC, &medium, 4096).expect("lossless scrub");
        assert_eq!(scrubbed.shape, ReadLessShape::Scrub);
        assert!(scrubbed.compressed.contains("status: healthy [x8]"));
        assert!(scrubbed.compressed.contains("unique-line-5"));
        assert!(!scrubbed.compressed.contains("\x1b["));

        // Verbose glob_find listing compresses cleanly by parent directory
        let mut glob_lines = vec!["Found 36 file(s):".to_string()];
        for dir in ["crates/zenko-core/src", "crates/zenko-tools/src", "webui/src/components"] {
            for i in 0..12 {
                glob_lines.push(format!("{dir}/module_{i:02}.rs"));
            }
        }
        let glob_raw = glob_lines.join("\n");
        let glob_res =
            try_compress_tool_output(TOOL_GLOB_FIND, &glob_raw, 256).expect("glob_list compressed");
        assert_eq!(glob_res.shape, ReadLessShape::GlobList);
        assert!(glob_res.compressed.contains("crates/zenko-core/src/ (12):"));
        assert!(glob_res.compressed.contains("[+4 more]"));
    }
}
