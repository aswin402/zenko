//! Zero-Dependency, Zero-Regex Pure-Rust Structural Code Outline Extractor.
//!
//! Powered by UTF-8-safe deterministic byte/token/brace/indentation state machines and
//! declarative keyword tables (zero regex, zero `if/else` ladders, zero hardcoded magic caps)
//! returning hash-anchored symbol spans (`line:hash| [kind] signature (Lstart..Lend)`).

use crate::fs_tools::line_hash;
use std::path::Path;
use zenko_core::{OUTLINE_MAX_BRACE_DEPTH, OUTLINE_MAX_ENTRIES, OUTLINE_MAX_SIGNATURE_CHARS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineSyntax {
    BraceRust,
    BraceJsTs,
    BraceGo,
    BraceCLike,
    IndentPython,
    MarkupMarkdown,
    ConfigToml,
}

const OUTLINE_EXT_TABLE: &[(&[&str], OutlineSyntax)] = &[
    (&["rs"], OutlineSyntax::BraceRust),
    (
        &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"],
        OutlineSyntax::BraceJsTs,
    ),
    (&["go"], OutlineSyntax::BraceGo),
    (
        &[
            "c", "h", "cc", "cpp", "cxx", "hpp", "hh", "java", "cs", "kt", "kts", "swift",
        ],
        OutlineSyntax::BraceCLike,
    ),
    (&["py", "pyi"], OutlineSyntax::IndentPython),
    (&["md", "mdx", "markdown"], OutlineSyntax::MarkupMarkdown),
    (&["toml"], OutlineSyntax::ConfigToml),
];

const RUST_MODIFIER_PREFIXES: &[&str] = &[
    "default ",
    "async ",
    "unsafe ",
    "extern \"C\" ",
    "extern ",
];

const RUST_DECL_PREFIXES: &[(&str, &str)] = &[
    ("fn ", "fn"),
    ("const fn ", "fn"),
    ("struct ", "struct"),
    ("enum ", "enum"),
    ("trait ", "trait"),
    ("impl ", "impl"),
    ("impl<", "impl"),
    ("mod ", "mod"),
    ("macro_rules!", "macro"),
];

const JS_MODIFIER_PREFIXES: &[&str] = &[
    "export ",
    "default ",
    "declare ",
    "abstract ",
    "async ",
    "public ",
    "private ",
    "protected ",
    "static ",
    "readonly ",
];

const JS_DECL_PREFIXES: &[(&str, &str)] = &[
    ("function ", "fn"),
    ("function*", "fn"),
    ("class ", "class"),
    ("interface ", "interface"),
    ("enum ", "enum"),
    ("namespace ", "namespace"),
];

const PYTHON_DECL_PREFIXES: &[(&str, &str)] = &[
    ("class ", "class"),
    ("def ", "fn"),
    ("async def ", "fn"),
];

const CLIKE_KEYWORD_KINDS: &[(&str, &str)] = &[
    ("class", "class"),
    ("struct", "struct"),
    ("enum", "enum"),
    ("interface", "interface"),
    ("protocol", "interface"),
    ("namespace", "namespace"),
    ("fun", "fn"),
    ("func", "fn"),
    ("trait", "type"),
];

const CONTROL_FLOW_HEADS: &[&str] = &[
    "if", "for", "while", "switch", "catch", "else", "return", "throw", "new",
];

pub fn detect_outline_syntax(path: &Path) -> Option<OutlineSyntax> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    OUTLINE_EXT_TABLE
        .iter()
        .find(|(exts, _)| exts.iter().any(|candidate| *candidate == ext))
        .map(|(_, syntax)| *syntax)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    pub line: usize,
    pub end_line: usize,
    pub hash: String,
    pub kind: &'static str,
    pub depth: usize,
    pub signature: String,
}

#[derive(Debug, Clone)]
pub struct OutlineReport {
    pub syntax: Option<OutlineSyntax>,
    pub total_lines: usize,
    pub entries: Vec<OutlineEntry>,
    pub truncated: bool,
}

