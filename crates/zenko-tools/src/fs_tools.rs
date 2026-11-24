use crate::jail::{resolve_readable_path, resolve_workspace_or_hub_path};
use crate::outline::{extract_outline, format_outline_report};
use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use similar::TextDiff;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;
use zenko_core::{
    APPLY_PATCH_AMBIGUITY_MAX_LOCS, APPLY_PATCH_MAX_EDITS, DEFAULT_READ_FILE_LINE_WINDOW,
    DEFAULT_READ_FILE_MAX_BYTES, HASH_EDIT_MAX_HUNKS, HASH_EDIT_RECOVERY_RADIUS,
    OUTLINE_MAX_ENTRIES, OUTLINE_MAX_FILE_BYTES, READ_FILE_MAX_RANGES, ResourceLockKey, Result,
    RiskLevel, TOOL_APPLY_PATCH, TOOL_HASH_EDIT, TOOL_LIST_DIR, TOOL_READ_FILE, TOOL_WRITE_FILE,
    Tool, ToolConcurrency, ToolContext, ToolOutput, ToolSet, ToolSpec, ZenkoError,
};

/// Deterministic 3-hex-char content hash for Oh My Pi / Oh My OpenCode hash-anchored edits.
pub fn line_hash(line: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(line.trim_end_matches('\r').as_bytes());
    let digest = hasher.finalize();
    format!("{:02x}{:x}", digest[0], digest[1] >> 4)
}

pub fn unified_diff(path: &str, old: &str, new: &str) -> String {
    TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}

const MAX_EDIT_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SINGLE_READ_LINE_BYTES: usize = 128 * 1024;

fn detect_line_ending(text: &str) -> &'static str {
    match text.contains("\r\n") {
        true => "\r\n",
        false => "\n",
    }
}

/// Deterministic canonical resource-lock key for file mutation tools.
/// Disjoint relative workspace files receive distinct `ToolConcurrency::Keyed(...)` keys so
/// independent edits execute concurrently while same-file edits serialize strictly.
/// Absolute, tilde, `hub:`, or parent-traversal (`..`) paths fall back conservatively to `Global`
/// so relative vs absolute references to the same file can never race.
fn file_lock_key(args: &Value) -> ToolConcurrency {
    let Some(raw) = args.get("path").and_then(Value::as_str).map(str::trim) else {
        return ToolConcurrency::Global;
    };
    if raw.is_empty()
        || raw.starts_with('/')
        || raw.starts_with('\\')
        || raw.starts_with('~')
        || raw.contains(':')
    {
        return ToolConcurrency::Global;
    }
    let mut parts: Vec<&str> = Vec::new();
    for seg in raw.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => return ToolConcurrency::Global,
            other => parts.push(other),
        }
    }
    match parts.is_empty() {
        true => ToolConcurrency::Global,
        false => ToolConcurrency::Keyed(ResourceLockKey::File(parts.join("/"))),
    }
}

/// Formats a compact fresh `line:hash| text` recovery window around `target_line` (1-based)
/// so hash-mismatch errors give the LLM exact current anchors without an extra `read_file` turn.
fn format_hash_recovery_window(lines: &[String], target_line: usize) -> String {
    if lines.is_empty() {
        return "(file is empty)".into();
    }
    let clamped = target_line.clamp(1, lines.len());
    let lo = clamped.saturating_sub(HASH_EDIT_RECOVERY_RADIUS).max(1);
    let hi = clamped
        .saturating_add(HASH_EDIT_RECOVERY_RADIUS)
        .min(lines.len());
    let mut out = String::new();
    for line_no in lo..=hi {
        let text = &lines[line_no - 1];
        let h = line_hash(text);
        let marker = match line_no == clamped {
            true => ">>",
            false => "  ",
        };
        out.push_str(&format!("{marker} {line_no}:{h}| {text}\n"));
    }
    out
}

// ─── read_file (Hash-Anchored Lines, Multi-Range Slices & Structural Outline) ──

pub struct ReadFileTool;

#[derive(Debug, Clone, Copy)]
struct LineRange {
    start: usize,
    end: usize,
}

