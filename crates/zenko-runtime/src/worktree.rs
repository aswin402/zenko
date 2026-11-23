//! Async Git worktree isolation & merge manager for Phase 5 subagents.
//! Uses 100% non-blocking `tokio::process::Command`, zero regex, and canonical path jailing
//! inside `~/.zenko/worktrees/`.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use zenko_core::{
    Result, SAFE_SHELL_ENV_KEYS, SUBAGENT_DIFF_PREVIEW_MAX_CHARS, WORKTREE_BRANCH_PREFIX,
    WORKTREE_BRANCH_SUFFIX_MAX_CHARS, WORKTREE_DIRTY_MAX_FILE_BYTES,
    WORKTREE_DIRTY_MAX_UNTRACKED_FILES, WORKTREE_GIT_AUTHOR_EMAIL, WORKTREE_GIT_AUTHOR_NAME,
    WORKTREE_UNTRACKED_PREVIEW_MAX_CHARS, WORKTREE_UNTRACKED_PREVIEW_MAX_FILES, WORKTREES_DIR_NAME,
    WorktreeDiffReport, WorktreePruneReport, ZenkoError,
};

#[derive(Debug, Clone)]
pub struct WorktreeHandle {
    pub agent_id: String,
    pub repo_root: PathBuf,
    pub worktree_path: PathBuf,
    pub branch: String,
    pub base_commit: String,
    pub dirty_baseline: bool,
}

