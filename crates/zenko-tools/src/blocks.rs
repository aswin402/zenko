//! Pure-Rust Reusable Script Blocks (`script_block`) — zero JS/QuickJS engine overhead.
//! Stores reusable automation scripts atomically (`0600`) inside `<hub_dir>/blocks/<slug>.sh`
//! so agents can save recurring verification/audit/transform scripts once and re-run them
//! cleanly across turns inside Zenko's jailed workspace + Linux Landlock write sandbox.

use crate::jail::resolve_workspace_or_hub_path;
use crate::sandbox::{ShellSandboxPolicy, install_sandbox_on_command};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    FILE_MODE_OWNER_ONLY, Result, RiskLevel, SAFE_SHELL_ENV_KEYS, SCRIPT_BLOCK_ACTIONS,
    SCRIPT_BLOCK_MAX_BYTES, SCRIPT_BLOCK_MAX_LIST, SCRIPT_BLOCK_SLUG_MAX_CHARS,
    SCRIPT_BLOCKS_DIR_NAME, SHELL_BIN_UNIX, SHELL_MAX_TIMEOUT_SECS, SCRIPT_BLOCK_DESC_PREFIX,
    ScriptBlockAction, TOOL_SCRIPT_BLOCK, Tool, ToolConcurrency, ToolContext, ToolExposure,
    ToolOutput, ToolSet, ToolSpec, ZenkoError, write_owner_only_atomic,
};

pub struct ScriptBlockTool {
    os_sandbox: Arc<AtomicBool>,
}

impl ScriptBlockTool {
    pub fn new(os_sandbox: Arc<AtomicBool>) -> Self {
        Self { os_sandbox }
    }
}

fn normalize_block_slug(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ZenkoError::Tool("`name` slug cannot be empty".into()));
    }
    let mut slug = String::with_capacity(trimmed.len().min(SCRIPT_BLOCK_SLUG_MAX_CHARS));
    for ch in trimmed.chars() {
        if slug.len() >= SCRIPT_BLOCK_SLUG_MAX_CHARS {
            break;
        }
        match ch {
            'a'..='z' | '0'..='9' | '-' | '_' => slug.push(ch),
            'A'..='Z' => slug.push(ch.to_ascii_lowercase()),
            ' ' | '.' | '/' => {
                if !slug.ends_with('-') && !slug.is_empty() {
                    slug.push('-');
                }
            }
            _ => {}
        }
    }
    let clean = slug.trim_matches('-').to_string();
    match clean.is_empty() {
        true => Err(ZenkoError::Tool(format!(
            "invalid script block slug `{raw}` (use ASCII letters, digits, `-`, or `_`)"
        ))),
        false => Ok(clean),
    }
}

fn blocks_dir(hub_dir: &Path) -> PathBuf {
    hub_dir.join(SCRIPT_BLOCKS_DIR_NAME)
}

fn block_path(hub_dir: &Path, slug: &str) -> PathBuf {
    blocks_dir(hub_dir).join(format!("{slug}.sh"))
}

fn extract_block_description(content: &str) -> String {
    content
        .lines()
        .take(4)
        .find_map(|line| {
            line.trim()
                .strip_prefix(SCRIPT_BLOCK_DESC_PREFIX)
                .map(|rest| rest.trim().to_string())
        })
        .unwrap_or_else(|| "Reusable script block".into())
}

