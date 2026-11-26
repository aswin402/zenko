//! Native Rust Tier-Aware Shunt Context-Protection Gate
//! (inspired by Spotify `portal-ai-plugins/shunt`).
//!
//! Pure deterministic logic — zero regex, zero `if/else` ladders, declarative
//! binary table (`SHUNT_SHELL_BIN_SPECS`), explicit shell-argument FSM,
//! kernel-confined `openat(O_NOFOLLOW)` bounded scanning, strictly fail-open,
//! and "Never Worse" byte-savings verified before intercepting.

use crate::orchestrator::router::model_slot_rank;
use crate::policy::{segment_bin_name, strip_shell_wrappers, tokenize_shell};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use zenko_core::{
    ARTIFACT_PATH_PREFIX, DEFAULT_READ_FILE_LINE_WINDOW, OUTLINE_MAX_FILE_BYTES,
    READ_FILE_MODE_OUTLINE, SHUNT_BYTES_PER_TOKEN_ESTIMATE, SHUNT_FALLBACK_READ_WINDOW_LINES,
    SHUNT_LINE_PREFIX_ESTIMATE_BYTES, SHUNT_MAX_SCAN_BYTES, SHUNT_MIN_OUTPUT_CAP_BYTES,
    SHUNT_OUTLINE_MAX_SYMBOLS, SHUNT_SCAN_BUF_BYTES, SHUNT_SHELL_DEFAULT_SLICE_LINES,
    SHUNT_SHELL_END_OF_OPTIONS, SHUNT_SHELL_GLOB_CHARS, SHUNT_SHELL_LINES_LONG_EQ,
    SHUNT_SHELL_LINES_LONG_FLAG, SHUNT_SHELL_LINES_SHORT_FLAG, SHUNT_SHELL_STDIN_OPERAND,
    SHUNT_SHELL_VALUE_FLAGS, SUBAGENT_ROLE_EXPLORER, ShuntMode, ShuntShellBinBehavior,
    ShuntSourceKind, SubagentModelTier, TOOL_READ_FILE, TOOL_SHELL_EXEC, ZenkoConfig,
    find_shunt_shell_bin, find_subagent_role,
};
use zenko_tools::jail::{
    open_jailed_read_file, read_jailed_string_bounded, resolve_readable_path, select_jail_root,
};
use zenko_tools::outline::{extract_outline, format_outline_report};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellSliceBound {
    FullFile,
    ExplicitLines(usize),
    FromStartLine(usize),
}

