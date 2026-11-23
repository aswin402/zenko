//! Zero-regex deterministic single-pass secret redactor.
//! Scrubs active vault secrets, Bearer auth headers, Telegram `/bot<id>:<token>` paths,
//! and common API key prefixes before any log or diagnostic payload is written to disk.

use crate::constants::{
    MEMORY_INJECTION_BLOCKED_TAGS, REDACTED_PLACEHOLDER, SECRET_TOKEN_PREFIXES,
};
use serde_json::Value;

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':')
}

fn consume_token(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while end < bytes.len() && is_token_byte(bytes[end]) {
        end += 1;
    }
    end
}

fn match_ascii_ci(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.len() >= needle.len()
        && haystack[..needle.len()].eq_ignore_ascii_case(needle)
}

/// Checks for Telegram bot token pattern `bot<digits>:<alnum_token>` starting at `bytes[i..]`.
fn match_telegram_bot_token(bytes: &[u8], start: usize) -> Option<usize> {
    let slice = &bytes[start..];
    if !match_ascii_ci(slice, b"bot") {
        return None;
    }
    let mut pos = start + 3;
    let digits_start = pos;
    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
        pos += 1;
    }
    if pos.saturating_sub(digits_start) < 5 || pos >= bytes.len() || bytes[pos] != b':' {
        return None;
    }
    pos += 1;
    let tok_start = pos;
    let end = consume_token(bytes, tok_start);
    match end.saturating_sub(tok_start) >= 12 {
        true => Some(end),
        false => None,
    }
}

/// Deterministic zero-regex scrubber for log lines, wire dumps, and debug bundles.
pub fn scrub_secrets(input: &str, known_secrets: &[String]) -> String {
    let mut work = input.to_string();
    for secret in known_secrets {
        let trimmed = secret.trim();
        if trimmed.len() >= 6 && work.contains(trimmed) {
            work = work.replace(trimmed, REDACTED_PLACEHOLDER);
        }
    }

    let bytes = work.as_bytes();
    let mut out = String::with_capacity(work.len());
    let mut i = 0usize;

    while i < bytes.len() {
        let rest = &bytes[i..];

        // 1. `Bearer <token>` (case-insensitive)
        if match_ascii_ci(rest, b"bearer ") {
            let tok_start = i + 7;
            let tok_end = consume_token(bytes, tok_start);
            if tok_end.saturating_sub(tok_start) >= 8 {
                out.push_str(&work[i..tok_start]);
                out.push_str(REDACTED_PLACEHOLDER);
                i = tok_end;
                continue;
            }
        }

        // 2. Telegram `bot123456:AAH...`
        if let Some(tg_end) = match_telegram_bot_token(bytes, i) {
            out.push_str("bot");
            out.push_str(REDACTED_PLACEHOLDER);
            i = tg_end;
            continue;
        }

        // 3. Known API key prefixes (`sk-or-`, `sk-ant-`, `sk-`, `ghp_`, `xoxb-`, ...)
        let matched_prefix = SECRET_TOKEN_PREFIXES
            .iter()
            .find(|pref| rest.starts_with(pref.as_bytes()));
        if let Some(pref) = matched_prefix {
            let tok_end = consume_token(bytes, i);
            if tok_end.saturating_sub(i) >= pref.len() + 8 {
                out.push_str(REDACTED_PLACEHOLDER);
                i = tok_end;
                continue;
            }
        }

        let ch = work[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

const SECRET_JSON_KEY_SUBSTRINGS: &[&str] = &["api_key", "authorization", "secret"];
const NON_SECRET_TOKEN_FIELDS: &[&str] = &[
    "prompt_tokens",
    "completion_tokens",
    "total_tokens",
    "max_tokens",
    "cache_read_tokens",
    "cached_tokens",
];

fn is_secret_json_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_JSON_KEY_SUBSTRINGS
        .iter()
        .any(|needle| lower.contains(needle))
        || (lower.contains("token") && !NON_SECRET_TOKEN_FIELDS.contains(&lower.as_str()))
}

/// Recursively scrubs string values inside arbitrary JSON payloads.
pub fn scrub_json(val: Value, known_secrets: &[String]) -> Value {
    match val {
        Value::String(s) => Value::String(scrub_secrets(&s, known_secrets)),
        Value::Array(arr) => Value::Array(
            arr.into_iter()
                .map(|item| scrub_json(item, known_secrets))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| {
                    let next_v = match (is_secret_json_key(&k), v) {
                        (true, Value::String(ref s)) if !s.trim().is_empty() => {
                            Value::String(REDACTED_PLACEHOLDER.to_string())
                        }
                        (_, other) => scrub_json(other, known_secrets),
                    };
                    (k, next_v)
                })
                .collect(),
        ),
        primitive => primitive,
    }
}

/// Deterministic zero-regex prompt-injection tag sanitizer for untrusted external tool metadata
/// and outputs (e.g. external MCP server descriptions/results). Neutralizes angle-bracketed
/// control tags from `MEMORY_INJECTION_BLOCKED_TAGS` while preserving normal text.
pub fn sanitize_untrusted_markup(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let mut j = i + 1;
            if j < bytes.len() && bytes[j] == b'/' {
                j += 1;
            }
            let name_start = j;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || matches!(bytes[j], b'_' | b'-')) {
                j += 1;
            }
            let tag_name = &input[name_start..j];
            let is_blocked = !tag_name.is_empty()
                && MEMORY_INJECTION_BLOCKED_TAGS
                    .iter()
                    .any(|b| tag_name.eq_ignore_ascii_case(b));
            if is_blocked
                && let Some(rel_end) = input[j..].find('>') {
                    out.push('[');
                    out.push_str(tag_name);
                    out.push(']');
                    i = j + rel_end + 1;
                    continue;
                }
        }
        let ch = input[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_bearer_telegram_api_keys_and_vault_secrets_without_regex() {
        let known = vec!["my-custom-vault-secret-999".to_string()];
        let sample = "GET https://api.telegram.org/bot12345678:AAHxyz1234567890_abc/getMe \
                      Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig \
                      key=sk-or-v1-abcdef1234567890 \
                      custom=my-custom-vault-secret-999 \
                      normal=ok";
        let scrubbed = scrub_secrets(sample, &known);
        assert!(!scrubbed.contains("AAHxyz1234567890_abc"));
        assert!(!scrubbed.contains("eyJhbGciOiJIUzI1NiJ9"));
        assert!(!scrubbed.contains("sk-or-v1-abcdef1234567890"));
        assert!(!scrubbed.contains("my-custom-vault-secret-999"));
        assert!(scrubbed.contains("normal=ok"));

        let j = scrub_json(
            json!({ "api_key": "secret123", "prompt_tokens": 42, "msg": "Bearer supersecret1234" }),
            &known,
        );
        assert_eq!(j["api_key"], REDACTED_PLACEHOLDER);
        assert_eq!(j["prompt_tokens"], 42);
        assert_eq!(j["msg"], "Bearer [REDACTED]");
    }
}
