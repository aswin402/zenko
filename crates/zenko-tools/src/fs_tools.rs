use crate::codex_patch::{CodexPatchFileOp, CodexUpdateHunk, parse_auto_patch};
use crate::jail::{
    atomic_write_jailed, open_jailed_read_file, read_jailed_string_bounded, remove_jailed_file,
    resolve_readable_path, resolve_workspace_or_hub_path, select_jail_root,
};
use crate::outline::{extract_outline, find_outline_symbol_spans, format_outline_report};
use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use similar::TextDiff;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;
use zenko_core::{
    APPLY_PATCH_AMBIGUITY_MAX_LOCS, APPLY_PATCH_MAX_EDITS, APPLY_PATCH_MAX_FILES,
    DEFAULT_READ_FILE_LINE_WINDOW, DEFAULT_READ_FILE_MAX_BYTES, FILE_FRESHNESS_PREVIEW_LINES,
    FS_MAX_EDIT_FILE_BYTES, FS_MAX_SINGLE_READ_LINE_BYTES, FileFreshnessCheck, HASH_EDIT_MAX_HUNKS,
    HASH_EDIT_OPS, HASH_EDIT_RECOVERY_RADIUS, HashEditOp, OUTLINE_MAX_ENTRIES,
    OUTLINE_MAX_FILE_BYTES, PATCH_FILE_ACTIONS, PatchFileAction,
    READ_FILE_CONTEXT_DIET_HINT_MIN_LINES, READ_FILE_MAX_RANGES, READ_FILE_MAX_SYMBOL_QUERIES,
    READ_FILE_SYMBOL_NOT_FOUND_PREVIEW, ResourceLockKey, Result, RiskLevel, TOOL_APPLY_PATCH,
    TOOL_HASH_EDIT, TOOL_LIST_DIR, TOOL_READ_FILE, TOOL_WRITE_FILE, Tool, ToolConcurrency,
    ToolContext, ToolOutput, ToolSet, ToolSpec, ZenkoError,
};

async fn open_jailed_async(ctx: &ToolContext, abs_path: &Path) -> Result<tokio::fs::File> {
    let root = select_jail_root(
        abs_path,
        &ctx.workspace_dir,
        &ctx.hub_dir,
        Some(&ctx.data_dir),
    )
    .ok_or_else(|| {
        ZenkoError::PolicyDenied(format!(
            "path '{}' is outside all confined roots",
            abs_path.display()
        ))
    })?;
    let target = abs_path.to_path_buf();
    let std_file = tokio::task::spawn_blocking(move || open_jailed_read_file(&root, &target))
        .await
        .map_err(|e| ZenkoError::Tool(format!("confined open join error: {e}")))??;
    Ok(tokio::fs::File::from_std(std_file))
}

async fn read_jailed_string_async(
    ctx: &ToolContext,
    abs_path: &Path,
    max_bytes: u64,
    label: &str,
) -> Result<String> {
    let root = select_jail_root(
        abs_path,
        &ctx.workspace_dir,
        &ctx.hub_dir,
        Some(&ctx.data_dir),
    )
    .ok_or_else(|| {
        ZenkoError::PolicyDenied(format!(
            "path '{}' is outside all confined roots",
            abs_path.display()
        ))
    })?;
    let target = abs_path.to_path_buf();
    let lbl = label.to_string();
    tokio::task::spawn_blocking(move || read_jailed_string_bounded(&root, &target, max_bytes, &lbl))
        .await
        .map_err(|e| ZenkoError::Tool(format!("confined read join error: {e}")))?
}

async fn atomic_write_in_ctx(ctx: &ToolContext, abs_path: &Path, content: &str) -> Result<()> {
    let root = select_jail_root(abs_path, &ctx.workspace_dir, &ctx.hub_dir, None).ok_or_else(
        || {
            ZenkoError::PolicyDenied(format!(
                "write path '{}' is outside writable workspace and hub roots",
                abs_path.display()
            ))
        },
    )?;
    let target = abs_path.to_path_buf();
    let bytes = content.as_bytes().to_vec();
    tokio::task::spawn_blocking(move || atomic_write_jailed(&root, &target, &bytes))
        .await
        .map_err(|e| ZenkoError::Tool(format!("confined atomic write join error: {e}")))?
}

async fn remove_jailed_in_ctx(ctx: &ToolContext, abs_path: &Path) -> Result<()> {
    let root = select_jail_root(abs_path, &ctx.workspace_dir, &ctx.hub_dir, None).ok_or_else(
        || {
            ZenkoError::PolicyDenied(format!(
                "delete path '{}' is outside writable workspace and hub roots",
                abs_path.display()
            ))
        },
    )?;
    let target = abs_path.to_path_buf();
    tokio::task::spawn_blocking(move || remove_jailed_file(&root, &target))
        .await
        .map_err(|e| ZenkoError::Tool(format!("confined delete join error: {e}")))?
}

/// Computes full-content SHA-256 hex digest used by `FileStateBridge` stale-read detection.
pub(crate) fn content_sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{b:02x}");
    }
    hex
}

fn canonical_state_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub(crate) fn record_fresh_file_state(ctx: &ToolContext, path: &Path, content: &str) {
    if let Some(ref bridge) = ctx.file_state {
        let canon = canonical_state_path(path);
        let sha = content_sha256_hex(content.as_bytes());
        bridge.record_file_state(&ctx.session_id, &canon, sha, content.len() as u64);
    }
}

pub(crate) fn forget_fresh_file_state(ctx: &ToolContext, path: &Path) {
    if let Some(ref bridge) = ctx.file_state {
        let canon = canonical_state_path(path);
        bridge.forget_file_state(&ctx.session_id, &canon);
    }
}

fn format_fresh_head_preview(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return "(file is empty)".into();
    }
    let mut out = String::new();
    for (idx, line) in lines.iter().take(FILE_FRESHNESS_PREVIEW_LINES).enumerate() {
        let ln = idx + 1;
        let h = line_hash(line);
        out.push_str(&format!("{ln}:{h}| {line}\n"));
    }
    if lines.len() > FILE_FRESHNESS_PREVIEW_LINES {
        out.push_str(&format!(
            "... (+{} more lines; inspect with `read_file`)\n",
            lines.len() - FILE_FRESHNESS_PREVIEW_LINES
        ));
    }
    out
}

pub(crate) fn check_existing_file_freshness(
    ctx: &ToolContext,
    req_path: &str,
    abs_path: &Path,
    current_content: &str,
    allow_unread_existing: bool,
) -> std::result::Result<(), String> {
    let Some(ref bridge) = ctx.file_state else {
        return Ok(());
    };
    let canon = canonical_state_path(abs_path);
    let current_sha = content_sha256_hex(current_content.as_bytes());
    match bridge.check_file_freshness(&ctx.session_id, &canon, &current_sha) {
        FileFreshnessCheck::Fresh => Ok(()),
        FileFreshnessCheck::UnreadExisting => match allow_unread_existing {
            true => Ok(()),
            false => {
                let total_lines = current_content.lines().count();
                let preview = format_fresh_head_preview(current_content);
                Err(format!(
                    "Refused to overwrite existing unread file `{req_path}` (stale-read guard). Read it with `read_file` first, edit surgically with `hash_edit`/`apply_patch`, or pass `overwrite: true` to replace it intentionally.\nCurrent file preview ({total_lines} lines):\n{preview}"
                ))
            }
        },
        FileFreshnessCheck::ExternallyModified {
            recorded_sha256,
            current_sha256,
        } => {
            let total_lines = current_content.lines().count();
            let preview = format_fresh_head_preview(current_content);
            let old_short = &recorded_sha256[..recorded_sha256.len().min(10)];
            let new_short = &current_sha256[..current_sha256.len().min(10)];
            Err(format!(
                "File `{req_path}` changed externally on disk since you last read it (sha256 `{old_short}..` -> `{new_short}..`). Re-read with `read_file` before mutating so external edits are not clobbered.\nCurrent file preview ({total_lines} lines):\n{preview}"
            ))
        }
    }
}

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