pub fn extract_outline(path: &Path, content: &str, max_entries: usize) -> OutlineReport {
    let syntax = detect_outline_syntax(path);
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    let cap = max_entries.clamp(1, OUTLINE_MAX_ENTRIES);

    let all_entries = match syntax {
        Some(
            s @ (OutlineSyntax::BraceRust
            | OutlineSyntax::BraceJsTs
            | OutlineSyntax::BraceGo
            | OutlineSyntax::BraceCLike),
        ) => extract_brace_outline(&lines, s),
        Some(OutlineSyntax::IndentPython) => extract_python_outline(&lines),
        Some(OutlineSyntax::MarkupMarkdown) => extract_markdown_outline(&lines),
        Some(OutlineSyntax::ConfigToml) => extract_toml_outline(&lines),
        None => Vec::new(),
    };

    let truncated = all_entries.len() > cap;
    let entries = all_entries.into_iter().take(cap).collect();
    OutlineReport {
        syntax,
        total_lines,
        entries,
        truncated,
    }
}

pub fn format_outline_report(req_path: &str, report: &OutlineReport) -> String {
    if report.entries.is_empty() {
        return format!(
            "File: {req_path} ({} lines) — no structural symbols detected (use `mode=\"lines\"` to inspect raw content).",
            report.total_lines
        );
    }
    let mut out = format!(
        "File Outline: {req_path} ({} lines, {} symbol(s){})\n",
        report.total_lines,
        report.entries.len(),
        match report.truncated {
            true => ", truncated",
            false => "",
        }
    );
    for entry in &report.entries {
        let indent = "  ".repeat(entry.depth.min(4));
        let span = match entry.end_line > entry.line {
            true => format!(" (L{}..L{})", entry.line, entry.end_line),
            false => String::new(),
        };
        out.push_str(&format!(
            "{}:{}| {indent}[{}] {}{span}\n",
            entry.line, entry.hash, entry.kind, entry.signature
        ));
    }
    out
}

/// Deterministic zero-regex identifier extractor for an `OutlineEntry` signature.
/// Extracts ASCII identifier tokens (e.g., `pub async fn foo_bar<T>(...)` -> `["pub", "async", "fn", "foo_bar", "T"]`)
/// so `read_file(symbol=...)` can rank exact identifier hits ahead of loose substring hits.
fn signature_identifier_tokens(sig: &str) -> Vec<&str> {
    let bytes = sig.as_bytes();
    let mut tokens = Vec::new();
    let mut start: Option<usize> = None;
    for (idx, &b) in bytes.iter().enumerate() {
        let is_ident = b.is_ascii_alphanumeric() || b == b'_';
        match (start, is_ident) {
            (None, true) => start = Some(idx),
            (Some(s), false) => {
                tokens.push(&sig[s..idx]);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        tokens.push(&sig[s..]);
    }
    tokens
}

/// Finds matching structural outline entries for one or more requested symbol names using
/// deterministic ranked matching:
/// 1. Exact identifier match (case-sensitive)
/// 2. Case-insensitive identifier match
/// 3. Case-insensitive signature substring fallback
/// Results are deduplicated by `(line, end_line)` and sorted by ascending start line.
pub fn find_outline_symbol_spans<'a>(
    report: &'a OutlineReport,
    queries: &[String],
) -> Vec<&'a OutlineEntry> {
    let mut matched: Vec<&'a OutlineEntry> = Vec::new();
    let mut seen: Vec<(usize, usize)> = Vec::new();

    for raw_q in queries {
        let q = raw_q.trim();
        if q.is_empty() {
            continue;
        }
        let q_lower = q.to_ascii_lowercase();

        let exact: Vec<&'a OutlineEntry> = report
            .entries
            .iter()
            .filter(|e| signature_identifier_tokens(&e.signature).iter().any(|tok| *tok == q))
            .collect();

        let picked = match exact.is_empty() {
            false => exact,
            true => {
                let ci_ident: Vec<&'a OutlineEntry> = report
                    .entries
                    .iter()
                    .filter(|e| {
                        signature_identifier_tokens(&e.signature)
                            .iter()
                            .any(|tok| tok.eq_ignore_ascii_case(q))
                    })
                    .collect();
                match ci_ident.is_empty() {
                    false => ci_ident,
                    true => report
                        .entries
                        .iter()
                        .filter(|e| e.signature.to_ascii_lowercase().contains(&q_lower))
                        .collect(),
                }
            }
        };

        for entry in picked {
            let key = (entry.line, entry.end_line);
            if !seen.contains(&key) {
                seen.push(key);
                matched.push(entry);
            }
        }
    }

    matched.sort_by_key(|e| (e.line, e.end_line));
    matched
}