async fn run_git(cwd: &Path, args: &[&str]) -> std::result::Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd.env_clear();
    for key in SAFE_SHELL_ENV_KEYS {
        if let Ok(val) = std::env::var(key) {
            cmd.env(key, val);
        }
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0");

    let out = cmd
        .output()
        .await
        .map_err(|e| format!("failed to execute git: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    match out.status.success() {
        true => Ok(stdout),
        false => Err(match stderr.trim().is_empty() {
            true => stdout.trim().to_string(),
            false => stderr.trim().to_string(),
        }),
    }
}

/// Resolves the canonical git repository top-level directory for `dir`, or `None` if not inside git.
pub async fn resolve_git_toplevel(dir: &Path) -> Option<PathBuf> {
    let raw = run_git(dir, &["rev-parse", "--show-toplevel"]).await.ok()?;
    let trimmed = raw.trim();
    match trimmed.is_empty() {
        true => None,
        false => Some(PathBuf::from(trimmed)),
    }
}

fn sanitize_branch_suffix(agent_id: &str) -> String {
    let cleaned: String = agent_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    match cleaned.len() > WORKTREE_BRANCH_SUFFIX_MAX_CHARS {
        true => cleaned[cleaned.len() - WORKTREE_BRANCH_SUFFIX_MAX_CHARS..].to_string(),
        false => match cleaned.is_empty() {
            true => "worker".to_string(),
            false => cleaned,
        },
    }
}

/// Deterministic `git status --porcelain=v1` parser (zero regex).
fn parse_porcelain_paths(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in raw.lines() {
        if line.len() < 4 {
            continue;
        }
        let rest = &line[3..];
        let path_part = match rest.split_once(" -> ") {
            Some((_, dst)) => dst.trim(),
            None => rest.trim(),
        };
        let unquoted = path_part
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(path_part);
        if !unquoted.is_empty() {
            out.push(unquoted.to_string());
        }
    }
    out
}

/// Spawns an isolated git worktree on branch `zenko/agent-<suffix>` under `<data_dir>/worktrees/<agent_id>`.
/// When `include_uncommitted` is true, snapshots dirty tracked edits via non-invasive `git stash create`
/// (never touches `refs/stash` or parent workspace) and copies small untracked non-ignored files into the worktree.
pub async fn create_worktree(
    data_dir: &Path,
    workspace_dir: &Path,
    agent_id: &str,
    include_uncommitted: bool,
) -> Result<WorktreeHandle> {
    let repo_root = resolve_git_toplevel(workspace_dir).await.ok_or_else(|| {
        ZenkoError::Tool(format!(
            "Workspace `{}` is not inside a git repository; worktree isolation requires git.",
            workspace_dir.display()
        ))
    })?;

    let base_commit = run_git(&repo_root, &["rev-parse", "--verify", "HEAD"])
        .await
        .map_err(|e| {
            ZenkoError::Tool(format!(
                "Git repository `{}` has no initial commit yet (`HEAD` unresolved): {e}",
                repo_root.display()
            ))
        })?
        .trim()
        .to_string();

    let worktrees_root = data_dir.join(WORKTREES_DIR_NAME);
    tokio::fs::create_dir_all(&worktrees_root).await?;
    let safe_dir_name: String = agent_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let worktree_path = worktrees_root.join(match safe_dir_name.is_empty() {
        true => "agent".to_string(),
        false => safe_dir_name,
    });
    let branch = format!("{WORKTREE_BRANCH_PREFIX}{}", sanitize_branch_suffix(agent_id));
    let wt_str = worktree_path.display().to_string();

    run_git(&repo_root, &["worktree", "add", "-b", &branch, &wt_str, "HEAD"])
        .await
        .map_err(|e| ZenkoError::Tool(format!("git worktree add failed: {e}")))?;

    let mut dirty_baseline = false;
    if include_uncommitted {
        // 1. Non-invasive snapshot of tracked edits (`git stash create` never mutates `refs/stash`)
        if let Ok(stash_sha) = run_git(&repo_root, &["stash", "create"]).await {
            let sha = stash_sha.trim();
            if !sha.is_empty() {
                let applied = run_git(&worktree_path, &["stash", "apply", "--index", sha])
                    .await
                    .is_ok()
                    || run_git(&worktree_path, &["stash", "apply", sha])
                        .await
                        .is_ok();
                if applied {
                    dirty_baseline = true;
                }
            }
        }
        // 2. Copy small untracked non-ignored files so child sees parent's brand-new files too
        if let Ok(untracked) = run_git(
            &repo_root,
            &["ls-files", "--others", "--exclude-standard"],
        )
        .await
        {
            for rel in untracked
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty() && !s.contains(".."))
                .take(WORKTREE_DIRTY_MAX_UNTRACKED_FILES)
            {
                let src = repo_root.join(rel);
                let dst = worktree_path.join(rel);
                // Use `symlink_metadata` so an untracked symlink pointing outside `repo_root` is NEVER followed!
                if let Ok(meta) = tokio::fs::symlink_metadata(&src).await
                    && meta.file_type().is_file()
                        && !meta.file_type().is_symlink()
                        && meta.len() <= WORKTREE_DIRTY_MAX_FILE_BYTES
                    {
                        if let Some(parent) = dst.parent() {
                            let _ = tokio::fs::create_dir_all(parent).await;
                        }
                        if tokio::fs::copy(&src, &dst).await.is_ok() {
                            dirty_baseline = true;
                        }
                    }
            }
        }
    }

    Ok(WorktreeHandle {
        agent_id: agent_id.to_string(),
        repo_root,
        worktree_path,
        branch,
        base_commit,
        dirty_baseline,
    })
}