fn parse_read_ranges(args: &Value) -> Result<Vec<LineRange>> {
    if let Some(arr) = args.get("ranges").and_then(Value::as_array) {
        if arr.is_empty() {
            return Err(ZenkoError::Tool(
                "`ranges` must contain at least one `{start_line, end_line}` object".into(),
            ));
        }
        if arr.len() > READ_FILE_MAX_RANGES {
            return Err(ZenkoError::Tool(format!(
                "`ranges` exceeds max {READ_FILE_MAX_RANGES} slices per call"
            )));
        }
        let mut parsed = Vec::with_capacity(arr.len());
        for (idx, item) in arr.iter().enumerate() {
            let start = item
                .get("start_line")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    ZenkoError::Tool(format!("ranges[{idx}] missing integer `start_line`"))
                })?
                .max(1) as usize;
            let end = item
                .get("end_line")
                .and_then(Value::as_u64)
                .map(|v| (v as usize).max(start))
                .unwrap_or(start.saturating_add(DEFAULT_READ_FILE_LINE_WINDOW));
            parsed.push(LineRange { start, end });
        }
        parsed.sort_by_key(|r| (r.start, r.end));
        // Coalesce overlapping or directly adjacent windows cleanly.
        let mut merged: Vec<LineRange> = Vec::with_capacity(parsed.len());
        for r in parsed {
            match merged.last_mut() {
                Some(prev) if r.start <= prev.end.saturating_add(1) => {
                    prev.end = prev.end.max(r.end);
                }
                _ => merged.push(r),
            }
        }
        return Ok(merged);
    }

    let start = args
        .get("start_line")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let end = args
        .get("end_line")
        .and_then(Value::as_u64)
        .map(|v| (v as usize).max(start))
        .unwrap_or(start.saturating_add(DEFAULT_READ_FILE_LINE_WINDOW));
    Ok(vec![LineRange { start, end }])
}

#[async_trait]
impl Tool for ReadFileTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::CoreFs
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_READ_FILE.into(),
            description: "Read a workspace, `hub:`, or `artifact:` file. Returns hash-anchored lines (`line:hash| text`) for exact `hash_edit` mutations. Supports single window (`start_line`/`end_line`), multiple non-contiguous `ranges`, or `mode=\"outline\"` for an instant zero-dependency structural symbol map.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative workspace, `hub:`, or `artifact:` path to file" },
                    "mode": { "type": "string", "enum": ["lines", "outline"], "description": "`lines` (default) for hash-anchored line content, or `outline` for a compact structural symbol map with line spans & hashes" },
                    "start_line": { "type": "integer", "description": "1-based start line (default 1)" },
                    "end_line": { "type": "integer", "description": "1-based inclusive end line (default start_line + 200)" },
                    "ranges": {
                        "type": "array",
                        "description": "Optional list of up to 8 `{start_line, end_line}` slices to read multiple sections in one call",
                        "items": {
                            "type": "object",
                            "properties": {
                                "start_line": { "type": "integer" },
                                "end_line": { "type": "integer" }
                            },
                            "required": ["start_line"]
                        }
                    },
                    "max_symbols": { "type": "integer", "description": "Optional symbol cap when `mode=\"outline\"` (default 250)" }
                },
                "required": ["path"]
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
        let req_path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `path`".into()))?;
        let path = resolve_readable_path(
            &ctx.workspace_dir,
            &ctx.hub_dir,
            &ctx.data_dir,
            req_path,
            true,
        )?;
        let meta = tokio::fs::metadata(&path).await?;
        let file_bytes = meta.len();

        let mode = args
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("lines")
            .trim()
            .to_ascii_lowercase();

        if mode == "outline" {
            if file_bytes > OUTLINE_MAX_FILE_BYTES {
                return Ok(ToolOutput::err(format!(
                    "File '{}' ({file_bytes} bytes) exceeds outline size cap ({OUTLINE_MAX_FILE_BYTES} bytes). Read explicit `start_line`/`end_line` windows instead.",
                    req_path
                )));
            }
            let max_symbols = args
                .get("max_symbols")
                .and_then(Value::as_u64)
                .unwrap_or(OUTLINE_MAX_ENTRIES as u64)
                .clamp(1, OUTLINE_MAX_ENTRIES as u64) as usize;
            let raw = tokio::fs::read_to_string(&path).await?;
            let report = extract_outline(&path, &raw, max_symbols);
            let rendered = format_outline_report(req_path, &report);
            let mut out = ToolOutput::ok(rendered);
            out.metadata = Some(json!({
                "mode": "outline",
                "total_lines": report.total_lines,
                "symbols": report.entries.len(),
                "truncated": report.truncated,
            }));
            return Ok(out);
        }

        let ranges = parse_read_ranges(&args)?;
        let max_requested_end = ranges.iter().map(|r| r.end).max().unwrap_or(1);

        // Stream line-by-line with strict per-line and total RAM bounds so giant or minified files never spike RAM.
        let file = tokio::fs::File::open(&path).await?;
        let mut reader = BufReader::with_capacity(16 * 1024, file);
        let mut raw_line = Vec::with_capacity(512);
        let mut line_no = 0usize;
        let mut captured_per_range: Vec<Vec<String>> = vec![Vec::new(); ranges.len()];
        let mut out_bytes = 0usize;
        let byte_budget = ctx
            .max_output_bytes
            .saturating_mul(16)
            .clamp(64 * 1024, 512 * 1024);
        let stop_early = file_bytes > DEFAULT_READ_FILE_MAX_BYTES;
        let mut active_range_idx = 0usize;

        loop {
            raw_line.clear();
            let mut truncated_line = false;
            let n = {
                use tokio::io::AsyncReadExt;
                let mut limited = (&mut reader).take(MAX_SINGLE_READ_LINE_BYTES as u64);
                limited.read_until(b'\n', &mut raw_line).await?
            };
            if n == 0 {
                break;
            }
            if !raw_line.ends_with(b"\n") && n == MAX_SINGLE_READ_LINE_BYTES {
                truncated_line = true;
                let mut discard = [0u8; 4096];
                loop {
                    let buf = reader.fill_buf().await?;
                    if buf.is_empty() {
                        break;
                    }
                    if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                        reader.consume(pos + 1);
                        break;
                    }
                    let len = buf.len().min(discard.len());
                    discard[..len].copy_from_slice(&buf[..len]);
                    reader.consume(len);
                }
            }
            line_no += 1;

            while active_range_idx < ranges.len() && line_no > ranges[active_range_idx].end {
                active_range_idx += 1;
            }
            if let Some(cur) = ranges.get(active_range_idx)
                && line_no >= cur.start
                && line_no <= cur.end
                && out_bytes < byte_budget
            {
                let line_str = String::from_utf8_lossy(&raw_line);
                let clean = line_str.trim_end_matches(['\r', '\n']);
                let h = line_hash(clean);
                let suffix = match truncated_line {
                    true => " ...[line truncated]",
                    false => "",
                };
                let formatted = format!("{line_no}:{h}| {clean}{suffix}\n");
                out_bytes += formatted.len();
                captured_per_range[active_range_idx].push(formatted);
            }
            if stop_early && line_no >= max_requested_end {
                break;
            }
        }

        let total_captured: usize = captured_per_range.iter().map(Vec::len).sum();
        if total_captured == 0 && line_no > 0 {
            let first_start = ranges.first().map(|r| r.start).unwrap_or(1);
            return Ok(ToolOutput::ok(format!(
                "File '{}' has {line_no} lines (requested start_line={first_start}).",
                req_path
            )));
        }

        let total_label = match stop_early {
            true => format!("{line_no}+ ({} KB file, streamed)", file_bytes / 1024),
            false => format!("{line_no}"),
        };

        if ranges.len() == 1 {
            let r = ranges[0];
            let rows = &captured_per_range[0];
            let end_shown = r
                .start
                .saturating_add(rows.len().saturating_sub(1))
                .min(line_no);
            let mut out = format!(
                "File: {req_path} (lines {}..={end_shown} of {total_label})\n",
                r.start
            );
            for row in rows {
                out.push_str(row);
            }
            return Ok(ToolOutput::ok(out));
        }

        let mut out = format!(
            "File: {req_path} ({} slices, total {total_label} lines)\n",
            ranges.len()
        );
        for (r, rows) in ranges.iter().zip(captured_per_range.iter()) {
            if rows.is_empty() {
                out.push_str(&format!(
                    "\n--- lines {}..={} (past EOF; file has {line_no} lines) ---\n",
                    r.start, r.end
                ));
                continue;
            }
            let end_shown = r
                .start
                .saturating_add(rows.len().saturating_sub(1))
                .min(line_no);
            out.push_str(&format!("\n--- lines {}..={end_shown} ---\n", r.start));
            for row in rows {
                out.push_str(row);
            }
        }
        Ok(ToolOutput::ok(out))
    }
}

