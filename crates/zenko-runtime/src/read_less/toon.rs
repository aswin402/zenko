//! Native Pure-Rust TOON (Token-Oriented Object Notation) Encoder & Round-Trip Verifier
//! (inspired by `toon-format/toon`; zero external crates, zero regex, zero `.unwrap()`).
//!
//! Combines YAML-like indentation, safe single-key folding (`data.meta.page: 1`), inline
//! primitive arrays (`tags[3]: rust,mcp,sqlite`), tabular uniform arrays with adaptive
//! delimiters (`[N]{a,b}:` or `[N|]{a|b}:`), 1-level nested uniform field groups
//! (`items[N]{id,author{login,id},state}:`), keyed tabular maps (`services[N:]{status,ms}:`),
//! and strict minimal quoting preserving exact JSON types (including leading-zero strings `"05"`).

use serde_json::{Map, Value};
use std::collections::BTreeSet;
use zenko_core::{
    SMART_CRUSHER_TABULAR_MAX_CELL_CHARS, SMART_CRUSHER_TABULAR_MAX_COLS,
    SMART_CRUSHER_TABULAR_MAX_ITEMS, SMART_CRUSHER_TOON_INIT_CAP_BYTES,
    SMART_CRUSHER_TOON_INLINE_MAX_CHARS, SMART_CRUSHER_TOON_INLINE_MAX_ITEMS,
    SMART_CRUSHER_TOON_KEYED_MAP_MIN_ENTRIES, SMART_CRUSHER_TOON_MAX_DEPTH,
    SMART_CRUSHER_TOON_TABULAR_MIN_ITEMS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToonDelimiter {
    Comma,
    Pipe,
}

impl ToonDelimiter {
    fn as_char(self) -> char {
        match self {
            Self::Comma => ',',
            Self::Pipe => '|',
        }
    }

    fn header_suffix(self) -> &'static str {
        match self {
            Self::Comma => "",
            Self::Pipe => "|",
        }
    }
}

#[derive(Debug, Clone)]
enum TabularCol<'a> {
    Scalar(&'a str),
    NestedGroup(&'a str, Vec<&'a str>),
}

impl<'a> TabularCol<'a> {
    fn leaf_count(&self) -> usize {
        match self {
            Self::Scalar(_) => 1,
            Self::NestedGroup(_, sub) => sub.len(),
        }
    }

    fn render_header(&self, delim: ToonDelimiter) -> String {
        match self {
            Self::Scalar(k) => format_key(k),
            Self::NestedGroup(k, sub) => {
                let d = delim.as_char().to_string();
                let inner: Vec<String> = sub.iter().map(|sk| format_key(sk)).collect();
                format!("{}{{{}}}", format_key(k), inner.join(&d))
            }
        }
    }
}

struct ToonWriter {
    buf: String,
    max_bytes: usize,
    overflowed: bool,
}

impl ToonWriter {
    fn new(max_bytes: usize) -> Self {
        Self {
            buf: String::with_capacity(max_bytes.min(SMART_CRUSHER_TOON_INIT_CAP_BYTES)),
            max_bytes,
            overflowed: false,
        }
    }

    fn push_str(&mut self, s: &str) {
        if self.overflowed {
            return;
        }
        if self.buf.len().saturating_add(s.len()) > self.max_bytes {
            self.overflowed = true;
            return;
        }
        self.buf.push_str(s);
    }

    fn push_char(&mut self, c: char) {
        if self.overflowed {
            return;
        }
        if self.buf.len().saturating_add(c.len_utf8()) > self.max_bytes {
            self.overflowed = true;
            return;
        }
        self.buf.push(c);
    }

    fn push_indent(&mut self, level: usize) {
        for _ in 0..level {
            self.push_str("  ");
        }
    }

    fn push_newline(&mut self) {
        if !self.buf.is_empty() && !self.buf.ends_with('\n') {
            self.push_char('\n');
        }
    }
}

/// Encodes any `serde_json::Value` into canonical lossless TOON, aborting cleanly (`None`)
/// if output exceeds `max_bytes` or nesting exceeds `SMART_CRUSHER_TOON_MAX_DEPTH`.
pub fn encode_toon_bounded(val: &Value, max_bytes: usize) -> Option<String> {
    let mut w = ToonWriter::new(max_bytes);
    if !write_root(&mut w, val) || w.overflowed {
        return None;
    }
    let out = w.buf.trim_end_matches('\n').to_string();
    match out.len() <= max_bytes {
        true => Some(out),
        false => None,
    }
}

