//! SmartCrusher 2-Tier Pure-Rust JSON Compressor (Headroom + TOON inspired, zero ML, zero regex).
//!
//! - **Tier 1 (Lossless TOON — Token-Oriented Object Notation):** Converts uniform arrays,
//!   wrapped API/MCP objects (`{"meta":{...},"items":[...]}`), 1-level nested field groups
//!   (`author{login,id}`), keyed tabular maps (`services[N:]{...}:`), and inline primitive arrays
//!   into 100% lossless TOON whenever it stays within `SMART_CRUSHER_TOON_MAX_BYTES` (`3.2 KiB`)
//!   AND strictly beats minified JSON ("Never Worse vs Minified JSON" gate).
//! - **Tier 2 (Scored Keep/Drop Fallback):** Replaces blind head-only truncation on oversized or
//!   irregular arrays with deterministic signal scoring preserving:
//!   1) Structural anchors: first 2 + last 2 items,
//!   2) 100% of error / failed / `status >= 400` / `ok == false` items (bounded),
//!   3) Statistical numeric outliers (`> 2σ` via single-pass Welford online variance),
//!   4) Query-relevant keyword matches extracted from tool arguments,
//!   while preserving `_total_items`, `_omitted_items`, `_item_keys`, `_kept_reasons`, and `_sample`.

use super::split_optional_exit_header;
use super::toon::{encode_toon_bounded, max_nested_array_len};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use zenko_core::{
    READ_LESS_JSON_FULL_DEPTH, READ_LESS_JSON_PRIORITY_KEYS, READ_LESS_JSON_SCHEMA_SCAN_ITEMS,
    READ_LESS_MAX_JSON_KEYS, READ_LESS_MAX_STRING_CHARS, READ_LESS_SCRUB_MIN_BYTES,
    SMART_CRUSHER_ERROR_HTTP_MIN_CODE, SMART_CRUSHER_ERROR_KEYS, SMART_CRUSHER_ERROR_STATUS_VALUES,
    SMART_CRUSHER_HEAD_KEEP, SMART_CRUSHER_MAX_ERROR_ITEMS, SMART_CRUSHER_MAX_KEPT_BALANCED,
    SMART_CRUSHER_MAX_KEPT_HIGH, SMART_CRUSHER_MAX_KEPT_ULTRA, SMART_CRUSHER_MAX_OUTLIER_ITEMS,
    SMART_CRUSHER_MAX_QUERY_TOKENS, SMART_CRUSHER_MIN_NUMERIC_SAMPLES,
    SMART_CRUSHER_MIN_QUERY_TOKEN_LEN, SMART_CRUSHER_NULL_ERROR_VALUES, SMART_CRUSHER_OK_KEYS,
    SMART_CRUSHER_OUTLIER_SIGMA, SMART_CRUSHER_OUTLIER_SKIP_KEYS, SMART_CRUSHER_QUERY_ARG_KEYS,
    SMART_CRUSHER_QUERY_STOPWORDS, SMART_CRUSHER_STATUS_KEYS, SMART_CRUSHER_TABULAR_MAX_ITEMS,
    SMART_CRUSHER_TABULAR_MIN_SAVINGS_PCT, SMART_CRUSHER_TAIL_KEEP, SMART_CRUSHER_TOON_MAX_BYTES,
    SMART_CRUSHER_TOON_MIN_SAVINGS_VS_MINIFIED_PCT, ZMode,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonCrushKind {
    ToonLossless,
    JsonScored,
}

#[derive(Debug, Clone)]
pub struct JsonCrushOutput {
    pub kind: JsonCrushKind,
    pub rendered: String,
}

#[derive(Debug, Default, Clone, Copy)]
struct WelfordStats {
    n: usize,
    mean: f64,
    m2: f64,
}

impl WelfordStats {
    fn push(&mut self, x: f64) {
        if !x.is_finite() {
            return;
        }
        self.n += 1;
        let delta = x - self.mean;
        self.mean += delta / (self.n as f64);
        let delta2 = x - self.mean;
        self.m2 += delta * delta2;
    }

    fn stddev(&self) -> f64 {
        match self.n >= 2 {
            true => (self.m2 / ((self.n - 1) as f64)).sqrt(),
            false => 0.0,
        }
    }
}

/// Extracts lowercase ASCII query keywords from tool arguments (`query`, `command`, `url`, `filter`, etc.)
/// so Tier-2 SmartCrusher prioritizes JSON array rows relevant to the active call.
pub fn extract_query_tokens(tool_args: &Value) -> Vec<String> {
    let Some(obj) = tool_args.as_object() else {
        return Vec::new();
    };
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut tokens: Vec<String> = Vec::new();
    for key in SMART_CRUSHER_QUERY_ARG_KEYS {
        let Some(raw) = obj.get(*key).and_then(Value::as_str) else {
            continue;
        };
        for tok in raw.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
            if tokens.len() >= SMART_CRUSHER_MAX_QUERY_TOKENS {
                return tokens;
            }
            let lower = tok.to_ascii_lowercase();
            if lower.len() < SMART_CRUSHER_MIN_QUERY_TOKEN_LEN
                || lower.bytes().all(|b| b.is_ascii_digit())
                || SMART_CRUSHER_QUERY_STOPWORDS.contains(&lower.as_str())
            {
                continue;
            }
            if seen.insert(lower.clone()) {
                tokens.push(lower);
            }
        }
    }
    tokens
}