// ─── hash_edit (Multi-Hunk Atomic Hash-Anchored Edits + Inline Self-Healing) ──

#[derive(Debug, Clone)]
struct HashEditHunk {
    start_line: usize,
    start_hash: String,
    end_line: usize,
    end_hash: String,
    replacement: String,
}

fn parse_hash_edit_hunks(args: &Value) -> Result<Vec<HashEditHunk>> {
    if let Some(arr) = args.get("edits").and_then(Value::as_array) {
        if arr.is_empty() {
            return Err(ZenkoError::Tool(
                "`edits` must contain at least one hash-anchored edit hunk".into(),
            ));
        }
        if arr.len() > HASH_EDIT_MAX_HUNKS {
            return Err(ZenkoError::Tool(format!(
                "`edits` exceeds maximum {HASH_EDIT_MAX_HUNKS} hunks per call"
            )));
        }
        let mut hunks = Vec::with_capacity(arr.len());
        for (idx, item) in arr.iter().enumerate() {
            hunks.push(parse_single_hash_hunk(item, Some(idx))?);
        }
        return Ok(hunks);
    }
    Ok(vec![parse_single_hash_hunk(args, None)?])
}

fn parse_single_hash_hunk(obj: &Value, idx: Option<usize>) -> Result<HashEditHunk> {
    let prefix = match idx {
        Some(i) => format!("edits[{i}]."),
        None => String::new(),
    };
    let start_line = obj
        .get("start_line")
        .and_then(Value::as_u64)
        .ok_or_else(|| ZenkoError::Tool(format!("missing `{prefix}start_line`")))?
        as usize;
    let end_line = obj
        .get("end_line")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(start_line);
    let start_hash = obj
        .get("start_hash")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ZenkoError::Tool(format!("missing `{prefix}start_hash`")))?
        .to_ascii_lowercase();
    let end_hash = obj
        .get("end_hash")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| match end_line == start_line {
            true => start_hash.clone(),
            false => String::new(),
        });
    if end_hash.is_empty() {
        return Err(ZenkoError::Tool(format!("missing `{prefix}end_hash`")));
    }
    let replacement = obj
        .get("replacement")
        .and_then(Value::as_str)
        .ok_or_else(|| ZenkoError::Tool(format!("missing `{prefix}replacement`")))?
        .to_string();
    Ok(HashEditHunk {
        start_line,
        start_hash,
        end_line,
        end_hash,
        replacement,
    })
}

