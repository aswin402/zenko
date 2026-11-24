//! Pure-Rust persistent Unix PTY terminal session manager for `shell_exec(session=...)`.
//! - Zero external PTY crates (uses POSIX `posix_openpt` / `grantpt` / `unlockpt` / `ptsname_r`
//!   via `libc`, already pinned in `Cargo.lock`).
//! - Event-driven epoll-backed `tokio::io::unix::AsyncFd` reads (zero busy-polling!).
//! - Preserves `cd`, `export`, virtualenvs (`source .venv/bin/activate`), and interactive REPLs
//!   across turns while enforcing Landlock kernel write sandboxing & sanitized env vars.
//! - Zero-regex ANSI CSI/OSC escape stripper cleanly formats PTY output for LLMs.

use crate::sandbox::ShellSandboxPolicy;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    PTY_DEFAULT_COLS, PTY_DEFAULT_ROWS, PTY_IDLE_TIMEOUT_SECS, PTY_MAX_OUTPUT_BYTES,
    PTY_MAX_SESSIONS_PER_CHAT, PTY_MAX_TOTAL_SESSIONS, PTY_READ_BUF_BYTES, Result,
    SAFE_SHELL_ENV_KEYS, SHELL_BIN_UNIX, ZenkoError, new_id,
};

#[derive(Debug, Clone)]
pub struct PtySessionInfo {
    pub name: String,
    pub pid: u32,
    pub initial_cwd: String,
    pub idle_secs: u64,
}

#[derive(Debug, Clone)]
pub struct PtyExecOutcome {
    pub exit_code: i32,
    pub output: String,
    pub timed_out: bool,
}

#[derive(Clone, Copy)]
enum AnsiState {
    Normal,
    Esc,
    Csi,
    Osc,
    OscEsc,
}

/// Pure-Rust zero-regex finite-state ANSI CSI/OSC escape sequence stripper + `\r\n` normalizer.
pub fn strip_ansi_escapes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut state = AnsiState::Normal;

    for ch in raw.chars() {
        match state {
            AnsiState::Normal => match ch {
                '\x1b' => state = AnsiState::Esc,
                '\r' => {}
                c if c == '\n' || c == '\t' || !c.is_control() => out.push(c),
                _ => {}
            },
            AnsiState::Esc => match ch {
                '[' => state = AnsiState::Csi,
                ']' => state = AnsiState::Osc,
                _ => state = AnsiState::Normal,
            },
            AnsiState::Csi => {
                if ('@'..='~').contains(&ch) {
                    state = AnsiState::Normal;
                }
            }
            AnsiState::Osc => match ch {
                '\x07' => state = AnsiState::Normal,
                '\x1b' => state = AnsiState::OscEsc,
                _ => {}
            },
            AnsiState::OscEsc => {
                state = AnsiState::Normal;
            }
        }
    }
    out
}

#[cfg(unix)]
mod unix_pty {
    use super::*;
    use std::ffi::CStr;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::process::Stdio;
    use tokio::io::unix::AsyncFd;
    use tokio::process::{Child, Command};

    pub(super) struct LivePtySession {
        pub(super) name: String,
        pub(super) pid: u32,
        pub(super) initial_cwd: PathBuf,
        pub(super) last_used: Instant,
        master: AsyncFd<OwnedFd>,
        child: Child,
        pending_buf: Vec<u8>,
    }

