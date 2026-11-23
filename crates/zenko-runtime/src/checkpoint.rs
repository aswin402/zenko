//! Phase 6 Non-Invasive Workspace Checkpoints & 3-Mode Safe Rewind Engine.
//!
//! Architecture (synthesizing Hermes Agent's shadow Git store + hand-edit preservation,
//! Claude Code's 3-mode rewind, and Oh My Pi's non-destructive branch switch + lesson summary):
//! - Primary backend (`CheckpointBackend::ShadowGit`): isolated bare Git object store under
//!   `<data_dir>/checkpoints/git/<workspace_hash>.git` using an ephemeral `GIT_INDEX_FILE`.
//!   NEVER touches `<workspace>/.git`, never pollutes `git status`/`git stash`, works identically
//!   in both Git and non-Git workspaces, and catches BOTH file-tool AND shell-command mutations.
//! - Pure-Rust fallback (`CheckpointBackend::ContentBlobs`): SHA-256 content-addressed blob
//!   store under `<data_dir>/checkpoints/blobs/` used automatically if `git` is unavailable.
//! - Both backends share the exact same symlink-safe restore algebra, Hermes-style manual
//!   hand-edit preservation (`latest_post_ref` comparison), granular file filtering, and
//!   automatic pre-restore safety backups (making Undo itself 100% undoable).

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use similar::TextDiff;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::{Mutex as AsyncMutex, broadcast};
use zenko_core::{
    AgentEvent, CHECKPOINT_DIFF_PREVIEW_MAX_CHARS, CHECKPOINT_GIT_AUTHOR_EMAIL,
    CHECKPOINT_GIT_AUTHOR_NAME, CHECKPOINT_GIT_TIMEOUT_SECS, CHECKPOINT_MAX_TRACKED_FILES,
    CHECKPOINT_SKIP_DIRS, CHECKPOINT_TRIGGER_SAFETY, CHECKPOINTS_BLOBS_MANIFESTS_SUBDIR,
    CHECKPOINTS_BLOBS_OBJECTS_SUBDIR, CHECKPOINTS_BLOBS_SUBDIR, CHECKPOINTS_DIR_NAME,
    CHECKPOINTS_GIT_SUBDIR, CheckpointBackend, CheckpointDiffReport, CheckpointPruneReport,
    CheckpointsConfig, EventKind, EventRole, GIT_EMPTY_TREE_SHA, NOTE_KIND_REWIND_SUMMARY,
    REWIND_SUMMARY_EVENT_PREVIEW_CHARS, REWIND_SUMMARY_MAX_EVENTS, Result, RewindMode,
    RewindReport, RewindRequest, UsageStats, WorkspaceCheckpoint, ZenkoError, compact_preview,
    new_id,
};
use zenko_memory::SqliteStore;

#[derive(Debug, Clone, PartialEq, Eq)]
struct TreeEntry {
    oid: String,
    executable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct BlobManifest {
    files: BTreeMap<String, String>,
}

/// Thread-safe, per-workspace serialized checkpoint & rewind manager.
pub struct CheckpointManager {
    data_dir: PathBuf,
    store: SqliteStore,
    events: broadcast::Sender<AgentEvent>,
    workspace_locks: Mutex<HashMap<PathBuf, Arc<AsyncMutex<()>>>>,
}

impl CheckpointManager {
    pub fn new(
        data_dir: PathBuf,
        store: SqliteStore,
        events: broadcast::Sender<AgentEvent>,
    ) -> Arc<Self> {
        Arc::new(Self {
            data_dir,
            store,
            events,
            workspace_locks: Mutex::new(HashMap::new()),
        })
    }

