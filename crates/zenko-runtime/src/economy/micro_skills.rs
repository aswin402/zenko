//! Zero-LLM deterministic shell fail->fix micro-skill miner, hit tracker, and contradiction detector.
//! Pure byte/token state machine — zero regex, zero hardcoding, zero `.unwrap()`.

use std::collections::BTreeSet;
use zenko_core::{
    ECONOMY_CMD_PREVIEW_CHARS, ECONOMY_CONTRADICTION_MARKERS, ECONOMY_LEARNABLE_FAIL_MARKERS,
    ECONOMY_SHELL_WRAPPERS, FNV1A_OFFSET_BASIS, FNV1A_PRIME, MicroSkillStatus, RepoMicroSkill,
};

#[derive(Debug, Clone)]
pub struct ShellTraceEntry {
    pub command: String,
    pub is_error: bool,
    pub output_preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinedMicroSkill {
    pub signature: String,
    pub fact: String,
    pub failed_cmd: String,
    pub fixed_cmd: String,
}

/// Deterministic 64-bit FNV-1a hex signature for a micro-skill key.
pub fn micro_skill_signature(key: &str) -> String {
    let mut h = FNV1A_OFFSET_BASIS;
    for b in key.bytes() {
        h ^= u64::from(b.to_ascii_lowercase());
        h = h.wrapping_mul(FNV1A_PRIME);
    }
    format!("ms_{h:016x}")
}

fn is_env_assignment_token(tok: &str) -> bool {
    let Some((lhs, _)) = tok.split_once('=') else {
        return false;
    };
    !lhs.is_empty()
        && lhs
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Splits the first shell command segment (stopping before `&&`, `||`, `;`, `|`, `\n`) and
/// extracts a canonical executable + subcommand head (e.g., `pytest`, `python -m pytest`,
/// `cargo test`, `just verify`, `npm run build`) with zero regex.
pub fn extract_command_head(raw_cmd: &str) -> Option<String> {
    let first_line = raw_cmd.lines().next()?.trim();
    if first_line.is_empty() {
        return None;
    }
    // Truncate at first shell chain operator outside quotes
    let mut cut = first_line.len();
    let bytes = first_line.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut idx = 0usize;
    while idx < bytes.len() {
        match bytes[idx] {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b';' | b'|' if !in_single && !in_double => {
                cut = idx;
                break;
            }
            b'&' if !in_single && !in_double && idx + 1 < bytes.len() && bytes[idx + 1] == b'&' => {
                cut = idx;
                break;
            }
            _ => {}
        }
        idx += 1;
    }

    let segment = first_line[..cut].trim();
    let mut tokens: Vec<&str> = segment.split_whitespace().collect();
    // Skip leading env assignments (`RUST_LOG=debug ...`) and wrapper binaries (`env`, `time`, ...)
    while let Some(&first) = tokens.first() {
        let base = first.rsplit('/').next().unwrap_or(first);
        if is_env_assignment_token(first) || ECONOMY_SHELL_WRAPPERS.contains(&base) {
            tokens.remove(0);
            // Skip numeric duration arg following `timeout`
            if base == "timeout"
                && tokens
                    .first()
                    .is_some_and(|t| t.bytes().all(|b| b.is_ascii_digit() || matches!(b, b's' | b'm')))
            {
                tokens.remove(0);
            }
        } else {
            break;
        }
    }

    let bin_raw = *tokens.first()?;
    let bin = bin_raw
        .trim_matches(|c: char| matches!(c, '"' | '\''))
        .rsplit('/')
        .next()
        .unwrap_or(bin_raw);
    if bin.is_empty() {
        return None;
    }

    // Preserve meaningful 2-3 token invocations (`python -m pytest`, `npm run build`, `cargo test`, `just verify`)
    let head = match (tokens.get(1).copied(), tokens.get(2).copied()) {
        (Some("-m"), Some(module)) if !module.starts_with('-') => {
            format!("{bin} -m {module}")
        }
        (Some("run"), Some(script)) if !script.starts_with('-') => {
            format!("{bin} run {script}")
        }
        (Some(sub), _)
            if !sub.starts_with('-')
                && !sub.contains('/')
                && !sub.contains('.')
                && sub.len() <= 24 =>
        {
            format!("{bin} {sub}")
        }
        _ => bin.to_string(),
    };

    let clean: String = head.chars().take(ECONOMY_CMD_PREVIEW_CHARS).collect();
    match clean.is_empty() {
        true => None,
        false => Some(clean),
    }
}

fn contains_any_marker(text: &str, markers: &[&str]) -> bool {
    let lower = text.to_ascii_lowercase();
    markers.iter().any(|m| lower.contains(m))
}

/// Deterministically mines 1-line verified fail->fix repository facts from a completed turn's
/// `shell_exec` history with zero LLM calls and zero regex.
pub fn mine_turn_shell_micro_skills(traces: &[ShellTraceEntry]) -> Vec<MinedMicroSkill> {
    if traces.len() < 2 {
        return Vec::new();
    }
    let mut mined = Vec::new();
    let mut seen_sigs: BTreeSet<String> = BTreeSet::new();

    for (i, fail) in traces.iter().enumerate() {
        if !fail.is_error || !contains_any_marker(&fail.output_preview, ECONOMY_LEARNABLE_FAIL_MARKERS)
        {
            continue;
        }
        let Some(failed_head) = extract_command_head(&fail.command) else {
            continue;
        };

        // Look ahead for the first succeeding command with a distinct command head within the next 4 shell calls
        let lookahead_end = (i + 5).min(traces.len());
        let Some(fixed_head) = traces[i + 1..lookahead_end].iter().find_map(|succ| {
            if succ.is_error {
                return None;
            }
            let head = extract_command_head(&succ.command)?;
            match head.eq_ignore_ascii_case(&failed_head) {
                true => None,
                false => Some(head),
            }
        }) else {
            continue;
        };

        let sig = micro_skill_signature(&format!("{failed_head}=>{fixed_head}"));
        if !seen_sigs.insert(sig.clone()) {
            continue;
        }
        let fact = format!(
            "In this workspace, `{failed_head}` failed; use `{fixed_head}` instead."
        );
        mined.push(MinedMicroSkill {
            signature: sig,
            fact,
            failed_cmd: failed_head,
            fixed_cmd: fixed_head,
        });
    }
    mined
}

/// Detects active micro-skills whose recommended `fixed_cmd` contradicted reality in this turn
/// (failed with command-not-found / bad-flag / missing-subcommand -> triggers 1-strike retirement).
pub fn detect_contradicted_micro_skills(
    active_skills: &[RepoMicroSkill],
    traces: &[ShellTraceEntry],
) -> Vec<String> {
    let mut contradicted = Vec::new();
    for skill in active_skills {
        if skill.status != MicroSkillStatus::Active || skill.fixed_cmd.trim().is_empty() {
            continue;
        }
        let failed_match = traces.iter().any(|t| {
            t.is_error
                && contains_any_marker(&t.output_preview, ECONOMY_CONTRADICTION_MARKERS)
                && extract_command_head(&t.command)
                    .is_some_and(|head| head.eq_ignore_ascii_case(skill.fixed_cmd.trim()))
        });
        if failed_match {
            contradicted.push(skill.id.clone());
        }
    }
    contradicted
}

/// Detects which injected active micro-skills had their recommended command used successfully
/// during the turn so SQLite can maintain accurate need-rate statistics.
pub fn detect_hit_micro_skills(
    active_skills: &[RepoMicroSkill],
    traces: &[ShellTraceEntry],
) -> BTreeSet<String> {
    let mut hits = BTreeSet::new();
    for skill in active_skills {
        if skill.status != MicroSkillStatus::Active || skill.fixed_cmd.trim().is_empty() {
            continue;
        }
        let matched = traces.iter().any(|t| {
            !t.is_error
                && extract_command_head(&t.command)
                    .is_some_and(|head| head.eq_ignore_ascii_case(skill.fixed_cmd.trim()))
        });
        if matched {
            hits.insert(skill.id.clone());
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn extracts_command_heads_deterministically_skipping_wrappers_and_env() {
        assert_eq!(
            extract_command_head("RUST_LOG=info env cargo test --workspace && echo ok").as_deref(),
            Some("cargo test")
        );
        assert_eq!(
            extract_command_head("python3 -m pytest tests/test_api.py -q").as_deref(),
            Some("python3 -m pytest")
        );
        assert_eq!(
            extract_command_head("npm run build -- --watch").as_deref(),
            Some("npm run build")
        );
        assert_eq!(
            extract_command_head("timeout 30s /usr/bin/just verify").as_deref(),
            Some("just verify")
        );
    }

    #[test]
    fn mines_fail_to_fix_micro_skill_and_detects_contradiction_without_regex() {
        let traces = vec![
            ShellTraceEntry {
                command: "pytest tests/".into(),
                is_error: true,
                output_preview: "exit_code: 127\n/bin/sh: 1: pytest: command not found".into(),
            },
            ShellTraceEntry {
                command: "python3 -m pytest tests/".into(),
                is_error: false,
                output_preview: "exit_code: 0\n12 passed in 0.4s".into(),
            },
        ];
        let mined = mine_turn_shell_micro_skills(&traces);
        assert_eq!(mined.len(), 1);
        assert_eq!(mined[0].failed_cmd, "pytest");
        assert_eq!(mined[0].fixed_cmd, "python3 -m pytest");
        assert!(mined[0].fact.contains("python3 -m pytest"));

        let now = Utc::now();
        let active = vec![RepoMicroSkill {
            id: "ms_1".into(),
            repo_scope: "project:demo".into(),
            signature: mined[0].signature.clone(),
            fact: mined[0].fact.clone(),
            failed_cmd: mined[0].failed_cmd.clone(),
            fixed_cmd: mined[0].fixed_cmd.clone(),
            corroborations: 2,
            session_ids: vec!["s1".into(), "s2".into()],
            need_hits: 1,
            exposures: 2,
            contradictions: 0,
            status: MicroSkillStatus::Active,
            created_at: now,
            updated_at: now,
        }];
        let hits = detect_hit_micro_skills(&active, &traces);
        assert!(hits.contains("ms_1"));

        let broken_traces = vec![ShellTraceEntry {
            command: "python3 -m pytest tests/".into(),
            is_error: true,
            output_preview: "exit_code: 127\n/bin/sh: python3: command not found".into(),
        }];
        let contradicted = detect_contradicted_micro_skills(&active, &broken_traces);
        assert_eq!(contradicted, vec!["ms_1".to_string()]);
    }
}
