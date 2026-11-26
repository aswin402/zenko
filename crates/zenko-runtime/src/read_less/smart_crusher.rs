//! SmartCrusher 2-Tier Pure-Rust JSON Compressor (Headroom-inspired, zero ML, zero regex).
//!
//! - **Tier 1 (Lossless Tabular JSON -> Compact CSV):** When a JSON array of objects has
//!   consistent flat scalar keys (`SMART_CRUSHER_TABULAR_MIN_ITEMS..=SMART_CRUSHER_TABULAR_MAX_ITEMS`,
//!   `>= 80%` key overlap, compact CSV `<= 2.4 KiB` and saving `>= 30%` bytes with zero truncated
//!   cells), emits a 100% lossless tabular representation preserving every row.
//! - **Tier 2 (Scored Keep/Drop Fallback):** Replaces blind head-only truncation with deterministic
//!   signal scoring preserving:
//!   1) Structural anchors: first 2 + last 2 items,
//!   2) 100% of error / failed / `status >= 400` / `ok == false` items (bounded),
//!   3) Statistical numeric outliers (`> 2σ` via single-pass Welford online variance),
//!   4) Query-relevant keyword matches extracted from tool arguments,
//!   while preserving `_total_items`, `_omitted_items`, `_item_keys`, `_kept_reasons`, and `_sample`.

use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use zenko_core::{
    READ_LESS_JSON_FULL_DEPTH, READ_LESS_JSON_PRIORITY_KEYS, READ_LESS_JSON_SCHEMA_SCAN_ITEMS,
    READ_LESS_MAX_JSON_KEYS, READ_LESS_MAX_STRING_CHARS, SMART_CRUSHER_ERROR_KEYS,
    SMART_CRUSHER_ERROR_STATUS_VALUES, SMART_CRUSHER_HEAD_KEEP, SMART_CRUSHER_MAX_ERROR_ITEMS,
    SMART_CRUSHER_MAX_KEPT_BALANCED, SMART_CRUSHER_MAX_KEPT_HIGH, SMART_CRUSHER_MAX_KEPT_ULTRA,
    SMART_CRUSHER_MAX_OUTLIER_ITEMS, SMART_CRUSHER_MAX_QUERY_TOKENS,
    SMART_CRUSHER_MIN_NUMERIC_SAMPLES, SMART_CRUSHER_MIN_QUERY_TOKEN_LEN, SMART_CRUSHER_OK_KEYS,
    SMART_CRUSHER_OUTLIER_SIGMA, SMART_CRUSHER_QUERY_ARG_KEYS, SMART_CRUSHER_QUERY_STOPWORDS,
    SMART_CRUSHER_STATUS_KEYS, SMART_CRUSHER_TABULAR_MAX_BYTES,
    SMART_CRUSHER_TABULAR_MAX_CELL_CHARS, SMART_CRUSHER_TABULAR_MAX_COLS,
    SMART_CRUSHER_TABULAR_MAX_ITEMS, SMART_CRUSHER_TABULAR_MIN_ITEMS,
    SMART_CRUSHER_TABULAR_MIN_OVERLAP_PCT, SMART_CRUSHER_TABULAR_MIN_SAVINGS_PCT,
    SMART_CRUSHER_TAIL_KEEP, ZMode,
};

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

fn split_optional_exit_header(text: &str) -> (Option<&str>, &str) {
    match text.strip_prefix("exit_code:") {
        Some(_) => match text.find('\n') {
            Some(pos) => (Some(&text[..pos]), &text[pos + 1..]),
            None => (Some(text), ""),
        },
        None => (None, text),
    }
}

/// Top-level 2-tier SmartCrusher entry point for JSON tool outputs.
pub fn try_crush_json_body(text: &str, tool_args: &Value, zmode: ZMode) -> Option<String> {
    let (header, body) = split_optional_exit_header(text);
    let trimmed = body.trim();
    if !matches!(trimmed.as_bytes().first(), Some(b'{' | b'[')) {
        return None;
    }
    let parsed: Value = serde_json::from_str(trimmed).ok()?;

    // Fast path: top-level uniform flat object array -> lossless compact CSV table.
    if let Value::Array(items) = &parsed
        && let Some((cols, csv_body)) = try_render_lossless_tabular_csv(items)
    {
        let table = format!(
            "# lossless-tabular-json (rows={}, cols={})\n{csv_body}",
            items.len(),
            cols
        );
        let compact_raw_len = serde_json::to_string(&parsed).map(|s| s.len()).unwrap_or(trimmed.len());
        let saved_pct = compact_raw_len
            .saturating_sub(table.len())
            .saturating_mul(100)
            / compact_raw_len.max(1);
        if table.len() < compact_raw_len && saved_pct >= SMART_CRUSHER_TABULAR_MIN_SAVINGS_PCT {
            return Some(match header {
                Some(h) => format!("{h}\n{table}"),
                None => table,
            });
        }
    }

    let query_tokens = extract_query_tokens(tool_args);
    let max_kept = max_kept_for_zmode(zmode);
    let compacted = compact_json_value(&parsed, 0, &query_tokens, max_kept);
    let rendered = serde_json::to_string(&compacted).ok()?;
    Some(match header {
        Some(h) => format!("{h}\n{rendered}"),
        None => rendered,
    })
}