    impl LivePtySession {
        pub(super) fn spawn(
            name: String,
            cwd: &Path,
            hub_dir: &Path,
            os_sandbox: bool,
        ) -> Result<Self> {
            let (master_fd, slave_path, slave_fd) = open_pty_pair()?;

            // Disable slave ECHO so sentinel markers written to stdin never prematurely echo back.
            configure_slave_termios(slave_fd.as_raw_fd(), PTY_DEFAULT_COLS, PTY_DEFAULT_ROWS);

            let sandbox_policy = ShellSandboxPolicy::for_workspace(
                os_sandbox,
                cwd,
                Some(hub_dir),
                std::slice::from_ref(&slave_path),
            );
            #[cfg(target_os = "linux")]
            let prepared_ll = crate::sandbox::prepare_landlock_cstrings(&sandbox_policy);

            let slave_raw = slave_fd.as_raw_fd();
            let mut cmd = Command::new(SHELL_BIN_UNIX);
            cmd.current_dir(cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);

            cmd.env_clear();
            for key in SAFE_SHELL_ENV_KEYS {
                if let Ok(val) = std::env::var(key) {
                    cmd.env(key, val);
                }
            }
            cmd.env("TERM", "dumb");
            cmd.env("PS1", "");
            cmd.env("PS2", "");

            unsafe {
                cmd.pre_exec(move || {
                    if libc::setsid() < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    let _ = libc::ioctl(slave_raw, libc::TIOCSCTTY as _, 0);
                    if libc::dup2(slave_raw, libc::STDIN_FILENO) < 0
                        || libc::dup2(slave_raw, libc::STDOUT_FILENO) < 0
                        || libc::dup2(slave_raw, libc::STDERR_FILENO) < 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    if slave_raw > libc::STDERR_FILENO {
                        libc::close(slave_raw);
                    }
                    #[cfg(target_os = "linux")]
                    if let Some((ref dirs, ref files)) = prepared_ll {
                        crate::sandbox::apply_prepared_landlock_signal_safe(dirs, files);
                    }
                    Ok(())
                });
            }

            let child = cmd
                .spawn()
                .map_err(|e| ZenkoError::Tool(format!("failed to spawn PTY shell: {e}")))?;
            drop(slave_fd);

            // Set master fd non-blocking for event-driven AsyncFd reads/writes.
            set_nonblocking(master_fd.as_raw_fd())?;
            // SAFETY: `master_fd` is an exclusively owned, open `OwnedFd` that remains valid for the entire lifetime of `AsyncFd`.
            let async_master = unsafe { AsyncFd::register(master_fd) }
                .map_err(|e| ZenkoError::Tool(format!("failed to register PTY AsyncFd: {e}")))?;

            let pid = child.id().unwrap_or(0);
            Ok(Self {
                name,
                pid,
                initial_cwd: cwd.to_path_buf(),
                last_used: Instant::now(),
                master: async_master,
                child,
                pending_buf: Vec::with_capacity(4096),
            })
        }

        pub(super) fn is_alive(&mut self) -> bool {
            matches!(self.child.try_wait(), Ok(None))
        }

        async fn write_all(&mut self, data: &[u8], cancel: &CancellationToken) -> Result<()> {
            let mut offset = 0usize;
            while offset < data.len() {
                let mut guard = tokio::select! {
                    _ = cancel.cancelled() => return Err(ZenkoError::Cancelled),
                    res = self.master.writable() => {
                        res.map_err(|e| ZenkoError::Tool(format!("PTY write readiness failed: {e}")))?
                    }
                };
                let slice = &data[offset..];
                match guard.try_io(|inner| {
                    let n = unsafe {
                        libc::write(
                            inner.as_raw_fd(),
                            slice.as_ptr() as *const libc::c_void,
                            slice.len(),
                        )
                    };
                    match n >= 0 {
                        true => Ok(n as usize),
                        false => Err(std::io::Error::last_os_error()),
                    }
                }) {
                    Ok(Ok(n)) => offset += n,
                    Ok(Err(e)) => {
                        return Err(ZenkoError::Tool(format!("PTY write failed: {e}")));
                    }
                    Err(_would_block) => continue,
                }
            }
            Ok(())
        }

