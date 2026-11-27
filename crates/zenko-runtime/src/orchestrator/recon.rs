//! Fast, 5-way parallel read-only reconnaissance executed automatically for `Orchestrated` turns.
//! Uses `tokio::join!` across:
//! 1. Workspace layout + `REPO_MARKER_FILES`
//! 2. Git branch & dirty status (bounded timeout; silently skipped outside git repos)
//! 3. Prompt-referenced file probe (checks existence & line count inside workspace or `~/zenko_hub/`)
//! 4. SQLite FTS5 memory recall + active-branch prior brainstorm/plan continuity
//! 5. Recent ZenkoTasks snapshot
//!
//! Zero LLM tokens and zero side effects.

use crate::jobs::LocalJobSupervisor;
use crate::orchestrator::plan::format_plan_summary;
use crate::orchestrator::triage::extract_path_like_tokens;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::Command;
use zenko_core::{
    BRAIN_RECON_MAX_HITS, BrainstormSpec, CODE_STATS_RECON_MAX_FILES, CODE_STATS_RECON_TOP_LANGS,
    EventRole, ExecutionPlan, HUB_PATH_PREFIX, JobSupervisor, NOTE_KIND_BRAINSTORM_SPEC,
    NOTE_KIND_EXECUTION_PLAN, ORCH_RECON_GIT_TIMEOUT_MS, ORCH_RECON_MAX_FILES,
    ORCH_RECON_MAX_GIT_FILES, ORCH_RECON_MAX_MEMORIES, ORCH_RECON_MAX_MENTIONED_FILES,
    ORCH_RECON_MAX_PROBE_BYTES, ORCH_RECON_MAX_SESSION_HITS, ORCH_RECON_MAX_TASKS,
    REPO_MARKER_FILES, ReconSummary, SessionSearchFilter, compact_preview,
};
use zenko_memory::{
    SqliteStore, filter_query_terms, project_scope_for_workspace, resolve_git_root,
};

fn extract_recon_query(prompt: &str) -> String {
    filter_query_terms(prompt)
        .into_iter()
        .take(8)
        .collect::<Vec<_>>()
        .join(" ")
}

fn scan_workspace_sync(
    workspace_dir: &Path,
) -> (Vec<String>, Vec<String>, Option<String>, Option<String>) {
    let mut entries = Vec::new();
    let mut markers = Vec::new();

    for (filename, label) in REPO_MARKER_FILES {
        if workspace_dir.join(filename).exists() {
            markers.push(format!("{filename} ({label})"));
        }
    }

    if let Ok(rd) = std::fs::read_dir(workspace_dir) {
        let mut names: Vec<(bool, String)> = rd
            .flatten()
            .filter_map(|dent| {
                let name = dent.file_name().to_string_lossy().to_string();
                if name.starts_with('.') && name != ".github" {
                    return None;
                }
                let is_dir = dent.metadata().map(|m| m.is_dir()).unwrap_or(false);
                Some((is_dir, name))
            })
            .collect();
        names.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        for (is_dir, name) in names.into_iter().take(ORCH_RECON_MAX_FILES) {
            entries.push(match is_dir {
                true => format!("{name}/"),
                false => name,
            });
        }
    }

    let lang_stats = zenko_tools::code_stats::scan_workspace_code_stats(
        workspace_dir,
        CODE_STATS_RECON_MAX_FILES,
    );
    let language_summary = zenko_tools::code_stats::format_recon_language_summary(
        &lang_stats,
        CODE_STATS_RECON_TOP_LANGS,
    );
    let service_brief = zenko_tools::service_brief::scan_workspace_service_brief(workspace_dir);

    (entries, markers, language_summary, service_brief)
}

fn probe_mentioned_files_sync(
    workspace_dir: &Path,
    hub_dir: &Path,
    prompt: &str,
) -> Vec<String> {
    let canon_ws = std::fs::canonicalize(workspace_dir).ok();
    let canon_hub = std::fs::canonicalize(hub_dir).ok();
    let mut results = Vec::new();
    for raw in extract_path_like_tokens(prompt)
        .into_iter()
        .take(ORCH_RECON_MAX_MENTIONED_FILES)
    {
        if raw.contains("..") {
            continue;
        }
        let (target, canon_root) = match raw.strip_prefix(HUB_PATH_PREFIX) {
            Some(sub) => (hub_dir.join(sub.trim_start_matches('/')), canon_hub.as_ref()),
            None => (workspace_dir.join(&raw), canon_ws.as_ref()),
        };
        let status = match std::fs::canonicalize(&target) {
            Ok(canon_target) => {
                // Reject symlinks escaping outside workspace_dir / hub_dir
                if !canon_root.is_some_and(|root| canon_target.starts_with(root)) {
                    continue;
                }
                match std::fs::metadata(&canon_target) {
                    Ok(meta) if meta.is_dir() => "dir".to_string(),
                    Ok(meta) if meta.len() <= ORCH_RECON_MAX_PROBE_BYTES => {
                        let lines = std::fs::read_to_string(&canon_target)
                            .map(|s| s.lines().count())
                            .unwrap_or(0);
                        format!("{lines} lines, {}B", meta.len())
                    }
                    Ok(meta) => format!("{}B", meta.len()),
                    Err(_) => "missing".to_string(),
                }
            }
            Err(_) => "missing".to_string(),
        };
        results.push(format!("{raw} ({status})"));
    }
    results
}

