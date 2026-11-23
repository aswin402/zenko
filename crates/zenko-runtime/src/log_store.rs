//! Dual-tier categorized persistent log sink & flight recorder.
//! Routes scrubbed logs asynchronously into:
//! 1. Indexed SQLite `logs` ring table (queryable by category, session, level, search)
//! 2. Rotating category JSONL files under `~/.zenko/logs/`:
//!    - `all.jsonl`
//!    - `sessions/index.jsonl` + `sessions/<session_id>.jsonl`
//!    - `gateway.jsonl`, `webui.jsonl`, `tui.jsonl`, `cron.jsonl`, `system.jsonl`
//!    - `wire/<turn_id>.json` (opt-in via `ZENKO_DEBUG_WIRE=1`)
//!    - `crash.log` (via panic hook)

use chrono::Utc;
use serde_json::{Value, json};
use std::fs::{OpenOptions, metadata};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, OnceLock};
use zenko_core::{
    ENV_DEBUG_WIRE, FILE_MODE_OWNER_ONLY, LOG_ALL_FILE, LOG_CRASH_FILE, LOG_DIR_NAME,
    LOG_MAX_FILE_BYTES, LOG_MAX_ROTATED_FILES, LOG_MAX_SESSION_FILES, LOG_SESSIONS_SUBDIR,
    LOG_WIRE_MAX_FILES, LOG_WIRE_SUBDIR, LOG_WRITER_CHANNEL_CAP, LogCategory, LogEntry,
    SecretVault, scrub_json, scrub_secrets,
};
use zenko_memory::SqliteStore;

enum SinkCmd {
    Write(LogEntry),
    Flush(mpsc::Sender<()>),
}

pub struct CategorizedLogSink {
    logs_dir: PathBuf,
    tx: SyncSender<SinkCmd>,
    vault: Arc<SecretVault>,
    dropped_count: Arc<AtomicU64>,
}

impl CategorizedLogSink {
    pub fn start(data_dir: &Path, store: SqliteStore, vault: Arc<SecretVault>) -> Arc<Self> {
        let logs_dir = data_dir.join(LOG_DIR_NAME);
        let _ = std::fs::create_dir_all(logs_dir.join(LOG_SESSIONS_SUBDIR));
        let (tx, rx) = mpsc::sync_channel::<SinkCmd>(LOG_WRITER_CHANNEL_CAP);
        let dropped_count = Arc::new(AtomicU64::new(0));

        let worker_dir = logs_dir.clone();
        let worker_vault = vault.clone();
        let _ = std::thread::Builder::new()
            .name("zenko-log-sink".into())
            .spawn(move || {
                let mut secret_cache: Vec<String> = worker_vault.all_values();
                let mut writes_since_refresh = 0u64;
                let mut session_writes = 0u64;

                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        SinkCmd::Flush(ack) => {
                            let _ = ack.send(());
                        }
                        SinkCmd::Write(entry) => {
                            writes_since_refresh += 1;
                            if writes_since_refresh >= 64 {
                                secret_cache = worker_vault.all_values();
                                writes_since_refresh = 0;
                            }
                            let scrubbed = scrub_log_entry(entry, &secret_cache);
                            let _ = store.insert_log(&scrubbed);
                            write_categorized_jsonl(
                                &worker_dir,
                                &scrubbed,
                                LOG_MAX_FILE_BYTES,
                                &mut session_writes,
                            );
                        }
                    }
                }
            });

        Arc::new(Self {
            logs_dir,
            tx,
            vault,
            dropped_count,
        })
    }

    pub fn logs_dir(&self) -> &Path {
        &self.logs_dir
    }

    pub fn dropped_entries(&self) -> u64 {
        self.dropped_count.load(Ordering::Relaxed)
    }

    /// Non-blocking enqueue so hot agent/HTTP paths never wait on disk I/O.
    pub fn enqueue(&self, entry: LogEntry) {
        if self.tx.try_send(SinkCmd::Write(entry)).is_err() {
            self.dropped_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Blocks briefly until all queued log entries have been persisted to SQLite & JSONL.
    pub fn flush(&self) {
        let (ack_tx, ack_rx) = mpsc::channel();
        if self.tx.send(SinkCmd::Flush(ack_tx)).is_ok() {
            let _ = ack_rx.recv_timeout(std::time::Duration::from_secs(2));
        }
    }

    /// Opt-in sanitized LLM wire flight recorder (`ZENKO_DEBUG_WIRE=1`).
    pub fn record_wire_exchange(
        &self,
        session_id: &str,
        turn_id: &str,
        provider: &str,
        model: &str,
        request_meta: Value,
        response_meta: Value,
    ) {
        if !wire_debug_enabled() {
            return;
        }
        let wire_dir = self.logs_dir.join(LOG_WIRE_SUBDIR);
        if std::fs::create_dir_all(&wire_dir).is_err() {
            return;
        }
        let secrets = self.vault.all_values();
        let payload = scrub_json(
            json!({
                "timestamp": Utc::now().to_rfc3339(),
                "session_id": session_id,
                "turn_id": turn_id,
                "provider": provider,
                "model": model,
                "request": request_meta,
                "response": response_meta,
            }),
            &secrets,
        );
        let safe_turn = sanitize_filename_part(turn_id);
        let path = wire_dir.join(format!("{}_{safe_turn}.json", Utc::now().format("%Y%m%dT%H%M%S")));
        if let Ok(bytes) = serde_json::to_vec_pretty(&payload) {
            write_private_bytes(&path, &bytes);
            prune_oldest_files(&wire_dir, LOG_WIRE_MAX_FILES, "json");
        }
    }
}

pub fn wire_debug_enabled() -> bool {
    std::env::var(ENV_DEBUG_WIRE)
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

/// Persists a single scrubbed entry synchronously (useful for short-lived TUI/CLI commands
/// that don't boot the full `AgentRuntime`).
pub fn record_standalone_log(data_dir: &Path, store: &SqliteStore, vault: &SecretVault, entry: LogEntry) {
    let logs_dir = data_dir.join(LOG_DIR_NAME);
    let secrets = vault.all_values();
    let scrubbed = scrub_log_entry(entry, &secrets);
    let _ = store.insert_log(&scrubbed);
    let mut counter = 0u64;
    write_categorized_jsonl(&logs_dir, &scrubbed, LOG_MAX_FILE_BYTES, &mut counter);
}

/// Installs a one-time process panic hook writing structured crash dumps to `~/.zenko/logs/crash.log`.
pub fn install_panic_hook(logs_dir: PathBuf) {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    if INSTALLED.set(()).is_err() {
        return;
    }
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = std::fs::create_dir_all(&logs_dir);
        let crash_path = logs_dir.join(LOG_CRASH_FILE);
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed");
        let loc = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".into());
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("non-string panic payload");
        let bt = std::backtrace::Backtrace::force_capture();
        let dump = format!(
            "=== ZENKO PANIC {} ===\nthread:   {thread_name}\nlocation: {loc}\nmessage:  {msg}\nbacktrace:\n{bt}\n\n",
            Utc::now().to_rfc3339()
        );
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&crash_path) {
            let _ = f.write_all(dump.as_bytes());
        }
        prev(info);
    }));
}

