//! Pure-Rust, zero-regex Markdown-aware English document compressor.
//! Replaces Python `compress.py` scripts natively inside Zenko: compresses prose in
//! `MEMORY.md`, `USER.md`, `PROJECT_MEMORY.md`, and `ZENKO.md` using a single-pass O(N)
//! longest-match phrase engine while strictly locking YAML frontmatter, headings,
//! fenced code blocks, Markdown tables, inline code spans, Markdown links, double-quoted
//! literals, URLs, file paths, CLI flags, and uppercase identifiers verbatim, followed by
//! a structural integrity validator before any bytes are written.

use zenko_core::{
    ConciseMode, DOC_COMPRESS_LITE_FILLERS, DOC_COMPRESS_LITE_PHRASES,
    DOC_COMPRESS_PROTECTED_PREFIXES, DOC_COMPRESS_TELEGRAPHIC_STOPWORDS,
    DOC_COMPRESS_ULTRA_PHRASES, Result, ZenkoError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MdBlockState {
    Normal,
    Frontmatter,
    FencedCode { marker: u8, len: usize },
}

#[derive(Debug, Clone)]
pub struct MarkdownCompressionOutcome {
    pub compressed: String,
    pub original_bytes: usize,
    pub compressed_bytes: usize,
    pub bytes_saved: usize,
    pub tokens_saved: usize,
    pub savings_pct: f64,
    pub headings_preserved: usize,
    pub code_blocks_preserved: usize,
}

/// Detects a Markdown fence opener (` ``` ` or `~~~`) after optional up-to-3 leading spaces.
fn parse_fence_marker(line: &str) -> Option<(u8, usize)> {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && i < 3 && bytes[i] == b' ' {
        i += 1;
    }
    let ch = *bytes.get(i)?;
    if !matches!(ch, b'`' | b'~') {
        return None;
    }
    let mut len = 0usize;
    while i + len < bytes.len() && bytes[i + len] == ch {
        len += 1;
    }
    match len >= 3 {
        true => Some((ch, len)),
        false => None,
    }
}

fn is_closing_fence(line: &str, marker: u8, min_len: usize) -> bool {
    let trimmed = line.trim();
    trimmed.len() >= min_len && trimmed.bytes().all(|b| b == marker)
}

fn is_heading_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    (1..=6).contains(&hashes) && trimmed.as_bytes().get(hashes) == Some(&b' ')
}

fn is_table_or_rule_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|')
        || (trimmed.len() >= 3
            && (trimmed.bytes().all(|b| b == b'-' || b == b' ')
                || trimmed.bytes().all(|b| b == b'*' || b == b' ')
                || trimmed.bytes().all(|b| b == b'_' || b == b' ')))
}

/// Splits a prose line into `(preserved_prefix, body)` preserving list markers (`- `, `* `, `1. `, `> `).
fn split_markdown_line_prefix(line: &str) -> (&str, &str) {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }
    while i < bytes.len() && bytes[i] == b'>' {
        i += 1;
        if i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
    }
    if i + 1 < bytes.len() && matches!(bytes[i], b'-' | b'*' | b'+') && bytes[i + 1] == b' ' {
        let mut end = i + 2;
        if line[end..].starts_with("[ ] ")
            || line[end..].starts_with("[x] ")
            || line[end..].starts_with("[X] ")
        {
            end += 4;
        }
        return (&line[..end], &line[end..]);
    }
    let digit_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > digit_start
        && i + 1 < bytes.len()
        && matches!(bytes[i], b'.' | b')')
        && bytes[i + 1] == b' '
    {
        return (&line[..i + 2], &line[i + 2..]);
    }
    (&line[..digit_start], &line[digit_start..])
}

