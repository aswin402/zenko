//! Language-Agnostic Structural Turn Profiler (zero hardcoded keyword dictionaries,
//! zero regex, zero `if/else` ladders, zero `/commands`).
//!
//! Instead of guessing user intent from English word lists and locking out LLM tools,
//! Zenko profiles strictly objective structural signals (`PermissionMode::Plan`,
//! `is_first_turn`, mentioned workspace paths, prior turn errors, configured model slots)
//! and hands full strategic sovereignty to the LLM itself via `orchestrate_control`,
//! `record_brainstorm`, `ask_clarification`, `plan_update`, `session_task`, and `spawn_agent`.

use zenko_core::{
    DeliberationDepth, ExecutionTier, IntentKind, ModelRoutingRole, PermissionMode,
    ReasoningEffort, TriageReport,
};

#[derive(Debug, Clone)]
pub struct TriageInput<'a> {
    pub prompt: &'a str,
    pub prior_turn_errors: usize,
    pub is_first_turn: bool,
    pub permission_mode: PermissionMode,
    pub has_fast_model: bool,
    pub has_plan_model: bool,
}

const FILE_EXT_SUFFIXES: &[&str] = &[
    ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".toml", ".json", ".yaml", ".yml", ".sql",
    ".md", ".sh", ".c", ".h", ".cpp", ".hpp", ".java", ".kt", ".swift", ".rb", ".php", ".css",
    ".html",
];

