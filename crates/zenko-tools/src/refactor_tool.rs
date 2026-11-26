//! Unified Deferred `code_refactor` Tool — Pure-Rust Structural Search/Rewrite (`ast-grep`),
//! Code-Only Multi-File Replace (`sd`), Import-Aware File Move (`ripide`), Language LOC Stats
//! (`tokei`), and Weighted Dijkstra Semantic Diff (`difftastic`).
//!
//! Zero regex, zero C/tree-sitter deps, 100% kernel `O_NOFOLLOW` two-phase atomic commits,
//! and strict Plan Mode / read-only subagent safety (`dry_run: true` previews permitted read-only).

use crate::code_stats::{format_code_stats_table, scan_workspace_code_stats};
use crate::fs_tools::{
    atomic_write_in_ctx, check_existing_file_freshness, forget_fresh_file_state, line_hash,
    read_jailed_string_async, record_fresh_file_state, remove_jailed_in_ctx, unified_diff,
};
use crate::glob::CompiledGlob;
use crate::jail::{
    format_display_rel_path, resolve_readable_path, resolve_workspace_or_hub_path,
};
use crate::struct_code::{
    compute_structural_diff, extract_structural_pattern_anchor, format_structural_diff,
    replace_in_source, rewrite_relative_imports_for_move, rewrite_structural, search_structural,
    syntax_for_path,
};
use crate::trigram::{TrigramQueryPlan, query_trigram_candidates};
use async_trait::async_trait;
use ignore::WalkBuilder;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;
use zenko_core::{
    CODE_REFACTOR_ACTIONS, CODE_REFACTOR_MAX_FILES_APPLY, CODE_STATS_MAX_FILES,
    CodeRefactorAction, DEFAULT_SEARCH_MAX_FILE_BYTES, DEFAULT_SEARCH_SKIP_DIRS,
    FS_MAX_EDIT_FILE_BYTES, Result, RiskLevel, STRUCT_SEARCH_DEFAULT_MAX_MATCHES,
    STRUCT_SEARCH_MAX_MATCHES, TOOL_CODE_REFACTOR, Tool, ToolConcurrency, ToolContext,
    ToolExposure, ToolOutput, ToolSet, ToolSpec, ZenkoError, is_code_refactor_args_mutating,
};

pub struct CodeRefactorTool;

fn collect_target_files(
    root: &Path,
    ws_root: &Path,
    hub_root: &Path,
    compiled_glob: Option<&CompiledGlob>,
    anchor_hint: Option<&str>,
) -> Vec<PathBuf> {
    if root.is_file() {
        return vec![root.to_path_buf()];
    }
    if let Some(anchor) = anchor_hint
        && let TrigramQueryPlan::Candidates(cands) = query_trigram_candidates(root, anchor, true)
    {
        return cands
            .into_iter()
            .filter(|p| match compiled_glob {
                Some(g) => {
                    let rel = format_display_rel_path(p, ws_root, hub_root, "");
                    g.matches(&rel)
                }
                None => true,
            })
            .collect();
    }
    let mut files = Vec::new();
    let mut walker = WalkBuilder::new(root);
    walker
        .hidden(true)
        .require_git(false)
        .max_filesize(Some(DEFAULT_SEARCH_MAX_FILE_BYTES))
        .filter_entry(|dent| {
            if dent.depth() == 0 {
                return true;
            }
            let is_dir = dent.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            match is_dir {
                false => true,
                true => {
                    let name = dent.file_name().to_str().unwrap_or("");
                    !DEFAULT_SEARCH_SKIP_DIRS.contains(&name)
                }
            }
        });
    for dent in walker.build().flatten() {
        if !dent.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let p = dent.into_path();
        if let Some(g) = compiled_glob {
            let rel = format_display_rel_path(&p, ws_root, hub_root, "");
            if !g.matches(&rel) {
                continue;
            }
        }
        files.push(p);
    }
    files
}

fn ok_meta(content: String, diff: Option<String>, meta: Value) -> ToolOutput {
    ToolOutput {
        content,
        is_error: false,
        diff,
        metadata: Some(meta),
    }
}

