//! Async, event-driven background process supervisor for Zenko.
//! - Uses 100% non-blocking `tokio::fs::File` streaming into owner-only (`0600`) files under `~/.zenko/tasks/<kind>/`
//! - Uses level-triggered `tokio::sync::watch` completion channels (immune to missed-wakeup races)
//! - Persists lifecycle metadata in SQLite, terminates full Unix process groups cleanly, and
//!   notifies owning sessions reactively with zero busy-polling.

use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{Mutex as AsyncMutex, broadcast, watch};
use tokio_util::sync::CancellationToken;
use zenko_core::{
    AgentEvent, BackgroundJobRecord, BackgroundJobStatus, FILE_MODE_OWNER_ONLY,
    JOB_LIVE_DELTA_COALESCE_MS, JOB_LIVE_DELTA_MAX_BYTES, JOB_MAX_LOG_BYTES, JOB_MAX_READ_BYTES,
    JOB_MIN_READ_BYTES, JOB_PUMP_DRAIN_MS, JOB_SIGKILL_WAIT_MS, JOB_STOP_GRACE_MS,
    JOB_STOP_WAIT_SECS, JOB_STREAM_BUF_BYTES, JobReadChunk, JobSupervisor, KILL_BIN_UNIX, Result,
    SAFE_SHELL_ENV_KEYS, SHELL_BIN_UNIX, TASKS_DIR_NAME, TaskKind, TerminalStreamKind, ZenkoError,
    new_id,
};
use zenko_memory::SqliteStore;
use zenko_tools::sandbox::{ShellSandboxPolicy, install_sandbox_on_command};

type FinishCallback = Arc<dyn Fn(&BackgroundJobRecord) + Send + Sync>;

struct ActiveJobHandle {
    session_id: String,
    stdin: Arc<AsyncMutex<Option<ChildStdin>>>,
    disk_log: Arc<AsyncMutex<AsyncBoundedDiskLog>>,
    bytes_tx: watch::Sender<u64>,
    stop_token: CancellationToken,
    bytes_rx: watch::Receiver<u64>,
    done_rx: watch::Receiver<bool>,
    notify_on_complete: Arc<AtomicBool>,
}

struct AsyncBoundedDiskLog {
    file: tokio::fs::File,
    bytes_written: u64,
    capped_notified: bool,
}

impl AsyncBoundedDiskLog {
    async fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut opts = tokio::fs::OpenOptions::new();
        opts.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            opts.mode(FILE_MODE_OWNER_ONLY);
        }
        let file = opts.open(path).await?;
        Ok(Self {
            file,
            bytes_written: 0,
            capped_notified: false,
        })
    }

    async fn write_chunk(&mut self, chunk: &[u8]) {
        if self.bytes_written >= JOB_MAX_LOG_BYTES {
            if !self.capped_notified {
                let _ = self
                    .file
                    .write_all(b"\n[job log capped at max size]\n")
                    .await;
                let _ = self.file.flush().await;
                self.capped_notified = true;
            }
            return;
        }
        let remaining = (JOB_MAX_LOG_BYTES - self.bytes_written) as usize;
        let slice = match chunk.len() <= remaining {
            true => chunk,
            false => &chunk[..remaining],
        };
        if self.file.write_all(slice).await.is_ok() {
            let _ = self.file.flush().await;
            self.bytes_written = self.bytes_written.saturating_add(slice.len() as u64);
        }
    }

    async fn clear(&mut self) -> Result<()> {
        self.file.set_len(0).await?;
        self.file.seek(std::io::SeekFrom::Start(0)).await?;
        self.file.flush().await?;
        self.bytes_written = 0;
        self.capped_notified = false;
        Ok(())
    }
}