/// Zero-regex deterministic ASCII lowercase word tokenizer (splits on non-alphanumeric bytes).
pub fn tokenize_words(prompt: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    for ch in prompt.chars() {
        match ch.is_ascii_alphanumeric() {
            true => cur.push(ch.to_ascii_lowercase()),
            false => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

/// Extracts syntax-level path-like tokens mentioned in the prompt (reused by zero-token recon).
pub fn extract_path_like_tokens(prompt: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in prompt.split_whitespace() {
        let clean = raw.trim_matches(|c: char| {
            matches!(
                c,
                '`' | '"' | '\'' | ',' | '.' | '(' | ')' | '[' | ']' | ':' | ';' | '?' | '!'
            )
        });
        if clean.is_empty()
            || clean.starts_with("http://")
            || clean.starts_with("https://")
            || clean.starts_with("--")
        {
            continue;
        }
        let has_slash = clean.contains('/') && !clean.starts_with('/');
        let has_ext = FILE_EXT_SUFFIXES.iter().any(|ext| clean.ends_with(ext));
        if (has_slash || has_ext) && !out.iter().any(|x| x == clean) {
            out.push(clean.to_string());
        }
    }
    out
}

/// Profiles a turn using strictly objective, language-agnostic structural metadata.
/// Never guesses read-only lockouts from keywords; the LLM owns strategy & depth completely.
pub fn classify_turn(input: &TriageInput<'_>) -> TriageReport {
    let trimmed = input.prompt.trim();
    let line_count = trimmed.lines().filter(|l| !l.trim().is_empty()).count();
    let path_count = extract_path_like_tokens(trimmed).len();
    let is_explicit_plan_mode = input.permission_mode == PermissionMode::Plan;

    let mut matched_signals: Vec<String> = Vec::with_capacity(6);
    if is_explicit_plan_mode {
        matched_signals.push("explicit_plan_mode".into());
    }
    if input.is_first_turn {
        matched_signals.push("first_turn_grounding".into());
    }
    if path_count > 0 {
        matched_signals.push(format!("paths({path_count})"));
    }
    if input.prior_turn_errors > 0 {
        matched_signals.push(format!("prior_errors({})", input.prior_turn_errors));
    }
    if line_count >= 4 {
        matched_signals.push(format!("multiline({line_count})"));
    }

    let needs_recon =
        is_explicit_plan_mode || input.is_first_turn || path_count > 0 || input.prior_turn_errors > 0;

    let tier = match is_explicit_plan_mode || needs_recon || line_count >= 4 {
        true => ExecutionTier::Orchestrated,
        false => ExecutionTier::Standard,
    };

    // Only explicit user-chosen `PermissionMode::Plan` restricts mutations.
    // All other modes grant full LLM freedom to read, brainstorm, plan, delegate, or edit.
    let intent = match is_explicit_plan_mode {
        true => IntentKind::Advisory,
        false => IntentKind::Implement,
    };

    let deliberation = match is_explicit_plan_mode || input.prior_turn_errors > 0 {
        true => DeliberationDepth::Deliberate,
        false => DeliberationDepth::Standard,
    };

    let recommended_model_role = match (is_explicit_plan_mode, input.has_plan_model) {
        (true, true) => ModelRoutingRole::Plan,
        _ => ModelRoutingRole::Lead,
    };

    let recommended_effort = match (is_explicit_plan_mode, input.prior_turn_errors > 0) {
        (true, _) | (_, true) => ReasoningEffort::High,
        (false, false) => ReasoningEffort::Medium,
    };

    let complexity_score = (path_count.min(4) as i32)
        + (input.prior_turn_errors.min(3) as i32)
        + match line_count {
            0..=3 => 0,
            4..=12 => 1,
            _ => 2,
        };

    let reason = match (is_explicit_plan_mode, needs_recon) {
        (true, _) => {
            "Explicit read-only Plan Mode -> situational recon + Plan role (LLM controls brainstorm & DAG).".into()
        }
        (false, true) => {
            "LLM-sovereign turn -> lightweight situational recon attached; LLM controls posture, brainstorming, gear & delegation.".into()
        }
        (false, false) => {
            "LLM-sovereign turn -> direct ReAct membrane; LLM may call `orchestrate_control`, `record_brainstorm`, or `spawn_agent` anytime.".into()
        }
    };

    TriageReport {
        tier,
        intent,
        deliberation,
        complexity_score,
        ambiguity_score: 0,
        risk_score: 0,
        matched_signals,
        recommended_model_role,
        recommended_effort,
        needs_recon,
        needs_brainstorm: false,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiler_is_language_agnostic_and_never_keyword_locks_llm() {
        // Non-English, casual, or "explain + fix" prompts MUST stay sovereign (never keyword-locked read-only)
        for prompt in [
            "hi!",
            "explain why auth fails and fix it",
            "compare Redis vs SQLite and build whichever is cleaner",
            "இதை சரி செய் crates/zenko-runtime/src/lib.rs",
            "don't hesitate, refactor everything",
        ] {
            let report = classify_turn(&TriageInput {
                prompt,
                prior_turn_errors: 0,
                is_first_turn: false,
                permission_mode: PermissionMode::Autonomous,
                has_fast_model: true,
                has_plan_model: true,
            });
            assert_eq!(
                report.intent,
                IntentKind::Implement,
                "prompt `{prompt}` must never be keyword-locked read-only"
            );
            assert!(!report.intent.is_read_only());
            assert_eq!(report.recommended_model_role, ModelRoutingRole::Lead);
            assert!(!report.needs_brainstorm);
        }

        // Explicit user PermissionMode::Plan honors read-only Advisory + Plan slot
        let plan_mode = classify_turn(&TriageInput {
            prompt: "design the caching layer",
            prior_turn_errors: 0,
            is_first_turn: true,
            permission_mode: PermissionMode::Plan,
            has_fast_model: true,
            has_plan_model: true,
        });
        assert_eq!(plan_mode.intent, IntentKind::Advisory);
        assert!(plan_mode.intent.is_read_only());
        assert_eq!(plan_mode.recommended_model_role, ModelRoutingRole::Plan);
        assert_eq!(plan_mode.recommended_effort, ReasoningEffort::High);
        assert!(plan_mode.needs_recon);

        // Mentioned paths or prior errors automatically attach zero-token recon
        let with_path = classify_turn(&TriageInput {
            prompt: "check crates/zenko-core/src/types.rs",
            prior_turn_errors: 0,
            is_first_turn: false,
            permission_mode: PermissionMode::Autonomous,
            has_fast_model: true,
            has_plan_model: false,
        });
        assert!(with_path.needs_recon);
        assert!(with_path.matched_signals.iter().any(|s| s == "paths(1)"));
    }
}