fn max_kept_for_zmode(zmode: ZMode) -> usize {
    match zmode {
        ZMode::Lite | ZMode::Balanced => SMART_CRUSHER_MAX_KEPT_BALANCED,
        ZMode::High => SMART_CRUSHER_MAX_KEPT_HIGH,
        ZMode::Ultra => SMART_CRUSHER_MAX_KEPT_ULTRA,
    }
}

fn parse_json_payload(text: &str) -> Option<(Option<&str>, usize, Value)> {
    let (header, body) = split_optional_exit_header(text);
    let trimmed = body.trim();
    if !matches!(trimmed.as_bytes().first(), Some(b'{' | b'[')) {
        return None;
    }
    let parsed: Value = serde_json::from_str(trimmed).ok()?;
    Some((header, trimmed.len(), parsed))
}

/// Core Tier-1 TOON encoder operating directly on an already-parsed `Value` (zero double parsing).
fn try_encode_toon_value(parsed: &Value, raw_trimmed_len: usize) -> Option<String> {
    // Oversized arrays (> 40 rows) skip lossless Tier-1 so Tier-2 Scored Keep/Drop crushes them cleanly.
    if max_nested_array_len(parsed, 0) > SMART_CRUSHER_TABULAR_MAX_ITEMS {
        return None;
    }

    let minified_len = serde_json::to_string(parsed).ok()?.len();
    if minified_len < READ_LESS_SCRUB_MIN_BYTES {
        return None;
    }
    let toon = encode_toon_bounded(parsed, SMART_CRUSHER_TOON_MAX_BYTES)?;

    // Strict "Never Worse vs Minified JSON" gate:
    // - Always strictly smaller than minified JSON.
    // - Must save >= 20% vs minified JSON, OR (>= 30% vs pretty raw AND still beat minified JSON).
    if toon.len() >= minified_len {
        return None;
    }
    let saved_vs_min_pct = minified_len
        .saturating_sub(toon.len())
        .saturating_mul(100)
        / minified_len.max(1);
    let saved_vs_raw_pct = raw_trimmed_len
        .saturating_sub(toon.len())
        .saturating_mul(100)
        / raw_trimmed_len.max(1);
    let beats_gate = saved_vs_min_pct >= SMART_CRUSHER_TOON_MIN_SAVINGS_VS_MINIFIED_PCT
        || (raw_trimmed_len > minified_len
            && saved_vs_raw_pct >= SMART_CRUSHER_TABULAR_MIN_SAVINGS_PCT);
    match beats_gate {
        true => Some(toon),
        false => None,
    }
}

