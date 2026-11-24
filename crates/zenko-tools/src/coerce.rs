//! Pure-Rust, zero-regex Schema-Driven Argument Coercion, Malformed JSON Repair,
//! Fuzzy Unknown-Tool Suggestion, and Compact Schema-Hint Error Formatting.
//!
//! Inspired by Hermes Agent's `arg_coercion` + bounded self-correction, implemented
//! deterministically via `match` dispatch and finite-state byte/char machines (zero regex,
//! zero `.unwrap()`, zero hardcoded tool aliases).

use serde_json::{Map, Number, Value};
use std::collections::HashSet;
use zenko_core::{
    TOOL_COERCE_MAX_DEPTH, TOOL_COERCE_MAX_JSON_REPAIR_BYTES, TOOL_SCHEMA_HINT_MAX_PROPS, ToolSpec,
};

/// Repairs provider `_raw` malformed JSON payloads (when present) and recursively coerces
/// argument types, key casing (`camelCase` -> schema `snake_case`), and case-insensitive enums
/// strictly against `schema`.
pub fn coerce_and_repair_args(args: Value, schema: &Value) -> Value {
    let repaired = recover_raw_json_object(args);
    coerce_value_to_schema(repaired, schema, 0)
}

/// Detects `{"_raw": "..."}` SSE fallback wrappers (or bare JSON strings) and runs a
/// deterministic single-pass state-machine JSON repair pass.
fn recover_raw_json_object(val: Value) -> Value {
    match &val {
        Value::Object(map) if map.len() == 1 => match map.get("_raw").and_then(Value::as_str) {
            Some(raw) => repair_json_value(raw)
                .filter(Value::is_object)
                .unwrap_or(val),
            None => val,
        },
        Value::String(raw) => repair_json_value(raw)
            .filter(Value::is_object)
            .unwrap_or(val),
        _ => val,
    }
}

/// Deterministic zero-regex JSON repair state machine:
/// - Strips optional markdown code fences (` ```json ... ``` `)
/// - Removes trailing commas immediately before `}` or `]` outside strings
/// - Closes unclosed quotes and unclosed `{` / `[` delimiters in LIFO stack order
pub fn repair_json_value(raw: &str) -> Option<Value> {
    let trimmed = strip_markdown_fence(raw.trim());
    if trimmed.is_empty() || trimmed.len() > TOOL_COERCE_MAX_JSON_REPAIR_BYTES {
        return None;
    }
    if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
        return Some(parsed);
    }

    let start_idx = trimmed.find(['{', '['])?;
    let candidate = &trimmed[start_idx..];

    let sanitized = balance_and_clean_json(candidate);
    serde_json::from_str::<Value>(&sanitized).ok()
}

fn strip_markdown_fence(s: &str) -> &str {
    let Some(rest) = s.strip_prefix("```") else {
        return s;
    };
    let after_lang = match rest.find('\n') {
        Some(nl) => &rest[nl + 1..],
        None => rest,
    };
    let without_tail = after_lang
        .trim_end()
        .strip_suffix("```")
        .unwrap_or(after_lang);
    without_tail.trim()
}

fn balance_and_clean_json(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(input.len() + 16);
    let mut stack: Vec<char> = Vec::with_capacity(16);
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0usize;

    while i < len {
        let ch = chars[i];
        match (in_string, escaped, ch) {
            (true, true, c) => {
                out.push(c);
                escaped = false;
            }
            (true, false, '\\') => {
                out.push('\\');
                escaped = true;
            }
            (true, false, '"') => {
                out.push('"');
                in_string = false;
            }
            (true, false, '\n') => out.push_str("\\n"),
            (true, false, '\r') => out.push_str("\\r"),
            (true, false, '\t') => out.push_str("\\t"),
            (true, false, c) => out.push(c),
            (false, _, '"') => {
                out.push('"');
                in_string = true;
            }
            (false, _, '{') => {
                out.push('{');
                stack.push('}');
            }
            (false, _, '[') => {
                out.push('[');
                stack.push(']');
            }
            (false, _, '}' | ']') => {
                out.push(ch);
                if stack.last().copied() == Some(ch) {
                    stack.pop();
                }
            }
            (false, _, ',') => {
                // Skip trailing commas right before a closing `}` or `]` or EOF.
                let mut look = i + 1;
                while look < len && chars[look].is_whitespace() {
                    look += 1;
                }
                let is_trailing = look >= len || matches!(chars[look], '}' | ']');
                if !is_trailing {
                    out.push(',');
                }
            }
            (false, _, c) => out.push(c),
        }
        i += 1;
    }

    if in_string {
        if escaped {
            out.pop();
        }
        out.push('"');
    }
    while let Some(closer) = stack.pop() {
        out.push(closer);
    }
    out
}

