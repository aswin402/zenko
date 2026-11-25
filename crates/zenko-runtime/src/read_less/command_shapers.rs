//! Zero-regex, deterministic command-aware output shapers (inspired by `rtk-ai/rtk`).
//! Classifies `shell_exec` & `git_tool` invocations into ecosystem families (`cargo`, `tsc`,
//! `lint`, `git`, `pkg`, `docker`, `test`) and compacts verbose compiler, linter, diff, log,
//! package-install, and container build outputs while preserving errors and diagnostics verbatim.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use zenko_core::{
    MCP_TOOL_PREFIX, READ_LESS_BUILD_PROGRESS_PREFIXES, READ_LESS_DIAG_MAX_GROUPS,
    READ_LESS_DIAG_SAMPLES_BALANCED, READ_LESS_DIAG_SAMPLES_HIGH, READ_LESS_DIAG_SAMPLES_ULTRA,
    READ_LESS_GENERATED_DIFF_SUFFIXES, READ_LESS_GIT_LOG_MAX_ENTRIES,
    READ_LESS_LOCKFILE_DIFF_NAMES, READ_LESS_PKG_MAX_WARN_LINES, TOOL_AGENT_WAIT, TOOL_GIT,
    TOOL_GLOB_FIND, TOOL_JOB_READ, TOOL_SCRIPT_BLOCK, TOOL_SHELL_EXEC, TOOL_SPAWN_AGENT,
    TOOL_WEB_FETCH, TOOL_WEB_SEARCH, ZMode,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandFamily {
    RustCompiler,
    TypeScriptCompiler,
    Linter,
    GitDiff,
    GitLog,
    GitStatus,
    PackageInstall,
    ContainerOrK8s,
    TestRunner,
    Generic,
}

impl CommandFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RustCompiler => "cargo",
            Self::TypeScriptCompiler => "tsc",
            Self::Linter => "lint",
            Self::GitDiff | Self::GitLog | Self::GitStatus => "git",
            Self::PackageInstall => "pkg",
            Self::ContainerOrK8s => "docker",
            Self::TestRunner => "test",
            Self::Generic => "shell",
        }
    }
}

/// Maps `(tool_name, CommandFamily, custom_filter_hit)` to a stable analytics bucket label.
pub fn command_family_label(
    tool_name: &str,
    family: CommandFamily,
    custom_filter_hit: bool,
) -> &'static str {
    if custom_filter_hit {
        return "custom";
    }
    if tool_name.starts_with(MCP_TOOL_PREFIX) {
        return "mcp";
    }
    if family != CommandFamily::Generic {
        return family.as_str();
    }
    match tool_name {
        TOOL_GIT => "git",
        TOOL_GLOB_FIND => "glob",
        TOOL_WEB_FETCH | TOOL_WEB_SEARCH => "web",
        TOOL_SPAWN_AGENT | TOOL_AGENT_WAIT => "agent",
        TOOL_SCRIPT_BLOCK => "script",
        TOOL_JOB_READ => "job",
        _ => "shell",
    }
}

/// Deterministic zero-regex command classifier inspecting `shell_exec` or `git_tool` arguments.
/// Returns `(CommandFamily, Option<normalized_command_string>)`.
pub fn classify_tool_command(tool_name: &str, tool_args: &Value) -> (CommandFamily, Option<String>) {
    match tool_name {
        TOOL_GIT => {
            let action = tool_args
                .get("action")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_ascii_lowercase())
                .unwrap_or_default();
            let fam = match action.as_str() {
                "diff" | "show" => CommandFamily::GitDiff,
                "log" => CommandFamily::GitLog,
                "status" => CommandFamily::GitStatus,
                _ => CommandFamily::Generic,
            };
            let cmd = match action.is_empty() {
                true => None,
                false => Some(format!("git {action}")),
            };
            (fam, cmd)
        }
        TOOL_SHELL_EXEC => {
            let Some(raw_cmd) = tool_args.get("command").and_then(Value::as_str) else {
                return (CommandFamily::Generic, None);
            };
            let primary = extract_primary_shell_segment(raw_cmd);
            let fam = classify_shell_command_str(primary);
            (fam, Some(primary.to_string()))
        }
        _ => (CommandFamily::Generic, None),
    }
}

/// Picks the main executable segment of simple chains like `cd webui && npm run build`.
pub fn extract_primary_shell_segment(cmd: &str) -> &str {
    let trimmed = cmd.trim();
    // Respect pipes as left-side source; pick the last segment after `&&` or `;` when chaining `cd`/`export`.
    let last_chain = trimmed
        .rsplit("&&")
        .next()
        .unwrap_or(trimmed)
        .rsplit(';')
        .next()
        .unwrap_or(trimmed)
        .trim();
    // Drop trailing pipe consumer (`| head`, `| tail`, etc.) only for classification.
    last_chain.split('|').next().unwrap_or(last_chain).trim()
}