fn attach_optional_header(header: Option<&str>, body: String) -> String {
    match header {
        Some(h) => format!("{h}\n{body}"),
        None => body,
    }
}

/// Tier-1 strictly lossless TOON compressor (safe to run below `read_less_min_bytes` because
/// zero rows, keys, or cells are dropped). Handles top-level arrays, wrapped API/MCP objects,
/// 1-level nested uniform field groups, keyed maps, and inline arrays, gated strictly against
/// **minified JSON** so irregular/tiny payloads never incur TOON syntax overhead ("Never Worse").
pub fn try_crush_json_toon_only(text: &str) -> Option<String> {
    let (header, raw_len, parsed) = parse_json_payload(text)?;
    let toon = try_encode_toon_value(&parsed, raw_len)?;
    Some(attach_optional_header(header, toon))
}

/// Top-level 2-tier SmartCrusher entry point for JSON tool outputs (single JSON parse).
pub fn try_crush_json_body(
    text: &str,
    tool_args: &Value,
    zmode: ZMode,
) -> Option<JsonCrushOutput> {
    let (header, raw_len, parsed) = parse_json_payload(text)?;
    if let Some(toon) = try_encode_toon_value(&parsed, raw_len) {
        return Some(JsonCrushOutput {
            kind: JsonCrushKind::ToonLossless,
            rendered: attach_optional_header(header, toon),
        });
    }

    let query_tokens = extract_query_tokens(tool_args);
    let max_kept = max_kept_for_zmode(zmode);
    let compacted = compact_json_value(&parsed, 0, &query_tokens, max_kept);
    let rendered = serde_json::to_string(&compacted).ok()?;
    Some(JsonCrushOutput {
        kind: JsonCrushKind::JsonScored,
        rendered: attach_optional_header(header, rendered),
    })
}

fn value_is_error_signal(val: &Value, depth: usize) -> bool {
    let Value::Object(map) = val else {
        return false;
    };
    for (k, v) in map {
        if SMART_CRUSHER_ERROR_KEYS
            .iter()
            .any(|ek| k.eq_ignore_ascii_case(ek))
        {
            match v {
                Value::Null => {}
                Value::Bool(b) => {
                    if *b {
                        return true;
                    }
                }
                Value::String(s) => {
                    let t = s.trim();
                    if !t.is_empty()
                        && !SMART_CRUSHER_NULL_ERROR_VALUES
                            .iter()
                            .any(|nv| t.eq_ignore_ascii_case(nv))
                    {
                        return true;
                    }
                }
                Value::Number(n) => {
                    if n.as_i64().is_some_and(|x| x != 0) || n.as_f64().is_some_and(|x| x != 0.0) {
                        return true;
                    }
                }
                Value::Array(a) => {
                    if !a.is_empty() {
                        return true;
                    }
                }
                Value::Object(o) => {
                    if !o.is_empty() {
                        return true;
                    }
                }
            }
        }
        if SMART_CRUSHER_OK_KEYS
            .iter()
            .any(|ok| k.eq_ignore_ascii_case(ok))
            && matches!(v, Value::Bool(false))
        {
            return true;
        }
        if SMART_CRUSHER_STATUS_KEYS
            .iter()
            .any(|sk| k.eq_ignore_ascii_case(sk))
        {
            match v {
                Value::Number(n) => {
                    if n.as_i64()
                        .is_some_and(|code| code >= SMART_CRUSHER_ERROR_HTTP_MIN_CODE)
                    {
                        return true;
                    }
                }
                Value::String(s) => {
                    let lower = s.trim().to_ascii_lowercase();
                    if SMART_CRUSHER_ERROR_STATUS_VALUES
                        .iter()
                        .any(|ev| lower == *ev || lower.starts_with(ev))
                    {
                        return true;
                    }
                }
                _ => {}
            }
        }
        if depth == 0 && matches!(v, Value::Object(_)) && value_is_error_signal(v, 1) {
            return true;
        }
    }
    false
}