#[async_trait]
impl Tool for CodeRefactorTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::CodeIntel
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        match is_code_refactor_args_mutating(args) {
            false => ToolConcurrency::Shared,
            true => ToolConcurrency::Global,
        }
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_CODE_REFACTOR.into(),
            description: "Pure-Rust structural code intelligence & atomic refactoring toolkit (zero C/tree-sitter bloat). Read-only actions (`struct_search`, `stats`, `struct_diff`) and any `dry_run: true` preview work in Plan Mode; mutating actions (`struct_rewrite`, `replace`, `move_imports`) commit atomically via kernel `O_NOFOLLOW` two-phase writes.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": CODE_REFACTOR_ACTIONS,
                        "description": "`struct_search` ($X/$$$ARGS metavariable search), `struct_rewrite` (metavariable rewrite), `replace` (multi-file literal/code-only replace), `move_imports` (move file + rewrite relative imports), `stats` (Tokei language LOC/comment/blank breakdown), `struct_diff` (Difftastic Dijkstra semantic diff)."
                    },
                    "path": { "type": "string", "description": "Target directory or file path (default '.'). For `move_imports`: source file. For `struct_diff`: left/before file." },
                    "other_path": { "type": "string", "description": "For `struct_diff`: right/after file to compare against `path`." },
                    "move_to": { "type": "string", "description": "For `move_imports`: destination file path." },
                    "file_glob": { "type": "string", "description": "Optional glob filter (e.g. `**/*.rs`, `src/**/*.{ts,tsx}`)." },
                    "pattern": { "type": "string", "description": "Structural pattern with `$X`, `$$$ARGS`, `$_` (for `struct_search` / `struct_rewrite`)." },
                    "rewrite": { "type": "string", "description": "Rewrite template expanding `$X` / `$$$ARGS` (for `struct_rewrite`)." },
                    "find": { "type": "string", "description": "Exact text or identifier to find (for `replace`)." },
                    "replace": { "type": "string", "description": "Replacement text (for `replace`)." },
                    "whole_word": { "type": "boolean", "description": "Match identifier word boundaries only (for `replace`, default true)." },
                    "code_only": { "type": "boolean", "description": "Skip comments and string literals using language syntax FSM (for `replace`, default true)." },
                    "dry_run": { "type": "boolean", "description": "Preview unified diffs without modifying disk (default false; permitted in Plan Mode)." },
                    "with_hashes": { "type": "boolean", "description": "Include `line:hash|` anchors on `struct_search` hits (default true)." },
                    "max_matches": { "type": "integer", "description": format!("Max matches for `struct_search` (default {STRUCT_SEARCH_DEFAULT_MAX_MATCHES}, cap {STRUCT_SEARCH_MAX_MATCHES}).") }
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
        let Some(action) = CodeRefactorAction::parse(raw_action) else {
            return Err(ZenkoError::Tool(format!(
                "invalid `code_refactor` action `{raw_action}` (expected: {})",
                CODE_REFACTOR_ACTIONS.join(", ")
            )));
        };

        let ws_canon = ctx
            .workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| ctx.workspace_dir.clone());
        let hub_canon = ctx
            .hub_dir
            .canonicalize()
            .unwrap_or_else(|_| ctx.hub_dir.clone());
        let req_path = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let compiled_glob = args
            .get("file_glob")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(CompiledGlob::compile);
        let dry_run = args
            .get("dry_run")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        match action {
            CodeRefactorAction::Stats => {
                let target = resolve_readable_path(
                    &ctx.workspace_dir,
                    &ctx.hub_dir,
                    &ctx.data_dir,
                    req_path,
                    true,
                )?;
                let stats = tokio::task::spawn_blocking(move || {
                    scan_workspace_code_stats(&target, CODE_STATS_MAX_FILES)
                })
                .await
                .map_err(|e| ZenkoError::Tool(format!("code stats join error: {e}")))?;
                let rendered = format_code_stats_table(&stats);
                Ok(ok_meta(
                    rendered,
                    None,
                    serde_json::to_value(&stats).unwrap_or(Value::Null),
                ))
            }
            CodeRefactorAction::StructDiff => {
                let left_req = args
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`struct_diff` requires `path`".into()))?;
                let right_req = args
                    .get("other_path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`struct_diff` requires `other_path`".into()))?;
                let left_abs = resolve_readable_path(
                    &ctx.workspace_dir,
                    &ctx.hub_dir,
                    &ctx.data_dir,
                    left_req,
                    true,
                )?;
                let right_abs = resolve_readable_path(
                    &ctx.workspace_dir,
                    &ctx.hub_dir,
                    &ctx.data_dir,
                    right_req,
                    true,
                )?;
                let before =
                    read_jailed_string_async(ctx, &left_abs, FS_MAX_EDIT_FILE_BYTES, left_req)
                        .await?;
                let after =
                    read_jailed_string_async(ctx, &right_abs, FS_MAX_EDIT_FILE_BYTES, right_req)
                        .await?;
                let spec = syntax_for_path(&right_abs);
                let report = compute_structural_diff(spec, &before, &after);
                let label = format!("{left_req} -> {right_req}");
                let rendered = format_structural_diff(&label, &report);
                Ok(ok_meta(
                    rendered,
                    None,
                    serde_json::to_value(&report).unwrap_or(Value::Null),
                ))
            }
            CodeRefactorAction::StructSearch => {
                let pattern = args
                    .get("pattern")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`struct_search` requires `pattern`".into()))?
                    .to_string();
                let root = resolve_readable_path(
                    &ctx.workspace_dir,
                    &ctx.hub_dir,
                    &ctx.data_dir,
                    req_path,
                    true,
                )?;
                let with_hashes = args
                    .get("with_hashes")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let max_matches = args
                    .get("max_matches")
                    .and_then(Value::as_u64)
                    .unwrap_or(STRUCT_SEARCH_DEFAULT_MAX_MATCHES as u64)
                    .clamp(1, STRUCT_SEARCH_MAX_MATCHES as u64) as usize;

                let anchor = extract_structural_pattern_anchor(syntax_for_path(&root), &pattern);
                let files = collect_target_files(
                    &root,
                    &ws_canon,
                    &hub_canon,
                    compiled_glob.as_ref(),
                    anchor.as_deref(),
                );

                let mut total_hits = 0usize;
                let mut grouped: Vec<String> = Vec::new();
                for file_path in files {
                    if cancel.is_cancelled() || total_hits >= max_matches {
                        break;
                    }
                    let rel = format_display_rel_path(&file_path, &ws_canon, &hub_canon, "");
                    let Ok(content) =
                        read_jailed_string_async(ctx, &file_path, FS_MAX_EDIT_FILE_BYTES, &rel)
                            .await
                    else {
                        continue;
                    };
                    let spec = syntax_for_path(&file_path);
                    let remaining = max_matches.saturating_sub(total_hits);
                    let hits = search_structural(spec, &content, &pattern, remaining)?;
                    if hits.is_empty() {
                        continue;
                    }
                    let src_lines: Vec<&str> = content.lines().collect();
                    let mut block = vec![format!("{rel}:")];
                    for h in hits {
                        total_hits += 1;
                        let first_line = src_lines
                            .get(h.start_line.saturating_sub(1))
                            .copied()
                            .unwrap_or("");
                        let preview = h.matched_text.lines().next().unwrap_or("").trim();
                        let line_tag = match with_hashes {
                            true => format!("{}:{}|", h.start_line, line_hash(first_line)),
                            false => format!("L{}:", h.start_line),
                        };
                        let span_note = match h.end_line > h.start_line {
                            true => format!(" (..L{})", h.end_line),
                            false => String::new(),
                        };
                        let binds = match h.bindings.is_empty() {
                            true => String::new(),
                            false => {
                                let pairs: Vec<String> = h
                                    .bindings
                                    .iter()
                                    .map(|(k, v)| format!("${k}={v}"))
                                    .collect();
                                format!("  [{}]", pairs.join(", "))
                            }
                        };
                        block.push(format!("  {line_tag}{span_note} {preview}{binds}"));
                    }
                    grouped.push(block.join("\n"));
                }

                let text = match total_hits == 0 {
                    true => format!("No structural matches for `{pattern}`."),
                    false => format!(
                        "Structural matches ({total_hits}{}):\n{}",
                        match total_hits >= max_matches {
                            true => " [limit reached]",
                            false => "",
                        },
                        grouped.join("\n")
                    ),
                };
                Ok(ok_meta(text, None, json!({ "matches": total_hits })))
            }
            CodeRefactorAction::StructRewrite => {
                let pattern = args
                    .get("pattern")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`struct_rewrite` requires `pattern`".into()))?
                    .to_string();
                let rewrite = args
                    .get("rewrite")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`struct_rewrite` requires `rewrite`".into()))?
                    .to_string();
                let root =
                    resolve_workspace_or_hub_path(&ctx.workspace_dir, &ctx.hub_dir, req_path, true)?;
                let anchor = extract_structural_pattern_anchor(syntax_for_path(&root), &pattern);
                let files = collect_target_files(
                    &root,
                    &ws_canon,
                    &hub_canon,
                    compiled_glob.as_ref(),
                    anchor.as_deref(),
                );

                // Two-phase plan: validate every file before mutating disk.
                let mut planned: Vec<(String, PathBuf, String, String, usize)> = Vec::new();
                for file_path in files {
                    if cancel.is_cancelled() {
                        break;
                    }
                    let rel = format_display_rel_path(&file_path, &ws_canon, &hub_canon, "");
                    let Ok(old) =
                        read_jailed_string_async(ctx, &file_path, FS_MAX_EDIT_FILE_BYTES, &rel)
                            .await
                    else {
                        continue;
                    };
                    let (next, count) =
                        rewrite_structural(syntax_for_path(&file_path), &old, &pattern, &rewrite)?;
                    if count == 0 {
                        continue;
                    }
                    if planned.len() >= CODE_REFACTOR_MAX_FILES_APPLY {
                        return Ok(ToolOutput::err(format!(
                            "Refused `struct_rewrite`: matches exceed max {CODE_REFACTOR_MAX_FILES_APPLY} files per atomic call. Narrow `path` or `file_glob`."
                        )));
                    }
                    if !dry_run
                        && let Err(stale) =
                            check_existing_file_freshness(ctx, &rel, &file_path, &old, true)
                    {
                        return Ok(ToolOutput::err(stale));
                    }
                    planned.push((rel, file_path, old, next, count));
                }

                commit_or_preview_batch(ctx, "struct_rewrite", dry_run, planned).await
            }
            CodeRefactorAction::Replace => {
                let find = args
                    .get("find")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| ZenkoError::Tool("`replace` requires non-empty `find`".into()))?;
                let replace = args
                    .get("replace")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`replace` requires `replace`".into()))?;
                let whole_word = args
                    .get("whole_word")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let code_only = args
                    .get("code_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let root =
                    resolve_workspace_or_hub_path(&ctx.workspace_dir, &ctx.hub_dir, req_path, true)?;
                let files = collect_target_files(
                    &root,
                    &ws_canon,
                    &hub_canon,
                    compiled_glob.as_ref(),
                    Some(find),
                );

                let mut planned: Vec<(String, PathBuf, String, String, usize)> = Vec::new();
                for file_path in files {
                    if cancel.is_cancelled() {
                        break;
                    }
                    let rel = format_display_rel_path(&file_path, &ws_canon, &hub_canon, "");
                    let Ok(old) =
                        read_jailed_string_async(ctx, &file_path, FS_MAX_EDIT_FILE_BYTES, &rel)
                            .await
                    else {
                        continue;
                    };
                    let (next, count) = replace_in_source(
                        syntax_for_path(&file_path),
                        &old,
                        find,
                        replace,
                        whole_word,
                        code_only,
                    );
                    if count == 0 {
                        continue;
                    }
                    if planned.len() >= CODE_REFACTOR_MAX_FILES_APPLY {
                        return Ok(ToolOutput::err(format!(
                            "Refused `replace`: matches exceed max {CODE_REFACTOR_MAX_FILES_APPLY} files per atomic call. Narrow `path` or `file_glob`."
                        )));
                    }
                    if !dry_run
                        && let Err(stale) =
                            check_existing_file_freshness(ctx, &rel, &file_path, &old, true)
                    {
                        return Ok(ToolOutput::err(stale));
                    }
                    planned.push((rel, file_path, old, next, count));
                }

                commit_or_preview_batch(ctx, "replace", dry_run, planned).await
            }
            CodeRefactorAction::MoveImports => {
                let src_req = args
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`move_imports` requires `path`".into()))?;
                let dest_req = args
                    .get("move_to")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ZenkoError::Tool("`move_imports` requires `move_to`".into()))?;
                let src_abs =
                    resolve_workspace_or_hub_path(&ctx.workspace_dir, &ctx.hub_dir, src_req, true)?;
                let dest_abs =
                    resolve_workspace_or_hub_path(&ctx.workspace_dir, &ctx.hub_dir, dest_req, false)?;
                if dest_abs.exists() {
                    return Ok(ToolOutput::err(format!(
                        "Refused `move_imports`: destination `{dest_req}` already exists."
                    )));
                }
                let src_body =
                    read_jailed_string_async(ctx, &src_abs, FS_MAX_EDIT_FILE_BYTES, src_req)
                        .await?;
                if !dry_run
                    && let Err(stale) =
                        check_existing_file_freshness(ctx, src_req, &src_abs, &src_body, true)
                {
                    return Ok(ToolOutput::err(stale));
                }

                let src_rel_str = format_display_rel_path(&src_abs, &ws_canon, &hub_canon, src_req);
                let dest_rel_str =
                    format_display_rel_path(&dest_abs, &ws_canon, &hub_canon, dest_req);
                let src_rel = PathBuf::from(&src_rel_str);
                let dest_rel = PathBuf::from(&dest_rel_str);

                let stem_anchor = src_rel
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .filter(|s| s.len() >= 3);
                let scan_files = collect_target_files(
                    &ws_canon,
                    &ws_canon,
                    &hub_canon,
                    compiled_glob.as_ref(),
                    stem_anchor,
                );

                let mut import_updates: Vec<(String, PathBuf, String, String, usize)> = Vec::new();
                for f in scan_files {
                    if f == src_abs || cancel.is_cancelled() {
                        continue;
                    }
                    let rel_str = format_display_rel_path(&f, &ws_canon, &hub_canon, "");
                    let Ok(old) =
                        read_jailed_string_async(ctx, &f, FS_MAX_EDIT_FILE_BYTES, &rel_str).await
                    else {
                        continue;
                    };
                    let (next, count) = rewrite_relative_imports_for_move(
                        Path::new(&rel_str),
                        &src_rel,
                        &dest_rel,
                        syntax_for_path(&f),
                        &old,
                    );
                    if count == 0 {
                        continue;
                    }
                    if !dry_run
                        && let Err(stale) = check_existing_file_freshness(ctx, &rel_str, &f, &old, true)
                    {
                        return Ok(ToolOutput::err(stale));
                    }
                    import_updates.push((rel_str, f, old, next, count));
                }

                let mut diffs = vec![format!("move {src_rel_str} -> {dest_rel_str}")];
                let mut edited_files = vec![dest_rel_str.clone()];
                let mut total_import_rewrites = 0usize;

                for (rel, path, old, next, count) in &import_updates {
                    total_import_rewrites += *count;
                    diffs.push(unified_diff(rel, old, next));
                    edited_files.push(rel.clone());
                    if !dry_run {
                        atomic_write_in_ctx(ctx, path, next).await?;
                        record_fresh_file_state(ctx, path, next);
                    }
                }
                if !dry_run {
                    atomic_write_in_ctx(ctx, &dest_abs, &src_body).await?;
                    remove_jailed_in_ctx(ctx, &src_abs).await?;
                    forget_fresh_file_state(ctx, &src_abs);
                    record_fresh_file_state(ctx, &dest_abs, &src_body);
                }

                let verb = match dry_run {
                    true => "[dry-run] Would move",
                    false => "Moved",
                };
                let summary = format!(
                    "{verb} `{src_rel_str}` -> `{dest_rel_str}` and updated {total_import_rewrites} relative import(s) across {} file(s).",
                    import_updates.len()
                );
                Ok(ok_meta(
                    summary,
                    Some(diffs.join("\n")),
                    json!({
                        "dry_run": dry_run,
                        "moved_from": src_rel_str,
                        "moved_to": dest_rel_str,
                        "import_rewrites": total_import_rewrites,
                        "edited_files": edited_files,
                    }),
                ))
            }
        }
    }
}

