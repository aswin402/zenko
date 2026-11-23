//! Structured native Rust Git tool (`git_tool`) invoking `git` directly via `argv`
//! (zero `/bin/sh`, zero quoting/injection hazards, strict Plan Mode read/write separation).

use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    GIT_ACTIONS, GIT_SENSITIVE_FILE_PATTERNS, GIT_TOOL_DEFAULT_BLAME_LINES,
    GIT_TOOL_DEFAULT_LOG_LIMIT, GIT_TOOL_MAX_LOG_LIMIT, GIT_TOOL_MAX_OUTPUT_CHARS,
    GIT_TOOL_TIMEOUT_SECS, GitAction, ResourceLockKey, Result, RiskLevel, TOOL_GIT, Tool,
    ToolConcurrency, ToolContext, ToolExposure, ToolOutput, ToolSpec, ZenkoError,
    is_git_tool_args_mutating,
};

pub struct GitTool;

#[async_trait]
impl Tool for GitTool {
    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        match is_git_tool_args_mutating(args) {
            false => ToolConcurrency::Shared,
            true => ToolConcurrency::Keyed(ResourceLockKey::Git),
        }
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn spec(&self) -> ToolSpec {
        let action_enum: Vec<Value> = GIT_ACTIONS
            .iter()
            .map(|&a| Value::String(a.to_string()))
            .collect();
        ToolSpec {
            name: TOOL_GIT.into(),
            description: "Structured native Git inspection & workflow tool (zero shell quoting). Read-only actions (`status`, `diff`, `log`, `blame`, `show`, `branch` list) work in Plan Mode; mutating actions (`stage`, `unstage`, `commit`, `branch` create/checkout) require approval/checkpoint.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": action_enum,
                        "description": "Git operation to execute."
                    },
                    "staged": { "type": "boolean", "description": "For `diff`: inspect staged (`--cached`) changes." },
                    "stat_only": { "type": "boolean", "description": "For `diff`/`show`: return compact `--stat` summary only." },
                    "rev": { "type": "string", "description": "Commit SHA, branch, or range (e.g. `HEAD~1..HEAD`, `main`)." },
                    "path": { "type": "string", "description": "Optional single file path filter (for `diff`, `log`, `blame`, `show`, `stage`, `unstage`)." },
                    "paths": { "type": "array", "items": { "type": "string" }, "description": "Paths to pass to `diff`, `stage`, or `unstage`." },
                    "limit": { "type": "integer", "description": "Max commits for `log` (default 20, cap 100)." },
                    "start_line": { "type": "integer", "description": "Optional 1-based start line for `blame`." },
                    "end_line": { "type": "integer", "description": "Optional 1-based end line for `blame`." },
                    "branch_action": { "type": "string", "enum": ["list", "create", "checkout"], "description": "Sub-action for `branch` (default `list`)." },
                    "name": { "type": "string", "description": "Branch name for `branch` create/checkout." },
                    "message": { "type": "string", "description": "Commit message for `commit`." },
                    "allow_sensitive": { "type": "boolean", "description": "For `stage`: explicitly allow staging credential/env files (default false)." }
                },
                "required": ["action"]
            }),
            risk: RiskLevel::Moderate,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let raw_action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `action`".into()))?;
        let action = GitAction::parse(raw_action).ok_or_else(|| {
            ZenkoError::Tool(format!(
                "Unknown `git_tool` action `{raw_action}`. Valid actions: {}.",
                GIT_ACTIONS.join(", ")
            ))
        })?;

        let ws = &ctx.workspace_dir;
        match action {
            GitAction::Status => exec_status(ws, cancel).await,
            GitAction::Diff => exec_diff(ws, &args, cancel).await,
            GitAction::Log => exec_log(ws, &args, cancel).await,
            GitAction::Blame => exec_blame(ws, &args, cancel).await,
            GitAction::Show => exec_show(ws, &args, cancel).await,
            GitAction::Branch => exec_branch(ws, &args, cancel).await,
            GitAction::Stage => exec_stage(ws, &args, cancel).await,
            GitAction::Unstage => exec_unstage(ws, &args, cancel).await,
            GitAction::Commit => exec_commit(ws, &args, cancel).await,
        }
    }
}

