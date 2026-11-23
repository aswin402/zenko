use crate::jail::{resolve_readable_path, resolve_workspace_or_hub_path};
use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use similar::TextDiff;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;
use zenko_core::{
    DEFAULT_READ_FILE_LINE_WINDOW, DEFAULT_READ_FILE_MAX_BYTES, ResourceLockKey, Result, RiskLevel,
    Tool, ToolConcurrency, ToolContext, ToolOutput, ToolSpec, ZenkoError,
};

/// Deterministic 3-hex-char content hash for Oh My Pi-style hash-anchored edits.
pub fn line_hash(line: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(line.trim_end_matches('\r').as_bytes());
    let digest = hasher.finalize();
    format!("{:02x}{:x}", digest[0], digest[1] >> 4)
}

pub fn unified_diff(path: &str, old: &str, new: &str) -> String {
    TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}

/// Deterministic canonical resource-lock key for file mutation tools.
/// Disjoint workspace files receive distinct `ToolConcurrency::Keyed(...)` keys so
/// independent edits execute concurrently while same-file edits serialize strictly.
/// Any path containing parent traversal (`..`) falls back conservatively to `Global`.
fn file_lock_key(args: &Value) -> ToolConcurrency {
    let Some(raw) = args.get("path").and_then(Value::as_str).map(str::trim) else {
        return ToolConcurrency::Global;
    };
    if raw.is_empty() {
        return ToolConcurrency::Global;
    }
    let mut parts: Vec<&str> = Vec::new();
    for seg in raw.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => return ToolConcurrency::Global,
            other => parts.push(other),
        }
    }
    match parts.is_empty() {
        true => ToolConcurrency::Global,
        false => ToolConcurrency::Keyed(ResourceLockKey::File(parts.join("/"))),
    }
}

// ─── read_file (with Oh My Pi hash-anchored lines) ─────────────────────────────

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_file".into(),
            description: "Read a file inside the workspace. Returns hash-anchored lines (`line:hash| text`) so you can perform exact `hash_edit` mutations without line-drift errors.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative or workspace path to file" },
                    "start_line": { "type": "integer", "description": "1-based start line (default 1)" },
                    "end_line": { "type": "integer", "description": "1-based inclusive end line (default start_line + 200)" }
                },
                "required": ["path"]
            }),
            risk: RiskLevel::Safe,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let req_path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `path`".into()))?;
        let path = resolve_readable_path(
            &ctx.workspace_dir,
            &ctx.hub_dir,
            &ctx.data_dir,
            req_path,
            true,
        )?;
        let meta = tokio::fs::metadata(&path).await?;
        let file_bytes = meta.len();

        let start = args
            .get("start_line")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1) as usize;
        let requested_end = args
            .get("end_line")
            .and_then(Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(start.saturating_add(DEFAULT_READ_FILE_LINE_WINDOW));

        // Stream line-by-line with bounded memory so giant files never spike RAM.
        let file = tokio::fs::File::open(&path).await?;
        let mut reader = BufReader::with_capacity(16 * 1024, file);
        let mut line_buf = String::new();
        let mut line_no = 0usize;
        let mut captured = Vec::new();
        let mut out_bytes = 0usize;
        // Generous RAM guard (<= 512 KiB) so requested line windows capture both head & tail;
        // runtime artifact spilling enforces `ctx.max_output_bytes` with 35% Head / 65% Tail.
        let byte_budget = ctx
            .max_output_bytes
            .saturating_mul(16)
            .clamp(64 * 1024, 512 * 1024);
        let stop_early = file_bytes > DEFAULT_READ_FILE_MAX_BYTES;

        loop {
            line_buf.clear();
            let n = reader.read_line(&mut line_buf).await?;
            if n == 0 {
                break;
            }
            line_no += 1;
            if line_no >= start && line_no <= requested_end && out_bytes < byte_budget {
                let clean = line_buf.trim_end_matches(['\r', '\n']);
                let h = line_hash(clean);
                let formatted = format!("{line_no}:{h}| {clean}\n");
                out_bytes += formatted.len();
                captured.push(formatted);
            }
            if stop_early && line_no >= requested_end {
                break;
            }
        }

        if captured.is_empty() && line_no > 0 {
            return Ok(ToolOutput::ok(format!(
                "File '{}' has {line_no} lines (requested start_line={start}).",
                req_path
            )));
        }

        let end_shown = start.saturating_add(captured.len().saturating_sub(1)).min(line_no);
        let total_label = match stop_early {
            true => format!("{line_no}+ ({} KB file, streamed)", file_bytes / 1024),
            false => format!("{line_no}"),
        };
        let mut out = format!("File: {req_path} (lines {start}..={end_shown} of {total_label})\n");
        for row in captured {
            out.push_str(&row);
        }
        Ok(ToolOutput::ok(out))
    }
}

// ─── hash_edit (Oh My Pi specialty) ───────────────────────────────────────────

