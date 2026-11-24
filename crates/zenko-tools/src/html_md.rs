//! Pure-Rust, zero-regex finite-state HTML-to-Markdown converter and HTML entity decoder.
//! Preserves headings, fenced/inline code, links, lists, blockquotes, and table rows for
//! high-signal LLM comprehension without pulling heavy DOM parser dependencies.

#[derive(Clone, Copy, PartialEq, Eq)]
enum SkipElement {
    Script,
    Style,
    NoScript,
    Svg,
    Template,
}

impl SkipElement {
    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "script" => Some(Self::Script),
            "style" => Some(Self::Style),
            "noscript" => Some(Self::NoScript),
            "svg" => Some(Self::Svg),
            "template" => Some(Self::Template),
            _ => None,
        }
    }

    fn close_marker(self) -> &'static [u8] {
        match self {
            Self::Script => b"</script>",
            Self::Style => b"</style>",
            Self::NoScript => b"</noscript>",
            Self::Svg => b"</svg>",
            Self::Template => b"</template>",
        }
    }
}

/// Declarative HTML entity lookup table (zero regex, zero `if/else` ladders).
const NAMED_ENTITIES: &[(&str, &str)] = &[
    ("amp", "&"),
    ("lt", "<"),
    ("gt", ">"),
    ("quot", "\""),
    ("apos", "'"),
    ("nbsp", " "),
    ("mdash", "—"),
    ("ndash", "–"),
    ("hellip", "..."),
    ("copy", "©"),
    ("reg", "®"),
    ("trade", "™"),
    ("lsquo", "‘"),
    ("rsquo", "’"),
    ("ldquo", "“"),
    ("rdquo", "”"),
    ("bull", "•"),
    ("rarr", "→"),
    ("larr", "←"),
];

const HTML_SNIFF_MARKERS: &[&str] = &[
    "<!doctype html",
    "<html",
    "<head",
    "<body",
    "<div",
    "<p>",
    "<p ",
    "<h1",
    "<h2",
    "<article",
    "<main",
    "<section",
    "<ul",
    "<ol",
    "<table",
    "<pre",
];

/// Determines whether a fetched response body should be parsed as HTML based on `Content-Type`
/// or structural HTML tag sniffing (never mangles plain text, Markdown, JSON, or source code).
pub fn should_parse_as_html(content_type: Option<&str>, body: &str) -> bool {
    if let Some(ct) = content_type {
        let mime = ct
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if mime == "text/html" || mime == "application/xhtml+xml" {
            return true;
        }
        if mime.starts_with("application/json")
            || mime.starts_with("text/plain")
            || mime.starts_with("text/markdown")
            || mime.starts_with("application/xml")
            || mime.starts_with("text/xml")
            || mime.starts_with("text/csv")
        {
            return false;
        }
    }
    let prefix_len = body.floor_char_boundary(body.len().min(2048));
    let sample = body[..prefix_len].to_ascii_lowercase();
    HTML_SNIFF_MARKERS.iter().any(|m| sample.contains(m))
}

/// Decodes standard named, decimal (`&#39;`), and hexadecimal (`&#x27;`) HTML entities deterministically.
pub fn decode_html_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'&'
            && let Some(rel_semi) = bytes[i + 1..bytes.len().min(i + 14)]
                .iter()
                .position(|&b| b == b';')
        {
            let entity = &input[i + 1..i + 1 + rel_semi];
            if let Some(decoded) = resolve_entity(entity) {
                out.push_str(&decoded);
                i += rel_semi + 2;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn resolve_entity(entity: &str) -> Option<String> {
    if let Some(num) = entity.strip_prefix('#') {
        let code_point = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => num.parse::<u32>().ok(),
        }?;
        return char::from_u32(code_point).map(|c| c.to_string());
    }
    let lower = entity.to_ascii_lowercase();
    NAMED_ENTITIES
        .iter()
        .find(|(k, _)| *k == lower)
        .map(|(_, v)| (*v).to_string())
}

struct ParsedTag {
    name: String,
    is_closing: bool,
    is_self_closing: bool,
    href: Option<String>,
}

fn parse_tag_inner(inner: &str) -> Option<ParsedTag> {
    let trimmed = inner.trim();
    if trimmed.is_empty() || trimmed.starts_with('!') || trimmed.starts_with('?') {
        return None;
    }
    let (is_closing, after_slash) = match trimmed.strip_prefix('/') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, trimmed),
    };
    let is_self_closing = !is_closing && after_slash.ends_with('/');
    let body = after_slash.trim_end_matches('/').trim();

    let name_end = body
        .find(|c: char| c.is_ascii_whitespace() || c == '/')
        .unwrap_or(body.len());
    let name = body[..name_end].to_ascii_lowercase();
    if name.is_empty() {
        return None;
    }
    let attrs = &body[name_end..];
    let href = match !is_closing && name == "a" {
        true => extract_attr(attrs, "href"),
        false => None,
    };
    Some(ParsedTag {
        name,
        is_closing,
        is_self_closing,
        href,
    })
}

