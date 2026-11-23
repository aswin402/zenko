//! Phase 8 Declarative Workspace File-Change Watchers (`[[watchers]]` in `zenko.toml`).
//!
//! Zero new crates: reuses `ignore::WalkBuilder` + Phase 7's zero-regex `CompiledGlob`,
//! stays 0% CPU asleep on `watcher_notify` when no watchers are enabled, enforces debounce,
//! cooldown, hourly rate caps, and suppresses self-edit feedback loops from Zenko's own turns.

use crate::runtime::{AgentRuntime, TurnOptions};
use ignore::WalkBuilder;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use zenko_core::{
    AgentEvent, DEFAULT_SEARCH_SKIP_DIRS, ExecutionTier, LogChannel, LogDirection, LogStatusTone,
    ORCH_STAGE_TRIGGER, WATCHER_MAX_FILES_PER_SCAN, WATCHER_MAX_SLEEP_SECS,
    WATCHER_MIN_POLL_INTERVAL_SECS, WATCHER_PREVIEW_MAX_FILES, WATCHER_SELF_EDIT_SUPPRESS_SECS,
    WatcherActionKind, WorkspaceWatcherConfig, emit_hono_line,
};
use zenko_tools::glob::CompiledGlob;

type FileSigMap = HashMap<String, (u64, u64)>;

struct WatcherRuntimeState {
    root: PathBuf,
    glob_raw: String,
    baseline: FileSigMap,
    pending_changed: Vec<String>,
    pending_since: Option<Instant>,
    last_scan: Instant,
}

/// Zero-CPU-when-idle background loop monitoring configured `[[watchers]]`.
pub async fn run_workspace_watchers(runtime: Arc<AgentRuntime>, cancel: CancellationToken) {
    info!("workspace file-change watcher service started (0% CPU when no watchers enabled)");
    let mut states: HashMap<String, WatcherRuntimeState> = HashMap::new();

    loop {
        if cancel.is_cancelled() {
            break;
        }
        let cfg = runtime.config();
        let active_watchers: Vec<WorkspaceWatcherConfig> = cfg
            .watchers
            .iter()
            .filter(|w| w.enabled && !w.id.trim().is_empty())
            .cloned()
            .collect();

        if active_watchers.is_empty() {
            states.clear();
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = runtime.watcher_notify.notified() => continue,
            }
        }

        // Prune removed or disabled watchers so state never leaks across hot config reloads.
        let active_ids: HashSet<&str> = active_watchers.iter().map(|w| w.id.as_str()).collect();
        states.retain(|id, _| active_ids.contains(id.as_str()));

        let ws_base = cfg.resolved_workspace_dir();

        for watcher in &active_watchers {
            let poll_secs = watcher.poll_interval_secs.max(WATCHER_MIN_POLL_INTERVAL_SECS);
            let scan_root = resolve_watcher_root(&ws_base, &watcher.path);
            let glob = CompiledGlob::compile(&watcher.glob);
            let now_inst = Instant::now();

            let needs_baseline = match states.get(&watcher.id) {
                None => true,
                Some(existing) => existing.root != scan_root || existing.glob_raw != watcher.glob,
            };
            if needs_baseline {
                let root_clone = scan_root.clone();
                let glob_clone = glob.clone();
                let initial = tokio::task::spawn_blocking(move || {
                    scan_matching_files(&root_clone, &glob_clone)
                })
                .await
                .unwrap_or_default();
                states.insert(
                    watcher.id.clone(),
                    WatcherRuntimeState {
                        root: scan_root.clone(),
                        glob_raw: watcher.glob.clone(),
                        baseline: initial,
                        pending_changed: Vec::new(),
                        pending_since: None,
                        last_scan: Instant::now(),
                    },
                );
                continue;
            }

            let Some(entry) = states.get_mut(&watcher.id) else {
                continue;
            };

            if now_inst.duration_since(entry.last_scan).as_secs() >= poll_secs {
                let root_clone = scan_root.clone();
                let glob_clone = glob.clone();
                let latest = tokio::task::spawn_blocking(move || {
                    scan_matching_files(&root_clone, &glob_clone)
                })
                .await
                .unwrap_or_default();
                let scanned_at = Instant::now();
                entry.last_scan = scanned_at;

                let changed = diff_file_signatures(&entry.baseline, &latest);
                entry.baseline = latest;

                if !changed.is_empty() {
                    // Self-edit loop suppression: ignore changes produced by Zenko's own active turns!
                    if runtime.recently_mutated_workspace(WATCHER_SELF_EDIT_SUPPRESS_SECS) {
                        continue;
                    }
                    for p in changed {
                        if !entry.pending_changed.contains(&p) {
                            entry.pending_changed.push(p);
                        }
                    }
                    if entry.pending_since.is_none() {
                        entry.pending_since = Some(scanned_at);
                    }
                }
            }

            // Fire once debounce window has elapsed
            let check_inst = Instant::now();
            let debounce_ready = entry
                .pending_since
                .is_some_and(|t| check_inst.duration_since(t).as_secs() >= watcher.debounce_secs);
            if debounce_ready && !entry.pending_changed.is_empty() {
                let files = std::mem::take(&mut entry.pending_changed);
                entry.pending_since = None;
                let limiter_key = format!("watcher:{}", watcher.id);
                if let Err(reason) = runtime.trigger_limiter.check_and_record(
                    &limiter_key,
                    watcher.cooldown_secs,
                    watcher.max_per_hour,
                ) {
                    warn!(watcher_id = %watcher.id, reason = %reason, "watcher trigger rate-limited");
                    continue;
                }
                dispatch_watcher_action(&runtime, watcher, &scan_root, &files).await;
            }
        }

        let sleep_ms = compute_next_watcher_sleep_ms(Instant::now(), &active_watchers, &states);
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = runtime.watcher_notify.notified() => continue,
            _ = tokio::time::sleep(Duration::from_millis(sleep_ms)) => {}
        }
    }
}

