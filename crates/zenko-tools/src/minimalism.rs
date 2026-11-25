//! Native Rust Code Minimalism Engine & Deferred `code_minimalism` Tool
//! (inspired by DietrichGebert/ponytail; 100% pure Rust, zero regex, zero `/commands`, zero hardcoding).
//!
//! Algorithms:
//! 1. Single-pass unified-diff parser (`parse_unified_diff_counts` & `analyze_unified_diff`)
//! 2. Outline-backed workspace bloat, long-function, thin-wrapper & duplicate-helper detector
//! 3. Comment-scoped byte/char FSM shortcut debt scanner (`DEBT:` / `SHORTCUT:` / `HACK:` / `PONYTAIL:`)
//! 4. Real SQLite-backed Minimalism Gain & Token Economy scoreboard (`action="gain"`)

use crate::outline::extract_outline;
use async_trait::async_trait;
use ignore::WalkBuilder;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    CodeMinimalismAction, DebtItem, DebtScanReport, DiffAuditReport, DuplicateSymbolFinding,
    EconomyBridge, LargeFileFinding, LongFunctionFinding, MINIMALISM_AUDIT_MAX_FILE_BYTES,
    MINIMALISM_AUDIT_MAX_FILES, MINIMALISM_AUDIT_TOP_N, MINIMALISM_DEBT_MARKERS,
    MINIMALISM_DEBT_MAX_ITEMS, MINIMALISM_DEBT_TEXT_MAX_CHARS, MINIMALISM_DIFF_LARGE_HUNK_LOC,
    MINIMALISM_DUPLICATE_SYMBOL_MIN_CHARS, MINIMALISM_IGNORED_DUP_SYMBOLS,
    MINIMALISM_LARGE_FILE_LINES, MINIMALISM_LONG_FN_LINES, MINIMALISM_MANIFEST_FILES,
    MINIMALISM_SOURCE_EXTS, MINIMALISM_WRAPPER_MAX_LINES, MINIMALISM_WRAPPER_SUFFIXES,
    MinimalismGainReport, OversizedHunkFinding, RepoAuditReport, Result, RiskLevel,
    TOOL_CODE_MINIMALISM, ThinWrapperFinding, Tool, ToolConcurrency, ToolContext, ToolExposure,
    ToolOutput, ToolSet, ToolSpec, ZenkoError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommentStyle {
    Slash,
    Hash,
}

fn comment_style_for_ext(ext: &str) -> CommentStyle {
    match ext {
        "py" | "pyi" => CommentStyle::Hash,
        _ => CommentStyle::Slash,
    }
}

fn is_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .is_some_and(|ext| MINIMALISM_SOURCE_EXTS.contains(&ext.as_str()))
}

fn is_manifest_file(rel_path: &str) -> bool {
    let base = rel_path.rsplit('/').next().unwrap_or(rel_path);
    MINIMALISM_MANIFEST_FILES
        .iter()
        .any(|m| m.eq_ignore_ascii_case(base))
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    match trimmed.chars().count() <= max_chars {
        true => trimmed.to_string(),
        false => format!("{}...", trimmed.chars().take(max_chars).collect::<String>()),
    }
}

/// Deterministic zero-regex single-pass unified-diff counter reused by both `code_minimalism`
/// and runtime edit hooks (`write_file`, `hash_edit`, `apply_patch`, `batch_tools`).
/// Returns `(lines_added, lines_deleted, new_files_created)`.
pub fn parse_unified_diff_counts(diff: &str) -> (u64, u64, usize) {
    let mut added = 0u64;
    let mut deleted = 0u64;
    let mut new_files = 0usize;
    let mut prev_dev_null = false;

    for line in diff.lines() {
        match line {
            l if l.starts_with("diff --git ") => {
                prev_dev_null = false;
            }
            l if l == "--- /dev/null" || l.starts_with("--- /dev/null\t") => {
                prev_dev_null = true;
            }
            l if l.starts_with("--- ") => {
                prev_dev_null = false;
            }
            l if l.starts_with("+++ ") => {
                if prev_dev_null && !l.contains("/dev/null") {
                    new_files = new_files.saturating_add(1);
                }
                prev_dev_null = false;
            }
            l if l.starts_with("@@ ") || l.starts_with('\\') => {}
            l if l.starts_with('+') => {
                added = added.saturating_add(1);
            }
            l if l.starts_with('-') => {
                deleted = deleted.saturating_add(1);
            }
            _ => {}
        }
    }
    (added, deleted, new_files)
}