/// Returns the maximum length of any JSON array anywhere in `val` (up to depth guard),
/// so SmartCrusher can route oversized arrays (`> 40` rows) directly to Tier-2 scored keep/drop.
pub fn max_nested_array_len(val: &Value, depth: usize) -> usize {
    if depth > SMART_CRUSHER_TOON_MAX_DEPTH {
        return usize::MAX;
    }
    match val {
        Value::Array(items) => {
            let mut best = items.len();
            for item in items {
                best = best.max(max_nested_array_len(item, depth + 1));
            }
            best
        }
        Value::Object(map) => {
            let mut best = 0usize;
            for v in map.values() {
                best = best.max(max_nested_array_len(v, depth + 1));
            }
            best
        }
        _ => 0,
    }
}

fn is_foldable_ident(s: &str) -> bool {
    let bytes = s.as_bytes();
    let Some(&first) = bytes.first() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return false;
    }
    bytes
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn is_safe_unquoted_key(s: &str) -> bool {
    is_foldable_ident(s)
}

fn format_key(k: &str) -> String {
    match is_safe_unquoted_key(k) {
        true => k.to_string(),
        false => quote_string(k),
    }
}

fn quote_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn looks_like_numeric_literal(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() {
        return false;
    }
    let start = match b[0] == b'-' {
        true => 1,
        false => 0,
    };
    if start >= b.len() || !b[start].is_ascii_digit() {
        return false;
    }
    let mut i = start;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        if i >= b.len() || !b[i].is_ascii_digit() {
            return false;
        }
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < b.len() && matches!(b[i], b'e' | b'E') {
        i += 1;
        if i < b.len() && matches!(b[i], b'+' | b'-') {
            i += 1;
        }
        if i >= b.len() || !b[i].is_ascii_digit() {
            return false;
        }
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    i == b.len()
}

fn needs_quotes(s: &str, delim: Option<ToonDelimiter>) -> bool {
    if s.is_empty()
        || s.trim() != s
        || matches!(s, "true" | "false" | "null")
        || looks_like_numeric_literal(s)
        || s.starts_with('-')
        || s.starts_with('"')
    {
        return true;
    }
    let active_d = delim.map(ToonDelimiter::as_char);
    for ch in s.chars() {
        if active_d == Some(ch)
            || matches!(
                ch,
                ':' | '{' | '}' | '[' | ']' | '\\' | '"' | '\n' | '\r' | '\t'
            )
            || (ch as u32) < 0x20
        {
            return true;
        }
    }
    false
}

fn format_primitive(val: &Value, delim: Option<ToonDelimiter>) -> Option<String> {
    match val {
        Value::Null => Some("null".to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(match needs_quotes(s, delim) {
            true => quote_string(s),
            false => s.clone(),
        }),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn is_scalar(v: &Value) -> bool {
    matches!(v, Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_))
}

fn scalar_within_cell_cap(v: &Value) -> bool {
    match v {
        Value::String(s) => s.len() <= SMART_CRUSHER_TABULAR_MAX_CELL_CHARS,
        other => is_scalar(other),
    }
}

fn pick_delimiter_for_strings<'a>(iter: impl Iterator<Item = &'a str>) -> ToonDelimiter {
    let mut commas = 0usize;
    let mut pipes = 0usize;
    for s in iter {
        if s.contains(',') {
            commas += 1;
        }
        if s.contains('|') {
            pipes += 1;
        }
    }
    match commas > 0 && pipes < commas {
        true => ToonDelimiter::Pipe,
        false => ToonDelimiter::Comma,
    }
}

/// Detects whether `items` is a uniform tabular object array (with optional 1-level uniform
/// nested scalar sub-objects like `author{login,id}`) eligible for TOON Tabular Form.
fn detect_tabular_schema<'a>(items: &'a [Value]) -> Option<(Vec<TabularCol<'a>>, ToonDelimiter)> {
    if items.len() < SMART_CRUSHER_TOON_TABULAR_MIN_ITEMS
        || items.len() > SMART_CRUSHER_TABULAR_MAX_ITEMS
    {
        return None;
    }
    let first_obj = items[0].as_object()?;
    if first_obj.is_empty() {
        return None;
    }

    let expected_top: Vec<&'a str> = first_obj.keys().map(String::as_str).collect();
    let expected_top_set: BTreeSet<&'a str> = expected_top.iter().copied().collect();

    // First verify exact identical top-level keys across every item (guarantees 100% lossless round-trip).
    for item in &items[1..] {
        let obj = item.as_object()?;
        if obj.len() != expected_top.len() {
            return None;
        }
        for k in obj.keys() {
            if !expected_top_set.contains(k.as_str()) {
                return None;
            }
        }
    }

    let mut cols: Vec<TabularCol<'a>> = Vec::with_capacity(expected_top.len());
    let mut leaf_total = 0usize;
    let mut string_samples: Vec<&'a str> = Vec::new();

    for &top_key in &expected_top {
        match first_obj.get(top_key)? {
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                for item in items {
                    let v = item.as_object()?.get(top_key)?;
                    if !scalar_within_cell_cap(v) {
                        return None;
                    }
                    if let Value::String(s) = v {
                        string_samples.push(s.as_str());
                    }
                }
                leaf_total += 1;
                if leaf_total > SMART_CRUSHER_TABULAR_MAX_COLS {
                    return None;
                }
                cols.push(TabularCol::Scalar(top_key));
            }
            Value::Object(sub_first) => {
                // 1-level uniform nested field group: `author{login,id}`
                if sub_first.is_empty() || !is_safe_unquoted_key(top_key) {
                    return None;
                }
                let sub_keys: Vec<&'a str> = sub_first.keys().map(String::as_str).collect();
                let sub_key_set: BTreeSet<&'a str> = sub_keys.iter().copied().collect();
                for item in items {
                    let Value::Object(sub_obj) = item.as_object()?.get(top_key)? else {
                        return None;
                    };
                    if sub_obj.len() != sub_keys.len() {
                        return None;
                    }
                    for (sk, sv) in sub_obj {
                        if !sub_key_set.contains(sk.as_str()) || !scalar_within_cell_cap(sv) {
                            return None;
                        }
                        if let Value::String(s) = sv {
                            string_samples.push(s.as_str());
                        }
                    }
                }
                leaf_total += sub_keys.len();
                if leaf_total > SMART_CRUSHER_TABULAR_MAX_COLS {
                    return None;
                }
                cols.push(TabularCol::NestedGroup(top_key, sub_keys));
            }
            Value::Array(_) => return None,
        }
    }

    if cols.is_empty() || leaf_total == 0 {
        return None;
    }
    let delim = pick_delimiter_for_strings(string_samples.into_iter());
    Some((cols, delim))
}

