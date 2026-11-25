//! Pure-Rust, zero-regex Read-Less Structural Tool-Output Compressor (CCR-backed).
//! Shrinks bulky ephemeral outputs (`shell_exec`, `job_read`, `git_tool`, `web_fetch`,
//! `script_block`, and external `mcp__*` blobs) BEFORE they consume LLM context tokens,
//! while strictly excluding `read_file` and code editors so line numbers and hash anchors
//! remain untouched. Every compressed payload is paired with an atomic raw `artifact:` file
//! so 100% of the original bytes stay recoverable on demand.

use serde_json::{Map, Value, json};
use std::collections::{BTreeSet, HashMap};
use zenko_core::{
    ARTIFACT_PATH_PREFIX, MCP_TOOL_PREFIX, READ_LESS_DEDUP_MIN_REPEATS, READ_LESS_ELIGIBLE_TOOLS,
    READ_LESS_MAX_JSON_ARRAY_SAMPLE, READ_LESS_MAX_JSON_KEYS, READ_LESS_MAX_SAMPLE_ROWS,
    READ_LESS_MAX_STRING_CHARS, READ_LESS_MAX_TAIL_LINES, READ_LESS_MIN_LINES,
    READ_LESS_MIN_SAVED_BYTES, READ_LESS_MIN_SAVINGS_PCT, READ_LESS_TAIL_ROWS,
};
use zenko_tools::pty::strip_ansi_escapes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadLessShape {
    Json,
    Csv,
    TestOutput,
    BuildOrLog,
}

impl ReadLessShape {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
            Self::TestOutput => "test_output",
            Self::BuildOrLog => "build_log",
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

/// Returns `true` strictly for noisy/ephemeral tools eligible for Read-Less compression.
/// Never matches `read_file`, `hash_edit`, `apply_patch`, `write_file`, `code_search`, or `lsp_tool`.
pub fn is_read_less_eligible(tool_name: &str) -> bool {
    READ_LESS_ELIGIBLE_TOOLS.contains(&tool_name) || tool_name.starts_with(MCP_TOOL_PREFIX)
}

/// Attempts deterministic structural compression of `raw`. Returns `None` when the tool is
/// ineligible, the payload is already compact/spilled, or compression yields insufficient savings.
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
    if original_bytes < min_bytes && line_count < READ_LESS_MIN_LINES {
        return None;
    }
    if raw.contains("[read-less:") || raw.contains("[spilled full `") || raw.contains(ARTIFACT_PATH_PREFIX) {
        return None;
    }