pub struct HashEditTool;

#[async_trait]
impl Tool for HashEditTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::CoreFs
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        file_lock_key(args)
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_HASH_EDIT.into(),
            description: "Atomically replace one or multiple non-overlapping line spans verified by 3-char hashes from `read_file`. Supports single hunk (`start_line`/`start_hash`/`end_line`/`end_hash`/`replacement`) or multi-hunk `edits[]` applied bottom-up. On hash mismatch, returns fresh hash-anchored lines inline so you can retry immediately without re-reading.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace or `hub:` file path" },
                    "start_line": { "type": "integer", "description": "1-based start line (single-hunk mode)" },
                    "start_hash": { "type": "string", "description": "3-char hash of start_line (single-hunk mode)" },
                    "end_line": { "type": "integer", "description": "1-based inclusive end line (defaults to start_line)" },
                    "end_hash": { "type": "string", "description": "3-char hash of end_line (defaults to start_hash when single line)" },
                    "replacement": { "type": "string", "description": "Replacement text for the anchored span (empty string deletes span)" },
                    "edits": {
                        "type": "array",
                        "description": "Multi-hunk atomic edits applied bottom-up in one write (non-overlapping line ranges)",
                        "items": {
                            "type": "object",
                            "properties": {
                                "start_line": { "type": "integer" },
                                "start_hash": { "type": "string" },
                                "end_line": { "type": "integer" },
                                "end_hash": { "type": "string" },
                                "replacement": { "type": "string" }
                            },
                            "required": ["start_line", "start_hash", "replacement"]
                        }
                    }
                },
                "required": ["path"]
            }),
            risk: RiskLevel::Moderate,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let req_path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `path`".into()))?;
        let mut hunks = parse_hash_edit_hunks(&args)?;

        let path = resolve_workspace_or_hub_path(&ctx.workspace_dir, &ctx.hub_dir, req_path, true)?;
        let file_len = tokio::fs::metadata(&path).await?.len();
        if file_len > MAX_EDIT_FILE_BYTES {
            return Ok(ToolOutput::err(format!(
                "Refused to hash_edit '{}': file size ({file_len} bytes) exceeds edit safety limit ({MAX_EDIT_FILE_BYTES} bytes).",
                req_path
            )));
        }
        let old = tokio::fs::read_to_string(&path).await?;
        let eol = detect_line_ending(&old);
        let mut lines: Vec<String> = old.lines().map(String::from).collect();

        // Step 1: Bounds & overlap validation across all hunks before mutating anything.
        hunks.sort_by_key(|h| (h.start_line, h.end_line));
        for (i, h) in hunks.iter().enumerate() {
            if h.start_line == 0 || h.end_line < h.start_line || h.end_line > lines.len() {
                let tail_window = format_hash_recovery_window(&lines, lines.len());
                return Ok(ToolOutput::err(format!(
                    "Invalid range {}..={} in `{req_path}` (file has {} lines).\nRecent tail lines:\n{tail_window}",
                    h.start_line,
                    h.end_line,
                    lines.len()
                )));
            }
            if i > 0 && h.start_line <= hunks[i - 1].end_line {
                return Ok(ToolOutput::err(format!(
                    "Overlapping hash_edit hunks: {}..={} overlaps {}..={}. Merge overlapping edits into a single span.",
                    hunks[i - 1].start_line,
                    hunks[i - 1].end_line,
                    h.start_line,
                    h.end_line
                )));
            }
        }

        // Step 2: Verify every anchor upfront; on any mismatch, return fresh hash-anchored lines inline.
        let mut mismatches = Vec::new();
        for h in &hunks {
            let actual_start = line_hash(&lines[h.start_line - 1]);
            let actual_end = line_hash(&lines[h.end_line - 1]);
            if actual_start != h.start_hash || actual_end != h.end_hash {
                let mut detail = format!(
                    "Hash mismatch on span {}..={} in `{req_path}`: expected start {}:{}, end {}:{} — actual start {}:{actual_start}, end {}:{actual_end}.",
                    h.start_line,
                    h.end_line,
                    h.start_line,
                    h.start_hash,
                    h.end_line,
                    h.end_hash,
                    h.start_line,
                    h.end_line
                );
                detail.push_str("\nFresh lines around start:\n");
                detail.push_str(&format_hash_recovery_window(&lines, h.start_line));
                if h.end_line.saturating_sub(h.start_line) > HASH_EDIT_RECOVERY_RADIUS {
                    detail.push_str("Fresh lines around end:\n");
                    detail.push_str(&format_hash_recovery_window(&lines, h.end_line));
                }
                mismatches.push(detail);
            }
        }
        if !mismatches.is_empty() {
            return Ok(ToolOutput::err(format!(
                "{}\nRetry `hash_edit` directly using the fresh `line:hash|` anchors above (no extra `read_file` needed).",
                mismatches.join("\n")
            )));
        }

        // Step 3: Apply splices bottom-up (descending `start_line`) so earlier line indices never drift.
        let applied_spans: Vec<String> = hunks
            .iter()
            .map(|h| format!("{}..={}", h.start_line, h.end_line))
            .collect();
        for h in hunks.into_iter().rev() {
            let replacement_lines: Vec<String> = match h.replacement.is_empty() {
                true => Vec::new(),
                false => h.replacement.lines().map(String::from).collect(),
            };
            lines.splice((h.start_line - 1)..h.end_line, replacement_lines);
        }

        let mut new = lines.join(eol);
        if old.ends_with('\n') && !new.ends_with('\n') {
            new.push_str(eol);
        }

        atomic_write(&path, &new).await?;
        let diff = unified_diff(req_path, &old, &new);
        Ok(ToolOutput::ok_with_diff(
            format!(
                "Applied {} hash_edit hunk(s) to {req_path} (lines {}).",
                applied_spans.len(),
                applied_spans.join(", ")
            ),
            diff,
        ))
    }
}