// ─── Brace-Scoped Language Scanner (Rust / TS / JS / Go / C-Like) ─────────────

#[derive(Default)]
struct BraceLexState {
    in_block_comment: bool,
    in_backtick_string: bool,
    rust_raw_hashes: Option<usize>,
}

struct OpenBraceSymbol {
    entry_idx: usize,
    target_depth: i32,
    pending_open: bool,
}

fn extract_brace_outline(lines: &[&str], syntax: OutlineSyntax) -> Vec<OutlineEntry> {
    let mut entries: Vec<OutlineEntry> = Vec::new();
    let mut open_stack: Vec<OpenBraceSymbol> = Vec::new();
    let mut lex = BraceLexState::default();
    let mut brace_depth: i32 = 0;

    for (idx, &raw_line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let was_in_comment = lex.in_block_comment;
        let (opens, closes, code_prefix) = scan_brace_line(raw_line, syntax, &mut lex);
        let trimmed = code_prefix.trim();

        if !was_in_comment && !trimmed.is_empty()
            && let Some((kind, sig)) = classify_brace_declaration(trimmed, syntax, brace_depth)
        {
            let entry_idx = entries.len();
            let nesting = open_stack.len();
            entries.push(OutlineEntry {
                line: line_no,
                end_line: line_no,
                hash: line_hash(raw_line),
                kind,
                depth: nesting,
                signature: compact_signature(&sig),
            });

            let has_semicolon_only = opens == 0 && trimmed.ends_with(';');
            if !has_semicolon_only {
                open_stack.push(OpenBraceSymbol {
                    entry_idx,
                    target_depth: brace_depth,
                    pending_open: opens == 0,
                });
            }
        }

        if opens > 0 {
            for sym in open_stack.iter_mut().rev() {
                if sym.pending_open {
                    sym.pending_open = false;
                }
            }
        }

        brace_depth = (brace_depth + opens - closes).max(0);

        while let Some(top) = open_stack.last() {
            let ready_to_close = match top.pending_open {
                true => trimmed.ends_with(';'),
                false => brace_depth <= top.target_depth,
            };
            match ready_to_close {
                true => {
                    if let Some(s) = open_stack.pop() {
                        entries[s.entry_idx].end_line = line_no;
                    }
                }
                false => break,
            }
        }
    }

    let final_line = lines.len().max(1);
    for sym in open_stack {
        entries[sym.entry_idx].end_line = final_line;
    }
    entries
}

/// Detects Rust raw string starts (`r#"..."#`, `br#"..."#`, `cr#"..."#`) at byte index `i`,
/// returning `Some((hash_count, first_body_byte_idx))` when a valid prefix begins at `i`.
fn match_rust_raw_string_start(bytes: &[u8], i: usize) -> Option<(usize, usize)> {
    let len = bytes.len();
    let prev_ident = i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
    if prev_ident {
        return None;
    }
    let r_pos = match bytes.get(i) {
        Some(b'r') => i,
        Some(b'b' | b'c') if bytes.get(i + 1) == Some(&b'r') => i + 1,
        _ => return None,
    };
    let mut j = r_pos + 1;
    while j < len && bytes[j] == b'#' {
        j += 1;
    }
    match bytes.get(j) {
        Some(b'"') => Some((j - (r_pos + 1), j + 1)),
        _ => None,
    }
}