fn render_block_script(description: Option<&str>, script: &str) -> String {
    let body = script.trim_start_matches('\u{feff}');
    let has_shebang = body.starts_with("#!");
    let desc_line = description
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|d| format!("{SCRIPT_BLOCK_DESC_PREFIX} {}\n", d.replace('\n', " ")));
    let mut out = String::with_capacity(body.len() + 96);
    match (has_shebang, desc_line) {
        (true, Some(dl)) => match body.find('\n') {
            Some(pos) => {
                out.push_str(&body[..pos + 1]);
                out.push_str(&dl);
                out.push_str(&body[pos + 1..]);
            }
            None => {
                out.push_str(body);
                out.push('\n');
                out.push_str(&dl);
            }
        },
        (true, None) => out.push_str(body),
        (false, Some(dl)) => {
            out.push_str("#!/bin/sh\n");
            out.push_str(&dl);
            out.push_str(body);
        }
        (false, None) => {
            out.push_str("#!/bin/sh\n");
            out.push_str(body);
        }
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[async_trait]
impl Tool for ScriptBlockTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::Execution
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        match args
            .get("action")
            .and_then(Value::as_str)
            .and_then(ScriptBlockAction::parse)
        {
            Some(ScriptBlockAction::List | ScriptBlockAction::Read) => ToolConcurrency::Shared,
            _ => ToolConcurrency::Global,
        }
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_SCRIPT_BLOCK.into(),
            description: "Save, list, inspect, delete, and re-run reusable shell script blocks stored under `hub:blocks/` inside the jailed workspace and Linux Landlock write sandbox — saves tokens by avoiding repeated boilerplate script generation across turns.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": SCRIPT_BLOCK_ACTIONS,
                        "description": "Block operation: `list`, `save`, `read`, `run`, or `delete`."
                    },
                    "name": {
                        "type": "string",
                        "description": "Reusable block slug (e.g. `verify-workspace`, `lint-rust`). Required for `save`, `read`, `run`, `delete`."
                    },
                    "description": {
                        "type": "string",
                        "description": "Concise 1-line description stored in header metadata on `save`."
                    },
                    "script": {
                        "type": "string",
                        "description": "Shell script body to persist on `save`."
                    },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional positional arguments (`$1`, `$2`, ...) passed to the script on `run`."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional jailed working directory relative to workspace (or `hub:...`) for `run`."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Optional execution timeout override in seconds for `run`."
                    }
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
        let action = ScriptBlockAction::parse(raw_action).ok_or_else(|| {
            ZenkoError::Tool(format!(
                "Unknown `script_block` action `{raw_action}`. Valid actions: {}.",
                SCRIPT_BLOCK_ACTIONS.join(", ")
            ))
        })?;

        match action {
            ScriptBlockAction::List => {
                let dir = blocks_dir(&ctx.hub_dir);
                let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
                    return Ok(ToolOutput::ok("No reusable script blocks saved yet."));
                };
                let mut rows: Vec<(String, String, u64)> = Vec::new();
                while let Ok(Some(ent)) = rd.next_entry().await {
                    let path = ent.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("sh") {
                        continue;
                    }
                    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                        continue;
                    };
                    let bytes = ent.metadata().await.map(|m| m.len()).unwrap_or(0);
                    let desc = tokio::fs::read_to_string(&path)
                        .await
                        .map(|s| extract_block_description(&s))
                        .unwrap_or_else(|_| "Reusable script block".into());
                    rows.push((stem.to_string(), desc, bytes));
                    if rows.len() >= SCRIPT_BLOCK_MAX_LIST {
                        break;
                    }
                }
                rows.sort_by(|a, b| a.0.cmp(&b.0));
                if rows.is_empty() {
                    return Ok(ToolOutput::ok("No reusable script blocks saved yet."));
                }
                let text = rows
                    .iter()
                    .map(|(slug, desc, bytes)| format!("- `{slug}` ({bytes} B): {desc}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(ToolOutput::ok(format!(
                    "Saved script blocks ({}):\n{text}",
                    rows.len()
                )))
            }
            ScriptBlockAction::Save => {
                let name = args
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`name` is required for `save`".into()))?;
                let slug = normalize_block_slug(name)?;
                let script = args
                    .get("script")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| ZenkoError::Tool("`script` is required for `save`".into()))?;
                if script.len() > SCRIPT_BLOCK_MAX_BYTES {
                    return Ok(ToolOutput::err(format!(
                        "Script block exceeds max size ({SCRIPT_BLOCK_MAX_BYTES} bytes)."
                    )));
                }
                let desc = args.get("description").and_then(Value::as_str);
                let rendered = render_block_script(desc, script);
                let path = block_path(&ctx.hub_dir, &slug);
                let path_clone = path.clone();
                let bytes_len = rendered.len();
                tokio::task::spawn_blocking(move || -> Result<()> {
                    if let Some(parent) = path_clone.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    write_owner_only_atomic(&path_clone, rendered.as_bytes())?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(
                            &path_clone,
                            std::fs::Permissions::from_mode(FILE_MODE_OWNER_ONLY | 0o100),
                        );
                    }
                    Ok(())
                })
                .await
                .map_err(|e| ZenkoError::Tool(format!("save block worker failed: {e}")))??;
                Ok(ToolOutput::ok(format!(
                    "Saved script block `{slug}` ({bytes_len} bytes) -> `hub:{SCRIPT_BLOCKS_DIR_NAME}/{slug}.sh`. Re-run via `script_block(action=\"run\", name=\"{slug}\")`."
                )))
            }
            ScriptBlockAction::Read => {
                let name = args
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`name` is required for `read`".into()))?;
                let slug = normalize_block_slug(name)?;
                let path = block_path(&ctx.hub_dir, &slug);
                match tokio::fs::read_to_string(&path).await {
                    Ok(body) => Ok(ToolOutput::ok(format!("# Script Block `{slug}`\n\n{body}"))),
                    Err(_) => Ok(ToolOutput::err(format!("Script block `{slug}` not found."))),
                }
            }
            ScriptBlockAction::Delete => {
                let name = args
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`name` is required for `delete`".into()))?;
                let slug = normalize_block_slug(name)?;
                let path = block_path(&ctx.hub_dir, &slug);
                match tokio::fs::remove_file(&path).await {
                    Ok(()) => Ok(ToolOutput::ok(format!("Deleted script block `{slug}`."))),
                    Err(_) => Ok(ToolOutput::err(format!("Script block `{slug}` not found."))),
                }
            }
            ScriptBlockAction::Run => {
                let name = args
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`name` is required for `run`".into()))?;
                let slug = normalize_block_slug(name)?;
                let path = block_path(&ctx.hub_dir, &slug);
                if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
                    return Ok(ToolOutput::err(format!(
                        "Script block `{slug}` not found. Use `script_block(action=\"list\")` or `save` first."
                    )));
                }

                let exec_dir = match args
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    Some(rel_cwd) => {
                        let resolved = resolve_workspace_or_hub_path(
                            &ctx.workspace_dir,
                            &ctx.hub_dir,
                            rel_cwd,
                            true,
                        )?;
                        match tokio::fs::metadata(&resolved).await {
                            Ok(meta) if meta.is_dir() => resolved,
                            _ => {
                                return Ok(ToolOutput::err(format!(
                                    "Invalid `cwd` `{rel_cwd}` (must be an existing workspace/hub directory)."
                                )));
                            }
                        }
                    }
                    None => ctx.workspace_dir.clone(),
                };

                let script_args: Vec<String> = args
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .take(32)
                            .collect()
                    })
                    .unwrap_or_default();

                let timeout_secs = args
                    .get("timeout_secs")
                    .and_then(Value::as_u64)
                    .unwrap_or(ctx.shell_timeout_secs)
                    .clamp(1, SHELL_MAX_TIMEOUT_SECS);

                let mut cmd = Command::new(SHELL_BIN_UNIX);
                cmd.arg(&path);
                for a in &script_args {
                    cmd.arg(a);
                }
                cmd.current_dir(&exec_dir)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true)
                    .env_clear();
                for key in SAFE_SHELL_ENV_KEYS {
                    if let Ok(val) = std::env::var(key) {
                        cmd.env(key, val);
                    }
                }
                let policy = ShellSandboxPolicy::for_workspace(
                    self.os_sandbox.load(Ordering::Relaxed),
                    &ctx.workspace_dir,
                    Some(&ctx.hub_dir),
                    &[],
                );
                install_sandbox_on_command(&mut cmd, &policy);

                let child = cmd
                    .spawn()
                    .map_err(|e| ZenkoError::Tool(format!("failed to execute block `{slug}`: {e}")))?;

                let output = tokio::select! {
                    _ = cancel.cancelled() => return Err(ZenkoError::Cancelled),
                    res = tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output()) => {
                        match res {
                            Ok(Ok(o)) => o,
                            Ok(Err(e)) => return Err(ZenkoError::Tool(format!("block `{slug}` failed: {e}"))),
                            Err(_) => return Ok(ToolOutput::err(format!("Script block `{slug}` timed out after {timeout_secs}s"))),
                        }
                    }
                };

                let code = output.status.code().unwrap_or(-1);
                let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stderr.is_empty() {
                    if !combined.is_empty() && !combined.ends_with('\n') {
                        combined.push('\n');
                    }
                    combined.push_str(&stderr);
                }
                let text = format!("exit_code: {code}\n{combined}");
                Ok(match code == 0 {
                    true => ToolOutput::ok(text),
                    false => ToolOutput::err(text),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Notify;

    #[tokio::test]
    async fn script_block_save_list_read_run_and_delete_roundtrip() {
        let dir = std::env::temp_dir().join(format!("zenko_blocks_{}", zenko_core::new_id()));
        let ws = dir.join("ws");
        let hub = dir.join("hub");
        std::fs::create_dir_all(&ws).expect("ws");
        std::fs::create_dir_all(&hub).expect("hub");

        let tool = ScriptBlockTool::new(Arc::new(AtomicBool::new(true)));
        let ctx = ToolContext {
            session_id: "s1".into(),
            workspace_dir: ws.clone(),
            data_dir: dir.clone(),
            hub_dir: hub.clone(),
            shell_timeout_secs: 15,
            auto_background_after_secs: 0,
            max_output_bytes: 32 * 1024,
            headless_no_wait: true,
            demote_bg: Arc::new(Notify::new()),
            file_state: None,
        };

        let saved = tool
            .execute(
                &ctx,
                json!({
                    "action": "save",
                    "name": "Echo Greeting",
                    "description": "Echo workspace arg",
                    "script": "echo \"hello:$1:$(basename $(pwd))\""
                }),
                CancellationToken::new(),
            )
            .await
            .expect("save");
        assert!(!saved.is_error, "{}", saved.content);

        let listed = tool
            .execute(&ctx, json!({ "action": "list" }), CancellationToken::new())
            .await
            .expect("list");
        assert!(listed.content.contains("echo-greeting"));
        assert!(listed.content.contains("Echo workspace arg"));

        let ran = tool
            .execute(
                &ctx,
                json!({ "action": "run", "name": "echo-greeting", "args": ["zenko"] }),
                CancellationToken::new(),
            )
            .await
            .expect("run");
        assert!(!ran.is_error, "{}", ran.content);
        assert!(ran.content.contains("hello:zenko:ws"), "out={}", ran.content);

        let deleted = tool
            .execute(
                &ctx,
                json!({ "action": "delete", "name": "echo-greeting" }),
                CancellationToken::new(),
            )
            .await
            .expect("delete");
        assert!(!deleted.is_error);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
