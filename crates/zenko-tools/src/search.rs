use crate::jail::resolve_readable_path;
use async_trait::async_trait;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use ignore::WalkBuilder;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zenko_core::{
    DEFAULT_SEARCH_MAX_FILE_BYTES, DEFAULT_SEARCH_SKIP_DIRS, Result, RiskLevel, Tool,
    ToolConcurrency, ToolContext, ToolOutput, ToolSpec, ZenkoError,
};

pub struct CodeSearchTool;

#[async_trait]
impl Tool for CodeSearchTool {
    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "code_search".into(),
            description: "Ultra-fast native Rust regex search across workspace files (respects .gitignore and skips build/vendor/binary artifacts).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regex or literal pattern to search" },
                    "path": { "type": "string", "description": "Subdirectory to search (default '.')" },
                    "max_results": { "type": "integer", "description": "Maximum matching lines (default 60)" }
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
        let max_results = args
            .get("max_results")
            .and_then(Value::as_u64)
            .unwrap_or(60)
            .clamp(1, 200) as usize;

        let output = tokio::task::spawn_blocking(move || -> Result<String> {
            let matcher = RegexMatcherBuilder::new()
                .case_smart(true)
                .build(&pattern)
                .map_err(|e| ZenkoError::Tool(format!("invalid regex: {e}")))?;

            let mut searcher = SearcherBuilder::new()
                .binary_detection(BinaryDetection::quit(b'\x00'))
                .build();
            let mut matches = Vec::new();

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
                let rel = file_path
                    .strip_prefix(&root)
                    .or_else(|_| file_path.strip_prefix(&ws_root))
                    .unwrap_or(file_path)
                    .display()
                    .to_string();

                let _ = searcher.search_path(
                    &matcher,
                    file_path,
                    UTF8(|line_num, line| {
                        if matches.len() >= max_results {
                            return Ok(false);
                        }
                        matches.push(format!("{rel}:{line_num}: {}", line.trim_end()));
                        Ok(true)
                    }),
                );
                if matches.len() >= max_results {
                    break;
                }
            }

            match matches.is_empty() {
                true => Ok("No matches found.".to_string()),
                false => Ok(matches.join("\n")),
            }
        })
        .await
        .map_err(|e| ZenkoError::Tool(format!("search join error: {e}")))??;

        Ok(ToolOutput::ok(output))
    }
}