/// Inspects committed + uncommitted + untracked changes inside a subagent worktree without mutating git state.
pub async fn inspect_worktree_diff(handle: &WorktreeHandle) -> Result<WorktreeDiffReport> {
    let mut files = BTreeSet::new();
    let base_commit = run_git(&handle.repo_root, &["merge-base", "HEAD", &handle.branch])
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "HEAD".to_string());

    let mut diff_stat = String::new();
    let mut patch_preview = String::new();
    let mut untracked_files: Vec<String> = Vec::new();

    match handle.worktree_path.exists() {
        true => {
            if let Ok(porcelain) = run_git(&handle.worktree_path, &["status", "--porcelain=v1"]).await {
                for p in parse_porcelain_paths(&porcelain) {
                    files.insert(p);
                }
            }
            if let Ok(untracked) = run_git(
                &handle.worktree_path,
                &["ls-files", "--others", "--exclude-standard"],
            )
            .await
            {
                for line in untracked.lines().map(str::trim).filter(|s| !s.is_empty()) {
                    files.insert(line.to_string());
                    untracked_files.push(line.to_string());
                }
            }
            if let Ok(tracked_names) =
                run_git(&handle.worktree_path, &["diff", "--name-only", &base_commit]).await
            {
                for line in tracked_names.lines().map(str::trim).filter(|s| !s.is_empty()) {
                    files.insert(line.to_string());
                }
            }
            if let Ok(stat) = run_git(&handle.worktree_path, &["diff", "--stat", &base_commit]).await {
                diff_stat = stat.trim().to_string();
            }
            if let Ok(patch) = run_git(&handle.worktree_path, &["diff", &base_commit]).await {
                patch_preview = patch;
            }
        }
        false => {
            let rev_range = format!("{base_commit}..{}", handle.branch);
            if let Ok(branch_names) =
                run_git(&handle.repo_root, &["diff", "--name-only", &rev_range]).await
            {
                for line in branch_names.lines().map(str::trim).filter(|s| !s.is_empty()) {
                    files.insert(line.to_string());
                }
                if let Ok(stat) =
                    run_git(&handle.repo_root, &["diff", "--stat", &rev_range]).await
                {
                    diff_stat = stat.trim().to_string();
                }
                if let Ok(patch) = run_git(&handle.repo_root, &["diff", &rev_range]).await {
                    patch_preview = patch;
                }
            }
        }
    }

    // Always include untracked brand-new files in `patch_preview` (even when tracked edits also exist!)
    if !untracked_files.is_empty() && handle.worktree_path.exists() {
        let mut untracked_notes = Vec::new();
        for rel in untracked_files
            .iter()
            .take(WORKTREE_UNTRACKED_PREVIEW_MAX_FILES)
        {
            let full = handle.worktree_path.join(rel);
            if let Ok(meta) = tokio::fs::symlink_metadata(&full).await
                && meta.file_type().is_file() && !meta.file_type().is_symlink()
                    && let Ok(body) = tokio::fs::read_to_string(&full).await {
                        let snippet = truncate_chars(&body, WORKTREE_UNTRACKED_PREVIEW_MAX_CHARS);
                        untracked_notes.push(format!("--- /dev/null\n+++ b/{rel}\n{snippet}"));
                    }
        }
        if !untracked_notes.is_empty() {
            if !patch_preview.trim().is_empty() {
                patch_preview.push_str("\n\n");
            }
            patch_preview.push_str(&untracked_notes.join("\n\n"));
        }
    }
    patch_preview = truncate_chars(&patch_preview, SUBAGENT_DIFF_PREVIEW_MAX_CHARS);

    let changed_files: Vec<String> = files.into_iter().collect();
    if diff_stat.is_empty() && !changed_files.is_empty() {
        diff_stat = format!("{} file(s) changed: {}", changed_files.len(), changed_files.join(", "));
    }

    Ok(WorktreeDiffReport {
        agent_id: handle.agent_id.clone(),
        branch: handle.branch.clone(),
        worktree_path: handle.worktree_path.display().to_string(),
        repo_root: handle.repo_root.display().to_string(),
        base_commit: Some(match handle.base_commit.is_empty() {
            true => base_commit,
            false => handle.base_commit.clone(),
        }),
        dirty_baseline: handle.dirty_baseline,
        has_changes: !changed_files.is_empty(),
        changed_files,
        diff_stat,
        patch_preview,
        merged: false,
        cleaned_up: false,
    })
}