fn schema_primary_type(schema: &Value) -> Option<&str> {
    match schema.get("type") {
        Some(Value::String(t)) => Some(t.as_str()),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null"),
        _ => match (schema.get("properties"), schema.get("items")) {
            (Some(_), _) => Some("object"),
            (_, Some(_)) => Some("array"),
            _ => None,
        },
    }
}

fn coerce_value_to_schema(val: Value, schema: &Value, depth: usize) -> Value {
    if depth >= TOOL_COERCE_MAX_DEPTH {
        return val;
    }
    if val.is_null() {
        return val;
    }

    match schema_primary_type(schema) {
        Some("object") => coerce_object(val, schema, depth),
        Some("array") => coerce_array(val, schema, depth),
        Some("integer") => coerce_integer(val),
        Some("number") => coerce_number(val),
        Some("boolean") => coerce_boolean(val),
        Some("string") => coerce_string(val, schema),
        _ => val,
    }
}

fn coerce_object(val: Value, schema: &Value, depth: usize) -> Value {
    let map = match val {
        Value::Object(m) => m,
        Value::String(ref raw) => match repair_json_value(raw) {
            Some(Value::Object(m)) => m,
            _ => return val,
        },
        other => return other,
    };

    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return Value::Object(map);
    };

    let mut out = Map::with_capacity(map.len());
    let existing_keys: HashSet<String> = map.keys().cloned().collect();

    for (raw_key, raw_val) in map {
        let canonical_key = resolve_schema_property_key(&raw_key, props, &existing_keys);
        let coerced = match props.get(&canonical_key) {
            Some(prop_schema) => coerce_value_to_schema(raw_val, prop_schema, depth + 1),
            None => raw_val,
        };
        out.insert(canonical_key, coerced);
    }

    Value::Object(out)
}

/// Maps accidental `camelCase` or wrong-case keys (e.g. `startLine` -> `start_line`) strictly to
/// declared schema property names when the canonical key is not already present (zero hardcoded aliases).
fn resolve_schema_property_key(
    raw_key: &str,
    props: &Map<String, Value>,
    existing_keys: &HashSet<String>,
) -> String {
    if props.contains_key(raw_key) {
        return raw_key.to_string();
    }
    let snake = camel_to_snake(raw_key);
    if props.contains_key(&snake) && !existing_keys.contains(&snake) {
        return snake;
    }
    for prop_key in props.keys() {
        if prop_key.eq_ignore_ascii_case(raw_key) && !existing_keys.contains(prop_key) {
            return prop_key.clone();
        }
    }
    raw_key.to_string()
}

fn camel_to_snake(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 4);
    let mut prev_is_lower_or_digit = false;
    for ch in input.chars() {
        match ch {
            '-' => {
                out.push('_');
                prev_is_lower_or_digit = false;
            }
            c if c.is_ascii_uppercase() => {
                if prev_is_lower_or_digit && !out.ends_with('_') {
                    out.push('_');
                }
                out.push(c.to_ascii_lowercase());
                prev_is_lower_or_digit = false;
            }
            c => {
                out.push(c);
                prev_is_lower_or_digit = c.is_ascii_lowercase() || c.is_ascii_digit();
            }
        }
    }
    out
}

fn coerce_array(val: Value, schema: &Value, depth: usize) -> Value {
    let items_schema = schema.get("items");
    let raw_items = match val {
        Value::Array(arr) => arr,
        Value::String(ref raw) => match repair_json_value(raw) {
            Some(Value::Array(arr)) => arr,
            _ => match items_schema.and_then(schema_primary_type) {
                Some("string" | "integer" | "number" | "boolean") if !raw.trim().is_empty() => {
                    vec![Value::String(raw.clone())]
                }
                _ => return val,
            },
        },
        other @ (Value::Number(_) | Value::Bool(_) | Value::Object(_)) => vec![other],
        other => return other,
    };

    let coerced = match items_schema {
        Some(item_sch) => raw_items
            .into_iter()
            .map(|item| coerce_value_to_schema(item, item_sch, depth + 1))
            .collect(),
        None => raw_items,
    };
    Value::Array(coerced)
}