/// Single-pass Welford outlier detector over top-level numeric fields of JSON object arrays.
/// Returns indices ordered by highest z-score (`> SMART_CRUSHER_OUTLIER_SIGMA`).
fn find_numeric_outliers(items: &[Value]) -> Vec<(usize, f64)> {
    let mut stats_by_key: BTreeMap<&str, WelfordStats> = BTreeMap::new();
    for item in items {
        let Some(obj) = item.as_object() else {
            continue;
        };
        for (k, v) in obj {
            // Skip monotonically increasing IDs / indices / timestamps via declarative table
            if SMART_CRUSHER_OUTLIER_SKIP_KEYS
                .iter()
                .any(|sk| k.eq_ignore_ascii_case(sk))
            {
                continue;
            }
            if let Some(num) = v.as_f64() {
                stats_by_key.entry(k.as_str()).or_default().push(num);
            }
        }
    }

    let active_stats: Vec<(&str, f64, f64)> = stats_by_key
        .into_iter()
        .filter_map(|(k, st)| {
            if st.n < SMART_CRUSHER_MIN_NUMERIC_SAMPLES {
                return None;
            }
            let sd = st.stddev();
            match sd > 1e-9 {
                true => Some((k, st.mean, sd)),
                false => None,
            }
        })
        .collect();
    if active_stats.is_empty() {
        return Vec::new();
    }

    let mut ranked: Vec<(usize, f64)> = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let mut best_z = 0.0f64;
        for &(k, mean, sd) in &active_stats {
            if let Some(num) = obj.get(k).and_then(Value::as_f64) {
                let z = (num - mean).abs() / sd;
                if z > best_z {
                    best_z = z;
                }
            }
        }
        if best_z > SMART_CRUSHER_OUTLIER_SIGMA {
            ranked.push((idx, best_z));
        }
    }

    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked
}

fn query_relevance_score(val: &Value, query_tokens: &[String]) -> usize {
    if query_tokens.is_empty() {
        return 0;
    }
    let hay = val.to_string().to_ascii_lowercase();
    query_tokens.iter().filter(|tok| hay.contains(tok.as_str())).count()
}