/// Cleanly commits worktree edits onto the subagent branch and merges into the parent repository.
/// Never clobbers uncommitted changes in the main repository; aborts cleanly on merge conflicts.
pub async fn merge_worktree_into_main(
    data_dir: &Path,
    handle: &WorktreeHandle,
    cleanup: bool,
) -> Result<WorktreeDiffReport> {
    let mut report = inspect_worktree_diff(handle).await?;
    if !report.has_changes {
        if cleanup {
            let _ = remove_worktree(data_dir, handle).await;
            report.cleaned_up = true;
        }
        return Ok(report);
    }

    let author_name_cfg = format!("user.name={WORKTREE_GIT_AUTHOR_NAME}");
    let author_email_cfg = format!("user.email={WORKTREE_GIT_AUTHOR_EMAIL}");

    // 1. Commit any uncommitted / untracked changes inside the isolated worktree onto its branch
    if handle.worktree_path.exists() {
        let dirty = run_git(&handle.worktree_path, &["status", "--porcelain=v1"])
            .await
            .unwrap_or_default();
        if !dirty.trim().is_empty() {
            run_git(&handle.worktree_path, &["add", "-A"])
                .await
                .map_err(|e| ZenkoError::Tool(format!("git add in worktree failed: {e}")))?;
            let msg = format!("zenko: subagent {} changes", handle.agent_id);
            run_git(
                &handle.worktree_path,
                &[
                    "-c",
                    &author_name_cfg,
                    "-c",
                    &author_email_cfg,
                    "commit",
                    "-m",
                    &msg,
                ],
            )
            .await
            .map_err(|e| ZenkoError::Tool(format!("git commit in worktree failed: {e}")))?;
        }
    }

    // 2. Guard parent repository against uncommitted tracked edits before merging
    let main_dirty = run_git(
        &handle.repo_root,
        &["status", "--porcelain=v1", "--untracked-files=no"],
    )
    .await
    .unwrap_or_default();
    if !main_dirty.trim().is_empty() {
        return Err(ZenkoError::PolicyDenied(format!(
            "Cannot merge subagent branch `{}` while main repository `{}` has uncommitted tracked changes. Commit or stash main changes first.",
            handle.branch,
            handle.repo_root.display()
        )));
    }

    // 3. Merge into main branch cleanly; abort immediately on conflict so main stays untouched
    if let Err(err) = run_git(
        &handle.repo_root,
        &[
            "-c",
            &author_name_cfg,
            "-c",
            &author_email_cfg,
            "merge",
            "--no-edit",
            &handle.branch,
        ],
    )
    .await
    {
        let _ = run_git(&handle.repo_root, &["merge", "--abort"]).await;
        return Err(ZenkoError::Tool(format!(
            "Merge conflict merging `{}` into main repo (aborted cleanly; worktree preserved at `{}`): {err}",
            handle.branch,
            handle.worktree_path.display()
        )));
    }

    report.merged = true;
    if cleanup {
        let _ = remove_worktree(data_dir, handle).await;
        report.cleaned_up = true;
    }
    Ok(report)
}

/// Removes a subagent's git worktree and temporary branch with canonical path-jail verification.
pub async fn remove_worktree(data_dir: &Path, handle: &WorktreeHandle) -> Result<()> {
    let wt_str = handle.worktree_path.display().to_string();
    let _ = run_git(
        &handle.repo_root,
        &["worktree", "remove", "--force", &wt_str],
    )
    .await;
    let _ = run_git(&handle.repo_root, &["worktree", "prune"]).await;
    let _ = run_git(&handle.repo_root, &["branch", "-D", &handle.branch]).await;

    // Canonical path-jailed fallback cleanup so no orphaned directory remains under ~/.zenko/worktrees/
    let allowed_root = data_dir.join(WORKTREES_DIR_NAME);
    if handle.worktree_path.exists()
        && let (Ok(canon_root), Ok(canon_wt)) = (
            std::fs::canonicalize(&allowed_root),
            std::fs::canonicalize(&handle.worktree_path),
        )
            && canon_wt.starts_with(&canon_root) && canon_wt != canon_root {
                let _ = tokio::fs::remove_dir_all(&canon_wt).await;
            }
    Ok(())
}