/// Shared async byte-range / tail log reader used by `LocalJobSupervisor`.
pub async fn read_job_log_slice(
    job: BackgroundJobRecord,
    byte_offset: Option<u64>,
    max_bytes: usize,
) -> Result<JobReadChunk> {
    let cap = max_bytes.clamp(JOB_MIN_READ_BYTES, JOB_MAX_READ_BYTES);
    let path = PathBuf::from(&job.log_path);
    if !path.exists() {
        return Ok(JobReadChunk {
            job,
            offset: 0,
            next_offset: 0,
            total_bytes: 0,
            content: String::new(),
        });
    }

    let mut file = tokio::fs::File::open(&path).await?;
    let total_bytes = file.metadata().await?.len();
    let start = match byte_offset {
        Some(off) => off.min(total_bytes),
        None => total_bytes.saturating_sub(cap as u64),
    };
    file.seek(std::io::SeekFrom::Start(start)).await?;
    let to_read = (total_bytes.saturating_sub(start) as usize).min(cap);
    let mut buf = vec![0u8; to_read];
    let mut read_total = 0usize;
    while read_total < to_read {
        let n = file.read(&mut buf[read_total..]).await?;
        if n == 0 {
            break;
        }
        read_total += n;
    }
    buf.truncate(read_total);
    let content = String::from_utf8_lossy(&buf).to_string();

    Ok(JobReadChunk {
        job,
        offset: start,
        next_offset: start.saturating_add(read_total as u64),
        total_bytes,
        content,
    })
}

/// Shared synchronous byte-range / tail log reader for offline CLI & MCP fallbacks (zero duplication).
pub fn read_job_log_slice_sync(
    job: BackgroundJobRecord,
    byte_offset: Option<u64>,
    max_bytes: usize,
) -> Result<JobReadChunk> {
    use std::io::{Read, Seek};

    let cap = max_bytes.clamp(JOB_MIN_READ_BYTES, JOB_MAX_READ_BYTES);
    let path = PathBuf::from(&job.log_path);
    if !path.exists() {
        return Ok(JobReadChunk {
            job,
            offset: 0,
            next_offset: 0,
            total_bytes: 0,
            content: String::new(),
        });
    }

    let mut file = std::fs::File::open(&path)?;
    let total_bytes = file.metadata()?.len();
    let start = match byte_offset {
        Some(off) => off.min(total_bytes),
        None => total_bytes.saturating_sub(cap as u64),
    };
    file.seek(std::io::SeekFrom::Start(start))?;
    let to_read = (total_bytes.saturating_sub(start) as usize).min(cap);
    let mut buf = vec![0u8; to_read];
    let mut read_total = 0usize;
    while read_total < to_read {
        let n = file.read(&mut buf[read_total..])?;
        if n == 0 {
            break;
        }
        read_total += n;
    }
    buf.truncate(read_total);
    let content = String::from_utf8_lossy(&buf).to_string();

    Ok(JobReadChunk {
        job,
        offset: start,
        next_offset: start.saturating_add(read_total as u64),
        total_bytes,
        content,
    })
}

pub struct LocalJobSupervisor {
    jobs_dir: PathBuf,
    hub_dir: PathBuf,
    os_sandbox: Arc<AtomicBool>,
    store: SqliteStore,
    events: broadcast::Sender<AgentEvent>,
    active: Arc<Mutex<HashMap<String, ActiveJobHandle>>>,
    on_finish: Option<FinishCallback>,
}

impl LocalJobSupervisor {
    pub fn new(
        data_dir: &Path,
        store: SqliteStore,
        events: broadcast::Sender<AgentEvent>,
        on_finish: Option<FinishCallback>,
    ) -> Arc<Self> {
        Self::new_with_sandbox(
            data_dir,
            data_dir,
            Arc::new(AtomicBool::new(true)),
            store,
            events,
            on_finish,
        )
    }

    pub fn new_with_sandbox(
        data_dir: &Path,
        hub_dir: &Path,
        os_sandbox: Arc<AtomicBool>,
        store: SqliteStore,
        events: broadcast::Sender<AgentEvent>,
        on_finish: Option<FinishCallback>,
    ) -> Arc<Self> {
        let jobs_dir = data_dir.join(TASKS_DIR_NAME);
        let _ = std::fs::create_dir_all(&jobs_dir);
        Arc::new(Self {
            jobs_dir,
            hub_dir: hub_dir.to_path_buf(),
            os_sandbox,
            store,
            events,
            active: Arc::new(Mutex::new(HashMap::new())),
            on_finish,
        })
    }