// ─── apply_patch (Ambiguity-Guarded Multi-Edit Search-and-Replace) ────────────

#[derive(Debug, Clone)]
struct PatchEditItem {
    old_text: String,
    new_text: String,
    replace_all: bool,
    expected_occurrences: Option<usize>,
}

fn parse_patch_edits(args: &Value) -> Result<Vec<PatchEditItem>> {
    if let Some(arr) = args.get("edits").and_then(Value::as_array) {
        if arr.is_empty() {
            return Err(ZenkoError::Tool(
                "`edits` must contain at least one `{old_text, new_text}` entry".into(),
            ));
        }
        if arr.len() > APPLY_PATCH_MAX_EDITS {
            return Err(ZenkoError::Tool(format!(
                "`edits` exceeds max {APPLY_PATCH_MAX_EDITS} operations per call"
            )));
        }
        let mut out = Vec::with_capacity(arr.len());
        for (idx, item) in arr.iter().enumerate() {
            out.push(parse_single_patch_edit(item, Some(idx))?);
        }
        return Ok(out);
    }
    Ok(vec![parse_single_patch_edit(args, None)?])
}

fn parse_single_patch_edit(obj: &Value, idx: Option<usize>) -> Result<PatchEditItem> {
    let prefix = match idx {
        Some(i) => format!("edits[{i}]."),
        None => String::new(),
    };
    let old_text = obj
        .get("old_text")
        .and_then(Value::as_str)
        .ok_or_else(|| ZenkoError::Tool(format!("missing `{prefix}old_text`")))?
        .to_string();
    let new_text = obj
        .get("new_text")
        .and_then(Value::as_str)
        .ok_or_else(|| ZenkoError::Tool(format!("missing `{prefix}new_text`")))?
        .to_string();
    let replace_all = obj
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let expected_occurrences = obj
        .get("expected_occurrences")
        .and_then(Value::as_u64)
        .map(|n| n as usize);
    Ok(PatchEditItem {
        old_text,
        new_text,
        replace_all,
        expected_occurrences,
    })
}

fn byte_offset_to_line_no(text: &str, byte_idx: usize) -> usize {
    text[..byte_idx].bytes().filter(|&b| b == b'\n').count() + 1
}

fn format_occurrence_locations(text: &str, line_numbers: &[usize]) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut rows = Vec::new();
    for &ln in line_numbers.iter().take(APPLY_PATCH_AMBIGUITY_MAX_LOCS) {
        let preview = lines.get(ln.saturating_sub(1)).copied().unwrap_or("");
        let h = line_hash(preview);
        rows.push(format!("  {ln}:{h}| {}", preview.trim()));
    }
    if line_numbers.len() > APPLY_PATCH_AMBIGUITY_MAX_LOCS {
        rows.push(format!(
            "  ... (+{} more occurrences)",
            line_numbers.len() - APPLY_PATCH_AMBIGUITY_MAX_LOCS
        ));
    }
    rows.join("\n")
}