/// Deterministically resolves the origin git repo root of a worktree from its `.git` file
/// (`gitdir: <repo>/.git/worktrees/<name>`), supporting multi-repo orphan pruning with zero regex.
async fn resolve_worktree_origin_repo(worktree_path: &Path) -> Option<PathBuf> {
    let dot_git = worktree_path.join(".git");
    let raw = tokio::fs::read_to_string(&dot_git).await.ok()?;
    let gitdir_str = raw
        .lines()
        .next()?
        .trim()
        .strip_prefix("gitdir:")?
        .trim();
    let gitdir = PathBuf::from(gitdir_str);
    // `<repo>/.git/worktrees/<name>` -> parent() = `worktrees`, parent() = `.git`, parent() = `<repo>`
    let worktrees_dir = gitdir.parent()?;
    let dot_git_dir = worktrees_dir.parent()?;
    let repo_root = dot_git_dir.parent()?;
    match repo_root.exists() {
        true => Some(repo_root.to_path_buf()),
        false => None,
    }
}

/// Safely prunes orphaned worktree directories under `<data_dir>/worktrees/` that are not in
/// `preserved_ids` (e.g. crash leftovers after `SIGKILL` or deleted sessions), resolving each
/// orphan's actual origin git repo from its `.git` pointer and pruning stale git worktree refs.
pub async fn prune_orphan_worktrees(
    data_dir: &Path,
    workspace_dir: &Path,
    preserved_ids: &HashSet<String>,
) -> Result<WorktreePruneReport> {
    let worktrees_root = data_dir.join(WORKTREES_DIR_NAME);
    let default_repo = resolve_git_toplevel(workspace_dir).await;
    let mut touched_repos: BTreeSet<PathBuf> = BTreeSet::new();
    if let Some(ref repo) = default_repo {
        touched_repos.insert(repo.clone());
    }
    let mut removed_dirs = Vec::new();

    if worktrees_root.exists()
        && let Ok(canon_root) = std::fs::canonicalize(&worktrees_root)
            && let Ok(mut rd) = tokio::fs::read_dir(&worktrees_root).await {
                while let Ok(Some(ent)) = rd.next_entry().await {
                    let path = ent.path();
                    let name = ent.file_name().to_string_lossy().to_string();
                    if preserved_ids.contains(&name) {
                        continue;
                    }
                    let wt_str = path.display().to_string();
                    let origin_repo = resolve_worktree_origin_repo(&path)
                        .await
                        .or_else(|| default_repo.clone());
                    if let Some(ref repo) = origin_repo {
                        touched_repos.insert(repo.clone());
                        let _ = run_git(repo, &["worktree", "remove", "--force", &wt_str]).await;
                        let branch =
                            format!("{WORKTREE_BRANCH_PREFIX}{}", sanitize_branch_suffix(&name));
                        let _ = run_git(repo, &["branch", "-D", &branch]).await;
                    }
                    if path.exists()
                        && let Ok(canon_wt) = std::fs::canonicalize(&path)
                            && canon_wt.starts_with(&canon_root) && canon_wt != canon_root {
                                let _ = tokio::fs::remove_dir_all(&canon_wt).await;
                            }
                    removed_dirs.push(name);
                }
            }

    for repo in touched_repos {
        let _ = run_git(&repo, &["worktree", "prune"]).await;
    }

    let removed_count = removed_dirs.len();
    Ok(WorktreePruneReport {
        removed_dirs,
        removed_count,
    })
}