/// UTF-8-safe single-line lexer updating multi-line comment/raw-string state, returning
/// `(open_braces, close_braces, comment_stripped_code_text)`.
fn scan_brace_line(
    line: &str,
    syntax: OutlineSyntax,
    state: &mut BraceLexState,
) -> (i32, i32, String) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut opens = 0i32;
    let mut closes = 0i32;
    let mut code_bytes: Vec<u8> = Vec::with_capacity(len);
    let mut in_double = false;
    let mut in_single = false;

    while i < len {
        if let Some(hashes) = state.rust_raw_hashes {
            let is_end = bytes[i] == b'"'
                && i + hashes < len
                && bytes[i + 1..=i + hashes].iter().all(|&b| b == b'#');
            match is_end {
                true => {
                    state.rust_raw_hashes = None;
                    i += 1 + hashes;
                }
                false => i += 1,
            }
            continue;
        }
        if state.in_block_comment {
            match (bytes[i], bytes.get(i + 1)) {
                (b'*', Some(b'/')) => {
                    state.in_block_comment = false;
                    i += 2;
                }
                _ => i += 1,
            }
            continue;
        }
        if state.in_backtick_string {
            match bytes[i] {
                b'\\' if i + 1 < len => i += 2,
                b'`' => {
                    state.in_backtick_string = false;
                    i += 1;
                }
                _ => i += 1,
            }
            continue;
        }
        if in_double {
            match bytes[i] {
                b'\\' if i + 1 < len => i += 2,
                b'"' => {
                    in_double = false;
                    i += 1;
                }
                _ => i += 1,
            }
            continue;
        }
        if in_single {
            match bytes[i] {
                b'\\' if i + 1 < len => i += 2,
                b'\'' => {
                    in_single = false;
                    i += 1;
                }
                _ => i += 1,
            }
            continue;
        }

        // Line comment `//` or block comment `/*`
        match (bytes[i], bytes.get(i + 1)) {
            (b'/', Some(b'/')) => break,
            (b'/', Some(b'*')) => {
                state.in_block_comment = true;
                i += 2;
                continue;
            }
            _ => {}
        }

        // Rust raw string / raw byte string / raw C string (`r#"..."#`, `br#"..."#`, `cr#"..."#`)
        if syntax == OutlineSyntax::BraceRust
            && let Some((hashes, next_i)) = match_rust_raw_string_start(bytes, i)
        {
            state.rust_raw_hashes = Some(hashes);
            i = next_i;
            continue;
        }

        match bytes[i] {
            b'"' => {
                in_double = true;
                code_bytes.push(b'"');
                i += 1;
            }
            b'\'' if syntax != OutlineSyntax::BraceRust => {
                in_single = true;
                code_bytes.push(b'\'');
                i += 1;
            }
            b'\'' if syntax == OutlineSyntax::BraceRust => {
                // Distinguish Rust char literal `'x'` / `'\n'` from lifetime `'a`
                let is_char_lit = (i + 2 < len && bytes[i + 2] == b'\'')
                    || (i + 3 < len && bytes[i + 1] == b'\\' && bytes[i + 3] == b'\'');
                if is_char_lit {
                    in_single = true;
                }
                code_bytes.push(b'\'');
                i += 1;
            }
            b'`' if matches!(syntax, OutlineSyntax::BraceJsTs | OutlineSyntax::BraceGo) => {
                state.in_backtick_string = true;
                code_bytes.push(b'`');
                i += 1;
            }
            b'{' => {
                opens += 1;
                code_bytes.push(b'{');
                i += 1;
            }
            b'}' => {
                closes += 1;
                code_bytes.push(b'}');
                i += 1;
            }
            b => {
                code_bytes.push(b);
                i += 1;
            }
        }
    }

    let code = String::from_utf8_lossy(&code_bytes).into_owned();
    (opens, closes, code)
}

fn classify_brace_declaration(
    trimmed: &str,
    syntax: OutlineSyntax,
    depth: i32,
) -> Option<(&'static str, String)> {
    match syntax {
        OutlineSyntax::BraceRust => classify_rust_decl(trimmed, depth),
        OutlineSyntax::BraceJsTs => classify_js_ts_decl(trimmed, depth),
        OutlineSyntax::BraceGo => classify_go_decl(trimmed, depth),
        OutlineSyntax::BraceCLike => classify_c_like_decl(trimmed, depth),
        _ => None,
    }
}