fn apply_single_patch_to_buffer(
    req_path: &str,
    buf: &str,
    eol: &str,
    edit: &PatchEditItem,
    edit_label: &str,
) -> std::result::Result<(String, usize), String> {
    if edit.old_text.is_empty() {
        return Err(format!(
            "{edit_label}: `old_text` must be non-empty to prevent accidental prepending."
        ));
    }

    // 1. Try exact non-overlapping byte occurrences first.
    let exact_offsets: Vec<usize> = buf
        .match_indices(&edit.old_text)
        .map(|(idx, _)| idx)
        .collect();
    if !exact_offsets.is_empty() {
        let count = exact_offsets.len();
        if let Some(exp) = edit.expected_occurrences
            && count != exp
        {
            let lines: Vec<usize> = exact_offsets
                .iter()
                .map(|&off| byte_offset_to_line_no(buf, off))
                .collect();
            return Err(format!(
                "{edit_label} in `{req_path}`: expected {exp} occurrence(s), found {count}:\n{}",
                format_occurrence_locations(buf, &lines)
            ));
        }
        if count > 1 && !edit.replace_all && edit.expected_occurrences.is_none() {
            let lines: Vec<usize> = exact_offsets
                .iter()
                .map(|&off| byte_offset_to_line_no(buf, off))
                .collect();
            return Err(format!(
                "Ambiguous {edit_label} in `{req_path}`: `old_text` matched {count} places:\n{}\nInclude more surrounding context, pass `replace_all: true`, or target exact lines with `hash_edit`.",
                format_occurrence_locations(buf, &lines)
            ));
        }

        let mut out = String::with_capacity(buf.len() + edit.new_text.len() * count);
        let mut cursor = 0usize;
        for &off in &exact_offsets {
            out.push_str(&buf[cursor..off]);
            out.push_str(&edit.new_text);
            cursor = off + edit.old_text.len();
        }
        out.push_str(&buf[cursor..]);
        return Ok((out, count));
    }

    // 2. Fall back to Codex-inspired multi-stage tolerant line sequence seeking (all non-overlapping matches).
    let file_lines: Vec<&str> = buf.lines().collect();
    let pattern_lines: Vec<&str> = edit.old_text.lines().collect();
    let matches = seek_sequence_all(&file_lines, &pattern_lines);
    if matches.is_empty() {
        return Err(format!(
            "Could not locate target block for {edit_label} in `{req_path}` even with tolerant seeking."
        ));
    }
    let count = matches.len();
    let one_based: Vec<usize> = matches.iter().map(|&idx| idx + 1).collect();
    if let Some(exp) = edit.expected_occurrences
        && count != exp
    {
        return Err(format!(
            "{edit_label} in `{req_path}`: expected {exp} occurrence(s), found {count} (tolerant match):\n{}",
            format_occurrence_locations(buf, &one_based)
        ));
    }
    if count > 1 && !edit.replace_all && edit.expected_occurrences.is_none() {
        return Err(format!(
            "Ambiguous {edit_label} in `{req_path}`: `old_text` matched {count} places (tolerant match):\n{}\nInclude more surrounding context, pass `replace_all: true`, or target exact lines with `hash_edit`.",
            format_occurrence_locations(buf, &one_based)
        ));
    }

    let mut rebuilt: Vec<String> = file_lines.iter().map(|s| (*s).to_string()).collect();
    let rep: Vec<String> = match edit.new_text.is_empty() {
        true => Vec::new(),
        false => edit.new_text.lines().map(String::from).collect(),
    };
    for &start in matches.iter().rev() {
        rebuilt.splice(start..(start + pattern_lines.len()), rep.clone());
    }
    let mut joined = rebuilt.join(eol);
    if buf.ends_with('\n') && !joined.ends_with('\n') {
        joined.push_str(eol);
    }
    Ok((joined, count))
}

pub struct ApplyPatchTool;