fn truncate_chars(s: &str, max: usize) -> String {
    match s.len() <= max {
        true => s.to_string(),
        false => {
            let cut = s.floor_char_boundary(max);
            format!("{}...\n[truncated]", &s[..cut])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenko_core::new_id;

    #[tokio::test]
    async fn worktree_isolation_diff_merge_and_cleanup() {
        let root = std::env::temp_dir().join(format!("zenko_wt_test_{}", new_id()));
        let repo = root.join("repo");
        let data_dir = root.join("data");
        tokio::fs::create_dir_all(&repo).await.expect("mkdir repo");
        tokio::fs::create_dir_all(&data_dir)
            .await
            .expect("mkdir data");

        run_git(&repo, &["init"]).await.expect("git init");
        tokio::fs::write(repo.join("README.md"), "hello main\n")
            .await
            .expect("write readme");
        run_git(&repo, &["add", "."]).await.expect("add");
        run_git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@local",
                "commit",
                "-m",
                "init",
            ],
        )
        .await
        .expect("commit");

        let agent_id = format!("agt_{}", new_id());
        let handle = create_worktree(&data_dir, &repo, &agent_id, false)
            .await
            .expect("create worktree");
        assert!(handle.worktree_path.exists());
        assert!(handle.branch.starts_with(WORKTREE_BRANCH_PREFIX));
        assert!(!handle.base_commit.is_empty());
        assert!(!handle.dirty_baseline);

        // Edit & add file strictly inside isolated worktree
        tokio::fs::write(handle.worktree_path.join("worker.txt"), "isolated change\n")
            .await
            .expect("write worker");
        tokio::fs::write(
            handle.worktree_path.join("README.md"),
            "hello main\nupdated by subagent\n",
        )
        .await
        .expect("edit readme");

        // Main repo MUST remain untouched before merge
        assert!(!repo.join("worker.txt").exists());
        assert_eq!(
            tokio::fs::read_to_string(repo.join("README.md"))
                .await
                .expect("read main readme"),
            "hello main\n"
        );

        let report = inspect_worktree_diff(&handle).await.expect("inspect diff");
        assert!(report.has_changes);
        assert!(report.changed_files.contains(&"worker.txt".to_string()));
        assert!(report.changed_files.contains(&"README.md".to_string()));
        assert!(
            report.patch_preview.contains("updated by subagent")
                && report.patch_preview.contains("isolated change"),
            "patch_preview must include BOTH tracked edits AND untracked new files: {}",
            report.patch_preview
        );

        // Merge cleanly + cleanup worktree and branch
        let merged = merge_worktree_into_main(&data_dir, &handle, true)
            .await
            .expect("merge worktree");
        assert!(merged.merged);
        assert!(merged.cleaned_up);
        assert!(repo.join("worker.txt").exists());
        assert!(!handle.worktree_path.exists());
        let branches = run_git(&repo, &["branch", "--list", &handle.branch])
            .await
            .expect("list branch");
        assert!(branches.trim().is_empty(), "temp branch must be deleted");

        // Dirty-baseline snapshotting (`include_uncommitted = true`) without touching parent refs/stash
        tokio::fs::write(repo.join("README.md"), "dirty parent tracked edit\n")
            .await
            .expect("dirty tracked edit");
        tokio::fs::write(repo.join("untracked_note.txt"), "brand new file\n")
            .await
            .expect("dirty untracked file");
        let dirty_id = format!("agt_{}", new_id());
        let dirty_wt = create_worktree(&data_dir, &repo, &dirty_id, true)
            .await
            .expect("create dirty baseline worktree");
        assert!(dirty_wt.dirty_baseline);
        assert_eq!(
            tokio::fs::read_to_string(dirty_wt.worktree_path.join("README.md"))
                .await
                .expect("wt readme"),
            "dirty parent tracked edit\n"
        );
        assert_eq!(
            tokio::fs::read_to_string(dirty_wt.worktree_path.join("untracked_note.txt"))
                .await
                .expect("wt untracked"),
            "brand new file\n"
        );
        let stash_list = run_git(&repo, &["stash", "list"]).await.unwrap_or_default();
        assert!(stash_list.trim().is_empty(), "refs/stash must remain untouched");

        // Orphan pruning removes unpreserved dirs while keeping preserved ones
        let orphan_dir = data_dir.join(WORKTREES_DIR_NAME).join("orphan_crash_wt");
        tokio::fs::create_dir_all(&orphan_dir).await.expect("mkdir orphan");
        let mut preserved = HashSet::new();
        preserved.insert(dirty_id.clone());
        let pruned = prune_orphan_worktrees(&data_dir, &repo, &preserved)
            .await
            .expect("prune orphans");
        assert!(pruned.removed_dirs.contains(&"orphan_crash_wt".to_string()));
        assert!(!orphan_dir.exists());
        assert!(dirty_wt.worktree_path.exists());
        let _ = remove_worktree(&data_dir, &dirty_wt).await;

        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