        async fn read_chunk(
            &mut self,
            deadline: tokio::time::Instant,
            cancel: &CancellationToken,
        ) -> Result<Option<Vec<u8>>> {
            let mut buf = vec![0u8; PTY_READ_BUF_BYTES];
            loop {
                let mut guard = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(ZenkoError::Cancelled),
                    _ = tokio::time::sleep_until(deadline) => return Ok(None),
                    res = self.master.readable() => {
                        res.map_err(|e| ZenkoError::Tool(format!("PTY read readiness failed: {e}")))?
                    }
                };
                match guard.try_io(|inner| {
                    let n = unsafe {
                        libc::read(
                            inner.as_raw_fd(),
                            buf.as_mut_ptr() as *mut libc::c_void,
                            buf.len(),
                        )
                    };
                    match n >= 0 {
                        true => Ok(n as usize),
                        false => Err(std::io::Error::last_os_error()),
                    }
                }) {
                    Ok(Ok(0)) => return Ok(Some(Vec::new())),
                    Ok(Ok(n)) => {
                        buf.truncate(n);
                        return Ok(Some(buf));
                    }
                    Ok(Err(e)) if e.raw_os_error() == Some(libc::EIO) => {
                        return Ok(Some(Vec::new()));
                    }
                    Ok(Err(e)) => {
                        return Err(ZenkoError::Tool(format!("PTY read failed: {e}")));
                    }
                    Err(_would_block) => continue,
                }
            }
        }

        /// Executes `command` inside the persistent PTY shell using a unique nonce sentinel
        /// to deterministically capture exact stdout/stderr and exit code.
        pub(super) async fn exec_command(
            &mut self,
            command: &str,
            timeout: Duration,
            cancel: &CancellationToken,
        ) -> Result<PtyExecOutcome> {
            self.last_used = Instant::now();
            self.drain_stale_nonblocking();

            let sentinel = format!("__ZENKO_PTY_DONE_{}__:", new_id());
            let framed = format!(
                "{command}\n__zenko_ec=$?\nprintf '\\n{sentinel}%d\\n' \"$__zenko_ec\"\n"
            );
            self.write_all(framed.as_bytes(), cancel).await?;

            let deadline = tokio::time::Instant::now() + timeout;
            let mut captured = std::mem::take(&mut self.pending_buf);

            loop {
                if let Some((body_bytes, exit_code, rest)) =
                    extract_sentinel_frame(&captured, sentinel.as_bytes())
                {
                    self.pending_buf = rest;
                    let text = strip_ansi_escapes(&String::from_utf8_lossy(&body_bytes));
                    return Ok(PtyExecOutcome {
                        exit_code,
                        output: text.trim_end_matches('\n').to_string(),
                        timed_out: false,
                    });
                }

                match self.read_chunk(deadline, cancel).await? {
                    None => {
                        // Send Ctrl-C (`0x03`) on timeout so the persistent shell remains usable.
                        let _ = self.write_all(&[0x03, b'\n'], cancel).await;
                        let text = strip_ansi_escapes(&String::from_utf8_lossy(&captured));
                        return Ok(PtyExecOutcome {
                            exit_code: 124,
                            output: text,
                            timed_out: true,
                        });
                    }
                    Some(chunk) if chunk.is_empty() => {
                        let text = strip_ansi_escapes(&String::from_utf8_lossy(&captured));
                        return Ok(PtyExecOutcome {
                            exit_code: -1,
                            output: text,
                            timed_out: false,
                        });
                    }
                    Some(chunk) => {
                        append_bounded(&mut captured, &chunk, PTY_MAX_OUTPUT_BYTES);
                    }
                }
            }
        }

        /// Writes raw stdin bytes into the persistent PTY (for interactive prompts or REPLs)
        /// and collects any output produced within `wait`.
        pub(super) async fn send_raw(
            &mut self,
            input: &str,
            wait: Duration,
            cancel: &CancellationToken,
        ) -> Result<String> {
            self.last_used = Instant::now();
            if !input.is_empty() {
                self.write_all(input.as_bytes(), cancel).await?;
            }
            let deadline = tokio::time::Instant::now() + wait;
            let mut captured = std::mem::take(&mut self.pending_buf);
            loop {
                match self.read_chunk(deadline, cancel).await? {
                    None => break,
                    Some(chunk) if chunk.is_empty() => break,
                    Some(chunk) => append_bounded(&mut captured, &chunk, PTY_MAX_OUTPUT_BYTES),
                }
            }
            Ok(strip_ansi_escapes(&String::from_utf8_lossy(&captured)))
        }

