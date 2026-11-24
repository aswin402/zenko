//! Fast `.gitignore`-respecting file discovery (`glob_find`) powered by `ignore::WalkBuilder`
//! and a pure-Rust deterministic 2-pointer / DP glob matcher — ZERO regex, zero shell calls,
//! with `mtime` (newest-first, Claude Code style) and lexicographical `path` sorting.

use crate::jail::{format_display_rel_path, resolve_readable_path};
use async_trait::async_trait;
use ignore::WalkBuilder;
use serde_json::{Value, json};
use std::time::SystemTime;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    DEFAULT_SEARCH_SKIP_DIRS, GLOB_DEFAULT_MAX_RESULTS, GLOB_MAX_BRACE_EXPANSIONS,
    GLOB_MAX_RESULTS_CAP, GlobFileType, GlobSort, Result, RiskLevel, TOOL_GLOB_FIND, Tool,
    ToolConcurrency, ToolContext, ToolExposure, ToolOutput, ToolSet, ToolSpec, ZenkoError,
};

pub struct GlobFindTool;

#[async_trait]
impl Tool for GlobFindTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::CodeIntel
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Eager
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_GLOB_FIND.into(),
            description: "Fast `.gitignore`-respecting wildcard file discovery (`*`, `**`, `?`, `[a-z]`, `{a,b}`). Defaults to sorting by modification time (most recently edited files first).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Glob pattern (e.g. `**/*.rs`, `src/**/*.{ts,tsx}`, `Cargo.toml`). Defaults to `**/*`." },
                    "path": { "type": "string", "description": "Base directory inside workspace or `hub:` (default '.')." },
                    "sort": { "type": "string", "enum": ["mtime", "path"], "description": "Result order: `mtime` (newest first, default) or `path` (lexicographical)." },
                    "file_type": { "type": "string", "enum": ["file", "dir", "any"], "description": "Match only files (default), directories, or both." },
                    "max_depth": { "type": "integer", "description": "Optional max directory depth." },
                    "include_hidden": { "type": "boolean", "description": "Include hidden dotfiles/directories (default false)." },
                    "respect_gitignore": { "type": "boolean", "description": "Respect `.gitignore` rules (default true)." },
                    "max_results": { "type": "integer", "description": "Maximum paths to return (default 100, cap 500)." }
                }
            }),
            risk: RiskLevel::Safe,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let raw_pattern = args
            .get("pattern")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("**/*")
            .to_string();
        let req_path = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let root = resolve_readable_path(
            &ctx.workspace_dir,
            &ctx.hub_dir,
            &ctx.data_dir,
            req_path,
            true,
        )?;
        let ws_root = ctx
            .workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| ctx.workspace_dir.clone());
        let hub_root = ctx
            .hub_dir
            .canonicalize()
            .unwrap_or_else(|_| ctx.hub_dir.clone());
        let sort = args
            .get("sort")
            .and_then(Value::as_str)
            .map(GlobSort::from_str_loose)
            .unwrap_or(GlobSort::Mtime);
        let file_type = args
            .get("file_type")
            .and_then(Value::as_str)
            .map(GlobFileType::from_str_loose)
            .unwrap_or(GlobFileType::File);
        let max_depth = args
            .get("max_depth")
            .and_then(Value::as_u64)
            .map(|d| d.clamp(1, 64) as usize);
        let include_hidden = args
            .get("include_hidden")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let respect_gitignore = args
            .get("respect_gitignore")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let max_results = args
            .get("max_results")
            .and_then(Value::as_u64)
            .unwrap_or(GLOB_DEFAULT_MAX_RESULTS as u64)
            .clamp(1, GLOB_MAX_RESULTS_CAP as u64) as usize;

        let compiled = CompiledGlob::compile(&raw_pattern);

        let (matched_paths, total_matched, truncated) = tokio::task::spawn_blocking(move || -> Result<(Vec<String>, usize, bool)> {
            let mut walker = WalkBuilder::new(&root);
            walker
                .hidden(!include_hidden)
                .git_ignore(respect_gitignore)
                .git_global(respect_gitignore)
                .git_exclude(respect_gitignore)
                .require_git(false)
                .max_depth(max_depth)
                .filter_entry(move |dent| {
                    if dent.depth() == 0 {
                        return true;
                    }
                    let is_dir = dent.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                    if !is_dir {
                        return true;
                    }
                    let name = dent.file_name().to_str().unwrap_or("");
                    if name == ".git" {
                        return false;
                    }
                    match respect_gitignore {
                        true => !DEFAULT_SEARCH_SKIP_DIRS.contains(&name),
                        false => true,
                    }
                });

            let mut entries: Vec<(String, SystemTime)> = Vec::new();
            for dent in walker.build().flatten() {
                if cancel.is_cancelled() {
                    return Err(ZenkoError::Cancelled);
                }
                if dent.depth() == 0 {
                    continue;
                }
                let Some(ft) = dent.file_type() else {
                    continue;
                };
                let type_ok = match file_type {
                    GlobFileType::File => ft.is_file(),
                    GlobFileType::Dir => ft.is_dir(),
                    GlobFileType::Any => ft.is_file() || ft.is_dir(),
                };
                if !type_ok {
                    continue;
                }

                let rel_from_search = dent
                    .path()
                    .strip_prefix(&root)
                    .unwrap_or(dent.path())
                    .to_string_lossy()
                    .replace('\\', "/");
                if !compiled.matches(&rel_from_search) {
                    continue;
                }

                let display_rel =
                    format_display_rel_path(dent.path(), &ws_root, &hub_root, &rel_from_search);
                let mtime = dent
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                entries.push((display_rel, mtime));
            }

            match sort {
                GlobSort::Mtime => entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0))),
                GlobSort::Path => entries.sort_by(|a, b| a.0.cmp(&b.0)),
            }

            let total = entries.len();
            let was_truncated = total > max_results;
            let paths: Vec<String> = entries
                .into_iter()
                .take(max_results)
                .map(|(p, _)| p)
                .collect();
            Ok((paths, total, was_truncated))
        })
        .await
        .map_err(|e| ZenkoError::Tool(format!("glob join error: {e}")))??;

        let content = match matched_paths.is_empty() {
            true => format!("No paths matched pattern `{raw_pattern}`."),
            false => {
                let mut header = format!(
                    "Matched {} path(s) for `{raw_pattern}`",
                    matched_paths.len()
                );
                if truncated {
                    header.push_str(&format!(" (showing top {max_results} of {total_matched})"));
                }
                format!("{header}:\n{}", matched_paths.join("\n"))
            }
        };
        let mut out = ToolOutput::ok(content);
        out.metadata = Some(json!({
            "pattern": raw_pattern,
            "returned": matched_paths.len(),
            "total_matched": total_matched,
            "truncated": truncated,
            "paths": matched_paths,
        }));
        Ok(out)
    }
}

