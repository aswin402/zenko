//! Zero-regex, SHA-256 trust-gated declarative TOML filter DSL (`schema_version = 1`).
//! Supports user-global `~/.zenko/filters.toml` (trusted by default) and project-local
//! `<workspace>/.zenko/filters.toml` (strictly trust-gated by SHA-256 content hash in SQLite so
//! untrusted repositories cannot hide errors or spoof tool output).

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use zenko_core::{
    CUSTOM_FILTERS_FILE_NAME, CUSTOM_FILTERS_MAX_FILE_BYTES, CUSTOM_FILTERS_MAX_ON_EMPTY_CHARS,
    CUSTOM_FILTERS_MAX_RULES, CUSTOM_FILTERS_PROJECT_DIR, CUSTOM_FILTERS_SCHEMA_VERSION,
    CustomFilterTestFailure, CustomFilterTrustStatus, CustomFilterVerifyReport,
    READ_LESS_DEDUP_MIN_REPEATS, READ_LESS_FAIL_PREFIXES, READ_LESS_FAIL_SUBSTRS,
};
use zenko_memory::SqliteStore;
use zenko_tools::pty::strip_ansi_escapes;

#[derive(Debug, Clone, Deserialize)]
pub struct FilterFileV1 {
    pub schema_version: u32,
    #[serde(default)]
    pub filters: BTreeMap<String, DeclarativeFilterRule>,
    #[serde(default)]
    pub tests: BTreeMap<String, Vec<FilterTestCase>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeclarativeFilterRule {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub match_command_prefixes: Vec<String>,
    #[serde(default = "default_true")]
    pub strip_ansi: bool,
    #[serde(default)]
    pub strip_prefixes: Vec<String>,
    #[serde(default)]
    pub strip_contains: Vec<String>,
    #[serde(default)]
    pub keep_prefixes: Vec<String>,
    #[serde(default)]
    pub keep_contains: Vec<String>,
    #[serde(default = "default_true")]
    pub collapse_repeats: bool,
    #[serde(default)]
    pub max_lines: Option<usize>,
    #[serde(default)]
    pub max_line_chars: Option<usize>,
    #[serde(default)]
    pub on_empty: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FilterTestCase {
    pub name: String,
    pub input: String,
    pub expected: String,
}

fn default_true() -> bool {
    true
}

pub fn user_filter_path(data_dir: &Path) -> PathBuf {
    data_dir.join(CUSTOM_FILTERS_FILE_NAME)
}

pub fn project_filter_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir
        .join(CUSTOM_FILTERS_PROJECT_DIR)
        .join(CUSTOM_FILTERS_FILE_NAME)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn read_bounded_bytes(path: &Path) -> std::result::Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("failed to stat filter file: {e}"))?;
    if meta.len() as usize > CUSTOM_FILTERS_MAX_FILE_BYTES {
        return Err(format!(
            "filter file exceeds {} byte safety cap",
            CUSTOM_FILTERS_MAX_FILE_BYTES
        ));
    }
    std::fs::read(path).map_err(|e| format!("failed to read filter file: {e}"))
}

fn parse_filter_file(bytes: &[u8]) -> std::result::Result<FilterFileV1, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "filter file is not valid UTF-8".to_string())?;
    let parsed: FilterFileV1 =
        toml::from_str(text).map_err(|e| format!("invalid TOML syntax: {e}"))?;
    if parsed.schema_version != CUSTOM_FILTERS_SCHEMA_VERSION {
        return Err(format!(
            "unsupported schema_version {} (expected {})",
            parsed.schema_version, CUSTOM_FILTERS_SCHEMA_VERSION
        ));
    }
    if parsed.filters.len() > CUSTOM_FILTERS_MAX_RULES {
        return Err(format!(
            "too many rules: {} (max {})",
            parsed.filters.len(),
            CUSTOM_FILTERS_MAX_RULES
        ));
    }
    for (name, rule) in &parsed.filters {
        if rule
            .match_command_prefixes
            .iter()
            .all(|p| p.trim().is_empty())
        {
            return Err(format!(
                "filter `{name}` must define at least one non-empty `match_command_prefixes` entry"
            ));
        }
    }
    Ok(parsed)
}