/// Detects whether a JSON object is a uniform map of scalar objects eligible for Keyed Tabular Form:
/// `services[4:]{status,latency_ms}:`
fn detect_keyed_tabular<'a>(
    map: &'a Map<String, Value>,
) -> Option<(Vec<TabularCol<'a>>, ToonDelimiter)> {
    if map.len() < SMART_CRUSHER_TOON_KEYED_MAP_MIN_ENTRIES
        || map.len() > SMART_CRUSHER_TABULAR_MAX_ITEMS
    {
        return None;
    }
    let mut vals: Vec<&'a Value> = Vec::with_capacity(map.len());
    for v in map.values() {
        vals.push(v);
    }
    let first_obj = vals[0].as_object()?;
    if first_obj.is_empty() {
        return None;
    }
    let keys: Vec<&'a str> = first_obj.keys().map(String::as_str).collect();
    if keys.len() > SMART_CRUSHER_TABULAR_MAX_COLS {
        return None;
    }
    let key_set: BTreeSet<&'a str> = keys.iter().copied().collect();
    let mut string_samples: Vec<&'a str> = Vec::new();

    for v in &vals {
        let obj = v.as_object()?;
        if obj.len() != keys.len() {
            return None;
        }
        for (k, fv) in obj {
            if !key_set.contains(k.as_str()) || !scalar_within_cell_cap(fv) {
                return None;
            }
            if let Value::String(s) = fv {
                string_samples.push(s.as_str());
            }
        }
    }

    let delim = pick_delimiter_for_strings(string_samples.into_iter());
    let cols = keys.into_iter().map(TabularCol::Scalar).collect();
    Some((cols, delim))
}

fn try_format_inline_primitive_array(items: &[Value]) -> Option<(ToonDelimiter, String)> {
    if items.is_empty() || items.len() > SMART_CRUSHER_TOON_INLINE_MAX_ITEMS {
        return None;
    }
    if !items.iter().all(is_scalar) {
        return None;
    }
    let delim = pick_delimiter_for_strings(
        items
            .iter()
            .filter_map(|v| v.as_str()),
    );
    let sep = delim.as_char().to_string();
    let mut parts: Vec<String> = Vec::with_capacity(items.len());
    let mut total_chars = 0usize;
    for item in items {
        let cell = format_primitive(item, Some(delim))?;
        total_chars = total_chars.saturating_add(cell.len() + 1);
        if total_chars > SMART_CRUSHER_TOON_INLINE_MAX_CHARS {
            return None;
        }
        parts.push(cell);
    }
    Some((delim, parts.join(&sep)))
}