fn coerce_integer(val: Value) -> Value {
    match val {
        Value::Number(ref n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(_), _, _) | (_, Some(_), _) => val,
            (None, None, Some(f)) if f.is_finite() && f.fract() == 0.0 => {
                Value::Number(Number::from(f as i64))
            }
            _ => val,
        },
        Value::String(ref s) => {
            let t = s.trim();
            if let Ok(i) = t.parse::<i64>() {
                return Value::Number(Number::from(i));
            }
            if let Ok(u) = t.parse::<u64>() {
                return Value::Number(Number::from(u));
            }
            if let Ok(f) = t.parse::<f64>()
                && f.is_finite()
                && f.fract() == 0.0
            {
                return Value::Number(Number::from(f as i64));
            }
            val
        }
        Value::Bool(b) => Value::Number(Number::from(match b {
            true => 1i64,
            false => 0i64,
        })),
        other => other,
    }
}

fn coerce_number(val: Value) -> Value {
    match val {
        Value::Number(_) => val,
        Value::String(ref s) => match s.trim().parse::<f64>().ok().and_then(Number::from_f64) {
            Some(n) => Value::Number(n),
            None => val,
        },
        Value::Bool(b) => Value::Number(Number::from(match b {
            true => 1i64,
            false => 0i64,
        })),
        other => other,
    }
}

fn coerce_boolean(val: Value) -> Value {
    match val {
        Value::Bool(_) => val,
        Value::String(ref s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Value::Bool(true),
            "false" | "0" | "no" | "off" => Value::Bool(false),
            _ => val,
        },
        Value::Number(ref n) => match n.as_i64() {
            Some(1) => Value::Bool(true),
            Some(0) => Value::Bool(false),
            _ => val,
        },
        other => other,
    }
}

fn coerce_string(val: Value, schema: &Value) -> Value {
    let raw = match val {
        Value::String(s) => s,
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        other => return other,
    };

    if let Some(variants) = schema.get("enum").and_then(Value::as_array) {
        let trimmed = raw.trim();
        for variant in variants.iter().filter_map(Value::as_str) {
            if variant == trimmed {
                return Value::String(variant.to_string());
            }
        }
        for variant in variants.iter().filter_map(Value::as_str) {
            if variant.eq_ignore_ascii_case(trimmed) {
                return Value::String(variant.to_string());
            }
        }
    }

    Value::String(raw)
}

/// Formats a compact 1-line parameter signature directly from `spec.parameters`
/// so validation failures immediately tell the LLM how to fix its call on the next turn.
pub fn format_compact_schema_hint(spec: &ToolSpec) -> String {
    let Some(props) = spec.parameters.get("properties").and_then(Value::as_object) else {
        return String::new();
    };
    if props.is_empty() {
        return String::new();
    }
    let required: HashSet<&str> = spec
        .parameters
        .get("required")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut ordered: Vec<(&str, &Value, bool)> = props
        .iter()
        .map(|(k, v)| (k.as_str(), v, required.contains(k.as_str())))
        .collect();
    // Required parameters first, then alphabetical.
    ordered.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(b.0)));

    let total = ordered.len();
    let shown: Vec<String> = ordered
        .into_iter()
        .take(TOOL_SCHEMA_HINT_MAX_PROPS)
        .map(|(name, prop, is_req)| {
            let star = match is_req {
                true => "*",
                false => "",
            };
            let ty = schema_primary_type(prop).unwrap_or("any");
            let enum_hint = prop
                .get("enum")
                .and_then(Value::as_array)
                .filter(|vals| !vals.is_empty() && vals.len() <= 6)
                .map(|vals| {
                    let opts: Vec<&str> = vals.iter().filter_map(Value::as_str).collect();
                    match opts.is_empty() {
                        true => String::new(),
                        false => format!("[{}]", opts.join("|")),
                    }
                })
                .unwrap_or_default();
            format!("{name}{star}: {ty}{enum_hint}")
        })
        .collect();

    let more = match total > TOOL_SCHEMA_HINT_MAX_PROPS {
        true => format!(", +{} more", total - TOOL_SCHEMA_HINT_MAX_PROPS),
        false => String::new(),
    };
    format!(
        "Expected params for `{}` (* = required): {}{more}",
        spec.name,
        shown.join(", ")
    )
}