fn strip_prefix_table<'a>(mut cur: &'a str, prefixes: &[&str]) -> &'a str {
    while let Some(rest) = prefixes.iter().find_map(|p| cur.strip_prefix(p)) {
        cur = rest.trim_start();
    }
    cur
}

fn strip_rust_visibility(s: &str) -> &str {
    let cur = s.trim_start();
    let after_pub = match cur.strip_prefix("pub") {
        Some(rest) => match rest.strip_prefix('(') {
            Some(inside) => match inside.find(')') {
                Some(close) => inside[close + 1..].trim_start(),
                None => cur,
            },
            None => match rest.starts_with(char::is_whitespace) {
                true => rest.trim_start(),
                false => cur,
            },
        },
        None => cur,
    };
    strip_prefix_table(after_pub, RUST_MODIFIER_PREFIXES)
}

fn classify_rust_decl(trimmed: &str, depth: i32) -> Option<(&'static str, String)> {
    if depth > OUTLINE_MAX_BRACE_DEPTH || trimmed.starts_with('#') || trimmed.starts_with("use ") {
        return None;
    }
    let core = strip_rust_visibility(trimmed);
    if let Some(&(_, kind)) = RUST_DECL_PREFIXES.iter().find(|&&(p, _)| core.starts_with(p)) {
        return Some((kind, trimmed.to_string()));
    }
    match (depth, core.starts_with("type "), core.starts_with("const ") && core.contains(':')) {
        (0, true, _) => Some(("type", trimmed.to_string())),
        (0, false, true) => Some(("const", trimmed.to_string())),
        _ => None,
    }
}

fn classify_js_ts_decl(trimmed: &str, depth: i32) -> Option<(&'static str, String)> {
    if depth > OUTLINE_MAX_BRACE_DEPTH
        || trimmed.starts_with("import ")
        || trimmed.starts_with("return ")
    {
        return None;
    }
    let core = strip_prefix_table(trimmed, JS_MODIFIER_PREFIXES);
    if let Some(&(_, kind)) = JS_DECL_PREFIXES.iter().find(|&&(p, _)| core.starts_with(p)) {
        return Some((kind, trimmed.to_string()));
    }
    if core.starts_with("type ") && core.contains('=') {
        return Some(("type", trimmed.to_string()));
    }
    match (
        depth,
        core.starts_with("const ") || core.starts_with("let "),
        trimmed.contains("=>"),
        trimmed.starts_with("export "),
    ) {
        (0, true, true, _) => Some(("fn", trimmed.to_string())),
        (0, true, false, true) => Some(("const", trimmed.to_string())),
        (1, _, _, _) if looks_like_method_signature(core) => Some(("method", trimmed.to_string())),
        _ => None,
    }
}

fn looks_like_method_signature(core: &str) -> bool {
    let Some(paren) = core.find('(') else {
        return false;
    };
    let head = core[..paren].trim();
    if head.is_empty() || head.contains(' ') || CONTROL_FLOW_HEADS.contains(&head) {
        return false;
    }
    head.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$'))
        && (core.contains('{') || core.ends_with(')'))
}

fn classify_go_decl(trimmed: &str, depth: i32) -> Option<(&'static str, String)> {
    if depth > 0 {
        return None;
    }
    match (
        trimmed.starts_with("func "),
        trimmed.starts_with("type "),
        trimmed.contains(" struct"),
        trimmed.contains(" interface"),
    ) {
        (true, _, _, _) => Some(("fn", trimmed.to_string())),
        (false, true, true, _) => Some(("struct", trimmed.to_string())),
        (false, true, false, true) => Some(("interface", trimmed.to_string())),
        (false, true, false, false) => Some(("type", trimmed.to_string())),
        _ => None,
    }
}