    fn workspace_lock(&self, workspace_dir: &Path) -> Arc<AsyncMutex<()>> {
        let key = canonical_workspace_key(workspace_dir);
        let mut guard = self
            .workspace_locks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard
            .entry(key)
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    fn shadow_repo_path(&self, workspace_dir: &Path) -> PathBuf {
        let key = canonical_workspace_key(workspace_dir);
        let hash = sha256_prefix_16(key.to_string_lossy().as_bytes());
        self.data_dir
            .join(CHECKPOINTS_DIR_NAME)
            .join(CHECKPOINTS_GIT_SUBDIR)
            .join(format!("{hash}.git"))
    }

    fn blobs_root(&self) -> PathBuf {
        self.data_dir
            .join(CHECKPOINTS_DIR_NAME)
            .join(CHECKPOINTS_BLOBS_SUBDIR)
    }

    /// Captures a lightweight pre-mutation workspace checkpoint (at most once per turn unless
    /// `is_safety_backup` is true). Prefers `ShadowGit` and falls back seamlessly to `ContentBlobs`.
    #[allow(clippy::too_many_arguments)]
    pub async fn capture_checkpoint(
        &self,
        cfg: &CheckpointsConfig,
        session_id: &str,
        turn_id: &str,
        event_id: Option<String>,
        workspace_dir: &Path,
        trigger_tool: &str,
        label: String,
        is_safety_backup: bool,
    ) -> Result<WorkspaceCheckpoint> {
        let ws_canon = canonical_workspace_key(workspace_dir);
        let ws_lock = self.workspace_lock(&ws_canon);
        let _guard = ws_lock.lock().await;

        let (backend, pre_ref) = self
            .capture_tree_snapshot(&ws_canon, cfg.max_file_bytes)
            .await?;

        let cp = WorkspaceCheckpoint {
            id: new_id(),
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            event_id,
            workspace_dir: ws_canon.display().to_string(),
            backend,
            pre_ref: pre_ref.clone(),
            post_ref: None,
            trigger_tool: trigger_tool.to_string(),
            label,
            changed_files: Vec::new(),
            is_safety_backup,
            created_at: Utc::now(),
        };
        if backend == CheckpointBackend::ShadowGit {
            self.pin_shadow_git_ref(&ws_canon, &cp.id, "pre", &pre_ref)
                .await;
        }
        self.store.insert_checkpoint(&cp)?;

        // Enforce rolling per-session retention ceiling asynchronously (with mark-and-sweep GC)
        if cfg.max_per_session > 0 {
            if let Ok(stale) = self
                .store
                .prune_session_checkpoints(session_id, cfg.max_per_session)
            {
                for old in &stale {
                    if old.backend == CheckpointBackend::ShadowGit {
                        self.unpin_shadow_git_refs(old).await;
                    }
                }
                if stale.iter().any(|c| c.backend == CheckpointBackend::ContentBlobs) {
                    let _ = self.gc_content_blobs().await;
                }
            }
        }

        let _ = self.events.send(AgentEvent::CheckpointCreated {
            checkpoint: cp.clone(),
        });
        Ok(cp)
    }

    /// Finalizes a turn checkpoint after tool mutations finish by recording `post_ref` and exact
    /// agent-changed files (enabling zero-false-positive manual hand-edit detection on rewind),
    /// and pushes the finalized checkpoint metadata to live SSE subscribers.
    pub async fn finalize_checkpoint(
        &self,
        cfg: &CheckpointsConfig,
        checkpoint_id: &str,
    ) -> Result<Option<WorkspaceCheckpoint>> {
        let Some(mut cp) = self.store.get_checkpoint(checkpoint_id)? else {
            return Ok(None);
        };
        let ws_dir = PathBuf::from(&cp.workspace_dir);
        if !ws_dir.exists() {
            return Ok(Some(cp));
        }
        let ws_lock = self.workspace_lock(&ws_dir);
        let _guard = ws_lock.lock().await;

        let post_ref = self
            .capture_tree_for_backend(&ws_dir, cp.backend, cfg.max_file_bytes)
            .await?;
        if cp.backend == CheckpointBackend::ShadowGit {
            self.pin_shadow_git_ref(&ws_dir, &cp.id, "post", &post_ref)
                .await;
        }
        let pre_map = self
            .read_tree_map(&ws_dir, cp.backend, &cp.pre_ref)
            .await?;
        let post_map = self
            .read_tree_map(&ws_dir, cp.backend, &post_ref)
            .await?;
        let changed = diff_tree_paths(&pre_map, &post_map);
        self.store
            .update_checkpoint_post_state(&cp.id, &post_ref, &changed)?;
        cp.post_ref = Some(post_ref);
        cp.changed_files = changed;
        let _ = self.events.send(AgentEvent::CheckpointCreated {
            checkpoint: cp.clone(),
        });
        Ok(Some(cp))
    }

    /// Computes a dry-run diff preview between `checkpoint_id`'s pre-turn state and the live disk,
    /// flagging any files that were manually hand-edited after the last agent turn.
    pub async fn diff_checkpoint(
        &self,
        cfg: &CheckpointsConfig,
        checkpoint_id: &str,
    ) -> Result<CheckpointDiffReport> {
        let cp = self
            .store
            .get_checkpoint(checkpoint_id)?
            .ok_or_else(|| ZenkoError::NotFound(format!("checkpoint `{checkpoint_id}` not found")))?;
        let ws_dir = PathBuf::from(&cp.workspace_dir);
        let ws_lock = self.workspace_lock(&ws_dir);
        let _guard = ws_lock.lock().await;

        let cur_ref = self
            .capture_tree_for_backend(&ws_dir, cp.backend, cfg.max_file_bytes)
            .await?;
        let pre_map = self.read_tree_map(&ws_dir, cp.backend, &cp.pre_ref).await?;
        let cur_map = self.read_tree_map(&ws_dir, cp.backend, &cur_ref).await?;
        let latest_post_map = self.load_authoritative_post_map(&ws_dir, &cp).await?;

        let changed_files = diff_tree_paths(&pre_map, &cur_map);
        let manual_edit_conflicts =
            detect_manual_edit_conflicts(&changed_files, &cur_map, latest_post_map.as_ref());

        let (diff_stat, patch_preview) = self
            .compute_diff_text(&ws_dir, cp.backend, &cp.pre_ref, &cur_ref, &pre_map, &cur_map)
            .await?;

        Ok(CheckpointDiffReport {
            checkpoint_id: cp.id,
            session_id: cp.session_id,
            workspace_dir: cp.workspace_dir,
            changed_files,
            manual_edit_conflicts,
            diff_stat,
            patch_preview,
        })
    }

    /// Executes 3-mode rewind (`code_and_conversation`, `code_only`, `conversation_only`):
    /// - Auto-creates a pre-restore safety backup before touching disk (undo is itself undoable!).
    /// - Preserves manual hand-edits by default unless `req.overwrite_manual_edits` is true.
    /// - Skips symlinks cleanly (never follows links outside the workspace).
    /// - Non-destructively switches `head_event_id` (preserving abandoned branches in SQLite)
    ///   and optionally records a compact `<rewind_summary>` lesson note.
    pub async fn rewind(
        &self,
        cfg: &CheckpointsConfig,
        req: &RewindRequest,
    ) -> Result<RewindReport> {
        let cp = self
            .store
            .get_checkpoint(&req.checkpoint_id)?
            .ok_or_else(|| {
                ZenkoError::NotFound(format!("checkpoint `{}` not found", req.checkpoint_id))
            })?;
        let ws_dir = PathBuf::from(&cp.workspace_dir);

        let mut restored_files = Vec::new();
        let mut deleted_files = Vec::new();
        let mut preserved_manual_edits = Vec::new();
        let mut skipped_symlinks = 0usize;
        let mut safety_checkpoint_id = None;

        if matches!(
            req.mode,
            RewindMode::CodeAndConversation | RewindMode::CodeOnly
        ) {
            if !ws_dir.exists() {
                return Err(ZenkoError::Tool(format!(
                    "Workspace directory `{}` no longer exists.",
                    ws_dir.display()
                )));
            }
            let ws_lock = self.workspace_lock(&ws_dir);
            let _guard = ws_lock.lock().await;

            // 1. Capture pre-restore safety snapshot so undo itself is 100% reversible
            let cur_ref = self
                .capture_tree_for_backend(&ws_dir, cp.backend, cfg.max_file_bytes)
                .await?;
            let safety_cp = WorkspaceCheckpoint {
                id: new_id(),
                session_id: cp.session_id.clone(),
                turn_id: format!("rewind_{}", new_id()),
                event_id: self
                    .store
                    .get_session(&cp.session_id)?
                    .and_then(|s| s.head_event_id),
                workspace_dir: cp.workspace_dir.clone(),
                backend: cp.backend,
                pre_ref: cur_ref.clone(),
                post_ref: Some(cur_ref.clone()),
                trigger_tool: CHECKPOINT_TRIGGER_SAFETY.into(),
                label: format!("Safety backup before rewind to {}", &cp.id[..cp.id.len().min(8)]),
                changed_files: Vec::new(),
                is_safety_backup: true,
                created_at: Utc::now(),
            };
            self.store.insert_checkpoint(&safety_cp)?;
            safety_checkpoint_id = Some(safety_cp.id.clone());

            // 2. Compute exact per-file diff & manual hand-edit protection set
            let pre_entries = self
                .read_tree_entries(&ws_dir, cp.backend, &cp.pre_ref)
                .await?;
            let pre_map: BTreeMap<String, String> = pre_entries
                .iter()
                .map(|(k, v)| (k.clone(), v.oid.clone()))
                .collect();
            let cur_map = self.read_tree_map(&ws_dir, cp.backend, &cur_ref).await?;
            let latest_post_map = self.load_authoritative_post_map(&ws_dir, &cp).await?;

            let file_filter: Option<BTreeSet<String>> = match req.files.is_empty() {
                true => None,
                false => Some(
                    req.files
                        .iter()
                        .map(|f| normalize_rel_path(f))
                        .filter(|f| !f.is_empty())
                        .collect(),
                ),
            };

            let all_diff = diff_tree_paths(&pre_map, &cur_map);
            let target_paths: Vec<String> = match &file_filter {
                Some(filter) => all_diff
                    .into_iter()
                    .filter(|p| filter.contains(p))
                    .collect(),
                None => all_diff,
            };

            let preserve_manual = cfg.preserve_manual_edits && !req.overwrite_manual_edits;
            let manual_set: BTreeSet<String> = match preserve_manual {
                true => detect_manual_edit_conflicts(
                    &target_paths,
                    &cur_map,
                    latest_post_map.as_ref(),
                )
                .into_iter()
                .collect(),
                false => BTreeSet::new(),
            };

            // 3. Apply exact byte-for-byte restore while blocking symlink leaf & ancestor escapes
            for rel in target_paths {
                if manual_set.contains(&rel) {
                    preserved_manual_edits.push(rel);
                    continue;
                }
                let Some(abs_path) = join_safe_rel_path(&ws_dir, &rel) else {
                    continue;
                };
                if has_symlink_component(&ws_dir, &rel).await {
                    skipped_symlinks += 1;
                    continue;
                }
                match pre_entries.get(&rel) {
                    Some(entry) => {
                        let bytes = self
                            .read_blob_bytes(&ws_dir, cp.backend, &entry.oid)
                            .await?;
                        if let Some(parent) = abs_path.parent() {
                            tokio::fs::create_dir_all(parent).await?;
                        }
                        tokio::fs::write(&abs_path, bytes).await?;
                        #[cfg(unix)]
                        if entry.executable {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = tokio::fs::set_permissions(
                                &abs_path,
                                std::fs::Permissions::from_mode(0o755),
                            )
                            .await;
                        }
                        restored_files.push(rel);
                    }
                    None => {
                        if tokio::fs::remove_file(&abs_path).await.is_ok() {
                            prune_empty_parents(&ws_dir, abs_path.parent()).await;
                            deleted_files.push(rel);
                        }
                    }
                }
            }

            // Record post-restore workspace state onto the safety backup so subsequent rewinds
            // recognize restored disk state cleanly as known state (not manual hand-edits)
            if let Ok(post_restore_ref) = self
                .capture_tree_for_backend(&ws_dir, cp.backend, cfg.max_file_bytes)
                .await
            {
                let mut touched = restored_files.clone();
                touched.extend(deleted_files.iter().cloned());
                let _ = self.store.update_checkpoint_post_state(
                    &safety_cp.id,
                    &post_restore_ref,
                    &touched,
                );
            }
        }

        // 4. Non-destructive conversation branch rewind + optional Oh My Pi lesson summary
        let mut new_head_event_id = None;
        let mut rewind_summary = None;
        if matches!(
            req.mode,
            RewindMode::CodeAndConversation | RewindMode::ConversationOnly
        ) {
            if req.summarize_abandoned {
                rewind_summary = build_abandoned_branch_summary(
                    &self.store,
                    &cp.session_id,
                    cp.event_id.as_deref(),
                );
            }
            self.store
                .set_session_head(&cp.session_id, cp.event_id.as_deref())?;
            new_head_event_id = cp.event_id.clone();

            if let Some(ref sum) = rewind_summary {
                if let Ok(ev) = self.store.append_event(
                    &cp.session_id,
                    cp.event_id.as_deref(),
                    EventRole::System,
                    EventKind::SystemNote,
                    serde_json::json!({
                        "note_kind": NOTE_KIND_REWIND_SUMMARY,
                        "checkpoint_id": cp.id,
                        "mode": req.mode.as_str(),
                        "summary": sum,
                        "text": format!("Rewound to checkpoint {}: {sum}", &cp.id[..cp.id.len().min(8)]),
                    }),
                    &UsageStats::default(),
                ) {
                    new_head_event_id = Some(ev.id);
                }
            }
        }

        let report = RewindReport {
            checkpoint_id: cp.id,
            session_id: cp.session_id,
            mode: req.mode,
            restored_files,
            deleted_files,
            preserved_manual_edits,
            skipped_symlinks,
            safety_checkpoint_id,
            new_head_event_id,
            rewind_summary,
        };
        let _ = self.events.send(AgentEvent::SessionRewound {
            report: report.clone(),
        });
        Ok(report)
    }

    /// Prunes expired checkpoints, unpins their shadow-git refs, and runs Mark-and-Sweep GC.
    pub async fn prune_checkpoints(&self, cfg: &CheckpointsConfig) -> Result<CheckpointPruneReport> {
        let expired = self.store.prune_expired_checkpoints(cfg.retention_days)?;
        let removed_checkpoints = expired.len();
        for old in &expired {
            if old.backend == CheckpointBackend::ShadowGit {
                self.unpin_shadow_git_refs(old).await;
            }
        }
        let reclaimed_bytes = self.gc_content_blobs().await;

        // Run `git gc --auto` across existing shadow repos quietly (active checkpoints remain pinned via refs)
        let git_root = self
            .data_dir
            .join(CHECKPOINTS_DIR_NAME)
            .join(CHECKPOINTS_GIT_SUBDIR);
        if let Ok(mut entries) = tokio::fs::read_dir(&git_root).await {
            while let Ok(Some(ent)) = entries.next_entry().await {
                let p = ent.path();
                if p.is_dir() {
                    let _ = run_git_env(
                        &[("GIT_DIR", p.as_os_str())],
                        &["gc", "--auto", "--quiet"],
                        None,
                    )
                    .await;
                }
            }
        }

        Ok(CheckpointPruneReport {
            removed_checkpoints,
            reclaimed_bytes,
        })
    }

    /// Exact per-file Zenko provenance algebra:
    /// Anchors on the newest finalized non-safety checkpoint (same backend for hash compatibility),
    /// then overlays any newer safety-backup rewinds strictly for paths Zenko actually wrote/deleted
    /// (`safety.changed_files`). This ensures Undo-of-Undo works without `--force` while keeping
    /// preserved human hand-edits protected across chained rewinds.
    async fn load_authoritative_post_map(
        &self,
        ws_dir: &Path,
        cp: &WorkspaceCheckpoint,
    ) -> Result<Option<BTreeMap<String, String>>> {
        let recent: Vec<WorkspaceCheckpoint> = self
            .store
            .recent_finalized_checkpoints_for_workspace(&cp.workspace_dir, 16)?
            .into_iter()
            .filter(|c| c.backend == cp.backend)
            .collect();
        if recent.is_empty() {
            return match cp.post_ref.as_deref() {
                Some(r) => Ok(Some(self.read_tree_map(ws_dir, cp.backend, r).await?)),
                None => Ok(None),
            };
        }

        let base_idx = recent
            .iter()
            .position(|c| !c.is_safety_backup)
            .unwrap_or(recent.len() - 1);
        let base_cp = &recent[base_idx];
        let Some(base_post_ref) = base_cp.post_ref.as_deref() else {
            return Ok(None);
        };
        let mut auth_map = self
            .read_tree_map(ws_dir, base_cp.backend, base_post_ref)
            .await?;

        for safety in recent[..base_idx].iter().rev() {
            let Some(post_ref) = safety.post_ref.as_deref() else {
                continue;
            };
            let post_map = self
                .read_tree_map(ws_dir, safety.backend, post_ref)
                .await?;
            for touched in &safety.changed_files {
                match post_map.get(touched) {
                    Some(hash) => {
                        auth_map.insert(touched.clone(), hash.clone());
                    }
                    None => {
                        auth_map.remove(touched);
                    }
                }
            }
        }
        Ok(Some(auth_map))
    }

    async fn capture_tree_snapshot(
        &self,
        ws_dir: &Path,
        max_file_bytes: u64,
    ) -> Result<(CheckpointBackend, String)> {
        match self.capture_shadow_git_tree(ws_dir, max_file_bytes).await {
            Ok(tree_sha) => Ok((CheckpointBackend::ShadowGit, tree_sha)),
            Err(_) => {
                let manifest_id = self.capture_blob_manifest(ws_dir, max_file_bytes).await?;
                Ok((CheckpointBackend::ContentBlobs, manifest_id))
            }
        }
    }

    async fn capture_tree_for_backend(
        &self,
        ws_dir: &Path,
        backend: CheckpointBackend,
        max_file_bytes: u64,
    ) -> Result<String> {
        match backend {
            CheckpointBackend::ShadowGit => self.capture_shadow_git_tree(ws_dir, max_file_bytes).await,
            CheckpointBackend::ContentBlobs => self.capture_blob_manifest(ws_dir, max_file_bytes).await,
        }
    }

    async fn ensure_shadow_repo(&self, ws_dir: &Path) -> Result<PathBuf> {
        let repo = self.shadow_repo_path(ws_dir);
        if !repo.join("HEAD").exists() {
            tokio::fs::create_dir_all(&repo).await?;
            let (ok, _out, err) = run_git_env(
                &[],
                &["init", "--bare", "--quiet", &repo.display().to_string()],
                None,
            )
            .await?;
            if !ok {
                return Err(ZenkoError::Tool(format!(
                    "shadow git init failed: {}",
                    err.trim()
                )));
            }
            let _ = run_git_env(
                &[("GIT_DIR", repo.as_os_str())],
                &["config", "user.name", CHECKPOINT_GIT_AUTHOR_NAME],
                None,
            )
            .await;
            let _ = run_git_env(
                &[("GIT_DIR", repo.as_os_str())],
                &["config", "user.email", CHECKPOINT_GIT_AUTHOR_EMAIL],
                None,
            )
            .await;
            let _ = run_git_env(
                &[("GIT_DIR", repo.as_os_str())],
                &["config", "gc.pruneExpire", "never"],
                None,
            )
            .await;
            let _ = run_git_env(
                &[("GIT_DIR", repo.as_os_str())],
                &["config", "gc.auto", "0"],
                None,
            )
            .await;
        }
        Ok(repo)
    }

    async fn pin_shadow_git_ref(&self, ws_dir: &Path, cp_id: &str, suffix: &str, tree_sha: &str) {
        if tree_sha.is_empty() || tree_sha == GIT_EMPTY_TREE_SHA {
            return;
        }
        let repo = self.shadow_repo_path(ws_dir);
        if !repo.join("HEAD").exists() {
            return;
        }
        let ref_name = format!("refs/zenko/checkpoints/{cp_id}/{suffix}");
        let _ = run_git_env(
            &[("GIT_DIR", repo.as_os_str())],
            &["update-ref", &ref_name, tree_sha],
            None,
        )
        .await;
    }

    async fn unpin_shadow_git_refs(&self, cp: &WorkspaceCheckpoint) {
        let repo = self.shadow_repo_path(Path::new(&cp.workspace_dir));
        if !repo.join("HEAD").exists() {
            return;
        }
        for suffix in ["pre", "post"] {
            let ref_name = format!("refs/zenko/checkpoints/{}/{suffix}", cp.id);
            let _ = run_git_env(
                &[("GIT_DIR", repo.as_os_str())],
                &["update-ref", "-d", &ref_name],
                None,
            )
            .await;
        }
    }

    async fn capture_shadow_git_tree(&self, ws_dir: &Path, max_file_bytes: u64) -> Result<String> {
        let repo = self.ensure_shadow_repo(ws_dir).await?;
        let rel_files = walk_workspace_safe_files(ws_dir, max_file_bytes).await;
        if rel_files.is_empty() {
            return Ok(GIT_EMPTY_TREE_SHA.to_string());
        }

        let tmp_index = repo.join(format!("idx_{}", new_id()));
        let mut pathspec = Vec::with_capacity(rel_files.len() * 24);
        for rel in &rel_files {
            pathspec.extend_from_slice(rel.as_bytes());
            pathspec.push(0);
        }

        let envs = [
            ("GIT_DIR", repo.as_os_str()),
            ("GIT_WORK_TREE", ws_dir.as_os_str()),
            ("GIT_INDEX_FILE", tmp_index.as_os_str()),
        ];

        let add_res = run_git_env(
            &envs,
            &[
                "add",
                "-f",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ],
            Some(&pathspec),
        )
        .await;

        let tree_res = match add_res {
            Ok((true, _, _)) => run_git_env(&envs, &["write-tree"], None).await,
            Ok((false, _, err)) => Err(ZenkoError::Tool(format!(
                "shadow git add failed: {}",
                err.trim()
            ))),
            Err(e) => Err(e),
        };
        let _ = tokio::fs::remove_file(&tmp_index).await;

        let (ok, stdout, stderr) = tree_res?;
        if !ok {
            return Err(ZenkoError::Tool(format!(
                "shadow git write-tree failed: {}",
                stderr.trim()
            )));
        }
        let sha = stdout.trim().to_string();
        if sha.is_empty() {
            return Err(ZenkoError::Tool("shadow git returned empty tree SHA".into()));
        }
        Ok(sha)
    }

    async fn capture_blob_manifest(&self, ws_dir: &Path, max_file_bytes: u64) -> Result<String> {
        let root = self.blobs_root();
        let obj_dir = root.join(CHECKPOINTS_BLOBS_OBJECTS_SUBDIR);
        let man_dir = root.join(CHECKPOINTS_BLOBS_MANIFESTS_SUBDIR);
        tokio::fs::create_dir_all(&obj_dir).await?;
        tokio::fs::create_dir_all(&man_dir).await?;

        let rel_files = walk_workspace_safe_files(ws_dir, max_file_bytes).await;
        let mut manifest = BlobManifest::default();
        for rel in rel_files {
            let Some(abs) = join_safe_rel_path(ws_dir, &rel) else {
                continue;
            };
            let Ok(bytes) = tokio::fs::read(&abs).await else {
                continue;
            };
            let digest = sha256_hex(&bytes);
            let blob_path = obj_dir.join(&digest);
            if !blob_path.exists() {
                let _ = tokio::fs::write(&blob_path, &bytes).await;
            }
            manifest.files.insert(rel, digest);
        }

        let raw = serde_json::to_vec(&manifest)?;
        let manifest_id = sha256_hex(&raw);
        let manifest_path = man_dir.join(format!("{manifest_id}.json"));
        if !manifest_path.exists() {
            tokio::fs::write(&manifest_path, raw).await?;
        }
        Ok(manifest_id)
    }

    async fn read_tree_entries(
        &self,
        ws_dir: &Path,
        backend: CheckpointBackend,
        tree_ref: &str,
    ) -> Result<BTreeMap<String, TreeEntry>> {
        match backend {
            CheckpointBackend::ShadowGit => {
                if tree_ref == GIT_EMPTY_TREE_SHA {
                    return Ok(BTreeMap::new());
                }
                let repo = self.shadow_repo_path(ws_dir);
                let (ok, stdout, stderr) = run_git_env(
                    &[("GIT_DIR", repo.as_os_str())],
                    &["ls-tree", "-r", "-z", tree_ref],
                    None,
                )
                .await?;
                if !ok {
                    return Err(ZenkoError::Tool(format!(
                        "shadow git ls-tree failed: {}",
                        stderr.trim()
                    )));
                }
                Ok(parse_git_ls_tree_nul(&stdout))
            }
            CheckpointBackend::ContentBlobs => {
                let manifest_path = self
                    .blobs_root()
                    .join(CHECKPOINTS_BLOBS_MANIFESTS_SUBDIR)
                    .join(format!("{tree_ref}.json"));
                let raw = tokio::fs::read(&manifest_path).await.map_err(|e| {
                    ZenkoError::Storage(format!("failed to read checkpoint manifest: {e}"))
                })?;
                let manifest: BlobManifest = serde_json::from_slice(&raw)?;
                Ok(manifest
                    .files
                    .into_iter()
                    .map(|(k, oid)| (k, TreeEntry { oid, executable: false }))
                    .collect())
            }
        }
    }

    async fn read_tree_map(
        &self,
        ws_dir: &Path,
        backend: CheckpointBackend,
        tree_ref: &str,
    ) -> Result<BTreeMap<String, String>> {
        let entries = self.read_tree_entries(ws_dir, backend, tree_ref).await?;
        Ok(entries.into_iter().map(|(k, v)| (k, v.oid)).collect())
    }

    async fn read_blob_bytes(
        &self,
        ws_dir: &Path,
        backend: CheckpointBackend,
        blob_id: &str,
    ) -> Result<Vec<u8>> {
        match backend {
            CheckpointBackend::ShadowGit => {
                let repo = self.shadow_repo_path(ws_dir);
                let (ok, stdout, stderr) = run_git_env_bytes(
                    &[("GIT_DIR", repo.as_os_str())],
                    &["cat-file", "blob", blob_id],
                )
                .await?;
                if !ok {
                    return Err(ZenkoError::Tool(format!(
                        "shadow git cat-file failed: {}",
                        String::from_utf8_lossy(&stderr).trim()
                    )));
                }
                Ok(stdout)
            }
            CheckpointBackend::ContentBlobs => {
                let path = self
                    .blobs_root()
                    .join(CHECKPOINTS_BLOBS_OBJECTS_SUBDIR)
                    .join(blob_id);
                Ok(tokio::fs::read(&path).await?)
            }
        }
    }

    async fn compute_diff_text(
        &self,
        ws_dir: &Path,
        backend: CheckpointBackend,
        pre_ref: &str,
        cur_ref: &str,
        pre_map: &BTreeMap<String, String>,
        cur_map: &BTreeMap<String, String>,
    ) -> Result<(String, String)> {
        if pre_ref == cur_ref {
            return Ok((String::new(), String::new()));
        }
        match backend {
            CheckpointBackend::ShadowGit => {
                let repo = self.shadow_repo_path(ws_dir);
                let envs = [("GIT_DIR", repo.as_os_str())];
                let (_, stat_out, _) =
                    run_git_env(&envs, &["diff-tree", "-r", "--stat", pre_ref, cur_ref], None)
                        .await?;
                let (_, patch_out, _) = run_git_env(
                    &envs,
                    &["diff-tree", "-p", "--no-color", pre_ref, cur_ref],
                    None,
                )
                .await?;
                Ok((
                    stat_out.trim().to_string(),
                    truncate_chars(&patch_out, CHECKPOINT_DIFF_PREVIEW_MAX_CHARS),
                ))
            }
            CheckpointBackend::ContentBlobs => {
                let changed = diff_tree_paths(pre_map, cur_map);
                let mut stat_lines = Vec::new();
                let mut patches = Vec::new();
                for rel in changed {
                    let old_bytes = match pre_map.get(&rel) {
                        Some(id) => self.read_blob_bytes(ws_dir, backend, id).await.unwrap_or_default(),
                        None => Vec::new(),
                    };
                    let new_bytes = match cur_map.get(&rel) {
                        Some(id) => self.read_blob_bytes(ws_dir, backend, id).await.unwrap_or_default(),
                        None => Vec::new(),
                    };
                    let old_txt = String::from_utf8_lossy(&old_bytes);
                    let new_txt = String::from_utf8_lossy(&new_bytes);
                    let diff = TextDiff::from_lines(&*old_txt, &*new_txt);
                    let udiff = diff
                        .unified_diff()
                        .context_radius(2)
                        .header(&format!("a/{rel}"), &format!("b/{rel}"))
                        .to_string();
                    stat_lines.push(rel);
                    patches.push(udiff);
                }
                Ok((
                    format!("{} file(s) changed: {}", stat_lines.len(), stat_lines.join(", ")),
                    truncate_chars(&patches.join("\n"), CHECKPOINT_DIFF_PREVIEW_MAX_CHARS),
                ))
            }
        }
    }

    /// Deterministic Mark-and-Sweep GC over content-addressed manifests & blob objects:
    /// 1. Mark all manifests referenced by surviving SQLite `content_blobs` checkpoints.
    /// 2. Sweep unreferenced manifests (never deleting shared content-addressed manifests!).
    /// 3. Mark all blob SHAs referenced by live manifests, then sweep orphaned `objects/*`.
    async fn gc_content_blobs(&self) -> u64 {
        let Ok(live_manifests) = self.store.list_live_blob_manifest_refs() else {
            return 0;
        };
        let man_dir = self.blobs_root().join(CHECKPOINTS_BLOBS_MANIFESTS_SUBDIR);
        let obj_dir = self.blobs_root().join(CHECKPOINTS_BLOBS_OBJECTS_SUBDIR);
        let mut reclaimed = 0u64;
        let mut live_blobs: HashSet<String> = HashSet::new();

        if let Ok(mut rd) = tokio::fs::read_dir(&man_dir).await {
            while let Ok(Some(ent)) = rd.next_entry().await {
                let path = ent.path();
                let fname = ent.file_name().to_string_lossy().into_owned();
                let Some(stem) = fname.strip_suffix(".json") else {
                    continue;
                };
                match live_manifests.contains(stem) {
                    true => {
                        if let Ok(raw) = tokio::fs::read(&path).await {
                            if let Ok(man) = serde_json::from_slice::<BlobManifest>(&raw) {
                                live_blobs.extend(man.files.into_values());
                            }
                        }
                    }
                    false => {
                        if let Ok(meta) = tokio::fs::metadata(&path).await {
                            reclaimed = reclaimed.saturating_add(meta.len());
                        }
                        let _ = tokio::fs::remove_file(&path).await;
                    }
                }
            }
        }

        if let Ok(mut rd) = tokio::fs::read_dir(&obj_dir).await {
            while let Ok(Some(ent)) = rd.next_entry().await {
                let path = ent.path();
                let name = ent.file_name().to_string_lossy().into_owned();
                if !live_blobs.contains(&name) {
                    if let Ok(meta) = tokio::fs::metadata(&path).await {
                        reclaimed = reclaimed.saturating_add(meta.len());
                    }
                    let _ = tokio::fs::remove_file(&path).await;
                }
            }
        }
        reclaimed
    }
}

/// Pure deterministic lexicographical workspace walker using an explicit stack (zero recursion,
/// zero symlinks, skips build/VCS artifacts and files above `max_file_bytes`).
async fn walk_workspace_safe_files(ws_dir: &Path, max_file_bytes: u64) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, String)> = vec![(ws_dir.to_path_buf(), String::new())];