fn classify_shell_command_str(cmd: &str) -> CommandFamily {
    let mut toks: Vec<&str> = Vec::with_capacity(8);
    let mut iter = cmd.split_whitespace().peekable();

    // Skip leading env assignments (`RUST_LOG=debug`, `CI=1`) and common runner wrappers.
    while let Some(&tok) = iter.peek() {
        if tok.contains('=') && !tok.starts_with('-') {
            iter.next();
            continue;
        }
        match tok {
            "sudo" | "env" | "time" | "nice" | "nohup" | "rtk" => {
                iter.next();
            }
            "timeout" => {
                iter.next();
                if iter.peek().is_some() {
                    iter.next(); // duration arg
                }
            }
            _ => break,
        }
    }

    for tok in iter.take(6) {
        toks.push(tok);
    }
    if toks.is_empty() {
        return CommandFamily::Generic;
    }

    // Unwrap package/script runners (`npx tsc`, `pnpm exec eslint`, `uv run pytest`, `bunx vitest`)
    let mut idx = 0usize;
    let first_bin = basename_lower(toks[0]);
    if matches!(first_bin.as_str(), "npx" | "bunx") && toks.len() > 1 {
        idx = 1;
    } else if matches!(first_bin.as_str(), "pnpm" | "npm" | "yarn" | "bun" | "uv" | "poetry")
        && toks.len() > 2
        && matches!(toks[1], "exec" | "dlx" | "run")
    {
        // Note: `npm run test` / `pnpm run build` handled below if target isn't an explicit binary
        let candidate = basename_lower(toks[2]);
        if matches!(
            candidate.as_str(),
            "tsc"
                | "eslint"
                | "biome"
                | "oxlint"
                | "ruff"
                | "mypy"
                | "pytest"
                | "vitest"
                | "jest"
                | "playwright"
        ) {
            idx = 2;
        }
    }

    let bin = basename_lower(toks[idx]);
    let sub = toks
        .iter()
        .skip(idx + 1)
        .copied()
        .find(|t| !t.starts_with('-'))
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    match bin.as_str() {
        "cargo" => match sub.as_str() {
            "test" | "nextest" => CommandFamily::TestRunner,
            "check" | "clippy" | "build" | "rustc" | "bench" | "doc" => {
                CommandFamily::RustCompiler
            }
            _ => CommandFamily::RustCompiler,
        },
        "rustc" | "clippy-driver" => CommandFamily::RustCompiler,
        "tsc" => CommandFamily::TypeScriptCompiler,
        "eslint" | "biome" | "oxlint" | "ruff" | "mypy" | "flake8" | "pylint" | "golangci-lint"
        | "shellcheck" | "hadolint" => CommandFamily::Linter,
        "git" => match sub.as_str() {
            "diff" | "show" => CommandFamily::GitDiff,
            "log" | "reflog" => CommandFamily::GitLog,
            "status" => CommandFamily::GitStatus,
            _ => CommandFamily::Generic,
        },
        "pytest" | "vitest" | "jest" | "playwright" | "mocha" | "ava" | "rspec" => {
            CommandFamily::TestRunner
        }
        "go" => match sub.as_str() {
            "test" => CommandFamily::TestRunner,
            "vet" | "build" => CommandFamily::Linter,
            _ => CommandFamily::Generic,
        },
        "npm" | "pnpm" | "yarn" | "bun" => match sub.as_str() {
            "test" | "t" => CommandFamily::TestRunner,
            "install" | "i" | "ci" | "add" | "update" | "upgrade" | "audit" | "outdated" => {
                CommandFamily::PackageInstall
            }
            "run" => {
                let script = toks
                    .iter()
                    .skip(idx + 2)
                    .copied()
                    .find(|t| !t.starts_with('-'))
                    .map(str::to_ascii_lowercase)
                    .unwrap_or_default();
                if script.contains("test") {
                    CommandFamily::TestRunner
                } else if script.contains("lint") || script.contains("typecheck") {
                    CommandFamily::Linter
                } else {
                    CommandFamily::Generic
                }
            }
            _ => CommandFamily::Generic,
        },
        "pip" | "pip3" | "uv" | "poetry" | "brew" | "apt" | "apt-get" => match sub.as_str() {
            "install" | "sync" | "add" | "upgrade" | "update" | "lock" => {
                CommandFamily::PackageInstall
            }
            _ => CommandFamily::Generic,
        },
        "docker" | "podman" | "kubectl" | "helm" => CommandFamily::ContainerOrK8s,
        _ => CommandFamily::Generic,
    }
}

fn basename_lower(tok: &str) -> String {
    tok.rsplit('/')
        .next()
        .unwrap_or(tok)
        .trim_matches(|c| matches!(c, '"' | '\''))
        .to_ascii_lowercase()
}