fn parse_requested_symbols(args: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(single) = args.get("symbol").and_then(Value::as_str) {
        let s = single.trim();
        if !s.is_empty() {
            out.push(s.to_string());
        }
    }
    if let Some(arr) = args.get("symbols").and_then(Value::as_array) {
        for item in arr {
            if out.len() >= READ_FILE_MAX_SYMBOL_QUERIES {
                break;
            }
            if let Some(s) = item.as_str().map(str::trim).filter(|s| !s.is_empty())
                && !out.iter().any(|existing| existing == s)
            {
                out.push(s.to_string());
            }
        }
    }
    out
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
            description: "Read a workspace, `hub:`, or `artifact:` file through kernel-confined `O_NOFOLLOW` handles. Returns hash-anchored lines (`line:hash| text`) for exact `hash_edit` mutations. Supports single window (`start_line`/`end_line`), multiple non-contiguous `ranges`, one-shot targeted `symbol`/`symbols` spans, or `mode=\"outline\"` for a zero-dependency structural symbol map.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative workspace, `hub:`, or `artifact:` path to file" },
                    "mode": { "type": "string", "enum": ["lines", "outline"], "description": "`lines` (default) for hash-anchored line content, or `outline` for a compact structural symbol map with line spans & hashes" },
                    "symbol": { "type": "string", "description": "One-shot targeted structural symbol name (function, struct, class, section) returning ONLY that symbol's hash-anchored lines" },
                    "symbols": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional list of up to 8 targeted symbol names to read in a single call"
                    },
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

        let mode = args
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("lines")
            .trim()
            .to_ascii_lowercase();

        if mode == "outline" {
            let max_symbols = args
                .get("max_symbols")
                .and_then(Value::as_u64)
                .unwrap_or(OUTLINE_MAX_ENTRIES as u64)
                .clamp(1, OUTLINE_MAX_ENTRIES as u64) as usize;
            let raw = read_jailed_string_async(ctx, &path, OUTLINE_MAX_FILE_BYTES, req_path)
                .await?;
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

        let requested_symbols = parse_requested_symbols(&args);
        if !requested_symbols.is_empty() {
            let raw = read_jailed_string_async(ctx, &path, OUTLINE_MAX_FILE_BYTES, req_path)
                .await?;
            record_fresh_file_state(ctx, &path, &raw);
            let report = extract_outline(&path, &raw, OUTLINE_MAX_ENTRIES);
            let hits = find_outline_symbol_spans(&report, &requested_symbols);
            if hits.is_empty() {
                let preview: Vec<String> = report
                    .entries
                    .iter()
                    .take(READ_FILE_SYMBOL_NOT_FOUND_PREVIEW)
                    .map(|e| format!("  {}:{}| [{}] {} (L{}..L{})", e.line, e.hash, e.kind, e.signature, e.line, e.end_line))
                    .collect();
                let avail = match preview.is_empty() {
                    true => "No structural symbols detected in this file; read explicit `start_line`/`end_line` windows.".to_string(),
                    false => format!(
                        "Available symbols ({} total, showing up to {}):\n{}",
                        report.entries.len(),
                        preview.len(),
                        preview.join("\n")
                    ),
                };
                return Ok(ToolOutput::ok(format!(
                    "Symbol(s) [{}] not found in `{req_path}` ({} lines).\n{avail}",
                    requested_symbols.join(", "),
                    report.total_lines
                )));
            }
            let lines: Vec<&str> = raw.lines().collect();
            let mut out = format!(
                "File: {req_path} ({} matched symbol(s), total {} lines)\n",
                hits.len(),
                lines.len()
            );
            for entry in &hits {
                let start = entry.line.max(1);
                let end = entry.end_line.max(start).min(lines.len());
                out.push_str(&format!(
                    "\n--- [{}] {} (lines {start}..={end}) ---\n",
                    entry.kind, entry.signature
                ));
                for ln in start..=end {
                    let text = lines[ln - 1];
                    let h = line_hash(text);
                    out.push_str(&format!("{ln}:{h}| {text}\n"));
                }
            }
            let mut res = ToolOutput::ok(out);
            res.metadata = Some(json!({
                "mode": "symbol",
                "matched_symbols": hits.len(),
                "total_lines": lines.len(),
            }));
            return Ok(res);
        }

        let was_explicit_window = args.get("start_line").is_some()
            || args.get("end_line").is_some()
            || args.get("ranges").is_some();
        let ranges = parse_read_ranges(&args)?;
        let max_requested_end = ranges.iter().map(|r| r.end).max().unwrap_or(1);

        // Stream line-by-line via kernel-confined `openat(O_NOFOLLOW)` handle with strict RAM bounds.
        let file = open_jailed_async(ctx, &path).await?;
        let file_bytes = file.metadata().await?.len();
        let mut reader = BufReader::with_capacity(16 * 1024, file);
        let mut raw_line = Vec::with_capacity(512);
        let mut line_no = 0usize;
        let mut captured_per_range: Vec<Vec<String>> = vec![Vec::new(); ranges.len()];
        let mut out_bytes = 0usize;
        let mut hit_byte_budget = false;
        let byte_budget = ctx
            .max_output_bytes
            .saturating_mul(16)
            .clamp(64 * 1024, 512 * 1024);
        let stop_early = file_bytes > DEFAULT_READ_FILE_MAX_BYTES;
        let track_hash = ctx.file_state.is_some() && !stop_early;
        let mut full_hasher = track_hash.then(Sha256::new);
        let mut active_range_idx = 0usize;

        loop {
            raw_line.clear();
            let mut truncated_line = false;
            let n = {
                use tokio::io::AsyncReadExt;
                let mut limited = (&mut reader).take(FS_MAX_SINGLE_READ_LINE_BYTES as u64);
                limited.read_until(b'\n', &mut raw_line).await?
            };
            if n == 0 {
                break;
            }
            if let Some(ref mut hasher) = full_hasher {
                hasher.update(&raw_line);
            }
            if !raw_line.ends_with(b"\n") && n == FS_MAX_SINGLE_READ_LINE_BYTES {
                truncated_line = true;
                let mut discard = [0u8; 4096];
                loop {
                    let buf = reader.fill_buf().await?;
                    if buf.is_empty() {
                        break;
                    }
                    if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                        if let Some(ref mut hasher) = full_hasher {
                            hasher.update(&buf[..=pos]);
                        }
                        reader.consume(pos + 1);
                        break;
                    }
                    let len = buf.len().min(discard.len());
                    discard[..len].copy_from_slice(&buf[..len]);
                    if let Some(ref mut hasher) = full_hasher {
                        hasher.update(&discard[..len]);
                    }
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
            {
                match out_bytes < byte_budget {
                    true => {
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
                    false => {
                        hit_byte_budget = true;
                    }
                }
            }
            if stop_early && line_no >= max_requested_end {
                break;
            }
        }

        if let (Some(bridge), Some(hasher)) = (ctx.file_state.as_ref(), full_hasher) {
            let digest = hasher.finalize();
            let mut sha = String::with_capacity(64);
            for b in digest {
                use std::fmt::Write as _;
                let _ = write!(&mut sha, "{b:02x}");
            }
            let canon = canonical_state_path(&path);
            bridge.record_file_state(&ctx.session_id, &canon, sha, file_bytes);
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
            let budget_note = match hit_byte_budget {
                true => " [byte budget reached; narrow range to read further]",
                false => "",
            };
            let mut out = format!(
                "File: {req_path} (lines {}..={end_shown} of {total_label}){budget_note}\n",
                r.start
            );
            for row in rows {
                out.push_str(row);
            }
            if !was_explicit_window
                && line_no >= READ_FILE_CONTEXT_DIET_HINT_MIN_LINES
                && end_shown < line_no
            {
                out.push_str(&format!(
                    "\n[context-diet: {total_label} lines total — use `mode=\"outline\"`, `symbol=\"<name>\"`, or `ranges=[...]` instead of paginating full files]\n"
                ));
            }
            return Ok(ToolOutput::ok(out));
        }

        let mut out = format!(
            "File: {req_path} ({} slices, total {total_label} lines)\n",
            ranges.len()
        );
        for (r, rows) in ranges.iter().zip(captured_per_range.iter()) {
            if rows.is_empty() {
                let reason = match (r.start > line_no, hit_byte_budget) {
                    (true, _) => format!("past EOF; file has {line_no} lines"),
                    (false, true) => "omitted: read byte budget reached; request this range separately".to_string(),
                    (false, false) => "empty".to_string(),
                };
                out.push_str(&format!("\n--- lines {}..={} ({reason}) ---\n", r.start, r.end));
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

// ─── hash_edit (Multi-Hunk Atomic Hashline Edits + Fine-Grained Ops + Recovery) ──

#[derive(Debug, Clone)]
struct HashEditHunk {
    op: HashEditOp,
    start_line: usize,
    start_hash: String,
    end_line: usize,
    end_hash: String,
    replacement: String,
}

fn hash_edit_op_order(op: HashEditOp) -> u8 {
    match op {
        HashEditOp::InsertBefore => 0,
        HashEditOp::Replace | HashEditOp::Delete => 1,
        HashEditOp::InsertAfter => 2,
    }
}

fn hash_hunks_conflict(prev: &HashEditHunk, cur: &HashEditHunk) -> bool {
    match (prev.op, cur.op) {
        (HashEditOp::InsertBefore, HashEditOp::InsertBefore)
        | (HashEditOp::InsertAfter, HashEditOp::InsertAfter) => prev.start_line == cur.start_line,
        (HashEditOp::InsertBefore, HashEditOp::InsertAfter) => false,
        (HashEditOp::InsertAfter, HashEditOp::InsertBefore) => prev.start_line == cur.start_line,
        (HashEditOp::Replace | HashEditOp::Delete, HashEditOp::Replace | HashEditOp::Delete) => {
            cur.start_line <= prev.end_line
        }
        (
            HashEditOp::InsertBefore | HashEditOp::InsertAfter,
            HashEditOp::Replace | HashEditOp::Delete,
        ) => prev.start_line >= cur.start_line && prev.start_line <= cur.end_line,
        (
            HashEditOp::Replace | HashEditOp::Delete,
            HashEditOp::InsertBefore | HashEditOp::InsertAfter,
        ) => cur.start_line >= prev.start_line && cur.start_line <= prev.end_line,
    }
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
    let op = match obj.get("op").and_then(Value::as_str) {
        Some(raw) => HashEditOp::parse(raw).ok_or_else(|| {
            ZenkoError::Tool(format!(
                "invalid `{prefix}op` `{raw}`. Valid ops: {}.",
                HASH_EDIT_OPS.join(", ")
            ))
        })?,
        None => HashEditOp::Replace,
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
    if matches!(op, HashEditOp::InsertAfter | HashEditOp::InsertBefore) && end_line != start_line {
        return Err(ZenkoError::Tool(format!(
            "`{prefix}op` `{}` anchors on a single line (`start_line`={start_line}, omit `end_line` or set equal to `start_line`)",
            op.as_str()
        )));
    }
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
    let replacement = match op {
        HashEditOp::Delete => obj
            .get("replacement")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        HashEditOp::Replace | HashEditOp::InsertAfter | HashEditOp::InsertBefore => obj
            .get("replacement")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool(format!("missing `{prefix}replacement`")))?
            .to_string(),
    };
    Ok(HashEditHunk {
        op,
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
            description: "Atomically apply one or multiple fine-grained hashline edits (`replace`, `insert_after`, `insert_before`, `delete`) verified by 3-char hashes from `read_file`. Use `insert_after`/`insert_before` to insert lines anchored on a line hash without duplicating existing code. On hash mismatch, returns fresh hash-anchored lines inline so you can retry immediately.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace or `hub:` file path" },
                    "op": {
                        "type": "string",
                        "enum": HASH_EDIT_OPS,
                        "description": "Edit operation for single-hunk mode: `replace` (default), `insert_after`, `insert_before`, or `delete`"
                    },
                    "start_line": { "type": "integer", "description": "1-based start/anchor line (single-hunk mode)" },
                    "start_hash": { "type": "string", "description": "3-char hash of start_line (single-hunk mode)" },
                    "end_line": { "type": "integer", "description": "1-based inclusive end line (defaults to start_line)" },
                    "end_hash": { "type": "string", "description": "3-char hash of end_line (defaults to start_hash when single line)" },
                    "replacement": { "type": "string", "description": "Replacement or inserted text (optional for `delete`)" },
                    "edits": {
                        "type": "array",
                        "description": "Multi-hunk atomic edits applied bottom-up in one write (non-conflicting line anchors/spans)",
                        "items": {
                            "type": "object",
                            "properties": {
                                "op": { "type": "string", "enum": HASH_EDIT_OPS },
                                "start_line": { "type": "integer" },
                                "start_hash": { "type": "string" },
                                "end_line": { "type": "integer" },
                                "end_hash": { "type": "string" },
                                "replacement": { "type": "string" }
                            },
                            "required": ["start_line", "start_hash"]
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
        let old = read_jailed_string_async(ctx, &path, FS_MAX_EDIT_FILE_BYTES, req_path).await?;
        let eol = detect_line_ending(&old);
        let mut lines: Vec<String> = old.lines().map(String::from).collect();

        // Step 1: Bounds & conflict validation across all hunks before mutating anything.
        hunks.sort_by_key(|h| (h.start_line, hash_edit_op_order(h.op), h.end_line));
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
            if i > 0 && hash_hunks_conflict(&hunks[i - 1], h) {
                return Ok(ToolOutput::err(format!(
                    "Conflicting hash_edit hunks: {} {}..={} conflicts with {} {}..={}. Merge overlapping edits into a single hunk.",
                    hunks[i - 1].op.as_str(),
                    hunks[i - 1].start_line,
                    hunks[i - 1].end_line,
                    h.op.as_str(),
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
                    "Hash mismatch on {} {}..={} in `{req_path}`: expected start {}:{}, end {}:{} — actual start {}:{actual_start}, end {}:{actual_end}.",
                    h.op.as_str(),
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

        // Step 3: Apply splices bottom-up so earlier line indices never drift.
        let applied_spans: Vec<String> = hunks
            .iter()
            .map(|h| format!("{}({}..={})", h.op.as_str(), h.start_line, h.end_line))
            .collect();
        for h in hunks.into_iter().rev() {
            let replacement_lines: Vec<String> = match h.replacement.is_empty() {
                true => Vec::new(),
                false => h.replacement.lines().map(String::from).collect(),
            };
            match h.op {
                HashEditOp::Replace => {
                    lines.splice((h.start_line - 1)..h.end_line, replacement_lines);
                }
                HashEditOp::Delete => {
                    lines.splice((h.start_line - 1)..h.end_line, std::iter::empty());
                }
                HashEditOp::InsertAfter => {
                    lines.splice(h.start_line..h.start_line, replacement_lines);
                }
                HashEditOp::InsertBefore => {
                    let idx = h.start_line - 1;
                    lines.splice(idx..idx, replacement_lines);
                }
            }
        }

        let mut new = lines.join(eol);
        if old.ends_with('\n') && !new.ends_with('\n') {
            new.push_str(eol);
        }

        atomic_write_in_ctx(ctx, &path, &new).await?;
        record_fresh_file_state(ctx, &path, &new);
        let diff = unified_diff(req_path, &old, &new);
        Ok(ToolOutput::ok_with_diff(
            format!(
                "Applied {} hash_edit hunk(s) to {req_path} ({}).",
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

enum PlannedFileTx {
    Create {
        req_path: String,
        abs_path: PathBuf,
        content: String,
    },
    Update {
        req_path: String,
        abs_path: PathBuf,
        old_content: String,
        new_content: String,
        replacements: usize,
        move_to: Option<(String, PathBuf)>,
    },
    Delete {
        req_path: String,
        abs_path: PathBuf,
        old_content: String,
    },
}

fn apply_codex_hunk_to_buffer(
    req_path: &str,
    buf: &str,
    eol: &str,
    hunk: &CodexUpdateHunk,
    hunk_idx: usize,
) -> std::result::Result<(String, usize), String> {
    let file_lines: Vec<&str> = buf.lines().collect();
    if hunk.old_lines.is_empty() {
        let mut rebuilt: Vec<String> = file_lines.iter().map(|s| (*s).to_string()).collect();
        rebuilt.extend(hunk.new_lines.iter().cloned());
        let mut joined = rebuilt.join(eol);
        if !joined.is_empty() && !joined.ends_with('\n') {
            joined.push_str(eol);
        }
        return Ok((joined, 1));
    }
    let pattern_refs: Vec<&str> = hunk.old_lines.iter().map(String::as_str).collect();
    let mut matches = seek_sequence_all(&file_lines, &pattern_refs);
    if matches.is_empty() {
        return Err(format!(
            "Codex patch hunk #{hunk_idx} in `{req_path}` could not locate target lines."
        ));
    }
    let chosen_idx = match (matches.len(), hunk.is_eof, hunk.context_header.as_deref()) {
        (1, _, _) => matches[0],
        (_, true, _) => {
            let eof_start = file_lines.len().saturating_sub(pattern_refs.len());
            match matches.iter().copied().find(|&m| m == eof_start) {
                Some(m) => m,
                None => {
                    let one_based: Vec<usize> = matches.iter().map(|&i| i + 1).collect();
                    return Err(format!(
                        "Ambiguous Codex patch hunk #{hunk_idx} in `{req_path}` matched {} places:\n{}",
                        matches.len(),
                        format_occurrence_locations(buf, &one_based)
                    ));
                }
            }
        }
        (_, false, Some(ctx_hdr)) if !ctx_hdr.trim().is_empty() => {
            let needle = ctx_hdr.trim();
            let ctx_pos = file_lines.iter().position(|l| l.contains(needle));
            match ctx_pos {
                Some(cpos) => {
                    matches.retain(|&m| m >= cpos);
                    match matches.first().copied() {
                        Some(first) => first,
                        None => {
                            return Err(format!(
                                "Codex patch hunk #{hunk_idx} in `{req_path}` found no match after `@@ {needle}`."
                            ));
                        }
                    }
                }
                None => {
                    let one_based: Vec<usize> = matches.iter().map(|&i| i + 1).collect();
                    return Err(format!(
                        "Ambiguous Codex patch hunk #{hunk_idx} in `{req_path}` matched {} places:\n{}",
                        matches.len(),
                        format_occurrence_locations(buf, &one_based)
                    ));
                }
            }
        }
        _ => {
            let one_based: Vec<usize> = matches.iter().map(|&i| i + 1).collect();
            return Err(format!(
                "Ambiguous Codex patch hunk #{hunk_idx} in `{req_path}` matched {} places:\n{}",
                matches.len(),
                format_occurrence_locations(buf, &one_based)
            ));
        }
    };

    let mut rebuilt: Vec<String> = file_lines.iter().map(|s| (*s).to_string()).collect();
    rebuilt.splice(
        chosen_idx..(chosen_idx + pattern_refs.len()),
        hunk.new_lines.clone(),
    );
    let mut joined = rebuilt.join(eol);
    if buf.ends_with('\n') && !joined.ends_with('\n') {
        joined.push_str(eol);
    }
    Ok((joined, 1))
}

async fn plan_codex_patch_transaction(
    ctx: &ToolContext,
    raw_patch: &str,
) -> Result<std::result::Result<Vec<PlannedFileTx>, String>> {
    let (_format, ops) = parse_auto_patch(raw_patch)?;
    let mut planned = Vec::with_capacity(ops.len());
    let mut touched: HashSet<PathBuf> = HashSet::new();

    for op in ops {
        match op {
            CodexPatchFileOp::Add { path: req_path, content } => {
                let abs = resolve_workspace_or_hub_path(
                    &ctx.workspace_dir,
                    &ctx.hub_dir,
                    &req_path,
                    false,
                )?;
                let key = canonical_state_path(&abs);
                if !touched.insert(key) {
                    return Ok(Err(format!(
                        "Duplicate operation targeting `{req_path}` in the same patch transaction."
                    )));
                }
                if abs.exists() {
                    return Ok(Err(format!(
                        "Refused to add `{req_path}`: file already exists on disk."
                    )));
                }
                planned.push(PlannedFileTx::Create {
                    req_path,
                    abs_path: abs,
                    content,
                });
            }
            CodexPatchFileOp::Delete { path: req_path } => {
                let abs = resolve_workspace_or_hub_path(
                    &ctx.workspace_dir,
                    &ctx.hub_dir,
                    &req_path,
                    true,
                )?;
                let key = canonical_state_path(&abs);
                if !touched.insert(key) {
                    return Ok(Err(format!(
                        "Duplicate operation targeting `{req_path}` in the same patch transaction."
                    )));
                }
                let meta = tokio::fs::metadata(&abs).await?;
                if !meta.is_file() {
                    return Ok(Err(format!(
                        "Refused to delete `{req_path}`: target is not a regular file."
                    )));
                }
                if meta.len() > FS_MAX_EDIT_FILE_BYTES {
                    return Ok(Err(format!(
                        "Refused to delete `{req_path}`: file size ({} bytes) exceeds safety limit.",
                        meta.len()
                    )));
                }
                let old_content = tokio::fs::read_to_string(&abs).await.unwrap_or_default();
                if let Err(stale) =
                    check_existing_file_freshness(ctx, &req_path, &abs, &old_content, true)
                {
                    return Ok(Err(stale));
                }
                planned.push(PlannedFileTx::Delete {
                    req_path,
                    abs_path: abs,
                    old_content,
                });
            }
            CodexPatchFileOp::Update {
                path: req_path,
                move_to,
                hunks,
            } => {
                let abs = resolve_workspace_or_hub_path(
                    &ctx.workspace_dir,
                    &ctx.hub_dir,
                    &req_path,
                    true,
                )?;
                let key = canonical_state_path(&abs);
                if !touched.insert(key) {
                    return Ok(Err(format!(
                        "Duplicate operation targeting `{req_path}` in the same patch transaction."
                    )));
                }
                let old_content =
                    read_jailed_string_async(ctx, &abs, FS_MAX_EDIT_FILE_BYTES, &req_path).await?;
                if let Err(stale) =
                    check_existing_file_freshness(ctx, &req_path, &abs, &old_content, true)
                {
                    return Ok(Err(stale));
                }
                let resolved_move = match move_to {
                    Some(dest_req) => {
                        let dest_abs = resolve_workspace_or_hub_path(
                            &ctx.workspace_dir,
                            &ctx.hub_dir,
                            &dest_req,
                            false,
                        )?;
                        let dest_key = canonical_state_path(&dest_abs);
                        if dest_abs != abs {
                            if !touched.insert(dest_key) {
                                return Ok(Err(format!(
                                    "Conflicting patch destination `{dest_req}`."
                                )));
                            }
                            if dest_abs.exists() {
                                return Ok(Err(format!(
                                    "Refused to move `{req_path}` -> `{dest_req}`: destination already exists."
                                )));
                            }
                        }
                        Some((dest_req, dest_abs))
                    }
                    None => None,
                };
                let eol = detect_line_ending(&old_content);
                let mut working = old_content.clone();
                let mut replacements = 0usize;
                for (h_idx, hunk) in hunks.iter().enumerate() {
                    match apply_codex_hunk_to_buffer(&req_path, &working, eol, hunk, h_idx) {
                        Ok((next, count)) => {
                            working = next;
                            replacements += count;
                        }
                        Err(err) => return Ok(Err(err)),
                    }
                }
                planned.push(PlannedFileTx::Update {
                    req_path,
                    abs_path: abs,
                    old_content,
                    new_content: working,
                    replacements,
                    move_to: resolved_move,
                });
            }
        }
    }
    Ok(Ok(planned))
}

async fn plan_structured_files_transaction(
    ctx: &ToolContext,
    items: &[Value],
) -> Result<std::result::Result<Vec<PlannedFileTx>, String>> {
    if items.is_empty() {
        return Err(ZenkoError::Tool(
            "`files` must contain at least one file operation".into(),
        ));
    }
    if items.len() > APPLY_PATCH_MAX_FILES {
        return Err(ZenkoError::Tool(format!(
            "`files` exceeds maximum {APPLY_PATCH_MAX_FILES} files per atomic patch"
        )));
    }
    let mut planned = Vec::with_capacity(items.len());
    let mut touched: HashSet<PathBuf> = HashSet::new();

    for (idx, item) in items.iter().enumerate() {
        let req_path = item
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ZenkoError::Tool(format!("missing `files[{idx}].path`")))?
            .to_string();
        let action = match item.get("action").and_then(Value::as_str) {
            Some(raw) => PatchFileAction::parse(raw).ok_or_else(|| {
                ZenkoError::Tool(format!(
                    "invalid `files[{idx}].action` `{raw}`. Valid actions: {}.",
                    PATCH_FILE_ACTIONS.join(", ")
                ))
            })?,
            None => PatchFileAction::Update,
        };
        let overwrite = item
            .get("overwrite")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        match action {
            PatchFileAction::Create => {
                let content = item
                    .get("content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ZenkoError::Tool(format!(
                            "missing `files[{idx}].content` for `action=\"create\"`"
                        ))
                    })?
                    .to_string();
                let abs = resolve_workspace_or_hub_path(
                    &ctx.workspace_dir,
                    &ctx.hub_dir,
                    &req_path,
                    false,
                )?;
                let key = canonical_state_path(&abs);
                if !touched.insert(key) {
                    return Ok(Err(format!(
                        "Duplicate operation targeting `{req_path}` in `files[]`."
                    )));
                }
                if abs.exists() && !overwrite {
                    return Ok(Err(format!(
                        "Refused to create `{req_path}`: file already exists (pass `overwrite: true` or use `action=\"update\"`)."
                    )));
                }
                planned.push(PlannedFileTx::Create {
                    req_path,
                    abs_path: abs,
                    content,
                });
            }
            PatchFileAction::Delete => {
                let abs = resolve_workspace_or_hub_path(
                    &ctx.workspace_dir,
                    &ctx.hub_dir,
                    &req_path,
                    true,
                )?;
                let key = canonical_state_path(&abs);
                if !touched.insert(key) {
                    return Ok(Err(format!(
                        "Duplicate operation targeting `{req_path}` in `files[]`."
                    )));
                }
                let meta = tokio::fs::metadata(&abs).await?;
                if !meta.is_file() {
                    return Ok(Err(format!(
                        "Refused to delete `{req_path}`: target is not a regular file."
                    )));
                }
                let old_content = tokio::fs::read_to_string(&abs).await.unwrap_or_default();
                if let Err(stale) =
                    check_existing_file_freshness(ctx, &req_path, &abs, &old_content, true)
                {
                    return Ok(Err(stale));
                }
                planned.push(PlannedFileTx::Delete {
                    req_path,
                    abs_path: abs,
                    old_content,
                });
            }
            PatchFileAction::Update | PatchFileAction::Move => {
                let abs = resolve_workspace_or_hub_path(
                    &ctx.workspace_dir,
                    &ctx.hub_dir,
                    &req_path,
                    true,
                )?;
                let key = canonical_state_path(&abs);
                if !touched.insert(key) {
                    return Ok(Err(format!(
                        "Duplicate operation targeting `{req_path}` in `files[]`."
                    )));
                }
                let old_content =
                    read_jailed_string_async(ctx, &abs, FS_MAX_EDIT_FILE_BYTES, &req_path).await?;
                if let Err(stale) =
                    check_existing_file_freshness(ctx, &req_path, &abs, &old_content, true)
                {
                    return Ok(Err(stale));
                }
                let move_to_req = item
                    .get("move_to")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                if action == PatchFileAction::Move && move_to_req.is_none() {
                    return Err(ZenkoError::Tool(format!(
                        "missing `files[{idx}].move_to` for `action=\"move\"`"
                    )));
                }
                let resolved_move = match move_to_req {
                    Some(dest_req) => {
                        let dest_abs = resolve_workspace_or_hub_path(
                            &ctx.workspace_dir,
                            &ctx.hub_dir,
                            &dest_req,
                            false,
                        )?;
                        let dest_key = canonical_state_path(&dest_abs);
                        if dest_abs != abs {
                            if !touched.insert(dest_key) {
                                return Ok(Err(format!(
                                    "Conflicting destination `{dest_req}` in `files[]`."
                                )));
                            }
                            if dest_abs.exists() && !overwrite {
                                return Ok(Err(format!(
                                    "Refused to move `{req_path}` -> `{dest_req}`: destination already exists."
                                )));
                            }
                        }
                        Some((dest_req, dest_abs))
                    }
                    None => None,
                };
                let has_edits =
                    item.get("edits").is_some() || item.get("old_text").is_some();
                let (working, replacements) = match (action, has_edits) {
                    (PatchFileAction::Move, false) => (old_content.clone(), 0),
                    _ => {
                        let edits = parse_patch_edits(item)?;
                        let eol = detect_line_ending(&old_content);
                        let mut cur = old_content.clone();
                        let mut total = 0usize;
                        for (e_idx, edit) in edits.iter().enumerate() {
                            let label = format!("files[{idx}].edits[{e_idx}]");
                            match apply_single_patch_to_buffer(&req_path, &cur, eol, edit, &label) {
                                Ok((next, count)) => {
                                    cur = next;
                                    total += count;
                                }
                                Err(msg) => return Ok(Err(msg)),
                            }
                        }
                        (cur, total)
                    }
                };
                planned.push(PlannedFileTx::Update {
                    req_path,
                    abs_path: abs,
                    old_content,
                    new_content: working,
                    replacements,
                    move_to: resolved_move,
                });
            }
        }
    }
    Ok(Ok(planned))
}

async fn commit_planned_file_tx(
    ctx: &ToolContext,
    ops: Vec<PlannedFileTx>,
) -> Result<ToolOutput> {
    let mut diffs: Vec<String> = Vec::with_capacity(ops.len());
    let mut summaries: Vec<String> = Vec::with_capacity(ops.len());
    let mut edited_files: Vec<String> = Vec::new();
    let mut deleted_files: Vec<String> = Vec::new();

    for op in ops {
        match op {
            PlannedFileTx::Create {
                req_path,
                abs_path,
                content,
            } => {
                atomic_write_in_ctx(ctx, &abs_path, &content).await?;
                record_fresh_file_state(ctx, &abs_path, &content);
                diffs.push(unified_diff(&req_path, "", &content));
                summaries.push(format!("created `{req_path}` ({} bytes)", content.len()));
                edited_files.push(req_path);
            }
            PlannedFileTx::Update {
                req_path,
                abs_path,
                old_content,
                new_content,
                replacements,
                move_to,
            } => match move_to {
                Some((dest_req, dest_abs)) if dest_abs != abs_path => {
                    atomic_write_in_ctx(ctx, &dest_abs, &new_content).await?;
                    remove_jailed_in_ctx(ctx, &abs_path).await?;
                    forget_fresh_file_state(ctx, &abs_path);
                    record_fresh_file_state(ctx, &dest_abs, &new_content);
                    diffs.push(unified_diff(
                        &format!("{req_path} -> {dest_req}"),
                        &old_content,
                        &new_content,
                    ));
                    summaries.push(format!(
                        "moved `{req_path}` -> `{dest_req}` ({replacements} replacement(s))"
                    ));
                    deleted_files.push(req_path);
                    edited_files.push(dest_req);
                }
                _ => {
                    atomic_write_in_ctx(ctx, &abs_path, &new_content).await?;
                    record_fresh_file_state(ctx, &abs_path, &new_content);
                    diffs.push(unified_diff(&req_path, &old_content, &new_content));
                    summaries.push(format!(
                        "updated `{req_path}` ({replacements} replacement(s))"
                    ));
                    edited_files.push(req_path);
                }
            },
            PlannedFileTx::Delete {
                req_path,
                abs_path,
                old_content,
            } => {
                remove_jailed_in_ctx(ctx, &abs_path).await?;
                forget_fresh_file_state(ctx, &abs_path);
                diffs.push(unified_diff(&req_path, &old_content, ""));
                summaries.push(format!("deleted `{req_path}`"));
                deleted_files.push(req_path);
            }
        }
    }

    let combined_diff = diffs.join("\n");
    let mut out = ToolOutput::ok_with_diff(
        format!(
            "Committed atomic patch across {} file(s): {}.",
            summaries.len(),
            summaries.join("; ")
        ),
        combined_diff,
    );
    out.metadata = Some(json!({
        "edited_files": edited_files,
        "deleted_files": deleted_files,
    }));
    Ok(out)
}

pub struct ApplyPatchTool;

#[async_trait]
impl Tool for ApplyPatchTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::CoreFs
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        match args.get("patch").is_some() || args.get("files").is_some() {
            true => ToolConcurrency::Global,
            false => file_lock_key(args),
        }
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_APPLY_PATCH.into(),
            description: "Atomically apply single-file or multi-file patches (create, update, move/rename, delete) with two-phase validation (zero disk writes if any file fails), kernel-confined `O_NOFOLLOW` I/O, tolerant seeking, ambiguity protection, and stale-read safety. Supports single-file `path`+`old_text`/`new_text` or `edits[]`, structured multi-file `files[]`, OR raw/fenced `patch` in EITHER Codex (`*** Begin Patch ... *** End Patch`) OR standard POSIX/Git unified diff (`--- a/file` / `+++ b/file` / `@@ ... @@`) format.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace or `hub:` file path (single-file mode)" },
                    "old_text": { "type": "string", "description": "Existing block to locate (single-file mode)" },
                    "new_text": { "type": "string", "description": "Replacement block (single-file mode)" },
                    "replace_all": { "type": "boolean", "description": "Replace every non-overlapping match instead of requiring uniqueness (default false)" },
                    "expected_occurrences": { "type": "integer", "description": "Optional exact expected match count guard" },
                    "edits": {
                        "type": "array",
                        "description": "Atomic sequential list of `{old_text, new_text, replace_all?, expected_occurrences?}` edits (single-file mode)",
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
                    },
                    "files": {
                        "type": "array",
                        "description": "Two-phase atomic multi-file transaction (`create`, `update`, `move`, `delete`) across up to 24 jailed files",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": { "type": "string" },
                                "action": { "type": "string", "enum": PATCH_FILE_ACTIONS },
                                "move_to": { "type": "string" },
                                "content": { "type": "string" },
                                "overwrite": { "type": "boolean" },
                                "old_text": { "type": "string" },
                                "new_text": { "type": "string" },
                                "replace_all": { "type": "boolean" },
                                "expected_occurrences": { "type": "integer" },
                                "edits": { "type": "array" }
                            },
                            "required": ["path"]
                        }
                    },
                    "patch": {
                        "type": "string",
                        "description": "Multi-file patch block (raw or fenced): accepts EITHER Codex (`*** Begin Patch ... *** End Patch`) OR standard POSIX/Git unified diff (`--- a/file` / `+++ b/file` / `@@ ... @@`)"
                    }
                }
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
        if let Some(raw_patch) = args.get("patch").and_then(Value::as_str) {
            return match plan_codex_patch_transaction(ctx, raw_patch).await? {
                Ok(planned) => commit_planned_file_tx(ctx, planned).await,
                Err(msg) => Ok(ToolOutput::err(msg)),
            };
        }

        if let Some(files) = args.get("files").and_then(Value::as_array) {
            return match plan_structured_files_transaction(ctx, files).await? {
                Ok(planned) => commit_planned_file_tx(ctx, planned).await,
                Err(msg) => Ok(ToolOutput::err(msg)),
            };
        }

        let req_path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ZenkoError::Tool("missing `path`, `files`, or `patch`".into())
            })?;
        let edits = parse_patch_edits(&args)?;

        let path = resolve_workspace_or_hub_path(&ctx.workspace_dir, &ctx.hub_dir, req_path, true)?;
        let old = read_jailed_string_async(ctx, &path, FS_MAX_EDIT_FILE_BYTES, req_path).await?;
        if let Err(stale) = check_existing_file_freshness(ctx, req_path, &path, &old, true) {
            return Ok(ToolOutput::err(stale));
        }
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

        atomic_write_in_ctx(ctx, &path, &working).await?;
        record_fresh_file_state(ctx, &path, &working);
        let diff = unified_diff(req_path, &old, &working);
        let mut out = ToolOutput::ok_with_diff(
            format!(
                "Patched {req_path} successfully ({total_replacements} replacement(s) across {} edit(s)).",
                edits.len()
            ),
            diff,
        );
        out.metadata = Some(json!({ "edited_files": [req_path] }));
        Ok(out)
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
            description: "Create, overwrite, or atomically append (`append: true`) to a file inside the workspace through kernel-confined `O_NOFOLLOW` writes. Protects against blind overwrites of existing unread files (pass `overwrite: true` to replace intentionally) and blocks silent clobbers when a previously read file changed externally on disk.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" },
                    "overwrite": {
                        "type": "boolean",
                        "description": "Explicitly allow overwriting an existing file that has not been read in this session (default false)"
                    },
                    "append": {
                        "type": "boolean",
                        "description": "Atomically append `content` to the end of the file (creating it if missing) without rewriting prior contents (default false)"
                    }
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
        let overwrite = args
            .get("overwrite")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let append = args
            .get("append")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let path = resolve_workspace_or_hub_path(&ctx.workspace_dir, &ctx.hub_dir, req_path, false)?;
        let existed = path.is_file();
        let old = match (existed, append) {
            (true, true) => {
                read_jailed_string_async(ctx, &path, FS_MAX_EDIT_FILE_BYTES, req_path).await?
            }
            (true, false) => read_jailed_string_async(ctx, &path, FS_MAX_EDIT_FILE_BYTES, req_path)
                .await
                .unwrap_or_default(),
            (false, _) => String::new(),
        };
        if existed
            && let Err(stale_msg) =
                check_existing_file_freshness(ctx, req_path, &path, &old, overwrite || append)
        {
            return Ok(ToolOutput::err(stale_msg));
        }

        let (final_content, summary) = match append {
            true => {
                let next_len = old.len().saturating_add(content.len()) as u64;
                if next_len > FS_MAX_EDIT_FILE_BYTES {
                    return Ok(ToolOutput::err(format!(
                        "Refused to append to '{req_path}': resulting size ({next_len} bytes) exceeds edit safety limit ({FS_MAX_EDIT_FILE_BYTES} bytes)."
                    )));
                }
                let mut combined = String::with_capacity(old.len() + content.len());
                combined.push_str(&old);
                combined.push_str(content);
                let msg = format!(
                    "Appended {} bytes to {req_path} (total {} bytes).",
                    content.len(),
                    combined.len()
                );
                (combined, msg)
            }
            false => (
                content.to_string(),
                format!("Wrote {} bytes to {req_path}.", content.len()),
            ),
        };

        atomic_write_in_ctx(ctx, &path, &final_content).await?;
        record_fresh_file_state(ctx, &path, &final_content);
        let diff = unified_diff(req_path, &old, &final_content);
        Ok(ToolOutput::ok_with_diff(summary, diff))
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

    fn exposure(&self) -> zenko_core::ToolExposure {
        zenko_core::ToolExposure::Deferred
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

pub(crate) async fn atomic_write(path: &std::path::Path, content: &str) -> Result<()> {
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
            file_state: None,
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

    #[derive(Debug, Default)]
    struct MockFileState(std::sync::Mutex<std::collections::HashMap<PathBuf, String>>);

    impl zenko_core::FileStateBridge for MockFileState {
        fn record_file_state(&self, _sid: &str, path: &Path, sha256: String, _bytes: u64) {
            if let Ok(mut map) = self.0.lock() {
                map.insert(path.to_path_buf(), sha256);
            }
        }
        fn forget_file_state(&self, _sid: &str, path: &Path) {
            if let Ok(mut map) = self.0.lock() {
                map.remove(path);
            }
        }
        fn check_file_freshness(
            &self,
            _sid: &str,
            path: &Path,
            current_sha256: &str,
        ) -> FileFreshnessCheck {
            let guard = self.0.lock().expect("lock");
            match guard.get(path) {
                None => FileFreshnessCheck::UnreadExisting,
                Some(prev) if prev == current_sha256 => FileFreshnessCheck::Fresh,
                Some(prev) => FileFreshnessCheck::ExternallyModified {
                    recorded_sha256: prev.clone(),
                    current_sha256: current_sha256.to_string(),
                },
            }
        }
    }

    #[tokio::test]
    async fn hash_edit_supports_insert_after_insert_before_and_delete() {
        let dir = std::env::temp_dir().join(format!("zenko_fs_v3_ops_{}", zenko_core::new_id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("lib.rs");
        std::fs::write(&file, "use std::io;\nfn alpha() {}\nfn dead() {}\n").expect("write");

        let ctx = test_ctx(&dir);
        let cancel = CancellationToken::new();
        let h1 = line_hash("use std::io;");
        let h2 = line_hash("fn alpha() {}");
        let h3 = line_hash("fn dead() {}");

        let res = HashEditTool
            .execute(
                &ctx,
                json!({
                    "path": "lib.rs",
                    "edits": [
                        { "op": "insert_after", "start_line": 1, "start_hash": h1, "replacement": "use std::fs;" },
                        { "op": "insert_before", "start_line": 2, "start_hash": h2, "replacement": "// doc comment" },
                        { "op": "delete", "start_line": 3, "start_hash": h3 }
                    ]
                }),
                cancel,
            )
            .await
            .expect("exec");
        assert!(!res.is_error, "err={}", res.content);
        assert_eq!(
            std::fs::read_to_string(&file).expect("read"),
            "use std::io;\nuse std::fs;\n// doc comment\nfn alpha() {}\n"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn apply_patch_executes_atomic_multi_file_and_codex_patch_with_rollback() {
        let dir = std::env::temp_dir().join(format!("zenko_patch_v3_{}", zenko_core::new_id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("a.rs"), "let x = 1;\n").expect("a");
        std::fs::write(dir.join("obsolete.rs"), "old\n").expect("obs");

        let ctx = test_ctx(&dir);
        let cancel = CancellationToken::new();

        // 1. Failing second file in `files[]` must NOT touch the first file on disk!
        let fail_tx = ApplyPatchTool
            .execute(
                &ctx,
                json!({
                    "files": [
                        { "path": "a.rs", "action": "update", "old_text": "let x = 1;", "new_text": "let x = 99;" },
                        { "path": "a.rs", "action": "delete" }
                    ]
                }),
                cancel.clone(),
            )
            .await
            .expect("fail_tx");
        assert!(fail_tx.is_error);
        assert_eq!(std::fs::read_to_string(dir.join("a.rs")).expect("read"), "let x = 1;\n");

        // 2. Native Codex multi-file patch (add + update/move + delete) succeeds atomically!
        let codex = "*** Begin Patch\n*** Add File: created.rs\n+pub const OK: bool = true;\n*** Update File: a.rs\n*** Move to: b.rs\n@@\n-let x = 1;\n+let x = 2;\n*** Delete File: obsolete.rs\n*** End Patch\n";
        let ok = ApplyPatchTool
            .execute(&ctx, json!({ "patch": codex }), cancel)
            .await
            .expect("codex");
        assert!(!ok.is_error, "codex err={}", ok.content);
        assert!(!dir.join("a.rs").exists());
        assert!(!dir.join("obsolete.rs").exists());
        assert_eq!(std::fs::read_to_string(dir.join("b.rs")).expect("b"), "let x = 2;\n");
        assert_eq!(
            std::fs::read_to_string(dir.join("created.rs")).expect("created"),
            "pub const OK: bool = true;\n"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn stale_read_guard_blocks_unread_overwrite_and_external_drift() {
        let dir = std::env::temp_dir().join(format!("zenko_stale_v3_{}", zenko_core::new_id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("config.toml");
        std::fs::write(&file, "mode = \"safe\"\n").expect("write");

        let mut ctx = test_ctx(&dir);
        ctx.file_state = Some(std::sync::Arc::new(MockFileState::default()));
        let cancel = CancellationToken::new();

        // 1. Blind overwrite of existing unread file is refused unless `overwrite: true`
        let blocked = WriteFileTool
            .execute(
                &ctx,
                json!({ "path": "config.toml", "content": "mode = \"fast\"\n" }),
                cancel.clone(),
            )
            .await
            .expect("blocked");
        assert!(blocked.is_error);
        assert!(blocked.content.contains("stale-read guard"), "out={}", blocked.content);

        // 2. Explicit `overwrite: true` succeeds under Guarded Freedom!
        let forced = WriteFileTool
            .execute(
                &ctx,
                json!({ "path": "config.toml", "content": "mode = \"fast\"\n", "overwrite": true }),
                cancel.clone(),
            )
            .await
            .expect("forced");
        assert!(!forced.is_error, "forced err={}", forced.content);

        // 3. Reading records fingerprint; external disk edit causes stale-read block!
        let _ = ReadFileTool
            .execute(&ctx, json!({ "path": "config.toml" }), cancel.clone())
            .await
            .expect("read");
        std::fs::write(&file, "mode = \"external_edit\"\n").expect("ext");
        let drifted = ApplyPatchTool
            .execute(
                &ctx,
                json!({ "path": "config.toml", "old_text": "external_edit", "new_text": "clobber" }),
                cancel,
            )
            .await
            .expect("drift");
        assert!(drifted.is_error);
        assert!(drifted.content.contains("changed externally"), "out={}", drifted.content);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn chisel_symbol_read_context_diet_unified_diff_and_append_work_end_to_end() {
        let dir = std::env::temp_dir().join(format!("zenko_chisel_{}", zenko_core::new_id()));
        std::fs::create_dir_all(dir.join("src")).expect("mkdir");

        // Build a 320-line Rust file with distinct symbols to test one-shot symbol read & Context Diet hint
        let mut big = String::new();
        for i in 1..=305 {
            big.push_str(&format!("// filler line {i}\n"));
        }
        big.push_str("pub fn compute_tax(amount: u64) -> u64 {\n    amount / 10\n}\n\npub struct Ledger {\n    pub total: u64,\n}\n");
        std::fs::write(dir.join("src/billing.rs"), &big).expect("write billing");

        let mut ctx = test_ctx(&dir);
        ctx.file_state = Some(std::sync::Arc::new(MockFileState::default()));
        let cancel = CancellationToken::new();

        // 1. Unwindowed read of >= 300-line file emits Context Diet footer; explicit window does NOT
        let unwindowed = ReadFileTool
            .execute(&ctx, json!({ "path": "src/billing.rs" }), cancel.clone())
            .await
            .expect("unwindowed");
        assert!(!unwindowed.is_error);
        assert!(
            unwindowed.content.contains("[context-diet:"),
            "expected context-diet footer on large unwindowed read: {}",
            unwindowed.content
        );
        let windowed = ReadFileTool
            .execute(
                &ctx,
                json!({ "path": "src/billing.rs", "start_line": 1, "end_line": 5 }),
                cancel.clone(),
            )
            .await
            .expect("windowed");
        assert!(!windowed.content.contains("[context-diet:"));

        // 2. One-shot `read_file(symbol=...)` returns ONLY that symbol with hash anchors
        let sym = ReadFileTool
            .execute(
                &ctx,
                json!({ "path": "src/billing.rs", "symbol": "compute_tax" }),
                cancel.clone(),
            )
            .await
            .expect("symbol read");
        assert!(!sym.is_error);
        assert!(sym.content.contains("pub fn compute_tax"));
        assert!(sym.content.contains("amount / 10"));
        assert!(!sym.content.contains("pub struct Ledger"));

        // 3. Standard POSIX/Git unified diff works directly in `apply_patch`
        let udiff = "\
```diff
--- a/src/billing.rs
+++ b/src/billing.rs
@@ -306,3 +306,3 @@ pub fn compute_tax
 pub fn compute_tax(amount: u64) -> u64 {
-    amount / 10
+    amount / 5
 }
```";
        let patched = ApplyPatchTool
            .execute(&ctx, json!({ "patch": udiff }), cancel.clone())
            .await
            .expect("unified diff patch");
        assert!(!patched.is_error, "unified diff failed: {}", patched.content);
        let updated = std::fs::read_to_string(dir.join("src/billing.rs")).expect("read");
        assert!(updated.contains("amount / 5"));

        // 4. `write_file(append=true)` performs atomic O(delta) append and preserves prior bytes
        std::fs::write(dir.join("NOTES.md"), "# Notes\n").expect("notes");
        let app = WriteFileTool
            .execute(
                &ctx,
                json!({ "path": "NOTES.md", "content": "- added entry\n", "append": true }),
                cancel,
            )
            .await
            .expect("append");
        assert!(!app.is_error, "append failed: {}", app.content);
        assert!(app.content.contains("Appended"));
        assert_eq!(
            std::fs::read_to_string(dir.join("NOTES.md")).expect("notes after"),
            "# Notes\n- added entry\n"
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}