    while let Some((dir, prefix)) = stack.pop() {
        let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        let mut children: Vec<(String, PathBuf)> = Vec::new();
        while let Ok(Some(ent)) = rd.next_entry().await {
            let name = ent.file_name().to_string_lossy().into_owned();
            children.push((name, ent.path()));
        }
        // Sort ascending, process files in lexicographical order, and push subdirectories in reverse
        // onto the LIFO stack so directory traversal visits `a/` before `z/` deterministically.
        children.sort_by(|a, b| a.0.cmp(&b.0));
        let mut subdirs: Vec<(PathBuf, String)> = Vec::new();

        for (name, path) in children {
            let Ok(meta) = tokio::fs::symlink_metadata(&path).await else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            let rel = match prefix.is_empty() {
                true => name.clone(),
                false => format!("{prefix}/{name}"),
            };
            match (meta.is_dir(), meta.is_file()) {
                (true, _) => {
                    if !CHECKPOINT_SKIP_DIRS.contains(&name.as_str()) {
                        subdirs.push((path, rel));
                    }
                }
                (false, true) if meta.len() <= max_file_bytes => {
                    out.push(rel);
                    if out.len() >= CHECKPOINT_MAX_TRACKED_FILES {
                        out.sort();
                        return out;
                    }
                }
                _ => {}
            }
        }
        for sub in subdirs.into_iter().rev() {
            stack.push(sub);
        }
    }
    out.sort();
    out
}