/// Normalizes carriage-return (`\r`) progress bar overwrites on each line while preserving `\r\n`.
pub fn normalize_carriage_returns(text: &str) -> String {
    if !text.as_bytes().contains(&b'\r') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    for (idx, raw_line) in text.split('\n').enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        match line.contains('\r') {
            false => out.push_str(line),
            true => {
                let final_frame = line
                    .rsplit('\r')
                    .find(|s| !s.trim().is_empty())
                    .unwrap_or("");
                out.push_str(final_frame);
            }
        }
    }
    out
}

fn diag_samples_for_zmode(zmode: ZMode) -> Option<usize> {
    match zmode {
        ZMode::Lite => None,
        ZMode::Balanced => Some(READ_LESS_DIAG_SAMPLES_BALANCED),
        ZMode::High => Some(READ_LESS_DIAG_SAMPLES_HIGH),
        ZMode::Ultra => Some(READ_LESS_DIAG_SAMPLES_ULTRA),
    }
}

/// Groups compiler & linter diagnostics (`cargo`/`rustc`/`clippy`, `tsc`, `eslint`/`biome`/`ruff`/`mypy`)
/// by `(severity, code_or_rule)` while preserving exact `file:line:col` locations and primary messages.
pub fn shape_compiler_or_lint(
    text: &str,
    family: CommandFamily,
    zmode: ZMode,
) -> Option<String> {
    let samples_cap = diag_samples_for_zmode(zmode)?;
    match family {
        CommandFamily::RustCompiler => shape_rust_compiler(text, samples_cap),
        CommandFamily::TypeScriptCompiler => shape_tsc_or_line_diags(text, samples_cap, true),
        CommandFamily::Linter => shape_tsc_or_line_diags(text, samples_cap, false),
        _ => None,
    }
}

fn shape_rust_compiler(text: &str, samples_cap: usize) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 12 {
        return None;
    }

    let mut exit_header: Option<&str> = None;
    let mut progress_collapsed = 0usize;
    let mut summary_lines: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut extra_diags: Vec<String> = Vec::new();
    let mut total_errors = 0usize;
    let mut total_warnings = 0usize;

    let mut i = 0usize;
    while i < lines.len() {
        let raw = lines[i];
        let trimmed = raw.trim();

        if i == 0 && trimmed.starts_with("exit_code:") {
            exit_header = Some(trimmed);
            i += 1;
            continue;
        }
        if READ_LESS_BUILD_PROGRESS_PREFIXES
            .iter()
            .any(|p| raw.trim_start().starts_with(p))
        {
            progress_collapsed += 1;
            i += 1;
            continue;
        }
        if trimmed.starts_with("Finished ")
            || trimmed.starts_with("error: could not compile")
            || trimmed.starts_with("error: aborting due to")
            || (trimmed.starts_with("warning: `") && trimmed.contains("generated"))
        {
            summary_lines.push(trimmed.to_string());
            i += 1;
            continue;
        }

        // Match Rust diagnostic headers: `error[E0425]: msg`, `error: msg`, `warning: msg`
        let parsed_hdr = parse_rust_diag_header(trimmed);
        if let Some((sev, code, msg)) = parsed_hdr {
            if sev == "error" {
                total_errors += 1;
            } else {
                total_warnings += 1;
            }
            // Scan ahead within this diagnostic block for `--> file:line:col` and skip ASCII caret spans.
            let mut loc: Option<&str> = None;
            let mut j = i + 1;
            while j < lines.len() {
                let next_t = lines[j].trim();
                if next_t.is_empty() {
                    j += 1;
                    break;
                }
                if parse_rust_diag_header(next_t).is_some()
                    || next_t.starts_with("Finished ")
                    || next_t.starts_with("error: could not compile")
                    || next_t.starts_with("For more information about")
                {
                    break;
                }
                if loc.is_none()
                    && let Some(path_pos) = next_t.strip_prefix("--> ")
                {
                    loc = Some(path_pos.trim());
                }
                j += 1;
            }
            let group_key = match code.is_empty() {
                true => sev.to_string(),
                false => format!("{sev}[{code}]"),
            };
            let entry = match loc {
                Some(l) => format!("{l} — {msg}"),
                None => msg.to_string(),
            };
            groups.entry(group_key).or_default().push(entry);
            i = j;
            continue;
        }

        if trimmed.starts_with("For more information about")
            || trimmed.starts_with("= note:")
            || trimmed.starts_with("= help:")
            || trimmed.starts_with('|')
        {
            i += 1;
            continue;
        }

        // Preserve unexpected panic / linker linesverbatim (bounded)
        let lower = trimmed.to_ascii_lowercase();
        if (lower.contains("undefined reference")
            || lower.contains("linking with")
            || lower.contains("panicked at"))
            && extra_diags.len() < 12
        {
            extra_diags.push(trimmed.to_string());
        }
        i += 1;
    }

    if total_errors == 0 && total_warnings == 0 && progress_collapsed < 6 {
        return None;
    }

    let mut out: Vec<String> = Vec::with_capacity(32);
    if let Some(h) = exit_header {
        out.push(h.to_string());
    }
    if progress_collapsed > 0 {
        out.push(format!(
            "[+{progress_collapsed} cargo build progress lines collapsed]"
        ));
    }
    if total_errors > 0 || total_warnings > 0 {
        out.push(format!(
            "Rust diagnostics: {total_errors} error(s), {total_warnings} warning(s) across {} group(s):",
            groups.len()
        ));
        // Emit errors first, then warnings.
        let mut ordered: Vec<(&String, &Vec<String>)> = groups.iter().collect();
        ordered.sort_by(|a, b| {
            let a_err = a.0.starts_with("error");
            let b_err = b.0.starts_with("error");
            b_err
                .cmp(&a_err)
                .then_with(|| b.1.len().cmp(&a.1.len()))
                .then_with(|| a.0.cmp(b.0))
        });
        for (key, items) in ordered.iter().take(READ_LESS_DIAG_MAX_GROUPS) {
            out.push(format!("  {key} (x{}):", items.len()));
            for item in items.iter().take(samples_cap) {
                out.push(format!("    - {item}"));
            }
            if items.len() > samples_cap {
                out.push(format!(
                    "    [+{} more `{key}` occurrence(s)]",
                    items.len() - samples_cap
                ));
            }
        }
        if ordered.len() > READ_LESS_DIAG_MAX_GROUPS {
            out.push(format!(
                "  [+{} more diagnostic group(s) omitted]",
                ordered.len() - READ_LESS_DIAG_MAX_GROUPS
            ));
        }
    }
    for d in extra_diags {
        out.push(d);
    }
    for s in summary_lines {
        out.push(s);
    }
    let rendered = out.join("\n");
    match rendered.len() < text.len() {
        true => Some(rendered),
        false => None,
    }
}

