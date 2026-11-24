//! Bounded, thread-safe session file-freshness tracker (`FileStateBridge`).
//! Implements Claude Code–style stale-read & blind-overwrite detection with strict
//! per-session and global LRU memory bounds (zero unbounded RAM growth).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use zenko_core::{
    FileFreshnessCheck, FileStateBridge, SESSION_FILE_TRACKER_MAX_FILES_PER_SESSION,
    SESSION_FILE_TRACKER_MAX_SESSIONS,
};

#[derive(Debug, Clone)]
struct RecordedFile {
    sha256: String,
    #[allow(dead_code)]
    bytes: u64,
}

#[derive(Debug, Default)]
struct SessionFiles {
    entries: HashMap<PathBuf, RecordedFile>,
    lru: VecDeque<PathBuf>,
}

impl SessionFiles {
    fn touch(&mut self, path: &Path) {
        if let Some(pos) = self.lru.iter().position(|p| p == path) {
            self.lru.remove(pos);
        }
        self.lru.push_back(path.to_path_buf());
    }

    fn record(&mut self, path: PathBuf, sha256: String, bytes: u64) {
        self.entries.insert(path.clone(), RecordedFile { sha256, bytes });
        self.touch(&path);
        while self.entries.len() > SESSION_FILE_TRACKER_MAX_FILES_PER_SESSION {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    fn forget(&mut self, path: &Path) {
        self.entries.remove(path);
        if let Some(pos) = self.lru.iter().position(|p| p == path) {
            self.lru.remove(pos);
        }
    }
}

#[derive(Debug, Default)]
struct TrackerInner {
    sessions: HashMap<String, SessionFiles>,
    session_lru: VecDeque<String>,
}

impl TrackerInner {
    fn touch_session(&mut self, session_id: &str) {
        if let Some(pos) = self.session_lru.iter().position(|s| s == session_id) {
            self.session_lru.remove(pos);
        }
        self.session_lru.push_back(session_id.to_string());
    }

    fn evict_if_needed(&mut self) {
        while self.sessions.len() > SESSION_FILE_TRACKER_MAX_SESSIONS {
            let Some(oldest) = self.session_lru.pop_front() else {
                break;
            };
            self.sessions.remove(&oldest);
        }
    }
}

#[derive(Debug, Default)]
pub struct SessionFileTracker {
    inner: Mutex<TrackerInner>,
}

impl SessionFileTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear_session(&self, session_id: &str) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.sessions.remove(session_id);
            if let Some(pos) = guard.session_lru.iter().position(|s| s == session_id) {
                guard.session_lru.remove(pos);
            }
        }
    }
}

impl FileStateBridge for SessionFileTracker {
    fn record_file_state(&self, session_id: &str, canonical_path: &Path, sha256: String, bytes: u64) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.touch_session(session_id);
            let bucket = guard.sessions.entry(session_id.to_string()).or_default();
            bucket.record(canonical_path.to_path_buf(), sha256, bytes);
            guard.evict_if_needed();
        }
    }

    fn forget_file_state(&self, session_id: &str, canonical_path: &Path) {
        if let Ok(mut guard) = self.inner.lock()
            && let Some(bucket) = guard.sessions.get_mut(session_id)
        {
            bucket.forget(canonical_path);
        }
    }

    fn check_file_freshness(
        &self,
        session_id: &str,
        canonical_path: &Path,
        current_sha256: &str,
    ) -> FileFreshnessCheck {
        let Ok(guard) = self.inner.lock() else {
            return FileFreshnessCheck::Fresh;
        };
        let Some(bucket) = guard.sessions.get(session_id) else {
            return FileFreshnessCheck::UnreadExisting;
        };
        match bucket.entries.get(canonical_path) {
            None => FileFreshnessCheck::UnreadExisting,
            Some(rec) if rec.sha256 == current_sha256 => FileFreshnessCheck::Fresh,
            Some(rec) => FileFreshnessCheck::ExternallyModified {
                recorded_sha256: rec.sha256.clone(),
                current_sha256: current_sha256.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_detects_unread_fresh_external_drift_and_evicts_lru() {
        let tracker = SessionFileTracker::new();
        let p = Path::new("/tmp/a.rs");

        assert_eq!(
            tracker.check_file_freshness("s1", p, "aaa"),
            FileFreshnessCheck::UnreadExisting
        );
        tracker.record_file_state("s1", p, "aaa".into(), 10);
        assert_eq!(
            tracker.check_file_freshness("s1", p, "aaa"),
            FileFreshnessCheck::Fresh
        );
        assert_eq!(
            tracker.check_file_freshness("s1", p, "bbb"),
            FileFreshnessCheck::ExternallyModified {
                recorded_sha256: "aaa".into(),
                current_sha256: "bbb".into(),
            }
        );

        tracker.forget_file_state("s1", p);
        assert_eq!(
            tracker.check_file_freshness("s1", p, "bbb"),
            FileFreshnessCheck::UnreadExisting
        );

        // Exceed session LRU bound cleanly
        for i in 0..(SESSION_FILE_TRACKER_MAX_SESSIONS + 5) {
            tracker.record_file_state(&format!("sess_{i}"), p, "h".into(), 1);
        }
        assert_eq!(
            tracker.check_file_freshness("sess_0", p, "h"),
            FileFreshnessCheck::UnreadExisting
        );
        assert_eq!(
            tracker.check_file_freshness(&format!("sess_{}", SESSION_FILE_TRACKER_MAX_SESSIONS + 4), p, "h"),
            FileFreshnessCheck::Fresh
        );
    }
}
