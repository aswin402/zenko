//! Pure-Rust, Zero-Regex Structural Code Intelligence Engine:
//! 1. Comment/string-aware Balanced Token-Tree Lexer driven by `LANGUAGE_SYNTAX_TABLE`
//! 2. `ast-grep`-inspired Metavariable Unifier & Template Rewriter (`$X`, `$$$ARGS`, `$_`)
//! 3. `sd`/`ripide`-inspired Code-Only Identifier Replacer & Relative-Import Rewriter
//! 4. `difftastic`-inspired Prefix/Suffix Pruning + Weighted Dijkstra Structural Diff
//!
//! Zero external C/tree-sitter deps, zero regex, zero `if/else` ladders, zero production `.unwrap()`.

use serde::Serialize;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::path::{Component, Path, PathBuf};
use zenko_core::{
    FALLBACK_CODE_SYNTAX, LanguageSyntaxSpec, Result, STRUCT_DIFF_MAX_EDITS_RENDER,
    STRUCT_DIFF_MAX_TOKENS_DIJKSTRA, STRUCT_META_SINGLE_PREFIX, STRUCT_META_VARIADIC_PREFIX,
    STRUCT_META_WILDCARD, ZenkoError, find_language_syntax_by_ext,
};

pub fn syntax_for_path(path: &Path) -> &'static LanguageSyntaxSpec {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(find_language_syntax_by_ext)
        .unwrap_or(&FALLBACK_CODE_SYNTAX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelimKind {
    Paren,
    Brace,
    Bracket,
}

impl DelimKind {
    fn from_open(b: u8) -> Option<Self> {
        match b {
            b'(' => Some(Self::Paren),
            b'{' => Some(Self::Brace),
            b'[' => Some(Self::Bracket),
            _ => None,
        }
    }

    fn from_close(b: u8) -> Option<Self> {
        match b {
            b')' => Some(Self::Paren),
            b'}' => Some(Self::Brace),
            b']' => Some(Self::Bracket),
            _ => None,
        }
    }

    pub fn open_str(self) -> &'static str {
        match self {
            Self::Paren => "(",
            Self::Brace => "{",
            Self::Bracket => "[",
        }
    }

    pub fn close_str(self) -> &'static str {
        match self {
            Self::Paren => ")",
            Self::Brace => "}",
            Self::Bracket => "]",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlatTokenKind {
    Ident,
    Literal,
    Punct,
    OpenDelim,
    CloseDelim,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatToken {
    pub kind: FlatTokenKind,
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenNode {
    Ident {
        text: String,
        start: usize,
        end: usize,
        line: usize,
    },
    Literal {
        text: String,
        start: usize,
        end: usize,
        line: usize,
    },
    Punct {
        text: String,
        start: usize,
        end: usize,
        line: usize,
    },
    Group {
        delim: DelimKind,
        start: usize,
        end: usize,
        line: usize,
        end_line: usize,
        children: Vec<TokenNode>,
    },
}

impl TokenNode {
    pub fn span(&self) -> (usize, usize) {
        match self {
            Self::Ident { start, end, .. }
            | Self::Literal { start, end, .. }
            | Self::Punct { start, end, .. }
            | Self::Group { start, end, .. } => (*start, *end),
        }
    }

    pub fn lines(&self) -> (usize, usize) {
        match self {
            Self::Ident { line, .. }
            | Self::Literal { line, .. }
            | Self::Punct { line, .. } => (*line, *line),
            Self::Group { line, end_line, .. } => (*line, *end_line),
        }
    }

    /// Canonical comment-free & formatting-free structural signature for equality checks (`$X == $X`).
    fn canonical_signature(&self, out: &mut String) {
        match self {
            Self::Ident { text, .. } | Self::Literal { text, .. } | Self::Punct { text, .. } => {
                out.push_str(text);
                out.push(' ');
            }
            Self::Group {
                delim, children, ..
            } => {
                out.push_str(delim.open_str());
                for ch in children {
                    ch.canonical_signature(out);
                }
                out.push_str(delim.close_str());
                out.push(' ');
            }
        }
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn quote_enabled(spec: &LanguageSyntaxSpec, byte: u8) -> bool {
    match byte {
        b'"' => spec.double_quote,
        b'\'' => spec.single_quote,
        b'`' => spec.backtick_quote,
        _ => false,
    }
}

fn match_prefix_len(bytes: &[u8], pos: usize, needle: Option<&str>) -> Option<usize> {
    let n = needle?.as_bytes();
    match !n.is_empty() && bytes[pos..].starts_with(n) {
        true => Some(n.len()),
        false => None,
    }
}

const MULTI_CHAR_PUNCTS: &[&str] = &[
    "...", "::", "->", "=>", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "?.",
    "??", "<<", ">>",
];

/// Comment/string-aware deterministic lexer emitting flat structural tokens with exact byte spans.
pub fn lex_code_tokens(spec: &LanguageSyntaxSpec, source: &str) -> Vec<FlatToken> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;

    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // Line comment -> skip until newline
        if let Some(len) = match_prefix_len(bytes, i, spec.line_comment) {
            i += len;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment (with optional nesting for Rust/Kotlin/Swift) -> skip cleanly
        if let Some(open_len) = match_prefix_len(bytes, i, spec.block_open) {
            i += open_len;
            let mut depth = 1usize;
            while i < bytes.len() && depth > 0 {
                if bytes[i] == b'\n' {
                    line += 1;
                    i += 1;
                    continue;
                }
                if spec.nested_block_comments
                    && let Some(l) = match_prefix_len(bytes, i, spec.block_open)
                {
                    depth += 1;
                    i += l;
                    continue;
                }
                if let Some(l) = match_prefix_len(bytes, i, spec.block_close) {
                    depth -= 1;
                    i += l;
                    continue;
                }
                i += 1;
            }
            continue;
        }
        // String literal
        if quote_enabled(spec, b) {
            let start = i;
            let start_line = line;
            let quote = b;
            i += 1;
            let mut escaped = false;
            while i < bytes.len() {
                let c = bytes[i];
                if c == b'\n' {
                    line += 1;
                }
                let done = match (escaped, c) {
                    (true, _) => {
                        escaped = false;
                        false
                    }
                    (false, b'\\') => {
                        escaped = true;
                        false
                    }
                    (false, ch) if ch == quote => true,
                    (false, _) => false,
                };
                i += 1;
                if done {
                    break;
                }
            }
            out.push(FlatToken {
                kind: FlatTokenKind::Literal,
                text: source[start..i].to_string(),
                start,
                end: i,
                line: start_line,
            });
            continue;
        }
        // Identifier or metavariable (`$X`, `$$$ARGS`, `$_`)
        if is_ident_start(b) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_cont(bytes[i]) {
                i += 1;
            }
            out.push(FlatToken {
                kind: FlatTokenKind::Ident,
                text: source[start..i].to_string(),
                start,
                end: i,
                line,
            });
            continue;
        }
        // Numeric literal
        if b.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
            {
                i += 1;
            }
            out.push(FlatToken {
                kind: FlatTokenKind::Literal,
                text: source[start..i].to_string(),
                start,
                end: i,
                line,
            });
            continue;
        }
        // Delimiters
        if DelimKind::from_open(b).is_some() {
            out.push(FlatToken {
                kind: FlatTokenKind::OpenDelim,
                text: source[i..i + 1].to_string(),
                start: i,
                end: i + 1,
                line,
            });
            i += 1;
            continue;
        }
        if DelimKind::from_close(b).is_some() {
            out.push(FlatToken {
                kind: FlatTokenKind::CloseDelim,
                text: source[i..i + 1].to_string(),
                start: i,
                end: i + 1,
                line,
            });
            i += 1;
            continue;
        }
        // Multi-char punctuation operators
        let multi = MULTI_CHAR_PUNCTS
            .iter()
            .find(|op| bytes[i..].starts_with(op.as_bytes()))
            .copied();
        match multi {
            Some(op) => {
                out.push(FlatToken {
                    kind: FlatTokenKind::Punct,
                    text: op.to_string(),
                    start: i,
                    end: i + op.len(),
                    line,
                });
                i += op.len();
            }
            None => {
                // Advance safely across full UTF-8 character width
                let ch_len = source[i..]
                    .chars()
                    .next()
                    .map(char::len_utf8)
                    .unwrap_or(1);
                out.push(FlatToken {
                    kind: FlatTokenKind::Punct,
                    text: source[i..i + ch_len].to_string(),
                    start: i,
                    end: i + ch_len,
                    line,
                });
                i += ch_len;
            }
        }
    }

    out
}

/// Builds a balanced delimiter token tree (`Paren`, `Brace`, `Bracket`) from flat tokens.
/// Unmatched delimiters degrade safely into `Punct` tokens so partial snippets still parse cleanly.
pub fn lex_token_tree(spec: &LanguageSyntaxSpec, source: &str) -> Vec<TokenNode> {
    let flat = lex_code_tokens(spec, source);
    let mut stack: Vec<(DelimKind, usize, usize, Vec<TokenNode>)> = Vec::new();
    let mut root: Vec<TokenNode> = Vec::new();

    for tok in flat {
        match tok.kind {
            FlatTokenKind::OpenDelim => {
                if let Some(delim) = tok.text.as_bytes().first().copied().and_then(DelimKind::from_open) {
                    stack.push((delim, tok.start, tok.line, Vec::new()));
                }
            }
            FlatTokenKind::CloseDelim => {
                let close_delim = tok.text.as_bytes().first().copied().and_then(DelimKind::from_close);
                match (stack.last().map(|s| s.0), close_delim) {
                    (Some(open_d), Some(cd)) if open_d == cd => {
                        if let Some((delim, start, line, children)) = stack.pop() {
                            let group = TokenNode::Group {
                                delim,
                                start,
                                end: tok.end,
                                line,
                                end_line: tok.line,
                                children,
                            };
                            match stack.last_mut() {
                                Some((_, _, _, parent)) => parent.push(group),
                                None => root.push(group),
                            }
                        }
                    }
                    _ => {
                        let p = TokenNode::Punct {
                            text: tok.text,
                            start: tok.start,
                            end: tok.end,
                            line: tok.line,
                        };
                        match stack.last_mut() {
                            Some((_, _, _, parent)) => parent.push(p),
                            None => root.push(p),
                        }
                    }
                }
            }
            FlatTokenKind::Ident => {
                let node = TokenNode::Ident {
                    text: tok.text,
                    start: tok.start,
                    end: tok.end,
                    line: tok.line,
                };
                match stack.last_mut() {
                    Some((_, _, _, parent)) => parent.push(node),
                    None => root.push(node),
                }
            }
            FlatTokenKind::Literal => {
                let node = TokenNode::Literal {
                    text: tok.text,
                    start: tok.start,
                    end: tok.end,
                    line: tok.line,
                };
                match stack.last_mut() {
                    Some((_, _, _, parent)) => parent.push(node),
                    None => root.push(node),
                }
            }
            FlatTokenKind::Punct => {
                let node = TokenNode::Punct {
                    text: tok.text,
                    start: tok.start,
                    end: tok.end,
                    line: tok.line,
                };
                match stack.last_mut() {
                    Some((_, _, _, parent)) => parent.push(node),
                    None => root.push(node),
                }
            }
        }
    }

    // Drain any unclosed groups gracefully into flat nodes
    while let Some((delim, start, line, children)) = stack.pop() {
        let mut target = match stack.last_mut() {
            Some((_, _, _, parent)) => std::mem::take(parent),
            None => std::mem::take(&mut root),
        };
        target.push(TokenNode::Punct {
            text: delim.open_str().to_string(),
            start,
            end: start + 1,
            line,
        });
        target.extend(children);
        match stack.last_mut() {
            Some((_, _, _, parent)) => *parent = target,
            None => root = target,
        }
    }

    root
}

// ─── 1. `ast-grep`-Inspired Structural Metavariable Unifier & Rewriter ───────

#[derive(Debug, Clone, PartialEq, Eq)]
enum MetaVarKind {
    Wildcard,
    Single(String),
    Variadic(String),
}

fn parse_metavar(ident: &str) -> Option<MetaVarKind> {
    if ident == STRUCT_META_WILDCARD {
        return Some(MetaVarKind::Wildcard);
    }
    if let Some(rest) = ident.strip_prefix(STRUCT_META_VARIADIC_PREFIX)
        && !rest.is_empty()
        && rest
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    {
        return Some(MetaVarKind::Variadic(rest.to_string()));
    }
    if let Some(rest) = ident.strip_prefix(STRUCT_META_SINGLE_PREFIX)
        && !rest.is_empty()
        && !rest.starts_with('$')
        && rest
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    {
        return Some(MetaVarKind::Single(rest.to_string()));
    }
    None
}

#[derive(Debug, Clone)]
struct BoundSlice {
    raw_text: String,
    canonical: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StructuralMatch {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub matched_text: String,
    pub bindings: BTreeMap<String, String>,
}

fn slice_canonical(nodes: &[TokenNode]) -> String {
    let mut s = String::new();
    for n in nodes {
        n.canonical_signature(&mut s);
    }
    s
}

fn slice_raw(source: &str, nodes: &[TokenNode]) -> String {
    match (nodes.first(), nodes.last()) {
        (Some(first), Some(last)) => {
            let (s, _) = first.span();
            let (_, e) = last.span();
            source.get(s..e).unwrap_or("").to_string()
        }
        _ => String::new(),
    }
}

/// Computes how many target nodes form one balanced expression atom starting at `start`
/// (e.g., `foo`, `foo.bar(a, b)`, `a::b[0]`, or a single literal/group) so `$X` matches
/// compound expressions cleanly without consuming commas or statement terminators.
fn expr_atom_len(target: &[TokenNode], start: usize) -> usize {
    if start >= target.len() {
        return 0;
    }
    let mut len = 1usize;
    // Allow chained postfix accesses: `.ident`, `::ident`, `(args)`, `[idx]`
    while start + len < target.len() {
        match &target[start + len] {
            TokenNode::Group {
                delim: DelimKind::Paren | DelimKind::Bracket,
                ..
            } => {
                len += 1;
            }
            TokenNode::Punct { text, .. } if matches!(text.as_str(), "." | "::" | "?.") => {
                if matches!(target.get(start + len + 1), Some(TokenNode::Ident { .. })) {
                    len += 2;
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    len
}

fn bind_or_verify(
    bindings: &mut BTreeMap<String, BoundSlice>,
    name: &str,
    source: &str,
    nodes: &[TokenNode],
) -> bool {
    let canonical = slice_canonical(nodes);
    match bindings.get(name) {
        Some(existing) => existing.canonical == canonical,
        None => {
            bindings.insert(
                name.to_string(),
                BoundSlice {
                    raw_text: slice_raw(source, nodes),
                    canonical,
                },
            );
            true
        }
    }
}

fn match_seq(
    pat: &[TokenNode],
    target: &[TokenNode],
    source: &str,
    p_idx: usize,
    t_idx: usize,
    bindings: &mut BTreeMap<String, BoundSlice>,
    require_full_target: bool,
) -> Option<usize> {
    if p_idx == pat.len() {
        return match !require_full_target || t_idx == target.len() {
            true => Some(t_idx),
            false => None,
        };
    }

    match &pat[p_idx] {
        TokenNode::Ident { text, .. } => match parse_metavar(text) {
            Some(MetaVarKind::Variadic(name)) => {
                // Non-greedy shortest-first capture of 0..=(target.len() - t_idx) nodes
                for take in 0..=(target.len().saturating_sub(t_idx)) {
                    let mut fork = bindings.clone();
                    if !bind_or_verify(&mut fork, &name, source, &target[t_idx..t_idx + take]) {
                        continue;
                    }
                    if let Some(end_idx) = match_seq(
                        pat,
                        target,
                        source,
                        p_idx + 1,
                        t_idx + take,
                        &mut fork,
                        require_full_target,
                    ) {
                        *bindings = fork;
                        return Some(end_idx);
                    }
                }
                None
            }
            Some(MetaVarKind::Single(name)) => {
                if t_idx >= target.len() {
                    return None;
                }
                let max_expr = expr_atom_len(target, t_idx);
                // Try compound expression length first (e.g. `obj.field(1)`), then single token
                let lens: &[usize] = match max_expr > 1 {
                    true => &[max_expr, 1],
                    false => &[1],
                };
                for &take in lens {
                    let mut fork = bindings.clone();
                    if !bind_or_verify(&mut fork, &name, source, &target[t_idx..t_idx + take]) {
                        continue;
                    }
                    if let Some(end_idx) = match_seq(
                        pat,
                        target,
                        source,
                        p_idx + 1,
                        t_idx + take,
                        &mut fork,
                        require_full_target,
                    ) {
                        *bindings = fork;
                        return Some(end_idx);
                    }
                }
                None
            }
            Some(MetaVarKind::Wildcard) => {
                if t_idx >= target.len() {
                    return None;
                }
                let max_expr = expr_atom_len(target, t_idx);
                let lens: &[usize] = match max_expr > 1 {
                    true => &[max_expr, 1],
                    false => &[1],
                };
                for &take in lens {
                    let mut fork = bindings.clone();
                    if let Some(end_idx) = match_seq(
                        pat,
                        target,
                        source,
                        p_idx + 1,
                        t_idx + take,
                        &mut fork,
                        require_full_target,
                    ) {
                        *bindings = fork;
                        return Some(end_idx);
                    }
                }
                None
            }
            None => match target.get(t_idx) {
                Some(TokenNode::Ident { text: t_text, .. }) if t_text == text => match_seq(
                    pat,
                    target,
                    source,
                    p_idx + 1,
                    t_idx + 1,
                    bindings,
                    require_full_target,
                ),
                _ => None,
            },
        },
        TokenNode::Literal { text, .. } => match target.get(t_idx) {
            Some(TokenNode::Literal { text: t_text, .. }) if t_text == text => match_seq(
                pat,
                target,
                source,
                p_idx + 1,
                t_idx + 1,
                bindings,
                require_full_target,
            ),
            _ => None,
        },
        TokenNode::Punct { text, .. } => match target.get(t_idx) {
            Some(TokenNode::Punct { text: t_text, .. }) if t_text == text => match_seq(
                pat,
                target,
                source,
                p_idx + 1,
                t_idx + 1,
                bindings,
                require_full_target,
            ),
            _ => None,
        },
        TokenNode::Group {
            delim,
            children: p_children,
            ..
        } => match target.get(t_idx) {
            Some(TokenNode::Group {
                delim: t_delim,
                children: t_children,
                ..
            }) if t_delim == delim => {
                let mut fork = bindings.clone();
                let p_norm = strip_trailing_comma(p_children);
                let t_norm = strip_trailing_comma(t_children);
                if match_seq(p_norm, t_norm, source, 0, 0, &mut fork, true).is_some()
                    && let Some(end_idx) = match_seq(
                        pat,
                        target,
                        source,
                        p_idx + 1,
                        t_idx + 1,
                        &mut fork,
                        require_full_target,
                    )
                {
                    *bindings = fork;
                    return Some(end_idx);
                }
                None
            }
            _ => None,
        },
    }
}

fn strip_trailing_comma(nodes: &[TokenNode]) -> &[TokenNode] {
    match nodes.last() {
        Some(TokenNode::Punct { text, .. }) if text == "," => &nodes[..nodes.len() - 1],
        _ => nodes,
    }
}

fn search_nodes_recursive(
    pat: &[TokenNode],
    nodes: &[TokenNode],
    source: &str,
    max_matches: usize,
    out: &mut Vec<StructuralMatch>,
) {
    let mut i = 0usize;
    while i < nodes.len() && out.len() < max_matches {
        let mut bindings = BTreeMap::new();
        if let Some(end_idx) = match_seq(pat, nodes, source, 0, i, &mut bindings, false)
            && end_idx > i
        {
            let (start_byte, _) = nodes[i].span();
            let (_, end_byte) = nodes[end_idx - 1].span();
            let (start_line, _) = nodes[i].lines();
            let (_, end_line) = nodes[end_idx - 1].lines();
            let flat_bindings: BTreeMap<String, String> = bindings
                .into_iter()
                .map(|(k, v)| (k, v.raw_text))
                .collect();
            out.push(StructuralMatch {
                start_byte,
                end_byte,
                start_line,
                end_line,
                matched_text: source.get(start_byte..end_byte).unwrap_or("").to_string(),
                bindings: flat_bindings,
            });
            i = end_idx;
            continue;
        }
        if let TokenNode::Group { children, .. } = &nodes[i] {
            search_nodes_recursive(pat, children, source, max_matches, out);
        }
        i += 1;
    }
}

/// Extracts the longest literal identifier anchor (`>= 3` chars) from a structural pattern so
/// `code_refactor` can accelerate workspace scans with the `tgrep` trigram index.
pub fn extract_structural_pattern_anchor(spec: &LanguageSyntaxSpec, pattern: &str) -> Option<String> {
    lex_code_tokens(spec, pattern)
        .into_iter()
        .filter(|t| t.kind == FlatTokenKind::Ident && parse_metavar(&t.text).is_none())
        .map(|t| t.text)
        .filter(|s| s.len() >= 3)
        .max_by_key(|s| s.len())
}

/// Runs comment- and formatting-insensitive structural metavariable search on `source`.
pub fn search_structural(
    spec: &LanguageSyntaxSpec,
    source: &str,
    pattern: &str,
    max_matches: usize,
) -> Result<Vec<StructuralMatch>> {
    let pat_tree = lex_token_tree(spec, pattern);
    if pat_tree.is_empty() {
        return Err(ZenkoError::Tool(
            "structural `pattern` parsed to zero code tokens".into(),
        ));
    }
    let src_tree = lex_token_tree(spec, source);
    let mut matches = Vec::new();
    search_nodes_recursive(&pat_tree, &src_tree, source, max_matches.max(1), &mut matches);
    Ok(matches)
}

/// Expands `$NAME` and `$$$NAME` template placeholders using captured bindings.
pub fn expand_rewrite_template(template: &str, bindings: &BTreeMap<String, String>) -> String {
    let bytes = template.as_bytes();
    let mut out = String::with_capacity(template.len() + 32);
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_cont(bytes[i]) {
                i += 1;
            }
            let tok = &template[start..i];
            match parse_metavar(tok) {
                Some(MetaVarKind::Single(name) | MetaVarKind::Variadic(name)) => {
                    match bindings.get(&name) {
                        Some(val) => out.push_str(val),
                        None => out.push_str(tok),
                    }
                }
                _ => out.push_str(tok),
            }
            continue;
        }
        let ch_len = template[i..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(1);
        out.push_str(&template[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Applies non-overlapping structural metavariable rewrites right-to-left, preserving surrounding formatting.
pub fn rewrite_structural(
    spec: &LanguageSyntaxSpec,
    source: &str,
    pattern: &str,
    template: &str,
) -> Result<(String, usize)> {
    let matches = search_structural(spec, source, pattern, usize::MAX / 4)?;
    if matches.is_empty() {
        return Ok((source.to_string(), 0));
    }
    let mut out = source.to_string();
    let count = matches.len();
    for m in matches.into_iter().rev() {
        let replacement = expand_rewrite_template(template, &m.bindings);
        out.replace_range(m.start_byte..m.end_byte, &replacement);
    }
    Ok((out, count))
}

// ─── 2. `sd` / `ripide`-Inspired Code-Only Replace & Relative Import Rewriter ─

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Replaces literal or identifier-boundary occurrences in `source`.
/// When `code_only == true`, skips comments and string literals using the shared language lexer FSM!
pub fn replace_in_source(
    spec: &LanguageSyntaxSpec,
    source: &str,
    find: &str,
    replace: &str,
    whole_word: bool,
    code_only: bool,
) -> (String, usize) {
    if find.is_empty() {
        return (source.to_string(), 0);
    }

    if code_only {
        let tokens = lex_code_tokens(spec, source);
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for tok in tokens {
            let matches_tok = match (tok.kind, whole_word) {
                (FlatTokenKind::Ident, true) => tok.text == find,
                (FlatTokenKind::Ident, false) => tok.text.contains(find),
                _ => false,
            };
            if matches_tok {
                if whole_word {
                    spans.push((tok.start, tok.end));
                } else {
                    let mut offset = 0usize;
                    while let Some(pos) = tok.text[offset..].find(find) {
                        let abs_s = tok.start + offset + pos;
                        spans.push((abs_s, abs_s + find.len()));
                        offset += pos + find.len();
                    }
                }
            }
        }
        if spans.is_empty() {
            return (source.to_string(), 0);
        }
        let mut out = source.to_string();
        let count = spans.len();
        for (s, e) in spans.into_iter().rev() {
            out.replace_range(s..e, replace);
        }
        return (out, count);
    }

    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    let mut count = 0usize;
    while let Some(rel) = source[cursor..].find(find) {
        let start = cursor + rel;
        let end = start + find.len();
        let boundary_ok = match whole_word {
            false => true,
            true => {
                let left_ok = start == 0 || !is_word_byte(bytes[start - 1]);
                let right_ok = end == bytes.len() || !is_word_byte(bytes[end]);
                left_ok && right_ok
            }
        };
        if boundary_ok {
            out.push_str(&source[cursor..start]);
            out.push_str(replace);
            count += 1;
            cursor = end;
        } else {
            out.push_str(&source[cursor..start + 1]);
            cursor = start + 1;
        }
    }
    out.push_str(&source[cursor..]);
    (out, count)
}

fn normalize_rel_path(path: &Path) -> PathBuf {
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(seg) => parts.push(seg.to_os_string()),
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    parts.into_iter().collect()
}

fn relative_specifier(from_dir: &Path, to_file: &Path, keep_ext: Option<&str>) -> String {
    let from_parts: Vec<&std::ffi::OsStr> = from_dir
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s),
            _ => None,
        })
        .collect();
    let to_parts: Vec<&std::ffi::OsStr> = to_file
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s),
            _ => None,
        })
        .collect();

    let common = from_parts
        .iter()
        .zip(to_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let ups = from_parts.len().saturating_sub(common);
    let mut segs: Vec<String> = Vec::new();
    match ups {
        0 => segs.push(".".to_string()),
        n => {
            for _ in 0..n {
                segs.push("..".to_string());
            }
        }
    }
    for part in &to_parts[common..] {
        segs.push(part.to_string_lossy().to_string());
    }
    let mut joined = segs.join("/");
    if keep_ext.is_none()
        && let Some(ext) = to_file.extension().and_then(|e| e.to_str())
    {
        let dot_ext = format!(".{ext}");
        if let Some(stripped) = joined.strip_suffix(&dot_ext) {
            joined = stripped.to_string();
        }
    }
    joined
}

fn matches_moved_target(resolved: &Path, old_rel: &Path) -> bool {
    if resolved == old_rel {
        return true;
    }
    let old_no_ext = old_rel.with_extension("");
    let res_no_ext = resolved.with_extension("");
    res_no_ext == old_no_ext
        && (resolved.extension().is_none()
            || matches!(
                resolved.extension().and_then(|e| e.to_str()),
                Some("js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs")
            ))
}

/// Deterministically rewrites relative import/require string literals in `source` when
/// `old_rel` moves to `new_rel`, preserving quotes and extension style (`.js`/extensionless).
pub fn rewrite_relative_imports_for_move(
    referencing_rel: &Path,
    old_rel: &Path,
    new_rel: &Path,
    spec: &LanguageSyntaxSpec,
    source: &str,
) -> (String, usize) {
    let tokens = lex_code_tokens(spec, source);
    let ref_dir = referencing_rel.parent().unwrap_or_else(|| Path::new(""));
    let mut edits: Vec<(usize, usize, String)> = Vec::new();

    for (idx, tok) in tokens.iter().enumerate() {
        if tok.kind != FlatTokenKind::Literal || tok.text.len() < 4 {
            continue;
        }
        let quote = tok.text.as_bytes()[0];
        if !matches!(quote, b'"' | b'\'') || tok.text.as_bytes()[tok.text.len() - 1] != quote {
            continue;
        }
        let inner = &tok.text[1..tok.text.len() - 1];
        if !inner.starts_with("./") && !inner.starts_with("../") {
            continue;
        }
        // Only rewrite literals immediately adjacent to import/export/from/require context.
        let prev_is_import = idx > 0
            && match &tokens[idx - 1] {
                prev_tok if matches!(prev_tok.text.as_str(), "from" | "import") => true,
                p if p.kind == FlatTokenKind::OpenDelim
                    && idx >= 2
                    && matches!(tokens[idx - 2].text.as_str(), "require" | "import") =>
                {
                    true
                }
                _ => false,
            };
        if !prev_is_import {
            continue;
        }

        let candidate = normalize_rel_path(&ref_dir.join(inner));
        if matches_moved_target(&candidate, old_rel) {
            let keep_ext = Path::new(inner).extension().and_then(|e| e.to_str());
            let mut next_spec = relative_specifier(ref_dir, new_rel, keep_ext);
            // Preserve ESM `.js` suffix when moving a `.ts` file imported with `.js`
            if let Some(orig_ext) = keep_ext
                && let Some(new_ext) = new_rel.extension().and_then(|e| e.to_str())
                && orig_ext != new_ext
            {
                let suffix = format!(".{new_ext}");
                if let Some(base) = next_spec.strip_suffix(&suffix) {
                    next_spec = format!("{base}.{orig_ext}");
                }
            }
            let q = quote as char;
            edits.push((tok.start, tok.end, format!("{q}{next_spec}{q}")));
        }
    }

    if edits.is_empty() {
        return (source.to_string(), 0);
    }
    let count = edits.len();
    let mut out = source.to_string();
    for (s, e, rep) in edits.into_iter().rev() {
        out.replace_range(s..e, &rep);
    }
    (out, count)
}

// ─── 3. `difftastic`-Inspired Prefix/Suffix Pruning + Weighted Dijkstra Diff ──

#[derive(Debug, Clone, Serialize)]
pub struct SemanticDiffHunk {
    pub op: &'static str,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub old_tokens: String,
    pub new_tokens: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StructuralDiffReport {
    pub semantic_changes: usize,
    pub formatting_only: bool,
    pub truncated: bool,
    pub hunks: Vec<SemanticDiffHunk>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditStep {
    Match,
    Replace,
    Delete,
    Insert,
}

fn substitution_cost(a: &FlatToken, b: &FlatToken) -> u32 {
    match (a.kind == b.kind, a.text == b.text) {
        (true, true) => 0,
        (true, false) => 2,
        (false, _) => 5,
    }
}

fn strip_formatting_trailing_commas(tokens: Vec<FlatToken>) -> Vec<FlatToken> {
    let mut out = Vec::with_capacity(tokens.len());
    for idx in 0..tokens.len() {
        let is_trailing_comma = tokens[idx].kind == FlatTokenKind::Punct
            && tokens[idx].text == ","
            && tokens
                .get(idx + 1)
                .is_some_and(|next| next.kind == FlatTokenKind::CloseDelim);
        if !is_trailing_comma {
            out.push(tokens[idx].clone());
        }
    }
    out
}

/// Computes a weighted shortest-path token diff via Dijkstra's algorithm on the pruned edit graph.
pub fn compute_structural_diff(
    spec: &LanguageSyntaxSpec,
    before: &str,
    after: &str,
) -> StructuralDiffReport {
    let lhs = strip_formatting_trailing_commas(lex_code_tokens(spec, before));
    let rhs = strip_formatting_trailing_commas(lex_code_tokens(spec, after));

    // Phase 1: Prune identical prefix & suffix in O(N) (Difftastic boundary pruning)
    let prefix = lhs
        .iter()
        .zip(rhs.iter())
        .take_while(|(a, b)| a.kind == b.kind && a.text == b.text)
        .count();
    let rem_l = lhs.len() - prefix;
    let rem_r = rhs.len() - prefix;
    let suffix = lhs[prefix..]
        .iter()
        .rev()
        .zip(rhs[prefix..].iter().rev())
        .take(rem_l.min(rem_r))
        .take_while(|(a, b)| a.kind == b.kind && a.text == b.text)
        .count();

    let mid_l = &lhs[prefix..lhs.len() - suffix];
    let mid_r = &rhs[prefix..rhs.len() - suffix];

    if mid_l.is_empty() && mid_r.is_empty() {
        return StructuralDiffReport {
            semantic_changes: 0,
            formatting_only: before != after,
            truncated: false,
            hunks: Vec::new(),
        };
    }

    // Bounded Dijkstra shortest-path edit graph when within token cap
    if mid_l.len().saturating_add(mid_r.len()) > STRUCT_DIFF_MAX_TOKENS_DIJKSTRA {
        let hunks = vec![SemanticDiffHunk {
            op: "replace",
            old_line: mid_l.first().map(|t| t.line),
            new_line: mid_r.first().map(|t| t.line),
            old_tokens: format!("({} tokens changed)", mid_l.len()),
            new_tokens: format!("({} tokens changed)", mid_r.len()),
        }];
        return StructuralDiffReport {
            semantic_changes: mid_l.len().max(mid_r.len()),
            formatting_only: false,
            truncated: true,
            hunks,
        };
    }

    let rows = mid_l.len() + 1;
    let cols = mid_r.len() + 1;
    let total = rows * cols;
    let mut dist = vec![u32::MAX; total];
    let mut prev: Vec<Option<EditStep>> = vec![None; total];
    let mut heap: BinaryHeap<Reverse<(u32, usize, usize)>> = BinaryHeap::new();

    dist[0] = 0;
    heap.push(Reverse((0, 0, 0)));

    while let Some(Reverse((cost, i, j))) = heap.pop() {
        let idx = i * cols + j;
        if cost > dist[idx] {
            continue;
        }
        if i == mid_l.len() && j == mid_r.len() {
            break;
        }
        // Diagonal (Match / Substitute)
        if i < mid_l.len() && j < mid_r.len() {
            let step_cost = substitution_cost(&mid_l[i], &mid_r[j]);
            let next_cost = cost.saturating_add(step_cost);
            let n_idx = (i + 1) * cols + (j + 1);
            if next_cost < dist[n_idx] {
                dist[n_idx] = next_cost;
                prev[n_idx] = Some(match step_cost {
                    0 => EditStep::Match,
                    _ => EditStep::Replace,
                });
                heap.push(Reverse((next_cost, i + 1, j + 1)));
            }
        }
        // Delete from left
        if i < mid_l.len() {
            let next_cost = cost.saturating_add(3);
            let n_idx = (i + 1) * cols + j;
            if next_cost < dist[n_idx] {
                dist[n_idx] = next_cost;
                prev[n_idx] = Some(EditStep::Delete);
                heap.push(Reverse((next_cost, i + 1, j)));
            }
        }
        // Insert into right
        if j < mid_r.len() {
            let next_cost = cost.saturating_add(3);
            let n_idx = i * cols + (j + 1);
            if next_cost < dist[n_idx] {
                dist[n_idx] = next_cost;
                prev[n_idx] = Some(EditStep::Insert);
                heap.push(Reverse((next_cost, i, j + 1)));
            }
        }
    }

    // Reconstruct optimal path
    let mut ops: Vec<(EditStep, Option<&FlatToken>, Option<&FlatToken>)> = Vec::new();
    let (mut ci, mut cj) = (mid_l.len(), mid_r.len());
    while ci > 0 || cj > 0 {
        match prev[ci * cols + cj] {
            Some(step @ (EditStep::Match | EditStep::Replace)) => {
                ci -= 1;
                cj -= 1;
                ops.push((step, Some(&mid_l[ci]), Some(&mid_r[cj])));
            }
            Some(EditStep::Delete) => {
                ci -= 1;
                ops.push((EditStep::Delete, Some(&mid_l[ci]), None));
            }
            Some(EditStep::Insert) => {
                cj -= 1;
                ops.push((EditStep::Insert, None, Some(&mid_r[cj])));
            }
            None => break,
        }
    }
    ops.reverse();

    // Group contiguous non-match runs into concise semantic hunks
    let mut hunks: Vec<SemanticDiffHunk> = Vec::new();
    let mut semantic_changes = 0usize;
    let mut k = 0usize;
    while k < ops.len() {
        if ops[k].0 == EditStep::Match {
            k += 1;
            continue;
        }
        let mut old_parts: Vec<&str> = Vec::new();
        let mut new_parts: Vec<&str> = Vec::new();
        let mut old_line: Option<usize> = None;
        let mut new_line: Option<usize> = None;
        while k < ops.len() && ops[k].0 != EditStep::Match {
            semantic_changes += 1;
            if let Some(lt) = ops[k].1 {
                old_line.get_or_insert(lt.line);
                old_parts.push(&lt.text);
            }
            if let Some(rt) = ops[k].2 {
                new_line.get_or_insert(rt.line);
                new_parts.push(&rt.text);
            }
            k += 1;
        }
        let op = match (old_parts.is_empty(), new_parts.is_empty()) {
            (false, false) => "replace",
            (false, true) => "delete",
            (true, false) => "insert",
            (true, true) => "noop",
        };
        if hunks.len() < STRUCT_DIFF_MAX_EDITS_RENDER {
            hunks.push(SemanticDiffHunk {
                op,
                old_line,
                new_line,
                old_tokens: old_parts.join(" "),
                new_tokens: new_parts.join(" "),
            });
        }
    }

    StructuralDiffReport {
        semantic_changes,
        formatting_only: semantic_changes == 0 && before != after,
        truncated: semantic_changes > STRUCT_DIFF_MAX_EDITS_RENDER,
        hunks,
    }
}

pub fn format_structural_diff(label: &str, report: &StructuralDiffReport) -> String {
    if report.semantic_changes == 0 {
        return match report.formatting_only {
            true => format!("{label}: 0 semantic changes (formatting/whitespace/comment-only diff)"),
            false => format!("{label}: identical"),
        };
    }
    let mut out = vec![format!(
        "{label}: {} semantic token change(s) across {} hunk(s){}:",
        report.semantic_changes,
        report.hunks.len(),
        match report.truncated {
            true => " (truncated)",
            false => "",
        }
    )];
    for h in &report.hunks {
        let loc = match (h.old_line, h.new_line) {
            (Some(a), Some(b)) if a == b => format!("L{a}"),
            (Some(a), Some(b)) => format!("L{a}->L{b}"),
            (Some(a), None) => format!("L{a}"),
            (None, Some(b)) => format!("+L{b}"),
            (None, None) => "L?".to_string(),
        };
        let detail = match h.op {
            "replace" => format!("`{}` => `{}`", h.old_tokens, h.new_tokens),
            "delete" => format!("- `{}`", h.old_tokens),
            "insert" => format!("+ `{}`", h.new_tokens),
            _ => String::new(),
        };
        out.push(format!("  [{loc}] {}: {detail}", h.op));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ast_grep_metavar_unification_multiline_and_rewrite_works() {
        let rust = find_language_syntax_by_ext("rs").expect("rs");
        let src = r#"
fn main() {
    old_call(
        user.id, // inline comment ignored
        compute(1, 2),
    );
    assert_eq!(alpha, alpha);
    assert_eq!(alpha, beta);
}
"#;
        // 1. Multi-line wrapped + commented call matches single-line pattern and rewrites cleanly
        let (rewritten, n) = rewrite_structural(rust, src, "old_call($A, $B)", "new_call($B, $A)")
            .expect("rewrite");
        assert_eq!(n, 1);
        assert!(rewritten.contains("new_call(compute(1, 2), user.id);"));

        // 2. Repeated metavariable enforces structural equality ($X == $X)
        let eq_hits = search_structural(rust, src, "assert_eq!($X, $X)", 10).expect("eq search");
        assert_eq!(eq_hits.len(), 1);
        assert_eq!(eq_hits[0].bindings.get("X").map(String::as_str), Some("alpha"));

        // 3. Variadic $$$ARGS matches multi-arg lists
        let var_hits = search_structural(rust, src, "compute($$$ARGS)", 10).expect("var search");
        assert_eq!(var_hits.len(), 1);
        assert_eq!(
            var_hits[0].bindings.get("ARGS").map(String::as_str),
            Some("1, 2")
        );
    }

    #[test]
    fn code_only_replace_skips_strings_and_comments_and_move_rewrites_imports() {
        let ts = find_language_syntax_by_ext("ts").expect("ts");
        let code = "// keep oldName in comment\nconst msg = \"oldName\";\nconst x = oldName + oldNameOther;\n";
        let (updated, count) = replace_in_source(ts, code, "oldName", "newName", true, true);
        assert_eq!(count, 1);
        assert!(updated.contains("// keep oldName in comment"));
        assert!(updated.contains("\"oldName\""));
        assert!(updated.contains("const x = newName + oldNameOther;"));

        let importer = "import { foo } from './utils/tax'\nconst s = './utils/tax'\n";
        let (moved_imp, imp_cnt) = rewrite_relative_imports_for_move(
            Path::new("src/index.ts"),
            Path::new("src/utils/tax.ts"),
            Path::new("src/billing/tax.ts"),
            ts,
            importer,
        );
        assert_eq!(imp_cnt, 1);
        assert!(moved_imp.contains("import { foo } from './billing/tax'"));
        assert!(moved_imp.contains("const s = './utils/tax'"));
    }

    #[test]
    fn difftastic_dijkstra_diff_ignores_formatting_and_isolates_semantic_edits() {
        let rust = find_language_syntax_by_ext("rs").expect("rs");
        let before = "pub fn add(a: i32, b: i32) -> i32 { a + b }\n";
        let reformatted = "pub fn add(\n    a: i32,\n    b: i32,\n) -> i32 {\n    // comment only\n    a + b\n}\n";
        let fmt_rep = compute_structural_diff(rust, before, reformatted);
        assert_eq!(fmt_rep.semantic_changes, 0);
        assert!(fmt_rep.formatting_only);

        let changed = "pub fn add(\n    a: i32,\n    b: i32,\n) -> i32 {\n    a * b\n}\n";
        let sem_rep = compute_structural_diff(rust, before, changed);
        assert_eq!(sem_rep.semantic_changes, 1);
        assert!(!sem_rep.formatting_only);
        assert_eq!(sem_rep.hunks[0].old_tokens, "+");
        assert_eq!(sem_rep.hunks[0].new_tokens, "*");
    }
}