async fn probe_git_state(workspace_dir: PathBuf) -> (Option<String>, Vec<String>) {
    let branch_fut = tokio::time::timeout(
        Duration::from_millis(ORCH_RECON_GIT_TIMEOUT_MS),
        Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&workspace_dir)
            .output(),
    );
    let status_fut = tokio::time::timeout(
        Duration::from_millis(ORCH_RECON_GIT_TIMEOUT_MS),
        Command::new("git")
            .args(["status", "--short"])
            .current_dir(&workspace_dir)
            .output(),
    );
    let (branch_res, status_res) = tokio::join!(branch_fut, status_fut);

    let branch = branch_res
        .ok()
        .and_then(std::result::Result::ok)
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    let dirty = status_res
        .ok()
        .and_then(std::result::Result::ok)
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .take(ORCH_RECON_MAX_GIT_FILES)
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    (branch, dirty)
}

fn load_prior_plan_summary_sync(store: &SqliteStore, session_id: &str) -> Option<String> {
    let events = store.get_active_branch(session_id).ok()?;
    for ev in events.iter().rev() {
        if ev.role != EventRole::System {
            continue;
        }
        match ev.content_json.get("note_kind").and_then(|v| v.as_str()) {
            Some(NOTE_KIND_EXECUTION_PLAN) => {
                if let Some(plan) = ev
                    .content_json
                    .get("plan")
                    .and_then(|v| serde_json::from_value::<ExecutionPlan>(v.clone()).ok())
                {
                    return Some(format!("active_plan: {}", format_plan_summary(&plan)));
                }
            }
            Some(NOTE_KIND_BRAINSTORM_SPEC) => {
                if let Some(spec) = ev
                    .content_json
                    .get("spec")
                    .and_then(|v| serde_json::from_value::<BrainstormSpec>(v.clone()).ok())
                {
                    return Some(format!(
                        "prior_brainstorm: selected=`{}` · {}",
                        spec.selected_option_id,
                        compact_preview(&spec.problem, 72)
                    ));
                }
            }
            _ => {}
        }
    }
    None
}