pub struct HashEditTool;

#[async_trait]
impl Tool for HashEditTool {
    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        file_lock_key(args)
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "hash_edit".into(),
            description: "Replace an inclusive line range verified by line hashes from `read_file`. Prevents whitespace and line-drift edit errors.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "start_line": { "type": "integer", "description": "1-based start line" },
                    "start_hash": { "type": "string", "description": "3-char hash of start_line" },
                    "end_line": { "type": "integer", "description": "1-based inclusive end line" },
                    "end_hash": { "type": "string", "description": "3-char hash of end_line" },
                    "replacement": { "type": "string", "description": "Replacement text for the anchored span" }
                },
                "required": ["path", "start_line", "start_hash", "end_line", "end_hash", "replacement"]
            }),
            risk: RiskLevel::Moderate,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let req_path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `path`".into()))?;
        let start_line = args
            .get("start_line")
            .and_then(Value::as_u64)
            .ok_or_else(|| ZenkoError::Tool("missing `start_line`".into()))? as usize;
        let end_line = args
            .get("end_line")
            .and_then(Value::as_u64)
            .ok_or_else(|| ZenkoError::Tool("missing `end_line`".into()))? as usize;
        let start_hash = args
            .get("start_hash")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `start_hash`".into()))?;
        let end_hash = args
            .get("end_hash")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `end_hash`".into()))?;
        let replacement = args
            .get("replacement")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `replacement`".into()))?;

        let path = resolve_workspace_or_hub_path(&ctx.workspace_dir, &ctx.hub_dir, req_path, true)?;
        let old = tokio::fs::read_to_string(&path).await?;
        let mut lines: Vec<String> = old.lines().map(String::from).collect();

        if start_line == 0 || end_line < start_line || end_line > lines.len() {
            return Ok(ToolOutput::err(format!(
                "Invalid range {start_line}..={end_line} for file with {} lines",
                lines.len()
            )));
        }

        let actual_start = line_hash(&lines[start_line - 1]);
        let actual_end = line_hash(&lines[end_line - 1]);
        if actual_start != start_hash || actual_end != end_hash {
            return Ok(ToolOutput::err(format!(
                "Hash mismatch! Expected start {start_line}:{start_hash} (actual {actual_start}), \
                 end {end_line}:{end_hash} (actual {actual_end}). Re-read the file first."
            )));
        }

        let replacement_lines: Vec<String> = if replacement.is_empty() {
            Vec::new()
        } else {
            replacement.lines().map(String::from).collect()
        };
        lines.splice((start_line - 1)..end_line, replacement_lines);

        let mut new = lines.join("\n");
        if old.ends_with('\n') && !new.ends_with('\n') {
            new.push('\n');
        }

        atomic_write(&path, &new).await?;
        let diff = unified_diff(req_path, &old, &new);
        Ok(ToolOutput::ok_with_diff(
            format!("Applied hash_edit to {req_path} (lines {start_line}..={end_line})."),
            diff,
        ))
    }
}

// ─── apply_patch (Codex tolerant seek_sequence specialty) ─────────────────────

pub struct ApplyPatchTool;

#[async_trait]
impl Tool for ApplyPatchTool {
    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        file_lock_key(args)
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "apply_patch".into(),
            description: "Apply a tolerant search-and-replace patch to a workspace file using multi-stage sequence seeking (exact, right-trimmed, fully-trimmed) with atomic writes.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_text": { "type": "string", "description": "Existing block to locate" },
                    "new_text": { "type": "string", "description": "Replacement block" }
                },
                "required": ["path", "old_text", "new_text"]
            }),
            risk: RiskLevel::Moderate,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let req_path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `path`".into()))?;
        let old_text = args
            .get("old_text")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `old_text`".into()))?;
        let new_text = args
            .get("new_text")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `new_text`".into()))?;

        let path = resolve_workspace_or_hub_path(&ctx.workspace_dir, &ctx.hub_dir, req_path, true)?;
        let old = tokio::fs::read_to_string(&path).await?;

        let new = if let Some(idx) = old.find(old_text) {
            let mut out = String::with_capacity(old.len() + new_text.len());
            out.push_str(&old[..idx]);
            out.push_str(new_text);
            out.push_str(&old[idx + old_text.len()..]);
            out
        } else {
            let file_lines: Vec<&str> = old.lines().collect();
            let pattern_lines: Vec<&str> = old_text.lines().collect();
            let Some(start) = seek_sequence(&file_lines, &pattern_lines) else {
                return Ok(ToolOutput::err(format!(
                    "Could not locate target block in {req_path} even with tolerant seeking."
                )));
            };
            let mut rebuilt: Vec<String> = file_lines.iter().map(|s| (*s).to_string()).collect();
            let rep: Vec<String> = new_text.lines().map(String::from).collect();
            rebuilt.splice(start..(start + pattern_lines.len()), rep);
            let mut joined = rebuilt.join("\n");
            if old.ends_with('\n') && !joined.ends_with('\n') {
                joined.push('\n');
            }
            joined
        };

        atomic_write(&path, &new).await?;
        let diff = unified_diff(req_path, &old, &new);
        Ok(ToolOutput::ok_with_diff(
            format!("Patched {req_path} successfully."),
            diff,
        ))
    }
}

