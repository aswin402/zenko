//! Zero-Dependency, Zero-Regex Pure-Rust Structural Code Outline Extractor.
//!
//! Powered by deterministic byte/token/brace/indentation state machines across declarative
//! language tables — instant even when LSP servers are cold or uninstalled, returning
//! hash-anchored symbol spans (`line:hash| [kind] signature (Lstart..Lend)`) directly
//! actionable by `read_file(ranges=...)` and `hash_edit`.

use crate::fs_tools::line_hash;
use std::path::Path;
use zenko_core::OUTLINE_MAX_ENTRIES;

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
        &["c", "h", "cc", "cpp", "cxx", "hpp", "hh", "java", "cs", "kt", "kts", "swift"],
        OutlineSyntax::BraceCLike,
    ),
    (&["py", "pyi"], OutlineSyntax::IndentPython),
    (&["md", "mdx", "markdown"], OutlineSyntax::MarkupMarkdown),
    (&["toml"], OutlineSyntax::ConfigToml),
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
        Some(OutlineSyntax::BraceRust) => extract_brace_outline(&lines, OutlineSyntax::BraceRust),
        Some(OutlineSyntax::BraceJsTs) => extract_brace_outline(&lines, OutlineSyntax::BraceJsTs),
        Some(OutlineSyntax::BraceGo) => extract_brace_outline(&lines, OutlineSyntax::BraceGo),
        Some(OutlineSyntax::BraceCLike) => extract_brace_outline(&lines, OutlineSyntax::BraceCLike),
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

        if !was_in_comment && !trimmed.is_empty() {
            if let Some((kind, sig)) = classify_brace_declaration(trimmed, syntax, brace_depth) {
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
        }

        if opens > 0 {
            for sym in open_stack.iter_mut().rev() {
                if sym.pending_open {
                    sym.pending_open = false;
                }
            }
        }

        brace_depth = (brace_depth + opens - closes).max(0);

        // Close any symbol whose opening block depth has now returned.
        while let Some(top) = open_stack.last() {
            let ready_to_close = match top.pending_open {
                true => trimmed.ends_with(';'),
                false => brace_depth <= top.target_depth,
            };
            if ready_to_close {
                let sym = open_stack.pop();
                if let Some(s) = sym {
                    entries[s.entry_idx].end_line = line_no;
                }
            } else {
                break;
            }
        }
    }

    let final_line = lines.len().max(1);
    for sym in open_stack {
        entries[sym.entry_idx].end_line = final_line;
    }
    entries
}

/// Scans one source line updating multi-line comment/raw-string state, returning
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
    let mut code = String::with_capacity(len);
    let mut in_double = false;
    let mut in_single = false;

    while i < len {
        if let Some(hashes) = state.rust_raw_hashes {
            if bytes[i] == b'"'
                && i + hashes < len
                && bytes[i + 1..=i + hashes].iter().all(|&b| b == b'#')
            {
                state.rust_raw_hashes = None;
                i += 1 + hashes;
            } else {
                i += 1;
            }
            continue;
        }
        if state.in_block_comment {
            if bytes[i] == b'*' && i + 1 < len && bytes[i + 1] == b'/' {
                state.in_block_comment = false;
                i += 2;
            } else {
                i += 1;
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

        // Line comment `//`
        if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            break;
        }
        // Block comment `/*`
        if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            state.in_block_comment = true;
            i += 2;
            continue;
        }
        // Rust raw string literal `r#"..."#`
        if syntax == OutlineSyntax::BraceRust
            && bytes[i] == b'r'
            && i + 1 < len
            && matches!(bytes[i + 1], b'#' | b'"')
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_')
        {
            let mut j = i + 1;
            while j < len && bytes[j] == b'#' {
                j += 1;
            }
            if j < len && bytes[j] == b'"' {
                state.rust_raw_hashes = Some(j - (i + 1));
                i = j + 1;
                continue;
            }
        }

        match bytes[i] {
            b'"' => {
                in_double = true;
                code.push('"');
                i += 1;
            }
            b'\'' if syntax != OutlineSyntax::BraceRust => {
                in_single = true;
                code.push('\'');
                i += 1;
            }
            b'\'' if syntax == OutlineSyntax::BraceRust => {
                // Distinguish Rust char literal `'x'` / `'\n'` from lifetime `'a`
                let is_char_lit = (i + 2 < len && bytes[i + 2] == b'\'')
                    || (i + 3 < len && bytes[i + 1] == b'\\' && bytes[i + 3] == b'\'');
                match is_char_lit {
                    true => {
                        in_single = true;
                        code.push('\'');
                        i += 1;
                    }
                    false => {
                        code.push('\'');
                        i += 1;
                    }
                }
            }
            b'`' if matches!(syntax, OutlineSyntax::BraceJsTs | OutlineSyntax::BraceGo) => {
                state.in_backtick_string = true;
                code.push('`');
                i += 1;
            }
            b'{' => {
                opens += 1;
                code.push('{');
                i += 1;
            }
            b'}' => {
                closes += 1;
                code.push('}');
                i += 1;
            }
            b => {
                code.push(b as char);
                i += 1;
            }
        }
    }

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