fn extract_attr(attrs: &str, target: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let needle = target.as_bytes();
    let mut i = 0usize;

    while i + needle.len() < bytes.len() {
        let boundary_ok = i == 0 || bytes[i - 1].is_ascii_whitespace();
        if boundary_ok && &bytes[i..i + needle.len()] == needle {
            let mut j = i + needle.len();
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j >= attrs.len() {
                    return None;
                }
                let val_slice = &attrs[j..];
                let raw_val = match val_slice.as_bytes().first().copied() {
                    Some(q @ (b'"' | b'\'')) => {
                        let rest = &val_slice[1..];
                        let end = rest.find(q as char).unwrap_or(rest.len());
                        &rest[..end]
                    }
                    Some(_) => {
                        let end = val_slice
                            .find(|c: char| c.is_ascii_whitespace() || c == '>')
                            .unwrap_or(val_slice.len());
                        &val_slice[..end]
                    }
                    None => "",
                };
                let decoded = decode_html_entities(raw_val.trim());
                return match decoded.is_empty() {
                    true => None,
                    false => Some(decoded),
                };
            }
        }
        i += 1;
    }
    None
}

fn resolve_href(base_origin: Option<&str>, base_dir: Option<&str>, href: &str) -> Option<String> {
    let trimmed = href.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("javascript:") || lower.starts_with("data:") || lower.starts_with("mailto:")
    {
        return None;
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(trimmed.to_string());
    }
    let origin = base_origin?;
    match (trimmed.starts_with("//"), trimmed.starts_with('/')) {
        (true, _) => {
            let scheme = origin.split("://").next().unwrap_or("https");
            Some(format!("{scheme}:{trimmed}"))
        }
        (false, true) => Some(format!("{origin}{trimmed}")),
        (false, false) => {
            let dir = base_dir.unwrap_or("/");
            Some(format!("{origin}{dir}{trimmed}"))
        }
    }
}

fn ensure_newlines(out: &mut String, count: usize) {
    if out.is_empty() {
        return;
    }
    let trailing = out.bytes().rev().take_while(|&b| b == b'\n').count();
    for _ in trailing..count {
        out.push('\n');
    }
}

fn push_text_fragment(dest: &mut String, text: &str, preserve_ws: bool) {
    if text.is_empty() {
        return;
    }
    let decoded = decode_html_entities(text);
    if preserve_ws {
        dest.push_str(&decoded);
        return;
    }
    let mut prev_ws = dest.chars().last().is_some_and(char::is_whitespace);
    for ch in decoded.chars() {
        match ch.is_whitespace() {
            true => {
                if !prev_ws && !dest.is_empty() {
                    dest.push(' ');
                    prev_ws = true;
                }
            }
            false => {
                dest.push(ch);
                prev_ws = false;
            }
        }
    }
}