/// Codex-inspired tolerant line sequence seeker: exact -> trim_end -> trim.
pub fn seek_sequence(lines: &[&str], pattern: &[&str]) -> Option<usize> {
    if pattern.is_empty() || pattern.len() > lines.len() {
        return None;
    }
    let last = lines.len() - pattern.len();
    for i in 0..=last {
        if lines[i..i + pattern.len()] == *pattern {
            return Some(i);
        }
    }
    for i in 0..=last {
        if lines[i..i + pattern.len()]
            .iter()
            .zip(pattern)
            .all(|(a, b)| a.trim_end() == b.trim_end())
        {
            return Some(i);
        }
    }
    for i in 0..=last {
        if lines[i..i + pattern.len()]
            .iter()
            .zip(pattern)
            .all(|(a, b)| a.trim() == b.trim())
        {
            return Some(i);
        }
    }
    None
}

// ─── write_file & list_dir ────────────────────────────────────────────────────

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        file_lock_key(args)
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write_file".into(),
            description: "Create or overwrite a file inside the workspace atomically.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
            risk: RiskLevel::Moderate,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let req_path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `path`".into()))?;
        let content = args
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `content`".into()))?;

        let path = resolve_workspace_or_hub_path(&ctx.workspace_dir, &ctx.hub_dir, req_path, false)?;
        let old = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        atomic_write(&path, content).await?;
        let diff = unified_diff(req_path, &old, content);
        Ok(ToolOutput::ok_with_diff(
            format!("Wrote {} bytes to {req_path}.", content.len()),
            diff,
        ))
    }
}

pub struct ListDirTool;

#[async_trait]
impl Tool for ListDirTool {
    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_dir".into(),
            description: "List files and directories inside a workspace path.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path (default '.')" }
                }
            }),
            risk: RiskLevel::Safe,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let req_path = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let path = resolve_readable_path(
            &ctx.workspace_dir,
            &ctx.hub_dir,
            &ctx.data_dir,
            req_path,
            true,
        )?;
        let mut entries = tokio::fs::read_dir(&path).await?;
        let mut rows = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let meta = entry.metadata().await?;
            let kind = if meta.is_dir() { "dir " } else { "file" };
            let name = entry.file_name().to_string_lossy().to_string();
            rows.push(format!("{kind}  {:>8}  {name}", meta.len()));
        }
        rows.sort();
        Ok(ToolOutput::ok(rows.join("\n")))
    }
}

async fn atomic_write(path: &std::path::Path, content: &str) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    tokio::fs::create_dir_all(parent).await?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let tmp = parent.join(format!(".{name}.zenko_tmp_{}", zenko_core::new_id()));
    if let Err(err) = tokio::fs::write(&tmp, content).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(err.into());
    }
    if let Err(err) = tokio::fs::rename(&tmp, path).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(err.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hash_edit_and_tolerant_patch() {
        let dir = std::env::temp_dir().join(format!("zenko_fs_{}", zenko_core::new_id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let file = dir.join("demo.txt");
        std::fs::write(&file, "alpha\nbeta\ngamma\n").expect("write");

        let ctx = ToolContext {
            session_id: "s1".into(),
            workspace_dir: dir.clone(),
            data_dir: dir.clone(),
            hub_dir: dir.clone(),
            shell_timeout_secs: 10,
            auto_background_after_secs: 0,
            max_output_bytes: 4096,
            headless_no_wait: false,
            demote_bg: std::sync::Arc::new(tokio::sync::Notify::new()),
        };
        let cancel = CancellationToken::new();

        let h2 = line_hash("beta");
        let res = HashEditTool
            .execute(
                &ctx,
                json!({
                    "path": "demo.txt",
                    "start_line": 2,
                    "start_hash": h2,
                    "end_line": 2,
                    "end_hash": h2,
                    "replacement": "BETA_UPDATED"
                }),
                cancel.clone(),
            )
            .await
            .expect("hash_edit");
        assert!(!res.is_error);
        assert!(res.diff.is_some());

        let patched = ApplyPatchTool
            .execute(
                &ctx,
                json!({
                    "path": "demo.txt",
                    "old_text": "   BETA_UPDATED  ",
                    "new_text": "BETA_FINAL"
                }),
                cancel,
            )
            .await
            .expect("apply_patch");
        assert!(!patched.is_error);
        let final_txt = std::fs::read_to_string(&file).expect("read");
        assert!(final_txt.contains("BETA_FINAL"));

        let _ = std::fs::remove_dir_all(dir);
    }
}
