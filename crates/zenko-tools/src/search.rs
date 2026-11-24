use crate::fs_tools::line_hash;
use crate::glob::CompiledGlob;
use crate::jail::{format_display_rel_path, resolve_readable_path};
use async_trait::async_trait;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use ignore::WalkBuilder;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zenko_core::{
    CODE_SEARCH_DEFAULT_MAX_RESULTS, CODE_SEARCH_MAX_CONTEXT_LINES, CODE_SEARCH_MAX_RESULTS_CAP,
    DEFAULT_SEARCH_MAX_FILE_BYTES, DEFAULT_SEARCH_SKIP_DIRS, Result, RiskLevel, TOOL_CODE_SEARCH,
    Tool, ToolConcurrency, ToolContext, ToolOutput, ToolSet, ToolSpec, ZenkoError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchOutputMode {
    Matches,
    FilesOnly,
    Count,
}

impl SearchOutputMode {
    fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "files_only" | "files" => Self::FilesOnly,
            "count" | "counts" => Self::Count,
            _ => Self::Matches,
        }
    }
}

pub struct CodeSearchTool;

#[async_trait]
impl Tool for CodeSearchTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::CodeIntel
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_CODE_SEARCH.into(),
            description: "Ultra-fast native Rust search across workspace or `hub:` files (respects .gitignore and skips build/vendor/binary artifacts). Supports regex or exact `fixed_strings`, zero-regex `file_glob` filtering, `output_mode` (`matches`, `files_only`, `count`), `context_lines`, and optional `with_hashes` (`line:hash|`) for immediate `hash_edit`.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regex or literal pattern to search" },
                    "path": { "type": "string", "description": "Subdirectory or file to search (default '.')" },
                    "file_glob": { "type": "string", "description": "Optional glob filter (e.g. `**/*.rs`, `webui/src/**/*.{ts,tsx}`)" },
                    "fixed_strings": { "type": "boolean", "description": "Treat `pattern` as exact literal text instead of regex (default false)" },
                    "output_mode": { "type": "string", "enum": ["matches", "files_only", "count"], "description": "Result format: `matches` (default), `files_only`, or per-file `count`" },
                    "context_lines": { "type": "integer", "description": "Optional surrounding context lines (0..=5, default 0)" },
                    "with_hashes": { "type": "boolean", "description": "Include 3-char `hash_edit` anchors (`file:line:hash| text`, default false)" },
                    "max_results": { "type": "integer", "description": "Maximum lines/files returned (default 60, cap 200)" }
                },
                "required": ["pattern"]
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
        let pattern = args
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `pattern`".into()))?
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
        let compiled_glob = args
            .get("file_glob")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(CompiledGlob::compile);
        let fixed_strings = args
            .get("fixed_strings")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let output_mode = args
            .get("output_mode")
            .and_then(Value::as_str)
            .map(SearchOutputMode::from_str_loose)
            .unwrap_or(SearchOutputMode::Matches);
        let context_lines = args
            .get("context_lines")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(CODE_SEARCH_MAX_CONTEXT_LINES as u64) as usize;
        let with_hashes = args
            .get("with_hashes")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let max_results = args
            .get("max_results")
            .and_then(Value::as_u64)
            .unwrap_or(CODE_SEARCH_DEFAULT_MAX_RESULTS as u64)
            .clamp(1, CODE_SEARCH_MAX_RESULTS_CAP as u64) as usize;

        let output = tokio::task::spawn_blocking(move || -> Result<String> {
            let matcher = RegexMatcherBuilder::new()
                .case_smart(true)
                .fixed_strings(fixed_strings)
                .build(&pattern)
                .map_err(|e| {
                    ZenkoError::Tool(format!(
                        "invalid search pattern: {e} (hint: pass `fixed_strings: true` for exact literal text)"
                    ))
                })?;

            let mut searcher = SearcherBuilder::new()
                .binary_detection(BinaryDetection::quit(b'\x00'))
                .build();
            let mut rows = Vec::new();

            let mut walker = WalkBuilder::new(&root);
            walker
                .hidden(true)
                .require_git(false)
                .max_filesize(Some(DEFAULT_SEARCH_MAX_FILE_BYTES))
                .filter_entry(|dent| {
                    if dent.depth() == 0 {
                        return true;
                    }
                    let is_dir = dent.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                    if !is_dir {
                        return true;
                    }
                    let name = dent.file_name().to_str().unwrap_or("");
                    !DEFAULT_SEARCH_SKIP_DIRS.contains(&name)
                });

            for dent in walker.build().flatten() {
                if cancel.is_cancelled() {
                    return Err(ZenkoError::Cancelled);
                }
                if !dent.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                    continue;
                }
                let file_path = dent.path();
                let rel_for_glob = file_path
                    .strip_prefix(&root)
                    .or_else(|_| file_path.strip_prefix(&ws_root))
                    .unwrap_or(file_path)
                    .to_string_lossy()
                    .replace('\\', "/");
                if let Some(ref g) = compiled_glob
                    && !g.matches(&rel_for_glob)
                {
                    continue;
                }

                let display_rel =
                    format_display_rel_path(file_path, &ws_root, &hub_root, &rel_for_glob);

                match output_mode {
                    SearchOutputMode::FilesOnly => {
                        let mut found = false;
                        let _ = searcher.search_path(
                            &matcher,
                            file_path,
                            UTF8(|_, _| {
                                found = true;
                                Ok(false)
                            }),
                        );
                        if found {
                            rows.push(display_rel);
                        }
                    }
                    SearchOutputMode::Count => {
                        let mut file_hits = 0usize;
                        let _ = searcher.search_path(
                            &matcher,
                            file_path,
                            UTF8(|_, _| {
                                file_hits += 1;
                                Ok(true)
                            }),
                        );
                        if file_hits > 0 {
                            rows.push(format!("{display_rel}: {file_hits}"));
                        }
                    }
                    SearchOutputMode::Matches if context_lines == 0 => {
                        let _ = searcher.search_path(
                            &matcher,
                            file_path,
                            UTF8(|line_num, line| {
                                if rows.len() >= max_results {
                                    return Ok(false);
                                }
                                let clean = line.trim_end_matches(['\r', '\n']);
                                let formatted = match with_hashes {
                                    true => {
                                        let h = line_hash(clean);
                                        format!("{display_rel}:{line_num}:{h}| {clean}")
                                    }
                                    false => format!("{display_rel}:{line_num}: {clean}"),
                                };
                                rows.push(formatted);
                                Ok(true)
                            }),
                        );
                    }
                    SearchOutputMode::Matches => {
                        let mut hit_lines: Vec<usize> = Vec::new();
                        let _ = searcher.search_path(
                            &matcher,
                            file_path,
                            UTF8(|line_num, _| {
                                if hit_lines.len() >= max_results {
                                    return Ok(false);
                                }
                                hit_lines.push(line_num as usize);
                                Ok(true)
                            }),
                        );
                        if !hit_lines.is_empty()
                            && let Ok(raw_text) = std::fs::read_to_string(file_path)
                        {
                            let all_lines: Vec<&str> = raw_text.lines().collect();
                            let mut emitted_up_to = 0usize;
                            for hit in hit_lines {
                                if rows.len() >= max_results {
                                    break;
                                }
                                let lo = hit.saturating_sub(context_lines).max(1);
                                let hi = hit.saturating_add(context_lines).min(all_lines.len());
                                if emitted_up_to > 0 && lo > emitted_up_to + 1 {
                                    rows.push("--".to_string());
                                }
                                let start = lo.max(emitted_up_to + 1);
                                for ln in start..=hi {
                                    if rows.len() >= max_results {
                                        break;
                                    }
                                    let text = all_lines.get(ln - 1).copied().unwrap_or("");
                                    let sep = match ln == hit {
                                        true => ':',
                                        false => '-',
                                    };
                                    let row = match with_hashes {
                                        true => format!(
                                            "{display_rel}{sep}{ln}:{}| {text}",
                                            line_hash(text)
                                        ),
                                        false => format!("{display_rel}{sep}{ln}{sep} {text}"),
                                    };
                                    rows.push(row);
                                    emitted_up_to = ln;
                                }
                            }
                        }
                    }
                }

                if rows.len() >= max_results {
                    break;
                }
            }

            match rows.is_empty() {
                true => Ok("No matches found.".to_string()),
                false => Ok(rows.join("\n")),
            }
        })
        .await
        .map_err(|e| ZenkoError::Tool(format!("search join error: {e}")))??;

        Ok(ToolOutput::ok(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Notify;

    #[tokio::test]
    async fn code_search_supports_glob_fixed_strings_modes_and_hashes() {
        let dir = std::env::temp_dir().join(format!("zenko_search_v2_{}", zenko_core::new_id()));
        std::fs::create_dir_all(dir.join("src")).expect("src");
        std::fs::write(
            dir.join("src/a.rs"),
            "fn alpha() {\n    let v: Vec<String> = Vec::new();\n    println!(\"{v:?}\");\n}\n",
        )
        .expect("a.rs");
        std::fs::write(dir.join("src/b.txt"), "Vec<String> here too\n").expect("b.txt");

        let ctx = ToolContext {
            session_id: "s1".into(),
            workspace_dir: dir.clone(),
            data_dir: dir.clone(),
            hub_dir: dir.clone(),
            shell_timeout_secs: 10,
            auto_background_after_secs: 0,
            max_output_bytes: 16_384,
            headless_no_wait: true,
            demote_bg: Arc::new(Notify::new()),
        };

        let rs_only = CodeSearchTool
            .execute(
                &ctx,
                json!({
                    "pattern": "Vec<String>",
                    "fixed_strings": true,
                    "file_glob": "**/*.rs",
                    "with_hashes": true,
                    "context_lines": 1
                }),
                CancellationToken::new(),
            )
            .await
            .expect("search rs");
        assert!(!rs_only.is_error);
        assert!(rs_only.content.contains("src/a.rs:2:"));
        assert!(rs_only.content.contains("|     let v: Vec<String>"));
        assert!(!rs_only.content.contains("src/b.txt"));

        let files_only = CodeSearchTool
            .execute(
                &ctx,
                json!({
                    "pattern": "Vec<String>",
                    "fixed_strings": true,
                    "output_mode": "files_only"
                }),
                CancellationToken::new(),
            )
            .await
            .expect("files_only");
        assert!(files_only.content.contains("src/a.rs"));
        assert!(files_only.content.contains("src/b.txt"));

        let counts = CodeSearchTool
            .execute(
                &ctx,
                json!({
                    "pattern": "Vec<String>",
                    "fixed_strings": true,
                    "output_mode": "count"
                }),
                CancellationToken::new(),
            )
            .await
            .expect("count");
        assert!(counts.content.contains("src/a.rs: 1"));

        let _ = std::fs::remove_dir_all(dir);
    }
}