/// Computes the exact sleep duration until the earliest upcoming poll or pending debounce deadline.
fn compute_next_watcher_sleep_ms(
    now: Instant,
    active_watchers: &[WorkspaceWatcherConfig],
    states: &HashMap<String, WatcherRuntimeState>,
) -> u64 {
    let max_ms = WATCHER_MAX_SLEEP_SECS.saturating_mul(1_000);
    let mut wake_ms = max_ms;
    for watcher in active_watchers {
        let poll_ms = watcher
            .poll_interval_secs
            .max(WATCHER_MIN_POLL_INTERVAL_SECS)
            .saturating_mul(1_000);
        match states.get(&watcher.id) {
            Some(st) => {
                let since_scan = now.duration_since(st.last_scan).as_millis() as u64;
                wake_ms = wake_ms.min(poll_ms.saturating_sub(since_scan));
                if let Some(pending_since) = st.pending_since {
                    if !st.pending_changed.is_empty() {
                        let deb_ms = watcher.debounce_secs.saturating_mul(1_000);
                        let since_pend = now.duration_since(pending_since).as_millis() as u64;
                        wake_ms = wake_ms.min(deb_ms.saturating_sub(since_pend));
                    }
                }
            }
            None => {
                wake_ms = wake_ms.min(poll_ms);
            }
        }
    }
    wake_ms.clamp(200, max_ms)
}

async fn dispatch_watcher_action(
    runtime: &Arc<AgentRuntime>,
    watcher: &WorkspaceWatcherConfig,
    scan_root: &Path,
    changed_files: &[String],
) {
    let preview_files = changed_files
        .iter()
        .take(WATCHER_PREVIEW_MAX_FILES)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let log_entry = emit_hono_line(
        LogDirection::In,
        LogChannel::System,
        &format!(
            "watcher/{} ({}) · {} file(s): {}",
            watcher.id,
            watcher.action.as_str(),
            changed_files.len(),
            preview_files
        ),
        Some(("trigger", LogStatusTone::Info)),
        None,
    );
    runtime.logs.push(log_entry);
    runtime.emit(AgentEvent::OrchestratorUpdated {
        session_id: format!("watcher:{}", watcher.id),
        stage: ORCH_STAGE_TRIGGER.into(),
        tier: ExecutionTier::Standard,
        detail: format!(
            "Watcher `{}` fired on {} file(s): {}",
            watcher.id,
            changed_files.len(),
            preview_files
        ),
        model: watcher.model.clone(),
    });

    match watcher.action {
        WatcherActionKind::GoalLoop => {
            if let Some(ref gid) = watcher
                .goal_loop_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let _ = runtime.resume_and_wake_goal(gid);
            }
        }
        WatcherActionKind::Script => {
            if let Some(ref script_rel) = watcher
                .script_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let _ = runtime
                    .dispatch_hub_script(&format!("watcher:{}", watcher.id), script_rel)
                    .await;
            }
        }
        WatcherActionKind::Prompt => {
            let cfg = runtime.config();
            let prompt = match watcher.prompt.trim().is_empty() {
                true => format!(
                    "Workspace watcher `{}` detected file changes: {}",
                    watcher.id, preview_files
                ),
                false => watcher
                    .prompt
                    .replace("{files}", &preview_files)
                    .replace("{watcher_id}", &watcher.id),
            };
            if let Ok(sess) = runtime.store.find_or_create_channel_session(
                "watcher",
                &watcher.id,
                None,
                &format!("Watcher: {}", watcher.id),
                cfg.policy.mode,
            ) {
                let rt = runtime.clone();
                let sid = sess.id.clone();
                let ws = scan_root.to_path_buf();
                let model = watcher.model.clone();
                tokio::spawn(async move {
                    let _ = rt
                        .run_turn_with_options(
                            &sid,
                            &prompt,
                            TurnOptions {
                                workspace_override: Some(ws),
                                model_override: model,
                                headless_no_wait: true,
                                ..TurnOptions::default()
                            },
                        )
                        .await;
                });
            }
        }
    }
}