async fn exec_status(ws: &Path, cancel: CancellationToken) -> Result<ToolOutput> {
    let (ok, stdout, stderr) = run_git(ws, &["status", "--porcelain=v1", "-b"], cancel).await?;
    if !ok {
        return Ok(ToolOutput::err(format!("git status failed: {}", stderr.trim())));
    }
    let mut branch = String::new();
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    let mut untracked = Vec::new();

    for line in stdout.lines() {
        if let Some(hdr) = line.strip_prefix("## ") {
            branch = hdr.trim().to_string();
            continue;
        }
        if line.len() < 3 {
            continue;
        }
        let bytes = line.as_bytes();
        let x = bytes[0] as char;
        let y = bytes[1] as char;
        let path = line[3..].trim().to_string();
        match (x, y) {
            ('?', '?') => untracked.push(path),
            _ => {
                if x != ' ' {
                    staged.push(format!("{x} {path}"));
                }
                if y != ' ' {
                    unstaged.push(format!("{y} {path}"));
                }
            }
        }
    }

    let summary = format!(
        "Branch: {}\nStaged: {} | Modified: {} | Untracked: {}\n{}",
        match branch.is_empty() {
            true => "(detached)",
            false => branch.as_str(),
        },
        staged.len(),
        unstaged.len(),
        untracked.len(),
        match stdout.trim().is_empty() {
            true => "(clean working tree)".to_string(),
            false => truncate_chars(stdout.trim(), GIT_TOOL_MAX_OUTPUT_CHARS),
        }
    );
    let mut out = ToolOutput::ok(summary);
    out.metadata = Some(json!({
        "branch": branch,
        "staged": staged,
        "unstaged": unstaged,
        "untracked": untracked,
    }));
    Ok(out)
}

async fn exec_diff(ws: &Path, args: &Value, cancel: CancellationToken) -> Result<ToolOutput> {
    let mut argv: Vec<String> = vec!["diff".into(), "--no-color".into()];
    if args.get("staged").and_then(Value::as_bool).unwrap_or(false) {
        argv.push("--cached".into());
    }
    if args
        .get("stat_only")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        argv.push("--stat".into());
    }
    if let Some(rev) = opt_safe_rev(args.get("rev"))? {
        argv.push(rev);
    }
    let paths = collect_safe_paths(args)?;
    if !paths.is_empty() {
        argv.push("--".into());
        argv.extend(paths);
    }
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let (ok, stdout, stderr) = run_git(ws, &refs, cancel).await?;
    if !ok {
        return Ok(ToolOutput::err(format!("git diff failed: {}", stderr.trim())));
    }
    let text = match stdout.trim().is_empty() {
        true => "(no diff)".to_string(),
        false => truncate_chars(&stdout, GIT_TOOL_MAX_OUTPUT_CHARS),
    };
    Ok(ToolOutput::ok(text))
}

async fn exec_log(ws: &Path, args: &Value, cancel: CancellationToken) -> Result<ToolOutput> {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(GIT_TOOL_DEFAULT_LOG_LIMIT as u64)
        .clamp(1, GIT_TOOL_MAX_LOG_LIMIT as u64);
    let mut argv: Vec<String> = vec![
        "log".into(),
        format!("-n{limit}"),
        "--date=short".into(),
        "--pretty=format:%H%x1f%h%x1f%an%x1f%ad%x1f%s".into(),
    ];
    if let Some(rev) = opt_safe_rev(args.get("rev"))? {
        argv.push(rev);
    }
    let paths = collect_safe_paths(args)?;
    if !paths.is_empty() {
        argv.push("--".into());
        argv.extend(paths);
    }
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let (ok, stdout, stderr) = run_git(ws, &refs, cancel).await?;
    if !ok {
        return Ok(ToolOutput::err(format!("git log failed: {}", stderr.trim())));
    }
    let commits = parse_git_log_records(&stdout);
    let formatted: Vec<String> = commits
        .iter()
        .map(|c| {
            format!(
                "{}  {}  {}  {}",
                c["short_sha"].as_str().unwrap_or(""),
                c["date"].as_str().unwrap_or(""),
                c["author"].as_str().unwrap_or(""),
                c["subject"].as_str().unwrap_or(""),
            )
        })
        .collect();
    let mut out = ToolOutput::ok(match formatted.is_empty() {
        true => "(no commits)".to_string(),
        false => formatted.join("\n"),
    });
    out.metadata = Some(json!({ "commits": commits }));
    Ok(out)
}