fn parse_rust_diag_header(trimmed: &str) -> Option<(&'static str, &str, &str)> {
    for (sev, prefix) in [("error", "error"), ("warning", "warning")] {
        let Some(rest) = trimmed.strip_prefix(prefix) else {
            continue;
        };
        if let Some(after_bracket) = rest.strip_prefix('[')
            && let Some((code, msg_part)) = after_bracket.split_once("]:")
        {
            return Some((sev, code.trim(), msg_part.trim()));
        }
        if let Some(msg) = rest.strip_prefix(':') {
            let m = msg.trim();
            if !m.is_empty()
                && !m.starts_with("could not compile")
                && !m.starts_with("aborting due to")
                && !m.starts_with("build failed")
            {
                return Some((sev, "", m));
            }
        }
    }
    None
}

fn shape_tsc_or_line_diags(text: &str, samples_cap: usize, is_tsc: bool) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 10 {
        return None;
    }

    let mut exit_header: Option<&str> = None;
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut files_seen: BTreeSet<String> = BTreeSet::new();
    let mut summary_lines: Vec<String> = Vec::new();
    let mut current_file: Option<&str> = None;
    let mut total_diags = 0usize;

    for (idx, raw) in lines.iter().enumerate() {
        let trimmed = raw.trim();
        if idx == 0 && trimmed.starts_with("exit_code:") {
            exit_header = Some(trimmed);
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        // ESLint stylish file heading (unindented file path)
        if !is_tsc
            && !raw.starts_with(' ')
            && !raw.starts_with('\t')
            && (trimmed.contains('/') || trimmed.contains('\\'))
            && !trimmed.contains(": ")
            && trimmed.contains('.')
        {
            current_file = Some(trimmed);
            continue;
        }

        if let Some((file, code, item)) = parse_tsc_or_lint_line(trimmed, current_file, is_tsc) {
            total_diags += 1;
            if !file.is_empty() {
                files_seen.insert(file.to_string());
            }
            groups.entry(code).or_default().push(item);
        } else if trimmed.starts_with("Found ")
            || trimmed.starts_with("✖ ")
            || trimmed.starts_with("Summary:")
            || trimmed.contains(" problems (")
            || trimmed.contains(" errors in ")
        {
            summary_lines.push(trimmed.to_string());
        }
    }

    if total_diags < 6 || groups.is_empty() {
        return None;
    }

    let mut ordered: Vec<(&String, &Vec<String>)> = groups.iter().collect();
    ordered.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(b.0)));

    let label = match is_tsc {
        true => "TypeScript diagnostics",
        false => "Linter diagnostics",
    };
    let mut out: Vec<String> = Vec::with_capacity(32);
    if let Some(h) = exit_header {
        out.push(h.to_string());
    }
    out.push(format!(
        "{label}: {total_diags} issue(s) across {} file(s) ({} rule/code group(s)):",
        files_seen.len().max(1),
        ordered.len()
    ));
    for (code, items) in ordered.iter().take(READ_LESS_DIAG_MAX_GROUPS) {
        out.push(format!("  {code} (x{}):", items.len()));
        for item in items.iter().take(samples_cap) {
            out.push(format!("    - {item}"));
        }
        if items.len() > samples_cap {
            out.push(format!(
                "    [+{} more `{code}` occurrence(s)]",
                items.len() - samples_cap
            ));
        }
    }
    for s in summary_lines {
        out.push(s);
    }

    let rendered = out.join("\n");
    match rendered.len() < text.len() {
        true => Some(rendered),
        false => None,
    }
}