fn scrub_log_entry(mut entry: LogEntry, secrets: &[String]) -> LogEntry {
    entry.message = scrub_secrets(&entry.message, secrets);
    entry.payload = entry.payload.map(|v| scrub_json(v, secrets));
    entry
}

fn write_categorized_jsonl(
    logs_dir: &Path,
    entry: &LogEntry,
    max_file_bytes: u64,
    session_writes: &mut u64,
) {
    let Ok(mut line) = serde_json::to_string(entry) else {
        return;
    };
    line.push('\n');
    let bytes = line.as_bytes();

    // 1. Unified firehose (`all.jsonl`)
    append_rotating(logs_dir.join(LOG_ALL_FILE), bytes, max_file_bytes);

    // 2. Category file (`sessions/index.jsonl`, `gateway.jsonl`, `webui.jsonl`, `tui.jsonl`, `cron.jsonl`, `system.jsonl`)
    let cat = LogCategory::from_str_loose(&entry.category);
    let cat_path = match cat.relative_file_parts() {
        (Some(sub), file) => logs_dir.join(sub).join(file),
        (None, file) => logs_dir.join(file),
    };
    append_rotating(cat_path, bytes, max_file_bytes);

    // 3. Dedicated per-session flight recorder (`sessions/<session_id>.jsonl`)
    if let Some(ref sid) = entry.session_id {
        let safe_sid = sanitize_filename_part(sid);
        if !safe_sid.is_empty() {
            let sessions_dir = logs_dir.join(LOG_SESSIONS_SUBDIR);
            let session_file = sessions_dir.join(format!("{safe_sid}.jsonl"));
            append_rotating(session_file, bytes, max_file_bytes);
            *session_writes += 1;
            if (*session_writes).is_multiple_of(32) {
                prune_oldest_files(&sessions_dir, LOG_MAX_SESSION_FILES, "jsonl");
            }
        }
    }
}

fn append_rotating(path: PathBuf, bytes: &[u8], max_bytes: u64) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(meta) = metadata(&path)
        && meta.len().saturating_add(bytes.len() as u64) > max_bytes {
            rotate_file_chain(&path, LOG_MAX_ROTATED_FILES);
        }
    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(FILE_MODE_OWNER_ONLY);
    }
    if let Ok(mut f) = opts.open(&path) {
        let _ = f.write_all(bytes);
    }
}

fn rotate_file_chain(base: &Path, keep: usize) {
    for idx in (1..=keep).rev() {
        let dst = rotated_path(base, idx);
        match idx {
            1 => {
                let _ = std::fs::rename(base, &dst);
            }
            n => {
                let src = rotated_path(base, n - 1);
                if src.exists() {
                    let _ = std::fs::rename(&src, &dst);
                }
            }
        }
    }
}