async fn exec_blame(ws: &Path, args: &Value, cancel: CancellationToken) -> Result<ToolOutput> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ZenkoError::Tool("`path` is required for `git_tool(action=\"blame\")`".into()))?;
    validate_rel_git_path(path)?;

    let mut argv: Vec<String> = vec!["blame".into(), "--date=short".into()];
    if let Some(start) = args.get("start_line").and_then(Value::as_u64) {
        let start = start.max(1);
        let end = args
            .get("end_line")
            .and_then(Value::as_u64)
            .unwrap_or(start.saturating_add(GIT_TOOL_DEFAULT_BLAME_LINES))
            .max(start);
        argv.push(format!("-L{start},{end}"));
    }
    if let Some(rev) = opt_safe_rev(args.get("rev"))? {
        argv.push(rev);
    }
    argv.push("--".into());
    argv.push(path.to_string());

    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let (ok, stdout, stderr) = run_git(ws, &refs, cancel).await?;
    if !ok {
        return Ok(ToolOutput::err(format!("git blame failed: {}", stderr.trim())));
    }
    Ok(ToolOutput::ok(truncate_chars(&stdout, GIT_TOOL_MAX_OUTPUT_CHARS)))
}

async fn exec_show(ws: &Path, args: &Value, cancel: CancellationToken) -> Result<ToolOutput> {
    let rev = opt_safe_rev(args.get("rev"))?.unwrap_or_else(|| "HEAD".into());
    let mut argv: Vec<String> = vec!["show".into(), "--no-color".into()];
    if args
        .get("stat_only")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        argv.push("--stat".into());
    }
    argv.push(rev);
    let paths = collect_safe_paths(args)?;
    if !paths.is_empty() {
        argv.push("--".into());
        argv.extend(paths);
    }
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let (ok, stdout, stderr) = run_git(ws, &refs, cancel).await?;
    if !ok {
        return Ok(ToolOutput::err(format!("git show failed: {}", stderr.trim())));
    }
    Ok(ToolOutput::ok(truncate_chars(&stdout, GIT_TOOL_MAX_OUTPUT_CHARS)))
}

async fn exec_branch(ws: &Path, args: &Value, cancel: CancellationToken) -> Result<ToolOutput> {
    let sub = args
        .get("branch_action")
        .or_else(|| args.get("mode"))
        .and_then(Value::as_str)
        .unwrap_or("list")
        .trim()
        .to_ascii_lowercase();
    match sub.as_str() {
        "list" => {
            let (ok, stdout, stderr) = run_git(
                ws,
                &[
                    "branch",
                    "-a",
                    "--format=%(if)%(HEAD)%(then)* %(else)  %(end)%(refname:short)  %(objectname:short)  %(subject)",
                ],
                cancel,
            )
            .await?;
            if !ok {
                return Ok(ToolOutput::err(format!("git branch failed: {}", stderr.trim())));
            }
            Ok(ToolOutput::ok(truncate_chars(stdout.trim(), GIT_TOOL_MAX_OUTPUT_CHARS)))
        }
        "create" => {
            let name = require_safe_branch_name(args)?;
            let mut argv = vec!["checkout", "-b", name.as_str()];
            let base = opt_safe_rev(args.get("rev"))?;
            if let Some(ref b) = base {
                argv.push(b.as_str());
            }
            let (ok, stdout, stderr) = run_git(ws, &argv, cancel).await?;
            match ok {
                true => Ok(ToolOutput::ok(format!(
                    "Created and switched to branch `{name}`.\n{}{}",
                    stdout.trim(),
                    stderr.trim()
                ))),
                false => Ok(ToolOutput::err(format!(
                    "git checkout -b failed: {}",
                    stderr.trim()
                ))),
            }
        }
        "checkout" | "switch" => {
            let name = require_safe_branch_name(args)?;
            let (ok, stdout, stderr) = run_git(ws, &["checkout", name.as_str()], cancel).await?;
            match ok {
                true => Ok(ToolOutput::ok(format!(
                    "Switched to branch `{name}`.\n{}{}",
                    stdout.trim(),
                    stderr.trim()
                ))),
                false => Ok(ToolOutput::err(format!(
                    "git checkout failed: {}",
                    stderr.trim()
                ))),
            }
        }
        other => Ok(ToolOutput::err(format!(
            "Unsupported branch_action `{other}`. Use `list`, `create`, or `checkout`."
        ))),
    }
}