fn parse_tsc_or_lint_line<'a>(
    trimmed: &'a str,
    current_file: Option<&'a str>,
    is_tsc: bool,
) -> Option<(&'a str, String, String)> {
    // 1. TypeScript format: `src/app.ts(42,10): error TS2345: Argument of type ...`
    if let Some((loc, rest)) = trimmed.split_once("): ")
        && let Some((file, coords)) = loc.rsplit_once('(')
        && coords.contains(',')
        && (rest.starts_with("error TS") || rest.starts_with("warning TS"))
    {
        let (code, msg) = rest.split_once(':').unwrap_or((rest, ""));
        return Some((
            file.trim(),
            code.trim().to_string(),
            format!("{}({}) — {}", file.trim(), coords.trim(), msg.trim()),
        ));
    }

    if is_tsc {
        return None;
    }

    // 2. Colon-delimited `file:line:col: CODE msg` (ruff, mypy, golangci-lint, tsc --pretty)
    let mut parts = trimmed.splitn(4, ':');
    if let (Some(file), Some(line_s), Some(col_s), Some(rest)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
        && !file.is_empty()
        && line_s.chars().all(|c| c.is_ascii_digit())
        && !line_s.is_empty()
        && col_s.trim().chars().all(|c| c.is_ascii_digit())
        && !col_s.trim().is_empty()
    {
        let msg = rest.trim();
        let code = extract_lint_code(msg).unwrap_or_else(|| "lint".to_string());
        return Some((
            file.trim(),
            code,
            format!("{}:{}:{} — {msg}", file.trim(), line_s, col_s.trim()),
        ));
    }

    // 3. ESLint stylish indented line: `12:4  error  Unexpected any  @typescript-eslint/no-explicit-any`
    if let Some(file) = current_file {
        let cols: Vec<&str> = trimmed.split_whitespace().collect();
        if cols.len() >= 4
            && cols[0].contains(':')
            && matches!(cols[1], "error" | "warning")
        {
            let rule = cols.last().copied().unwrap_or("eslint");
            let msg = cols[2..cols.len() - 1].join(" ");
            return Some((
                file,
                format!("{} {rule}", cols[1]),
                format!("{file}:{} — {msg}", cols[0]),
            ));
        }
    }

    None
}

fn extract_lint_code(msg: &str) -> Option<String> {
    // Trailing bracketed code `[no-unused-vars]` or `(no-unused-vars)`
    if let Some(open) = msg.rfind('[')
        && msg.ends_with(']')
    {
        let inner = msg[open + 1..msg.len() - 1].trim();
        if !inner.is_empty() && inner.len() <= 48 && !inner.contains(' ') {
            return Some(inner.to_string());
        }
    }
    // Leading code like `E501 Line too long`, `F401 ...`, `error TS2322:`
    let first = msg.split_whitespace().next()?;
    if first.len() >= 2
        && first.len() <= 16
        && first.bytes().any(|b| b.is_ascii_digit())
        && first
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':'))
    {
        return Some(first.trim_end_matches(':').to_string());
    }
    None
}

fn is_lockfile_or_generated_path(path: &str) -> bool {
    let clean = path.trim().trim_start_matches("a/").trim_start_matches("b/");
    let file_name = clean.rsplit('/').next().unwrap_or(clean);
    if READ_LESS_LOCKFILE_DIFF_NAMES
        .iter()
        .any(|name| file_name.eq_ignore_ascii_case(name))
    {
        return true;
    }
    if clean.starts_with("dist/") || clean.contains("/dist/") {
        return true;
    }
    READ_LESS_GENERATED_DIFF_SUFFIXES
        .iter()
        .any(|suf| file_name.ends_with(suf))
}