/// Extracts a clean symbol identifier from an `OutlineEntry` signature without regex.
fn extract_symbol_name(kind: &str, sig: &str) -> Option<String> {
    let trimmed = sig.trim();
    let keyword_anchors: &[&str] = match kind {
        "fn" => &["fn ", "function ", "function*", "def ", "func ", "fun "],
        "struct" => &["struct "],
        "enum" => &["enum "],
        "trait" => &["trait "],
        "class" => &["class "],
        "interface" => &["interface ", "protocol "],
        _ => return None,
    };

    for anchor in keyword_anchors {
        if let Some(pos) = trimmed.find(anchor) {
            let after = trimmed[pos + anchor.len()..].trim_start();
            let ident: String = after
                .bytes()
                .take_while(|&b| is_ident_byte(b))
                .map(char::from)
                .collect();
            if !ident.is_empty() {
                return Some(ident);
            }
        }
    }

    // Fallback for JS/TS `const foo = (...) =>` or `name(...) {` method syntax.
    let first_tok: String = trimmed
        .split_whitespace()
        .find(|tok| {
            !matches!(
                *tok,
                "pub"
                    | "async"
                    | "unsafe"
                    | "const"
                    | "let"
                    | "var"
                    | "export"
                    | "default"
                    | "static"
                    | "public"
                    | "private"
                    | "protected"
                    | "readonly"
                    | "override"
            )
        })?
        .bytes()
        .take_while(|&b| is_ident_byte(b))
        .map(char::from)
        .collect();
    match first_tok.is_empty() {
        true => None,
        false => Some(first_tok),
    }
}