/// Deterministic zero-regex fuzzy matcher combining token overlap, substring boost, and
/// Damerau-Levenshtein distance over permitted tool names.
pub fn suggest_similar_tools(query: &str, candidates: &[&str], max_results: usize) -> Vec<String> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() || candidates.is_empty() || max_results == 0 {
        return Vec::new();
    }
    let q_tokens = split_ident_tokens(&q);

    let mut scored: Vec<(f64, &str)> = candidates
        .iter()
        .filter_map(|&cand| {
            let c = cand.to_ascii_lowercase();
            let dist = damerau_levenshtein(&q, &c);
            let max_len = q.len().max(c.len()).max(1) as f64;
            let edit_sim = 1.0 - (dist as f64 / max_len);

            let c_tokens = split_ident_tokens(&c);
            let shared = q_tokens
                .iter()
                .filter(|qt| {
                    c_tokens
                        .iter()
                        .any(|ct| ct == *qt || ct.starts_with(qt.as_str()) || qt.starts_with(ct.as_str()))
                })
                .count();
            let union = (q_tokens.len() + c_tokens.len()).saturating_sub(shared).max(1);
            let tok_sim = shared as f64 / union as f64;

            let substr_bonus = match c.contains(&q) || q.contains(&c) {
                true => 0.35,
                false => 0.0,
            };
            let score = edit_sim * 0.55 + tok_sim * 0.45 + substr_bonus;
            match score >= 0.30 {
                true => Some((score, cand)),
                false => None,
            }
        })
        .collect();

    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(max_results)
        .map(|(_, name)| name.to_string())
        .collect()
}

fn split_ident_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        match ch.is_ascii_alphanumeric() {
            true => cur.push(ch.to_ascii_lowercase()),
            false => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn damerau_levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut d = vec![vec![0usize; n + 1]; m + 1];
    for (i, row) in d.iter_mut().enumerate().take(m + 1) {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate().take(n + 1) {
        *cell = j;
    }
    for i in 1..=m {
        for j in 1..=n {
            let cost = match a_chars[i - 1] == b_chars[j - 1] {
                true => 0,
                false => 1,
            };
            let mut best = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1
                && j > 1
                && a_chars[i - 1] == b_chars[j - 2]
                && a_chars[i - 2] == b_chars[j - 1]
            {
                best = best.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = best;
        }
    }
    d[m][n]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use zenko_core::RiskLevel;

    #[test]
    fn repairs_raw_fenced_and_truncated_json_objects() {
        let schema = json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "start_line": { "type": "integer" },
                "include_hidden": { "type": "boolean" },
                "sort": { "type": "string", "enum": ["mtime", "path"] }
            },
            "required": ["path"]
        });

        let raw = json!({
            "_raw": "```json\n{\"path\": \"src/main.rs\", \"startLine\": \"42\", \"include_hidden\": \"true\", \"sort\": \"MTIME\",}\n```"
        });
        let fixed = coerce_and_repair_args(raw, &schema);
        assert_eq!(fixed["path"], json!("src/main.rs"));
        assert_eq!(fixed["start_line"], json!(42));
        assert_eq!(fixed["include_hidden"], json!(true));
        assert_eq!(fixed["sort"], json!("mtime"));

        let unclosed = json!({ "_raw": "{\"path\": \"Cargo.toml\", \"start_line\": \"10\"" });
        let fixed_unclosed = coerce_and_repair_args(unclosed, &schema);
        assert_eq!(fixed_unclosed["path"], json!("Cargo.toml"));
        assert_eq!(fixed_unclosed["start_line"], json!(10));
    }

    #[test]
    fn coerces_nested_arrays_and_scalar_lifts() {
        let schema = json!({
            "type": "object",
            "properties": {
                "names": { "type": "array", "items": { "type": "string" } },
                "ranges": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "start_line": { "type": "integer" },
                            "end_line": { "type": "integer" }
                        }
                    }
                }
            }
        });
        let input = json!({
            "names": "git_tool",
            "ranges": "[{\"startLine\": \"1\", \"endLine\": \"20\"}]"
        });
        let coerced = coerce_and_repair_args(input, &schema);
        assert_eq!(coerced["names"], json!(["git_tool"]));
        assert_eq!(coerced["ranges"][0]["start_line"], json!(1));
        assert_eq!(coerced["ranges"][0]["end_line"], json!(20));
    }

    #[test]
    fn fuzzy_suggests_closest_permitted_tools_and_formats_schema_hint() {
        let candidates = [
            "read_file",
            "hash_edit",
            "apply_patch",
            "code_search",
            "glob_find",
            "git_tool",
        ];
        let hits = suggest_similar_tools("readfile", &candidates, 3);
        assert_eq!(hits.first().map(String::as_str), Some("read_file"));

        let hits2 = suggest_similar_tools("hashedit", &candidates, 3);
        assert_eq!(hits2.first().map(String::as_str), Some("hash_edit"));

        let spec = ToolSpec {
            name: "glob_find".into(),
            description: "glob".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "sort": { "type": "string", "enum": ["mtime", "path"] }
                },
                "required": ["pattern"]
            }),
            risk: RiskLevel::Safe,
        };
        let hint = format_compact_schema_hint(&spec);
        assert!(hint.contains("pattern*: string"), "hint={hint}");
        assert!(hint.contains("sort: string[mtime|path]"), "hint={hint}");
    }
}