fn strip_rust_visibility(s: &str) -> &str {
    let mut cur = s.trim_start();
    if let Some(rest) = cur.strip_prefix("pub") {
        if let Some(after_paren) = rest.strip_prefix('(')
            && let Some(close) = after_paren.find(')')
        {
            cur = after_paren[close + 1..].trim_start();
        } else if rest.starts_with(char::is_whitespace) {
            cur = rest.trim_start();
        }
    }
    loop {
        let next = cur
            .strip_prefix("default ")
            .or_else(|| cur.strip_prefix("async "))
            .or_else(|| cur.strip_prefix("unsafe "))
            .or_else(|| cur.strip_prefix("const "))
            .or_else(|| cur.strip_prefix("extern \"C\" "))
            .or_else(|| cur.strip_prefix("extern "));
        match next {
            Some(r) => cur = r.trim_start(),
            None => break,
        }
    }
    cur
}

fn classify_rust_decl(trimmed: &str, depth: i32) -> Option<(&'static str, String)> {
    if depth > 2 || trimmed.starts_with('#') || trimmed.starts_with("use ") {
        return None;
    }
    let core = strip_rust_visibility(trimmed);
    let kind = match () {
        _ if core.starts_with("fn ") => "fn",
        _ if core.starts_with("struct ") => "struct",
        _ if core.starts_with("enum ") => "enum",
        _ if core.starts_with("trait ") => "trait",
        _ if core.starts_with("impl ") || core.starts_with("impl<") => "impl",
        _ if core.starts_with("mod ") => "mod",
        _ if core.starts_with("type ") && depth == 0 => "type",
        _ if core.starts_with("macro_rules!") => "macro",
        _ if depth == 0
            && (trimmed.starts_with("pub const ")
                || trimmed.starts_with("pub(crate) const ")
                || trimmed.starts_with("const "))
            && trimmed.contains(':') =>
        {
            "const"
        }
        _ => return None,
    };
    Some((kind, trimmed.to_string()))
}

fn strip_js_modifiers(s: &str) -> &str {
    let mut cur = s.trim_start();
    loop {
        let next = cur
            .strip_prefix("export ")
            .or_else(|| cur.strip_prefix("default "))
            .or_else(|| cur.strip_prefix("declare "))
            .or_else(|| cur.strip_prefix("abstract "))
            .or_else(|| cur.strip_prefix("async "))
            .or_else(|| cur.strip_prefix("public "))
            .or_else(|| cur.strip_prefix("private "))
            .or_else(|| cur.strip_prefix("protected "))
            .or_else(|| cur.strip_prefix("static "))
            .or_else(|| cur.strip_prefix("readonly "));
        match next {
            Some(r) => cur = r.trim_start(),
            None => break,
        }
    }
    cur
}

fn classify_js_ts_decl(trimmed: &str, depth: i32) -> Option<(&'static str, String)> {
    if depth > 2 || trimmed.starts_with("import ") || trimmed.starts_with("return ") {
        return None;
    }
    let core = strip_js_modifiers(trimmed);
    let kind = match () {
        _ if core.starts_with("function ") || core.starts_with("function*") => "fn",
        _ if core.starts_with("class ") => "class",
        _ if core.starts_with("interface ") => "interface",
        _ if core.starts_with("type ") && core.contains('=') => "type",
        _ if core.starts_with("enum ") => "enum",
        _ if core.starts_with("namespace ") => "namespace",
        _ if depth == 0
            && (core.starts_with("const ") || core.starts_with("let "))
            && (trimmed.contains("=>") || trimmed.starts_with("export ")) =>
        {
            match trimmed.contains("=>") {
                true => "fn",
                false => "const",
            }
        }
        _ if depth == 1 && looks_like_method_signature(core) => "method",
        _ => return None,
    };
    Some((kind, trimmed.to_string()))
}