    pub fn jobs_dir(&self) -> &Path {
        &self.jobs_dir
    }

    /// Subscribes to event-driven byte-progress & terminal completion watch channels
    /// for a currently running job (powers zero-polling `jobs_follow` streaming).
    pub fn subscribe_progress(&self, job_id: &str) -> Option<(watch::Receiver<u64>, watch::Receiver<bool>)> {
        self.active
            .lock()
            .ok()
            .and_then(|map| map.get(job_id).map(|h| (h.bytes_rx.clone(), h.done_rx.clone())))
    }

    /// Counts currently running *async background* jobs owned by `session_id`
    /// (`notify_on_complete == true`, excluding synchronous foreground `shell_exec` calls).
    pub fn active_background_jobs_count_for_session(&self, session_id: &str) -> usize {
        self.active
            .lock()
            .map(|map| {
                map.values()
                    .filter(|h| {
                        h.session_id == session_id && h.notify_on_complete.load(Ordering::SeqCst)
                    })
                    .count()
            })
            .unwrap_or(0)
    }
}

/// Deterministic incremental UTF-8 decoder (zero regex): buffers only trailing incomplete
/// multi-byte codepoints (<= 3 bytes) across pipe reads so emoji/Unicode never splits into `\u{FFFD}`.
#[derive(Default)]
struct Utf8ChunkDecoder {
    pending: Vec<u8>,
}

impl Utf8ChunkDecoder {
    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut out = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(valid) => {
                    out.push_str(valid);
                    self.pending.clear();
                    break;
                }
                Err(err) => {
                    let valid_up_to = err.valid_up_to();
                    if valid_up_to > 0
                        && let Ok(valid) = std::str::from_utf8(&self.pending[..valid_up_to]) {
                            out.push_str(valid);
                        }
                    match err.error_len() {
                        Some(invalid_len) => {
                            out.push('\u{FFFD}');
                            self.pending.drain(..valid_up_to + invalid_len);
                        }
                        None => {
                            self.pending.drain(..valid_up_to);
                            break;
                        }
                    }
                }
            }
        }
        out
    }

    fn finish(&mut self) -> String {
        match self.pending.is_empty() {
            true => String::new(),
            false => {
                let tail = String::from_utf8_lossy(&self.pending).into_owned();
                self.pending.clear();
                tail
            }
        }
    }
}

struct LiveStreamEmitter {
    job_id: String,
    session_id: String,
    stream: TerminalStreamKind,
    events: broadcast::Sender<AgentEvent>,
    decoder: Utf8ChunkDecoder,
    pending_text: String,
    last_emit: Instant,
    last_bytes: u64,
}

impl LiveStreamEmitter {
    fn new(
        job_id: String,
        session_id: String,
        stream: TerminalStreamKind,
        events: broadcast::Sender<AgentEvent>,
    ) -> Self {
        Self {
            job_id,
            session_id,
            stream,
            events,
            decoder: Utf8ChunkDecoder::default(),
            pending_text: String::new(),
            last_emit: Instant::now()
                .checked_sub(Duration::from_millis(JOB_LIVE_DELTA_COALESCE_MS))
                .unwrap_or_else(Instant::now),
            last_bytes: 0,
        }
    }