async fn exec_stage(ws: &Path, args: &Value, cancel: CancellationToken) -> Result<ToolOutput> {
    let paths = collect_safe_paths(args)?;
    if paths.is_empty() {
        return Ok(ToolOutput::err(
            "Provide `paths` (or `path`) to stage via `git_tool(action=\"stage\")`.",
        ));
    }
    let allow_sensitive = args
        .get("allow_sensitive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !allow_sensitive {
        let sensitive: Vec<&String> = paths
            .iter()
            .filter(|p| is_sensitive_git_path(p))
            .collect();
        if !sensitive.is_empty() {
            let list: Vec<&str> = sensitive.iter().map(|s| s.as_str()).collect();
            return Ok(ToolOutput::err(format!(
                "Refused to stage potentially sensitive credential/env path(s): {}. Pass `allow_sensitive: true` if you explicitly intend to commit them.",
                list.join(", ")
            )));
        }
    }
    let mut argv: Vec<String> = vec!["add".into(), "--".into()];
    argv.extend(paths.iter().cloned());
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let (ok, _stdout, stderr) = run_git(ws, &refs, cancel).await?;
    match ok {
        true => Ok(ToolOutput::ok(format!("Staged {} path(s): {}", paths.len(), paths.join(", ")))),
        false => Ok(ToolOutput::err(format!("git add failed: {}", stderr.trim()))),
    }
}

async fn exec_unstage(ws: &Path, args: &Value, cancel: CancellationToken) -> Result<ToolOutput> {
    let paths = collect_safe_paths(args)?;
    if paths.is_empty() {
        return Ok(ToolOutput::err(
            "Provide `paths` (or `path`) to unstage via `git_tool(action=\"unstage\")`.",
        ));
    }
    let mut argv: Vec<String> = vec!["reset".into(), "HEAD".into(), "--".into()];
    argv.extend(paths.iter().cloned());
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let (ok, _stdout, stderr) = run_git(ws, &refs, cancel.clone()).await?;
    if ok {
        return Ok(ToolOutput::ok(format!(
            "Unstaged {} path(s): {}",
            paths.len(),
            paths.join(", ")
        )));
    }
    // Unborn branch (no commits yet): `git rm --cached -- <paths>` cleanly unstages initial files.
    let mut fallback: Vec<String> = vec!["rm".into(), "--cached".into(), "--".into()];
    fallback.extend(paths.iter().cloned());
    let fb_refs: Vec<&str> = fallback.iter().map(String::as_str).collect();
    let (ok2, _, stderr2) = run_git(ws, &fb_refs, cancel).await?;
    match ok2 {
        true => Ok(ToolOutput::ok(format!(
            "Unstaged {} path(s) (unborn HEAD): {}",
            paths.len(),
            paths.join(", ")
        ))),
        false => Ok(ToolOutput::err(format!(
            "git unstage failed: {} ({})",
            stderr.trim(),
            stderr2.trim()
        ))),
    }
}