/// Compacts `git diff` / `git show` by folding lockfile/generated hunks into 1-line `+A/-D` summaries
/// and dropping noisy `index <sha>..<sha>` lines while keeping all source hunks 100% verbatim.
pub fn shape_git_diff(text: &str) -> Option<String> {
    if !text.contains("diff --git ") {
        return None;
    }
    let mut out: Vec<String> = Vec::with_capacity(text.lines().count() / 2 + 8);
    let mut in_collapsed_file = false;
    let mut collapsed_header = String::new();
    let mut collapsed_adds = 0usize;
    let mut collapsed_dels = 0usize;
    let mut collapsed_files = 0usize;
    let mut stripped_meta = 0usize;

    let flush_collapsed = |out: &mut Vec<String>,
                           in_col: &mut bool,
                           hdr: &mut String,
                           adds: &mut usize,
                           dels: &mut usize| {
        if *in_col {
            out.push(format!(
                "{hdr} [lockfile/generated diff collapsed: +{} / -{} lines]",
                *adds, *dels
            ));
            *in_col = false;
            hdr.clear();
            *adds = 0;
            *dels = 0;
        }
    };

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            flush_collapsed(
                &mut out,
                &mut in_collapsed_file,
                &mut collapsed_header,
                &mut collapsed_adds,
                &mut collapsed_dels,
            );
            let target_path = rest.split_whitespace().last().unwrap_or("");
            if is_lockfile_or_generated_path(target_path) {
                in_collapsed_file = true;
                collapsed_header = line.to_string();
                collapsed_files += 1;
                continue;
            }
            out.push(line.to_string());
            continue;
        }
        if in_collapsed_file {
            if line.starts_with('+') && !line.starts_with("+++") {
                collapsed_adds += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                collapsed_dels += 1;
            }
            continue;
        }
        if line.starts_with("index ") && line.contains("..") {
            stripped_meta += 1;
            continue;
        }
        out.push(line.to_string());
    }
    flush_collapsed(
        &mut out,
        &mut in_collapsed_file,
        &mut collapsed_header,
        &mut collapsed_adds,
        &mut collapsed_dels,
    );

    if collapsed_files == 0 && stripped_meta == 0 {
        return None;
    }
    let rendered = out.join("\n");
    match rendered.len() < text.len() {
        true => Some(rendered),
        false => None,
    }
}

/// Compacts standard multi-line `git log` output into `<8-sha> <date> <author> — <subject>` lines.
pub fn shape_git_log(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let commit_count = lines
        .iter()
        .filter(|l| l.starts_with("commit ") && l.len() >= 15)
        .count();
    if commit_count < 3 {
        return None;
    }

    let mut entries: Vec<String> = Vec::with_capacity(commit_count.min(READ_LESS_GIT_LOG_MAX_ENTRIES));
    let mut sha = String::new();
    let mut author = String::new();
    let mut date = String::new();
    let mut subject = String::new();
    let mut total_commits = 0usize;

    let flush = |entries: &mut Vec<String>,
                 total: &mut usize,
                 sha: &mut String,
                 author: &mut String,
                 date: &mut String,
                 subject: &mut String| {
        if sha.is_empty() {
            return;
        }
        *total += 1;
        if entries.len() < READ_LESS_GIT_LOG_MAX_ENTRIES {
            let short_sha: String = sha.chars().take(8).collect();
            let clean_author = author.split('<').next().unwrap_or(author).trim();
            entries.push(format!("{short_sha} {date} ({clean_author}) {subject}"));
        }
        sha.clear();
        author.clear();
        date.clear();
        subject.clear();
    };

    for line in &lines {
        if let Some(rest) = line.strip_prefix("commit ") {
            flush(
                &mut entries,
                &mut total_commits,
                &mut sha,
                &mut author,
                &mut date,
                &mut subject,
            );
            sha = rest.split_whitespace().next().unwrap_or("").to_string();
        } else if let Some(rest) = line.strip_prefix("Author:") {
            author = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("Date:") {
            // Compact date to first 3 tokens (e.g. `Wed Nov 25` or `2026-11-25`)
            let d = rest.trim();
            date = d.split_whitespace().take(3).collect::<Vec<_>>().join(" ");
        } else if subject.is_empty() && line.starts_with("    ") && !line.trim().is_empty() {
            subject = line.trim().to_string();
        }
    }
    flush(
        &mut entries,
        &mut total_commits,
        &mut sha,
        &mut author,
        &mut date,
        &mut subject,
    );

    if total_commits > READ_LESS_GIT_LOG_MAX_ENTRIES {
        entries.push(format!(
            "[+{} older commit(s) omitted]",
            total_commits - READ_LESS_GIT_LOG_MAX_ENTRIES
        ));
    }
    let rendered = entries.join("\n");
    match rendered.len() < text.len() {
        true => Some(rendered),
        false => None,
    }
}

/// Removes boilerplate `(use "git ..." to ...)` hints from long `git status` prose.
pub fn shape_git_status(text: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    let mut dropped_hints = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("(use \"git ") && trimmed.ends_with(')') {
            dropped_hints += 1;
            continue;
        }
        out.push(line.trim_end());
    }
    if dropped_hints == 0 {
        return None;
    }
    let rendered = out.join("\n");
    match rendered.len() < text.len() {
        true => Some(rendered),
        false => None,
    }
}