/// Deterministic comment-scoped byte/char FSM extracting shortcut/debt markers without regex.
/// String literals and constant tables outside comments are skipped so marker definitions never self-trigger.
pub fn scan_file_debt_markers(
    rel_path: &str,
    content: &str,
    ext: &str,
    out: &mut Vec<DebtItem>,
    max_items: usize,
) -> usize {
    let style = comment_style_for_ext(ext);
    let bytes = content.as_bytes();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut found = 0usize;

    #[derive(Clone, Copy)]
    enum State {
        Code,
        SingleQuote,
        DoubleQuote,
        LineComment,
        BlockComment,
    }

    let mut state = State::Code;
    let mut comment_start = 0usize;
    let mut comment_line = 1usize;

    let mut check_comment = |slice: &str, start_line: usize| {
        let mut cur_line = start_line;
        for raw_seg in slice.split('\n') {
            if let Some((marker, text)) = find_debt_marker_in_segment(raw_seg) {
                found += 1;
                if out.len() < max_items {
                    out.push(DebtItem {
                        path: rel_path.to_string(),
                        line: cur_line,
                        marker: marker.to_string(),
                        text: truncate_chars(text, MINIMALISM_DEBT_TEXT_MAX_CHARS),
                    });
                }
            }
            cur_line += 1;
        }
    };

    while i < bytes.len() {
        let b = bytes[i];
        match state {
            State::Code => match (style, b, bytes.get(i + 1).copied()) {
                (_, b'\n', _) => {
                    line += 1;
                    i += 1;
                }
                (_, b'"', _) => {
                    state = State::DoubleQuote;
                    i += 1;
                }
                (CommentStyle::Hash, b'\'', _) => {
                    state = State::SingleQuote;
                    i += 1;
                }
                (CommentStyle::Slash, b'\'', _) => {
                    // Distinguish Rust lifetime `'a` from char literal `'x'` / `'\n'`
                    let is_char_lit = bytes.get(i + 2) == Some(&b'\'')
                        || (bytes.get(i + 1) == Some(&b'\\') && bytes.get(i + 3) == Some(&b'\''));
                    if is_char_lit {
                        state = State::SingleQuote;
                    }
                    i += 1;
                }
                (CommentStyle::Slash, b'/', Some(b'/')) => {
                    state = State::LineComment;
                    comment_start = i + 2;
                    comment_line = line;
                    i += 2;
                }
                (CommentStyle::Slash, b'/', Some(b'*')) => {
                    state = State::BlockComment;
                    comment_start = i + 2;
                    comment_line = line;
                    i += 2;
                }
                (CommentStyle::Hash, b'#', _) => {
                    state = State::LineComment;
                    comment_start = i + 1;
                    comment_line = line;
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            },
            State::DoubleQuote => match b {
                b'\\' if i + 1 < bytes.len() => {
                    if bytes[i + 1] == b'\n' {
                        line += 1;
                    }
                    i += 2;
                }
                b'"' => {
                    state = State::Code;
                    i += 1;
                }
                b'\n' => {
                    line += 1;
                    state = State::Code;
                    i += 1;
                }
                _ => i += 1,
            },
            State::SingleQuote => match b {
                b'\\' if i + 1 < bytes.len() => {
                    if bytes[i + 1] == b'\n' {
                        line += 1;
                    }
                    i += 2;
                }
                b'\'' => {
                    state = State::Code;
                    i += 1;
                }
                b'\n' => {
                    line += 1;
                    state = State::Code;
                    i += 1;
                }
                _ => i += 1,
            },
            State::LineComment => match b {
                b'\n' => {
                    if let Some(slice) = content.get(comment_start..i) {
                        check_comment(slice, comment_line);
                    }
                    line += 1;
                    state = State::Code;
                    i += 1;
                }
                _ => i += 1,
            },
            State::BlockComment => match (b, bytes.get(i + 1).copied()) {
                (b'*', Some(b'/')) => {
                    if let Some(slice) = content.get(comment_start..i) {
                        check_comment(slice, comment_line);
                    }
                    state = State::Code;
                    i += 2;
                }
                (b'\n', _) => {
                    line += 1;
                    i += 1;
                }
                _ => i += 1,
            },
        }
    }
    if matches!(state, State::LineComment | State::BlockComment)
        && let Some(slice) = content.get(comment_start..bytes.len())
    {
        check_comment(slice, comment_line);
    }
    found
}

fn find_debt_marker_in_segment(seg: &str) -> Option<(&'static str, &str)> {
    let upper = seg.to_ascii_uppercase();
    let up_bytes = upper.as_bytes();
    for &marker in MINIMALISM_DEBT_MARKERS {
        let needle = marker.to_ascii_uppercase();
        let mut offset = 0usize;
        while let Some(rel) = upper[offset..].find(&needle) {
            let pos = offset + rel;
            let boundary_ok = pos == 0 || !is_ident_byte(up_bytes[pos - 1]);
            if boundary_ok {
                let tail = seg
                    .get(pos + needle.len()..)
                    .map(str::trim)
                    .unwrap_or("")
                    .trim_end_matches("*/")
                    .trim();
                return Some((marker, tail));
            }
            offset = pos + 1;
        }
    }
    None
}

/// Walks the workspace once (respecting `.gitignore`) to scan comment-scoped debt markers.
pub fn scan_workspace_debt(workspace_dir: &Path) -> DebtScanReport {
    let mut items = Vec::new();
    let mut files_scanned = 0usize;
    let mut total_found = 0usize;

    for entry in WalkBuilder::new(workspace_dir)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build()
        .flatten()
    {
        if files_scanned >= MINIMALISM_AUDIT_MAX_FILES {
            break;
        }
        let path = entry.path();
        if !path.is_file() || !is_source_file(path) {
            continue;
        }
        let Ok(meta) = path.metadata() else {
            continue;
        };
        if meta.len() > MINIMALISM_AUDIT_MAX_FILE_BYTES {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let rel = path
            .strip_prefix(workspace_dir)
            .unwrap_or(path)
            .display()
            .to_string();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        files_scanned += 1;
        total_found += scan_file_debt_markers(
            &rel,
            &content,
            &ext,
            &mut items,
            MINIMALISM_DEBT_MAX_ITEMS,
        );
    }

    DebtScanReport {
        files_scanned,
        total_found,
        truncated: total_found > items.len(),
        items,
    }
}

fn collect_workspace_symbol_map(
    workspace_dir: &Path,
) -> (RepoAuditReport, BTreeMap<String, Vec<String>>) {
    let mut files_scanned = 0usize;
    let mut total_lines = 0usize;
    let mut large_files: Vec<LargeFileFinding> = Vec::new();
    let mut long_functions: Vec<LongFunctionFinding> = Vec::new();
    let mut thin_wrappers: Vec<ThinWrapperFinding> = Vec::new();
    let mut sym_locations: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut debt_count = 0usize;
    let mut scratch_debt = Vec::new();

    for entry in WalkBuilder::new(workspace_dir)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build()
        .flatten()
    {
        if files_scanned >= MINIMALISM_AUDIT_MAX_FILES {
            break;
        }
        let path = entry.path();
        if !path.is_file() || !is_source_file(path) {
            continue;
        }
        let Ok(meta) = path.metadata() else {
            continue;
        };
        if meta.len() > MINIMALISM_AUDIT_MAX_FILE_BYTES {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let rel = path
            .strip_prefix(workspace_dir)
            .unwrap_or(path)
            .display()
            .to_string();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();

        files_scanned += 1;
        scratch_debt.clear();
        debt_count += scan_file_debt_markers(&rel, &content, &ext, &mut scratch_debt, 0);

        let outline = extract_outline(path, &content, 240);
        total_lines += outline.total_lines;
        if outline.total_lines >= MINIMALISM_LARGE_FILE_LINES {
            large_files.push(LargeFileFinding {
                path: rel.clone(),
                lines: outline.total_lines,
            });
        }

        let mut seen_in_file: BTreeSet<String> = BTreeSet::new();
        for item in &outline.entries {
            let span = item.end_line.saturating_sub(item.line) + 1;
            if item.kind == "fn" && span >= MINIMALISM_LONG_FN_LINES {
                long_functions.push(LongFunctionFinding {
                    path: rel.clone(),
                    line: item.line,
                    span_lines: span,
                    signature: truncate_chars(&item.signature, 96),
                });
            }
            let Some(name) = extract_symbol_name(item.kind, &item.signature) else {
                continue;
            };
            if matches!(item.kind, "struct" | "class" | "trait" | "interface")
                && span <= MINIMALISM_WRAPPER_MAX_LINES
                && MINIMALISM_WRAPPER_SUFFIXES
                    .iter()
                    .any(|suf| name.ends_with(suf) && name.len() > suf.len())
            {
                thin_wrappers.push(ThinWrapperFinding {
                    path: rel.clone(),
                    line: item.line,
                    span_lines: span,
                    kind: item.kind.to_string(),
                    name: name.clone(),
                });
            }
            if item.kind == "fn"
                && name.len() >= MINIMALISM_DUPLICATE_SYMBOL_MIN_CHARS
                && MINIMALISM_IGNORED_DUP_SYMBOLS
                    .binary_search(&name.as_str())
                    .is_err()
                && seen_in_file.insert(name.clone())
            {
                sym_locations
                    .entry(name)
                    .or_default()
                    .push(format!("{rel}:L{}", item.line));
            }
        }
    }

    large_files.sort_by(|a, b| b.lines.cmp(&a.lines).then_with(|| a.path.cmp(&b.path)));
    large_files.truncate(MINIMALISM_AUDIT_TOP_N);

    long_functions.sort_by(|a, b| {
        b.span_lines
            .cmp(&a.span_lines)
            .then_with(|| a.path.cmp(&b.path))
    });
    long_functions.truncate(MINIMALISM_AUDIT_TOP_N);

    thin_wrappers.sort_by(|a, b| {
        a.span_lines
            .cmp(&b.span_lines)
            .then_with(|| a.path.cmp(&b.path))
    });
    thin_wrappers.truncate(MINIMALISM_AUDIT_TOP_N);

    let mut duplicate_symbols: Vec<DuplicateSymbolFinding> = sym_locations
        .iter()
        .filter(|(_, locs)| locs.len() >= 2)
        .map(|(sym, locs)| DuplicateSymbolFinding {
            symbol: sym.clone(),
            locations: locs.iter().take(6).cloned().collect(),
        })
        .collect();
    duplicate_symbols.sort_by(|a, b| {
        b.locations
            .len()
            .cmp(&a.locations.len())
            .then_with(|| a.symbol.cmp(&b.symbol))
    });
    duplicate_symbols.truncate(MINIMALISM_AUDIT_TOP_N);

    let report = RepoAuditReport {
        files_scanned,
        total_lines,
        large_files,
        long_functions,
        duplicate_symbols,
        thin_wrappers,
        debt_count,
    };
    (report, sym_locations)
}

/// Deterministic repo-wide structural minimalism & bloat scan (zero LLM, zero regex).
pub fn audit_workspace_minimalism(workspace_dir: &Path) -> RepoAuditReport {
    collect_workspace_symbol_map(workspace_dir).0
}

fn parse_hunk_new_start(header: &str) -> usize {
    let Some(plus_idx) = header.find('+') else {
        return 1;
    };
    let rest = &header[plus_idx + 1..];
    let digits: String = rest
        .bytes()
        .take_while(|b| b.is_ascii_digit())
        .map(char::from)
        .collect();
    digits.parse::<usize>().unwrap_or(1)
}

/// Runs `git diff` safely in `workspace_dir` (staged or working tree vs HEAD) and audits the diff.
pub fn audit_git_diff_minimalism(workspace_dir: &Path, staged: bool) -> DiffAuditReport {
    let args: &[&str] = match staged {
        true => &["diff", "--staged", "--unified=0"],
        false => &["diff", "HEAD", "--unified=0"],
    };
    let out = Command::new("git")
        .args(args)
        .current_dir(workspace_dir)
        .output();
    let diff_text = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => Command::new("git")
            .args(["diff", "--unified=0"])
            .current_dir(workspace_dir)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default(),
    };
    analyze_unified_diff(workspace_dir, &diff_text)
}

/// Deterministic zero-regex analyzer over any unified diff, cross-checked against workspace symbols.
pub fn analyze_unified_diff(workspace_dir: &Path, diff_text: &str) -> DiffAuditReport {
    let mut files_changed: BTreeSet<String> = BTreeSet::new();
    let mut new_files: BTreeSet<String> = BTreeSet::new();
    let mut manifest_changes: BTreeSet<String> = BTreeSet::new();
    let mut oversized_hunks: Vec<OversizedHunkFinding> = Vec::new();
    let mut added_fn_names: BTreeSet<String> = BTreeSet::new();

    let mut cur_file = String::new();
    let mut prev_dev_null = false;
    let mut hunk_start = 1usize;
    let mut hunk_added = 0usize;
    let mut lines_added = 0u64;
    let mut lines_deleted = 0u64;

    let flush_hunk = |file: &str,
                      start: usize,
                      added_n: usize,
                      hunks: &mut Vec<OversizedHunkFinding>| {
        if !file.is_empty() && added_n >= MINIMALISM_DIFF_LARGE_HUNK_LOC {
            hunks.push(OversizedHunkFinding {
                path: file.to_string(),
                start_line: start,
                added_lines: added_n,
            });
        }
    };

    for line in diff_text.lines() {
        match line {
            l if l.starts_with("diff --git ") => {
                flush_hunk(&cur_file, hunk_start, hunk_added, &mut oversized_hunks);
                hunk_added = 0;
                prev_dev_null = false;
            }
            l if l == "--- /dev/null" || l.starts_with("--- /dev/null\t") => {
                prev_dev_null = true;
            }
            l if l.starts_with("--- ") => {
                prev_dev_null = false;
            }
            l if l.starts_with("+++ ") => {
                let raw = l.trim_start_matches("+++ ").trim();
                let cleaned = raw.strip_prefix("b/").unwrap_or(raw);
                if cleaned != "/dev/null" && !cleaned.is_empty() {
                    cur_file = cleaned.to_string();
                    files_changed.insert(cur_file.clone());
                    if prev_dev_null {
                        new_files.insert(cur_file.clone());
                    }
                    if is_manifest_file(&cur_file) {
                        manifest_changes.insert(cur_file.clone());
                    }
                }
                prev_dev_null = false;
            }
            l if l.starts_with("@@ ") => {
                flush_hunk(&cur_file, hunk_start, hunk_added, &mut oversized_hunks);
                hunk_start = parse_hunk_new_start(l);
                hunk_added = 0;
            }
            l if l.starts_with('\\') => {}
            l if l.starts_with('+') => {
                lines_added = lines_added.saturating_add(1);
                hunk_added += 1;
                let code = &l[1..];
                if let Some(sym) = extract_symbol_name("fn", code)
                    && sym.len() >= MINIMALISM_DUPLICATE_SYMBOL_MIN_CHARS
                    && MINIMALISM_IGNORED_DUP_SYMBOLS
                        .binary_search(&sym.as_str())
                        .is_err()
                {
                    added_fn_names.insert(sym);
                }
            }
            l if l.starts_with('-') => {
                lines_deleted = lines_deleted.saturating_add(1);
            }
            _ => {}
        }
    }
    flush_hunk(&cur_file, hunk_start, hunk_added, &mut oversized_hunks);

    let mut duplicate_symbols = Vec::new();
    if !added_fn_names.is_empty() && workspace_dir.exists() {
        let (_, sym_map) = collect_workspace_symbol_map(workspace_dir);
        for name in &added_fn_names {
            if let Some(locs) = sym_map.get(name)
                && locs.len() >= 2
            {
                duplicate_symbols.push(DuplicateSymbolFinding {
                    symbol: name.clone(),
                    locations: locs.iter().take(6).cloned().collect(),
                });
            }
        }
    }

    let net_loc_delta = (lines_added as i64).saturating_sub(lines_deleted as i64);
    let mut findings = Vec::new();
    if !manifest_changes.is_empty() {
        findings.push(format!(
            "Dependency manifest touched ({}) — verify Rungs 2-5 (reuse/stdlib/native/existing dep) before adding packages.",
            manifest_changes.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    if !oversized_hunks.is_empty() {
        findings.push(format!(
            "{} oversized added hunk(s) (>= {} lines) — check if any block can reuse existing helpers or collapse.",
            oversized_hunks.len(),
            MINIMALISM_DIFF_LARGE_HUNK_LOC
        ));
    }
    if !duplicate_symbols.is_empty() {
        let names: Vec<&str> = duplicate_symbols
            .iter()
            .take(4)
            .map(|d| d.symbol.as_str())
            .collect();
        findings.push(format!(
            "Added symbol(s) also exist elsewhere in workspace ({}) — verify Rung 2 (Reuse).",
            names.join(", ")
        ));
    }
    if new_files.len() >= 2 {
        findings.push(format!(
            "{} new file(s) created — verify each file earns its own module boundary.",
            new_files.len()
        ));
    }
    if lines_added >= 100 && lines_deleted == 0 {
        findings.push(format!(
            "Purely additive diff (+{lines_added} / -0 LOC) — verify no superseded code was left behind."
        ));
    }

    DiffAuditReport {
        files_changed: files_changed.into_iter().collect(),
        new_files: new_files.into_iter().collect(),
        lines_added,
        lines_deleted,
        net_loc_delta,
        manifest_changes: manifest_changes.into_iter().collect(),
        oversized_hunks,
        duplicate_symbols,
        findings,
    }
}

// ─── Deferred Read-Only `code_minimalism` Tool ────────────────────────────────

fn ok_with_meta(content: String, meta: Value) -> ToolOutput {
    ToolOutput {
        content,
        is_error: false,
        diff: None,
        metadata: Some(meta),
    }
}

pub struct CodeMinimalismTool {
    bridge: Arc<dyn EconomyBridge>,
}

impl CodeMinimalismTool {
    pub fn new(bridge: Arc<dyn EconomyBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Tool for CodeMinimalismTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_CODE_MINIMALISM.into(),
            description: "Deterministic zero-regex Code Minimalism analyzer (7-rung Simplicity Ladder): audit git diffs for over-engineering (`audit_diff`), scan repo bloat & duplicate helpers (`audit_repo`), harvest comment-scoped shortcut debt markers (`debt_scan`), or inspect real net-LOC & Token Economy gain metrics (`gain`).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["audit_diff", "audit_repo", "debt_scan", "gain"],
                        "description": "Read-only minimalism analysis to execute."
                    },
                    "staged": {
                        "type": "boolean",
                        "description": "For action=audit_diff: inspect staged changes (`git diff --staged`) instead of working tree vs HEAD (default: false)."
                    }
                },
                "required": ["action"]
            }),
            risk: RiskLevel::Safe,
        }
    }

    fn toolset(&self) -> ToolSet {
        ToolSet::CodeIntel
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
            .unwrap_or("audit_diff");
        let Some(action) = CodeMinimalismAction::parse(raw_action) else {
            return Err(ZenkoError::Tool(format!(
                "Invalid action `{raw_action}` (expected audit_diff, audit_repo, debt_scan, or gain)"
            )));
        };

        let ws = ctx.workspace_dir.clone();
        match action {
            CodeMinimalismAction::AuditDiff => {
                let staged = args.get("staged").and_then(Value::as_bool).unwrap_or(false);
                let report = tokio::task::spawn_blocking(move || {
                    audit_git_diff_minimalism(&ws, staged)
                })
                .await
                .map_err(|e| ZenkoError::Tool(format!("audit_diff task failed: {e}")))?;

                let mut lines = vec![format!(
                    "Diff Minimalism Audit ({} file(s), {} new): +{} / -{} LOC (net {}{})",
                    report.files_changed.len(),
                    report.new_files.len(),
                    report.lines_added,
                    report.lines_deleted,
                    match report.net_loc_delta >= 0 {
                        true => "+",
                        false => "",
                    },
                    report.net_loc_delta,
                )];
                if report.findings.is_empty() {
                    lines.push("- No structural bloat signals detected in diff.".into());
                } else {
                    for f in &report.findings {
                        lines.push(format!("- {f}"));
                    }
                }
                for h in report.oversized_hunks.iter().take(6) {
                    lines.push(format!(
                        "  * hunk {}:L{} (+{} lines)",
                        h.path, h.start_line, h.added_lines
                    ));
                }
                for d in report.duplicate_symbols.iter().take(6) {
                    lines.push(format!(
                        "  * duplicate `{}` -> {}",
                        d.symbol,
                        d.locations.join(", ")
                    ));
                }
                Ok(ok_with_meta(
                    lines.join("\n"),
                    serde_json::to_value(&report).unwrap_or(Value::Null),
                ))
            }
            CodeMinimalismAction::AuditRepo => {
                let report = tokio::task::spawn_blocking(move || audit_workspace_minimalism(&ws))
                    .await
                    .map_err(|e| ZenkoError::Tool(format!("audit_repo task failed: {e}")))?;
                let _ = self
                    .bridge
                    .update_repo_debt_count(&ctx.workspace_dir, report.debt_count);

                let mut lines = vec![format!(
                    "Repo Minimalism Audit: {} source file(s), {} lines, {} shortcut debt marker(s)",
                    report.files_scanned, report.total_lines, report.debt_count
                )];
                if !report.large_files.is_empty() {
                    lines.push("Large files:".into());
                    for lf in report.large_files.iter().take(6) {
                        lines.push(format!("  - {} ({} lines)", lf.path, lf.lines));
                    }
                }
                if !report.long_functions.is_empty() {
                    lines.push("Long functions:".into());
                    for fn_item in report.long_functions.iter().take(6) {
                        lines.push(format!(
                            "  - {}:L{} ({} lines) {}",
                            fn_item.path, fn_item.line, fn_item.span_lines, fn_item.signature
                        ));
                    }
                }
                if !report.duplicate_symbols.is_empty() {
                    lines.push("Duplicate helper candidates:".into());
                    for dup in report.duplicate_symbols.iter().take(6) {
                        lines.push(format!(
                            "  - `{}` in {}",
                            dup.symbol,
                            dup.locations.join(", ")
                        ));
                    }
                }
                if !report.thin_wrappers.is_empty() {
                    lines.push("Thin wrapper candidates:".into());
                    for w in report.thin_wrappers.iter().take(6) {
                        lines.push(format!(
                            "  - {}:L{} [{} `{}` · {} lines]",
                            w.path, w.line, w.kind, w.name, w.span_lines
                        ));
                    }
                }
                Ok(ok_with_meta(
                    lines.join("\n"),
                    serde_json::to_value(&report).unwrap_or(Value::Null),
                ))
            }
            CodeMinimalismAction::DebtScan => {
                let report = tokio::task::spawn_blocking(move || scan_workspace_debt(&ws))
                    .await
                    .map_err(|e| ZenkoError::Tool(format!("debt_scan task failed: {e}")))?;
                let _ = self
                    .bridge
                    .update_repo_debt_count(&ctx.workspace_dir, report.total_found);

                let mut lines = vec![format!(
                    "Shortcut Debt Scan: {} marker(s) across {} source file(s){}",
                    report.total_found,
                    report.files_scanned,
                    match report.truncated {
                        true => " (truncated)",
                        false => "",
                    }
                )];
                for item in report.items.iter().take(25) {
                    lines.push(format!(
                        "- {}:L{} [{}] {}",
                        item.path, item.line, item.marker, item.text
                    ));
                }
                Ok(ok_with_meta(
                    lines.join("\n"),
                    serde_json::to_value(&report).unwrap_or(Value::Null),
                ))
            }
            CodeMinimalismAction::Gain => {
                let rep = self.bridge.economy_report(&ctx.workspace_dir)?;
                let mode = self.bridge.session_minimalism_mode(&ctx.session_id);
                let m = &rep.code_metrics;
                let total_churn = m.lines_added.saturating_add(m.lines_deleted);
                let deletion_ratio_pct = match total_churn > 0 {
                    true => ((m.lines_deleted as f64 / total_churn as f64) * 1000.0).round() / 10.0,
                    false => 0.0,
                };
                let gain = MinimalismGainReport {
                    repo_scope: rep.repo_scope.clone(),
                    mode,
                    metrics: m.clone(),
                    economy_summary: rep.summary.clone(),
                    deletion_ratio_pct,
                };
                let text = format!(
                    "Minimalism & Economy Gain ({scope}) · mode={mode}\n\
                     - Code Churn: +{added} / -{deleted} LOC (net {sign}{net}, deletion ratio {ratio:.1}%)\n\
                     - Activity: {turns} edit turn(s), {new_f} new file(s), {hints} bloat hint(s), {debt} debt marker(s)\n\
                     - Token Economy Net Saved: {tok} tok (~${usd:.4})",
                    scope = rep.repo_scope,
                    mode = mode.as_str(),
                    added = m.lines_added,
                    deleted = m.lines_deleted,
                    sign = match m.net_loc_delta >= 0 {
                        true => "+",
                        false => "",
                    },
                    net = m.net_loc_delta,
                    ratio = deletion_ratio_pct,
                    turns = m.turns_with_edits,
                    new_f = m.new_files_created,
                    hints = m.bloat_hints_issued,
                    debt = m.debt_markers_last_seen,
                    tok = rep.summary.net_tokens_saved,
                    usd = rep.summary.estimated_usd_saved,
                );
                Ok(ok_with_meta(
                    text,
                    serde_json::to_value(&gain).unwrap_or(Value::Null),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_diff_parser_counts_loc_new_files_manifests_and_large_hunks() {
        let diff = "\
diff --git a/Cargo.toml b/Cargo.toml
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -10,0 +11,1 @@
+some-crate = \"1.0\"
diff --git a/src/new_mod.rs b/src/new_mod.rs
--- /dev/null
+++ b/src/new_mod.rs
@@ -0,0 +1,4 @@
+pub fn compute_widget(x: i32) -> i32 {
+    x + 1
+}
+// done
diff --git a/src/old.rs b/src/old.rs
--- a/src/old.rs
+++ b/src/old.rs
@@ -5,2 +5,1 @@
-let a = 1;
-let b = 2;
+let a = 3;
";
        assert_eq!(parse_unified_diff_counts(diff), (6, 2, 1));
        let report = analyze_unified_diff(Path::new("/nonexistent"), diff);
        assert_eq!(report.lines_added, 6);
        assert_eq!(report.lines_deleted, 2);
        assert_eq!(report.net_loc_delta, 4);
        assert_eq!(report.new_files, vec!["src/new_mod.rs".to_string()]);
        assert_eq!(report.manifest_changes, vec!["Cargo.toml".to_string()]);
        assert!(report.findings.iter().any(|f| f.contains("Cargo.toml")));
    }

    #[test]
    fn comment_fsm_debt_scanner_ignores_string_literals_and_finds_real_comments() {
        let src = r#"
const MARKERS: &[&str] = &["DEBT:", "SHORTCUT:", "PONYTAIL:"];
fn run() {
    let s = "HACK: inside string literal should be ignored";
    // DEBT: replace linear scan with index once rows > 10k
    /* SHORTCUT: skips cache warm-up on cold start */
    let _ = (MARKERS, s);
}
"#;
        let mut items = Vec::new();
        let count = scan_file_debt_markers("src/lib.rs", src, "rs", &mut items, 20);
        assert_eq!(count, 2);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].marker, "DEBT:");
        assert_eq!(items[0].line, 5);
        assert!(items[0].text.contains("replace linear scan"));
        assert_eq!(items[1].marker, "SHORTCUT:");
        assert_eq!(items[1].line, 6);
    }

    #[test]
    fn repo_audit_detects_duplicates_thin_wrappers_and_debt_without_regex() {
        let dir = std::env::temp_dir().join(format!("zenko_min_test_{}", zenko_core::new_id()));
        std::fs::create_dir_all(dir.join("src")).expect("mkdir");
        std::fs::write(
            dir.join("src/a.rs"),
            "pub struct FooWrapper {\n    x: i32,\n}\n\npub fn normalize_payload(v: &str) -> String {\n    // PONYTAIL: inline helper for now\n    v.trim().to_string()\n}\n",
        )
        .expect("write a");
        std::fs::write(
            dir.join("src/b.rs"),
            "pub fn normalize_payload(v: &str) -> String {\n    v.trim().to_ascii_lowercase()\n}\n",
        )
        .expect("write b");

        let audit = audit_workspace_minimalism(&dir);
        assert_eq!(audit.files_scanned, 2);
        assert_eq!(audit.debt_count, 1);
        assert!(
            audit
                .duplicate_symbols
                .iter()
                .any(|d| d.symbol == "normalize_payload" && d.locations.len() == 2)
        );
        assert!(
            audit
                .thin_wrappers
                .iter()
                .any(|w| w.name == "FooWrapper" && w.kind == "struct")
        );

        let debt = scan_workspace_debt(&dir);
        assert_eq!(debt.total_found, 1);
        assert_eq!(debt.items[0].marker, "PONYTAIL:");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