/// Parses `git ls-tree -r -z` records (`<mode> <type> <sha>\t<path>\0`) deterministically (zero regex),
/// preserving Unix executable mode (`100755`).
fn parse_git_ls_tree_nul(raw: &str) -> BTreeMap<String, TreeEntry> {
    let mut out = BTreeMap::new();
    for rec in raw.split('\0') {
        let Some((header, path)) = rec.split_once('\t') else {
            continue;
        };
        let mut parts = header.split_whitespace();
        let mode = parts.next();
        let obj_type = parts.next();
        let sha = parts.next();
        if obj_type == Some("blob") {
            if let Some(oid) = sha {
                out.insert(
                    path.to_string(),
                    TreeEntry {
                        oid: oid.to_string(),
                        executable: mode == Some("100755"),
                    },
                );
            }
        }
    }
    out
}

fn diff_tree_paths(
    a: &BTreeMap<String, String>,
    b: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut paths: BTreeSet<String> = BTreeSet::new();
    for (k, v) in a {
        if b.get(k) != Some(v) {
            paths.insert(k.clone());
        }
    }
    for (k, v) in b {
        if a.get(k) != Some(v) {
            paths.insert(k.clone());
        }
    }
    paths.into_iter().collect()
}

/// Identifies files edited manually on disk AFTER the last agent turn (`cur != latest_post`).
fn detect_manual_edit_conflicts(
    changed_files: &[String],
    cur_map: &BTreeMap<String, String>,
    latest_post_map: Option<&BTreeMap<String, String>>,
) -> Vec<String> {
    let Some(post_map) = latest_post_map else {
        return Vec::new();
    };
    changed_files
        .iter()
        .filter(|path| cur_map.get(*path) != post_map.get(*path))
        .cloned()
        .collect()
}

