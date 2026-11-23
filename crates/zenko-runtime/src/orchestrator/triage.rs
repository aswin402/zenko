//! Declarative Phase-0 Intent Gate + Token-Boundary Triage, Ambiguity & Risk Classifier
//! (zero regex, zero `if/else` ladders).
//! Classifies every turn across 4 orthogonal dimensions:
//! 1. `IntentKind` (`Direct` / `Research` / `Advisory` / `Implement`) — prevents accidental edits on read-only asks
//! 2. `ExecutionTier` (`Instant` / `Standard` / `Orchestrated`)
//! 3. `DeliberationDepth` (`Standard` / `Deliberate` with Pre-Mortem for high-risk domains)
//! 4. `ModelRoutingRole` & `ReasoningEffort`

use zenko_core::{
    DeliberationDepth, ExecutionTier, IntentKind, ModelRoutingRole, ORCH_AMBIGUITY_ASK_THRESHOLD,
    ORCH_HIGH_RISK_MIN_SCORE, ORCH_INSTANT_MAX_SCORE, ORCH_ORCHESTRATED_MIN_SCORE, PermissionMode,
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

/// Declarative word-sequence signal rule matched strictly on token boundaries (never mid-word).
struct PhraseRule {
    id: &'static str,
    ngrams: &'static [&'static [&'static str]],
    weight: i32,
}

/// Declarative structural / diagnostic substring markers (compiler errors, stack traces, fences).
struct DiagnosticRule {
    id: &'static str,
    markers: &'static [&'static str],
    weight: i32,
}

const COMPLEXITY_PHRASE_RULES: &[PhraseRule] = &[
    PhraseRule {
        id: "architecture",
        ngrams: &[
            &["architect"],
            &["architecture"],
            &["system", "design"],
            &["from", "scratch"],
            &["redesign"],
            &["refactor"],
            &["migrate"],
            &["migration"],
        ],
        weight: 3,
    },
    PhraseRule {
        id: "multi_component",
        ngrams: &[
            &["end", "to", "end"],
            &["e2e"],
            &["full", "stack"],
            &["fullstack"],
            &["across"],
            &["multi", "file"],
            &["workspace"],
            &["pipeline"],
            &["orchestrator"],
            &["orchestrate"],
            &["subagent"],
            &["daemon"],
        ],
        weight: 2,
    },
    PhraseRule {
        id: "implementation",
        ngrams: &[
            &["implement"],
            &["build"],
            &["create"],
            &["integrate"],
            &["add", "feature"],
            &["database"],
            &["schema"],
            &["authentication"],
            &["security"],
            &["optimize"],
            &["benchmark"],
        ],
        weight: 2,
    },
    PhraseRule {
        id: "debugging",
        ngrams: &[
            &["debug"],
            &["investigate"],
            &["root", "cause"],
            &["fix", "the", "bug"],
            &["race", "condition"],
            &["memory", "leak"],
            &["deadlock"],
            &["performance"],
        ],
        weight: 2,
    },
    PhraseRule {
        id: "file_mutation",
        ngrams: &[
            &["edit"],
            &["write"],
            &["patch"],
            &["update"],
            &["delete"],
            &["rename"],
            &["run"],
        ],
        weight: 2,
    },
    PhraseRule {
        id: "trivial_greeting",
        ngrams: &[
            &["hello"],
            &["hi"],
            &["hey"],
            &["thanks"],
            &["thank", "you"],
            &["what", "is"],
            &["who", "are", "you"],
            &["ping"],
        ],
        weight: -2,
    },
];

const AMBIGUITY_PHRASE_RULES: &[PhraseRule] = &[
    PhraseRule {
        id: "design_comparison",
        ngrams: &[
            &["compare"],
            &["vs"],
            &["versus"],
            &["tradeoff"],
            &["trade", "off"],
            &["tradeoffs"],
            &["pros", "and", "cons"],
            &["options"],
        ],
        weight: 2,
    },
    PhraseRule {
        id: "decision_request",
        ngrams: &[
            &["which", "approach"],
            &["best", "way"],
            &["should", "we"],
            &["should", "i"],
            &["how", "should"],
            &["recommend"],
            &["choose"],
            &["decide"],
        ],
        weight: 2,
    },
    PhraseRule {
        id: "planning_ideation",
        ngrams: &[
            &["brainstorm"],
            &["design"],
            &["proposal"],
            &["plan"],
            &["ideas"],
            &["not", "sure"],
            &["unclear"],
        ],
        weight: 2,
    },
];

/// Phase-0 Intent Gate: explicit read-only / investigation rules vs mutation action verbs.
const READ_ONLY_INTENT_RULES: &[PhraseRule] = &[
    PhraseRule {
        id: "explicit_no_edit",
        ngrams: &[
            &["do", "not", "edit"],
            &["don", "t", "edit"],
            &["dont", "edit"],
            &["do", "not", "modify"],
            &["don", "t", "modify"],
            &["dont", "modify"],
            &["do", "not", "change"],
            &["don", "t", "change"],
            &["dont", "change"],
            &["do", "not", "write"],
            &["don", "t", "write"],
            &["dont", "write"],
            &["do", "not", "touch"],
            &["don", "t", "touch"],
            &["dont", "touch"],
            &["no", "edits"],
            &["without", "writing"],
            &["read", "only"],
            &["readonly"],
            &["without", "editing"],
            &["without", "modifying"],
            &["no", "code", "changes"],
            &["just", "explain"],
            &["only", "analyze"],
            &["only", "inspect"],
        ],
        weight: 5,
    },
    PhraseRule {
        id: "research_analysis",
        ngrams: &[
            &["explain"],
            &["summarize"],
            &["analyze"],
            &["analyse"],
            &["investigate"],
            &["inspect"],
            &["audit"],
            &["review"],
            &["walk", "through"],
            &["how", "does"],
            &["why", "does"],
            &["tell", "me"],
            &["research"],
        ],
        weight: 2,
    },
];

const MUTATION_INTENT_RULES: &[PhraseRule] = &[
    PhraseRule {
        id: "mutate_action",
        ngrams: &[
            &["implement"],
            &["refactor"],
            &["fix"],
            &["build"],
            &["create"],
            &["add"],
            &["edit"],
            &["write"],
            &["patch"],
            &["update"],
            &["modify"],
            &["change"],
            &["delete"],
            &["remove"],
            &["migrate"],
            &["apply"],
            &["install"],
            &["wire"],
            &["upgrade"],
            &["rename"],
            &["run"],
            &["start"],
            &["launch"],
            &["execute"],
            &["spawn"],
            &["stop"],
            &["kill"],
            &["restart"],
            &["deploy"],
            &["configure"],
            &["generate"],
            &["scaffold"],
            &["make"],
            &["commit"],
            &["push"],
            &["test"],
            &["clean"],
            &["format"],
            &["setup"],
            &["init"],
            &["initialize"],
            &["enable"],
            &["disable"],
            &["schedule"],
            &["trigger"],
        ],
        weight: 3,
    },
];

/// High-risk domains automatically triggering `DeliberationDepth::Deliberate` (Pre-Mortem + strict rubric).
const HIGH_RISK_DOMAIN_RULES: &[PhraseRule] = &[
    PhraseRule {
        id: "risk_security_auth",
        ngrams: &[
            &["auth"],
            &["authentication"],
            &["authorization"],
            &["security"],
            &["vault"],
            &["secret"],
            &["secrets"],
            &["encryption"],
            &["credential"],
            &["credentials"],
            &["oauth"],
            &["jwt"],
            &["rbac"],
            &["permission"],
            &["sandbox"],
        ],
        weight: 3,
    },
    PhraseRule {
        id: "risk_data_prod",
        ngrams: &[
            &["migration"],
            &["migrate"],
            &["schema"],
            &["sqlite"],
            &["database"],
            &["production"],
            &["deploy"],
            &["breaking"],
            &["drop"],
            &["wipe"],
            &["delete"],
        ],
        weight: 2,
    },
];

const DIAGNOSTIC_RULES: &[DiagnosticRule] = &[
    DiagnosticRule {
        id: "diagnostic_trace",
        markers: &[
            "error[e",
            "traceback (most recent call",
            "panicked at",
            "stack backtrace",
            "segmentation fault",
            "syntaxerror:",
            "typeerror:",
            "thread '",
        ],
        weight: 2,
    },
    DiagnosticRule {
        id: "code_fence",
        markers: &["```"],
        weight: 1,
    },
];

const FILE_EXT_SUFFIXES: &[&str] = &[
    ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".toml", ".json", ".yaml", ".yml", ".sql",
    ".md", ".sh",
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

fn contains_ngram(tokens: &[String], ngram: &[&str]) -> bool {
    match ngram.is_empty() || tokens.len() < ngram.len() {
        true => false,
        false => tokens
            .windows(ngram.len())
            .any(|win| win.iter().zip(ngram.iter()).all(|(a, b)| a.as_str() == *b)),
    }
}

fn evaluate_phrase_rules(
    tokens: &[String],
    rules: &[PhraseRule],
    matched: &mut Vec<String>,
) -> i32 {
    let mut score = 0i32;
    for rule in rules {
        if rule.ngrams.iter().any(|ng| contains_ngram(tokens, ng)) {
            score += rule.weight;
            matched.push(format!("{}:{:+}", rule.id, rule.weight));
        }
    }
    score
}

fn evaluate_diagnostic_rules(
    lower: &str,
    rules: &[DiagnosticRule],
    matched: &mut Vec<String>,
) -> i32 {
    let mut score = 0i32;
    for rule in rules {
        if rule.markers.iter().any(|m| lower.contains(m)) {
            score += rule.weight;
            matched.push(format!("{}:{:+}", rule.id, rule.weight));
        }
    }
    score
}

/// Extracts path-like tokens mentioned in the prompt (reused by both triage and zero-token recon).
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

fn count_list_items(prompt: &str) -> usize {
    prompt
        .lines()
        .filter(|line| {
            let t = line.trim_start();
            let is_bullet = t.starts_with("- ") || t.starts_with("* ");
            let is_numbered = t
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_digit)
                && t.chars()
                    .find(|c| !c.is_ascii_digit())
                    .is_some_and(|c| matches!(c, '.' | ')'));
            is_bullet || is_numbered
        })
        .count()
}

/// Deterministically classifies an incoming user prompt into `IntentKind`, `ExecutionTier`,
/// `DeliberationDepth`, `ModelRoutingRole`, and `ReasoningEffort`.
pub fn classify_turn(input: &TriageInput<'_>) -> TriageReport {
    let trimmed = input.prompt.trim();
    let lower = trimmed.to_ascii_lowercase();
    let tokens = tokenize_words(trimmed);
    let word_count = tokens.len();
    let line_count = trimmed.lines().filter(|l| !l.trim().is_empty()).count();
    let path_count = extract_path_like_tokens(trimmed).len();
    let list_items = count_list_items(trimmed);
    let question_marks = trimmed.chars().filter(|&c| c == '?').count();

    let mut matched_signals: Vec<String> = Vec::with_capacity(12);
    let mut complexity_score =
        evaluate_phrase_rules(&tokens, COMPLEXITY_PHRASE_RULES, &mut matched_signals);
    complexity_score += evaluate_diagnostic_rules(&lower, DIAGNOSTIC_RULES, &mut matched_signals);
    let mut ambiguity_score =
        evaluate_phrase_rules(&tokens, AMBIGUITY_PHRASE_RULES, &mut matched_signals);
    let read_only_score =
        evaluate_phrase_rules(&tokens, READ_ONLY_INTENT_RULES, &mut matched_signals);
    let mutation_score =
        evaluate_phrase_rules(&tokens, MUTATION_INTENT_RULES, &mut matched_signals);
    let risk_score = evaluate_phrase_rules(&tokens, HIGH_RISK_DOMAIN_RULES, &mut matched_signals);

    // Structural modifiers via declarative match ranges (zero if/else chains)
    let length_bonus = match word_count {
        0..=5 => -1,
        6..=24 => 0,
        25..=60 => 1,
        _ => 2,
    };
    complexity_score += length_bonus;
    if length_bonus != 0 {
        matched_signals.push(format!("words({word_count}):{length_bonus:+}"));
    }

    if line_count >= 4 {
        complexity_score += 1;
        matched_signals.push(format!("multiline({line_count}):+1"));
    }

    if list_items >= 3 {
        complexity_score += 2;
        matched_signals.push(format!("checklist({list_items}):+2"));
    }

    if path_count >= 2 {
        complexity_score += 2;
        matched_signals.push(format!("multi_path({path_count}):+2"));
    }

    if input.is_first_turn && (path_count >= 1 || complexity_score >= 3) {
        complexity_score += 1;
        matched_signals.push("first_turn_grounding:+1".into());
    }

    if input.prior_turn_errors > 0 {
        let err_bonus = (input.prior_turn_errors as i32).min(2);
        complexity_score += err_bonus;
        matched_signals.push(format!("prior_errors({}):+{err_bonus}", input.prior_turn_errors));
    }

    if question_marks >= 2 {
        ambiguity_score += 1;
        matched_signals.push(format!("multi_question({question_marks}):+1"));
    }

    match input.permission_mode {
        PermissionMode::Plan => {
            matched_signals.push("plan_mode:read_only".into());
        }
        PermissionMode::Strict if complexity_score >= 3 => {
            complexity_score += 1;
            matched_signals.push("strict_mode:+1".into());
        }
        _ => {}
    }

    let tier = match (
        input.permission_mode == PermissionMode::Plan && word_count > 3,
        complexity_score >= ORCH_ORCHESTRATED_MIN_SCORE
            || ambiguity_score >= ORCH_AMBIGUITY_ASK_THRESHOLD,
        complexity_score <= ORCH_INSTANT_MAX_SCORE && ambiguity_score == 0 && word_count <= 14,
    ) {
        (true, _, _) | (_, true, _) => ExecutionTier::Orchestrated,
        (false, false, true) => ExecutionTier::Instant,
        (false, false, false) => ExecutionTier::Standard,
    };

    // Phase-0 Intent Gate (distinguishes Plan Mode / explicit no-edit / read-only research / advisory vs implement)
    let intent = match (
        input.permission_mode == PermissionMode::Plan,
        read_only_score >= 5,
        ambiguity_score >= ORCH_AMBIGUITY_ASK_THRESHOLD,
        read_only_score >= 2,
        mutation_score == 0,
        tier == ExecutionTier::Instant,
    ) {
        (true, _, _, _, _, _) => IntentKind::Advisory,
        (false, true, true, _, _, _) => IntentKind::Advisory,
        (false, true, false, _, _, _) => IntentKind::Research,
        (false, false, true, _, true, _) => IntentKind::Advisory,
        (false, false, false, true, true, _) => IntentKind::Research,
        (false, false, false, false, true, true) => IntentKind::Direct,
        _ => IntentKind::Implement,
    };

    let deliberation = match (
        tier,
        risk_score >= ORCH_HIGH_RISK_MIN_SCORE,
        intent.is_read_only(),
    ) {
        (ExecutionTier::Orchestrated, true, false) => DeliberationDepth::Deliberate,
        _ => DeliberationDepth::Standard,
    };

    let recommended_model_role = match (
        input.permission_mode == PermissionMode::Plan,
        tier,
        input.has_fast_model,
        input.has_plan_model,
    ) {
        (true, _, _, true) => ModelRoutingRole::Plan,
        (false, ExecutionTier::Instant, true, _) => ModelRoutingRole::Fast,
        (false, ExecutionTier::Orchestrated, _, true) => ModelRoutingRole::Plan,
        _ => ModelRoutingRole::Lead,
    };
    let recommended_effort = ReasoningEffort::from_tier(tier);

    let needs_recon = tier == ExecutionTier::Orchestrated;
    let needs_brainstorm =
        tier == ExecutionTier::Orchestrated && intent != IntentKind::Research;

    let reason = match (tier, intent, deliberation) {
        (ExecutionTier::Instant, _, _) => format!(
            "Concise low-risk turn (intent={}, complexity={complexity_score}, effort={}) -> fast direct response.",
            intent.as_str(),
            recommended_effort.as_str()
        ),
        (ExecutionTier::Standard, _, _) => format!(
            "Scoped turn (intent={}, complexity={complexity_score}, effort={}) -> standard ReAct loop.",
            intent.as_str(),
            recommended_effort.as_str()
        ),
        (ExecutionTier::Orchestrated, IntentKind::Research, _) => format!(
            "Deep read-only investigation (intent=research, complexity={complexity_score}, effort={}) -> 5-Way Recon + Read-Only Intent Gate.",
            recommended_effort.as_str()
        ),
        (ExecutionTier::Orchestrated, IntentKind::Advisory, _) => format!(
            "Open-ended design comparison (intent=advisory, ambiguity={ambiguity_score}, effort={}) -> Recon + Structured Brainstorm (read-only).",
            recommended_effort.as_str()
        ),
        (ExecutionTier::Orchestrated, _, DeliberationDepth::Deliberate) => format!(
            "High-risk orchestrated mutation (risk={risk_score}, complexity={complexity_score}, depth=deliberate, effort={}) -> Recon + Pre-Mortem Brainstorm + DAG Plan + Verification Gate.",
            recommended_effort.as_str()
        ),
        (ExecutionTier::Orchestrated, _, DeliberationDepth::Standard) => format!(
            "Orchestrated mutation (complexity={complexity_score}, ambiguity={ambiguity_score}, effort={}) -> Recon + Phase-Gated Brainstorm/Plan + Verification Gate.",
            recommended_effort.as_str()
        ),
    };

    TriageReport {
        tier,
        intent,
        deliberation,
        complexity_score,
        ambiguity_score,
        risk_score,
        matched_signals,
        recommended_model_role,
        recommended_effort,
        needs_recon,
        needs_brainstorm,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_intent_tier_and_deliberation_depth_deterministically() {
        let instant = classify_turn(&TriageInput {
            prompt: "hi!",
            prior_turn_errors: 0,
            is_first_turn: true,
            permission_mode: PermissionMode::Balanced,
            has_fast_model: true,
            has_plan_model: false,
        });
        assert_eq!(instant.tier, ExecutionTier::Instant);
        assert_eq!(instant.intent, IntentKind::Direct);
        assert_eq!(instant.recommended_model_role, ModelRoutingRole::Fast);
        assert_eq!(instant.recommended_effort, ReasoningEffort::Low);

        // Read-only investigation with explicit "don't edit" MUST stay Research even with high complexity!
        let research = classify_turn(&TriageInput {
            prompt: "Investigate the authentication architecture across crates/zenko-runtime/src/runtime.rs and crates/zenko-webui/src/api.rs and explain why sessions deadlock — don't edit anything",
            prior_turn_errors: 0,
            is_first_turn: true,
            permission_mode: PermissionMode::Balanced,
            has_fast_model: true,
            has_plan_model: true,
        });
        assert_eq!(research.tier, ExecutionTier::Orchestrated);
        assert_eq!(research.intent, IntentKind::Research);
        assert!(research.intent.is_read_only());
        assert!(research.needs_recon);
        assert!(!research.needs_brainstorm);

        // High-risk auth + migration implementation triggers Deliberate depth!
        let deliberate = classify_turn(&TriageInput {
            prompt: "Design and refactor the authentication architecture across crates/zenko-runtime/src/runtime.rs and crates/zenko-webui/src/api.rs with database migration and security checks",
            prior_turn_errors: 0,
            is_first_turn: true,
            permission_mode: PermissionMode::Balanced,
            has_fast_model: true,
            has_plan_model: true,
        });
        assert_eq!(deliberate.tier, ExecutionTier::Orchestrated);
        assert_eq!(deliberate.intent, IntentKind::Implement);
        assert_eq!(deliberate.deliberation, DeliberationDepth::Deliberate);
        assert!(deliberate.risk_score >= ORCH_HIGH_RISK_MIN_SCORE);

        // Mixed inspect + execute verb ("inspect repo and start bg worker") MUST be Implement, NOT Research!
        let mixed_exec = classify_turn(&TriageInput {
            prompt: "inspect repo and start interactive bg worker",
            prior_turn_errors: 0,
            is_first_turn: true,
            permission_mode: PermissionMode::Autonomous,
            has_fast_model: true,
            has_plan_model: false,
        });
        assert_eq!(mixed_exec.intent, IntentKind::Implement);
        assert!(!mixed_exec.intent.is_read_only());

        // Open-ended comparison with explicit "Don't write code yet" -> Advisory (read-only brainstorm, NOT Implement!)
        let advisory_no_write = classify_turn(&TriageInput {
            prompt: "Brainstorm and compare options for adding per-IP rate limiting to crates/zenko-webui/src/api.rs: in-memory vs SQLite vs tower. Which approach should we choose? Don't write code yet.",
            prior_turn_errors: 0,
            is_first_turn: true,
            permission_mode: PermissionMode::Balanced,
            has_fast_model: true,
            has_plan_model: false,
        });
        assert_eq!(advisory_no_write.tier, ExecutionTier::Orchestrated);
        assert_eq!(advisory_no_write.intent, IntentKind::Advisory);
        assert!(advisory_no_write.intent.is_read_only());
        assert!(advisory_no_write.needs_brainstorm);

        // Open-ended comparison without mutation verb -> Advisory (read-only brainstorm)
        let advisory = classify_turn(&TriageInput {
            prompt: "Brainstorm & compare options: which approach should we choose for caching vs SQLite?",
            prior_turn_errors: 0,
            is_first_turn: false,
            permission_mode: PermissionMode::Balanced,
            has_fast_model: true,
            has_plan_model: false,
        });
        assert_eq!(advisory.tier, ExecutionTier::Orchestrated);
        assert_eq!(advisory.intent, IntentKind::Advisory);
        assert!(advisory.intent.is_read_only());
        assert!(advisory.needs_brainstorm);
    }
}