fn rotated_path(base: &Path, idx: usize) -> PathBuf {
    let mut s = base.as_os_str().to_os_string();
    s.push(format!(".{idx}"));
    PathBuf::from(s)
}

fn sanitize_filename_part(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(64)
        .collect()
}

fn write_private_bytes(path: &Path, bytes: &[u8]) {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(FILE_MODE_OWNER_ONLY);
    }
    if let Ok(mut f) = opts.open(path) {
        let _ = f.write_all(bytes);
    }
}

fn prune_oldest_files(dir: &Path, max_files: usize, ext: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let is_target_ext = p.extension().and_then(|x| x.to_str()) == Some(ext);
            let is_index = p.file_name().and_then(|n| n.to_str())
                == Some(zenko_core::LOG_SESSION_INDEX_FILE);
            if !is_target_ext || is_index {
                return None;
            }
            let mtime = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::UNIX_EPOCH);
            Some((mtime, p))
        })
        .collect();

    if files.len() <= max_files {
        return;
    }
    files.sort_by_key(|(mtime, _)| *mtime);
    let remove_count = files.len().saturating_sub(max_files);
    for (_, path) in files.into_iter().take(remove_count) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenko_core::{
        LogChannel, LogDirection, LogFilter, LogStatusTone, format_hono_entry, new_id,
    };

    #[test]
    fn routes_logs_to_category_and_session_files_redacts_secrets_and_queries_sqlite() {
        let dir = std::env::temp_dir().join(format!("zenko_log_sink_{}", new_id()));
        let store = SqliteStore::open(dir.join("zenko.db")).expect("open sqlite");
        let vault = Arc::new(SecretVault::open(dir.join("vault.enc"), dir.join("vault.key")));
        vault.set("OPENROUTER_API_KEY", "sk-or-secret-token-987654").expect("set secret");

        let sink = CategorizedLogSink::start(&dir, store.clone(), vault);

        sink.enqueue(
            format_hono_entry(
                LogDirection::Out,
                LogChannel::Tool,
                "read_file sk-or-secret-token-987654",
                Some(("ok", LogStatusTone::Ok)),
                Some("2.1ms"),
            )
            .with_session("sess_abc")
            .with_payload(json!({ "api_key": "sk-or-secret-token-987654", "lines": 42 })),
        );
        sink.enqueue(format_hono_entry(
            LogDirection::Out,
            LogChannel::Gateway,
            "telegram connected",
            Some(("ok", LogStatusTone::Ok)),
            None,
        ));
        sink.enqueue(format_hono_entry(
            LogDirection::Out,
            LogChannel::Http,
            "GET /api/bootstrap",
            Some(("200", LogStatusTone::Ok)),
            Some("1.2ms"),
        ));
        sink.enqueue(format_hono_entry(
            LogDirection::Out,
            LogChannel::Tui,
            "setup saved",
            Some(("ok", LogStatusTone::Ok)),
            None,
        ));
        sink.enqueue(format_hono_entry(
            LogDirection::Work,
            LogChannel::Cron,
            "daily digest",
            Some(("cron", LogStatusTone::Info)),
            Some("firing"),
        ));
        sink.flush();

        let logs_dir = sink.logs_dir();
        assert!(logs_dir.join("all.jsonl").exists());
        assert!(logs_dir.join("sessions/index.jsonl").exists());
        assert!(logs_dir.join("sessions/sess_abc.jsonl").exists());
        assert!(logs_dir.join("gateway.jsonl").exists());
        assert!(logs_dir.join("webui.jsonl").exists());
        assert!(logs_dir.join("tui.jsonl").exists());
        assert!(logs_dir.join("cron.jsonl").exists());

        // Verify secret was scrubbed before hitting disk or SQLite
        let sess_raw = std::fs::read_to_string(logs_dir.join("sessions/sess_abc.jsonl")).expect("read");
        assert!(!sess_raw.contains("sk-or-secret-token-987654"));
        assert!(sess_raw.contains("[REDACTED]"));

        // Verify SQLite filtered queries by category & session
        let only_cron = store
            .query_logs(&LogFilter {
                category: Some("cron".into()),
                ..Default::default()
            })
            .expect("query cron");
        assert_eq!(only_cron.len(), 1);
        assert_eq!(only_cron[0].category, "cron");

        let only_sess = store
            .query_logs(&LogFilter {
                session_id: Some("sess_abc".into()),
                ..Default::default()
            })
            .expect("query session");
        assert_eq!(only_sess.len(), 1);
        assert_eq!(only_sess[0].payload.as_ref().expect("payload")["lines"], 42);

        let counts = store.log_category_counts().expect("counts");
        assert_eq!(counts.get("session"), Some(&1));
        assert_eq!(counts.get("gateway"), Some(&1));
        assert_eq!(counts.get("webui"), Some(&1));
        assert_eq!(counts.get("tui"), Some(&1));
        assert_eq!(counts.get("cron"), Some(&1));

        let _ = std::fs::remove_dir_all(dir);
    }
}