fn build_abandoned_branch_summary(
    store: &SqliteStore,
    session_id: &str,
    target_event_id: Option<&str>,
) -> Option<String> {
    let branch = store.get_active_branch(session_id).ok()?;
    let start_idx = match target_event_id {
        Some(tid) => branch
            .iter()
            .position(|e| e.id == tid)
            .map(|i| i + 1)
            .unwrap_or(0),
        None => 0,
    };
    if start_idx >= branch.len() {
        return None;
    }
    let mut lines = Vec::new();
    for ev in branch[start_idx..].iter().take(REWIND_SUMMARY_MAX_EVENTS) {
        let preview = match ev.kind {
            EventKind::Message => ev
                .content_json
                .get("text")
                .and_then(|v| v.as_str())
                .map(|t| compact_preview(t, REWIND_SUMMARY_EVENT_PREVIEW_CHARS)),
            EventKind::ToolCall => ev
                .content_json
                .get("tool_calls")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    let names: Vec<&str> = arr
                        .iter()
                        .filter_map(|tc| tc.get("name").and_then(|n| n.as_str()))
                        .collect();
                    format!("called tools: {}", names.join(", "))
                }),
            EventKind::ToolResult => {
                let name = ev
                    .content_json
                    .get("tool_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool");
                let is_err = ev
                    .content_json
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                match is_err {
                    true => Some(format!(
                        "{name} failed: {}",
                        compact_preview(
                            ev.content_json
                                .get("content")
                                .and_then(|v| v.as_str())
                                .unwrap_or(""),
                            REWIND_SUMMARY_EVENT_PREVIEW_CHARS,
                        )
                    )),
                    false => None,
                }
            }
            _ => None,
        };
        if let Some(p) = preview.filter(|s| !s.is_empty()) {
            lines.push(format!("- [{}] {p}", ev.role.as_str()));
        }
    }
    match lines.is_empty() {
        true => None,
        false => Some(format!(
            "Abandoned branch ({} event(s)) before rewind:\n{}",
            branch.len() - start_idx,
            lines.join("\n")
        )),
    }
}