impl ShellSliceBound {
    /// Deterministic arithmetic calculation of lines the command would emit for a file of `total_lines`.
    pub fn estimated_output_lines(self, total_lines: usize) -> usize {
        match self {
            Self::FullFile => total_lines,
            Self::ExplicitLines(n) => n.min(total_lines),
            Self::FromStartLine(start_line) => {
                total_lines.saturating_sub(start_line.saturating_sub(1))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellBulkReadCandidate {
    pub bin: String,
    pub path: String,
    pub bound: ShellSliceBound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShuntOutcome {
    pub target_path: String,
    pub source_kind: ShuntSourceKind,
    pub total_lines: usize,
    pub estimated_output_lines: usize,
    pub original_bytes: usize,
    pub returned_bytes: usize,
    pub tokens_saved: usize,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShuntDecision {
    PassThrough,
    Advise {
        target_path: String,
        source_kind: ShuntSourceKind,
        total_lines: usize,
        note: String,
    },
    Redirect(ShuntOutcome),
}

/// Returns `true` when the active session is already running on a cheap `Fast` model or
/// `Fast`-tier subagent (`explorer`, `researcher`, `drafter`), so Shunt yields automatically.
pub fn is_fast_tier_context(
    cfg: &ZenkoConfig,
    active_model: Option<&str>,
    role_id: Option<&str>,
) -> bool {
    if role_id
        .and_then(find_subagent_role)
        .is_some_and(|r| r.model_tier == SubagentModelTier::Fast)
    {
        return true;
    }
    let fast_configured = !cfg.provider.fast_model.trim().is_empty();
    match (
        fast_configured,
        active_model.map(str::trim).filter(|s| !s.is_empty()),
    ) {
        (true, Some(model)) => model_slot_rank(cfg, model) == 0,
        _ => false,
    }
}

fn has_distinct_fast_model(cfg: &ZenkoConfig, active_model: Option<&str>) -> bool {
    let fast = cfg.provider.fast_model.trim();
    !fast.is_empty() && active_model.map(str::trim) != Some(fast)
}

fn parse_line_count_spec(behavior: ShuntShellBinBehavior, raw: &str) -> Option<ShellSliceBound> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    match (behavior, trimmed.strip_prefix('+')) {
        (ShuntShellBinBehavior::TailSlice, Some(rest)) => rest
            .parse::<usize>()
            .ok()
            .map(ShellSliceBound::FromStartLine),
        (_, Some(rest)) => rest
            .parse::<usize>()
            .ok()
            .map(ShellSliceBound::ExplicitLines),
        (_, None) => trimmed
            .strip_prefix('-')
            .unwrap_or(trimmed)
            .parse::<usize>()
            .ok()
            .map(ShellSliceBound::ExplicitLines),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellArgLexState {
    Options,
    OperandsOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellArgToken<'a> {
    EndOfOptions,
    Stdin,
    SliceFlagSeparate,
    SliceFlagInline(ShellSliceBound),
    OtherValueFlag,
    OtherFlag,
    Operand(&'a str),
}

fn try_parse_inline_slice_bound(
    behavior: ShuntShellBinBehavior,
    tok: &str,
) -> Option<ShellSliceBound> {
    if !matches!(
        behavior,
        ShuntShellBinBehavior::HeadSlice | ShuntShellBinBehavior::TailSlice
    ) {
        return None;
    }
    tok.strip_prefix(SHUNT_SHELL_LINES_LONG_EQ)
        .and_then(|v| parse_line_count_spec(behavior, v))
        .or_else(|| {
            tok.strip_prefix(SHUNT_SHELL_LINES_SHORT_FLAG)
                .filter(|v| !v.is_empty())
                .and_then(|v| parse_line_count_spec(behavior, v))
        })
        .or_else(|| {
            tok.strip_prefix('-')
                .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|d| parse_line_count_spec(behavior, d))
        })
}

fn classify_shell_arg_token(behavior: ShuntShellBinBehavior, tok: &str) -> ShellArgToken<'_> {
    let is_slice_bin = matches!(
        behavior,
        ShuntShellBinBehavior::HeadSlice | ShuntShellBinBehavior::TailSlice
    );
    match tok {
        SHUNT_SHELL_END_OF_OPTIONS => ShellArgToken::EndOfOptions,
        SHUNT_SHELL_STDIN_OPERAND => ShellArgToken::Stdin,
        t if is_slice_bin
            && (t == SHUNT_SHELL_LINES_SHORT_FLAG || t == SHUNT_SHELL_LINES_LONG_FLAG) =>
        {
            ShellArgToken::SliceFlagSeparate
        }
        t => match try_parse_inline_slice_bound(behavior, t) {
            Some(bound) => ShellArgToken::SliceFlagInline(bound),
            None => match (
                SHUNT_SHELL_VALUE_FLAGS.contains(&t),
                t.starts_with('-'),
            ) {
                (true, _) => ShellArgToken::OtherValueFlag,
                (false, true) => ShellArgToken::OtherFlag,
                (false, false) => ShellArgToken::Operand(t),
            },
        },
    }
}

/// Deterministic zero-regex detector for unpiped shell commands that dump large files
/// (`cat`/`less`/`more`/`nl`/`bat`, or `head`/`tail` with large/open-ended line bounds).
/// Piped targeted filters (e.g. `cat big.rs | grep foo`) return `None` immediately.
pub fn detect_shell_bulk_read(command: &str) -> Option<ShellBulkReadCandidate> {
    let parsed = tokenize_shell(command);
    if parsed.has_control_ops || parsed.tokens.is_empty() {
        return None;
    }
    let tokens = strip_shell_wrappers(&parsed.tokens);
    let first = tokens.first()?;
    let bin = segment_bin_name(first);
    let behavior = find_shunt_shell_bin(&bin)?;

    let mut bound = match behavior {
        ShuntShellBinBehavior::FullDump => ShellSliceBound::FullFile,
        ShuntShellBinBehavior::HeadSlice | ShuntShellBinBehavior::TailSlice => {
            ShellSliceBound::ExplicitLines(SHUNT_SHELL_DEFAULT_SLICE_LINES)
        }
    };

    let mut operands: Vec<&str> = Vec::new();
    let mut lex_state = ShellArgLexState::Options;
    let mut i = 1usize;
    while i < tokens.len() {
        let tok = tokens[i].as_str();
        let step = match lex_state {
            ShellArgLexState::OperandsOnly => {
                if tok != SHUNT_SHELL_STDIN_OPERAND {
                    operands.push(tok);
                }
                1
            }
            ShellArgLexState::Options => match classify_shell_arg_token(behavior, tok) {
                ShellArgToken::EndOfOptions => {
                    lex_state = ShellArgLexState::OperandsOnly;
                    1
                }
                ShellArgToken::Stdin | ShellArgToken::OtherFlag => 1,
                ShellArgToken::SliceFlagSeparate => {
                    if let Some(val) = tokens.get(i + 1)
                        && let Some(parsed_bound) = parse_line_count_spec(behavior, val)
                    {
                        bound = parsed_bound;
                    }
                    2
                }
                ShellArgToken::SliceFlagInline(parsed_bound) => {
                    bound = parsed_bound;
                    1
                }
                ShellArgToken::OtherValueFlag => 2,
                ShellArgToken::Operand(op) => {
                    operands.push(op);
                    1
                }
            },
        };
        i = i.saturating_add(step).min(tokens.len());
    }

    // Only intercept single-file direct dumps (multi-file / empty / glob patterns pass through safely)
    if operands.len() != 1 {
        return None;
    }
    let path = operands[0];
    if path.contains(SHUNT_SHELL_GLOB_CHARS) {
        return None;
    }

    Some(ShellBulkReadCandidate {
        bin,
        path: path.to_string(),
        bound,
    })
}

/// Counts total lines and byte size of a jailed file via fixed-size streaming reads over
/// `openat(O_NOFOLLOW | O_CLOEXEC)` (zero full-file heap allocation, fails open on error).
fn count_lines_jailed_bounded(root: &Path, abs_path: &Path) -> Option<(usize, u64)> {
    let mut file = open_jailed_read_file(root, abs_path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > SHUNT_MAX_SCAN_BYTES {
        return None;
    }
    let mut buf = [0u8; SHUNT_SCAN_BUF_BYTES];
    let mut newlines = 0usize;
    let mut total_read = 0u64;
    let mut last_byte = 0u8;
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        newlines += buf[..n].iter().filter(|&&b| b == b'\n').count();
        last_byte = buf[n - 1];
        total_read = total_read.saturating_add(n as u64);
        if total_read > SHUNT_MAX_SCAN_BYTES {
            return None;
        }
    }
    let total_lines = match (total_read > 0, last_byte == b'\n') {
        (false, _) => 0,
        (true, true) => newlines,
        (true, false) => newlines.saturating_add(1),
    };
    Some((total_lines, meta.len()))
}

fn is_unconstrained_read_file(args: &Value) -> bool {
    let raw_bypass = args.get("raw").and_then(Value::as_bool).unwrap_or(false);
    if raw_bypass {
        return false;
    }
    let is_outline = args
        .get("mode")
        .and_then(Value::as_str)
        .is_some_and(|m| m.trim().eq_ignore_ascii_case(READ_FILE_MODE_OUTLINE));
    let has_symbol = args
        .get("symbol")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty())
        || args
            .get("symbols")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty());
    let has_window = args.get("start_line").is_some()
        || args.get("end_line").is_some()
        || args.get("ranges").is_some();
    !(is_outline || has_symbol || has_window)
}

fn resolve_shunt_target_path(
    workspace_dir: &Path,
    hub_dir: &Path,
    data_dir: &Path,
    req_path: &str,
    shell_cwd: Option<&str>,
) -> Option<(PathBuf, PathBuf)> {
    let trimmed = req_path.trim();
    if trimmed.is_empty() || trimmed.starts_with(ARTIFACT_PATH_PREFIX) {
        return None;
    }
    let effective_ws = match shell_cwd.map(str::trim).filter(|s| !s.is_empty()) {
        Some(rel_cwd) => {
            zenko_tools::jail::resolve_workspace_or_hub_path(workspace_dir, hub_dir, rel_cwd, true)
                .ok()?
        }
        None => workspace_dir.to_path_buf(),
    };
    let abs = resolve_readable_path(&effective_ws, hub_dir, data_dir, trimmed, true).ok()?;
    let root = select_jail_root(&abs, workspace_dir, hub_dir, Some(data_dir))?;
    Some((root, abs))
}

fn build_shunt_hint_footer(
    req_path: &str,
    total_lines: usize,
    min_lines: usize,
    has_fast_worker: bool,
    source_kind: ShuntSourceKind,
) -> String {
    let explorer_clause = match has_fast_worker {
        true => format!(
            " or delegate broad analysis to `spawn_agent(role=\"{SUBAGENT_ROLE_EXPLORER}\", ...)` (runs on `fast_model`)"
        ),
        false => String::new(),
    };
    let bypass_clause = match source_kind {
        ShuntSourceKind::ShellRead => format!(
            "use `read_file(path=\"{req_path}\", symbol=\"...\")` / `ranges`, pipe through `grep`/`rg`, or pass `raw: true`"
        ),
        ShuntSourceKind::ReadFile => format!(
            "read targeted spans via `read_file(path=\"{req_path}\", symbol=\"...\")` or `ranges: [{{start_line, end_line}}]` (or pass `raw: true` for raw lines)"
        ),
    };
    format!(
        "[shunt: `{req_path}` has {total_lines} lines (threshold {min_lines}) — {bypass_clause}{explorer_clause}]"
    )
}

/// Evaluates Zenko's tier-aware Shunt gate before executing `read_file` or `shell_exec`.
/// Strictly fail-open: any non-matching command, fast-tier session, explicit window/symbol/raw
/// request, `artifact:` handle, or I/O mismatch returns `ShuntDecision::PassThrough`.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_shunt_pre_tool(
    cfg: &ZenkoConfig,
    workspace_dir: &Path,
    hub_dir: &Path,
    data_dir: &Path,
    active_model: Option<&str>,
    role_id: Option<&str>,
    tool_name: &str,
    args: &Value,
) -> ShuntDecision {
    let shunt_cfg = &cfg.economy.shunt;
    if !cfg.economy.enabled
        || !shunt_cfg.enabled
        || !shunt_cfg.mode.is_active()
        || is_fast_tier_context(cfg, active_model, role_id)
    {
        return ShuntDecision::PassThrough;
    }

    let min_lines = shunt_cfg.min_lines.max(1);
    let has_fast_worker = has_distinct_fast_model(cfg, active_model);

    match tool_name {
        TOOL_READ_FILE => {
            if !is_unconstrained_read_file(args) {
                return ShuntDecision::PassThrough;
            }
            let Some(req_path) = args.get("path").and_then(Value::as_str).map(str::trim) else {
                return ShuntDecision::PassThrough;
            };
            let Some((root, abs_path)) =
                resolve_shunt_target_path(workspace_dir, hub_dir, data_dir, req_path, None)
            else {
                return ShuntDecision::PassThrough;
            };
            let Some((total_lines, file_bytes)) = count_lines_jailed_bounded(&root, &abs_path)
            else {
                return ShuntDecision::PassThrough;
            };
            if total_lines < min_lines {
                return ShuntDecision::PassThrough;
            }

            let footer = build_shunt_hint_footer(
                req_path,
                total_lines,
                min_lines,
                has_fast_worker,
                ShuntSourceKind::ReadFile,
            );

            match shunt_cfg.mode {
                ShuntMode::Off => ShuntDecision::PassThrough,
                ShuntMode::Advise => ShuntDecision::Advise {
                    target_path: req_path.to_string(),
                    source_kind: ShuntSourceKind::ReadFile,
                    total_lines,
                    note: footer,
                },
                ShuntMode::Redirect => {
                    // Fail open if file is too large to outline or has zero structural symbols
                    // (e.g. plain prose/logs/CSV), so we NEVER return an empty useless outline.
                    if file_bytes > OUTLINE_MAX_FILE_BYTES {
                        return ShuntDecision::PassThrough;
                    }
                    let Ok(raw) = read_jailed_string_bounded(
                        &root,
                        &abs_path,
                        OUTLINE_MAX_FILE_BYTES,
                        req_path,
                    ) else {
                        return ShuntDecision::PassThrough;
                    };
                    let report = extract_outline(&abs_path, &raw, SHUNT_OUTLINE_MAX_SYMBOLS);
                    if report.entries.is_empty() {
                        return ShuntDecision::PassThrough;
                    }

                    let outline_body = format_outline_report(req_path, &report);
                    let content = format!("{outline_body}\n{footer}");
                    let window_lines = total_lines.min(DEFAULT_READ_FILE_LINE_WINDOW);
                    let avg_line_bytes = (file_bytes as usize).div_ceil(total_lines.max(1));
                    let estimated_raw_bytes = window_lines
                        .saturating_mul(avg_line_bytes.saturating_add(SHUNT_LINE_PREFIX_ESTIMATE_BYTES));
                    if content.len() >= estimated_raw_bytes {
                        return ShuntDecision::PassThrough;
                    }
                    let tokens_saved = estimated_raw_bytes
                        .saturating_sub(content.len())
                        .div_ceil(SHUNT_BYTES_PER_TOKEN_ESTIMATE);
                    ShuntDecision::Redirect(ShuntOutcome {
                        target_path: req_path.to_string(),
                        source_kind: ShuntSourceKind::ReadFile,
                        total_lines,
                        estimated_output_lines: window_lines,
                        original_bytes: estimated_raw_bytes,
                        returned_bytes: content.len(),
                        tokens_saved,
                        content,
                    })
                }
            }
        }
        TOOL_SHELL_EXEC => {
            if args.get("raw").and_then(Value::as_bool).unwrap_or(false)
                || args.get("session").is_some()
                || args.get("pty_action").is_some()
            {
                return ShuntDecision::PassThrough;
            }
            let Some(cmd) = args.get("command").and_then(Value::as_str) else {
                return ShuntDecision::PassThrough;
            };
            let Some(cand) = detect_shell_bulk_read(cmd) else {
                return ShuntDecision::PassThrough;
            };
            let cwd = args.get("cwd").and_then(Value::as_str);
            let Some((root, abs_path)) =
                resolve_shunt_target_path(workspace_dir, hub_dir, data_dir, &cand.path, cwd)
            else {
                return ShuntDecision::PassThrough;
            };
            let Some((total_lines, file_bytes)) = count_lines_jailed_bounded(&root, &abs_path)
            else {
                return ShuntDecision::PassThrough;
            };
            let estimated_lines = cand.bound.estimated_output_lines(total_lines);
            if estimated_lines < min_lines {
                return ShuntDecision::PassThrough;
            }

            let footer = build_shunt_hint_footer(
                &cand.path,
                total_lines,
                min_lines,
                has_fast_worker,
                ShuntSourceKind::ShellRead,
            );

            match shunt_cfg.mode {
                ShuntMode::Off => ShuntDecision::PassThrough,
                ShuntMode::Advise => ShuntDecision::Advise {
                    target_path: cand.path,
                    source_kind: ShuntSourceKind::ShellRead,
                    total_lines,
                    note: footer,
                },
                ShuntMode::Redirect => {
                    let outline_section = match file_bytes <= OUTLINE_MAX_FILE_BYTES {
                        true => read_jailed_string_bounded(
                            &root,
                            &abs_path,
                            OUTLINE_MAX_FILE_BYTES,
                            &cand.path,
                        )
                        .ok()
                        .and_then(|raw| {
                            let rep = extract_outline(&abs_path, &raw, SHUNT_OUTLINE_MAX_SYMBOLS);
                            match rep.entries.is_empty() {
                                true => None,
                                false => Some(format_outline_report(&cand.path, &rep)),
                            }
                        }),
                        false => None,
                    };

                    let content = match outline_section {
                        Some(outline) => format!(
                            "Shunt intercepted unpiped `{}` dump of `{}` ({total_lines} lines, {file_bytes} B) and returned its structural outline instead:\n\n{outline}\n{footer}",
                            cand.bin, cand.path
                        ),
                        None => format!(
                            "Shunt intercepted unpiped `{}` dump of `{}` ({total_lines} lines, {file_bytes} B).\nUse `code_search(path=\"{}\", pattern=\"...\")` or `read_file(path=\"{}\", start_line=1, end_line={SHUNT_FALLBACK_READ_WINDOW_LINES})` to inspect only needed sections.\n{footer}",
                            cand.bin, cand.path, cand.path, cand.path
                        ),
                    };

                    let scaled_bytes = ((file_bytes as u128)
                        .saturating_mul(estimated_lines as u128)
                        / (total_lines.max(1) as u128))
                        as usize;
                    let avoided_bytes =
                        scaled_bytes.min(cfg.policy.max_output_bytes.max(SHUNT_MIN_OUTPUT_CAP_BYTES));
                    if content.len() >= avoided_bytes {
                        return ShuntDecision::PassThrough;
                    }
                    let tokens_saved = avoided_bytes
                        .saturating_sub(content.len())
                        .div_ceil(SHUNT_BYTES_PER_TOKEN_ESTIMATE);
                    ShuntDecision::Redirect(ShuntOutcome {
                        target_path: cand.path,
                        source_kind: ShuntSourceKind::ShellRead,
                        total_lines,
                        estimated_output_lines: estimated_lines,
                        original_bytes: avoided_bytes,
                        returned_bytes: content.len(),
                        tokens_saved,
                        content,
                    })
                }
            }
        }
        _ => ShuntDecision::PassThrough,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_large_rust_file(lines: usize) -> String {
        let mut s = String::new();
        for i in 0..lines / 5 {
            s.push_str(&format!(
                "pub fn helper_{i}(x: usize) -> usize {{\n    let a = x + {i};\n    let b = a * 2;\n    b + 1\n}}\n"
            ));
        }
        s
    }

    #[test]
    fn shell_bulk_read_detector_distinguishes_dumps_pipes_and_slices() {
        let cat = detect_shell_bulk_read("cat src/big.rs").expect("cat detected");
        assert_eq!(cat.bin, "cat");
        assert_eq!(cat.path, "src/big.rs");
        assert_eq!(cat.bound, ShellSliceBound::FullFile);

        let wrapped = detect_shell_bulk_read("env FOO=1 nl -ba src/big.rs").expect("nl detected");
        assert_eq!(wrapped.bin, "nl");
        assert_eq!(wrapped.path, "src/big.rs");
        assert_eq!(wrapped.bound, ShellSliceBound::FullFile);

        // Piped targeted read MUST pass through untouched
        assert!(detect_shell_bulk_read("cat src/big.rs | grep helper").is_none());
        assert!(detect_shell_bulk_read("cat a.rs > b.rs").is_none());

        // Small `head` / `tail` slices compute exact bounded output lines
        let small_head = detect_shell_bulk_read("head -n 40 src/big.rs").expect("head");
        assert_eq!(small_head.bound.estimated_output_lines(500), 40);

        let big_head = detect_shell_bulk_read("head -500 src/big.rs").expect("big head");
        assert_eq!(big_head.bound.estimated_output_lines(600), 500);

        let open_tail = detect_shell_bulk_read("tail -n +1 src/big.rs").expect("open tail");
        assert_eq!(open_tail.bound.estimated_output_lines(600), 600);

        let small_tail_offset =
            detect_shell_bulk_read("tail -n +550 src/big.rs").expect("small tail offset");
        assert_eq!(small_tail_offset.bound.estimated_output_lines(600), 51);
    }

    #[test]
    fn shunt_gate_redirects_frontier_model_and_yields_to_fast_tier_and_explicit_options() {
        let base = std::env::temp_dir().join(format!("zenko_shunt_test_{}", zenko_core::new_id()));
        let ws = base.join("ws");
        let hub = base.join("hub");
        let data = base.join("data");
        let _ = std::fs::create_dir_all(&ws);
        let _ = std::fs::create_dir_all(&hub);
        let _ = std::fs::create_dir_all(&data);

        let big_rs = make_large_rust_file(500);
        let _ = std::fs::write(ws.join("big.rs"), &big_rs);
        let _ = std::fs::write(ws.join("small.rs"), "pub fn tiny() {}\n");
        let plain_log = (0..500)
            .map(|i| format!("2026-11-26 info line {i} payload"))
            .collect::<Vec<_>>()
            .join("\n");
        let _ = std::fs::write(ws.join("plain.log"), &plain_log);

        let mut cfg = ZenkoConfig::default();
        cfg.provider.lead_model = "anthropic/claude-sonnet-4".into();
        cfg.provider.fast_model = "google/gemini-2.5-flash".into();

        // 1. Lead model unconstrained read_file on 500-line Rust file -> Redirect with outline + savings
        let dec = evaluate_shunt_pre_tool(
            &cfg,
            &ws,
            &hub,
            &data,
            Some("anthropic/claude-sonnet-4"),
            None,
            TOOL_READ_FILE,
            &json!({ "path": "big.rs" }),
        );
        match dec {
            ShuntDecision::Redirect(out) => {
                assert_eq!(out.source_kind, ShuntSourceKind::ReadFile);
                assert!(out.tokens_saved > 0);
                assert!(out.content.contains("helper_0"));
                assert!(out.content.contains("spawn_agent(role=\"explorer\""));
            }
            other => panic!("expected Redirect on large unconstrained read_file, got {other:?}"),
        }

        // 2. Explicit `raw: true`, `start_line`, or `symbol` MUST pass through
        for args in [
            json!({ "path": "big.rs", "raw": true }),
            json!({ "path": "big.rs", "start_line": 1, "end_line": 100 }),
            json!({ "path": "big.rs", "symbol": "helper_0" }),
            json!({ "path": "small.rs" }),
        ] {
            assert_eq!(
                evaluate_shunt_pre_tool(
                    &cfg,
                    &ws,
                    &hub,
                    &data,
                    Some("anthropic/claude-sonnet-4"),
                    None,
                    TOOL_READ_FILE,
                    &args,
                ),
                ShuntDecision::PassThrough
            );
        }

        // 3. Fast model or Fast-tier subagent (`explorer` / `drafter`) MUST pass through automatically
        assert_eq!(
            evaluate_shunt_pre_tool(
                &cfg,
                &ws,
                &hub,
                &data,
                Some("google/gemini-2.5-flash"),
                None,
                TOOL_READ_FILE,
                &json!({ "path": "big.rs" }),
            ),
            ShuntDecision::PassThrough
        );
        assert_eq!(
            evaluate_shunt_pre_tool(
                &cfg,
                &ws,
                &hub,
                &data,
                Some("anthropic/claude-sonnet-4"),
                Some(SUBAGENT_ROLE_EXPLORER),
                TOOL_READ_FILE,
                &json!({ "path": "big.rs" }),
            ),
            ShuntDecision::PassThrough
        );

        // 4. Zero-symbol plain.log on `read_file` fails open (never returns empty outline),
        //    whereas unpiped `cat plain.log` DOES get shunted cleanly!
        assert_eq!(
            evaluate_shunt_pre_tool(
                &cfg,
                &ws,
                &hub,
                &data,
                Some("anthropic/claude-sonnet-4"),
                None,
                TOOL_READ_FILE,
                &json!({ "path": "plain.log" }),
            ),
            ShuntDecision::PassThrough
        );
        match evaluate_shunt_pre_tool(
            &cfg,
            &ws,
            &hub,
            &data,
            Some("anthropic/claude-sonnet-4"),
            None,
            TOOL_SHELL_EXEC,
            &json!({ "command": "cat plain.log" }),
        ) {
            ShuntDecision::Redirect(out) => {
                assert_eq!(out.source_kind, ShuntSourceKind::ShellRead);
                assert!(out.tokens_saved > 0);
                assert!(out.content.contains("code_search"));
            }
            other => panic!("expected Redirect on unpiped `cat plain.log`, got {other:?}"),
        }

        // 5. Piped `cat big.rs | grep helper_1` & bounded `head -n 20 big.rs` pass through
        for cmd in ["cat big.rs | grep helper_1", "head -n 20 big.rs"] {
            assert_eq!(
                evaluate_shunt_pre_tool(
                    &cfg,
                    &ws,
                    &hub,
                    &data,
                    Some("anthropic/claude-sonnet-4"),
                    None,
                    TOOL_SHELL_EXEC,
                    &json!({ "command": cmd }),
                ),
                ShuntDecision::PassThrough
            );
        }

        let _ = std::fs::remove_dir_all(&base);
    }
}