    fn ingest(&mut self, bytes: &[u8], bytes_written: u64) {
        self.last_bytes = bytes_written;
        let decoded = self.decoder.push(bytes);
        if decoded.is_empty() {
            return;
        }
        self.pending_text.push_str(&decoded);
        let should_flush = self.pending_text.len() >= JOB_LIVE_DELTA_MAX_BYTES
            || self.last_emit.elapsed() >= Duration::from_millis(JOB_LIVE_DELTA_COALESCE_MS)
            || decoded.contains('\n');
        if should_flush {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.pending_text.is_empty() {
            return;
        }
        let mut remaining = std::mem::take(&mut self.pending_text);
        while !remaining.is_empty() {
            let cut = remaining.floor_char_boundary(JOB_LIVE_DELTA_MAX_BYTES.min(remaining.len()));
            let piece = match cut == remaining.len() {
                true => std::mem::take(&mut remaining),
                false => {
                    let rest = remaining.split_off(cut);
                    std::mem::replace(&mut remaining, rest)
                }
            };
            let _ = self.events.send(AgentEvent::JobOutputDelta {
                job_id: self.job_id.clone(),
                session_id: self.session_id.clone(),
                stream: self.stream,
                chunk: piece,
                bytes_written: self.last_bytes,
            });
        }
        self.last_emit = Instant::now();
    }

    fn finish(mut self) {
        let tail = self.decoder.finish();
        if !tail.is_empty() {
            self.pending_text.push_str(&tail);
        }
        self.flush();
    }
}

async fn pump_stream<R: AsyncRead + Unpin>(
    mut reader: R,
    log: Arc<AsyncMutex<AsyncBoundedDiskLog>>,
    bytes_tx: watch::Sender<u64>,
    mut emitter: LiveStreamEmitter,
) {
    let mut buf = [0u8; JOB_STREAM_BUF_BYTES];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut guard = log.lock().await;
                guard.write_chunk(&buf[..n]).await;
                let written = guard.bytes_written;
                drop(guard);
                bytes_tx.send_replace(written);
                emitter.ingest(&buf[..n], written);
            }
        }
    }
    emitter.finish();
}

async fn terminate_process_group(pid: Option<u32>, child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(p) = pid {
        let pgid_arg = format!("-{p}");
        let _ = Command::new(KILL_BIN_UNIX)
            .args(["-TERM", &pgid_arg])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        if tokio::time::timeout(Duration::from_millis(JOB_STOP_GRACE_MS), child.wait())
            .await
            .is_ok()
        {
            return;
        }

        let _ = Command::new(KILL_BIN_UNIX)
            .args(["-KILL", &pgid_arg])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }

    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_millis(JOB_SIGKILL_WAIT_MS), child.wait()).await;
}