/// Pure-Rust zero-regex compiled glob matcher supporting `*`, `**`, `?`, `[a-z]`, `[!a-z]`,
/// and bounded `{a,b}` brace expansion.
#[derive(Debug, Clone)]
pub struct CompiledGlob {
    alternatives: Vec<Vec<String>>,
    match_basename_only: bool,
}

impl CompiledGlob {
    pub fn compile(raw: &str) -> Self {
        let normalized = raw.trim().replace('\\', "/");
        let clean = normalized.trim_start_matches("./");
        let match_basename_only = !clean.contains('/');
        let expanded = expand_braces(clean, GLOB_MAX_BRACE_EXPANSIONS);
        let alternatives = expanded
            .into_iter()
            .map(|pat| {
                pat.split('/')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect::<Vec<_>>()
            })
            .collect();
        Self {
            alternatives,
            match_basename_only,
        }
    }

    pub fn matches(&self, rel_path: &str) -> bool {
        let clean = rel_path.replace('\\', "/");
        let clean = clean.trim_start_matches("./").trim_start_matches('/');
        if self.match_basename_only {
            let base = clean.rsplit('/').next().unwrap_or(clean);
            return self
                .alternatives
                .iter()
                .any(|segs| segs.first().is_some_and(|p| match_segment(p, base)));
        }
        let path_segs: Vec<&str> = clean.split('/').filter(|s| !s.is_empty()).collect();
        self.alternatives
            .iter()
            .any(|pat_segs| match_path_segments(pat_segs, &path_segs))
    }
}

/// Deterministic iterative brace expander (`src/**/*.{ts,tsx}` -> `[src/**/*.ts, src/**/*.tsx]`).
fn expand_braces(pattern: &str, max_results: usize) -> Vec<String> {
    let mut queue = vec![pattern.to_string()];
    let mut out = Vec::new();
    while let Some(cur) = queue.pop() {
        if out.len() >= max_results {
            break;
        }
        let Some((open, close, items)) = find_first_brace_group(&cur) else {
            out.push(cur);
            continue;
        };
        let prefix = &cur[..open];
        let suffix = &cur[close + 1..];
        for item in items.into_iter().rev() {
            if queue.len() + out.len() >= max_results {
                break;
            }
            queue.push(format!("{prefix}{item}{suffix}"));
        }
    }
    if out.is_empty() {
        out.push(pattern.to_string());
    }
    out
}