#[derive(Debug, Clone, Copy)]
enum InlineSegment<'a> {
    Prose(&'a str),
    Verbatim(&'a str),
}

/// Deterministic finite-state inline tokenizer protecting `` `code` ``, `[label](url)`,
/// and double-quoted `"literals"` (honoring `\"` escapes) verbatim.
fn segment_inline_line(body: &str) -> Vec<InlineSegment<'_>> {
    let bytes = body.as_bytes();
    let mut segments: Vec<InlineSegment<'_>> = Vec::new();
    let mut i = 0usize;
    let mut prose_start = 0usize;

    while i < bytes.len() {
        match bytes[i] {
            b'`' => {
                let tick_start = i;
                while i < bytes.len() && bytes[i] == b'`' {
                    i += 1;
                }
                let tick_len = i - tick_start;
                let mut found_end = None;
                let mut scan = i;
                while scan < bytes.len() {
                    if bytes[scan] == b'`' {
                        let c_start = scan;
                        while scan < bytes.len() && bytes[scan] == b'`' {
                            scan += 1;
                        }
                        if scan - c_start == tick_len {
                            found_end = Some(scan);
                            break;
                        }
                    } else {
                        scan += 1;
                    }
                }
                match found_end {
                    Some(end) => {
                        if tick_start > prose_start {
                            segments.push(InlineSegment::Prose(&body[prose_start..tick_start]));
                        }
                        segments.push(InlineSegment::Verbatim(&body[tick_start..end]));
                        i = end;
                        prose_start = i;
                    }
                    None => {
                        i = tick_start + tick_len;
                    }
                }
            }
            b'"' => {
                let quote_start = i;
                let mut scan = i + 1;
                let mut found_end = None;
                while scan < bytes.len() {
                    match bytes[scan] {
                        b'\\' if scan + 1 < bytes.len() => scan += 2,
                        b'"' => {
                            found_end = Some(scan + 1);
                            break;
                        }
                        _ => scan += 1,
                    }
                }
                match found_end {
                    Some(end) if end > quote_start + 1 => {
                        if quote_start > prose_start {
                            segments.push(InlineSegment::Prose(&body[prose_start..quote_start]));
                        }
                        segments.push(InlineSegment::Verbatim(&body[quote_start..end]));
                        i = end;
                        prose_start = i;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            b'[' => {
                if let Some(close_bracket) = body[i + 1..].find(']') {
                    let after_bracket = i + 1 + close_bracket + 1;
                    if bytes.get(after_bracket) == Some(&b'(')
                        && let Some(close_paren) = body[after_bracket + 1..].find(')')
                    {
                        let end = after_bracket + 1 + close_paren + 1;
                        if i > prose_start {
                            segments.push(InlineSegment::Prose(&body[prose_start..i]));
                        }
                        segments.push(InlineSegment::Verbatim(&body[i..end]));
                        i = end;
                        prose_start = i;
                        continue;
                    }
                }
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    if bytes.len() > prose_start {
        segments.push(InlineSegment::Prose(&body[prose_start..]));
    }
    segments
}

fn is_protected_token(tok: &str) -> bool {
    let trimmed = tok.trim_matches(|c: char| matches!(c, '(' | ')' | ',' | '.' | ';' | ':'));
    if trimmed.is_empty() {
        return true;
    }
    DOC_COMPRESS_PROTECTED_PREFIXES
        .iter()
        .any(|p| trimmed.starts_with(p))
        || trimmed.contains('/')
        || trimmed.contains("::")
        || trimmed.contains('_')
        || trimmed.bytes().any(|b| b.is_ascii_digit())
        || (trimmed.len() >= 2
            && trimmed
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b == b'-'))
}

fn is_word_boundary_byte(b: Option<u8>) -> bool {
    match b {
        None => true,
        Some(ch) => !(ch.is_ascii_alphanumeric() || ch == b'_'),
    }
}

/// Single-pass O(N) longest-match phrase replacer across one or more declarative phrase tables.
/// Allocates a single lowercase shadow buffer and resolves overlapping phrases deterministically
/// by longest needle match at each word boundary (zero multi-pass rescanning, zero regex).
fn apply_phrase_tables_single_pass(input: &str, tables: &[&[(&str, &str)]]) -> String {
    let lower = input.to_ascii_lowercase();
    let lower_bytes = lower.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut pos = 0usize;

    while pos < lower.len() {
        let at_boundary = is_word_boundary_byte(
            pos.checked_sub(1)
                .and_then(|p| lower_bytes.get(p).copied()),
        );
        let best_match = match at_boundary && input.is_char_boundary(pos) {
            true => {
                let tail = &lower[pos..];
                let mut best: Option<(usize, &str)> = None;
                for &table in tables {
                    for &(needle, replacement) in table {
                        let n_len = needle.len();
                        if n_len == 0 || !tail.starts_with(needle) {
                            continue;
                        }
                        let end = pos + n_len;
                        let end_ok = needle.ends_with(' ')
                            || is_word_boundary_byte(lower_bytes.get(end).copied());
                        if end_ok && input.is_char_boundary(end) {
                            let is_longer = best.is_none_or(|(prev_len, _)| n_len > prev_len);
                            if is_longer {
                                best = Some((n_len, replacement));
                            }
                        }
                    }
                }
                best
            }
            false => None,
        };

        match best_match {
            Some((len, replacement)) => {
                out.push_str(replacement);
                pos += len;
            }
            None => {
                let ch_len = input[pos..].chars().next().map_or(1, char::len_utf8);
                out.push_str(&input[pos..pos + ch_len]);
                pos += ch_len;
            }
        }
    }
    out
}

fn compress_prose_segment(seg: &str, mode: ConciseMode) -> String {
    if !mode.is_active() {
        return seg.to_string();
    }
    let lead_space = seg.starts_with(char::is_whitespace);
    let trail_space = seg.ends_with(char::is_whitespace);

    let tables: &[&[(&str, &str)]] = match mode {
        ConciseMode::Off => &[],
        ConciseMode::Lite | ConciseMode::Full => &[DOC_COMPRESS_LITE_PHRASES],
        ConciseMode::Ultra => &[DOC_COMPRESS_LITE_PHRASES, DOC_COMPRESS_ULTRA_PHRASES],
    };
    let work = apply_phrase_tables_single_pass(seg, tables);

    let stopwords = match mode {
        ConciseMode::Off => &[][..],
        ConciseMode::Lite => DOC_COMPRESS_LITE_FILLERS,
        ConciseMode::Full | ConciseMode::Ultra => DOC_COMPRESS_TELEGRAPHIC_STOPWORDS,
    };

    let mut kept: Vec<&str> = Vec::new();
    for raw_tok in work.split_whitespace() {
        if is_protected_token(raw_tok) {
            kept.push(raw_tok);
            continue;
        }
        let core = raw_tok
            .trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '(' | ')'))
            .to_ascii_lowercase();
        let has_terminal_punct =
            raw_tok.ends_with('.') || raw_tok.ends_with('?') || raw_tok.ends_with('!');
        let drop_word = !has_terminal_punct
            && !raw_tok.starts_with("**")
            && !raw_tok.ends_with("**")
            && stopwords.binary_search(&core.as_str()).is_ok();
        if !drop_word {
            kept.push(raw_tok);
        }
    }

    let joined = kept.join(" ");
    let mut out = String::with_capacity(joined.len() + 2);
    if lead_space && !joined.is_empty() {
        out.push(' ');
    }
    out.push_str(&joined);
    if trail_space && !joined.is_empty() {
        out.push(' ');
    }
    out
}

fn compress_prose_line(line: &str, mode: ConciseMode) -> String {
    let (prefix, body) = split_markdown_line_prefix(line);
    if body.trim().is_empty() {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len());
    out.push_str(prefix);
    for seg in segment_inline_line(body) {
        match seg {
            InlineSegment::Verbatim(v) => out.push_str(v),
            InlineSegment::Prose(p) => out.push_str(&compress_prose_segment(p, mode)),
        }
    }
    let tail = out[prefix.len()..].trim_end();
    let mut collapsed = String::with_capacity(prefix.len() + tail.len());
    collapsed.push_str(prefix);
    let mut prev_space = false;
    for ch in tail.chars() {
        match ch == ' ' {
            true if prev_space => {}
            true => {
                collapsed.push(' ');
                prev_space = true;
            }
            false => {
                collapsed.push(ch);
                prev_space = false;
            }
        }
    }
    collapsed
}

struct StructuralDigest {
    headings: Vec<String>,
    code_blocks: Vec<String>,
    verbatim_spans: Vec<String>,
    urls: Vec<String>,
}

fn extract_structural_digest(doc: &str) -> StructuralDigest {
    let mut headings = Vec::new();
    let mut code_blocks = Vec::new();
    let mut verbatim_spans = Vec::new();
    let mut urls = Vec::new();
    let mut state = MdBlockState::Normal;
    let mut current_code = String::new();

    for (idx, line) in doc.lines().enumerate() {
        match state {
            MdBlockState::Normal => {
                if idx == 0 && line.trim() == "---" {
                    state = MdBlockState::Frontmatter;
                    continue;
                }
                if let Some((marker, len)) = parse_fence_marker(line) {
                    state = MdBlockState::FencedCode { marker, len };
                    current_code.clear();
                    current_code.push_str(line);
                    current_code.push('\n');
                    continue;
                }
                if is_heading_line(line) {
                    headings.push(line.trim().to_string());
                    continue;
                }
                for seg in segment_inline_line(line) {
                    if let InlineSegment::Verbatim(v) = seg {
                        verbatim_spans.push(v.to_string());
                    }
                }
                for tok in line.split_whitespace() {
                    let clean = tok.trim_matches(|c: char| {
                        matches!(c, '(' | ')' | ',' | '.' | ';' | '<' | '>' | '"')
                    });
                    if clean.starts_with("http://") || clean.starts_with("https://") {
                        urls.push(clean.to_string());
                    }
                }
            }
            MdBlockState::Frontmatter => {
                if line.trim() == "---" {
                    state = MdBlockState::Normal;
                }
            }
            MdBlockState::FencedCode { marker, len } => {
                current_code.push_str(line);
                current_code.push('\n');
                if is_closing_fence(line, marker, len) {
                    code_blocks.push(std::mem::take(&mut current_code));
                    state = MdBlockState::Normal;
                }
            }
        }
    }
    if !current_code.is_empty() {
        code_blocks.push(current_code);
    }

    StructuralDigest {
        headings,
        code_blocks,
        verbatim_spans,
        urls,
    }
}

/// Compresses an English Markdown document deterministically while preserving all code blocks,
/// headings, tables, frontmatter, inline code spans, quoted literals, paths, and URLs verbatim.
pub fn compress_markdown_doc(input: &str, mode: ConciseMode) -> Result<MarkdownCompressionOutcome> {
    let original_bytes = input.len();
    if !mode.is_active() || input.trim().is_empty() {
        let digest = extract_structural_digest(input);
        return Ok(MarkdownCompressionOutcome {
            compressed: input.to_string(),
            original_bytes,
            compressed_bytes: original_bytes,
            bytes_saved: 0,
            tokens_saved: 0,
            savings_pct: 0.0,
            headings_preserved: digest.headings.len(),
            code_blocks_preserved: digest.code_blocks.len(),
        });
    }

    let before_digest = extract_structural_digest(input);
    let mut state = MdBlockState::Normal;
    let mut out_lines: Vec<String> = Vec::new();
    let mut consecutive_blank = 0usize;

    for (idx, line) in input.lines().enumerate() {
        match state {
            MdBlockState::Normal => {
                if idx == 0 && line.trim() == "---" {
                    state = MdBlockState::Frontmatter;
                    out_lines.push(line.to_string());
                    consecutive_blank = 0;
                    continue;
                }
                if let Some((marker, len)) = parse_fence_marker(line) {
                    state = MdBlockState::FencedCode { marker, len };
                    out_lines.push(line.to_string());
                    consecutive_blank = 0;
                    continue;
                }
                if line.trim().is_empty() {
                    consecutive_blank += 1;
                    if consecutive_blank <= 1 && !out_lines.is_empty() {
                        out_lines.push(String::new());
                    }
                    continue;
                }
                consecutive_blank = 0;
                if is_heading_line(line) || is_table_or_rule_line(line) {
                    out_lines.push(line.to_string());
                } else {
                    out_lines.push(compress_prose_line(line, mode));
                }
            }
            MdBlockState::Frontmatter => {
                out_lines.push(line.to_string());
                if line.trim() == "---" {
                    state = MdBlockState::Normal;
                }
            }
            MdBlockState::FencedCode { marker, len } => {
                out_lines.push(line.to_string());
                if is_closing_fence(line, marker, len) {
                    state = MdBlockState::Normal;
                }
            }
        }
    }

    while out_lines.last().is_some_and(|l| l.is_empty()) {
        out_lines.pop();
    }
    let mut compressed = out_lines.join("\n");
    if input.ends_with('\n') && !compressed.ends_with('\n') {
        compressed.push('\n');
    }

    // Strict structural integrity gate: refuse to return a transform that alters any heading,
    // fenced code block, inline code span, quoted literal, link, or URL.
    let after_digest = extract_structural_digest(&compressed);
    if before_digest.headings != after_digest.headings
        || before_digest.code_blocks != after_digest.code_blocks
        || before_digest.verbatim_spans != after_digest.verbatim_spans
        || before_digest.urls != after_digest.urls
    {
        return Err(ZenkoError::Tool(
            "Markdown compressor structural validation failed; original document preserved untouched."
                .into(),
        ));
    }

    let compressed_bytes = compressed.len();
    let bytes_saved = original_bytes.saturating_sub(compressed_bytes);
    let tokens_saved = bytes_saved.div_ceil(4);
    let savings_pct = match original_bytes > 0 {
        true => ((bytes_saved as f64 / original_bytes as f64) * 1000.0).round() / 10.0,
        false => 0.0,
    };

    Ok(MarkdownCompressionOutcome {
        compressed,
        original_bytes,
        compressed_bytes,
        bytes_saved,
        tokens_saved,
        savings_pct,
        headings_preserved: after_digest.headings.len(),
        code_blocks_preserved: after_digest.code_blocks.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopword_tables_are_strictly_sorted() {
        assert!(DOC_COMPRESS_LITE_FILLERS.windows(2).all(|w| w[0] < w[1]));
        assert!(
            DOC_COMPRESS_TELEGRAPHIC_STOPWORDS
                .windows(2)
                .all(|w| w[0] < w[1])
        );
        assert!(!DOC_COMPRESS_TELEGRAPHIC_STOPWORDS.contains(&"not"));
        assert!(!DOC_COMPRESS_TELEGRAPHIC_STOPWORDS.contains(&"never"));
        assert!(!DOC_COMPRESS_TELEGRAPHIC_STOPWORDS.contains(&"no"));
    }

    #[test]
    fn compresses_prose_while_preserving_code_headings_urls_quotes_and_paths() {
        let doc = "---\nname: Project Rules\n---\n\n# Architecture Notes\n\n\n- Please make sure to run `cargo test --workspace` in order to verify the build prior to committing.\n- It is important to note that the config lives at `./crates/zenko-core/src/config.rs` and docs at https://github.com/aswin402/zenko.\n- Exact error message: \"Please note that the cache must not be altered in order to pass\".\n- Never drop negations when the tests do not pass.\n\n```rust\nfn main() {\n    println!(\"please note that the code stays verbatim\");\n}\n```\n";
        let res = compress_markdown_doc(doc, ConciseMode::Full).expect("compress");
        assert!(res.bytes_saved > 50, "saved={}", res.bytes_saved);
        assert_eq!(res.headings_preserved, 1);
        assert_eq!(res.code_blocks_preserved, 1);
        assert!(res.compressed.contains("# Architecture Notes"));
        assert!(res.compressed.contains("`cargo test --workspace`"));
        assert!(res.compressed.contains("./crates/zenko-core/src/config.rs"));
        assert!(res.compressed.contains("https://github.com/aswin402/zenko"));
        assert!(
            res.compressed
                .contains("\"Please note that the cache must not be altered in order to pass\"")
        );
        assert!(res.compressed.contains("println!(\"please note that the code stays verbatim\")"));
        assert!(res.compressed.contains("Never"));
        assert!(res.compressed.contains("not pass."));
    }
}