#[async_trait]
impl JobSupervisor for LocalJobSupervisor {
    async fn spawn_shell(
        &self,
        session_id: &str,
        command: &str,
        workspace_dir: &Path,
        kind: TaskKind,
        label: Option<String>,
        notify_on_complete: bool,
    ) -> Result<BackgroundJobRecord> {
        let job_id = format!("task_{}", new_id());
        let log_path = self
            .jobs_dir
            .join(kind.subdir())
            .join(format!("{job_id}.log"));
        let disk_log = Arc::new(AsyncMutex::new(AsyncBoundedDiskLog::open(&log_path).await?));

        let mut cmd = Command::new(SHELL_BIN_UNIX);
        cmd.arg("-c")
            .arg(command)
            .current_dir(workspace_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        #[cfg(unix)]
        cmd.process_group(0);

        cmd.env_clear();
        for key in SAFE_SHELL_ENV_KEYS {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }
        let sandbox_policy = ShellSandboxPolicy::for_workspace(
            self.os_sandbox.load(Ordering::Relaxed),
            workspace_dir,
            Some(&self.hub_dir),
            &[],
        );
        install_sandbox_on_command(&mut cmd, &sandbox_policy);

        let mut child = cmd
            .spawn()
            .map_err(|e| ZenkoError::Tool(format!("failed to spawn background task: {e}")))?;

        let pid = child.id();
        let stdin_handle = Arc::new(AsyncMutex::new(child.stdin.take()));
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let record = BackgroundJobRecord {
            id: job_id.clone(),
            session_id: session_id.to_string(),
            kind,
            label,
            command: command.to_string(),
            workspace_dir: workspace_dir.display().to_string(),
            pid,
            status: BackgroundJobStatus::Running,
            exit_code: None,
            log_path: log_path.display().to_string(),
            bytes_written: 0,
            started_at: Utc::now(),
            finished_at: None,
        };
        self.store.upsert_job(&record)?;
        let _ = self.events.send(AgentEvent::JobUpdated {
            job: record.clone(),
        });

        let stop_token = CancellationToken::new();
        let (bytes_tx, bytes_rx) = watch::channel(0u64);
        let (done_tx, done_rx) = watch::channel(false);
        let notify_flag = Arc::new(AtomicBool::new(notify_on_complete));
        if let Ok(mut map) = self.active.lock() {
            map.insert(
                job_id.clone(),
                ActiveJobHandle {
                    session_id: session_id.to_string(),
                    stdin: stdin_handle.clone(),
                    disk_log: disk_log.clone(),
                    bytes_tx: bytes_tx.clone(),
                    stop_token: stop_token.clone(),
                    bytes_rx,
                    done_rx,
                    notify_on_complete: notify_flag.clone(),
                },
            );
        }

        let store = self.store.clone();
        let events = self.events.clone();
        let active_map = self.active.clone();
        let on_finish = self.on_finish.clone();
        let mut final_rec = record.clone();

        tokio::spawn(async move {
            let out_task = stdout.map(|r| {
                let emitter = LiveStreamEmitter::new(
                    final_rec.id.clone(),
                    final_rec.session_id.clone(),
                    TerminalStreamKind::Stdout,
                    events.clone(),
                );
                tokio::spawn(pump_stream(r, disk_log.clone(), bytes_tx.clone(), emitter))
            });
            let err_task = stderr.map(|r| {
                let emitter = LiveStreamEmitter::new(
                    final_rec.id.clone(),
                    final_rec.session_id.clone(),
                    TerminalStreamKind::Stderr,
                    events.clone(),
                );
                tokio::spawn(pump_stream(r, disk_log.clone(), bytes_tx.clone(), emitter))
            });

            let (status, exit_code) = tokio::select! {
                biased;
                _ = stop_token.cancelled() => {
                    if let Ok(mut guard) = stdin_handle.try_lock() {
                        *guard = None;
                    }
                    terminate_process_group(pid, &mut child).await;
                    (BackgroundJobStatus::Stopped, None)
                }
                wait_res = child.wait() => match wait_res {
                    Ok(st) => {
                        #[cfg(unix)]
                        let code = {
                            use std::os::unix::process::ExitStatusExt as _;
                            st.code()
                                .or_else(|| st.signal().map(|sig| zenko_core::POSIX_SIGNAL_EXIT_BASE + sig))
                        };
                        #[cfg(not(unix))]
                        let code = st.code();
                        let s = match st.success() {
                            true => BackgroundJobStatus::Completed,
                            false => BackgroundJobStatus::Failed,
                        };
                        (s, code)
                    }
                    Err(_) => (BackgroundJobStatus::Failed, None),
                }
            };

            let drain_limit = Duration::from_millis(JOB_PUMP_DRAIN_MS);
            if let Some(t) = out_task {
                let _ = tokio::time::timeout(drain_limit, t).await;
            }
            if let Some(t) = err_task {
                let _ = tokio::time::timeout(drain_limit, t).await;
            }

            let written = disk_log.lock().await.bytes_written;
            final_rec.status = status;
            final_rec.exit_code = exit_code;
            final_rec.bytes_written = written;
            final_rec.finished_at = Some(Utc::now());

            let _ = store.upsert_job(&final_rec);
            if let Ok(mut map) = active_map.lock() {
                map.remove(&final_rec.id);
            }
            let _ = events.send(AgentEvent::JobUpdated {
                job: final_rec.clone(),
            });
            let _ = done_tx.send(true);

            if notify_flag.load(Ordering::SeqCst)
                && let Some(cb) = on_finish {
                    cb(&final_rec);
                }
        });

        Ok(record)
    }

    fn enable_completion_notify(&self, job_id: &str) {
        if let Ok(map) = self.active.lock()
            && let Some(h) = map.get(job_id) {
                h.notify_on_complete.store(true, Ordering::SeqCst);
            }
    }

    fn list_jobs(
        &self,
        session_id: Option<&str>,
        kind: Option<TaskKind>,
        limit: usize,
    ) -> Result<Vec<BackgroundJobRecord>> {
        let mut jobs = self.store.list_jobs(session_id, kind, limit)?;
        for job in &mut jobs {
            if job.status == BackgroundJobStatus::Running
                && let Ok(meta) = std::fs::metadata(&job.log_path) {
                    job.bytes_written = meta.len();
                }
        }
        Ok(jobs)
    }

    fn get_job(&self, job_id: &str) -> Result<Option<BackgroundJobRecord>> {
        let mut rec = self.store.get_job(job_id)?;
        if let Some(ref mut job) = rec
            && job.status == BackgroundJobStatus::Running
                && let Ok(meta) = std::fs::metadata(&job.log_path) {
                    job.bytes_written = meta.len();
                }
        Ok(rec)
    }

    fn update_job_meta(
        &self,
        job_id: &str,
        label: Option<String>,
        kind: Option<TaskKind>,
    ) -> Result<Option<BackgroundJobRecord>> {
        let updated = self.store.update_job_meta(job_id, label, kind)?;
        if let Some(ref rec) = updated {
            let _ = self.events.send(AgentEvent::JobUpdated { job: rec.clone() });
        }
        Ok(updated)
    }

    async fn restart_job(&self, job_id: &str) -> Result<BackgroundJobRecord> {
        let prev = self
            .get_job(job_id)?
            .ok_or_else(|| ZenkoError::NotFound(format!("task `{job_id}` not found")))?;
        if prev.status == BackgroundJobStatus::Running {
            let _ = self.stop_job(job_id).await;
        }
        let ws = PathBuf::from(&prev.workspace_dir);
        self.spawn_shell(
            &prev.session_id,
            &prev.command,
            &ws,
            prev.kind,
            prev.label,
            true,
        )
        .await
    }

    async fn delete_job(&self, job_id: &str) -> Result<bool> {
        let _ = self.stop_job(job_id).await;
        match self.store.delete_job(job_id)? {
            Some(rec) => {
                let _ = tokio::fs::remove_file(&rec.log_path).await;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn prune_jobs(&self, kind: Option<TaskKind>) -> Result<usize> {
        let removed = self.store.prune_finished_jobs(kind)?;
        let count = removed.len();
        for rec in removed {
            let _ = tokio::fs::remove_file(&rec.log_path).await;
        }
        Ok(count)
    }

    async fn read_job_output(
        &self,
        job_id: &str,
        byte_offset: Option<u64>,
        max_bytes: usize,
    ) -> Result<JobReadChunk> {
        let job = self
            .get_job(job_id)?
            .ok_or_else(|| ZenkoError::NotFound(format!("job `{job_id}` not found")))?;
        read_job_log_slice(job, byte_offset, max_bytes).await
    }

    async fn clear_job_output(&self, job_id: &str) -> Result<Option<BackgroundJobRecord>> {
        let Some(mut job) = self.store.get_job(job_id)? else {
            return Ok(None);
        };
        let active_handles = self
            .active
            .lock()
            .ok()
            .and_then(|map| {
                map.get(job_id)
                    .map(|h| (h.disk_log.clone(), h.bytes_tx.clone()))
            });

        if let Some((disk_log, bytes_tx)) = active_handles {
            let mut guard = disk_log.lock().await;
            guard.clear().await?;
            drop(guard);
            bytes_tx.send_replace(0);
        } else {
            let path = PathBuf::from(&job.log_path);
            if path.exists() {
                let mut opts = tokio::fs::OpenOptions::new();
                opts.write(true).truncate(true);
                let _ = opts.open(&path).await;
            }
        }

        job.bytes_written = 0;
        self.store.upsert_job(&job)?;
        let _ = self.events.send(AgentEvent::JobUpdated { job: job.clone() });
        Ok(Some(job))
    }

    async fn send_input(&self, job_id: &str, input: &str) -> Result<()> {
        let stdin_arc = self
            .active
            .lock()
            .ok()
            .and_then(|map| map.get(job_id).map(|h| h.stdin.clone()))
            .ok_or_else(|| ZenkoError::Tool(format!("job `{job_id}` is not running")))?;

        let mut guard = stdin_arc.lock().await;
        let Some(ref mut stdin) = *guard else {
            return Err(ZenkoError::Tool(format!("stdin closed for job `{job_id}`")));
        };
        stdin
            .write_all(input.as_bytes())
            .await
            .map_err(|e| ZenkoError::Tool(format!("failed writing stdin: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| ZenkoError::Tool(format!("failed flushing stdin: {e}")))?;
        Ok(())
    }

    async fn stop_job(&self, job_id: &str) -> Result<Option<BackgroundJobRecord>> {
        let handle_pair = self
            .active
            .lock()
            .ok()
            .and_then(|map| {
                map.get(job_id)
                    .map(|h| (h.stop_token.clone(), h.done_rx.clone()))
            });

        if let Some((stop_tok, mut done_rx)) = handle_pair {
            stop_tok.cancel();
            let _ = tokio::time::timeout(
                Duration::from_secs(JOB_STOP_WAIT_SECS),
                done_rx.wait_for(|&done| done),
            )
            .await;
        }
        self.get_job(job_id)
    }

    async fn wait_job(
        &self,
        job_id: &str,
        timeout: Duration,
    ) -> Result<Option<BackgroundJobRecord>> {
        let rx_opt = self
            .active
            .lock()
            .ok()
            .and_then(|map| map.get(job_id).map(|h| h.done_rx.clone()));

        if let Some(mut done_rx) = rx_opt {
            let _ = tokio::time::timeout(timeout, done_rx.wait_for(|&done| done)).await;
        }
        self.get_job(job_id)
    }

    async fn stop_session_jobs(&self, session_id: &str) -> usize {
        let handles: Vec<(CancellationToken, watch::Receiver<bool>)> = self
            .active
            .lock()
            .map(|map| {
                map.values()
                    .filter(|h| h.session_id == session_id)
                    .map(|h| (h.stop_token.clone(), h.done_rx.clone()))
                    .collect()
            })
            .unwrap_or_default();

        let count = handles.len();
        for (tok, _) in &handles {
            tok.cancel();
        }
        let deadline = Duration::from_secs(JOB_STOP_WAIT_SECS);
        for (_, mut done_rx) in handles {
            let _ = tokio::time::timeout(deadline, done_rx.wait_for(|&done| done)).await;
        }
        count
    }

    async fn stop_all(&self) {
        let handles: Vec<(CancellationToken, watch::Receiver<bool>)> = self
            .active
            .lock()
            .map(|map| {
                map.values()
                    .map(|h| (h.stop_token.clone(), h.done_rx.clone()))
                    .collect()
            })
            .unwrap_or_default();

        for (tok, _) in &handles {
            tok.cancel();
        }
        let deadline = Duration::from_secs(JOB_STOP_WAIT_SECS);
        for (_, mut done_rx) in handles {
            let _ = tokio::time::timeout(deadline, done_rx.wait_for(|&done| done)).await;
        }
    }

    fn active_count(&self) -> usize {
        self.active.lock().map(|m| m.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    #[tokio::test]
    async fn background_job_streams_output_accepts_stdin_and_stops_cleanly() {
        let dir = std::env::temp_dir().join(format!("zenko_jobs_test_{}", new_id()));
        let store = SqliteStore::open(dir.join("zenko.db")).expect("open sqlite");
        let (tx, _) = broadcast::channel(32);
        let finished_cb = Arc::new(AtomicBool::new(false));
        let cb_flag = finished_cb.clone();
        let sup = LocalJobSupervisor::new(
            &dir,
            store.clone(),
            tx,
            Some(Arc::new(move |_| {
                cb_flag.store(true, Ordering::SeqCst);
            })),
        );

        // 1. Foreground task (`notify_on_complete = false`) writes under tasks/<kind>/ and does NOT wake
        let fg_job = sup
            .spawn_shell("s1", "echo quiet-foreground", &dir, TaskKind::Build, Some("Unit Check".into()), false)
            .await
            .expect("spawn fg");
        assert!(fg_job.log_path.contains("/tasks/build/"));
        let fg_done = sup
            .wait_job(&fg_job.id, Duration::from_secs(3))
            .await
            .expect("wait fg")
            .expect("fg rec");
        assert_eq!(fg_done.status, BackgroundJobStatus::Completed);
        assert_eq!(fg_done.kind, TaskKind::Build);
        assert_eq!(fg_done.label.as_deref(), Some("Unit Check"));
        assert!(!finished_cb.load(Ordering::SeqCst));

        // 2. Background task (`notify_on_complete = true`) fires completion callback & supports CRUD update/delete
        let job = sup
            .spawn_shell("s1", "echo hello-async-world", &dir, TaskKind::Shell, None, true)
            .await
            .expect("spawn echo");
        let done = sup
            .wait_job(&job.id, Duration::from_secs(3))
            .await
            .expect("wait")
            .expect("rec");
        assert_eq!(done.status, BackgroundJobStatus::Completed);
        assert_eq!(done.exit_code, Some(0));
        assert!(finished_cb.load(Ordering::SeqCst));
        let chunk = sup
            .read_job_output(&job.id, Some(0), 4096)
            .await
            .expect("read");
        assert!(chunk.content.contains("hello-async-world"));

        let updated = sup
            .update_job_meta(&job.id, Some("Renamed Echo".into()), Some(TaskKind::Debug))
            .expect("update")
            .expect("updated rec");
        assert_eq!(updated.label.as_deref(), Some("Renamed Echo"));
        assert_eq!(updated.kind, TaskKind::Debug);
        assert!(Path::new(&job.log_path).exists());
        assert!(sup.delete_job(&job.id).await.expect("delete"));
        assert!(!Path::new(&job.log_path).exists());
        assert!(sup.get_job(&job.id).expect("get deleted").is_none());

        // Prune removes remaining finished tasks & logs
        assert_eq!(sup.prune_jobs(Some(TaskKind::Build)).await.expect("prune"), 1);

        // 3. Interactive long-running task accepts stdin, supports live notify promotion, restart, and stops cleanly
        finished_cb.store(false, Ordering::SeqCst);
        let long = sup
            .spawn_shell("s1", "read line; echo GOT:$line; sleep 30", &dir, TaskKind::Dev, Some("Dev Server".into()), false)
            .await
            .expect("spawn interactive");
        assert_eq!(sup.active_count(), 1);
        sup.enable_completion_notify(&long.id);
        sup.send_input(&long.id, "zenko-ping\n")
            .await
            .expect("send stdin");
        tokio::time::sleep(Duration::from_millis(120)).await;
        let out = sup
            .read_job_output(&long.id, Some(0), 4096)
            .await
            .expect("read interactive");
        assert!(out.content.contains("GOT:zenko-ping"));

        let stopped = sup
            .stop_job(&long.id)
            .await
            .expect("stop")
            .expect("stopped rec");
        assert_eq!(stopped.status, BackgroundJobStatus::Stopped);
        assert!(finished_cb.load(Ordering::SeqCst));
        assert_eq!(sup.active_count(), 0);

        // 4. Level-triggered watch guarantees stop_all() never misses concurrent exits
        let _j1 = sup
            .spawn_shell("s1", "sleep 30", &dir, TaskKind::Shell, None, false)
            .await
            .expect("j1");
        let _j2 = sup
            .spawn_shell("s1", "sleep 30", &dir, TaskKind::Shell, None, false)
            .await
            .expect("j2");
        let _j3 = sup
            .spawn_shell("s1", "sleep 30", &dir, TaskKind::Shell, None, false)
            .await
            .expect("j3");
        assert_eq!(sup.active_count(), 3);
        let t0 = Instant::now();
        sup.stop_all().await;
        assert_eq!(sup.active_count(), 0);
        assert!(
            t0.elapsed() < Duration::from_millis(1200),
            "stop_all should never hit missed-wakeup timeout: {:?}",
            t0.elapsed()
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn utf8_chunk_decoder_handles_split_multibyte_sequences() {
        let sample = "Build ✓ 🦊 done!\n";
        let bytes = sample.as_bytes();
        let mut decoder = Utf8ChunkDecoder::default();
        let mut rebuilt = String::new();
        // Feed 1 byte at a time to split every 3-byte and 4-byte UTF-8 sequence
        for chunk in bytes.chunks(1) {
            rebuilt.push_str(&decoder.push(chunk));
        }
        rebuilt.push_str(&decoder.finish());
        assert_eq!(rebuilt, sample);
        assert!(!rebuilt.contains('\u{FFFD}'));
    }
}