fn write_tabular_rows(
    w: &mut ToonWriter,
    items: &[Value],
    cols: &[TabularCol<'_>],
    delim: ToonDelimiter,
    indent: usize,
) -> bool {
    let sep = delim.as_char().to_string();
    let total_leaves: usize = cols.iter().map(TabularCol::leaf_count).sum();
    for item in items {
        let Some(obj) = item.as_object() else {
            return false;
        };
        let mut cells: Vec<String> = Vec::with_capacity(total_leaves);
        for col in cols {
            match col {
                TabularCol::Scalar(k) => {
                    let Some(v) = obj.get(*k).and_then(|v| format_primitive(v, Some(delim))) else {
                        return false;
                    };
                    cells.push(v);
                }
                TabularCol::NestedGroup(k, sub_keys) => {
                    let Some(sub_obj) = obj.get(*k).and_then(Value::as_object) else {
                        return false;
                    };
                    for sk in sub_keys {
                        let Some(v) = sub_obj.get(*sk).and_then(|v| format_primitive(v, Some(delim)))
                        else {
                            return false;
                        };
                        cells.push(v);
                    }
                }
            }
        }
        w.push_newline();
        w.push_indent(indent);
        w.push_str(&cells.join(&sep));
        if w.overflowed {
            return false;
        }
    }
    true
}

fn write_keyed_tabular_rows(
    w: &mut ToonWriter,
    map: &Map<String, Value>,
    cols: &[TabularCol<'_>],
    delim: ToonDelimiter,
    indent: usize,
) -> bool {
    let sep = delim.as_char().to_string();
    for (row_key, val) in map {
        let Some(obj) = val.as_object() else {
            return false;
        };
        let mut cells: Vec<String> = Vec::with_capacity(cols.len());
        for col in cols {
            let TabularCol::Scalar(k) = col else {
                return false;
            };
            let Some(cell) = obj.get(*k).and_then(|v| format_primitive(v, Some(delim))) else {
                return false;
            };
            cells.push(cell);
        }
        w.push_newline();
        w.push_indent(indent);
        w.push_str(&format_key(row_key));
        w.push_str(": ");
        w.push_str(&cells.join(&sep));
        if w.overflowed {
            return false;
        }
    }
    true
}

fn format_cols_header(cols: &[TabularCol<'_>], delim: ToonDelimiter) -> String {
    let sep = delim.as_char().to_string();
    let rendered: Vec<String> = cols.iter().map(|c| c.render_header(delim)).collect();
    rendered.join(&sep)
}

fn write_root(w: &mut ToonWriter, val: &Value) -> bool {
    match val {
        Value::Object(map) => {
            if map.is_empty() {
                w.push_str("{}");
                return true;
            }
            if let Some((cols, delim)) = detect_keyed_tabular(map) {
                w.push_str(&format!(
                    "[{}{}:]{{{}}}:",
                    map.len(),
                    delim.header_suffix(),
                    format_cols_header(&cols, delim)
                ));
                return write_keyed_tabular_rows(w, map, &cols, delim, 1);
            }
            write_object_entries(w, map, 0, 0)
        }
        Value::Array(items) => write_array_block(w, None, items, 0, 0),
        other => {
            if let Some(s) = format_primitive(other, None) {
                w.push_str(&s);
                true
            } else {
                false
            }
        }
    }
}

fn fold_single_key_chain<'a>(
    first_key: &'a str,
    first_val: &'a Value,
    depth: usize,
) -> (String, &'a Value, usize) {
    if !is_foldable_ident(first_key) {
        return (format_key(first_key), first_val, depth);
    }
    let mut path = first_key.to_string();
    let mut cur = first_val;
    let mut cur_depth = depth;
    while cur_depth < SMART_CRUSHER_TOON_MAX_DEPTH {
        let Value::Object(m) = cur else {
            break;
        };
        if m.len() != 1 {
            break;
        }
        let Some((next_k, next_v)) = m.iter().next() else {
            break;
        };
        if !is_foldable_ident(next_k) {
            break;
        }
        // Keep keyed tabular maps intact under their immediate parent key
        if let Value::Object(child_map) = next_v
            && detect_keyed_tabular(child_map).is_some()
        {
            path.push('.');
            path.push_str(next_k);
            return (path, next_v, cur_depth + 1);
        }
        path.push('.');
        path.push_str(next_k);
        cur = next_v;
        cur_depth += 1;
    }
    (path, cur, cur_depth)
}

fn write_object_entries(
    w: &mut ToonWriter,
    map: &Map<String, Value>,
    indent: usize,
    depth: usize,
) -> bool {
    if depth > SMART_CRUSHER_TOON_MAX_DEPTH {
        return false;
    }
    for (k, v) in map {
        if w.overflowed {
            return false;
        }
        let (key_expr, target_val, next_depth) = fold_single_key_chain(k, v, depth);
        if next_depth > SMART_CRUSHER_TOON_MAX_DEPTH {
            return false;
        }
        match target_val {
            Value::Array(items) => {
                w.push_newline();
                w.push_indent(indent);
                if !write_array_block(w, Some(&key_expr), items, indent, next_depth + 1) {
                    return false;
                }
            }
            Value::Object(child_map) => {
                w.push_newline();
                w.push_indent(indent);
                match (child_map.is_empty(), detect_keyed_tabular(child_map)) {
                    (true, _) => {
                        w.push_str(&key_expr);
                        w.push_str(": {}");
                    }
                    (false, Some((cols, delim))) => {
                        w.push_str(&format!(
                            "{key_expr}[{}{}:]{{{}}}:",
                            child_map.len(),
                            delim.header_suffix(),
                            format_cols_header(&cols, delim)
                        ));
                        if !write_keyed_tabular_rows(w, child_map, &cols, delim, indent + 1) {
                            return false;
                        }
                    }
                    (false, None) => {
                        w.push_str(&key_expr);
                        w.push_char(':');
                        if !write_object_entries(w, child_map, indent + 1, next_depth + 1) {
                            return false;
                        }
                    }
                }
            }
            prim => {
                let Some(s) = format_primitive(prim, None) else {
                    return false;
                };
                w.push_newline();
                w.push_indent(indent);
                w.push_str(&key_expr);
                w.push_str(": ");
                w.push_str(&s);
            }
        }
    }
    !w.overflowed
}

fn write_array_block(
    w: &mut ToonWriter,
    key_prefix: Option<&str>,
    items: &[Value],
    indent: usize,
    depth: usize,
) -> bool {
    if depth > SMART_CRUSHER_TOON_MAX_DEPTH {
        return false;
    }
    let prefix = key_prefix.unwrap_or("");
    if items.is_empty() {
        w.push_str(&format!("{prefix}[0]:"));
        return !w.overflowed;
    }

    if let Some((cols, delim)) = detect_tabular_schema(items) {
        w.push_str(&format!(
            "{prefix}[{}{}]{{{}}}:",
            items.len(),
            delim.header_suffix(),
            format_cols_header(&cols, delim)
        ));
        return write_tabular_rows(w, items, &cols, delim, indent + 1);
    }

    if let Some((delim, inline)) = try_format_inline_primitive_array(items) {
        w.push_str(&format!(
            "{prefix}[{}{}]: {inline}",
            items.len(),
            delim.header_suffix()
        ));
        return !w.overflowed;
    }

    // List form fallback (mixed, irregular, or deeply nested items — still 100% lossless)
    w.push_str(&format!("{prefix}[{}]:", items.len()));
    for item in items {
        if w.overflowed {
            return false;
        }
        match item {
            Value::Object(map) => {
                w.push_newline();
                w.push_indent(indent + 1);
                if map.is_empty() {
                    w.push_str("- {}");
                } else {
                    w.push_str("-");
                    if !write_object_entries(w, map, indent + 2, depth + 1) {
                        return false;
                    }
                }
            }
            Value::Array(sub) => {
                w.push_newline();
                w.push_indent(indent + 1);
                w.push_str("- ");
                if !write_array_block(w, None, sub, indent + 1, depth + 1) {
                    return false;
                }
            }
            prim => {
                let Some(s) = format_primitive(prim, None) else {
                    return false;
                };
                w.push_newline();
                w.push_indent(indent + 1);
                w.push_str("- ");
                w.push_str(&s);
            }
        }
    }
    !w.overflowed
}

// ── Round-Trip TOON Decoder (used for lossless verification & testing) ────────

#[cfg(test)]
#[derive(Debug, Clone)]
enum ParsedCol {
    Scalar(String),
    NestedGroup(String, Vec<String>),
}

#[cfg(test)]
struct Line<'a> {
    indent: usize,
    text: &'a str,
}

#[cfg(test)]
pub fn decode_toon(input: &str) -> std::result::Result<Value, String> {
    let lines: Vec<Line<'_>> = input
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let spaces = l.bytes().take_while(|&b| b == b' ').count();
            Line {
                indent: spaces / 2,
                text: &l[spaces..],
            }
        })
        .collect();
    if lines.is_empty() {
        return Ok(Value::Null);
    }
    let mut pos = 0usize;
    let first = lines[0].text;
    if first == "{}" {
        return Ok(Value::Object(Map::new()));
    }
    if first.starts_with('[') {
        let (hdr, rest) = split_header_and_rest(first)?;
        pos += 1;
        return parse_collection_from_header(&hdr, rest, &lines, &mut pos, 0);
    }
    if find_unquoted_colon(first).is_none() {
        return parse_scalar_token(first);
    }
    parse_object_block(&lines, &mut pos, 0)
}