fn is_protected_error_line(trimmed: &str) -> bool {
    if (trimmed.starts_with("test ") && trimmed.contains("FAILED"))
        || READ_LESS_FAIL_PREFIXES.iter().any(|p| trimmed.starts_with(p))
    {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    READ_LESS_FAIL_SUBSTRS.iter().any(|s| lower.contains(s))
}

fn rule_matches_command(rule: &DeclarativeFilterRule, command: &str) -> bool {
    let cmd_lower = command.trim().to_ascii_lowercase();
    rule.match_command_prefixes.iter().any(|raw_pref| {
        let pref = raw_pref.trim().to_ascii_lowercase();
        if pref.is_empty() {
            return false;
        }
        cmd_lower == pref
            || cmd_lower
                .strip_prefix(&pref)
                .is_some_and(|rest| rest.starts_with(' ') || rest.starts_with('\t'))
    })
}

/// Deterministically executes a single declarative filter rule (zero regex, never drops errors).
pub fn apply_rule(rule: &DeclarativeFilterRule, raw: &str, is_error: bool) -> String {
    let cleaned_cow;
    let src = match rule.strip_ansi {
        true => {
            cleaned_cow = strip_ansi_escapes(raw);
            cleaned_cow.as_str()
        }
        false => raw,
    };

    let has_keep_whitelist = !rule.keep_prefixes.is_empty() || !rule.keep_contains.is_empty();
    let mut kept: Vec<String> = Vec::new();

    for (idx, raw_line) in src.lines().enumerate() {
        let trimmed_end = raw_line.trim_end();
        let trimmed = trimmed_end.trim();

        if idx == 0 && trimmed.starts_with("exit_code:") {
            kept.push(trimmed_end.to_string());
            continue;
        }

        let protected = is_error && is_protected_error_line(trimmed);
        if !protected {
            if rule
                .strip_prefixes
                .iter()
                .any(|p| !p.is_empty() && trimmed.starts_with(p.as_str()))
                || rule
                    .strip_contains
                    .iter()
                    .any(|s| !s.is_empty() && trimmed_end.contains(s.as_str()))
            {
                continue;
            }
            if has_keep_whitelist {
                let matched_keep = rule
                    .keep_prefixes
                    .iter()
                    .any(|p| !p.is_empty() && trimmed.starts_with(p.as_str()))
                    || rule
                        .keep_contains
                        .iter()
                        .any(|s| !s.is_empty() && trimmed_end.contains(s.as_str()));
                if !matched_keep {
                    continue;
                }
            }
        }

        let line_out = match rule.max_line_chars {
            Some(cap) if cap >= 16 && trimmed_end.chars().count() > cap => {
                let cut: String = trimmed_end.chars().take(cap.saturating_sub(3)).collect();
                format!("{cut}...")
            }
            _ => trimmed_end.to_string(),
        };
        kept.push(line_out);
    }

    let collapsed = match rule.collapse_repeats {
        true => collapse_consecutive_lines(kept),
        false => kept,
    };

    let mut final_lines = match rule.max_lines {
        Some(cap) if cap > 0 && collapsed.len() > cap => {
            let omitted = collapsed.len().saturating_sub(cap);
            let mut slice = collapsed[..cap].to_vec();
            slice.push(format!("[+{omitted} lines truncated by custom filter]"));
            slice
        }
        _ => collapsed,
    };

    while final_lines.last().is_some_and(|l| l.trim().is_empty()) {
        final_lines.pop();
    }

    if final_lines.is_empty()
        && !is_error
        && let Some(ref on_empty) = rule.on_empty
    {
        let safe: String = on_empty
            .chars()
            .filter(|c| !c.is_control())
            .take(CUSTOM_FILTERS_MAX_ON_EMPTY_CHARS)
            .collect();
        if !safe.trim().is_empty() {
            return safe.trim().to_string();
        }
    }

    final_lines.join("\n")
}

fn collapse_consecutive_lines(lines: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    let mut i = 0usize;
    while i < lines.len() {
        let cur = &lines[i];
        if cur.trim().is_empty() {
            if out.last().is_some_and(|p: &String| !p.is_empty()) {
                out.push(String::new());
            }
            i += 1;
            continue;
        }
        let mut run = 1usize;
        while i + run < lines.len() && lines[i + run] == *cur {
            run += 1;
        }
        match run >= READ_LESS_DEDUP_MIN_REPEATS {
            true => out.push(format!("{cur} [x{run}]")),
            false => {
                for _ in 0..run {
                    out.push(cur.clone());
                }
            }
        }
        i += run;
    }
    out
}

/// Computes SHA-256 trust state for `<workspace>/.zenko/filters.toml` and rule counts.
pub fn inspect_filter_trust_status(
    store: &SqliteStore,
    repo_scope: &str,
    data_dir: &Path,
    workspace_dir: &Path,
) -> CustomFilterTrustStatus {
    let proj_path = project_filter_path(workspace_dir);
    let canonical_proj = std::fs::canonicalize(&proj_path).unwrap_or_else(|_| proj_path.clone());
    let proj_str = canonical_proj.display().to_string();

    let user_rule_count = read_bounded_bytes(&user_filter_path(data_dir))
        .ok()
        .and_then(|b| parse_filter_file(&b).ok())
        .map(|f| f.filters.len())
        .unwrap_or(0);

    let trusted_sha256 = store
        .get_trusted_filter_hash(repo_scope, &proj_str)
        .ok()
        .flatten();

    let Ok(bytes) = read_bounded_bytes(&proj_path) else {
        return CustomFilterTrustStatus {
            project_file_path: proj_str,
            project_file_exists: false,
            project_sha256: None,
            trusted: false,
            trusted_sha256,
            rule_count: 0,
            user_rule_count,
        };
    };

    let hash = sha256_hex(&bytes);
    let rule_count = parse_filter_file(&bytes)
        .map(|f| f.filters.len())
        .unwrap_or(0);
    let trusted = trusted_sha256.as_deref() == Some(hash.as_str());

    CustomFilterTrustStatus {
        project_file_path: proj_str,
        project_file_exists: true,
        project_sha256: Some(hash),
        trusted,
        trusted_sha256,
        rule_count,
        user_rule_count,
    }
}

/// Validates a `filters.toml` file and runs all inline `[[tests.<filter_name>]]` assertions.
pub fn verify_filter_file_at(path: &Path) -> CustomFilterVerifyReport {
    let file_path = path.display().to_string();
    if !path.exists() {
        return CustomFilterVerifyReport {
            file_path,
            exists: false,
            valid: true,
            ..CustomFilterVerifyReport::default()
        };
    }

    let bytes = match read_bounded_bytes(path) {
        Ok(b) => b,
        Err(err) => {
            return CustomFilterVerifyReport {
                file_path,
                exists: true,
                valid: false,
                errors: vec![err],
                ..CustomFilterVerifyReport::default()
            };
        }
    };

    let parsed = match parse_filter_file(&bytes) {
        Ok(f) => f,
        Err(err) => {
            return CustomFilterVerifyReport {
                file_path,
                exists: true,
                valid: false,
                errors: vec![err],
                ..CustomFilterVerifyReport::default()
            };
        }
    };

    let mut tests_passed = 0usize;
    let mut tests_failed = 0usize;
    let mut errors = Vec::new();
    let mut test_failures = Vec::new();

    for (filter_name, cases) in &parsed.tests {
        let Some(rule) = parsed.filters.get(filter_name) else {
            errors.push(format!(
                "test section `tests.{filter_name}` references unknown filter `{filter_name}`"
            ));
            continue;
        };
        for case in cases {
            let actual = apply_rule(rule, &case.input, false);
            let expected = case.expected.trim_end_matches('\n');
            if actual == expected {
                tests_passed += 1;
            } else {
                tests_failed += 1;
                test_failures.push(CustomFilterTestFailure {
                    filter_name: filter_name.clone(),
                    test_name: case.name.clone(),
                    expected: expected.to_string(),
                    actual,
                });
            }
        }
    }

    CustomFilterVerifyReport {
        file_path,
        exists: true,
        valid: errors.is_empty() && tests_failed == 0,
        rule_count: parsed.filters.len(),
        tests_passed,
        tests_failed,
        errors,
        test_failures,
    }
}

/// Applies the first matching trusted custom filter (project-local if SHA-256 trusted, else user-global).
/// Strictly enforces "Never Worse" (`shaped.len() < raw.len()`).
#[allow(clippy::too_many_arguments)]
pub fn try_apply_trusted_custom_filters(
    store: &SqliteStore,
    repo_scope: &str,
    data_dir: &Path,
    workspace_dir: &Path,
    command: &str,
    raw: &str,
    is_error: bool,
) -> Option<(String, String)> {
    // 1. Project-local `.zenko/filters.toml` ONLY when SHA-256 matches SQLite trust entry.
    let proj_path = project_filter_path(workspace_dir);
    if let Ok(bytes) = read_bounded_bytes(&proj_path) {
        let canonical = std::fs::canonicalize(&proj_path).unwrap_or(proj_path);
        let hash = sha256_hex(&bytes);
        let is_trusted = store
            .get_trusted_filter_hash(repo_scope, &canonical.display().to_string())
            .ok()
            .flatten()
            .as_deref()
            == Some(hash.as_str());
        if is_trusted
            && let Ok(file) = parse_filter_file(&bytes)
            && let Some(hit) = apply_first_matching_rule(&file, command, raw, is_error)
        {
            return Some(hit);
        }
    }

    // 2. User-global `~/.zenko/filters.toml` (owner-controlled, trusted by default).
    if let Ok(bytes) = read_bounded_bytes(&user_filter_path(data_dir))
        && let Ok(file) = parse_filter_file(&bytes)
        && let Some(hit) = apply_first_matching_rule(&file, command, raw, is_error)
    {
        return Some(hit);
    }

    None
}

fn apply_first_matching_rule(
    file: &FilterFileV1,
    command: &str,
    raw: &str,
    is_error: bool,
) -> Option<(String, String)> {
    for (name, rule) in &file.filters {
        if !rule_matches_command(rule, command) {
            continue;
        }
        let shaped = apply_rule(rule, raw, is_error);
        if shaped.len() < raw.len() {
            return Some((name.clone(), shaped));
        }
    }
    None
}

/// Computes and stores SHA-256 trust for `<workspace>/.zenko/filters.toml` after validating it.
pub fn trust_project_filter_file(
    store: &SqliteStore,
    repo_scope: &str,
    workspace_dir: &Path,
) -> std::result::Result<(String, CustomFilterVerifyReport), String> {
    let path = project_filter_path(workspace_dir);
    let report = verify_filter_file_at(&path);
    if !report.exists {
        return Err(format!("No filter file found at {}", path.display()));
    }
    if !report.valid {
        return Err(format!(
            "Refusing to trust invalid filter file ({} error(s), {} failed test(s))",
            report.errors.len(),
            report.tests_failed
        ));
    }
    let bytes = read_bounded_bytes(&path)?;
    let hash = sha256_hex(&bytes);
    let canonical = std::fs::canonicalize(&path).unwrap_or(path);
    store
        .trust_filter_file(repo_scope, &canonical.display().to_string(), &hash)
        .map_err(|e| e.to_string())?;
    Ok((hash, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_filter_dsl_verifies_inline_tests_and_enforces_sha256_trust() {
        let dir = std::env::temp_dir().join(format!("zenko_cfilter_{}", zenko_core::new_id()));
        let data_dir = dir.join("data");
        let ws = dir.join("repo");
        std::fs::create_dir_all(ws.join(".zenko")).expect("mkdir .zenko");
        std::fs::create_dir_all(&data_dir).expect("mkdir data");

        let toml_body = r#"
schema_version = 1

[filters.bazel-build]
description = "Drop INFO noise from bazel build"
match_command_prefixes = ["bazel build"]
strip_ansi = true
strip_prefixes = ["INFO:"]
collapse_repeats = true
max_lines = 20
on_empty = "bazel build: ok"

[[tests.bazel-build]]
name = "drops info lines and preserves target summary"
input = "INFO: Loading package...\nINFO: Analyzing...\nTarget //app:server up-to-date:\n  bazel-bin/app/server\n"
expected = "Target //app:server up-to-date:\n  bazel-bin/app/server"
"#;
        let filter_path = project_filter_path(&ws);
        std::fs::write(&filter_path, toml_body).expect("write filters.toml");

        let db = SqliteStore::open(data_dir.join("zenko.db")).expect("open db");
        let scope = "project:test";

        // 1. Verify passes inline tests
        let vrep = verify_filter_file_at(&filter_path);
        assert!(vrep.exists && vrep.valid);
        assert_eq!(vrep.rule_count, 1);
        assert_eq!(vrep.tests_passed, 1);
        assert_eq!(vrep.tests_failed, 0);

        // 2. Untrusted repo filter MUST NOT execute
        let sample = "INFO: Loading 1\nINFO: Loading 2\nINFO: Loading 3\nTarget //app:ok\n";
        assert!(
            try_apply_trusted_custom_filters(&db, scope, &data_dir, &ws, "bazel build //...", sample, false)
                .is_none()
        );

        // 3. After explicit SHA-256 trust, filter applies cleanly
        let (hash, _) = trust_project_filter_file(&db, scope, &ws).expect("trust");
        let status = inspect_filter_trust_status(&db, scope, &data_dir, &ws);
        assert!(status.trusted);
        assert_eq!(status.project_sha256.as_deref(), Some(hash.as_str()));

        let (rule_name, shaped) = try_apply_trusted_custom_filters(
            &db,
            scope,
            &data_dir,
            &ws,
            "bazel build //...",
            sample,
            false,
        )
        .expect("trusted filter applies");
        assert_eq!(rule_name, "bazel-build");
        assert_eq!(shaped, "Target //app:ok");

        // 4. Modifying filters.toml invalidates trust automatically (tamper-evident!)
        std::fs::write(&filter_path, format!("{toml_body}\n# tampered\n")).expect("tamper");
        let after_tamper = inspect_filter_trust_status(&db, scope, &data_dir, &ws);
        assert!(!after_tamper.trusted);
        assert!(
            try_apply_trusted_custom_filters(&db, scope, &data_dir, &ws, "bazel build //...", sample, false)
                .is_none()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