fn looks_like_method_signature(core: &str) -> bool {
    let Some(paren) = core.find('(') else {
        return false;
    };
    let head = core[..paren].trim();
    if head.is_empty()
        || head.contains(' ')
        || matches!(
            head,
            "if" | "for" | "while" | "switch" | "catch" | "return" | "throw" | "new"
        )
    {
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
    let kind = match () {
        _ if trimmed.starts_with("func ") => "fn",
        _ if trimmed.starts_with("type ") && trimmed.contains(" struct") => "struct",
        _ if trimmed.starts_with("type ") && trimmed.contains(" interface") => "interface",
        _ if trimmed.starts_with("type ") => "type",
        _ => return None,
    };
    Some((kind, trimmed.to_string()))
}

fn classify_c_like_decl(trimmed: &str, depth: i32) -> Option<(&'static str, String)> {
    if depth > 2 || trimmed.starts_with('#') || trimmed.starts_with("import ") || trimmed.starts_with("using ") {
        return None;
    }
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    let first_kw = words.iter().copied().find(|w| {
        matches!(
            *w,
            "class" | "struct" | "enum" | "interface" | "namespace" | "trait" | "protocol" | "fun" | "func"
        )
    });
    if let Some(kw) = first_kw {
        let kind = match kw {
            "class" => "class",
            "struct" => "struct",
            "enum" => "enum",
            "interface" | "protocol" => "interface",
            "namespace" => "namespace",
            "fun" | "func" => "fn",
            _ => "type",
        };
        return Some((kind, trimmed.to_string()));
    }
    // Top-level or class-level C/C++/Java method/function signature ending with `{`
    if depth <= 1
        && trimmed.contains('(')
        && trimmed.contains(')')
        && trimmed.ends_with('{')
        && !matches!(
            words.first().copied(),
            Some("if" | "for" | "while" | "switch" | "catch" | "else" | "return")
        )
    {
        return Some(("fn", trimmed.to_string()));
    }
    None
}

// ─── Indentation-Scoped Python Scanner ───────────────────────────────────────

fn extract_python_outline(lines: &[&str]) -> Vec<OutlineEntry> {
    let mut entries: Vec<OutlineEntry> = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new(); // (entry_idx, indent_cols)
    let mut last_code_line = 1usize;

    for (idx, &raw_line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw_line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = raw_line.len().saturating_sub(trimmed.len());

        while let Some(&(entry_idx, sym_indent)) = stack.last() {
            if indent <= sym_indent {
                stack.pop();
                entries[entry_idx].end_line = last_code_line.max(entries[entry_idx].line);
            } else {
                break;
            }
        }

        let kind = match () {
            _ if trimmed.starts_with("class ") => Some("class"),
            _ if trimmed.starts_with("def ") || trimmed.starts_with("async def ") => Some("fn"),
            _ => None,
        };
        if let Some(k) = kind {
            let entry_idx = entries.len();
            let depth = stack.len();
            entries.push(OutlineEntry {
                line: line_no,
                end_line: line_no,
                hash: line_hash(raw_line),
                kind: k,
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
    let mut open_headings: Vec<(usize, usize)> = Vec::new(); // (entry_idx, level)
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
                if lvl >= hashes {
                    open_headings.pop();
                    entries[entry_idx].end_line = line_no.saturating_sub(1).max(entries[entry_idx].line);
                } else {
                    break;
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
    let max_chars = 120usize;
    match cleaned.chars().count() > max_chars {
        true => {
            let cut: String = cleaned.chars().take(max_chars).collect();
            format!("{cut}...")
        }
        false => cleaned.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_rust_symbols_ignoring_comments_and_strings() {
        let src = r##"
// fn fake_comment() {}
/*
struct FakeBlock {
    x: i32,
}
*/
pub struct Engine {
    raw: &'static str,
}

impl Engine {
    pub async fn run(&self) -> usize {
        let tricky = "{ fn fake_in_str() {} }";
        let raw_s = r#"struct FakeRaw { }"#;
        tricky.len() + raw_s.len()
    }
}
"##;
        let report = extract_outline(Path::new("src/engine.rs"), src, 50);
        let names: Vec<(&str, &str, usize, usize)> = report
            .entries
            .iter()
            .map(|e| (e.kind, e.signature.as_str(), e.line, e.end_line))
            .collect();
        assert_eq!(names.len(), 3, "entries={names:?}");
        assert_eq!(names[0].0, "struct");
        assert!(names[0].1.contains("pub struct Engine"));
        assert_eq!(names[1].0, "impl");
        assert_eq!(names[2].0, "fn");
        assert!(names[2].1.contains("pub async fn run"));
        assert!(names[2].3 > names[2].2, "expected multi-line fn span");
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
        let md = "# Top\ntext\n```bash\n# not a heading\n```\n## Sub\nmore\n";
        let rep = extract_outline(Path::new("README.md"), md, 50);
        assert_eq!(rep.entries.len(), 2, "md={:?}", rep.entries);
        assert_eq!(rep.entries[0].signature, "# Top");
        assert_eq!(rep.entries[1].signature, "## Sub");
    }
}