#[cfg(test)]
struct CollectionHeader {
    len: usize,
    delim: ToonDelimiter,
    keyed: bool,
    cols: Option<Vec<ParsedCol>>,
}

#[cfg(test)]
fn find_unquoted_colon(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    let mut in_quotes = false;
    let mut brace_depth = 0usize;
    let mut bracket_depth = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if in_quotes {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_quotes = false;
            }
        } else {
            match b {
                b'"' => in_quotes = true,
                b'{' => brace_depth += 1,
                b'}' => brace_depth = brace_depth.saturating_sub(1),
                b'[' => bracket_depth += 1,
                b']' => bracket_depth = bracket_depth.saturating_sub(1),
                b':' if brace_depth == 0 && bracket_depth == 0 => return Some(i),
                _ => {}
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
fn find_unquoted_bracket(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    let mut in_quotes = false;
    while i < bytes.len() {
        match (in_quotes, bytes[i]) {
            (true, b'\\') => {
                i += 2;
                continue;
            }
            (true, b'"') => in_quotes = false,
            (false, b'"') => in_quotes = true,
            (false, b'[') => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg(test)]
fn split_header_and_rest(s: &str) -> std::result::Result<(CollectionHeader, &str), String> {
    let colon = find_unquoted_colon(s).ok_or_else(|| format!("missing ':' in header `{s}`"))?;
    let spec = &s[..colon];
    let rest = s[colon + 1..].trim_start();
    let Some(close_br) = spec.find(']') else {
        return Err(format!("missing ']' in `{spec}`"));
    };
    let mut inner = &spec[1..close_br];
    let keyed = inner.ends_with(':');
    if keyed {
        inner = &inner[..inner.len() - 1];
    }
    let delim = match inner.ends_with('|') {
        true => {
            inner = &inner[..inner.len() - 1];
            ToonDelimiter::Pipe
        }
        false => ToonDelimiter::Comma,
    };
    let len = inner
        .parse::<usize>()
        .map_err(|_| format!("bad array length `{inner}`"))?;
    let after = &spec[close_br + 1..];
    let cols = match after.is_empty() {
        true => None,
        false => {
            let stripped = after
                .strip_prefix('{')
                .and_then(|x| x.strip_suffix('}'))
                .ok_or_else(|| format!("malformed cols `{after}`"))?;
            Some(parse_col_list(stripped, delim)?)
        }
    };
    Ok((
        CollectionHeader {
            len,
            delim,
            keyed,
            cols,
        },
        rest,
    ))
}

#[cfg(test)]
fn split_delimited_respecting_quotes_and_braces(s: &str, delim: char) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let delim_byte = delim as u8;
    let mut start = 0usize;
    let mut i = 0usize;
    let mut in_quotes = false;
    let mut braces = 0usize;
    while i < bytes.len() {
        match (in_quotes, bytes[i]) {
            (true, b'\\') => {
                i += 2;
                continue;
            }
            (true, b'"') => in_quotes = false,
            (false, b'"') => in_quotes = true,
            (false, b'{') => braces += 1,
            (false, b'}') => braces = braces.saturating_sub(1),
            (false, b) if braces == 0 && b == delim_byte => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

#[cfg(test)]
fn parse_col_list(s: &str, delim: ToonDelimiter) -> std::result::Result<Vec<ParsedCol>, String> {
    let mut out = Vec::new();
    for raw in split_delimited_respecting_quotes_and_braces(s, delim.as_char()) {
        if let Some(open) = raw.find('{')
            && !raw.starts_with('"')
        {
            let name = parse_key_token(&raw[..open])?;
            let inner = raw[open + 1..]
                .strip_suffix('}')
                .ok_or_else(|| format!("unclosed nested col `{raw}`"))?;
            let mut sub = Vec::new();
            for sk in split_delimited_respecting_quotes_and_braces(inner, delim.as_char()) {
                sub.push(parse_key_token(sk)?);
            }
            out.push(ParsedCol::NestedGroup(name, sub));
        } else {
            out.push(ParsedCol::Scalar(parse_key_token(raw)?));
        }
    }
    Ok(out)
}

#[cfg(test)]
fn parse_key_token(raw: &str) -> std::result::Result<String, String> {
    let t = raw.trim();
    match t.starts_with('"') {
        true => unquote_string(t),
        false => Ok(t.to_string()),
    }
}

#[cfg(test)]
fn unquote_string(s: &str) -> std::result::Result<String, String> {
    let inner = s
        .strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .ok_or_else(|| format!("unclosed string `{s}`"))?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    let code = u32::from_str_radix(&hex, 16)
                        .map_err(|_| format!("bad unicode escape `\\u{hex}`"))?;
                    let c = char::from_u32(code)
                        .ok_or_else(|| format!("invalid char code {code}"))?;
                    out.push(c);
                }
                other => return Err(format!("bad escape `{other:?}`")),
            }
        } else {
            out.push(ch);
        }
    }
    Ok(out)
}

#[cfg(test)]
fn parse_scalar_token(raw: &str) -> std::result::Result<Value, String> {
    let t = raw.trim();
    if t.starts_with('"') {
        return Ok(Value::String(unquote_string(t)?));
    }
    match t {
        "null" => Ok(Value::Null),
        "true" => Ok(Value::Bool(true)),
        "false" => Ok(Value::Bool(false)),
        "{}" => Ok(Value::Object(Map::new())),
        _ if looks_like_numeric_literal(t) => serde_json::from_str::<Value>(t)
            .map_err(|e| format!("bad number `{t}`: {e}")),
        _ => Ok(Value::String(t.to_string())),
    }
}

#[cfg(test)]
fn insert_folded(map: &mut Map<String, Value>, key_expr: &str, val: Value) -> std::result::Result<(), String> {
    if key_expr.starts_with('"') || !key_expr.contains('.') {
        map.insert(parse_key_token(key_expr)?, val);
        return Ok(());
    }
    let parts: Vec<&str> = key_expr.split('.').collect();
    let mut cur = map;
    for seg in &parts[..parts.len() - 1] {
        let entry = cur
            .entry((*seg).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        let Value::Object(next) = entry else {
            return Err(format!("key fold collision at `{seg}`"));
        };
        cur = next;
    }
    if let Some(last) = parts.last() {
        cur.insert((*last).to_string(), val);
    }
    Ok(())
}

#[cfg(test)]
fn parse_object_block(
    lines: &[Line<'_>],
    pos: &mut usize,
    indent: usize,
) -> std::result::Result<Value, String> {
    let mut map = Map::new();
    while *pos < lines.len() && lines[*pos].indent == indent {
        let txt = lines[*pos].text;
        if txt.starts_with('-') {
            break;
        }
        let colon = find_unquoted_colon(txt).ok_or_else(|| format!("expected ':' in `{txt}`"))?;
        let lhs = &txt[..colon];
        let rhs = txt[colon + 1..].trim_start();

        match (find_unquoted_bracket(lhs), rhs.is_empty()) {
            (Some(br_idx), _) => {
                let key_expr = &lhs[..br_idx];
                let (hdr, rest) = split_header_and_rest(&txt[br_idx..])?;
                *pos += 1;
                let val = parse_collection_from_header(&hdr, rest, lines, pos, indent)?;
                insert_folded(&mut map, key_expr, val)?;
            }
            (None, true) => {
                *pos += 1;
                let child = parse_object_block(lines, pos, indent + 1)?;
                insert_folded(&mut map, lhs, child)?;
            }
            (None, false) => {
                *pos += 1;
                let val = parse_scalar_token(rhs)?;
                insert_folded(&mut map, lhs, val)?;
            }
        }
    }
    Ok(Value::Object(map))
}

#[cfg(test)]
fn parse_collection_from_header(
    hdr: &CollectionHeader,
    rest: &str,
    lines: &[Line<'_>],
    pos: &mut usize,
    indent: usize,
) -> std::result::Result<Value, String> {
    if hdr.len == 0 {
        return Ok(match hdr.keyed {
            true => Value::Object(Map::new()),
            false => Value::Array(Vec::new()),
        });
    }

    if hdr.keyed {
        let cols = hdr.cols.as_ref().ok_or("keyed tabular missing cols")?;
        let mut map = Map::new();
        for _ in 0..hdr.len {
            if *pos >= lines.len() || lines[*pos].indent != indent + 1 {
                return Err("truncated keyed tabular rows".into());
            }
            let row_txt = lines[*pos].text;
            *pos += 1;
            let colon = find_unquoted_colon(row_txt).ok_or("keyed row missing ':'")?;
            let row_key = parse_key_token(&row_txt[..colon])?;
            let vals_str = row_txt[colon + 1..].trim_start();
            let row_obj = parse_tabular_row(vals_str, cols, hdr.delim)?;
            map.insert(row_key, row_obj);
        }
        return Ok(Value::Object(map));
    }

    if let Some(cols) = &hdr.cols {
        let mut items = Vec::with_capacity(hdr.len);
        for _ in 0..hdr.len {
            if *pos >= lines.len() || lines[*pos].indent != indent + 1 {
                return Err("truncated tabular rows".into());
            }
            let row_txt = lines[*pos].text;
            *pos += 1;
            items.push(parse_tabular_row(row_txt, cols, hdr.delim)?);
        }
        return Ok(Value::Array(items));
    }

    if !rest.is_empty() {
        let parts = split_delimited_respecting_quotes_and_braces(rest, hdr.delim.as_char());
        if parts.len() != hdr.len {
            return Err(format!("inline array len mismatch: {} != {}", parts.len(), hdr.len));
        }
        let mut items = Vec::with_capacity(hdr.len);
        for p in parts {
            items.push(parse_scalar_token(p)?);
        }
        return Ok(Value::Array(items));
    }

    // List form
    let mut items = Vec::with_capacity(hdr.len);
    for _ in 0..hdr.len {
        if *pos >= lines.len() || lines[*pos].indent != indent + 1 {
            return Err("truncated list items".into());
        }
        let item_txt = lines[*pos].text;
        *pos += 1;
        match item_txt {
            "-" => items.push(parse_object_block(lines, pos, indent + 2)?),
            _ => match item_txt.strip_prefix("- ") {
                Some(after_dash) if after_dash.starts_with('[') => {
                    let (sub_hdr, sub_rest) = split_header_and_rest(after_dash)?;
                    items.push(parse_collection_from_header(
                        &sub_hdr,
                        sub_rest,
                        lines,
                        pos,
                        indent + 1,
                    )?);
                }
                Some(after_dash) => items.push(parse_scalar_token(after_dash)?),
                None => return Err(format!("expected list item `-`, got `{item_txt}`")),
            },
        }
    }
    Ok(Value::Array(items))
}

#[cfg(test)]
fn parse_tabular_row(
    row_txt: &str,
    cols: &[ParsedCol],
    delim: ToonDelimiter,
) -> std::result::Result<Value, String> {
    let raw_cells = split_delimited_respecting_quotes_and_braces(row_txt, delim.as_char());
    let mut idx = 0usize;
    let mut obj = Map::new();
    for col in cols {
        match col {
            ParsedCol::Scalar(name) => {
                let cell = raw_cells
                    .get(idx)
                    .ok_or_else(|| format!("missing cell {idx} in `{row_txt}`"))?;
                idx += 1;
                obj.insert(name.clone(), parse_scalar_token(cell)?);
            }
            ParsedCol::NestedGroup(name, sub_keys) => {
                let mut sub = Map::new();
                for sk in sub_keys {
                    let cell = raw_cells
                        .get(idx)
                        .ok_or_else(|| format!("missing nested cell {idx} in `{row_txt}`"))?;
                    idx += 1;
                    sub.insert(sk.clone(), parse_scalar_token(cell)?);
                }
                obj.insert(name.clone(), Value::Object(sub));
            }
        }
    }
    Ok(Value::Object(obj))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn toon_roundtrips_wrapped_nested_groups_keyed_maps_and_edge_cases_losslessly() {
        let payload = json!({
            "ok": true,
            "meta": {
                "pagination": {
                    "page": 1,
                    "total": 4
                }
            },
            "tags": ["rust", "mcp", "05", "true", "a,b"],
            "issues": [
                { "id": 101, "author": { "login": "alice", "role": "maintainer" }, "title": "Fix parser, edge case", "open": true },
                { "id": 102, "author": { "login": "bob", "role": "contributor" }, "title": "Add TOON encoder", "open": false },
                { "id": 103, "author": { "login": "carol", "role": "reviewer" }, "title": "Leading \"quote\" & line\nbreak", "open": true }
            ],
            "services": {
                "api": { "status": "ok", "ms": 12 },
                "auth": { "status": "ok", "ms": 8 },
                "db": { "status": "ok", "ms": 4 },
                "worker": { "status": "degraded", "ms": 140 }
            },
            "dotted.key": "keep-literal",
            "empty_obj": {},
            "empty_arr": [],
            "mixed": [1, "007", null, { "k": "v" }]
        });

        let encoded = encode_toon_bounded(&payload, 8192).expect("encode toon");
        assert!(encoded.contains("meta.pagination:"), "folded key missing:\n{encoded}");
        assert!(encoded.contains("issues[3|]{author{login|role}|id|open|title}:")
            || encoded.contains("author{login|role}"),
            "expected pipe-delimited nested group header:\n{encoded}");
        assert!(encoded.contains("services[4:]{ms,status}:"), "expected keyed tabular:\n{encoded}");
        assert!(encoded.contains("\"dotted.key\": keep-literal"), "dotted key must stay quoted:\n{encoded}");
        assert!(encoded.contains("\"05\""), "leading zero string must stay quoted:\n{encoded}");

        let decoded = decode_toon(&encoded).expect("decode toon");
        assert_eq!(decoded, payload);
    }

    #[test]
    fn toon_auto_selects_pipe_delimiter_when_cells_contain_commas() {
        let rows = json!([
            { "id": 1, "owner": "Smith, Alice", "region": "us-east-1" },
            { "id": 2, "owner": "Doe, Bob", "region": "eu-west-1" },
            { "id": 3, "owner": "Lee, Carol", "region": "ap-south-1" }
        ]);
        let encoded = encode_toon_bounded(&rows, 4096).expect("encode");
        assert!(encoded.starts_with("[3|]{"), "expected pipe header:\n{encoded}");
        assert!(encoded.contains("Smith, Alice"), "comma value should stay unquoted under pipe delim:\n{encoded}");
        assert_eq!(decode_toon(&encoded).expect("decode"), rows);
    }
}