/// Runs 5-way concurrent read-only recon across workspace layout, Git status,
/// prompt-referenced files, SQLite FTS5 memory + prior plan continuity, and ZenkoTasks.
pub async fn run_parallel_recon(
    workspace_dir: PathBuf,
    hub_dir: PathBuf,
    session_id: String,
    prompt: &str,
    store: SqliteStore,
    jobs: Arc<LocalJobSupervisor>,
) -> ReconSummary {
    let started = Instant::now();
    let query = extract_recon_query(prompt);
    let prompt_owned = prompt.to_string();

    let ws_dir_scan = workspace_dir.clone();
    let ws_fut = tokio::task::spawn_blocking(move || scan_workspace_sync(&ws_dir_scan));

    let git_fut = probe_git_state(workspace_dir.clone());

    let probe_ws = workspace_dir.clone();
    let probe_fut = tokio::task::spawn_blocking(move || {
        probe_mentioned_files_sync(&probe_ws, &hub_dir, &prompt_owned)
    });

    let mem_store = store.clone();
    let mem_ws = workspace_dir;
    let mem_fut = tokio::task::spawn_blocking(move || {
        let proj_scope = project_scope_for_workspace(&mem_ws);
        let hits = mem_store
            .search_memory_hybrid(&query, Some(&proj_scope), true, Some(&mem_ws), ORCH_RECON_MAX_MEMORIES)
            .unwrap_or_default()
            .into_iter()
            .map(|h| {
                let stale = match h.stale_paths.is_empty() {
                    true => String::new(),
                    false => format!(" [stale: {}]", h.stale_paths.join(",")),
                };
                format!(
                    "[{}] {}: {}{stale}",
                    h.entry.kind,
                    h.entry.title,
                    compact_preview(&h.entry.content, 80)
                )
            })
            .collect::<Vec<_>>();
        let canonical_ws = resolve_git_root(&mem_ws).display().to_string();
        let sess_hits = mem_store
            .search_sessions_fts(&SessionSearchFilter {
                query: query.clone(),
                workspace_dir: Some(canonical_ws),
                exclude_session_id: Some(session_id.clone()),
                limit: Some(ORCH_RECON_MAX_SESSION_HITS),
                ..Default::default()
            })
            .unwrap_or_default()
            .into_iter()
            .map(|sh| {
                format!(
                    "session={} ({}): {}",
                    zenko_core::truncate_utf8_prefix(&sh.session_id, 12),
                    compact_preview(&sh.session_title, 36),
                    compact_preview(&sh.snippet.replace('\n', " "), 84)
                )
            })
            .collect::<Vec<_>>();
        let prior_plan = load_prior_plan_summary_sync(&mem_store, &session_id);
        let brain_hits = match zenko_memory::brain::has_project_brain(&mem_ws) {
            true => zenko_memory::brain::search_brain_pages(
                &mem_ws,
                &query,
                true,
                BRAIN_RECON_MAX_HITS,
            )
            .into_iter()
            .map(|h| {
                let rev = match h.summary.has_reversals {
                    true => " [reversal]",
                    false => "",
                };
                format!(
                    "`{}` [{}][{}]{rev} {}: {}",
                    h.summary.id,
                    h.summary.category.as_str(),
                    h.summary.status.as_str(),
                    h.summary.title,
                    compact_preview(&h.summary.truth_preview, 84)
                )
            })
            .collect::<Vec<_>>(),
            false => Vec::new(),
        };
        let (okf_summary, okf_hits) = zenko_memory::okf::format_okf_recon(&mem_ws, &query);
        (hits, sess_hits, prior_plan, brain_hits, okf_summary, okf_hits)
    });

    let tasks_fut = tokio::task::spawn_blocking(move || {
        jobs.list_jobs(None, None, ORCH_RECON_MAX_TASKS)
            .unwrap_or_default()
            .into_iter()
            .map(|j| {
                format!(
                    "{} [{}][{}] `{}`",
                    zenko_core::truncate_utf8_prefix(&j.id, 14),
                    j.kind.as_str(),
                    j.status.as_str(),
                    compact_preview(&j.command, 48)
                )
            })
            .collect::<Vec<_>>()
    });

    let (ws_res, git_res, probe_res, mem_res, tasks_res) =
        tokio::join!(ws_fut, git_fut, probe_fut, mem_fut, tasks_fut);
    let (workspace_entries, repo_markers, language_summary, service_brief) =
        ws_res.unwrap_or_default();
    let (git_branch, git_dirty_files) = git_res;
    let mentioned_files = probe_res.unwrap_or_default();
    let (memory_hits, session_hits, prior_plan_summary, brain_hits, okf_summary, okf_hits) =
        mem_res.unwrap_or_default();
    let active_tasks = tasks_res.unwrap_or_default();

    ReconSummary {
        workspace_entries,
        repo_markers,
        language_summary,
        service_brief,
        okf_summary,
        okf_hits,
        memory_hits,
        session_hits,
        brain_hits,
        active_tasks,
        git_branch,
        git_dirty_files,
        mentioned_files,
        prior_plan_summary,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

pub fn format_recon_context(summary: &ReconSummary) -> String {
    let mut lines = Vec::with_capacity(10);
    if !summary.repo_markers.is_empty() {
        lines.push(format!("repo_stack: {}", summary.repo_markers.join(", ")));
    }
    if let Some(ref langs) = summary.language_summary {
        lines.push(format!("languages: {langs}"));
    }
    if let Some(ref brief) = summary.service_brief {
        lines.push(format!("service_brief: {brief}"));
    }
    if let Some(ref okf_sum) = summary.okf_summary {
        lines.push(format!("okf_knowledge: {okf_sum}"));
    }
    if !summary.okf_hits.is_empty() {
        lines.push(format!(
            "okf_concepts (verify `[STALE]`/`[DRIFTED]` entries before trusting; inspect via `invoke_tool(tool=\"{}\", args={{\"action\":\"read\",\"id\":\"...\"}})`):\n  - {}",
            zenko_core::TOOL_KNOWLEDGE_CATALOG,
            summary.okf_hits.join("\n  - ")
        ));
    }
    if let Some(ref br) = summary.git_branch {
        let dirty_suffix = match summary.git_dirty_files.is_empty() {
            true => "clean".to_string(),
            false => format!(
                "{} dirty ({})",
                summary.git_dirty_files.len(),
                summary.git_dirty_files.join(", ")
            ),
        };
        lines.push(format!("git: branch={br} ({dirty_suffix})"));
    }
    if !summary.mentioned_files.is_empty() {
        lines.push(format!(
            "prompt_files: {}",
            summary.mentioned_files.join(", ")
        ));
    }
    if !summary.workspace_entries.is_empty() {
        lines.push(format!(
            "top_entries: {}",
            summary.workspace_entries.join(", ")
        ));
    }
    if let Some(ref prior) = summary.prior_plan_summary {
        lines.push(prior.clone());
    }
    if !summary.brain_hits.is_empty() {
        lines.push(format!(
            "project_brain_truth (respect prior decisions & reversals; inspect via `invoke_tool(tool=\"{}\", args={{\"action\":\"read_page\",\"id\":\"...\"}})`):\n  - {}",
            zenko_core::TOOL_PROJECT_BRAIN,
            summary.brain_hits.join("\n  - ")
        ));
    }
    if !summary.memory_hits.is_empty() {
        lines.push(format!(
            "relevant_memory:\n  - {}",
            summary.memory_hits.join("\n  - ")
        ));
    }
    if !summary.session_hits.is_empty() {
        lines.push(format!(
            "related_past_sessions:\n  - {}",
            summary.session_hits.join("\n  - ")
        ));
    }
    if !summary.active_tasks.is_empty() {
        lines.push(format!(
            "recent_tasks:\n  - {}",
            summary.active_tasks.join("\n  - ")
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn recon_probe_blocks_symlink_escape_outside_workspace() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!("zenko_recon_sym_{}", zenko_core::new_id()));
        let ws = base.join("ws");
        let hub = base.join("hub");
        let outside = base.join("secret.txt");
        let _ = std::fs::create_dir_all(&ws);
        let _ = std::fs::create_dir_all(&hub);
        let _ = std::fs::write(&outside, "TOP_SECRET_LINE\n");
        let _ = std::fs::write(ws.join("safe.rs"), "fn ok() {}\nfn two() {}\n");
        let _ = symlink(&outside, ws.join("escape.rs"));

        let probed = probe_mentioned_files_sync(&ws, &hub, "Inspect safe.rs and escape.rs");
        assert_eq!(probed.len(), 1, "escaped symlink must be skipped, got {probed:?}");
        assert!(probed[0].starts_with("safe.rs (2 lines,"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn recon_emits_service_brief_only_when_catalog_or_codeowners_present() {
        let base = std::env::temp_dir().join(format!("zenko_recon_svc_{}", zenko_core::new_id()));
        let ws = base.join("ws");
        let _ = std::fs::create_dir_all(ws.join(".github"));

        let (_, _, _, none_brief) = scan_workspace_sync(&ws);
        assert!(none_brief.is_none(), "empty repo must emit zero service_brief bytes");

        let _ = std::fs::write(
            ws.join("catalog-info.yaml"),
            "kind: Component\nmetadata:\n  name: billing-api\nspec:\n  owner: @billing-team\n  lifecycle: production\n",
        );
        let _ = std::fs::write(ws.join(".github/CODEOWNERS"), "* @billing-team\n");

        let (_, _, _, brief) = scan_workspace_sync(&ws);
        let brief_str = brief.expect("service brief populated");
        assert!(brief_str.contains("Component/billing-api"));
        assert!(brief_str.contains("owner=@billing-team"));
        assert!(brief_str.contains("codeowners=@billing-team"));

        let formatted = format_recon_context(&ReconSummary {
            service_brief: Some(brief_str),
            ..ReconSummary::default()
        });
        assert!(formatted.contains("service_brief: Component/billing-api"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn recon_emits_okf_concepts_with_trust_badges_only_when_catalog_exists() {
        let base = std::env::temp_dir().join(format!("zenko_recon_okf_{}", zenko_core::new_id()));
        let ws = base.join("ws");
        let _ = std::fs::create_dir_all(&ws);

        let (none_sum, none_hits) = zenko_memory::okf::format_okf_recon(&ws, "auth");
        assert!(none_sum.is_none());
        assert!(none_hits.is_empty());

        let knowledge = ws.join("knowledge");
        let _ = std::fs::create_dir_all(&knowledge);
        let _ = std::fs::write(
            knowledge.join("auth.md"),
            "---\ntype: Architecture\ntitle: OAuth Flow\nverified_by: sec\nverified_at: 2099-01-01\nstale_after: 2099-12-31\n---\nToken validation pipeline.\n",
        );

        let (some_sum, some_hits) = zenko_memory::okf::format_okf_recon(&ws, "oauth validation");
        assert!(some_sum.as_deref().is_some_and(|s| s.contains("1 concepts (1 verified")));
        assert_eq!(some_hits.len(), 1);
        assert!(some_hits[0].contains("[VERIFIED]"));

        let formatted = format_recon_context(&ReconSummary {
            okf_summary: some_sum,
            okf_hits: some_hits,
            ..ReconSummary::default()
        });
        assert!(formatted.contains("okf_knowledge: 1 concepts"));
        assert!(formatted.contains("okf_concepts"));
        assert!(formatted.contains(zenko_core::TOOL_KNOWLEDGE_CATALOG));

        let _ = std::fs::remove_dir_all(&base);
    }
}