/// Converts HTML into clean, structured Markdown preserving headings, fenced/inline code,
/// links, lists, blockquotes, and table rows (zero regex).
pub fn html_to_markdown(
    html: &str,
    base_origin: Option<&str>,
    base_dir: Option<&str>,
) -> String {
    let mut out = String::with_capacity(html.len().min(32_768));
    let lower = html.to_ascii_lowercase();
    let lower_bytes = lower.as_bytes();
    let raw_bytes = html.as_bytes();
    let mut i = 0usize;
    let mut skip: Option<SkipElement> = None;
    let mut in_pre = false;
    let mut in_inline_code = false;
    let mut list_depth = 0usize;
    let mut active_link: Option<(Option<String>, String)> = None;
    let mut text_start = 0usize;

    while i < raw_bytes.len() {
        if let Some(el) = skip {
            let marker = el.close_marker();
            if lower_bytes[i..].starts_with(marker) {
                i += marker.len();
                text_start = i;
                skip = None;
            } else {
                i += 1;
            }
            continue;
        }

        if raw_bytes[i] != b'<' {
            i += 1;
            continue;
        }

        if i > text_start {
            let slice = &html[text_start..i];
            match active_link.as_mut() {
                Some((_, buf)) => push_text_fragment(buf, slice, in_pre),
                None => push_text_fragment(&mut out, slice, in_pre),
            }
        }

        if raw_bytes[i..].starts_with(b"<!--") {
            match html[i + 4..].find("-->") {
                Some(rel) => i += 4 + rel + 3,
                None => i = raw_bytes.len(),
            }
            text_start = i;
            continue;
        }

        let mut j = i + 1;
        let mut quote: Option<u8> = None;
        while j < raw_bytes.len() {
            let b = raw_bytes[j];
            match (quote, b) {
                (Some(q), c) if c == q => quote = None,
                (None, c @ (b'"' | b'\'')) => quote = Some(c),
                (None, b'>') => break,
                _ => {}
            }
            j += 1;
        }
        if j >= raw_bytes.len() {
            let slice = &html[i..];
            match active_link.as_mut() {
                Some((_, buf)) => push_text_fragment(buf, slice, in_pre),
                None => push_text_fragment(&mut out, slice, in_pre),
            }
            text_start = raw_bytes.len();
            break;
        }

        let inner = &html[i + 1..j];
        i = j + 1;
        text_start = i;

        let Some(tag) = parse_tag_inner(inner) else {
            continue;
        };

        if !tag.is_closing
            && !tag.is_self_closing
            && let Some(skip_el) = SkipElement::from_tag(&tag.name)
        {
            skip = Some(skip_el);
            continue;
        }

        match (tag.name.as_str(), tag.is_closing) {
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", false) => {
                let level = tag.name.as_bytes()[1].saturating_sub(b'0') as usize;
                ensure_newlines(&mut out, 2);
                for _ in 0..level {
                    out.push('#');
                }
                out.push(' ');
            }
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", true) => {
                ensure_newlines(&mut out, 2);
            }
            ("p" | "div" | "section" | "article" | "header" | "footer" | "main", false) => {
                if !in_pre {
                    ensure_newlines(&mut out, 2);
                }
            }
            ("p" | "section" | "article", true) => {
                if !in_pre {
                    ensure_newlines(&mut out, 2);
                }
            }
            ("br", false) => {
                out.push('\n');
            }
            ("hr", false) => {
                ensure_newlines(&mut out, 2);
                out.push_str("---\n\n");
            }
            ("pre", false) => {
                ensure_newlines(&mut out, 2);
                out.push_str("```\n");
                in_pre = true;
            }
            ("pre", true) => {
                if in_pre {
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("```\n\n");
                    in_pre = false;
                }
            }
            ("code", false) if !in_pre => {
                in_inline_code = true;
                match active_link.as_mut() {
                    Some((_, buf)) => buf.push('`'),
                    None => out.push('`'),
                }
            }
            ("code", true) if !in_pre && in_inline_code => {
                in_inline_code = false;
                match active_link.as_mut() {
                    Some((_, buf)) => buf.push('`'),
                    None => out.push('`'),
                }
            }
            ("ul" | "ol", false) => {
                list_depth = list_depth.saturating_add(1);
                ensure_newlines(&mut out, 1);
            }
            ("ul" | "ol", true) => {
                list_depth = list_depth.saturating_sub(1);
                ensure_newlines(&mut out, 2);
            }
            ("li", false) => {
                ensure_newlines(&mut out, 1);
                let indent = list_depth.saturating_sub(1).min(4);
                for _ in 0..indent {
                    out.push_str("  ");
                }
                out.push_str("- ");
            }
            ("blockquote", false) => {
                ensure_newlines(&mut out, 2);
                out.push_str("> ");
            }
            ("blockquote", true) => {
                ensure_newlines(&mut out, 2);
            }
            ("tr", false) => {
                ensure_newlines(&mut out, 1);
            }
            ("td" | "th", true) => {
                if !out.ends_with(" | ") && !out.ends_with('\n') {
                    out.push_str(" | ");
                }
            }
            ("a", false) if !tag.is_self_closing => {
                let resolved = tag
                    .href
                    .as_deref()
                    .and_then(|h| resolve_href(base_origin, base_dir, h));
                active_link = Some((resolved, String::new()));
            }
            ("a", true) => {
                if let Some((href_opt, label)) = active_link.take() {
                    let clean_label = label.trim();
                    match (clean_label.is_empty(), href_opt) {
                        (false, Some(url)) => {
                            out.push('[');
                            out.push_str(clean_label);
                            out.push_str("](");
                            out.push_str(&url);
                            out.push(')');
                        }
                        (false, None) => out.push_str(clean_label),
                        (true, Some(url)) => out.push_str(&url),
                        (true, None) => {}
                    }
                }
            }
            _ => {}
        }
    }

    if text_start < raw_bytes.len() && skip.is_none() {
        let slice = &html[text_start..];
        match active_link.as_mut() {
            Some((_, buf)) => push_text_fragment(buf, slice, in_pre),
            None => push_text_fragment(&mut out, slice, in_pre),
        }
    }
    if let Some((_, label)) = active_link.take() {
        out.push_str(label.trim());
    }
    if in_pre {
        out.push_str("\n```\n");
    }

    normalize_markdown_whitespace(&out)
}