fn compact_json_value(val: &Value, depth: usize, query_tokens: &[String], max_kept: usize) -> Value {
    match val {
        Value::String(s) => match s.len() > READ_LESS_MAX_STRING_CHARS {
            true => {
                let cut = s.floor_char_boundary(READ_LESS_MAX_STRING_CHARS);
                let omitted = s.len().saturating_sub(cut);
                Value::String(format!("{}... [+{omitted} chars]", &s[..cut]))
            }
            false => Value::String(s.clone()),
        },
        Value::Array(items) => {
            if items.len() <= max_kept {
                return Value::Array(
                    items
                        .iter()
                        .map(|v| compact_json_value(v, depth + 1, query_tokens, max_kept))
                        .collect(),
                );
            }

            // Tier 2: Deterministic scored keep/drop preserving head + tail + errors + Welford outliers + query hits.
            let mut selected: BTreeSet<usize> = BTreeSet::new();
            let head_n = SMART_CRUSHER_HEAD_KEEP.min(items.len());
            for idx in 0..head_n {
                selected.insert(idx);
            }
            let tail_start = items.len().saturating_sub(SMART_CRUSHER_TAIL_KEEP);
            for idx in tail_start..items.len() {
                selected.insert(idx);
            }

            // Priority 1: Error / anomaly items (never blindly dropped).
            let mut error_kept = 0usize;
            for (idx, item) in items.iter().enumerate() {
                if error_kept >= SMART_CRUSHER_MAX_ERROR_ITEMS {
                    break;
                }
                if value_is_error_signal(item, 0) && selected.insert(idx) {
                    error_kept += 1;
                }
            }

            // Priority 2: Statistical numeric outliers (> 2σ via single-pass Welford).
            let mut outlier_kept = 0usize;
            for (idx, _) in find_numeric_outliers(items) {
                if outlier_kept >= SMART_CRUSHER_MAX_OUTLIER_ITEMS || selected.len() >= max_kept {
                    break;
                }
                if selected.insert(idx) {
                    outlier_kept += 1;
                }
            }

            // Priority 3: Query-relevant keyword matches from active tool arguments.
            let mut query_kept = 0usize;
            if !query_tokens.is_empty() && selected.len() < max_kept {
                let mut q_ranked: Vec<(usize, usize)> = items
                    .iter()
                    .enumerate()
                    .filter(|(idx, _)| !selected.contains(idx))
                    .filter_map(|(idx, item)| {
                        let score = query_relevance_score(item, query_tokens);
                        match score > 0 {
                            true => Some((idx, score)),
                            false => None,
                        }
                    })
                    .collect();
                q_ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                for (idx, _) in q_ranked {
                    if selected.len() >= max_kept {
                        break;
                    }
                    if selected.insert(idx) {
                        query_kept += 1;
                    }
                }
            }

            let sample: Vec<Value> = selected
                .iter()
                .map(|&idx| compact_json_value(&items[idx], depth + 1, query_tokens, max_kept))
                .collect();

            let mut schema_keys: BTreeSet<&str> = BTreeSet::new();
            for item in items.iter().take(READ_LESS_JSON_SCHEMA_SCAN_ITEMS) {
                if let Value::Object(map) = item {
                    for k in map.keys() {
                        if schema_keys.len() < READ_LESS_MAX_JSON_KEYS {
                            schema_keys.insert(k.as_str());
                        }
                    }
                }
            }

            let mut reasons: Vec<String> = vec![format!("head:{head_n}"), format!("tail:{}", items.len() - tail_start)];
            if error_kept > 0 {
                reasons.push(format!("errors:{error_kept}"));
            }
            if outlier_kept > 0 {
                reasons.push(format!("outliers:{outlier_kept}"));
            }
            if query_kept > 0 {
                reasons.push(format!("query:{query_kept}"));
            }

            let mut out = Map::new();
            out.insert("_total_items".into(), json!(items.len()));
            out.insert(
                "_omitted_items".into(),
                json!(items.len().saturating_sub(sample.len())),
            );
            out.insert(
                "_kept_reasons".into(),
                Value::Array(reasons.into_iter().map(Value::String).collect()),
            );
            if !schema_keys.is_empty() {
                out.insert(
                    "_item_keys".into(),
                    Value::Array(
                        schema_keys
                            .into_iter()
                            .map(|k| Value::String(k.to_string()))
                            .collect(),
                    ),
                );
            }
            out.insert("_sample".into(), Value::Array(sample));
            Value::Object(out)
        }
        Value::Object(map) => {
            let mut out = Map::new();
            let mut count = 0usize;
            for (k, v) in map {
                let is_priority = READ_LESS_JSON_PRIORITY_KEYS
                    .iter()
                    .any(|pk| k.eq_ignore_ascii_case(pk));
                if depth >= READ_LESS_JSON_FULL_DEPTH
                    && !is_priority
                    && count >= READ_LESS_MAX_JSON_KEYS
                {
                    continue;
                }
                out.insert(
                    k.clone(),
                    compact_json_value(v, depth + 1, query_tokens, max_kept),
                );
                count += 1;
            }
            let omitted = map.len().saturating_sub(out.len());
            if omitted > 0 {
                out.insert("_omitted_keys".into(), json!(omitted));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read_less::toon::decode_toon;

    #[test]
    fn tier1_lossless_toon_keeps_every_row_with_pipe_delimiter_and_nested_groups() {
        let payload = json!({
            "ok": true,
            "meta": { "page": 1, "total": 18 },
            "items": (0..18)
                .map(|i| {
                    json!({
                        "service_id": i,
                        "service_name": format!("svc-{i}"),
                        "region": "us-east-1",
                        "status": "healthy",
                        "latency_ms": 12 + (i % 5),
                        "owner": { "name": "Smith, Alice", "team": "core" }
                    })
                })
                .collect::<Vec<_>>()
        });
        let raw = serde_json::to_string(&payload).expect("minified json");
        let crushed = try_crush_json_body(&raw, &Value::Null, ZMode::Balanced).expect("crushed");
        assert_eq!(crushed.kind, JsonCrushKind::ToonLossless);
        assert!(crushed.rendered.contains("items[18|]{"), "out={}", crushed.rendered);
        assert!(crushed.rendered.contains("owner{name|team}"), "out={}", crushed.rendered);
        assert!(crushed.rendered.len() * 100 <= raw.len() * 65);
        assert_eq!(decode_toon(&crushed.rendered).expect("lossless decode"), payload);
    }

    #[test]
    fn never_worse_gate_skips_tiny_and_irregular_minified_json() {
        let tiny = r#"{"a":1,"b":"x"}"#;
        assert!(try_crush_json_toon_only(tiny).is_none());

        // Deeply irregular mixed array with syntax-heavy values where indented list form is
        // strictly larger than minified JSON: TOON gate must decline cleanly.
        let irregular = serde_json::to_string(&json!([
            {"a": "x:y", "b": [1, {"c": "[z]", "d": {"e": "1:2"}}]},
            {"f": "p:q", "g": [2, {"h": "{w}", "i": {"j": "3:4"}}]},
            {"k": "m:n", "l": [3, {"m": "[v]", "n": {"o": "5:6"}}]},
            {"p": "r:s", "q": [4, {"r": "{u}", "s": {"t": "7:8"}}]},
            {"u": "a:b", "v": [5, {"w": "[t]", "x": {"y": "9:0"}}]},
            {"aa": "q:w", "bb": [6, {"cc": "{r}", "dd": {"ee": "1:3"}}]},
            {"ff": "z:x", "gg": [7, {"hh": "[s]", "ii": {"jj": "2:4"}}]}
        ]))
        .expect("irregular json");
        assert!(irregular.len() >= READ_LESS_SCRUB_MIN_BYTES);
        assert!(
            try_crush_json_toon_only(&irregular).is_none(),
            "irregular minified JSON where TOON is larger must be skipped"
        );
    }

    #[test]
    fn tier2_preserves_head_tail_errors_welford_outliers_and_query_matches() {
        let mut items: Vec<Value> = (0..80)
            .map(|i| {
                json!({
                    "id": i,
                    "service": format!("worker-{i}"),
                    "status": 200,
                    "ok": true,
                    "latency_ms": 14.0 + (i % 3) as f64,
                    "note": "routine heartbeat check"
                })
            })
            .collect();
        // Inject hidden error in the middle (item #33), ok=false (item #51), latency spike outlier (item #42), and query match (item #60).
        items[33]["status"] = json!(503);
        items[33]["error"] = json!("upstream database pool exhausted");
        items[51]["ok"] = json!(false);
        items[42]["latency_ms"] = json!(9850.0);
        items[60]["note"] = json!("payment_webhook retry scheduled");

        let raw = serde_json::to_string(&Value::Array(items)).expect("json");
        let args = json!({ "command": "curl -s https://api.local/jobs?filter=payment_webhook" });
        let crushed = try_crush_json_body(&raw, &args, ZMode::Balanced).expect("crushed");
        assert_eq!(crushed.kind, JsonCrushKind::JsonScored);
        let parsed: Value = serde_json::from_str(&crushed.rendered).expect("valid json");
        assert_eq!(parsed["_total_items"], json!(80));
        let sample = parsed["_sample"].as_array().expect("sample array");
        let ids: Vec<i64> = sample
            .iter()
            .filter_map(|v| v.get("id").and_then(Value::as_i64))
            .collect();
        // Head (0,1), tail (78,79), errors (33,51), outlier (42), query match (60) must ALL survive!
        for must_keep in [0, 1, 33, 42, 51, 60, 78, 79] {
            assert!(
                ids.contains(&must_keep),
                "expected item {must_keep} in kept ids {ids:?}"
            );
        }
        let reasons = parsed["_kept_reasons"].to_string();
        assert!(reasons.contains("errors:2"), "reasons={reasons}");
        assert!(reasons.contains("outliers:1"), "reasons={reasons}");
        assert!(reasons.contains("query:1"), "reasons={reasons}");
    }
}