fn is_sensitive_git_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let base = normalized.rsplit('/').next().unwrap_or(&normalized);
    GIT_SENSITIVE_FILE_PATTERNS.iter().any(|pat| {
        base == *pat
            || (pat.starts_with('.') && (base.ends_with(pat) || base.starts_with(pat)))
    })
}

async fn exec_commit(ws: &Path, args: &Value, cancel: CancellationToken) -> Result<ToolOutput> {
    let msg = args
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ZenkoError::Tool("`message` is required for `git_tool(action=\"commit\")`".into()))?;
    let (ok, stdout, stderr) = run_git(ws, &["commit", "-m", msg], cancel).await?;
    match ok {
        true => Ok(ToolOutput::ok(truncate_chars(
            stdout.trim(),
            GIT_TOOL_MAX_OUTPUT_CHARS,
        ))),
        false => Ok(ToolOutput::err(format!(
            "git commit failed: {} {}",
            stderr.trim(),
            stdout.trim()
        ))),
    }
}

/// Parses unit-separator (`\x1f`) delimited `git log` output deterministically (zero regex).
pub fn parse_git_log_records(raw: &str) -> Vec<Value> {
    raw.lines()
        .filter_map(|line| {
            let mut cols = line.split('\x1f');
            let sha = cols.next()?.trim();
            if sha.is_empty() {
                return None;
            }
            let short_sha = cols.next().unwrap_or("").trim();
            let author = cols.next().unwrap_or("").trim();
            let date = cols.next().unwrap_or("").trim();
            let subject = cols.next().unwrap_or("").trim();
            Some(json!({
                "sha": sha,
                "short_sha": short_sha,
                "author": author,
                "date": date,
                "subject": subject,
            }))
        })
        .collect()
}

fn opt_safe_rev(val: Option<&Value>) -> Result<Option<String>> {
    let Some(raw) = val.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if raw.starts_with('-') || raw.contains('\0') || raw.contains('\n') {
        return Err(ZenkoError::PolicyDenied(format!(
            "invalid git revision `{raw}` (leading `-` or control chars forbidden)"
        )));
    }
    Ok(Some(raw.to_string()))
}

fn require_safe_branch_name(args: &Value) -> Result<String> {
    let raw = args
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ZenkoError::Tool("`name` is required for branch create/checkout".into()))?;
    if raw.starts_with('-') || raw.contains("..") || raw.contains(char::is_whitespace) || raw.contains('\0') {
        return Err(ZenkoError::PolicyDenied(format!(
            "invalid branch name `{raw}`"
        )));
    }
    Ok(raw.to_string())
}

fn validate_rel_git_path(raw: &str) -> Result<()> {
    let p = Path::new(raw);
    if p.is_absolute()
        || raw.contains('\0')
        || p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ZenkoError::PolicyDenied(format!(
            "path `{raw}` must stay within the workspace repository"
        )));
    }
    Ok(())
}

fn collect_safe_paths(args: &Value) -> Result<Vec<String>> {
    let mut out = Vec::new();
    if let Some(p) = args.get("path").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        validate_rel_git_path(p)?;
        out.push(p.to_string());
    }
    if let Some(arr) = args.get("paths").and_then(Value::as_array) {
        for item in arr {
            if let Some(p) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                validate_rel_git_path(p)?;
                if !out.iter().any(|e| e == p) {
                    out.push(p.to_string());
                }
            }
        }
    }
    Ok(out)
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        None => s.to_string(),
        Some((byte_idx, _)) => format!("{}\n... [truncated at {max_chars} chars]", &s[..byte_idx]),
    }
}