/// Condenses noisy `npm`/`pnpm`/`yarn`/`bun`/`pip`/`uv` install logs while keeping warnings,
/// errors, security vulnerabilities, and final install counts intact.
pub fn shape_package_install(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 14 {
        return None;
    }
    let mut out: Vec<String> = Vec::new();
    let mut collapsed = 0usize;
    let mut kept_warns = 0usize;

    for (idx, raw) in lines.iter().enumerate() {
        let trimmed = raw.trim();
        if idx == 0 && trimmed.starts_with("exit_code:") {
            out.push(trimmed.to_string());
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        let is_important = lower.contains("err!")
            || lower.contains("error")
            || lower.contains("failed")
            || lower.contains("vulnerabilit")
            || lower.contains("warn")
            || lower.starts_with("added ")
            || lower.starts_with("removed ")
            || lower.starts_with("changed ")
            || lower.starts_with("installed ")
            || lower.starts_with("successfully installed")
            || lower.starts_with("done in ")
            || lower.starts_with("audited ");
        let is_noise = trimmed.starts_with("GET ")
            || trimmed.starts_with("Progress:")
            || trimmed.starts_with("Packages:")
            || trimmed.starts_with("Resolving ")
            || trimmed.starts_with("Downloading ")
            || trimmed.starts_with("Using cached ")
            || trimmed.starts_with("Collecting ")
            || trimmed.starts_with("Installing collected")
            || trimmed.starts_with("Preparing metadata")
            || trimmed.starts_with("Building wheel")
            || trimmed.starts_with("Fetching ")
            || trimmed.starts_with("Extracting ")
            || trimmed.starts_with("+ ")
            || trimmed.starts_with("├")
            || trimmed.starts_with("└")
            || trimmed.starts_with("│");

        if is_important && !is_noise {
            if lower.contains("warn") {
                if kept_warns < READ_LESS_PKG_MAX_WARN_LINES {
                    out.push(trimmed.to_string());
                    kept_warns += 1;
                } else {
                    collapsed += 1;
                }
            } else {
                out.push(trimmed.to_string());
            }
        } else {
            collapsed += 1;
        }
    }

    if collapsed < 8 {
        return None;
    }
    out.insert(
        usize::from(out.first().is_some_and(|l| l.starts_with("exit_code:"))),
        format!("[+{collapsed} package install progress/tree lines collapsed]"),
    );
    let rendered = out.join("\n");
    match rendered.len() < text.len() {
        true => Some(rendered),
        false => None,
    }
}

/// Condenses Docker BuildKit layer progress (`#14 ... DONE 0.3s`, `#8 CACHED`) while keeping
/// build errors, final exported image digests, and non-progress output verbatim.
pub fn shape_container_or_k8s(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 14 {
        return None;
    }
    let mut out: Vec<String> = Vec::new();
    let mut buildkit_steps = 0usize;
    let mut cached_steps = 0usize;

    for (idx, raw) in lines.iter().enumerate() {
        let trimmed = raw.trim();
        if idx == 0 && trimmed.starts_with("exit_code:") {
            out.push(trimmed.to_string());
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        // BuildKit lines start with `#<digits> `
        let is_buildkit = trimmed.starts_with('#')
            && trimmed
                .split_whitespace()
                .next()
                .is_some_and(|tok| tok.len() >= 2 && tok[1..].bytes().all(|b| b.is_ascii_digit()));
        if is_buildkit {
            let lower = trimmed.to_ascii_lowercase();
            let is_err = lower.contains("error")
                || lower.contains("failed")
                || trimmed.contains("=> ERROR")
                || lower.contains("naming to ")
                || lower.contains("writing image ");
            if is_err {
                out.push(trimmed.to_string());
            } else if trimmed.ends_with("CACHED") {
                cached_steps += 1;
            } else {
                buildkit_steps += 1;
            }
            continue;
        }
        out.push(trimmed.to_string());
    }

    if buildkit_steps + cached_steps < 8 {
        return None;
    }
    let pos = usize::from(out.first().is_some_and(|l| l.starts_with("exit_code:")));
    out.insert(
        pos,
        format!(
            "[+{buildkit_steps} container build lines, {cached_steps} cached layer(s) collapsed]"
        ),
    );
    let rendered = out.join("\n");
    match rendered.len() < text.len() {
        true => Some(rendered),
        false => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_wrapped_shell_commands_and_git_tool_deterministically() {
        assert_eq!(
            classify_tool_command(TOOL_SHELL_EXEC, &json!({ "command": "RUST_LOG=info cargo check --workspace" })).0,
            CommandFamily::RustCompiler
        );
        assert_eq!(
            classify_tool_command(TOOL_SHELL_EXEC, &json!({ "command": "cd webui && npx tsc --noEmit" })).0,
            CommandFamily::TypeScriptCompiler
        );
        assert_eq!(
            classify_tool_command(TOOL_SHELL_EXEC, &json!({ "command": "uv run pytest -v" })).0,
            CommandFamily::TestRunner
        );
        assert_eq!(
            classify_tool_command(TOOL_SHELL_EXEC, &json!({ "command": "pnpm install" })).0,
            CommandFamily::PackageInstall
        );
        assert_eq!(
            classify_tool_command(TOOL_SHELL_EXEC, &json!({ "command": "docker build -t app ." })).0,
            CommandFamily::ContainerOrK8s
        );
        assert_eq!(
            classify_tool_command(TOOL_GIT, &json!({ "action": "diff" })).0,
            CommandFamily::GitDiff
        );
    }

    #[test]
    fn groups_cargo_compiler_errors_and_strips_caret_art() {
        let mut raw = String::from("exit_code: 101\n");
        for i in 0..8 {
            raw.push_str(&format!("   Compiling crate-{i} v0.1.0\n"));
        }
        for i in 1..=7 {
            raw.push_str(&format!(
                "error[E0425]: cannot find value `foo_{i}` in this scope\n  --> src/lib.rs:{i}0:9\n   |\n{i}0 |     let x = foo_{i};\n   |             ^^^^^ not found in this scope\n   = help: consider importing it\n\n"
            ));
        }
        raw.push_str("error: could not compile `demo` (lib) due to 7 previous errors\n");

        // Lite keeps raw untouched; Balanced groups tightly!
        assert!(shape_compiler_or_lint(&raw, CommandFamily::RustCompiler, ZMode::Lite).is_none());
        let shaped = shape_compiler_or_lint(&raw, CommandFamily::RustCompiler, ZMode::Balanced)
            .expect("balanced rust diagnostics grouped");
        assert!(shaped.contains("error[E0425] (x7):"));
        assert!(shaped.contains("src/lib.rs:10:9"));
        assert!(shaped.contains("[+2 more `error[E0425]` occurrence(s)]"));
        assert!(shaped.contains("error: could not compile `demo`"));
        assert!(shaped.len() < raw.len() / 2);
    }

    #[test]
    fn groups_tsc_errors_and_collapses_lockfile_diffs_and_git_logs() {
        let mut tsc = String::from("exit_code: 2\n");
        for i in 1..=10 {
            tsc.push_str(&format!(
                "src/view_{i}.tsx({i},12): error TS2322: Type 'string' is not assignable to type 'number'.\n"
            ));
        }
        let tsc_out = shape_compiler_or_lint(&tsc, CommandFamily::TypeScriptCompiler, ZMode::High)
            .expect("tsc shaped");
        assert!(tsc_out.contains("error TS2322 (x10):"));
        assert!(tsc_out.contains("[+7 more `error TS2322` occurrence(s)]"));

        let mut diff = String::from(
            "diff --git a/Cargo.lock b/Cargo.lock\nindex 1111111..2222222 100644\n--- a/Cargo.lock\n+++ b/Cargo.lock\n@@ -1,50 +1,50 @@\n",
        );
        for i in 0..40 {
            diff.push_str(&format!("+version = \"0.1.{i}\"\n-version = \"0.0.{i}\"\n"));
        }
        diff.push_str(
            "diff --git a/src/main.rs b/src/main.rs\nindex aaaa..bbbb 100644\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,2 +1,2 @@\n-old()\n+new()\n",
        );
        let diff_out = shape_git_diff(&diff).expect("lockfile collapsed");
        assert!(diff_out.contains("Cargo.lock [lockfile/generated diff collapsed: +40 / -40 lines]"));
        assert!(diff_out.contains("+new()"));
        assert!(!diff_out.contains("index aaaa..bbbb"));
    }

    #[test]
    fn condenses_package_installs_and_docker_buildkit_preserving_errors() {
        let mut npm = String::from("exit_code: 0\n");
        for i in 0..20 {
            npm.push_str(&format!("GET https://registry.npmjs.org/pkg-{i} 200\n"));
        }
        npm.push_str("npm WARN deprecated foo@1.0.0: migrate to v2\n");
        npm.push_str("added 412 packages, and audited 413 packages in 6s\n");
        let npm_out = shape_package_install(&npm).expect("npm shaped");
        assert!(npm_out.contains("+20 package install progress/tree lines collapsed"));
        assert!(npm_out.contains("npm WARN deprecated"));
        assert!(npm_out.contains("added 412 packages"));

        let mut docker = String::from("exit_code: 1\n");
        for i in 1..=12 {
            docker.push_str(&format!("#{i} [builder {i}/12] RUN step {i}\n#{i} CACHED\n"));
        }
        docker.push_str("#13 => ERROR [builder 13/13] RUN cargo build\n");
        let docker_out = shape_container_or_k8s(&docker).expect("docker shaped");
        assert!(docker_out.contains("12 cached layer(s) collapsed"));
        assert!(docker_out.contains("=> ERROR [builder 13/13]"));
    }
}