fn classify_c_like_decl(trimmed: &str, depth: i32) -> Option<(&'static str, String)> {
    if depth > OUTLINE_MAX_BRACE_DEPTH
        || trimmed.starts_with('#')
        || trimmed.starts_with("import ")
        || trimmed.starts_with("using ")
    {
        return None;
    }
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if let Some(kind) = words.iter().find_map(|w| {
        CLIKE_KEYWORD_KINDS
            .iter()
            .find(|&&(kw, _)| kw == *w)
            .map(|&(_, k)| k)
    }) {
        return Some((kind, trimmed.to_string()));
    }
    let is_control = words
        .first()
        .is_some_and(|w| CONTROL_FLOW_HEADS.contains(w));
    match depth <= 1
        && !is_control
        && trimmed.contains('(')
        && trimmed.contains(')')
        && trimmed.ends_with('{')
    {
        true => Some(("fn", trimmed.to_string())),
        false => None,
    }
}

// ─── Indentation-Scoped Python Scanner ───────────────────────────────────────

fn extract_python_outline(lines: &[&str]) -> Vec<OutlineEntry> {
    let mut entries: Vec<OutlineEntry> = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut last_code_line = 1usize;

    for (idx, &raw_line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw_line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = raw_line.len().saturating_sub(trimmed.len());

        while let Some(&(entry_idx, sym_indent)) = stack.last() {
            match indent <= sym_indent {
                true => {
                    stack.pop();
                    entries[entry_idx].end_line = last_code_line.max(entries[entry_idx].line);
                }
                false => break,
            }
        }

        if let Some(&(_, kind)) = PYTHON_DECL_PREFIXES
            .iter()
            .find(|&&(prefix, _)| trimmed.starts_with(prefix))
        {
            let entry_idx = entries.len();
            let depth = stack.len();
            entries.push(OutlineEntry {
                line: line_no,
                end_line: line_no,
                hash: line_hash(raw_line),
                kind,
                depth,
                signature: compact_signature(trimmed),
            });
            stack.push((entry_idx, indent));
        }

        last_code_line = line_no;
    }

    for (entry_idx, _) in stack {
        entries[entry_idx].end_line = last_code_line.max(entries[entry_idx].line);
    }
    entries
}

// ─── Markdown Heading Outline Scanner ────────────────────────────────────────

fn extract_markdown_outline(lines: &[&str]) -> Vec<OutlineEntry> {
    let mut entries: Vec<OutlineEntry> = Vec::new();
    let mut open_headings: Vec<(usize, usize)> = Vec::new();
    let mut in_fence = false;

    for (idx, &raw_line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw_line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
        if (1..=6).contains(&hashes) && trimmed.as_bytes().get(hashes) == Some(&b' ') {
            while let Some(&(entry_idx, lvl)) = open_headings.last() {
                match lvl >= hashes {
                    true => {
                        open_headings.pop();
                        entries[entry_idx].end_line =
                            line_no.saturating_sub(1).max(entries[entry_idx].line);
                    }
                    false => break,
                }
            }
            let entry_idx = entries.len();
            entries.push(OutlineEntry {
                line: line_no,
                end_line: line_no,
                hash: line_hash(raw_line),
                kind: "heading",
                depth: hashes.saturating_sub(1),
                signature: compact_signature(trimmed),
            });
            open_headings.push((entry_idx, hashes));
        }
    }

    let final_line = lines.len().max(1);
    for (entry_idx, _) in open_headings {
        entries[entry_idx].end_line = final_line;
    }
    entries
}

// ─── TOML Section Outline Scanner ────────────────────────────────────────────

fn extract_toml_outline(lines: &[&str]) -> Vec<OutlineEntry> {
    let mut entries: Vec<OutlineEntry> = Vec::new();
    for (idx, &raw_line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw_line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if let Some(prev) = entries.last_mut() {
                prev.end_line = line_no.saturating_sub(1).max(prev.line);
            }
            entries.push(OutlineEntry {
                line: line_no,
                end_line: line_no,
                hash: line_hash(raw_line),
                kind: "section",
                depth: match trimmed.contains('.') {
                    true => 1,
                    false => 0,
                },
                signature: compact_signature(trimmed),
            });
        }
    }
    if let Some(last) = entries.last_mut() {
        last.end_line = lines.len().max(last.line);
    }
    entries
}