fn format_csv_cell(val: &Value) -> Option<String> {
    let raw = match val {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            if s.len() > SMART_CRUSHER_TABULAR_MAX_CELL_CHARS {
                return None;
            }
            s.clone()
        }
        // Non-scalar nested objects/arrays cannot be losslessly flattened into plain CSV cells.
        Value::Array(_) | Value::Object(_) => return None,
    };
    let needs_quote = raw.bytes().any(|b| matches!(b, b',' | b'"' | b'\n' | b'\r'));
    Some(match needs_quote {
        true => format!("\"{}\"", raw.replace('"', "\"\"")),
        false => raw,
    })
}

/// Tier 1: Attempts 100% lossless tabular CSV rendering for uniform flat JSON object arrays.
/// Returns `Some((col_count, csv_text))` only when every item is a flat scalar object, key overlap
/// is `>= 80%`, total rows are within `6..=40`, and rendered CSV fits within `2.4 KiB`.
fn try_render_lossless_tabular_csv(items: &[Value]) -> Option<(usize, String)> {
    if items.len() < SMART_CRUSHER_TABULAR_MIN_ITEMS
        || items.len() > SMART_CRUSHER_TABULAR_MAX_ITEMS
    {
        return None;
    }

    let mut cols: Vec<&str> = Vec::new();
    let mut seen_cols: BTreeSet<&str> = BTreeSet::new();
    let mut present_cells = 0usize;

    for item in items {
        let obj = item.as_object()?;
        if obj.is_empty() {
            return None;
        }
        present_cells = present_cells.saturating_add(obj.len());
        for (k, v) in obj {
            if matches!(v, Value::Array(_) | Value::Object(_)) {
                return None;
            }
            if seen_cols.insert(k.as_str()) {
                cols.push(k.as_str());
                if cols.len() > SMART_CRUSHER_TABULAR_MAX_COLS {
                    return None;
                }
            }
        }
    }

    if cols.is_empty() {
        return None;
    }
    let total_cells = items.len().saturating_mul(cols.len());
    let overlap_pct = present_cells.saturating_mul(100) / total_cells.max(1);
    if overlap_pct < SMART_CRUSHER_TABULAR_MIN_OVERLAP_PCT {
        return None;
    }

    let mut lines: Vec<String> = Vec::with_capacity(items.len() + 1);
    let header_cells: Vec<String> = cols
        .iter()
        .map(|c| {
            let needs_quote = c.bytes().any(|b| matches!(b, b',' | b'"' | b'\n' | b'\r'));
            match needs_quote {
                true => format!("\"{}\"", c.replace('"', "\"\"")),
                false => (*c).to_string(),
            }
        })
        .collect();
    lines.push(header_cells.join(","));

    let mut total_bytes = lines[0].len();
    for item in items {
        let obj = item.as_object()?;
        let mut row: Vec<String> = Vec::with_capacity(cols.len());
        for &col in &cols {
            let cell = match obj.get(col) {
                Some(v) => format_csv_cell(v)?,
                None => String::new(),
            };
            row.push(cell);
        }
        let line = row.join(",");
        total_bytes = total_bytes.saturating_add(line.len() + 1);
        if total_bytes > SMART_CRUSHER_TABULAR_MAX_BYTES {
            return None;
        }
        lines.push(line);
    }

    Some((cols.len(), lines.join("\n")))
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
                    if !t.is_empty() && !t.eq_ignore_ascii_case("none") && !t.eq_ignore_ascii_case("null") {
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
                    if n.as_i64().is_some_and(|code| code >= 400) {
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
            // Skip monotonically increasing IDs / indices / timestamps by avoiding purely uniform sequences
            if k.eq_ignore_ascii_case("id") || k.eq_ignore_ascii_case("index") {
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
            // Tier 1 (nested): lossless CSV when compact and saving >= 30% bytes.
            if let Some((cols, csv_body)) = try_render_lossless_tabular_csv(items) {
                let raw_len = serde_json::to_string(items).map(|s| s.len()).unwrap_or(0);
                let candidate = json!({
                    "_format": "lossless_csv",
                    "_rows": items.len(),
                    "_cols": cols,
                    "_csv": csv_body,
                });
                let cand_len = serde_json::to_string(&candidate).map(|s| s.len()).unwrap_or(raw_len);
                let saved_pct = raw_len.saturating_sub(cand_len).saturating_mul(100) / raw_len.max(1);
                if raw_len > 0 && cand_len < raw_len && saved_pct >= SMART_CRUSHER_TABULAR_MIN_SAVINGS_PCT {
                    return candidate;
                }
            }

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

    #[test]
    fn tier1_lossless_tabular_csv_keeps_every_row_and_saves_over_30_pct() {
        let items: Vec<Value> = (0..18)
            .map(|i| {
                json!({
                    "service_id": i,
                    "service_name": format!("svc-{i}"),
                    "region": "us-east-1",
                    "status": "healthy",
                    "latency_ms": 12 + (i % 5),
                    "owner": "Smith, Alice"
                })
            })
            .collect();
        let raw = serde_json::to_string_pretty(&Value::Array(items)).expect("json");
        let crushed = try_crush_json_body(&raw, &Value::Null, ZMode::Balanced).expect("crushed");
        assert!(crushed.starts_with("# lossless-tabular-json (rows=18, cols=6)"));
        assert!(crushed.contains("\"Smith, Alice\""));
        for i in 0..18 {
            assert!(crushed.contains(&format!("svc-{i}")), "missing row svc-{i}");
        }
        assert!(crushed.len() * 100 <= raw.len() * 65);
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
        let parsed: Value = serde_json::from_str(&crushed).expect("valid json");
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