async fn run_git(
    ws: &Path,
    args: &[&str],
    cancel: CancellationToken,
) -> Result<(bool, String, String)> {
    let mut cmd = Command::new("git");
    cmd.args(["-c", "core.quotepath=false"])
        .args(args)
        .current_dir(ws)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd
        .spawn()
        .map_err(|e| ZenkoError::Tool(format!("failed to spawn git: {e}")))?;

    let out = tokio::select! {
        _ = cancel.cancelled() => return Err(ZenkoError::Cancelled),
        res = tokio::time::timeout(Duration::from_secs(GIT_TOOL_TIMEOUT_SECS), child.wait_with_output()) => {
            res.map_err(|_| ZenkoError::Tool(format!("git timed out after {GIT_TOOL_TIMEOUT_SECS}s")))?
               .map_err(|e| ZenkoError::Tool(format!("git wait failed: {e}")))?
        }
    };
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Notify;

    #[tokio::test]
    async fn git_tool_handles_structured_status_stage_commit_log_diff_and_blame() {
        let dir = std::env::temp_dir().join(format!("zenko_git_tool_{}", zenko_core::new_id()));
        std::fs::create_dir_all(&dir).expect("dir");
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(&dir)
                .status()
                .expect("init")
                .success()
        );
        for pair in [["user.email", "test@zenko.local"], ["user.name", "Zenko Test"]] {
            assert!(
                std::process::Command::new("git")
                    .args(["config", pair[0], pair[1]])
                    .current_dir(&dir)
                    .status()
                    .expect("cfg")
                    .success()
            );
        }
        std::fs::write(dir.join("lib.rs"), "pub fn v1() -> u32 { 1 }\n").expect("write");

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
        };

        let st = GitTool
            .execute(&ctx, json!({"action": "status"}), CancellationToken::new())
            .await
            .expect("status");
        assert!(!st.is_error);
        assert!(st.content.contains("Untracked: 1"));

        let stage = GitTool
            .execute(
                &ctx,
                json!({"action": "stage", "paths": ["lib.rs"]}),
                CancellationToken::new(),
            )
            .await
            .expect("stage");
        assert!(!stage.is_error);

        let commit = GitTool
            .execute(
                &ctx,
                json!({"action": "commit", "message": "initial commit"}),
                CancellationToken::new(),
            )
            .await
            .expect("commit");
        assert!(!commit.is_error);

        let log = GitTool
            .execute(&ctx, json!({"action": "log", "limit": 5}), CancellationToken::new())
            .await
            .expect("log");
        assert!(!log.is_error);
        assert!(log.content.contains("initial commit"));

        let blame = GitTool
            .execute(
                &ctx,
                json!({"action": "blame", "path": "lib.rs", "start_line": 1, "end_line": 1}),
                CancellationToken::new(),
            )
            .await
            .expect("blame");
        assert!(!blame.is_error);
        assert!(blame.content.contains("pub fn v1"));

        // Sensitive file stage guard blocks `.env` by default, allows when `allow_sensitive: true`,
        // and `unstage` cleanly removes it from the index!
        std::fs::write(dir.join(".env"), "SECRET=123\n").expect("write env");
        let blocked = GitTool
            .execute(
                &ctx,
                json!({"action": "stage", "paths": [".env"]}),
                CancellationToken::new(),
            )
            .await
            .expect("stage env blocked");
        assert!(blocked.is_error && blocked.content.contains("sensitive"));

        let allowed = GitTool
            .execute(
                &ctx,
                json!({"action": "stage", "paths": [".env"], "allow_sensitive": true}),
                CancellationToken::new(),
            )
            .await
            .expect("stage env allowed");
        assert!(!allowed.is_error);

        let unstaged = GitTool
            .execute(
                &ctx,
                json!({"action": "unstage", "paths": [".env"]}),
                CancellationToken::new(),
            )
            .await
            .expect("unstage env");
        assert!(!unstaged.is_error);
        let st_after = GitTool
            .execute(&ctx, json!({"action": "status"}), CancellationToken::new())
            .await
            .expect("status after unstage");
        assert!(st_after.content.contains("Staged: 0"));

        // Flag injection via `rev` starting with `-` must be rejected
        let inj = GitTool
            .execute(
                &ctx,
                json!({"action": "log", "rev": "--exec=evil"}),
                CancellationToken::new(),
            )
            .await;
        assert!(inj.is_err());

        let _ = std::fs::remove_dir_all(dir);
    }
}