#[async_trait]
impl Tool for ApplyPatchTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::CoreFs
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        file_lock_key(args)
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_APPLY_PATCH.into(),
            description: "Apply one or multiple atomic search-and-replace edits to a workspace file with tolerant seeking (exact -> right-trimmed -> trimmed) and strict ambiguity protection (refuses multi-match silent corruption unless `replace_all` or `expected_occurrences` is set).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace or `hub:` file path" },
                    "old_text": { "type": "string", "description": "Existing block to locate (single-edit mode)" },
                    "new_text": { "type": "string", "description": "Replacement block (single-edit mode)" },
                    "replace_all": { "type": "boolean", "description": "Replace every non-overlapping match instead of requiring uniqueness (default false)" },
                    "expected_occurrences": { "type": "integer", "description": "Optional exact expected match count guard" },
                    "edits": {
                        "type": "array",
                        "description": "Atomic sequential list of `{old_text, new_text, replace_all?, expected_occurrences?}` edits applied before one atomic write",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old_text": { "type": "string" },
                                "new_text": { "type": "string" },
                                "replace_all": { "type": "boolean" },
                                "expected_occurrences": { "type": "integer" }
                            },
                            "required": ["old_text", "new_text"]
                        }
                    }
                },
                "required": ["path"]
            }),
            risk: RiskLevel::Moderate,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let req_path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `path`".into()))?;
        let edits = parse_patch_edits(&args)?;

        let path = resolve_workspace_or_hub_path(&ctx.workspace_dir, &ctx.hub_dir, req_path, true)?;
        let file_len = tokio::fs::metadata(&path).await?.len();
        if file_len > MAX_EDIT_FILE_BYTES {
            return Ok(ToolOutput::err(format!(
                "Refused to apply_patch '{}': file size ({file_len} bytes) exceeds edit safety limit ({MAX_EDIT_FILE_BYTES} bytes).",
                req_path
            )));
        }
        let old = tokio::fs::read_to_string(&path).await?;
        let eol = detect_line_ending(&old);

        let mut working = old.clone();
        let mut total_replacements = 0usize;
        for (idx, edit) in edits.iter().enumerate() {
            let label = match edits.len() {
                1 => "patch".to_string(),
                _ => format!("edits[{idx}]"),
            };
            match apply_single_patch_to_buffer(req_path, &working, eol, edit, &label) {
                Ok((next, count)) => {
                    working = next;
                    total_replacements += count;
                }
                Err(msg) => return Ok(ToolOutput::err(msg)),
            }
        }

        atomic_write(&path, &working).await?;
        let diff = unified_diff(req_path, &old, &working);
        Ok(ToolOutput::ok_with_diff(
            format!(
                "Patched {req_path} successfully ({total_replacements} replacement(s) across {} edit(s)).",
                edits.len()
            ),
            diff,
        ))
    }
}

/// Codex-inspired tolerant line sequence seeker returning the first match index.
pub fn seek_sequence(lines: &[&str], pattern: &[&str]) -> Option<usize> {
    seek_sequence_all(lines, pattern).into_iter().next()
}

/// Finds all non-overlapping tolerant line sequence matches at the tightest matching stage
/// (`exact` -> `trim_end` -> `trim`).
pub fn seek_sequence_all(lines: &[&str], pattern: &[&str]) -> Vec<usize> {
    if pattern.is_empty() || pattern.len() > lines.len() {
        return Vec::new();
    }
    let stages: &[fn(&str, &str) -> bool] = &[
        |a, b| a == b,
        |a, b| a.trim_end() == b.trim_end(),
        |a, b| a.trim() == b.trim(),
    ];
    for cmp in stages {
        let mut hits = Vec::new();
        let mut i = 0usize;
        let last = lines.len() - pattern.len();
        while i <= last {
            if lines[i..i + pattern.len()]
                .iter()
                .zip(pattern)
                .all(|(a, b)| cmp(a, b))
            {
                hits.push(i);
                i += pattern.len();
            } else {
                i += 1;
            }
        }
        if !hits.is_empty() {
            return hits;
        }
    }
    Vec::new()
}

// ─── write_file & list_dir ────────────────────────────────────────────────────

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::CoreFs
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        file_lock_key(args)
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_WRITE_FILE.into(),
            description: "Create or overwrite a file inside the workspace atomically.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
            risk: RiskLevel::Moderate,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let req_path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `path`".into()))?;
        let content = args
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `content`".into()))?;

        let path = resolve_workspace_or_hub_path(&ctx.workspace_dir, &ctx.hub_dir, req_path, false)?;
        let old = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        atomic_write(&path, content).await?;
        let diff = unified_diff(req_path, &old, content);
        Ok(ToolOutput::ok_with_diff(
            format!("Wrote {} bytes to {req_path}.", content.len()),
            diff,
        ))
    }
}

pub struct ListDirTool;

#[async_trait]
impl Tool for ListDirTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::CoreFs
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_LIST_DIR.into(),
            description: "List files and directories inside a workspace path.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path (default '.')" }
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
        let req_path = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let path = resolve_readable_path(
            &ctx.workspace_dir,
            &ctx.hub_dir,
            &ctx.data_dir,
            req_path,
            true,
        )?;
        let mut entries = tokio::fs::read_dir(&path).await?;
        let mut rows = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let meta = entry.metadata().await?;
            let kind = match meta.is_dir() {
                true => "dir ",
                false => "file",
            };
            let name = entry.file_name().to_string_lossy().to_string();
            rows.push(format!("{kind}  {:>8}  {name}", meta.len()));
        }
        rows.sort();
        Ok(ToolOutput::ok(rows.join("\n")))
    }
}