    let cleaned = strip_ansi_escapes(raw);
    let (shape, compressed) = compress_by_detected_shape(&cleaned)?;
    let compressed_bytes = compressed.len();
    if compressed_bytes >= original_bytes {
        return None;
    }
    let bytes_saved = original_bytes.saturating_sub(compressed_bytes);
    let pct_whole = (bytes_saved * 100) / original_bytes.max(1);
    if bytes_saved < READ_LESS_MIN_SAVED_BYTES || pct_whole < READ_LESS_MIN_SAVINGS_PCT {
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

fn compress_by_detected_shape(text: &str) -> Option<(ReadLessShape, String)> {
    // 1. JSON payload (whole body or body right after `exit_code: N\n`).
    if let Some(json_out) = try_compress_json_body(text) {
        return Some((ReadLessShape::Json, json_out));
    }
    // 2. Test runner output (cargo test / pytest / vitest / jest / go test).
    if let Some(test_out) = try_compress_test_output(text) {
        return Some((ReadLessShape::TestOutput, test_out));
    }
    // 3. Delimited CSV / TSV table.
    if let Some(csv_out) = try_compress_csv(text) {
        return Some((ReadLessShape::Csv, csv_out));
    }
    // 4. Build / compiler / git / generic bulky log deduplication.
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
            for item in items.iter().take(64) {
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
                let is_priority = matches!(
                    k.to_ascii_lowercase().as_str(),
                    "error" | "errors" | "message" | "status" | "code" | "ok" | "id" | "name" | "total" | "count"
                );
                if depth >= 3 && !is_priority && count >= READ_LESS_MAX_JSON_KEYS {
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
        || t.starts_with("--- PASS:")
        || t.starts_with("PASS ")
        || t.starts_with("✓ ")
        || t.starts_with("ok  \t")
        || t.ends_with(" PASSED")
}

fn is_failing_or_diag_line(line: &str) -> bool {
    let t = line.trim();
    let lower = t.to_ascii_lowercase();
    t.starts_with("test ") && (t.ends_with("FAILED") || t.contains("FAILED"))
        || t.starts_with("--- FAIL:")
        || t.starts_with("FAIL")
        || t.starts_with("✗ ")
        || t.starts_with("error[")
        || t.starts_with("error:")
        || t.starts_with("fatal:")
        || t.starts_with("E   ")
        || lower.contains("panicked at")
        || lower.contains("assertion ")
        || lower.contains("traceback (most recent")
        || lower.contains("permission denied")
}

fn is_test_summary_line(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("test result:")
        || (t.starts_with("running ") && t.contains(" test"))
        || t.starts_with("failures:")
        || t.starts_with("Test Files")
        || t.starts_with("Tests ")
        || t.starts_with("Summary:")
        || (t.starts_with("===") && (t.contains("passed") || t.contains("failed")))
}

fn is_build_progress_line(line: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "Compiling ",
        "Checking ",
        "Downloading ",
        "Downloaded ",
        "Updating ",
        "Fresh ",
        "Locking ",
        "Adding ",
        "Removing ",
        "Installing ",
        "Unpacking ",
        "Resolving ",
        "Fetching ",
        "Building ",
        "Doc-tests ",
    ];
    let t = line.trim_start();
    PREFIXES.iter().any(|p| t.starts_with(p))
}

fn try_compress_test_output(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let pass_count = lines.iter().filter(|l| is_passing_test_line(l)).count();
    let summary_count = lines.iter().filter(|l| is_test_summary_line(l)).count();
    if pass_count < 8 || (summary_count == 0 && pass_count < 16) {
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
            || (!trimmed.is_empty() && out.len() < 80);
        if keep_line {
            flush_collapsed(&mut out, &mut collapsed_pass, &mut collapsed_progress);
            out.push((*line).to_string());
        }
    }
    flush_collapsed(&mut out, &mut collapsed_pass, &mut collapsed_progress);
    Some(out.join("\n"))
}

fn detect_delimited_cols(lines: &[&str], delim: char) -> Option<usize> {
    let sample = &lines[..lines.len().min(12)];
    let first_cols = sample.first()?.split(delim).count();
    if first_cols < 3 {
        return None;
    }
    let consistent = sample
        .iter()
        .all(|l| l.split(delim).count() == first_cols);
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
    if rows.len() < 16 {
        return None;
    }
    let cols = detect_delimited_cols(&rows, ',').or_else(|| detect_delimited_cols(&rows, '\t'))?;
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

fn try_compress_build_or_log(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < READ_LESS_MIN_LINES {
        return None;
    }

    // Pass 1: collapse consecutive duplicates + build progress runs, count repeated lines.
    let mut deduped: Vec<String> = Vec::with_capacity(lines.len());
    let mut progress_count = 0usize;
    let mut freq: HashMap<&str, usize> = HashMap::new();
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
        let mut run = 1usize;
        while idx + run < lines.len() && lines[idx + run].trim() == trimmed {
            run += 1;
        }
        if run >= READ_LESS_DEDUP_MIN_REPEATS {
            deduped.push(format!("{cur} [x{run}]"));
        } else {
            let seen = freq.entry(trimmed).or_insert(0);
            *seen += 1;
            let keep = *seen <= 2 || is_failing_or_diag_line(cur) || idx + READ_LESS_MAX_TAIL_LINES >= lines.len();
            if keep {
                deduped.push(cur.to_string());
            }
        }
        idx += run;
    }
    if progress_count > 0 {
        deduped.push(format!("[+{progress_count} build/progress lines collapsed]"));
    }

    // Pass 2: if still verbose, keep head + all error/diag lines + bottom tail.
    let max_kept_lines = 48usize;
    if deduped.len() <= max_kept_lines {
        return Some(deduped.join("\n"));
    }
    let head_keep = 12usize;
    let tail_start = deduped.len().saturating_sub(READ_LESS_MAX_TAIL_LINES);
    let mut out: Vec<String> = Vec::with_capacity(max_kept_lines + 8);
    for line in &deduped[..head_keep.min(tail_start)] {
        out.push(line.clone());
    }
    let mut middle_omitted = 0usize;
    let mut middle_diags: Vec<&str> = Vec::new();
    for line in &deduped[head_keep.min(tail_start)..tail_start] {
        if is_failing_or_diag_line(line) && middle_diags.len() < 16 {
            middle_diags.push(line.as_str());
        } else {
            middle_omitted += 1;
        }
    }
    if !middle_diags.is_empty() {
        for d in middle_diags {
            out.push(d.to_string());
        }
    }
    if middle_omitted > 0 {
        out.push(format!("... [{middle_omitted} repetitive log lines collapsed] ..."));
    }
    for line in &deduped[tail_start..] {
        out.push(line.clone());
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
            "   Compiling zenko-core v0.0.20".to_string(),
            "   Compiling zenko-tools v0.0.20".to_string(),
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
    fn compresses_large_csv_keeping_header_sample_and_tail() {
        let mut lines = vec!["id,service,latency_ms,status".to_string()];
        for i in 0..80 {
            lines.push(format!("{i},api-gateway,{},ok", 10 + (i % 50)));
        }
        let raw = lines.join("\n");
        let res = try_compress_tool_output(TOOL_SHELL_EXEC, &raw, 256).expect("compressed");
        assert_eq!(res.shape, ReadLessShape::Csv);
        assert!(res.compressed.contains("id,service,latency_ms,status"));
        assert!(res.compressed.contains("80 data rows x 4 columns"));
        assert!(res.compressed.contains("79,api-gateway"));
    }
}
