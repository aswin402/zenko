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
    CodeMinimalismAction, CommentBloatFinding, DOX_DRIFT_MAX_FINDINGS, DOX_DRIFT_MIN_ADDED_LINES,
    DebtItem, DebtScanReport, DiffAuditReport, DormantDependencyFinding, DoxDriftFinding,
    DuplicateSymbolFinding, EconomyBridge, LargeFileFinding, LongFunctionFinding,
    MINIMALISM_ABSTRACTION_MIN_NAME_CHARS, MINIMALISM_AUDIT_MAX_FILE_BYTES,
    MINIMALISM_AUDIT_MAX_FILES, MINIMALISM_AUDIT_TOP_N, MINIMALISM_COMMENT_BLOAT_MIN_LINES,
    MINIMALISM_COMMENT_BLOAT_MIN_RATIO_PCT, MINIMALISM_COMMENT_BLOCK_MAX_LINES,
    MINIMALISM_DEBT_MARKERS, MINIMALISM_DEBT_MAX_ITEMS, MINIMALISM_DEBT_TEXT_MAX_CHARS,
    MINIMALISM_DIFF_COMMENT_BLOAT_MIN_ADDED, MINIMALISM_DIFF_COMMENT_BLOAT_RATIO_PCT,
    MINIMALISM_DIFF_LARGE_HUNK_LOC, MINIMALISM_DUPLICATE_SYMBOL_MIN_CHARS,
    MINIMALISM_IGNORED_DORMANT_DEPS, MINIMALISM_IGNORED_DUP_SYMBOLS, MINIMALISM_LARGE_FILE_LINES,
    MINIMALISM_LONG_FN_LINES, MINIMALISM_MANIFEST_FILES, MINIMALISM_SOURCE_EXTS,
    MINIMALISM_WRAPPER_MAX_LINES, MINIMALISM_WRAPPER_SUFFIXES, MinimalismGainReport,
    OversizedHunkFinding, REPO_RULE_FILES, RepoAuditReport, Result, RiskLevel,
    SingleImplAbstractionFinding, TOOL_CODE_MINIMALISM, ThinWrapperFinding, Tool, ToolConcurrency,
    ToolContext, ToolExposure, ToolOutput, ToolSet, ToolSpec, ZenkoError,
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
/// Recognizes both `git diff` (`--- /dev/null`) and `similar::TextDiff` (`@@ -0,0 +`) new-file hunks.
/// Returns `(lines_added, lines_deleted, new_files_created)`.
pub fn parse_unified_diff_counts(diff: &str) -> (u64, u64, usize) {
    let mut added = 0u64;
    let mut deleted = 0u64;
    let mut new_files = 0usize;
    let mut prev_dev_null = false;
    let mut file_counted_new = false;

    for line in diff.lines() {
        match line {
            l if l.starts_with("diff --git ") => {
                prev_dev_null = false;
                file_counted_new = false;
            }
            l if l == "--- /dev/null" || l.starts_with("--- /dev/null\t") => {
                prev_dev_null = true;
                file_counted_new = false;
            }
            l if l.starts_with("--- ") => {
                prev_dev_null = false;
                file_counted_new = false;
            }
            l if l.starts_with("+++ ") => {
                if prev_dev_null && !l.contains("/dev/null") && !file_counted_new {
                    new_files = new_files.saturating_add(1);
                    file_counted_new = true;
                }
                prev_dev_null = false;
            }
            l if l.starts_with("@@ -0,0 +") => {
                if !file_counted_new {
                    new_files = new_files.saturating_add(1);
                    file_counted_new = true;
                }
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

#[derive(Debug, Clone, Copy, Default)]
struct FileCommentStats {
    comment_lines: usize,
    code_lines: usize,
    longest_block_lines: usize,
    longest_block_line: usize,
}

/// Deterministic comment-scoped byte/char FSM extracting shortcut/debt markers AND
/// single-pass comment-vs-code line/block metrics without regex.
fn scan_file_comments_and_debt(
    rel_path: &str,
    content: &str,
    ext: &str,
    out: &mut Vec<DebtItem>,
    max_items: usize,
) -> (usize, FileCommentStats) {
    let style = comment_style_for_ext(ext);
    let bytes = content.as_bytes();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut found = 0usize;
    let mut stats = FileCommentStats::default();
    let mut line_has_code = false;
    let mut line_has_comment = false;
    let mut cur_comment_run = 0usize;
    let mut cur_comment_run_start = 1usize;

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

    let finalize_line = |line_num: usize,
                         has_code: &mut bool,
                         has_comment: &mut bool,
                         run: &mut usize,
                         run_start: &mut usize,
                         st: &mut FileCommentStats| {
        match (*has_code, *has_comment) {
            (true, _) => {
                st.code_lines += 1;
                *run = 0;
            }
            (false, true) => {
                st.comment_lines += 1;
                if *run == 0 {
                    *run_start = line_num;
                }
                *run += 1;
                if *run > st.longest_block_lines {
                    st.longest_block_lines = *run;
                    st.longest_block_line = *run_start;
                }
            }
            (false, false) => {
                *run = 0;
            }
        }
        *has_code = false;
        *has_comment = false;
    };

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
                    finalize_line(
                        line,
                        &mut line_has_code,
                        &mut line_has_comment,
                        &mut cur_comment_run,
                        &mut cur_comment_run_start,
                        &mut stats,
                    );
                    line += 1;
                    i += 1;
                }
                (_, b'"', _) => {
                    line_has_code = true;
                    state = State::DoubleQuote;
                    i += 1;
                }
                (CommentStyle::Hash, b'\'', _) => {
                    line_has_code = true;
                    state = State::SingleQuote;
                    i += 1;
                }
                (CommentStyle::Slash, b'\'', _) => {
                    line_has_code = true;
                    // Distinguish Rust lifetime `'a` from char literal `'x'` / `'\n'`
                    let is_char_lit = bytes.get(i + 2) == Some(&b'\'')
                        || (bytes.get(i + 1) == Some(&b'\\') && bytes.get(i + 3) == Some(&b'\''));
                    if is_char_lit {
                        state = State::SingleQuote;
                    }
                    i += 1;
                }
                (CommentStyle::Slash, b'/', Some(b'/')) => {
                    line_has_comment = true;
                    state = State::LineComment;
                    comment_start = i + 2;
                    comment_line = line;
                    i += 2;
                }
                (CommentStyle::Slash, b'/', Some(b'*')) => {
                    line_has_comment = true;
                    state = State::BlockComment;
                    comment_start = i + 2;
                    comment_line = line;
                    i += 2;
                }
                (CommentStyle::Hash, b'#', _) => {
                    line_has_comment = true;
                    state = State::LineComment;
                    comment_start = i + 1;
                    comment_line = line;
                    i += 1;
                }
                _ => {
                    if !b.is_ascii_whitespace() {
                        line_has_code = true;
                    }
                    i += 1;
                }
            },
            State::DoubleQuote => match b {
                b'\\' if i + 1 < bytes.len() => {
                    line_has_code = true;
                    if bytes[i + 1] == b'\n' {
                        finalize_line(
                            line,
                            &mut line_has_code,
                            &mut line_has_comment,
                            &mut cur_comment_run,
                            &mut cur_comment_run_start,
                            &mut stats,
                        );
                        line += 1;
                    }
                    i += 2;
                }
                b'"' => {
                    line_has_code = true;
                    state = State::Code;
                    i += 1;
                }
                b'\n' => {
                    finalize_line(
                        line,
                        &mut line_has_code,
                        &mut line_has_comment,
                        &mut cur_comment_run,
                        &mut cur_comment_run_start,
                        &mut stats,
                    );
                    line += 1;
                    state = State::Code;
                    i += 1;
                }
                _ => {
                    if !b.is_ascii_whitespace() {
                        line_has_code = true;
                    }
                    i += 1;
                }
            },
            State::SingleQuote => match b {
                b'\\' if i + 1 < bytes.len() => {
                    line_has_code = true;
                    if bytes[i + 1] == b'\n' {
                        finalize_line(
                            line,
                            &mut line_has_code,
                            &mut line_has_comment,
                            &mut cur_comment_run,
                            &mut cur_comment_run_start,
                            &mut stats,
                        );
                        line += 1;
                    }
                    i += 2;
                }
                b'\'' => {
                    line_has_code = true;
                    state = State::Code;
                    i += 1;
                }
                b'\n' => {
                    finalize_line(
                        line,
                        &mut line_has_code,
                        &mut line_has_comment,
                        &mut cur_comment_run,
                        &mut cur_comment_run_start,
                        &mut stats,
                    );
                    line += 1;
                    state = State::Code;
                    i += 1;
                }
                _ => {
                    if !b.is_ascii_whitespace() {
                        line_has_code = true;
                    }
                    i += 1;
                }
            },
            State::LineComment => match b {
                b'\n' => {
                    if let Some(slice) = content.get(comment_start..i) {
                        check_comment(slice, comment_line);
                    }
                    finalize_line(
                        line,
                        &mut line_has_code,
                        &mut line_has_comment,
                        &mut cur_comment_run,
                        &mut cur_comment_run_start,
                        &mut stats,
                    );
                    line += 1;
                    state = State::Code;
                    i += 1;
                }
                _ => {
                    line_has_comment = true;
                    i += 1;
                }
            },
            State::BlockComment => match (b, bytes.get(i + 1).copied()) {
                (b'*', Some(b'/')) => {
                    line_has_comment = true;
                    if let Some(slice) = content.get(comment_start..i) {
                        check_comment(slice, comment_line);
                    }
                    state = State::Code;
                    i += 2;
                }
                (b'\n', _) => {
                    line_has_comment = true;
                    finalize_line(
                        line,
                        &mut line_has_code,
                        &mut line_has_comment,
                        &mut cur_comment_run,
                        &mut cur_comment_run_start,
                        &mut stats,
                    );
                    line += 1;
                    i += 1;
                }
                _ => {
                    if !b.is_ascii_whitespace() {
                        line_has_comment = true;
                    }
                    i += 1;
                }
            },
        }
    }
    if matches!(state, State::LineComment | State::BlockComment)
        && let Some(slice) = content.get(comment_start..bytes.len())
    {
        check_comment(slice, comment_line);
    }
    if line_has_code || line_has_comment {
        finalize_line(
            line,
            &mut line_has_code,
            &mut line_has_comment,
            &mut cur_comment_run,
            &mut cur_comment_run_start,
            &mut stats,
        );
    }
    (found, stats)
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
    scan_file_comments_and_debt(rel_path, content, ext, out, max_items).0
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

/// Returns `true` when an interface span declares at least one behavioral method signature (`foo(`)
/// rather than acting as a pure data/DTO shape (`interface Props { ... }`).
fn interface_declares_methods(lines: &[&str], start_line: usize, end_line: usize) -> bool {
    let from = start_line.saturating_sub(1).min(lines.len());
    let to = end_line.min(lines.len());
    for raw in &lines[from..to] {
        let t = raw.trim();
        if t.is_empty()
            || t.starts_with("//")
            || t.starts_with("/*")
            || t.starts_with('*')
            || t.contains("interface ")
        {
            continue;
        }
        let bytes = t.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() && (is_ident_byte(bytes[i]) || bytes[i] == b'?') {
            i += 1;
        }
        if i > 0 {
            let rest = t[i..].trim_start();
            if rest.starts_with('(') || rest.starts_with('<') {
                return true;
            }
            if let Some(after_colon) = rest.strip_prefix(':') {
                let rhs = after_colon.trim_start();
                if rhs.starts_with('(') && rhs.contains("=>") {
                    return true;
                }
            }
        }
    }
    false
}

/// Skips balanced `<...>` generic parameters starting at `idx`, returning the index after `>`.
fn skip_angle_generics(bytes: &[u8], start: usize) -> usize {
    if bytes.get(start) != Some(&b'<') {
        return start;
    }
    let mut depth = 0usize;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => depth += 1,
            b'>' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    i
}

/// Extracts the trailing identifier from a qualified type path (`foo::Bar<T>` or `pkg.Bar<T>` -> `Bar`).
fn trailing_type_ident(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let without_generics = trimmed.split('<').next().unwrap_or(trimmed).trim();
    let tail = without_generics
        .rsplit("::")
        .next()
        .unwrap_or(without_generics)
        .rsplit('.')
        .next()
        .unwrap_or(without_generics)
        .trim();
    let ident: String = tail
        .bytes()
        .take_while(|&b| is_ident_byte(b))
        .map(char::from)
        .collect();
    match ident.len() >= MINIMALISM_ABSTRACTION_MIN_NAME_CHARS {
        true => Some(ident),
        false => None,
    }
}

/// Zero-regex concrete implementation collector for Rust `impl Trait for Type` and
/// TS/JS/Java/Kotlin/C# `implements Foo, Bar` / `extends Base`.
fn collect_abstraction_impls(content: &str, ext: &str, counts: &mut BTreeMap<String, usize>) {
    match ext {
        "rs" => {
            for raw in content.lines() {
                let t = raw.trim();
                if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
                    continue;
                }
                let after_impl = t
                    .strip_prefix("unsafe impl")
                    .or_else(|| t.strip_prefix("impl"));
                let Some(rest) = after_impl else {
                    continue;
                };
                if !rest.starts_with(|c: char| c.is_ascii_whitespace() || c == '<') {
                    continue;
                }
                let rest = rest.trim_start();
                let bytes = rest.as_bytes();
                let after_gen = skip_angle_generics(bytes, 0);
                let body = rest.get(after_gen..).unwrap_or("").trim_start();
                if let Some(for_pos) = body.find(" for ")
                    && let Some(trait_name) = trailing_type_ident(&body[..for_pos])
                {
                    *counts.entry(trait_name).or_insert(0) += 1;
                }
            }
        }
        "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" | "java" | "kt" | "kts"
        | "cs" => {
            for raw in content.lines() {
                let t = raw.trim();
                if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
                    continue;
                }
                for kw in [" implements ", " extends "] {
                    if let Some(pos) = t.find(kw) {
                        let tail = &t[pos + kw.len()..];
                        let clause = tail.split('{').next().unwrap_or(tail);
                        let clause = clause.split(" implements ").next().unwrap_or(clause);
                        for seg in clause.split(',') {
                            if let Some(name) = trailing_type_ident(seg) {
                                *counts.entry(name).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn is_diff_comment_line(ext: &str, code: &str) -> bool {
    let t = code.trim();
    if t.is_empty() {
        return false;
    }
    match comment_style_for_ext(ext) {
        CommentStyle::Hash => t.starts_with('#'),
        CommentStyle::Slash => t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'),
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
    let mut comment_bloat: Vec<CommentBloatFinding> = Vec::new();
    let mut declared_abstractions: Vec<(String, usize, String, String)> = Vec::new();
    let mut impl_counts: BTreeMap<String, usize> = BTreeMap::new();
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
        let (file_debt, c_stats) =
            scan_file_comments_and_debt(&rel, &content, &ext, &mut scratch_debt, 0);
        debt_count += file_debt;
        collect_abstraction_impls(&content, &ext, &mut impl_counts);

        let non_blank = c_stats.comment_lines.saturating_add(c_stats.code_lines);
        if non_blank >= MINIMALISM_COMMENT_BLOAT_MIN_LINES {
            let ratio_pct = (c_stats.comment_lines.saturating_mul(100)) / non_blank.max(1);
            if ratio_pct >= MINIMALISM_COMMENT_BLOAT_MIN_RATIO_PCT
                || c_stats.longest_block_lines >= MINIMALISM_COMMENT_BLOCK_MAX_LINES
            {
                comment_bloat.push(CommentBloatFinding {
                    path: rel.clone(),
                    comment_lines: c_stats.comment_lines,
                    code_lines: c_stats.code_lines,
                    comment_ratio_pct: ratio_pct,
                    longest_block_lines: c_stats.longest_block_lines,
                    longest_block_line: c_stats.longest_block_line,
                });
            }
        }

        let outline = extract_outline(path, &content, 240);
        let content_lines: Vec<&str> = content.lines().collect();
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
            if name.len() >= MINIMALISM_ABSTRACTION_MIN_NAME_CHARS {
                let is_behavioral_abstraction = match item.kind {
                    "trait" => true,
                    "class" => item.signature.contains("abstract "),
                    "interface" => {
                        interface_declares_methods(&content_lines, item.line, item.end_line)
                    }
                    _ => false,
                };
                if is_behavioral_abstraction {
                    declared_abstractions.push((
                        rel.clone(),
                        item.line,
                        item.kind.to_string(),
                        name.clone(),
                    ));
                }
            }
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

    let mut single_impl_abstractions: Vec<SingleImplAbstractionFinding> = declared_abstractions
        .into_iter()
        .filter_map(|(path, line, kind, name)| {
            let impl_count = impl_counts.get(&name).copied().unwrap_or(0);
            match impl_count <= 1 {
                true => Some(SingleImplAbstractionFinding {
                    path,
                    line,
                    kind,
                    name,
                    impl_count,
                }),
                false => None,
            }
        })
        .collect();
    single_impl_abstractions.sort_by(|a, b| {
        a.impl_count
            .cmp(&b.impl_count)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.line.cmp(&b.line))
    });
    single_impl_abstractions.truncate(MINIMALISM_AUDIT_TOP_N);

    comment_bloat.sort_by(|a, b| {
        b.comment_ratio_pct
            .cmp(&a.comment_ratio_pct)
            .then_with(|| b.longest_block_lines.cmp(&a.longest_block_lines))
            .then_with(|| a.path.cmp(&b.path))
    });
    comment_bloat.truncate(MINIMALISM_AUDIT_TOP_N);

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

    let dormant_dependencies = scan_dormant_dependencies(workspace_dir);

    let report = RepoAuditReport {
        files_scanned,
        total_lines,
        large_files,
        long_functions,
        duplicate_symbols,
        thin_wrappers,
        single_impl_abstractions,
        comment_bloat,
        dormant_dependencies,
        debt_count,
    };
    (report, sym_locations)
}

/// Pure-Rust zero-regex TOML `[dependencies]` section key extractor for `Cargo.toml`.
fn parse_cargo_dependencies(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_deps = false;
    for raw_line in content.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let sec = line.trim();
            in_deps = sec == "[dependencies]"
                || (sec.starts_with("[target.") && sec.ends_with(".dependencies]"));
            continue;
        }
        if !in_deps {
            continue;
        }
        let Some((lhs, _)) = line.split_once('=') else {
            continue;
        };
        let key = lhs.trim().trim_matches('"').trim_matches('\'');
        if !key.is_empty()
            && !key.contains('.')
            && key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            deps.push(key.to_string());
        }
    }
    deps
}

/// Deterministic zero-regex Rust crate reference collector (`foo::`, `use foo`, `extern crate foo`).
fn collect_rust_crate_refs(content: &str, out: &mut BTreeSet<String>) {
    let bytes = content.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if is_ident_byte(bytes[i]) && !bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && is_ident_byte(bytes[i]) {
                i += 1;
            }
            let ident = &content[start..i];
            if bytes.get(i) == Some(&b':') && bytes.get(i + 1) == Some(&b':') {
                out.insert(ident.to_string());
                i += 2;
                continue;
            }
            if ident == "use" || ident == "crate" {
                let mut j = i;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                let next_start = j;
                while j < bytes.len() && is_ident_byte(bytes[j]) {
                    j += 1;
                }
                if j > next_start {
                    out.insert(content[next_start..j].to_string());
                }
            }
        } else {
            i += 1;
        }
    }
}

fn normalize_npm_specifier(spec: &str) -> Option<String> {
    let s = spec.trim();
    if s.is_empty()
        || s.starts_with('.')
        || s.starts_with('/')
        || s.starts_with("@/")
        || s.starts_with("node:")
        || s.starts_with("http://")
        || s.starts_with("https://")
    {
        return None;
    }
    match s.starts_with('@') {
        true => {
            let mut parts = s.split('/');
            let scope = parts.next()?;
            let pkg = parts.next()?;
            Some(format!("{scope}/{pkg}"))
        }
        false => s.split('/').next().map(String::from),
    }
}

/// Deterministic zero-regex JS/TS/CSS package specifier collector (`from "..."`, `import "..."`, `require("...")`, `@import "..."`).
fn collect_js_css_imports(content: &str, out: &mut BTreeSet<String>) {
    let bytes = content.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let matched = [
            "from ",
            "from\t",
            "from\"",
            "from'",
            "import ",
            "import\"",
            "import'",
            "require(",
            "@import ",
        ]
        .iter()
        .find_map(|kw| {
            content
                .get(i..i + kw.len())
                .filter(|s| *s == *kw)
                .map(|_| kw.len())
        });
        let Some(kw_len) = matched else {
            i += 1;
            continue;
        };
        let mut j = i + kw_len;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        // Handle `from"` / `import"` where quote was consumed in keyword
        let quote = match bytes.get(j).copied() {
            Some(q @ (b'"' | b'\'')) => {
                j += 1;
                Some(q)
            }
            _ => match bytes.get(i + kw_len - 1).copied() {
                Some(q @ (b'"' | b'\'')) => Some(q),
                _ => None,
            },
        };
        if let Some(q) = quote {
            let start = j;
            while j < bytes.len() && bytes[j] != q && bytes[j] != b'\n' {
                j += 1;
            }
            if let Some(raw) = content.get(start..j)
                && let Some(pkg) = normalize_npm_specifier(raw)
            {
                out.insert(pkg);
            }
        }
        i = j.max(i + 1);
    }
}

/// Zero-regex dormant runtime dependency scanner for `Cargo.toml` and `package.json` manifests.
pub fn scan_dormant_dependencies(workspace_dir: &Path) -> Vec<DormantDependencyFinding> {
    let mut findings = Vec::new();
    let mut manifests = Vec::new();

    for entry in WalkBuilder::new(workspace_dir)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .max_depth(Some(4))
        .build()
        .flatten()
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name == "Cargo.toml" || name == "package.json" {
            manifests.push(path.to_path_buf());
        }
    }

    for manifest_path in manifests {
        let Ok(raw) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        let rel_manifest = manifest_path
            .strip_prefix(workspace_dir)
            .unwrap_or(&manifest_path)
            .display()
            .to_string();
        let Some(manifest_dir) = manifest_path.parent() else {
            continue;
        };
        let file_name = manifest_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        match file_name {
            "Cargo.toml" => {
                let deps = parse_cargo_dependencies(&raw);
                if deps.is_empty() {
                    continue;
                }
                let mut used_refs: BTreeSet<String> = BTreeSet::new();
                let mut scanned = 0usize;
                for src_entry in WalkBuilder::new(manifest_dir)
                    .hidden(true)
                    .git_ignore(true)
                    .build()
                    .flatten()
                {
                    if scanned >= MINIMALISM_AUDIT_MAX_FILES {
                        break;
                    }
                    let p = src_entry.path();
                    if p.is_file()
                        && p.extension().and_then(|e| e.to_str()) == Some("rs")
                        && p.metadata()
                            .is_ok_and(|m| m.len() <= MINIMALISM_AUDIT_MAX_FILE_BYTES)
                        && let Ok(text) = std::fs::read_to_string(p)
                    {
                        scanned += 1;
                        collect_rust_crate_refs(&text, &mut used_refs);
                    }
                }
                if scanned == 0 {
                    continue;
                }
                for dep in deps {
                    if dep.ends_with("-sys")
                        || MINIMALISM_IGNORED_DORMANT_DEPS
                            .binary_search(&dep.as_str())
                            .is_ok()
                    {
                        continue;
                    }
                    let norm = dep.replace('-', "_");
                    if !used_refs.contains(&norm) {
                        findings.push(DormantDependencyFinding {
                            manifest: rel_manifest.clone(),
                            package: dep,
                        });
                    }
                }
            }
            "package.json" => {
                let Ok(parsed) = serde_json::from_str::<Value>(&raw) else {
                    continue;
                };
                let Some(deps_obj) = parsed.get("dependencies").and_then(Value::as_object) else {
                    continue;
                };
                if deps_obj.is_empty() {
                    continue;
                }
                let mut used_pkgs: BTreeSet<String> = BTreeSet::new();
                let mut scanned = 0usize;
                for src_entry in WalkBuilder::new(manifest_dir)
                    .hidden(true)
                    .git_ignore(true)
                    .build()
                    .flatten()
                {
                    if scanned >= MINIMALISM_AUDIT_MAX_FILES {
                        break;
                    }
                    let p = src_entry.path();
                    let ext = p
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|s| s.to_ascii_lowercase())
                        .unwrap_or_default();
                    if p.is_file()
                        && matches!(
                            ext.as_str(),
                            "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" | "css"
                        )
                        && p.metadata()
                            .is_ok_and(|m| m.len() <= MINIMALISM_AUDIT_MAX_FILE_BYTES)
                        && let Ok(text) = std::fs::read_to_string(p)
                    {
                        scanned += 1;
                        collect_js_css_imports(&text, &mut used_pkgs);
                    }
                }
                if scanned == 0 {
                    continue;
                }
                for pkg in deps_obj.keys() {
                    if pkg.starts_with("@types/")
                        || MINIMALISM_IGNORED_DORMANT_DEPS
                            .binary_search(&pkg.as_str())
                            .is_ok()
                    {
                        continue;
                    }
                    if !used_pkgs.contains(pkg) {
                        findings.push(DormantDependencyFinding {
                            manifest: rel_manifest.clone(),
                            package: pkg.clone(),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    findings.sort_by(|a, b| a.manifest.cmp(&b.manifest).then_with(|| a.package.cmp(&b.package)));
    findings.truncate(MINIMALISM_AUDIT_TOP_N);
    findings
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
/// When `staged == false`, also folds in newly created untracked files (`git ls-files --others --exclude-standard`).
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
    let untracked: Vec<String> = match staged {
        true => Vec::new(),
        false => Command::new("git")
            .args(["ls-files", "--others", "--exclude-standard"])
            .current_dir(workspace_dir)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .take(MINIMALISM_AUDIT_MAX_FILES)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default(),
    };
    analyze_unified_diff_with_untracked(workspace_dir, &diff_text, &untracked)
}

/// Deterministic zero-regex analyzer over any unified diff, cross-checked against workspace symbols.
pub fn analyze_unified_diff(workspace_dir: &Path, diff_text: &str) -> DiffAuditReport {
    analyze_unified_diff_with_untracked(workspace_dir, diff_text, &[])
}

fn analyze_unified_diff_with_untracked(
    workspace_dir: &Path,
    diff_text: &str,
    untracked_files: &[String],
) -> DiffAuditReport {
    let mut files_changed: BTreeSet<String> = BTreeSet::new();
    let mut new_files: BTreeSet<String> = BTreeSet::new();
    let mut manifest_changes: BTreeSet<String> = BTreeSet::new();
    let mut added_by_file: BTreeMap<String, u64> = BTreeMap::new();
    let mut oversized_hunks: Vec<OversizedHunkFinding> = Vec::new();
    let mut added_fn_names: BTreeSet<String> = BTreeSet::new();

    let mut cur_file = String::new();
    let mut cur_ext = String::new();
    let mut prev_dev_null = false;
    let mut hunk_start = 1usize;
    let mut hunk_added = 0usize;
    let mut lines_added = 0u64;
    let mut lines_deleted = 0u64;
    let mut added_comment_lines = 0u64;
    let mut added_code_lines = 0u64;

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
                    cur_ext = Path::new(&cur_file)
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.to_ascii_lowercase())
                        .unwrap_or_default();
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
                if l.starts_with("@@ -0,0 +") && !cur_file.is_empty() {
                    new_files.insert(cur_file.clone());
                }
                hunk_start = parse_hunk_new_start(l);
                hunk_added = 0;
            }
            l if l.starts_with('\\') => {}
            l if l.starts_with('+') => {
                lines_added = lines_added.saturating_add(1);
                if !cur_file.is_empty() {
                    *added_by_file.entry(cur_file.clone()).or_insert(0) += 1;
                }
                hunk_added += 1;
                let code = &l[1..];
                if MINIMALISM_SOURCE_EXTS.contains(&cur_ext.as_str()) {
                    if is_diff_comment_line(&cur_ext, code) {
                        added_comment_lines = added_comment_lines.saturating_add(1);
                    } else if !code.trim().is_empty() {
                        added_code_lines = added_code_lines.saturating_add(1);
                    }
                }
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

    let mut scratch_untracked_debt = Vec::new();
    for rel in untracked_files {
        if files_changed.contains(rel) {
            continue;
        }
        let full = workspace_dir.join(rel);
        let Ok(meta) = full.metadata() else {
            continue;
        };
        if !meta.is_file() || meta.len() > MINIMALISM_AUDIT_MAX_FILE_BYTES {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&full) else {
            continue;
        };
        files_changed.insert(rel.clone());
        new_files.insert(rel.clone());
        if is_manifest_file(rel) {
            manifest_changes.insert(rel.clone());
        }
        let line_count = content.lines().count();
        lines_added = lines_added.saturating_add(line_count as u64);
        *added_by_file.entry(rel.clone()).or_insert(0) += line_count as u64;
        if line_count >= MINIMALISM_DIFF_LARGE_HUNK_LOC {
            oversized_hunks.push(OversizedHunkFinding {
                path: rel.clone(),
                start_line: 1,
                added_lines: line_count,
            });
        }
        if is_source_file(&full) {
            let ext = full
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .unwrap_or_default();
            scratch_untracked_debt.clear();
            let (_, c_stats) =
                scan_file_comments_and_debt(rel, &content, &ext, &mut scratch_untracked_debt, 0);
            added_comment_lines = added_comment_lines.saturating_add(c_stats.comment_lines as u64);
            added_code_lines = added_code_lines.saturating_add(c_stats.code_lines as u64);
            let outline = extract_outline(&full, &content, 120);
            for item in &outline.entries {
                if item.kind == "fn"
                    && let Some(sym) = extract_symbol_name("fn", &item.signature)
                    && sym.len() >= MINIMALISM_DUPLICATE_SYMBOL_MIN_CHARS
                    && MINIMALISM_IGNORED_DUP_SYMBOLS
                        .binary_search(&sym.as_str())
                        .is_err()
                {
                    added_fn_names.insert(sym);
                }
            }
        }
    }

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
    let total_added_src = added_comment_lines.saturating_add(added_code_lines);
    if total_added_src >= MINIMALISM_DIFF_COMMENT_BLOAT_MIN_ADDED {
        let ratio = (added_comment_lines.saturating_mul(100)) / total_added_src.max(1);
        if ratio >= MINIMALISM_DIFF_COMMENT_BLOAT_RATIO_PCT {
            findings.push(format!(
                "Heavy comment/docstring ratio in added source lines ({ratio}%: {added_comment_lines} comment vs {added_code_lines} code) — trim prose that merely restates code."
            ));
        }
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

    let dox_drift = detect_dox_drift(
        workspace_dir,
        &files_changed,
        &new_files,
        &added_by_file,
    );
    if !dox_drift.is_empty() {
        let docs: Vec<&str> = dox_drift.iter().take(4).map(|d| d.doc_path.as_str()).collect();
        findings.push(format!(
            "DOX drift risk: {} nested repo-rule doc(s) untouched despite structural edits in their subtree ({}) — update local contracts if ownership/invariants changed.",
            dox_drift.len(),
            docs.join(", ")
        ));
    }

    DiffAuditReport {
        files_changed: files_changed.into_iter().collect(),
        new_files: new_files.into_iter().collect(),
        lines_added,
        lines_deleted,
        net_loc_delta,
        added_comment_lines,
        added_code_lines,
        manifest_changes: manifest_changes.into_iter().collect(),
        oversized_hunks,
        duplicate_symbols,
        dox_drift,
        findings,
    }
}

fn is_repo_rule_doc(rel_path: &str) -> bool {
    let base = rel_path.rsplit('/').next().unwrap_or(rel_path);
    REPO_RULE_FILES.contains(&base)
}

/// Walks ancestor directories strictly below `workspace_dir` for every structurally modified file
/// (new file, manifest edit, or `>= DOX_DRIFT_MIN_ADDED_LINES` added lines) to flag the nearest
/// owning nested `AGENTS.md`/`ZENKO.md` when that doc was not updated in the same diff.
fn detect_dox_drift(
    workspace_dir: &Path,
    files_changed: &BTreeSet<String>,
    new_files: &BTreeSet<String>,
    added_by_file: &BTreeMap<String, u64>,
) -> Vec<DoxDriftFinding> {
    if !workspace_dir.is_dir() {
        return Vec::new();
    }
    let normalized_changed: BTreeSet<String> = files_changed
        .iter()
        .map(|p| p.replace('\\', "/"))
        .collect();
    let mut by_doc: BTreeMap<String, (BTreeSet<String>, usize)> = BTreeMap::new();

    for rel in files_changed {
        let rel_norm = rel.replace('\\', "/");
        if is_repo_rule_doc(&rel_norm) {
            continue;
        }
        let is_new = new_files.contains(rel);
        let added = added_by_file.get(rel).copied().unwrap_or(0);
        if !is_new && !is_manifest_file(&rel_norm) && added < DOX_DRIFT_MIN_ADDED_LINES {
            continue;
        }

        let mut dir_opt = Path::new(&rel_norm).parent();
        let mut nearest_doc: Option<String> = None;
        while let Some(rel_dir) = dir_opt {
            if rel_dir.as_os_str().is_empty() {
                break;
            }
            let full_dir = workspace_dir.join(rel_dir);
            for rule_name in REPO_RULE_FILES {
                let candidate = full_dir.join(rule_name);
                if candidate.is_file() {
                    let doc_rel = rel_dir.join(rule_name).to_string_lossy().replace('\\', "/");
                    nearest_doc = Some(doc_rel);
                    break;
                }
            }
            if nearest_doc.is_some() {
                break;
            }
            dir_opt = rel_dir.parent();
        }

        if let Some(doc_rel) = nearest_doc
            && !normalized_changed.contains(&doc_rel)
        {
            let entry = by_doc.entry(doc_rel).or_default();
            if entry.0.insert(rel_norm) && is_new {
                entry.1 = entry.1.saturating_add(1);
            }
        }
    }

    by_doc
        .into_iter()
        .take(DOX_DRIFT_MAX_FINDINGS)
        .map(|(doc_path, (files, new_files_count))| DoxDriftFinding {
            doc_path,
            changed_files: files.into_iter().take(6).collect(),
            new_files_count,
        })
        .collect()
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

                let total_src_added = report
                    .added_code_lines
                    .saturating_add(report.added_comment_lines);
                let src_ratio_note = match total_src_added > 0 {
                    true => {
                        let ratio = (report.added_comment_lines.saturating_mul(100))
                            / total_src_added.max(1);
                        format!(
                            " · src: {} code / {} comment ({ratio}%)",
                            report.added_code_lines, report.added_comment_lines
                        )
                    }
                    false => String::new(),
                };
                let mut lines = vec![format!(
                    "Diff Minimalism Audit ({} file(s), {} new): +{} / -{} LOC (net {}{}){src_ratio_note}",
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
                for drift in report.dox_drift.iter().take(6) {
                    lines.push(format!(
                        "  * dox drift `{}` untouched (changed: {})",
                        drift.doc_path,
                        drift.changed_files.join(", ")
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
                if !report.single_impl_abstractions.is_empty() {
                    lines.push("Single-impl abstraction candidates:".into());
                    for abs in report.single_impl_abstractions.iter().take(6) {
                        lines.push(format!(
                            "  - {}:L{} [{} `{}` · {} impl(s)]",
                            abs.path, abs.line, abs.kind, abs.name, abs.impl_count
                        ));
                    }
                }
                if !report.comment_bloat.is_empty() {
                    lines.push("Comment/docstring bloat candidates:".into());
                    for cb in report.comment_bloat.iter().take(6) {
                        lines.push(format!(
                            "  - {}:L{} [{}% comments ({}/{}L) · max block {}L]",
                            cb.path,
                            cb.longest_block_line.max(1),
                            cb.comment_ratio_pct,
                            cb.comment_lines,
                            cb.comment_lines + cb.code_lines,
                            cb.longest_block_lines
                        ));
                    }
                }
                if !report.dormant_dependencies.is_empty() {
                    lines.push("Dormant dependency candidates:".into());
                    for dep in report.dormant_dependencies.iter().take(8) {
                        lines.push(format!("  - `{}` ({})", dep.package, dep.manifest));
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
                     - Token Economy Net Saved: {tok} tok (~${usd:.4}) [Read-Less +{rl} · Output-Dedup +{od} ({od_hits} hits) · CCR Recoveries {ccr}]",
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
                    rl = rep.summary.read_less_tokens_saved,
                    od = rep.summary.output_dedup_tokens_saved,
                    od_hits = rep.summary.output_dedup_hits,
                    ccr = rep.summary.ccr_recoveries,
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

    #[test]
    fn similar_new_file_diff_and_untracked_git_files_are_counted_accurately() {
        // `fs_tools::unified_diff` emits `--- a/<path>` + `@@ -0,0 +1,N @@` (not `--- /dev/null`)
        let write_file_diff = crate::fs_tools::unified_diff(
            "src/fresh.rs",
            "",
            "pub fn fresh_helper() -> i32 {\n    42\n}\n",
        );
        assert_eq!(parse_unified_diff_counts(&write_file_diff), (3, 0, 1));
        let rep = analyze_unified_diff(Path::new("/nonexistent"), &write_file_diff);
        assert_eq!(rep.new_files, vec!["src/fresh.rs".to_string()]);
        assert_eq!(rep.lines_added, 3);

        // Real temporary git repo with a brand-new untracked file
        let dir = std::env::temp_dir().join(format!("zenko_min_git_{}", zenko_core::new_id()));
        std::fs::create_dir_all(dir.join("src")).expect("mkdir");
        let _ = Command::new("git").args(["init", "-q"]).current_dir(&dir).status();
        std::fs::write(dir.join("src/base.rs"), "pub fn base() {}\n").expect("base");
        let _ = Command::new("git").args(["add", "."]).current_dir(&dir).status();
        let _ = Command::new("git")
            .args([
                "-c",
                "user.email=test@zenko.local",
                "-c",
                "user.name=Zenko",
                "commit",
                "-q",
                "-m",
                "init",
            ])
            .current_dir(&dir)
            .status();
        std::fs::write(
            dir.join("src/untracked.rs"),
            "pub fn new_untracked_fn() {\n    let x = 1;\n    let _ = x;\n}\n",
        )
        .expect("untracked");

        let diff_rep = audit_git_diff_minimalism(&dir, false);
        assert!(diff_rep.new_files.contains(&"src/untracked.rs".to_string()));
        assert_eq!(diff_rep.lines_added, 4);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dormant_dependency_scanner_flags_unused_cargo_and_npm_packages_without_regex() {
        let dir = std::env::temp_dir().join(format!("zenko_min_deps_{}", zenko_core::new_id()));
        std::fs::create_dir_all(dir.join("src")).expect("mkdir rs");
        std::fs::create_dir_all(dir.join("web/src")).expect("mkdir web");

        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"\nunused-crate = \"2\"\nlibfoo-sys = \"0.1\"\n",
        )
        .expect("cargo toml");
        std::fs::write(
            dir.join("src/lib.rs"),
            "use serde::Serialize;\npub fn ok() { let _ = serde::ser::Impossible::<(), ()>::from; }\n",
        )
        .expect("lib.rs");

        std::fs::write(
            dir.join("web/package.json"),
            r#"{"dependencies":{"react":"^19","unused-pkg":"^1","@scope/used":"^2","@scope/dormant":"^1","tailwindcss":"^4"}}"#,
        )
        .expect("pkg json");
        std::fs::write(
            dir.join("web/src/App.tsx"),
            "import React from 'react'\nimport { run } from \"@scope/used/sub\"\nexport const App = () => run(React)\n",
        )
        .expect("app.tsx");

        let audit = audit_workspace_minimalism(&dir);
        let flagged: Vec<&str> = audit
            .dormant_dependencies
            .iter()
            .map(|d| d.package.as_str())
            .collect();
        assert!(flagged.contains(&"unused-crate"), "expected unused-crate in {flagged:?}");
        assert!(flagged.contains(&"unused-pkg"), "expected unused-pkg in {flagged:?}");
        assert!(flagged.contains(&"@scope/dormant"), "expected @scope/dormant in {flagged:?}");
        assert!(!flagged.contains(&"serde"));
        assert!(!flagged.contains(&"libfoo-sys"));
        assert!(!flagged.contains(&"react"));
        assert!(!flagged.contains(&"@scope/used"));
        assert!(!flagged.contains(&"tailwindcss"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn single_impl_abstractions_and_comment_bloat_are_detected_without_false_positives() {
        let dir = std::env::temp_dir().join(format!("zenko_min_chisle_{}", zenko_core::new_id()));
        std::fs::create_dir_all(dir.join("src")).expect("mkdir");

        std::fs::write(
            dir.join("src/traits.rs"),
            "pub trait SingleRunner {\n    fn run(&self);\n}\n\
             pub struct OnlyOne;\n\
             impl SingleRunner for OnlyOne {\n    fn run(&self) {}\n}\n\
             pub trait MultiRunner {\n    fn run(&self);\n}\n\
             pub struct Alpha;\n\
             pub struct Beta;\n\
             impl MultiRunner for Alpha {\n    fn run(&self) {}\n}\n\
             impl MultiRunner for Beta {\n    fn run(&self) {}\n}\n",
        )
        .expect("traits.rs");

        std::fs::write(
            dir.join("src/ui.ts"),
            "export interface PureDtoProps {\n  id: string;\n  label: string;\n}\n\
             export interface BehavioralPort {\n  execute(id: string): Promise<void>;\n}\n",
        )
        .expect("ui.ts");

        // Heavily over-commented source file (>= 40 lines, > 50% comments, 16-line prose block)
        let mut padded = String::new();
        for i in 0..18 {
            padded.push_str(&format!("// verbose narrative comment line {i} restating obvious code\n"));
        }
        for i in 0..24 {
            padded.push_str(&format!("pub fn step_{i}() -> usize {{ {i} }}\n"));
        }
        std::fs::write(dir.join("src/padded.rs"), padded).expect("padded.rs");

        let audit = audit_workspace_minimalism(&dir);
        let abs_names: Vec<&str> = audit
            .single_impl_abstractions
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert!(abs_names.contains(&"SingleRunner"), "got {abs_names:?}");
        assert!(abs_names.contains(&"BehavioralPort"), "got {abs_names:?}");
        assert!(!abs_names.contains(&"MultiRunner"), "2-impl trait must not trigger");
        assert!(!abs_names.contains(&"PureDtoProps"), "pure DTO interface must not trigger");

        assert!(
            audit
                .comment_bloat
                .iter()
                .any(|c| c.path.ends_with("padded.rs") && c.longest_block_lines >= 18),
            "expected padded.rs comment bloat in {:?}",
            audit.comment_bloat
        );

        // Diff-level comment bloat check
        let mut diff = String::from("diff --git a/src/new.rs b/src/new.rs\n--- /dev/null\n+++ b/src/new.rs\n@@ -0,0 +1,36 @@\n");
        for i in 0..20 {
            diff.push_str(&format!("+// explanatory prose line {i}\n"));
        }
        for i in 0..16 {
            diff.push_str(&format!("+let v_{i} = {i};\n"));
        }
        let diff_rep = analyze_unified_diff(&dir, &diff);
        assert_eq!(diff_rep.added_comment_lines, 20);
        assert_eq!(diff_rep.added_code_lines, 16);
        assert!(
            diff_rep
                .findings
                .iter()
                .any(|f| f.contains("Heavy comment/docstring ratio")),
            "expected comment bloat diff finding in {:?}",
            diff_rep.findings
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn audit_diff_detects_nested_dox_drift_and_clears_when_doc_updated() {
        let dir = std::env::temp_dir().join(format!("zenko_dox_drift_{}", zenko_core::new_id()));
        std::fs::create_dir_all(dir.join("crates/api/src")).expect("mkdir");
        std::fs::write(
            dir.join("crates/api/AGENTS.md"),
            "# API\n\n## Purpose\nHandles API routes.\n",
        )
        .expect("agents");

        let diff_without_doc = "diff --git a/crates/api/src/routes.rs b/crates/api/src/routes.rs\n\
--- /dev/null\n\
+++ b/crates/api/src/routes.rs\n\
@@ -0,0 +1,4 @@\n\
+pub fn health() {}\n\
+pub fn metrics() {}\n\
+pub fn status() {}\n\
+pub fn ready() {}\n";

        let rep = analyze_unified_diff(&dir, diff_without_doc);
        assert_eq!(rep.dox_drift.len(), 1);
        assert_eq!(rep.dox_drift[0].doc_path, "crates/api/AGENTS.md");
        assert_eq!(rep.dox_drift[0].new_files_count, 1);
        assert!(rep.findings.iter().any(|f| f.contains("DOX drift risk")));

        let diff_with_doc = format!(
            "{diff_without_doc}diff --git a/crates/api/AGENTS.md b/crates/api/AGENTS.md\n\
--- a/crates/api/AGENTS.md\n\
+++ b/crates/api/AGENTS.md\n\
@@ -3,0 +4,1 @@\n\
+- Added routes.rs endpoints.\n"
        );
        let clean = analyze_unified_diff(&dir, &diff_with_doc);
        assert!(clean.dox_drift.is_empty(), "expected zero drift once doc updated: {:?}", clean.dox_drift);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