fn canonical_workspace_key(ws_dir: &Path) -> PathBuf {
    std::fs::canonicalize(ws_dir).unwrap_or_else(|_| ws_dir.to_path_buf())
}

fn normalize_rel_path(raw: &str) -> String {
    raw.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

fn join_safe_rel_path(ws_dir: &Path, rel: &str) -> Option<PathBuf> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return None;
    }
    for comp in rel_path.components() {
        if matches!(
            comp,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return None;
        }
    }
    Some(ws_dir.join(rel_path))
}

/// Returns true if ANY component along `ws_dir / rel` (intermediate ancestor directory OR leaf)
/// is a symbolic link — preventing symlink-ancestor write/delete escapes outside the workspace.
async fn has_symlink_component(ws_dir: &Path, rel: &str) -> bool {
    let mut cur = ws_dir.to_path_buf();
    for comp in Path::new(rel).components() {
        if let Component::Normal(seg) = comp {
            cur.push(seg);
            if let Ok(meta) = tokio::fs::symlink_metadata(&cur).await {
                if meta.file_type().is_symlink() {
                    return true;
                }
            }
        }
    }
    false
}

async fn prune_empty_parents(ws_dir: &Path, start: Option<&Path>) {
    let mut cur = start.map(Path::to_path_buf);
    while let Some(dir) = cur {
        if dir == ws_dir || !dir.starts_with(ws_dir) {
            break;
        }
        match tokio::fs::remove_dir(&dir).await {
            Ok(()) => cur = dir.parent().map(Path::to_path_buf),
            Err(_) => break,
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn sha256_prefix_16(bytes: &[u8]) -> String {
    let full = sha256_hex(bytes);
    full[..full.len().min(16)].to_string()
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let prefix: String = s.chars().take(max_chars).collect();
    format!("{prefix}\n... [truncated]")
}

async fn run_git_env(
    envs: &[(&str, &std::ffi::OsStr)],
    args: &[&str],
    stdin_bytes: Option<&[u8]>,
) -> Result<(bool, String, String)> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(match stdin_bytes.is_some() {
            true => Stdio::piped(),
            false => Stdio::null(),
        })
        .kill_on_drop(true);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| ZenkoError::Tool(format!("git spawn failed: {e}")))?;
    if let Some(bytes) = stdin_bytes {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(bytes).await;
            let _ = stdin.shutdown().await;
        }
    }
    let out = tokio::time::timeout(
        Duration::from_secs(CHECKPOINT_GIT_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| ZenkoError::Tool("shadow git command timed out".into()))?
    .map_err(|e| ZenkoError::Tool(format!("shadow git wait failed: {e}")))?;

    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

async fn run_git_env_bytes(
    envs: &[(&str, &std::ffi::OsStr)],
    args: &[&str],
) -> Result<(bool, Vec<u8>, Vec<u8>)> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let child = cmd
        .spawn()
        .map_err(|e| ZenkoError::Tool(format!("git spawn failed: {e}")))?;
    let out = tokio::time::timeout(
        Duration::from_secs(CHECKPOINT_GIT_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| ZenkoError::Tool("shadow git command timed out".into()))?
    .map_err(|e| ZenkoError::Tool(format!("shadow git wait failed: {e}")))?;

    Ok((out.status.success(), out.stdout, out.stderr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenko_core::PermissionMode;

    #[tokio::test]
    async fn shadow_git_checkpoint_catches_shell_edits_preserves_hand_edits_and_never_touches_project_git() {
        let root = std::env::temp_dir().join(format!("zenko_cp_test_{}", new_id()));
        let data_dir = root.join("data");
        let ws = root.join("repo");
        std::fs::create_dir_all(&ws).expect("ws");

        // Initialize a real Git repo in workspace to prove project .git is NEVER disturbed
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(&ws)
                .status()
                .expect("git init")
                .success()
        );
        std::fs::write(ws.join("app.rs"), "fn main() { println!(\"v1\"); }\n").expect("write app");
        std::fs::write(ws.join("keep_hand.txt"), "initial\n").expect("write keep");
        assert!(
            std::process::Command::new("git")
                .args(["add", "."])
                .current_dir(&ws)
                .status()
                .expect("git add")
                .success()
        );
        assert!(
            std::process::Command::new("git")
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@local",
                    "commit",
                    "-q",
                    "-m",
                    "init",
                ])
                .current_dir(&ws)
                .status()
                .expect("git commit")
                .success()
        );
        let head_before = String::from_utf8(
            std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&ws)
                .output()
                .expect("head")
                .stdout,
        )
        .expect("utf8");

        let store = SqliteStore::open(data_dir.join("zenko.db")).expect("db");
        let session = store
            .create_session("CP Test", "cli", None, None, PermissionMode::Balanced)
            .expect("sess");
        let ev1 = store
            .append_event(
                &session.id,
                None,
                EventRole::User,
                EventKind::Message,
                serde_json::json!({"text": "refactor app.rs"}),
                &UsageStats::default(),
            )
            .expect("ev1");
        let _ev2 = store
            .append_event(
                &session.id,
                Some(&ev1.id),
                EventRole::Assistant,
                EventKind::Message,
                serde_json::json!({"text": "broken change"}),
                &UsageStats::default(),
            )
            .expect("ev2");

        let (tx, _) = broadcast::channel(32);
        let mgr = CheckpointManager::new(data_dir.clone(), store.clone(), tx);
        let cfg = CheckpointsConfig::default();

        let cp = mgr
            .capture_checkpoint(
                &cfg,
                &session.id,
                "turn_1",
                Some(ev1.id.clone()),
                &ws,
                "shell_exec",
                "Before shell_exec".into(),
                false,
            )
            .await
            .expect("capture");
        assert_eq!(cp.backend, CheckpointBackend::ShadowGit);

        // Simulate mutating file tool + raw shell mutation (creating & overwriting files)
        std::fs::write(ws.join("app.rs"), "fn main() { panic!(\"broken\"); }\n").expect("mutate");
        std::fs::write(ws.join("shell_junk.tmp"), "generated by bash\n").expect("junk");
        let finalized = mgr
            .finalize_checkpoint(&cfg, &cp.id)
            .await
            .expect("finalize")
            .expect("some");
        assert!(finalized.changed_files.contains(&"app.rs".to_string()));
        assert!(finalized.changed_files.contains(&"shell_junk.tmp".to_string()));

        // Simulate human manual hand-edit AFTER the agent's turn finished!
        std::fs::write(ws.join("keep_hand.txt"), "HUMAN_HAND_EDIT\n").expect("hand edit");

        // Dry-run diff must flag `keep_hand.txt` as manual_edit_conflict
        let diff = mgr.diff_checkpoint(&cfg, &cp.id).await.expect("diff");
        assert!(diff.manual_edit_conflicts.contains(&"keep_hand.txt".to_string()));
        assert!(!diff.manual_edit_conflicts.contains(&"app.rs".to_string()));

        // Rewind (default preserve_manual_edits = true): reverts app.rs, removes shell_junk.tmp,
        // PRESERVES human's keep_hand.txt, switches conversation head, and saves safety backup!
        let rep = mgr
            .rewind(
                &cfg,
                &RewindRequest {
                    checkpoint_id: cp.id.clone(),
                    mode: RewindMode::CodeAndConversation,
                    files: vec![],
                    overwrite_manual_edits: false,
                    summarize_abandoned: true,
                },
            )
            .await
            .expect("rewind");
        assert!(rep.restored_files.contains(&"app.rs".to_string()));
        assert!(rep.deleted_files.contains(&"shell_junk.tmp".to_string()));
        assert!(rep.preserved_manual_edits.contains(&"keep_hand.txt".to_string()));
        assert!(rep.safety_checkpoint_id.is_some());
        assert!(rep.rewind_summary.is_some());
        assert_eq!(
            std::fs::read_to_string(ws.join("app.rs")).expect("read"),
            "fn main() { println!(\"v1\"); }\n"
        );
        assert!(!ws.join("shell_junk.tmp").exists());
        assert_eq!(
            std::fs::read_to_string(ws.join("keep_hand.txt")).expect("read keep"),
            "HUMAN_HAND_EDIT\n"
        );

        // Undo-of-Undo WITHOUT overwrite_manual_edits (`false`): exact provenance algebra knows
        // `app.rs` was written by Zenko's rewind, so it restores `app.rs` cleanly without `--force`!
        let safety_id = rep.safety_checkpoint_id.expect("safety id");
        let undo_undo = mgr
            .rewind(
                &cfg,
                &RewindRequest {
                    checkpoint_id: safety_id,
                    mode: RewindMode::CodeOnly,
                    files: vec!["app.rs".into()],
                    overwrite_manual_edits: false,
                    summarize_abandoned: false,
                },
            )
            .await
            .expect("undo of undo without force");
        assert_eq!(undo_undo.restored_files, vec!["app.rs"]);
        assert!(
            std::fs::read_to_string(ws.join("app.rs"))
                .expect("read")
                .contains("panic!")
        );
        // Preserved human hand-edit must STILL stay protected across chained rewinds!
        let chained_diff = mgr.diff_checkpoint(&cfg, &cp.id).await.expect("chained diff");
        assert!(
            chained_diff
                .manual_edit_conflicts
                .contains(&"keep_hand.txt".to_string()),
            "human hand-edit must stay protected across chained rewinds: {chained_diff:?}"
        );

        // Verify project `.git` HEAD and stash list were NEVER touched!
        let head_after = String::from_utf8(
            std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&ws)
                .output()
                .expect("head")
                .stdout,
        )
        .expect("utf8");
        assert_eq!(head_before, head_after);
        let stash_list = String::from_utf8(
            std::process::Command::new("git")
                .args(["stash", "list"])
                .current_dir(&ws)
                .output()
                .expect("stash")
                .stdout,
        )
        .expect("utf8");
        assert!(stash_list.trim().is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn content_blobs_backend_restores_exact_bytes_in_non_git_workspace() {
        let root = std::env::temp_dir().join(format!("zenko_blob_test_{}", new_id()));
        let data_dir = root.join("data");
        let ws = root.join("plain_dir");
        std::fs::create_dir_all(&ws).expect("ws");
        std::fs::write(ws.join("notes.txt"), "hello world\n").expect("write");

        let store = SqliteStore::open(data_dir.join("zenko.db")).expect("db");
        let session = store
            .create_session("Blob Test", "cli", None, None, PermissionMode::Balanced)
            .expect("sess");
        let (tx, _) = broadcast::channel(16);
        let mgr = CheckpointManager::new(data_dir, store.clone(), tx);
        let cfg = CheckpointsConfig::default();

        let pre_ref = mgr
            .capture_blob_manifest(&ws, cfg.max_file_bytes)
            .await
            .expect("manifest");
        let cp = WorkspaceCheckpoint {
            id: "cp_blob_1".into(),
            session_id: session.id.clone(),
            turn_id: "t1".into(),
            event_id: None,
            workspace_dir: canonical_workspace_key(&ws).display().to_string(),
            backend: CheckpointBackend::ContentBlobs,
            pre_ref,
            post_ref: None,
            trigger_tool: "write_file".into(),
            label: "Blob snapshot".into(),
            changed_files: vec![],
            is_safety_backup: false,
            created_at: Utc::now(),
        };
        store.insert_checkpoint(&cp).expect("insert");

        std::fs::write(ws.join("notes.txt"), "corrupted\n").expect("mutate");
        std::fs::write(ws.join("extra.txt"), "delete me\n").expect("extra");
        mgr.finalize_checkpoint(&cfg, &cp.id)
            .await
            .expect("finalize");

        let rep = mgr
            .rewind(
                &cfg,
                &RewindRequest {
                    checkpoint_id: cp.id.clone(),
                    mode: RewindMode::CodeOnly,
                    files: vec![],
                    overwrite_manual_edits: false,
                    summarize_abandoned: false,
                },
            )
            .await
            .expect("rewind");
        assert_eq!(rep.restored_files, vec!["notes.txt"]);
        assert_eq!(rep.deleted_files, vec!["extra.txt"]);
        assert_eq!(
            std::fs::read_to_string(ws.join("notes.txt")).expect("read"),
            "hello world\n"
        );
        assert!(!ws.join("extra.txt").exists());

        // Verify Mark-and-Sweep GC keeps shared manifests alive & reclaims orphaned blobs!
        let shared_manifest = mgr
            .capture_blob_manifest(&ws, cfg.max_file_bytes)
            .await
            .expect("shared manifest");
        let cp_keep = WorkspaceCheckpoint {
            id: "cp_blob_keep".into(),
            session_id: session.id.clone(),
            turn_id: "t2".into(),
            event_id: None,
            workspace_dir: canonical_workspace_key(&ws).display().to_string(),
            backend: CheckpointBackend::ContentBlobs,
            pre_ref: shared_manifest.clone(),
            post_ref: Some(shared_manifest.clone()),
            trigger_tool: "write_file".into(),
            label: "Shared manifest survivor".into(),
            changed_files: vec![],
            is_safety_backup: false,
            created_at: Utc::now(),
        };
        store.insert_checkpoint(&cp_keep).expect("insert keep");
        // Drop orphan checkpoint `cp_blob_1` (which referenced `corrupted\n` and `delete me\n`)
        store.delete_checkpoint(&cp.id).expect("delete cp1");
        if let Some(safety_id) = rep.safety_checkpoint_id.as_deref() {
            let _ = store.delete_checkpoint(safety_id);
        }
        let reclaimed = mgr.gc_content_blobs().await;
        assert!(reclaimed > 0, "expected Mark-and-Sweep GC to reclaim orphaned manifests & blobs");
        // Survivor's shared manifest & blob must still be intact!
        let live_map = mgr
            .read_tree_map(&ws, CheckpointBackend::ContentBlobs, &shared_manifest)
            .await
            .expect("survivor manifest intact");
        assert!(live_map.contains_key("notes.txt"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rewind_blocks_symlink_ancestor_directory_escape() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!("zenko_symlink_esc_{}", new_id()));
        let data_dir = root.join("data");
        let ws = root.join("ws");
        let outside = root.join("outside_secret");
        std::fs::create_dir_all(ws.join("nested")).expect("nested");
        std::fs::create_dir_all(&outside).expect("outside");
        std::fs::write(ws.join("nested/target.txt"), "ORIGINAL_SAFE\n").expect("write");
        std::fs::write(outside.join("target.txt"), "DO_NOT_OVERWRITE\n").expect("outside file");

        let store = SqliteStore::open(data_dir.join("zenko.db")).expect("db");
        let session = store
            .create_session("Sym Test", "cli", None, None, PermissionMode::Balanced)
            .expect("sess");
        let (tx, _) = broadcast::channel(8);
        let mgr = CheckpointManager::new(data_dir, store, tx);
        let cfg = CheckpointsConfig::default();

        let cp = mgr
            .capture_checkpoint(
                &cfg,
                &session.id,
                "t1",
                None,
                &ws,
                "write_file",
                "Before symlink attack".into(),
                false,
            )
            .await
            .expect("capture");

        // Replace `ws/nested` directory with a symlink pointing outside the workspace!
        std::fs::remove_dir_all(ws.join("nested")).expect("rm nested");
        symlink(&outside, ws.join("nested")).expect("symlink");

        let rep = mgr
            .rewind(
                &cfg,
                &RewindRequest {
                    checkpoint_id: cp.id,
                    mode: RewindMode::CodeOnly,
                    files: vec![],
                    overwrite_manual_edits: true,
                    summarize_abandoned: false,
                },
            )
            .await
            .expect("rewind");
        assert!(rep.skipped_symlinks >= 1, "expected symlink ancestor to be skipped: {rep:?}");
        assert!(rep.restored_files.is_empty());
        assert_eq!(
            std::fs::read_to_string(outside.join("target.txt")).expect("read outside"),
            "DO_NOT_OVERWRITE\n",
            "symlink ancestor allowed overwrite outside workspace!"
        );

        let _ = std::fs::remove_dir_all(root);
    }
}