        fn drain_stale_nonblocking(&mut self) {
            self.pending_buf.clear();
            let fd = self.master.as_raw_fd();
            let mut tmp = [0u8; 1024];
            for _ in 0..8 {
                let n = unsafe {
                    libc::read(fd, tmp.as_mut_ptr() as *mut libc::c_void, tmp.len())
                };
                if n <= 0 {
                    break;
                }
            }
        }

        pub(super) async fn shutdown(&mut self) {
            if self.pid > 0 {
                unsafe {
                    libc::kill(-(self.pid as libc::pid_t), libc::SIGTERM);
                }
            }
            let _ = tokio::time::timeout(Duration::from_millis(300), self.child.wait()).await;
            if self.is_alive() {
                if self.pid > 0 {
                    unsafe {
                        libc::kill(-(self.pid as libc::pid_t), libc::SIGKILL);
                    }
                }
                let _ = self.child.start_kill();
                let _ = self.child.wait().await;
            }
        }
    }

    fn open_pty_pair() -> Result<(OwnedFd, PathBuf, OwnedFd)> {
        let master_raw = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
        if master_raw < 0 {
            return Err(ZenkoError::Tool(format!(
                "posix_openpt failed: {}",
                std::io::Error::last_os_error()
            )));
        }
        let master = unsafe { OwnedFd::from_raw_fd(master_raw) };
        unsafe {
            let flags = libc::fcntl(master.as_raw_fd(), libc::F_GETFD);
            if flags >= 0 {
                libc::fcntl(master.as_raw_fd(), libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
            if libc::grantpt(master.as_raw_fd()) != 0 || libc::unlockpt(master.as_raw_fd()) != 0 {
                return Err(ZenkoError::Tool(format!(
                    "grantpt/unlockpt failed: {}",
                    std::io::Error::last_os_error()
                )));
            }
        }

        let slave_name = slave_pts_name(master.as_raw_fd())?;
        let slave_cstr = std::ffi::CString::new(slave_name.as_str())
            .map_err(|_| ZenkoError::Tool("invalid PTY slave path".into()))?;
        let slave_raw =
            unsafe { libc::open(slave_cstr.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
        if slave_raw < 0 {
            return Err(ZenkoError::Tool(format!(
                "failed to open PTY slave `{slave_name}`: {}",
                std::io::Error::last_os_error()
            )));
        }
        let slave = unsafe { OwnedFd::from_raw_fd(slave_raw) };
        Ok((master, PathBuf::from(slave_name), slave))
    }

    fn slave_pts_name(master_fd: libc::c_int) -> Result<String> {
        #[cfg(target_os = "linux")]
        {
            let mut buf = [0u8; 256];
            let rc = unsafe {
                libc::ptsname_r(master_fd, buf.as_mut_ptr() as *mut libc::c_char, buf.len())
            };
            if rc != 0 {
                return Err(ZenkoError::Tool(format!(
                    "ptsname_r failed: {}",
                    std::io::Error::from_raw_os_error(rc)
                )));
            }
            let cstr = unsafe { CStr::from_ptr(buf.as_ptr() as *const libc::c_char) };
            Ok(cstr.to_string_lossy().into_owned())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let ptr = unsafe { libc::ptsname(master_fd) };
            if ptr.is_null() {
                return Err(ZenkoError::Tool("ptsname returned null".into()));
            }
            let cstr = unsafe { CStr::from_ptr(ptr) };
            Ok(cstr.to_string_lossy().into_owned())
        }
    }

    fn configure_slave_termios(slave_fd: libc::c_int, cols: u16, rows: u16) {
        unsafe {
            let mut term: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(slave_fd, &mut term) == 0 {
                term.c_lflag &= !(libc::ECHO | libc::ECHONL);
                let _ = libc::tcsetattr(slave_fd, libc::TCSANOW, &term);
            }
            let ws = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            let _ = libc::ioctl(slave_fd, libc::TIOCSWINSZ as _, &ws);
        }
    }

    fn set_nonblocking(fd: libc::c_int) -> Result<()> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(ZenkoError::Tool(format!(
                "fcntl(F_GETFL) failed: {}",
                std::io::Error::last_os_error()
            )));
        }
        let rc = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
        match rc == 0 {
            true => Ok(()),
            false => Err(ZenkoError::Tool(format!(
                "fcntl(F_SETFL) failed: {}",
                std::io::Error::last_os_error()
            ))),
        }
    }

    fn append_bounded(buf: &mut Vec<u8>, chunk: &[u8], cap: usize) {
        if buf.len() + chunk.len() <= cap {
            buf.extend_from_slice(chunk);
            return;
        }
        // Keep the latest `cap` bytes so the trailing sentinel marker is never dropped.
        buf.extend_from_slice(chunk);
        if buf.len() > cap {
            let drop_n = buf.len() - cap;
            buf.drain(..drop_n);
        }
    }

    fn extract_sentinel_frame(buf: &[u8], sentinel: &[u8]) -> Option<(Vec<u8>, i32, Vec<u8>)> {
        let pos = buf
            .windows(sentinel.len())
            .position(|win| win == sentinel)?;
        let after = &buf[pos + sentinel.len()..];
        let nl_rel = after.iter().position(|&b| b == b'\n')?;
        let code_raw = std::str::from_utf8(&after[..nl_rel])
            .ok()?
            .trim_end_matches('\r')
            .trim();
        let exit_code = code_raw.parse::<i32>().ok()?;
        let body = buf[..pos].to_vec();
        let rest = after[nl_rel + 1..].to_vec();
        Some((body, exit_code, rest))
    }
}

#[derive(Default)]
pub struct PtySessionManager {
    #[cfg(unix)]
    sessions: Mutex<HashMap<(String, String), Arc<AsyncMutex<unix_pty::LivePtySession>>>>,
}

impl PtySessionManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    #[cfg(unix)]
    pub async fn exec(
        &self,
        chat_session_id: &str,
        pty_name: &str,
        command: &str,
        cwd: &Path,
        hub_dir: &Path,
        os_sandbox: bool,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<PtyExecOutcome> {
        self.reap_idle().await;
        let handle = self.get_or_spawn(chat_session_id, pty_name, cwd, hub_dir, os_sandbox)?;
        let mut guard = handle.lock().await;
        if !guard.is_alive() {
            *guard = unix_pty::LivePtySession::spawn(
                pty_name.to_string(),
                cwd,
                hub_dir,
                os_sandbox,
            )?;
        }
        guard.exec_command(command, timeout, cancel).await
    }

    #[cfg(not(unix))]
    #[allow(clippy::too_many_arguments)]
    pub async fn exec(
        &self,
        _chat_session_id: &str,
        _pty_name: &str,
        _command: &str,
        _cwd: &Path,
        _hub_dir: &Path,
        _os_sandbox: bool,
        _timeout: Duration,
        _cancel: &CancellationToken,
    ) -> Result<PtyExecOutcome> {
        Err(ZenkoError::Tool(
            "Persistent PTY sessions are currently supported on Unix/Linux platforms.".into(),
        ))
    }

    #[cfg(unix)]
    pub async fn send_input(
        &self,
        chat_session_id: &str,
        pty_name: &str,
        input: &str,
        wait: Duration,
        cancel: &CancellationToken,
    ) -> Result<String> {
        let key = (chat_session_id.to_string(), pty_name.to_string());
        let handle = self
            .sessions
            .lock()
            .ok()
            .and_then(|map| map.get(&key).cloned())
            .ok_or_else(|| {
                ZenkoError::Tool(format!(
                    "No active PTY session `{pty_name}` found. Start one with `shell_exec(session=\"{pty_name}\", command=\"...\")`."
                ))
            })?;
        let mut guard = handle.lock().await;
        guard.send_raw(input, wait, cancel).await
    }

    #[cfg(not(unix))]
    pub async fn send_input(
        &self,
        _chat_session_id: &str,
        _pty_name: &str,
        _input: &str,
        _wait: Duration,
        _cancel: &CancellationToken,
    ) -> Result<String> {
        Err(ZenkoError::Tool(
            "Persistent PTY sessions are currently supported on Unix/Linux platforms.".into(),
        ))
    }

    #[cfg(unix)]
    pub async fn list_for_chat(&self, chat_session_id: &str) -> Vec<PtySessionInfo> {
        let handles: Vec<Arc<AsyncMutex<unix_pty::LivePtySession>>> = self
            .sessions
            .lock()
            .map(|map| {
                map.iter()
                    .filter(|((sid, _), _)| sid == chat_session_id)
                    .map(|(_, h)| h.clone())
                    .collect()
            })
            .unwrap_or_default();

        let mut out = Vec::with_capacity(handles.len());
        for h in handles {
            let mut guard = h.lock().await;
            if guard.is_alive() {
                out.push(PtySessionInfo {
                    name: guard.name.clone(),
                    pid: guard.pid,
                    initial_cwd: guard.initial_cwd.display().to_string(),
                    idle_secs: guard.last_used.elapsed().as_secs(),
                });
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    #[cfg(not(unix))]
    pub async fn list_for_chat(&self, _chat_session_id: &str) -> Vec<PtySessionInfo> {
        Vec::new()
    }

    #[cfg(unix)]
    pub async fn close_session(&self, chat_session_id: &str, pty_name: &str) -> bool {
        let key = (chat_session_id.to_string(), pty_name.to_string());
        let removed = self
            .sessions
            .lock()
            .ok()
            .and_then(|mut map| map.remove(&key));
        match removed {
            Some(handle) => {
                let mut guard = handle.lock().await;
                guard.shutdown().await;
                true
            }
            None => false,
        }
    }

    #[cfg(not(unix))]
    pub async fn close_session(&self, _chat_session_id: &str, _pty_name: &str) -> bool {
        false
    }

    #[cfg(unix)]
    pub async fn close_all(&self) -> usize {
        let drained: Vec<Arc<AsyncMutex<unix_pty::LivePtySession>>> = self
            .sessions
            .lock()
            .map(|mut map| map.drain().map(|(_, h)| h).collect())
            .unwrap_or_default();
        let count = drained.len();
        for h in drained {
            let mut guard = h.lock().await;
            guard.shutdown().await;
        }
        count
    }

    #[cfg(not(unix))]
    pub async fn close_all(&self) -> usize {
        0
    }

    #[cfg(unix)]
    fn get_or_spawn(
        &self,
        chat_session_id: &str,
        pty_name: &str,
        cwd: &Path,
        hub_dir: &Path,
        os_sandbox: bool,
    ) -> Result<Arc<AsyncMutex<unix_pty::LivePtySession>>> {
        let key = (chat_session_id.to_string(), pty_name.to_string());
        let mut map = self
            .sessions
            .lock()
            .map_err(|_| ZenkoError::Tool("PTY session map lock poisoned".into()))?;
        if let Some(existing) = map.get(&key) {
            return Ok(existing.clone());
        }
        let chat_count = map
            .keys()
            .filter(|(sid, _)| sid == chat_session_id)
            .count();
        if chat_count >= PTY_MAX_SESSIONS_PER_CHAT {
            return Err(ZenkoError::Tool(format!(
                "Session already has {PTY_MAX_SESSIONS_PER_CHAT} active PTY terminals. Close one with `pty_action=\"close\"` before creating `{pty_name}`."
            )));
        }
        if map.len() >= PTY_MAX_TOTAL_SESSIONS {
            return Err(ZenkoError::Tool(format!(
                "Global PTY pool cap ({PTY_MAX_TOTAL_SESSIONS}) reached. Close unused PTY sessions first."
            )));
        }
        let spawned = unix_pty::LivePtySession::spawn(
            pty_name.to_string(),
            cwd,
            hub_dir,
            os_sandbox,
        )?;
        let handle = Arc::new(AsyncMutex::new(spawned));
        map.insert(key, handle.clone());
        Ok(handle)
    }

    #[cfg(unix)]
    pub async fn reap_idle(&self) -> usize {
        let ttl = Duration::from_secs(PTY_IDLE_TIMEOUT_SECS);
        let candidates: Vec<((String, String), Arc<AsyncMutex<unix_pty::LivePtySession>>)> = self
            .sessions
            .lock()
            .map(|map| map.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();

        let mut stale_keys = Vec::new();
        for (key, handle) in candidates {
            if let Ok(mut guard) = handle.try_lock()
                && (!guard.is_alive() || guard.last_used.elapsed() >= ttl)
            {
                stale_keys.push(key);
            }
        }
        let mut to_close = Vec::new();
        if !stale_keys.is_empty()
            && let Ok(mut map) = self.sessions.lock()
        {
            for k in stale_keys {
                if let Some(h) = map.remove(&k) {
                    to_close.push(h);
                }
            }
        }
        let count = to_close.len();
        for h in to_close {
            let mut guard = h.lock().await;
            guard.shutdown().await;
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_stripper_cleans_csi_osc_and_cr_without_regex() {
        let raw = "\x1b]0;title\x07\x1b[32mPASS\x1b[0m\r\nline2\x1b[1;31m!\x1b[0m\n";
        assert_eq!(strip_ansi_escapes(raw), "PASS\nline2!\n");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn persistent_pty_preserves_env_cwd_and_isatty_across_calls() {
        let dir = std::env::temp_dir().join(format!("zenko_pty_test_{}", zenko_core::new_id()));
        let sub = dir.join("nested_pkg");
        std::fs::create_dir_all(&sub).expect("mkdir");

        let mgr = PtySessionManager::new();
        let cancel = CancellationToken::new();

        // 1. Export variable & cd into nested_pkg inside persistent PTY "dev"
        let r1 = mgr
            .exec(
                "chat1",
                "dev",
                "export ZENKO_PERSIST=ok_99 && cd nested_pkg && [ -t 0 ] && echo TTY_TRUE",
                &dir,
                &dir,
                true,
                Duration::from_secs(10),
                &cancel,
            )
            .await
            .expect("r1");
        assert_eq!(r1.exit_code, 0, "r1={r1:?}");
        assert!(r1.output.contains("TTY_TRUE"), "out={}", r1.output);

        // 2. Second call in same PTY "dev" observes persisted env & cwd!
        let r2 = mgr
            .exec(
                "chat1",
                "dev",
                "printf '%s|%s' \"$ZENKO_PERSIST\" \"$(pwd)\"",
                &dir,
                &dir,
                true,
                Duration::from_secs(10),
                &cancel,
            )
            .await
            .expect("r2");
        assert_eq!(r2.exit_code, 0, "r2={r2:?}");
        assert!(r2.output.contains("ok_99|"), "out={}", r2.output);
        assert!(r2.output.contains("nested_pkg"), "out={}", r2.output);

        // 3. List & close cleanly
        let list = mgr.list_for_chat("chat1").await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "dev");
        assert!(mgr.close_session("chat1", "dev").await);
        assert!(mgr.list_for_chat("chat1").await.is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }
}