async fn commit_or_preview_batch(
    ctx: &ToolContext,
    action_label: &str,
    dry_run: bool,
    planned: Vec<(String, PathBuf, String, String, usize)>,
) -> Result<ToolOutput> {
    if planned.is_empty() {
        return Ok(ok_meta(
            format!("`{action_label}` matched 0 files (no changes)."),
            None,
            json!({ "dry_run": dry_run, "files": 0, "replacements": 0 }),
        ));
    }

    let mut diffs = Vec::with_capacity(planned.len());
    let mut edited_files = Vec::with_capacity(planned.len());
    let mut total_replacements = 0usize;

    for (rel, path, old, next, count) in planned {
        total_replacements += count;
        diffs.push(unified_diff(&rel, &old, &next));
        edited_files.push(rel);
        if !dry_run {
            atomic_write_in_ctx(ctx, &path, &next).await?;
            record_fresh_file_state(ctx, &path, &next);
        }
    }

    let verb = match dry_run {
        true => "[dry-run] Would apply",
        false => "Applied",
    };
    let summary = format!(
        "{verb} `{action_label}`: {total_replacements} replacement(s) across {} file(s) ({}).",
        edited_files.len(),
        edited_files.join(", ")
    );
    Ok(ok_meta(
        summary,
        Some(diffs.join("\n")),
        json!({
            "dry_run": dry_run,
            "files": edited_files.len(),
            "replacements": total_replacements,
            "edited_files": edited_files,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx(dir: &Path) -> ToolContext {
        ToolContext {
            session_id: "s-ref".into(),
            workspace_dir: dir.to_path_buf(),
            data_dir: dir.to_path_buf(),
            hub_dir: dir.to_path_buf(),
            shell_timeout_secs: 10,
            auto_background_after_secs: 0,
            max_output_bytes: 4096,
            headless_no_wait: false,
            demote_bg: std::sync::Arc::new(tokio::sync::Notify::new()),
            file_state: None,
        }
    }

    #[tokio::test]
    async fn code_refactor_handles_stats_struct_search_rewrite_replace_move_and_diff() {
        let dir = std::env::temp_dir().join(format!("zenko_refactor_{}", zenko_core::new_id()));
        std::fs::create_dir_all(dir.join("src/utils")).expect("mkdir");
        std::fs::write(
            dir.join("src/utils/tax.ts"),
            "// legacyCalc comment\nexport function legacyCalc(a: number, b: number) { return a + b; }\n",
        )
        .expect("tax");
        std::fs::write(
            dir.join("src/index.ts"),
            "import { legacyCalc } from './utils/tax'\nconst x = legacyCalc(\n  10,\n  20,\n)\n",
        )
        .expect("index");
        let ctx = test_ctx(&dir);
        let cancel = CancellationToken::new();

        // 1. Tokei stats
        let st = CodeRefactorTool
            .execute(&ctx, json!({ "action": "stats" }), cancel.clone())
            .await
            .expect("stats");
        assert!(!st.is_error && st.content.contains("TypeScript"));

        // 2. Structural search across multiline call
        let ss = CodeRefactorTool
            .execute(
                &ctx,
                json!({ "action": "struct_search", "pattern": "legacyCalc($A, $B)" }),
                cancel.clone(),
            )
            .await
            .expect("struct_search");
        assert!(!ss.is_error && ss.content.contains("src/index.ts"));

        // 3. Dry-run code-only replace leaves disk untouched
        let dry = CodeRefactorTool
            .execute(
                &ctx,
                json!({
                    "action": "replace",
                    "find": "legacyCalc",
                    "replace": "computeTax",
                    "dry_run": true
                }),
                cancel.clone(),
            )
            .await
            .expect("dry replace");
        assert!(!dry.is_error && dry.content.contains("[dry-run]"));
        assert!(std::fs::read_to_string(dir.join("src/index.ts")).expect("r").contains("legacyCalc"));

        // 4. Live code-only replace updates code while leaving comment intact!
        let live = CodeRefactorTool
            .execute(
                &ctx,
                json!({
                    "action": "replace",
                    "find": "legacyCalc",
                    "replace": "computeTax",
                    "code_only": true
                }),
                cancel.clone(),
            )
            .await
            .expect("live replace");
        assert!(!live.is_error);
        let tax_after = std::fs::read_to_string(dir.join("src/utils/tax.ts")).expect("tax after");
        assert!(tax_after.contains("// legacyCalc comment"));
        assert!(tax_after.contains("export function computeTax"));

        // 5. Move file + rewrite relative imports
        let mv = CodeRefactorTool
            .execute(
                &ctx,
                json!({
                    "action": "move_imports",
                    "path": "src/utils/tax.ts",
                    "move_to": "src/billing/tax.ts"
                }),
                cancel.clone(),
            )
            .await
            .expect("move_imports");
        assert!(!mv.is_error, "move err: {}", mv.content);
        assert!(!dir.join("src/utils/tax.ts").exists());
        assert!(dir.join("src/billing/tax.ts").exists());
        assert!(
            std::fs::read_to_string(dir.join("src/index.ts"))
                .expect("idx")
                .contains("'./billing/tax'")
        );

        // 6. Structural diff ignores formatting-only differences
        std::fs::write(dir.join("a.rs"), "pub fn f(x: i32) -> i32 { x + 1 }\n").expect("a");
        std::fs::write(dir.join("b.rs"), "pub fn f(\n    x: i32,\n) -> i32 {\n    x + 1\n}\n")
            .expect("b");
        let sd = CodeRefactorTool
            .execute(
                &ctx,
                json!({ "action": "struct_diff", "path": "a.rs", "other_path": "b.rs" }),
                cancel,
            )
            .await
            .expect("struct_diff");
        assert!(!sd.is_error && sd.content.contains("0 semantic changes"));

        let _ = std::fs::remove_dir_all(dir);
    }
}