fn resolve_watcher_root(ws_base: &Path, raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "." {
        return ws_base.to_path_buf();
    }
    let expanded = zenko_core::expand_tilde(trimmed);
    match expanded.is_absolute() {
        true => expanded,
        false => ws_base.join(expanded),
    }
}

pub fn scan_matching_files(root: &Path, glob: &CompiledGlob) -> FileSigMap {
    let mut map = HashMap::new();
    if !root.exists() {
        return map;
    }
    let mut walker = WalkBuilder::new(root);
    walker
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .filter_entry(|dent| {
            if dent.depth() == 0 {
                return true;
            }
            if dent.file_type().is_some_and(|ft| ft.is_dir()) {
                let name = dent.file_name().to_string_lossy();
                if DEFAULT_SEARCH_SKIP_DIRS.contains(&name.as_ref()) {
                    return false;
                }
            }
            true
        });

    for dent in walker.build().flatten() {
        if map.len() >= WATCHER_MAX_FILES_PER_SCAN {
            break;
        }
        if !dent.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let rel = dent
            .path()
            .strip_prefix(root)
            .unwrap_or_else(|_| dent.path())
            .to_string_lossy()
            .replace('\\', "/");
        if !glob.matches(&rel) {
            continue;
        }
        let Ok(meta) = dent.metadata() else {
            continue;
        };
        let mtime_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        map.insert(rel, (mtime_ms, meta.len()));
    }
    map
}

pub fn diff_file_signatures(prev: &FileSigMap, curr: &FileSigMap) -> Vec<String> {
    let mut changed = Vec::new();
    for (path, sig) in curr {
        if prev.get(path) != Some(sig) {
            changed.push(path.clone());
        }
    }
    for path in prev.keys() {
        if !curr.contains_key(path) {
            changed.push(path.clone());
        }
    }
    changed.sort();
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_scan_and_diff_detects_matching_glob_changes_only() {
        let dir = std::env::temp_dir().join(format!("zenko_watch_{}", zenko_core::new_id()));
        std::fs::create_dir_all(dir.join("src")).expect("mkdir");
        std::fs::write(dir.join("src/main.rs"), "fn main() {}").expect("rs");
        std::fs::write(dir.join("README.md"), "# hi").expect("md");

        let glob = CompiledGlob::compile("**/*.rs");
        let snap1 = scan_matching_files(&dir, &glob);
        assert!(snap1.contains_key("src/main.rs"));
        assert!(!snap1.contains_key("README.md"));

        std::fs::write(dir.join("src/main.rs"), "fn main() { println!(\"ok\"); }").expect("edit");
        std::fs::write(dir.join("README.md"), "# edited").expect("edit md");

        let snap2 = scan_matching_files(&dir, &glob);
        let diff = diff_file_signatures(&snap1, &snap2);
        assert_eq!(diff, vec!["src/main.rs"]);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn watcher_sleep_wakes_promptly_for_pending_debounce() {
        let now = Instant::now();
        let watcher = WorkspaceWatcherConfig {
            id: "slow-poll-fast-debounce".into(),
            enabled: true,
            path: ".".into(),
            glob: "**/*.rs".into(),
            action: WatcherActionKind::Prompt,
            prompt: "check".into(),
            goal_loop_id: None,
            script_path: None,
            model: None,
            poll_interval_secs: 30,
            debounce_secs: 2,
            cooldown_secs: 15,
            max_per_hour: 30,
        };
        let mut states = HashMap::new();
        states.insert(
            watcher.id.clone(),
            WatcherRuntimeState {
                root: PathBuf::from("."),
                glob_raw: "**/*.rs".into(),
                baseline: HashMap::new(),
                pending_changed: vec!["src/main.rs".into()],
                pending_since: Some(now),
                last_scan: now,
            },
        );
        let sleep_ms = compute_next_watcher_sleep_ms(now, &[watcher], &states);
        assert_eq!(
            sleep_ms, 2_000,
            "pending 2s debounce must wake in 2s instead of waiting 30s poll_interval"
        );
    }
}