/// Converts HTML into compact flat plain text (for `format = "text"`).
pub fn html_to_plain_text(html: &str) -> String {
    let md = html_to_markdown(html, None, None);
    md.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "```")
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_markdown_whitespace(raw: &str) -> String {
    let mut lines_out: Vec<String> = Vec::new();
    let mut in_fence = false;
    let mut consecutive_blanks = 0usize;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            consecutive_blanks = 0;
            lines_out.push(trimmed.to_string());
            continue;
        }
        if in_fence {
            lines_out.push(line.to_string());
            continue;
        }
        let cleaned = line.trim_end().trim_end_matches(" |").trim_end();
        if cleaned.trim().is_empty() {
            if !lines_out.is_empty() && consecutive_blanks == 0 {
                lines_out.push(String::new());
                consecutive_blanks = 1;
            }
        } else {
            consecutive_blanks = 0;
            lines_out.push(cleaned.to_string());
        }
    }
    while lines_out.last().is_some_and(String::is_empty) {
        lines_out.pop();
    }
    lines_out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_structured_html_to_clean_markdown_with_links_code_and_entities() {
        let html = r#"
        <!doctype html>
        <html>
          <head><title>Ignored</title><style>body { color: red; }</style></head>
          <body>
            <h1>Zenko &amp; Rust</h1>
            <p>Read the <a href="/docs/guide">Guide &rarr;</a> and call <code>hash_edit()</code>.</p>
            <script>window.evil = 1;</script>
            <ul>
              <li>Fast &amp; safe</li>
              <li>Zero regex</li>
            </ul>
            <pre><code>fn main() {
    println!("hi");
}</code></pre>
          </body>
        </html>
        "#;
        let md = html_to_markdown(html, Some("https://zenko.dev"), Some("/"));
        assert!(md.contains("# Zenko & Rust"), "md={md}");
        assert!(
            md.contains("[Guide →](https://zenko.dev/docs/guide)"),
            "md={md}"
        );
        assert!(md.contains("`hash_edit()`"), "md={md}");
        assert!(md.contains("- Fast & safe"), "md={md}");
        assert!(
            md.contains("```\nfn main() {\n    println!(\"hi\");\n}\n```"),
            "md={md}"
        );
        assert!(!md.contains("window.evil"), "script must be stripped");
        assert!(!md.contains("color: red"), "style must be stripped");
    }

    #[test]
    fn skips_html_parsing_for_json_and_source_code() {
        assert!(!should_parse_as_html(
            Some("application/json; charset=utf-8"),
            "{\"html\":\"<div>ok</div>\"}"
        ));
        assert!(!should_parse_as_html(
            Some("text/plain"),
            "fn foo<T>(v: Vec<String>) -> Result<T> { todo!() }"
        ));
        assert!(should_parse_as_html(
            Some("text/html; charset=utf-8"),
            "plain body"
        ));
        assert!(should_parse_as_html(
            None,
            "<html><body><h1>Hi</h1></body></html>"
        ));
    }

    #[test]
    fn decodes_numeric_and_named_entities() {
        assert_eq!(
            decode_html_entities("A &amp; B &#39;ok&#x21; &lt;tag&gt;"),
            "A & B 'ok! <tag>"
        );
    }
}
