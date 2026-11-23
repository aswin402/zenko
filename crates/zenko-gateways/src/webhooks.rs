use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use subtle::ConstantTimeEq;
pub use zenko_core::{TriggerRateLimiter, TriggerRateSnapshot};
use zenko_core::WebhookRuleConfig;

type HmacSha256 = Hmac<Sha256>;

/// Verifies an `X-Zenko-Signature: sha256=<hex>` HMAC header in constant time.
pub fn verify_hmac_sha256(secret: &str, body: &[u8], signature_header: &str) -> bool {
    let sig_hex = signature_header
        .trim()
        .strip_prefix("sha256=")
        .unwrap_or(signature_header.trim());
    let Ok(provided) = hex::decode(sig_hex) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    let expected = mac.finalize().into_bytes();
    if provided.len() != expected.len() {
        return false;
    }
    expected.as_slice().ct_eq(&provided).into()
}

/// Returns `true` when at least one enabled webhook rule explicitly targets `source` (or wildcard `*`).
pub fn has_webhook_rules_for_source(rules: &[WebhookRuleConfig], source: &str) -> bool {
    let clean = source.trim();
    rules.iter().any(|r| {
        r.enabled && (r.source.trim() == "*" || r.source.trim().eq_ignore_ascii_case(clean))
    })
}

/// Deterministic zero-regex webhook rule matching using source equality + RFC 6901 JSON Pointer.
pub fn match_webhook_rule(rule: &WebhookRuleConfig, source: &str, payload: Option<&Value>) -> bool {
    if !rule.enabled {
        return false;
    }
    let src_ok = rule.source.trim() == "*" || rule.source.trim().eq_ignore_ascii_case(source.trim());
    if !src_ok {
        return false;
    }
    let Some(ptr) = rule
        .match_pointer
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return true;
    };
    let Some(json) = payload else {
        return false;
    };
    let normalized_ptr = match ptr.starts_with('/') {
        true => ptr.to_string(),
        false => format!("/{ptr}"),
    };
    let Some(val) = json.pointer(&normalized_ptr) else {
        return false;
    };
    if rule.match_values.is_empty() {
        return !val.is_null();
    }
    let scalar = match val {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => return false,
    };
    rule.match_values
        .iter()
        .any(|want| want.trim().eq_ignore_ascii_case(scalar.trim()))
}

/// Safe placeholder prompt formatter (`{source}`, `{rule_id}`, `{payload}`) with zero regex.
pub fn render_webhook_prompt(
    template: &str,
    source: &str,
    rule_id: &str,
    payload_text: &str,
) -> String {
    let trimmed = template.trim();
    if trimmed.is_empty() {
        return format!("Incoming signed webhook `{rule_id}` from `{source}`:\n{payload_text}");
    }
    trimmed
        .replace("{source}", source)
        .replace("{rule_id}", rule_id)
        .replace("{payload}", payload_text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hmac_verification_works() {
        let secret = "super-secret";
        let body = b"{\"event\":\"deploy\"}";
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac");
        mac.update(body);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(verify_hmac_sha256(secret, body, &sig));
        assert!(!verify_hmac_sha256(secret, b"tampered", &sig));
    }

    #[test]
    fn webhook_json_pointer_rule_matching_and_rate_limiter_work() {
        let rule = WebhookRuleConfig {
            id: "gh-ci-fail".into(),
            enabled: true,
            source: "github".into(),
            match_pointer: Some("/workflow_run/conclusion".into()),
            match_values: vec!["failure".into(), "timed_out".into()],
            prompt_template: "CI failed on {source} ({rule_id}): {payload}".into(),
            session_key: None,
            workspace_dir: None,
            model: None,
            goal_loop_id: None,
            cooldown_secs: 60,
            max_per_hour: 2,
        };
        let fail_payload = json!({"workflow_run": {"conclusion": "failure"}});
        let ok_payload = json!({"workflow_run": {"conclusion": "success"}});
        assert!(match_webhook_rule(&rule, "github", Some(&fail_payload)));
        assert!(!match_webhook_rule(&rule, "github", Some(&ok_payload)));
        assert!(!match_webhook_rule(&rule, "gitlab", Some(&fail_payload)));
        assert!(has_webhook_rules_for_source(&[rule.clone()], "github"));
        assert!(!has_webhook_rules_for_source(&[rule.clone()], "gitlab"));

        let rendered = render_webhook_prompt(&rule.prompt_template, "github", &rule.id, "{}");
        assert_eq!(rendered, "CI failed on github (gh-ci-fail): {}");

        let limiter = TriggerRateLimiter::new();
        assert!(limiter.check_and_record("gh-ci-fail", 0, 2).is_ok());
        assert!(limiter.check_and_record("gh-ci-fail", 0, 2).is_ok());
        assert!(limiter.check_and_record("gh-ci-fail", 0, 2).is_err());
        assert_eq!(limiter.snapshot("gh-ci-fail").fires_last_hour, 2);

        // Cooldown gate blocks immediate back-to-back trigger
        assert!(limiter.check_and_record("cool-rule", 60, 10).is_ok());
        assert!(limiter.check_and_record("cool-rule", 60, 10).is_err());
    }
}