fn find_first_brace_group(s: &str) -> Option<(usize, usize, Vec<&str>)> {
    let bytes = s.as_bytes();
    let open = bytes.iter().position(|&b| b == b'{')?;
    let mut depth = 0usize;
    let mut close = None;
    let mut split_positions = Vec::new();
    for (idx, &b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    close = Some(idx);
                    break;
                }
            }
            b',' if depth == 1 => split_positions.push(idx),
            _ => {}
        }
    }
    let close_idx = close?;
    let mut items = Vec::with_capacity(split_positions.len() + 1);
    let mut start = open + 1;
    for comma in split_positions {
        items.push(&s[start..comma]);
        start = comma + 1;
    }
    items.push(&s[start..close_idx]);
    Some((open, close_idx, items))
}

/// Deterministic $O(P \cdot S)$ DP matching path segments where `**` matches 0..N segments.
fn match_path_segments(pat: &[String], path: &[&str]) -> bool {
    let n = path.len();
    let mut prev = vec![false; n + 1];
    let mut curr = vec![false; n + 1];
    prev[0] = true;

    for p_seg in pat {
        curr.fill(false);
        if p_seg == "**" {
            let mut any_prev = false;
            for j in 0..=n {
                any_prev |= prev[j];
                curr[j] = any_prev;
            }
        } else {
            for j in 1..=n {
                if prev[j - 1] && match_segment(p_seg, path[j - 1]) {
                    curr[j] = true;
                }
            }
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

/// Zero-allocation two-pointer matcher for ASCII `*`/`?` segments (fast path for 99% of globs).
fn match_wildcard_ascii(pat: &[u8], text: &[u8]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star_pi: Option<usize> = None;
    let mut star_ti = 0usize;

    while ti < text.len() {
        match (pat.get(pi).copied(), star_pi) {
            (Some(b'?'), _) => {
                pi += 1;
                ti += 1;
            }
            (Some(p), _) if p == text[ti] => {
                pi += 1;
                ti += 1;
            }
            (Some(b'*'), _) => {
                star_pi = Some(pi);
                star_ti = ti;
                pi += 1;
            }
            (_, Some(sp)) => {
                star_ti += 1;
                ti = star_ti;
                pi = sp + 1;
            }
            (_, None) => return false,
        }
    }
    while pi < pat.len() && pat[pi] == b'*' {
        pi += 1;
    }
    pi == pat.len()
}

/// Deterministic single-segment wildcard matcher (`*`, `?`, `[abc]`, `[a-z]`, `[!0-9]`).
fn match_segment(pat: &str, text: &str) -> bool {
    if pat.is_ascii() && text.is_ascii() && !pat.as_bytes().contains(&b'[') {
        return match_wildcard_ascii(pat.as_bytes(), text.as_bytes());
    }
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (m, n) = (p.len(), t.len());
    let mut prev = vec![false; n + 1];
    let mut curr = vec![false; n + 1];
    prev[0] = true;

    let mut pi = 0usize;
    while pi < m {
        curr.fill(false);
        match p[pi] {
            '*' => {
                let mut any = false;
                for j in 0..=n {
                    any |= prev[j];
                    curr[j] = any;
                }
                pi += 1;
            }
            '?' => {
                curr[1..=n].copy_from_slice(&prev[..n]);
                pi += 1;
            }
            '[' => {
                if let Some((end_idx, negate, ranges)) = parse_char_class(&p, pi) {
                    for j in 1..=n {
                        if prev[j - 1] && char_class_matches(t[j - 1], negate, &ranges) {
                            curr[j] = true;
                        }
                    }
                    pi = end_idx + 1;
                } else {
                    for j in 1..=n {
                        curr[j] = prev[j - 1] && t[j - 1] == '[';
                    }
                    pi += 1;
                }
            }
            lit => {
                for j in 1..=n {
                    curr[j] = prev[j - 1] && t[j - 1] == lit;
                }
                pi += 1;
            }
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

type CharClassSpec = (usize, bool, Vec<(char, char)>);

fn parse_char_class(p: &[char], start: usize) -> Option<CharClassSpec> {
    let mut i = start + 1;
    if i >= p.len() {
        return None;
    }
    let negate = matches!(p[i], '!' | '^');
    if negate {
        i += 1;
    }
    let mut ranges = Vec::new();
    while i < p.len() && p[i] != ']' {
        if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
            ranges.push((p[i], p[i + 2]));
            i += 3;
        } else {
            ranges.push((p[i], p[i]));
            i += 1;
        }
    }
    match i < p.len() && p[i] == ']' {
        true => Some((i, negate, ranges)),
        false => None,
    }
}

fn char_class_matches(ch: char, negate: bool, ranges: &[(char, char)]) -> bool {
    let hit = ranges.iter().any(|&(lo, hi)| ch >= lo && ch <= hi);
    match negate {
        true => !hit,
        false => hit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Notify;

    #[test]
    fn zero_regex_glob_matches_double_star_braces_and_char_classes() {
        let g1 = CompiledGlob::compile("src/**/*.{ts,tsx}");
        assert!(g1.matches("src/App.tsx"));
        assert!(g1.matches("src/components/chat/Phase6Bar.tsx"));
        assert!(g1.matches("src/lib/api.ts"));
        assert!(!g1.matches("src/lib/api.js"));
        assert!(!g1.matches("other/App.tsx"));

        let g2 = CompiledGlob::compile("*.rs");
        assert!(g2.matches("main.rs"));
        assert!(g2.matches("crates/zenko-core/src/lib.rs"));
        assert!(!g2.matches("Cargo.toml"));

        let g3 = CompiledGlob::compile("test_[0-9]?.txt");
        assert!(g3.matches("test_4a.txt"));
        assert!(!g3.matches("test_aa.txt"));
    }

    #[tokio::test]
    async fn glob_find_respects_gitignore_and_sorts_by_mtime() {
        let dir = std::env::temp_dir().join(format!("zenko_glob_{}", zenko_core::new_id()));
        std::fs::create_dir_all(dir.join("src")).expect("src");
        std::fs::create_dir_all(dir.join("ignored_dir")).expect("ignored");
        std::fs::write(dir.join(".gitignore"), "ignored_dir/\n*.log\n").expect("gi");
        std::fs::write(dir.join("src/old.rs"), "fn old() {}\n").expect("old");
        std::fs::write(dir.join("ignored_dir/skip.rs"), "fn skip() {}\n").expect("skip");
        std::fs::write(dir.join("debug.log"), "log\n").expect("log");
        std::thread::sleep(std::time::Duration::from_millis(25));
        std::fs::write(dir.join("src/newest.rs"), "fn newest() {}\n").expect("new");

        let ctx = ToolContext {
            session_id: "s1".into(),
            workspace_dir: dir.clone(),
            data_dir: dir.clone(),
            hub_dir: dir.clone(),
            shell_timeout_secs: 10,
            auto_background_after_secs: 0,
            max_output_bytes: 64 * 1024,
            headless_no_wait: true,
            demote_bg: Arc::new(Notify::new()),
            file_state: None,
        };

        let out = GlobFindTool
            .execute(
                &ctx,
                json!({"pattern": "**/*.rs", "sort": "mtime"}),
                CancellationToken::new(),
            )
            .await
            .expect("glob exec");
        assert!(!out.is_error);
        let paths = out
            .metadata
            .as_ref()
            .and_then(|m| m.get("paths"))
            .and_then(Value::as_array)
            .expect("paths array");
        assert_eq!(paths.len(), 2, "ignored_dir/skip.rs must be filtered by .gitignore");
        assert_eq!(paths[0].as_str(), Some("src/newest.rs"));
        assert_eq!(paths[1].as_str(), Some("src/old.rs"));

        // `respect_gitignore: false` must also traverse default skip dirs like `dist/`
        std::fs::create_dir_all(dir.join("dist")).expect("dist");
        std::fs::write(dir.join("dist/bundle.js"), "console.log(1);\n").expect("bundle");
        let dist_out = GlobFindTool
            .execute(
                &ctx,
                json!({"pattern": "**/*.js", "respect_gitignore": false}),
                CancellationToken::new(),
            )
            .await
            .expect("glob dist");
        assert!(dist_out.content.contains("dist/bundle.js"), "dist_out={}", dist_out.content);

        // `hub:` paths must be displayed with `hub:` prefix instead of leaking absolute host paths
        let hub_tmp = std::env::temp_dir().join(format!("zenko_glob_hub_{}", zenko_core::new_id()));
        std::fs::create_dir_all(hub_tmp.join("scripts")).expect("scripts");
        std::fs::write(hub_tmp.join("scripts/check.py"), "print(1)\n").expect("py");
        let mut hub_ctx = ctx.clone();
        hub_ctx.hub_dir = hub_tmp.clone();
        let hub_out = GlobFindTool
            .execute(
                &hub_ctx,
                json!({"path": "hub:scripts", "pattern": "*.py"}),
                CancellationToken::new(),
            )
            .await
            .expect("glob hub");
        assert!(hub_out.content.contains("hub:scripts/check.py"), "hub_out={}", hub_out.content);
        let _ = std::fs::remove_dir_all(hub_tmp);

        let _ = std::fs::remove_dir_all(dir);
    }
}