fn compact_signature(sig: &str) -> String {
    let cleaned = sig.trim().trim_end_matches('{').trim();
    match cleaned.chars().count() > OUTLINE_MAX_SIGNATURE_CHARS {
        true => {
            let cut: String = cleaned.chars().take(OUTLINE_MAX_SIGNATURE_CHARS).collect();
            format!("{cut}...")
        }
        false => cleaned.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_rust_symbols_preserving_utf8_and_handling_raw_byte_strings() {
        let src = r##"
// fn fake_comment() {}
/*
struct FakeBlock {
    x: i32,
}
*/
pub(in crate::core) const LIMIT: usize = 99;

pub struct Engine {
    raw: &'static str,
}

impl Engine {
    pub async fn run(&self) -> usize {
        let tricky = "{ fn fake_in_str() {} }";
        let raw_s = r#"struct FakeRaw { }"#;
        let byte_raw = br#"quote " { fn fake_byte_raw() {} }"#;
        let arrow = "→ UTF-8 🦊 café";
        tricky.len() + raw_s.len() + byte_raw.len() + arrow.len()
    }
}

pub fn after_raw_bytes_café() -> usize {
    1
}
"##;
        let report = extract_outline(Path::new("src/engine.rs"), src, 50);
        let names: Vec<(&str, &str, usize, usize)> = report
            .entries
            .iter()
            .map(|e| (e.kind, e.signature.as_str(), e.line, e.end_line))
            .collect();
        assert_eq!(names.len(), 5, "entries={names:?}");
        assert_eq!(names[0].0, "const");
        assert!(names[0].1.contains("LIMIT"));
        assert_eq!(names[1].0, "struct");
        assert!(names[1].1.contains("pub struct Engine"));
        assert_eq!(names[2].0, "impl");
        assert_eq!(names[3].0, "fn");
        assert!(names[3].1.contains("pub async fn run"));
        assert!(names[3].3 > names[3].2, "expected multi-line fn span");
        assert_eq!(names[4].0, "fn");
        assert!(
            names[4].1.contains("after_raw_bytes_café"),
            "UTF-8 signature corrupted: {}",
            names[4].1
        );
    }

    #[test]
    fn extracts_typescript_and_python_outlines() {
        let ts = r#"
export interface Config {
    port: number;
}
export class Server {
    async start(port: number) {
        return port;
    }
}
export const buildApp = (cfg: Config) => {
    return new Server();
};
"#;
        let ts_rep = extract_outline(Path::new("server.ts"), ts, 50);
        assert_eq!(ts_rep.entries.len(), 4, "ts={:?}", ts_rep.entries);
        assert_eq!(ts_rep.entries[0].kind, "interface");
        assert_eq!(ts_rep.entries[1].kind, "class");
        assert_eq!(ts_rep.entries[2].kind, "method");
        assert_eq!(ts_rep.entries[3].kind, "fn");

        let py = r#"
class Planner:
    def plan(self, goal):
        return goal

async def run_all():
    return True
"#;
        let py_rep = extract_outline(Path::new("planner.py"), py, 50);
        assert_eq!(py_rep.entries.len(), 3, "py={:?}", py_rep.entries);
        assert_eq!(py_rep.entries[0].kind, "class");
        assert_eq!(py_rep.entries[1].kind, "fn");
        assert_eq!(py_rep.entries[1].depth, 1);
        assert_eq!(py_rep.entries[2].kind, "fn");
        assert_eq!(py_rep.entries[2].depth, 0);
    }

    #[test]
    fn extracts_markdown_headings_ignoring_fenced_blocks() {
        let md = "# Top — Overview\ntext\n```bash\n# not a heading\n```\n## Sub\nmore\n";
        let rep = extract_outline(Path::new("README.md"), md, 50);
        assert_eq!(rep.entries.len(), 2, "md={:?}", rep.entries);
        assert_eq!(rep.entries[0].signature, "# Top — Overview");
        assert_eq!(rep.entries[1].signature, "## Sub");
    }
}