async fn atomic_write(path: &std::path::Path, content: &str) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    tokio::fs::create_dir_all(parent).await?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let tmp = parent.join(format!(".{name}.zenko_tmp_{}", zenko_core::new_id()));
    if let Err(err) = tokio::fs::write(&tmp, content).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(err.into());
    }
    if let Err(err) = tokio::fs::rename(&tmp, path).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(err.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx(dir: &std::path::Path) -> ToolContext {
        ToolContext {
            session_id: "s1".into(),
            workspace_dir: dir.to_path_buf(),
            data_dir: dir.to_path_buf(),
            hub_dir: dir.to_path_buf(),
            shell_timeout_secs: 10,
            auto_background_after_secs: 0,
            max_output_bytes: 4096,
            headless_no_wait: false,
            demote_bg: std::sync::Arc::new(tokio::sync::Notify::new()),
        }
    }

    #[tokio::test]
    async fn hash_edit_multi_hunk_and_self_healing_recovery() {
        let dir = std::env::temp_dir().join(format!("zenko_fs_v2_{}", zenko_core::new_id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("demo.rs");
        std::fs::write(
            &file,
            "fn one() {}\nlet a = 1;\nfn two() {}\nlet b = 2;\nfn three() {}\n",
        )
        .expect("write");

        let ctx = test_ctx(&dir);
        let cancel = CancellationToken::new();

        // 1. Stale hash returns inline fresh hash-anchored lines so no extra read_file is needed.
        let stale = HashEditTool
            .execute(
                &ctx,
                json!({
                    "path": "demo.rs",
                    "start_line": 2,
                    "start_hash": "000",
                    "end_line": 2,
                    "end_hash": "000",
                    "replacement": "let a = 10;"
                }),
                cancel.clone(),
            )
            .await
            .expect("stale");
        assert!(stale.is_error);
        let expected_h2 = line_hash("let a = 1;");
        assert!(
            stale.content.contains(&format!("2:{expected_h2}| let a = 1;")),
            "stale recovery missing fresh anchor: {}",
            stale.content
        );

        // 2. Multi-hunk atomic bottom-up edit succeeds without line drift.
        let h4 = line_hash("let b = 2;");
        let multi = HashEditTool
            .execute(
                &ctx,
                json!({
                    "path": "demo.rs",
                    "edits": [
                        { "start_line": 2, "start_hash": expected_h2, "replacement": "let a = 10;\nlet a2 = 11;" },
                        { "start_line": 4, "start_hash": h4, "replacement": "let b = 20;" }
                    ]
                }),
                cancel.clone(),
            )
            .await
            .expect("multi");
        assert!(!multi.is_error, "multi err: {}", multi.content);
        let after = std::fs::read_to_string(&file).expect("read");
        assert!(after.contains("let a = 10;\nlet a2 = 11;\nfn two() {}\nlet b = 20;\n"));

        // 3. Read file supports multi-range slices & outline mode.
        let outline = ReadFileTool
            .execute(
                &ctx,
                json!({ "path": "demo.rs", "mode": "outline" }),
                cancel.clone(),
            )
            .await
            .expect("outline");
        assert!(!outline.is_error);
        assert!(outline.content.contains("[fn] fn one()"));
        assert!(outline.content.contains("[fn] fn three()"));

        let ranges = ReadFileTool
            .execute(
                &ctx,
                json!({
                    "path": "demo.rs",
                    "ranges": [
                        { "start_line": 1, "end_line": 1 },
                        { "start_line": 5, "end_line": 6 }
                    ]
                }),
                cancel,
            )
            .await
            .expect("ranges");
        assert!(!ranges.is_error);
        assert!(ranges.content.contains("1:"));
        assert!(ranges.content.contains("fn three()"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn apply_patch_guards_ambiguity_and_supports_multi_edit() {
        let dir = std::env::temp_dir().join(format!("zenko_patch_v2_{}", zenko_core::new_id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("dup.txt");
        std::fs::write(&file, "foo\nbar\nfoo\nbaz\n").expect("write");

        let ctx = test_ctx(&dir);
        let cancel = CancellationToken::new();

        // Ambiguous match must be refused with line numbers instead of silently mutating match #1.
        let amb = ApplyPatchTool
            .execute(
                &ctx,
                json!({ "path": "dup.txt", "old_text": "foo", "new_text": "qux" }),
                cancel.clone(),
            )
            .await
            .expect("amb");
        assert!(amb.is_error);
        assert!(amb.content.contains("Ambiguous"), "amb={}", amb.content);
        assert!(amb.content.contains("1:"));
        assert!(amb.content.contains("3:"));

        // Explicit replace_all + multi-edit succeeds transactionally.
        let ok = ApplyPatchTool
            .execute(
                &ctx,
                json!({
                    "path": "dup.txt",
                    "edits": [
                        { "old_text": "foo", "new_text": "qux", "replace_all": true, "expected_occurrences": 2 },
                        { "old_text": "baz", "new_text": "BAZ" }
                    ]
                }),
                cancel,
            )
            .await
            .expect("ok");
        assert!(!ok.is_error, "err={}", ok.content);
        assert_eq!(std::fs::read_to_string(&file).expect("read"), "qux\nbar\nqux\nBAZ\n");

        let _ = std::fs::remove_dir_all(dir);
    }
}
