//! Native Rust async JSON-RPC 2.0 Language Server Protocol (LSP) pool & `lsp_tool`.
//! - Strict `Content-Length` byte framing (zero regex, zero external LSP crates).
//! - Declarative `LSP_SERVER_REGISTRY` (`rust-analyzer`, `typescript-language-server`,
//!   `pyright-langserver`, `gopls`, `clangd`) + user `[[lsp.servers]]` overrides.
//! - Lazy per-`(server, project_root)` pooling with automatic idle eviction (low-RAM discipline).
//! - Automatic `didOpen`/`didChange` sync via SHA-256 content hashes + exact UTF-8 <-> UTF-16 columns.
//! - Push (`textDocument/publishDiagnostics`) + pull (`textDocument/diagnostic`) freshness support.

use crate::jail::resolve_readable_path;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex as AsyncMutex, Notify, oneshot};
use tokio_util::sync::CancellationToken;
use zenko_core::{
    LSP_ACTIONS, LSP_AUTO_DIAG_MAX_ITEMS, LSP_AUTO_DIAG_WAIT_MS, LSP_MAX_FRAME_BYTES,
    LSP_MAX_RESULTS, LSP_SERVER_REGISTRY, LSP_STARTUP_TIMEOUT_SECS, LSP_STDERR_RING_BYTES,
    LspAction, LspConfig, LspServerRuntimeStatus, Result, RiskLevel, TOOL_LSP, Tool,
    ToolConcurrency, ToolContext, ToolExposure, ToolOutput, ToolSpec, ZenkoError,
    find_lsp_server_by_extension, find_lsp_server_by_id,
};

#[derive(Debug, Clone)]
struct ResolvedServerSpec {
    id: String,
    language_id: String,
    command: String,
    args: Vec<String>,
    root_markers: Vec<String>,
}

#[derive(Debug, Clone)]
struct OpenDocState {
    version: i64,
    content_hash: String,
}

type DiagnosticsMap = Arc<Mutex<HashMap<String, (u64, Vec<Value>)>>>;

struct LspClientSession {
    server_id: String,
    language_id: String,
    workspace_root: PathBuf,
    stdin: AsyncMutex<ChildStdin>,
    child: AsyncMutex<Option<Child>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    /// Per-URI `(monotonic_generation, diagnostics)` so edits never return stale cached diagnostics.
    diagnostics: DiagnosticsMap,
    diag_gen: Arc<AtomicU64>,
    diag_notify: Arc<Notify>,
    stderr_tail: Arc<Mutex<String>>,
    open_docs: AsyncMutex<HashMap<String, OpenDocState>>,
    supports_pull_diag: AtomicU64,
    last_used: Mutex<(Instant, DateTime<Utc>)>,
    alive: Arc<std::sync::atomic::AtomicBool>,
}

impl LspClientSession {
    async fn start(spec: &ResolvedServerSpec, root: PathBuf) -> Result<Arc<Self>> {
        let resolved_bin = resolve_local_lsp_command(&spec.command, &root);
        let mut cmd = Command::new(&resolved_bin);
        cmd.args(&spec.args)
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| {
            ZenkoError::Tool(format!(
                "LSP server `{}` (`{}`) is unavailable on PATH ({e}). Install `{}` or configure `[[lsp.servers]]` in zenko.toml.",
                spec.id, resolved_bin, spec.command
            ))
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ZenkoError::Tool(format!("failed to capture stdin for `{}`", spec.id)))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ZenkoError::Tool(format!("failed to capture stdout for `{}`", spec.id)))?;
        let stderr = child.stderr.take();

        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let diagnostics: DiagnosticsMap = Arc::new(Mutex::new(HashMap::new()));
        let diag_gen = Arc::new(AtomicU64::new(1));
        let diag_notify = Arc::new(Notify::new());
        let stderr_tail = Arc::new(Mutex::new(String::new()));
        let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));

        if let Some(err_stream) = stderr {
            let tail_ref = stderr_tail.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(err_stream);
                let mut line = String::new();
                while let Ok(n) = reader.read_line(&mut line).await {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut buf) = tail_ref.lock() {
                        buf.push_str(&line);
                        if buf.len() > LSP_STDERR_RING_BYTES {
                            let excess = buf.len() - LSP_STDERR_RING_BYTES;
                            let cut = buf.ceil_char_boundary(excess);
                            buf.drain(..cut);
                        }
                    }
                    line.clear();
                }
            });
        }

        let session = Arc::new(Self {
            server_id: spec.id.clone(),
            language_id: spec.language_id.clone(),
            workspace_root: root.clone(),
            stdin: AsyncMutex::new(stdin),
            child: AsyncMutex::new(Some(child)),
            next_id: AtomicU64::new(1),
            pending: pending.clone(),
            diagnostics: diagnostics.clone(),
            diag_gen: diag_gen.clone(),
            diag_notify: diag_notify.clone(),
            stderr_tail: stderr_tail.clone(),
            open_docs: AsyncMutex::new(HashMap::new()),
            supports_pull_diag: AtomicU64::new(0),
            last_used: Mutex::new((Instant::now(), Utc::now())),
            alive: alive.clone(),
        });

        let weak_sess = Arc::downgrade(&session);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            while let Ok(Some(msg)) = read_lsp_frame(&mut reader).await {
                let has_id = msg.get("id").is_some();
                let method = msg.get("method").and_then(Value::as_str);

                match (has_id, method) {
                    // Response to our request
                    (true, None) => {
                        if let Some(id) = msg.get("id").and_then(Value::as_u64)
                            && let Some(tx) = pending
                                .lock()
                                .ok()
                                .and_then(|mut map| map.remove(&id))
                            {
                                let _ = tx.send(msg);
                            }
                    }
                    // Server -> client request (e.g. `workspace/configuration` expects an array of results)
                    (true, Some(req_method)) => {
                        if let Some(sess) = weak_sess.upgrade() {
                            let result_val = match req_method {
                                "workspace/configuration" => {
                                    let count = msg
                                        .pointer("/params/items")
                                        .and_then(Value::as_array)
                                        .map(|a| a.len())
                                        .unwrap_or(0);
                                    Value::Array(vec![Value::Null; count])
                                }
                                _ => Value::Null,
                            };
                            let reply = json!({
                                "jsonrpc": "2.0",
                                "id": msg.get("id").cloned().unwrap_or(Value::Null),
                                "result": result_val,
                            });
                            let _ = sess.write_raw(&reply).await;
                        }
                    }
                    // Server notification
                    (false, Some("textDocument/publishDiagnostics")) => {
                        if let Some(params) = msg.get("params")
                            && let Some(uri) = params.get("uri").and_then(Value::as_str) {
                                let items = params
                                    .get("diagnostics")
                                    .and_then(Value::as_array)
                                    .cloned()
                                    .unwrap_or_default();
                                let seq = diag_gen.fetch_add(1, Ordering::SeqCst) + 1;
                                if let Ok(mut map) = diagnostics.lock() {
                                    map.insert(uri.to_string(), (seq, items));
                                }
                                diag_notify.notify_waiters();
                            }
                    }
                    _ => {}
                }
            }
            alive.store(false, Ordering::SeqCst);
            if let Ok(mut map) = pending.lock() {
                map.clear();
            }
        });

        // Perform mandatory LSP `initialize` -> `initialized` handshake
        let root_uri = path_to_file_uri(&root);
        let init_params = json!({
            "processId": std::process::id(),
            "clientInfo": { "name": "zenko", "version": env!("CARGO_PKG_VERSION") },
            "rootUri": root_uri,
            "workspaceFolders": [{
                "uri": root_uri,
                "name": root.file_name().and_then(|n| n.to_str()).unwrap_or("workspace"),
            }],
            "capabilities": {
                "textDocument": {
                    "synchronization": { "didSave": true },
                    "publishDiagnostics": { "relatedInformation": true },
                    "diagnostic": { "dynamicRegistration": false },
                    "definition": { "linkSupport": false },
                    "typeDefinition": { "linkSupport": false },
                    "implementation": { "linkSupport": false },
                    "references": {},
                    "hover": { "contentFormat": ["markdown", "plaintext"] },
                    "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
                    "callHierarchy": {}
                },
                "workspace": {
                    "symbol": {},
                    "workspaceFolders": true
                }
            }
        });

        let init_resp = session
            .request(
                "initialize",
                init_params,
                Duration::from_secs(LSP_STARTUP_TIMEOUT_SECS),
                CancellationToken::new(),
            )
            .await?;

        let has_pull = init_resp
            .get("capabilities")
            .and_then(|c| c.get("diagnosticProvider"))
            .is_some();
        if has_pull {
            session.supports_pull_diag.store(1, Ordering::Relaxed);
        }
        session.notify("initialized", json!({})).await?;
        Ok(session)
    }

    fn touch(&self) {
        if let Ok(mut g) = self.last_used.lock() {
            *g = (Instant::now(), Utc::now());
        }
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    fn idle_elapsed(&self) -> Duration {
        self.last_used
            .lock()
            .map(|g| g.0.elapsed())
            .unwrap_or_default()
    }

    async fn write_raw(&self, payload: &Value) -> Result<()> {
        let frame = encode_lsp_frame(payload)?;
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(&frame)
            .await
            .map_err(|e| ZenkoError::Tool(format!("LSP write failed ({}): {e}", self.server_id)))?;
        stdin
            .flush()
            .await
            .map_err(|e| ZenkoError::Tool(format!("LSP flush failed ({}): {e}", self.server_id)))?;
        Ok(())
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.touch();
        self.write_raw(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .await
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> Result<Value> {
        self.touch();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        if let Ok(mut map) = self.pending.lock() {
            map.insert(id, tx);
        }
        if let Err(e) = self
            .write_raw(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            }))
            .await
        {
            if let Ok(mut map) = self.pending.lock() {
                map.remove(&id);
            }
            return Err(e);
        }

        let resp = tokio::select! {
            _ = cancel.cancelled() => {
                if let Ok(mut map) = self.pending.lock() {
                    map.remove(&id);
                }
                return Err(ZenkoError::Cancelled);
            }
            res = tokio::time::timeout(timeout, rx) => match res {
                Ok(Ok(val)) => val,
                Ok(Err(_)) => {
                    let stderr_note = self.stderr_summary();
                    return Err(ZenkoError::Tool(format!(
                        "LSP server `{}` exited while handling `{method}`{stderr_note}",
                        self.server_id
                    )));
                }
                Err(_) => {
                    if let Ok(mut map) = self.pending.lock() {
                        map.remove(&id);
                    }
                    return Err(ZenkoError::Tool(format!(
                        "LSP `{}` request `{method}` timed out after {}s",
                        self.server_id,
                        timeout.as_secs()
                    )));
                }
            }
        };

        if let Some(err) = resp.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown LSP error");
            return Err(ZenkoError::Tool(format!(
                "LSP `{}` error in `{method}`: {msg}",
                self.server_id
            )));
        }
        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    fn stderr_summary(&self) -> String {
        self.stderr_tail
            .lock()
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|s| format!(" (stderr: {})", s.lines().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ")))
            .unwrap_or_default()
    }

    /// Ensures `abs_path` is opened & synchronized with the language server via SHA-256 diff check,
    /// sending `didSave` on change and returning `(uri, text, changed, before_diag_gen)` so push
    /// diagnostics never return stale results after an edit!
    async fn ensure_document_synced(&self, abs_path: &Path) -> Result<(String, String, bool, u64)> {
        let text = tokio::fs::read_to_string(abs_path).await?;
        let uri = path_to_file_uri(abs_path);
        let hash = sha256_hex(text.as_bytes());
        let before_gen = self.diag_gen.load(Ordering::SeqCst);
        let mut docs = self.open_docs.lock().await;
        let changed = match docs.get_mut(&uri) {
            None => {
                docs.insert(
                    uri.clone(),
                    OpenDocState {
                        version: 1,
                        content_hash: hash,
                    },
                );
                drop(docs);
                if let Ok(mut map) = self.diagnostics.lock() {
                    map.remove(&uri);
                }
                self.notify(
                    "textDocument/didOpen",
                    json!({
                        "textDocument": {
                            "uri": uri,
                            "languageId": self.language_id,
                            "version": 1,
                            "text": text,
                        }
                    }),
                )
                .await?;
                let _ = self
                    .notify(
                        "textDocument/didSave",
                        json!({
                            "textDocument": { "uri": uri },
                            "text": text,
                        }),
                    )
                    .await;
                true
            }
            Some(state) if state.content_hash != hash => {
                state.version += 1;
                state.content_hash = hash;
                let ver = state.version;
                drop(docs);
                if let Ok(mut map) = self.diagnostics.lock() {
                    map.remove(&uri);
                }
                self.notify(
                    "textDocument/didChange",
                    json!({
                        "textDocument": { "uri": uri, "version": ver },
                        "contentChanges": [{ "text": text }],
                    }),
                )
                .await?;
                let _ = self
                    .notify(
                        "textDocument/didSave",
                        json!({
                            "textDocument": { "uri": uri },
                            "text": text,
                        }),
                    )
                    .await;
                true
            }
            Some(_) => false,
        };
        Ok((uri, text, changed, before_gen))
    }

    async fn status(&self) -> LspServerRuntimeStatus {
        let open_documents = self.open_docs.lock().await.len();
        let cached_diagnostics_files = self
            .diagnostics
            .lock()
            .map(|m| m.len())
            .unwrap_or(0);
        let last_used_at = self
            .last_used
            .lock()
            .map(|g| g.1)
            .unwrap_or_else(|_| Utc::now());
        LspServerRuntimeStatus {
            server_id: self.server_id.clone(),
            language_id: self.language_id.clone(),
            workspace_root: self.workspace_root.display().to_string(),
            running: self.is_alive(),
            open_documents,
            cached_diagnostics_files,
            last_used_at,
        }
    }

    async fn shutdown(&self) {
        let _ = self
            .request(
                "shutdown",
                Value::Null,
                Duration::from_secs(2),
                CancellationToken::new(),
            )
            .await;
        let _ = self.notify("exit", Value::Null).await;
        self.alive.store(false, Ordering::SeqCst);
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    }
}

/// Shared lazy-pooled LSP server supervisor with automatic idle eviction & graceful shutdown.
pub struct LspPoolManager {
    config: Arc<RwLock<LspConfig>>,
    sessions: AsyncMutex<HashMap<(String, PathBuf), Arc<LspClientSession>>>,
}

impl LspPoolManager {
    pub fn new(config: Arc<RwLock<LspConfig>>) -> Arc<Self> {
        Arc::new(Self {
            config,
            sessions: AsyncMutex::new(HashMap::new()),
        })
    }

    fn current_config(&self) -> LspConfig {
        self.config.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub async fn list_status(&self) -> Vec<LspServerRuntimeStatus> {
        let list: Vec<Arc<LspClientSession>> = self
            .sessions
            .lock()
            .await
            .values()
            .cloned()
            .collect();
        let mut out = Vec::with_capacity(list.len());
        for s in list {
            out.push(s.status().await);
        }
        out.sort_by(|a, b| a.server_id.cmp(&b.server_id));
        out
    }

    pub async fn evict_idle(&self) -> usize {
        let idle_ttl = Duration::from_secs(self.current_config().idle_timeout_secs.max(30));
        let mut to_stop = Vec::new();
        {
            let mut map = self.sessions.lock().await;
            let stale_keys: Vec<(String, PathBuf)> = map
                .iter()
                .filter(|(_, s)| !s.is_alive() || s.idle_elapsed() >= idle_ttl)
                .map(|(k, _)| k.clone())
                .collect();
            for k in stale_keys {
                if let Some(s) = map.remove(&k) {
                    to_stop.push(s);
                }
            }
        }
        let count = to_stop.len();
        for s in to_stop {
            s.shutdown().await;
        }
        count
    }

    pub async fn stop_all(&self) -> usize {
        let drained: Vec<Arc<LspClientSession>> = {
            let mut map = self.sessions.lock().await;
            map.drain().map(|(_, s)| s).collect()
        };
        let count = drained.len();
        for s in drained {
            s.shutdown().await;
        }
        count
    }

    fn resolve_server_for_file(
        &self,
        cfg: &LspConfig,
        abs_path: &Path,
        explicit_server: Option<&str>,
    ) -> Result<ResolvedServerSpec> {
        if let Some(id) = explicit_server.map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(ov) = cfg.servers.iter().find(|s| s.id.eq_ignore_ascii_case(id)) {
                if !ov.enabled {
                    return Err(ZenkoError::PolicyDenied(format!(
                        "LSP server `{id}` is disabled in zenko.toml"
                    )));
                }
                let inferred_lang = abs_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .and_then(find_lsp_server_by_extension)
                    .or_else(|| find_lsp_server_by_id(&ov.id))
                    .map(|r| r.language_id.to_string())
                    .unwrap_or_else(|| ov.id.clone());
                return Ok(ResolvedServerSpec {
                    id: ov.id.clone(),
                    language_id: inferred_lang,
                    command: ov.command.clone(),
                    args: ov.args.clone(),
                    root_markers: ov.root_markers.clone(),
                });
            }
            if let Some(reg) = find_lsp_server_by_id(id) {
                return Ok(ResolvedServerSpec {
                    id: reg.id.to_string(),
                    language_id: reg.language_id.to_string(),
                    command: reg.command.to_string(),
                    args: reg.args.iter().map(|s| (*s).to_string()).collect(),
                    root_markers: reg.root_markers.iter().map(|s| (*s).to_string()).collect(),
                });
            }
            return Err(ZenkoError::Tool(format!("Unknown LSP server `{id}`.")));
        }

        let ext = abs_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if let Some(ov) = cfg.servers.iter().find(|s| {
            s.enabled
                && s.extensions
                    .iter()
                    .any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(&ext))
        }) {
            let inferred_lang = find_lsp_server_by_extension(&ext)
                .or_else(|| find_lsp_server_by_id(&ov.id))
                .map(|r| r.language_id.to_string())
                .unwrap_or_else(|| ov.id.clone());
            return Ok(ResolvedServerSpec {
                id: ov.id.clone(),
                language_id: inferred_lang,
                command: ov.command.clone(),
                args: ov.args.clone(),
                root_markers: ov.root_markers.clone(),
            });
        }
        let reg = find_lsp_server_by_extension(&ext).ok_or_else(|| {
            ZenkoError::Tool(format!(
                "No LSP server configured for `.{ext}` files. Supported built-ins: {}.",
                LSP_SERVER_REGISTRY
                    .iter()
                    .map(|s| s.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        if let Some(ov) = cfg.servers.iter().find(|s| s.id.eq_ignore_ascii_case(reg.id)) {
            if !ov.enabled {
                return Err(ZenkoError::PolicyDenied(format!(
                    "LSP server `{}` is disabled in zenko.toml",
                    reg.id
                )));
            }
            return Ok(ResolvedServerSpec {
                id: ov.id.clone(),
                language_id: reg.language_id.to_string(),
                command: ov.command.clone(),
                args: ov.args.clone(),
                root_markers: match ov.root_markers.is_empty() {
                    true => reg.root_markers.iter().map(|s| (*s).to_string()).collect(),
                    false => ov.root_markers.clone(),
                },
            });
        }
        Ok(ResolvedServerSpec {
            id: reg.id.to_string(),
            language_id: reg.language_id.to_string(),
            command: reg.command.to_string(),
            args: reg.args.iter().map(|s| (*s).to_string()).collect(),
            root_markers: reg.root_markers.iter().map(|s| (*s).to_string()).collect(),
        })
    }

    async fn get_or_spawn(
        &self,
        spec: &ResolvedServerSpec,
        root: PathBuf,
    ) -> Result<Arc<LspClientSession>> {
        let key = (spec.id.clone(), root.clone());
        {
            let mut map = self.sessions.lock().await;
            if let Some(existing) = map.get(&key) {
                if existing.is_alive() {
                    existing.touch();
                    return Ok(existing.clone());
                }
                map.remove(&key);
            }
        }
        let fresh = LspClientSession::start(spec, root).await?;
        let winner = {
            let mut map = self.sessions.lock().await;
            if let Some(existing) = map.get(&key) {
                if existing.is_alive() {
                    existing.touch();
                    Some(existing.clone())
                } else {
                    map.insert(key, fresh.clone());
                    None
                }
            } else {
                map.insert(key, fresh.clone());
                None
            }
        };
        if let Some(existing) = winner {
            fresh.shutdown().await;
            return Ok(existing);
        }
        Ok(fresh)
    }

    /// Oh My Pi–style Diagnostics-on-Edit hook: if an LSP server for `abs_path` is ALREADY warm
    /// and `auto_diagnostics_on_edit` is enabled, syncs the edited file (`didChange` + `didSave`)
    /// and returns a compact `<lsp_diagnostics>` block of fresh compiler/type errors (never cold-spawns!).
    pub async fn warm_diagnostics_for_edited_file(
        &self,
        ws_root: &Path,
        abs_path: &Path,
        cancel: CancellationToken,
    ) -> Option<String> {
        let cfg = self.current_config();
        if !cfg.enabled || !cfg.auto_diagnostics_on_edit || !abs_path.is_file() {
            return None;
        }
        let spec = self.resolve_server_for_file(&cfg, abs_path, None).ok()?;
        let ws_canon = ws_root.canonicalize().unwrap_or_else(|_| ws_root.to_path_buf());
        let root = find_project_root(abs_path, &ws_canon, &spec.root_markers);
        let key = (spec.id.clone(), root);
        let sess = {
            let map = self.sessions.lock().await;
            map.get(&key).filter(|s| s.is_alive()).cloned()?
        };
        let (uri, file_text, changed, before_gen) = sess.ensure_document_synced(abs_path).await.ok()?;
        let wait_ms = cfg.diagnostics_wait_ms.min(LSP_AUTO_DIAG_WAIT_MS).clamp(80, 1500);
        let items = collect_diagnostics(
            &sess,
            &uri,
            changed,
            before_gen,
            Duration::from_millis(wait_ms),
            Duration::from_secs(cfg.request_timeout_secs.clamp(2, 5)),
            cancel,
        )
        .await
        .ok()?;
        // Surface ERROR (1) and WARNING (2) diagnostics concisely; ignore noisy hints on edit
        let actionable: Vec<&Value> = items
            .iter()
            .filter(|d| d.get("severity").and_then(Value::as_u64).unwrap_or(1) <= 2)
            .take(LSP_AUTO_DIAG_MAX_ITEMS)
            .collect();
        if actionable.is_empty() {
            return None;
        }
        let rel = abs_path
            .strip_prefix(&ws_canon)
            .unwrap_or(abs_path)
            .display()
            .to_string();
        let file_lines: Vec<&str> = file_text.lines().collect();
        let formatted: Vec<String> = actionable
            .iter()
            .map(|d| format_single_diagnostic(&rel, &file_lines, d))
            .collect();
        Some(format!(
            "\n\n<lsp_diagnostics server=\"{}\" file=\"{rel}\" count=\"{}\">\n{}\n</lsp_diagnostics>",
            spec.id,
            actionable.len(),
            formatted.join("\n")
        ))
    }
}

pub struct LspTool {
    pool: Arc<LspPoolManager>,
}

impl LspTool {
    pub fn new(pool: Arc<LspPoolManager>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl Tool for LspTool {
    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Shared
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn spec(&self) -> ToolSpec {
        let action_enum: Vec<Value> = LSP_ACTIONS
            .iter()
            .map(|&a| Value::String(a.to_string()))
            .collect();
        ToolSpec {
            name: TOOL_LSP.into(),
            description: "IDE-grade semantic code intelligence via pooled local Language Servers (`rust-analyzer`, `typescript-language-server`, `pyright-langserver`, `gopls`, `clangd`): instant `diagnostics`, `definition`, `references`, `hover`, `document_symbols`, `workspace_symbols`, `incoming_calls`, `outgoing_calls`, and `status`.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": action_enum,
                        "description": "LSP code intelligence action."
                    },
                    "path": { "type": "string", "description": "Workspace file path (required for file-scoped actions)." },
                    "line": { "type": "integer", "description": "1-based line number (for definition, references, hover, calls)." },
                    "column": { "type": "integer", "description": "1-based UTF-8 character column (for definition, references, hover, calls)." },
                    "query": { "type": "string", "description": "Symbol query string (for `workspace_symbols`)." },
                    "server": { "type": "string", "description": "Optional explicit LSP server ID override (e.g. `rust-analyzer`)." }
                },
                "required": ["action"]
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
        let cfg = self.pool.current_config();
        if !cfg.enabled {
            return Ok(ToolOutput::err(
                "`lsp_tool` is disabled in `zenko.toml` (`[lsp].enabled = false`).",
            ));
        }
        let raw_action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool("missing `action`".into()))?;
        let action = LspAction::parse(raw_action).ok_or_else(|| {
            ZenkoError::Tool(format!(
                "Unknown `lsp_tool` action `{raw_action}`. Valid actions: {}.",
                LSP_ACTIONS.join(", ")
            ))
        })?;

        if action == LspAction::Status {
            let status = self.pool.list_status().await;
            let text = match status.is_empty() {
                true => "No active LSP servers running (servers spawn lazily on first code intelligence request).".to_string(),
                false => status
                    .iter()
                    .map(|s| {
                        format!(
                            "- {} [{}] root={} docs={} diag_files={}",
                            s.server_id,
                            match s.running {
                                true => "running",
                                false => "stopped",
                            },
                            s.workspace_root,
                            s.open_documents,
                            s.cached_diagnostics_files
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            let mut out = ToolOutput::ok(text);
            out.metadata = serde_json::to_value(&status).ok();
            return Ok(out);
        }

        let explicit_server = args.get("server").and_then(Value::as_str);
        let timeout = Duration::from_secs(cfg.request_timeout_secs.clamp(3, 60));
        let ws_canon = ctx
            .workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| ctx.workspace_dir.clone());

        // `workspace_symbols` can run against any active server or explicit `server` or inferred `path`
        if action == LspAction::WorkspaceSymbols {
            let query = args
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let sess = if let Some(req_path) = args.get("path").and_then(Value::as_str) {
                let abs = resolve_readable_path(
                    &ctx.workspace_dir,
                    &ctx.hub_dir,
                    &ctx.data_dir,
                    req_path,
                    true,
                )?;
                let spec = self.pool.resolve_server_for_file(&cfg, &abs, explicit_server)?;
                let root = find_project_root(&abs, &ws_canon, &spec.root_markers);
                self.pool.get_or_spawn(&spec, root).await?
            } else if let Some(id) = explicit_server {
                let spec = self
                    .pool
                    .resolve_server_for_file(&cfg, &ws_canon, Some(id))?;
                self.pool.get_or_spawn(&spec, ws_canon.clone()).await?
            } else {
                let active = self
                    .pool
                    .sessions
                    .lock()
                    .await
                    .values()
                    .find(|s| s.is_alive())
                    .cloned();
                match active {
                    Some(s) => s,
                    None => {
                        return Ok(ToolOutput::err(
                            "Pass `path` (any file in the target language) or `server` (e.g. `rust-analyzer`) to start the LSP server for `workspace_symbols`.",
                        ));
                    }
                }
            };
            let res = sess
                .request("workspace/symbol", json!({ "query": query }), timeout, cancel)
                .await?;
            return Ok(format_symbols_output(&ws_canon, &res, "Workspace Symbols"));
        }

        let req_path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ZenkoError::Tool(format!("`path` is required for `lsp_tool(action=\"{}\")`", action.as_str())))?;
        let abs_path = resolve_readable_path(
            &ctx.workspace_dir,
            &ctx.hub_dir,
            &ctx.data_dir,
            req_path,
            true,
        )?;
        let spec = self
            .pool
            .resolve_server_for_file(&cfg, &abs_path, explicit_server)?;
        let root = find_project_root(&abs_path, &ws_canon, &spec.root_markers);
        let sess = self.pool.get_or_spawn(&spec, root).await?;
        let (uri, file_text, changed, before_gen) = sess.ensure_document_synced(&abs_path).await?;

        match action {
            LspAction::Diagnostics => {
                let items = collect_diagnostics(
                    &sess,
                    &uri,
                    changed,
                    before_gen,
                    Duration::from_millis(cfg.diagnostics_wait_ms.clamp(100, 5_000)),
                    timeout,
                    cancel,
                )
                .await?;
                Ok(format_diagnostics_output(req_path, &file_text, &items))
            }
            LspAction::DocumentSymbols => {
                let res = sess
                    .request(
                        "textDocument/documentSymbol",
                        json!({ "textDocument": { "uri": uri } }),
                        timeout,
                        cancel,
                    )
                    .await?;
                Ok(format_symbols_output(&ws_canon, &res, &format!("Symbols in {req_path}")))
            }
            LspAction::Definition
            | LspAction::TypeDefinition
            | LspAction::Implementation
            | LspAction::References
            | LspAction::Hover
            | LspAction::IncomingCalls
            | LspAction::OutgoingCalls => {
                let line_1 = args
                    .get("line")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| ZenkoError::Tool("`line` (1-based) is required".into()))?
                    .max(1) as usize;
                let col_1 = args
                    .get("column")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .max(1) as usize;
                let pos = to_lsp_position(&file_text, line_1, col_1);

                match action {
                    LspAction::Definition => {
                        let res = sess
                            .request(
                                "textDocument/definition",
                                json!({ "textDocument": { "uri": uri }, "position": pos }),
                                timeout,
                                cancel,
                            )
                            .await?;
                        Ok(format_locations_output(&ws_canon, &res, "Definition").await)
                    }
                    LspAction::TypeDefinition => {
                        let res = sess
                            .request(
                                "textDocument/typeDefinition",
                                json!({ "textDocument": { "uri": uri }, "position": pos }),
                                timeout,
                                cancel,
                            )
                            .await?;
                        Ok(format_locations_output(&ws_canon, &res, "Type Definition").await)
                    }
                    LspAction::Implementation => {
                        let res = sess
                            .request(
                                "textDocument/implementation",
                                json!({ "textDocument": { "uri": uri }, "position": pos }),
                                timeout,
                                cancel,
                            )
                            .await?;
                        Ok(format_locations_output(&ws_canon, &res, "Implementation").await)
                    }
                    LspAction::References => {
                        let res = sess
                            .request(
                                "textDocument/references",
                                json!({
                                    "textDocument": { "uri": uri },
                                    "position": pos,
                                    "context": { "includeDeclaration": true }
                                }),
                                timeout,
                                cancel,
                            )
                            .await?;
                        Ok(format_locations_output(&ws_canon, &res, "References").await)
                    }
                    LspAction::Hover => {
                        let res = sess
                            .request(
                                "textDocument/hover",
                                json!({ "textDocument": { "uri": uri }, "position": pos }),
                                timeout,
                                cancel,
                            )
                            .await?;
                        Ok(format_hover_output(&res))
                    }
                    LspAction::IncomingCalls | LspAction::OutgoingCalls => {
                        let prep = sess
                            .request(
                                "textDocument/prepareCallHierarchy",
                                json!({ "textDocument": { "uri": uri }, "position": pos }),
                                timeout,
                                cancel.clone(),
                            )
                            .await?;
                        let Some(item) = prep.as_array().and_then(|a| a.first()).cloned() else {
                            return Ok(ToolOutput::ok("No call hierarchy symbol found at position."));
                        };
                        let rpc_method = match action {
                            LspAction::IncomingCalls => "callHierarchy/incomingCalls",
                            _ => "callHierarchy/outgoingCalls",
                        };
                        let calls = sess
                            .request(rpc_method, json!({ "item": item }), timeout, cancel)
                            .await?;
                        Ok(format_call_hierarchy_output(&ws_canon, &calls, action))
                    }
                    _ => Ok(ToolOutput::ok("ok")),
                }
            }
            LspAction::Status | LspAction::WorkspaceSymbols => Ok(ToolOutput::ok("ok")),
        }
    }
}

async fn collect_diagnostics(
    sess: &LspClientSession,
    uri: &str,
    changed: bool,
    before_gen: u64,
    wait_budget: Duration,
    req_timeout: Duration,
    cancel: CancellationToken,
) -> Result<Vec<Value>> {
    if sess.supports_pull_diag.load(Ordering::Relaxed) == 1
        && let Ok(res) = sess
            .request(
                "textDocument/diagnostic",
                json!({ "textDocument": { "uri": uri } }),
                req_timeout,
                cancel.clone(),
            )
            .await
            && let Some(items) = res.get("items").and_then(Value::as_array) {
                return Ok(items.clone());
            }

    let deadline = tokio::time::Instant::now() + wait_budget;
    loop {
        let ready = sess
            .diagnostics
            .lock()
            .ok()
            .and_then(|m| m.get(uri).map(|(g, items)| (!changed || *g > before_gen, items.clone())));
        if let Some((true, items)) = ready {
            return Ok(items);
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            break;
        }
        let notified = sess.diag_notify.notified();
        // Re-check after registering waiter so zero notifications can slip through!
        let recheck = sess
            .diagnostics
            .lock()
            .ok()
            .and_then(|m| m.get(uri).map(|(g, items)| (!changed || *g > before_gen, items.clone())));
        if let Some((true, items)) = recheck {
            return Ok(items);
        }
        tokio::select! {
            _ = cancel.cancelled() => return Err(ZenkoError::Cancelled),
            _ = tokio::time::sleep_until(deadline) => break,
            _ = notified => {}
        }
    }
    Ok(sess
        .diagnostics
        .lock()
        .ok()
        .and_then(|m| m.get(uri).map(|(_, items)| items.clone()))
        .unwrap_or_default())
}

/// Encodes a JSON-RPC 2.0 LSP message with strict `Content-Length` framing.
pub fn encode_lsp_frame(payload: &Value) -> Result<Vec<u8>> {
    let body = serde_json::to_vec(payload)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut frame = Vec::with_capacity(header.len() + body.len());
    frame.extend_from_slice(header.as_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Reads a single `Content-Length`-framed LSP JSON-RPC message deterministically (zero regex).
pub async fn read_lsp_frame<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((key, val)) = trimmed.split_once(':')
            && key.trim().eq_ignore_ascii_case("Content-Length")
                && let Ok(len) = val.trim().parse::<usize>() {
                    content_length = Some(len);
                }
    }
    let Some(len) = content_length else {
        return Ok(None);
    };
    if len > LSP_MAX_FRAME_BYTES {
        return Err(ZenkoError::Tool(format!(
            "LSP frame exceeded {} MiB safety bound ({len} bytes)",
            LSP_MAX_FRAME_BYTES / (1024 * 1024)
        )));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    let val: Value = serde_json::from_slice(&body)?;
    Ok(Some(val))
}

/// Converts 1-based `(line, utf8_char_col)` into 0-based LSP `{ line, character }` in UTF-16 code units.
pub fn to_lsp_position(file_text: &str, line_1: usize, col_1: usize) -> Value {
    let line_0 = line_1.saturating_sub(1);
    let line_str = file_text.lines().nth(line_0).unwrap_or("");
    let utf16_col = utf8_char_col_to_utf16(line_str, col_1.saturating_sub(1));
    json!({ "line": line_0, "character": utf16_col })
}

pub fn utf8_char_col_to_utf16(line: &str, char_col_0: usize) -> usize {
    line.chars()
        .take(char_col_0)
        .map(char::len_utf16)
        .sum()
}

pub fn utf16_col_to_utf8_char(line: &str, utf16_col_0: usize) -> usize {
    let mut units = 0usize;
    let mut chars = 0usize;
    for ch in line.chars() {
        let len = ch.len_utf16();
        if units + len > utf16_col_0 {
            break;
        }
        units += len;
        chars += 1;
    }
    chars
}

/// Resolves project-local LSP binaries (`node_modules/.bin/<cmd>`, `.venv/bin/<cmd>`,
/// `venv/bin/<cmd>`) before falling back to global `$PATH` (Oh My Pi local-env pattern).
fn resolve_local_lsp_command(command: &str, project_root: &Path) -> String {
    if command.contains('/') || command.contains('\\') {
        return command.to_string();
    }
    for rel in ["node_modules/.bin", ".venv/bin", "venv/bin"] {
        let candidate = project_root.join(rel).join(command);
        if candidate.is_file() {
            return candidate.display().to_string();
        }
    }
    command.to_string()
}

fn find_project_root(abs_file: &Path, ws_root: &Path, markers: &[String]) -> PathBuf {
    let mut cur = match abs_file.is_dir() {
        true => Some(abs_file),
        false => abs_file.parent(),
    };
    while let Some(dir) = cur {
        if markers.iter().any(|m| dir.join(m).exists()) {
            return dir.to_path_buf();
        }
        if dir == ws_root || !dir.starts_with(ws_root) {
            break;
        }
        cur = dir.parent();
    }
    ws_root.to_path_buf()
}

pub fn path_to_file_uri(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let mut out = String::with_capacity(raw.len() + 12);
    out.push_str("file://");
    if !raw.starts_with('/') {
        out.push('/');
    }
    for b in raw.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => {
                out.push(*b as char);
            }
            other => {
                out.push_str(&format!("%{other:02X}"));
            }
        }
    }
    out
}

pub fn file_uri_to_display(ws_root: &Path, uri: &str) -> String {
    let stripped = uri.strip_prefix("file://").unwrap_or(uri);
    let decoded = decode_uri_path(stripped);
    let p = Path::new(&decoded);
    p.strip_prefix(ws_root)
        .unwrap_or(p)
        .display()
        .to_string()
}

fn decode_uri_path(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

fn severity_label(sev: u64) -> &'static str {
    match sev {
        1 => "ERROR",
        2 => "WARN",
        3 => "INFO",
        4 => "HINT",
        _ => "DIAG",
    }
}

fn symbol_kind_label(kind: u64) -> &'static str {
    match kind {
        2 => "module",
        5 => "class",
        6 => "method",
        10 => "enum",
        11 => "interface",
        12 => "fn",
        13 => "var",
        14 => "const",
        23 => "struct",
        _ => "sym",
    }
}

fn format_single_diagnostic(path: &str, file_lines: &[&str], d: &Value) -> String {
    let line_0 = d
        .pointer("/range/start/line")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let utf16_col = d
        .pointer("/range/start/character")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let col_1 = file_lines
        .get(line_0)
        .map(|l| utf16_col_to_utf8_char(l, utf16_col) + 1)
        .unwrap_or(utf16_col + 1);
    let sev = severity_label(d.get("severity").and_then(Value::as_u64).unwrap_or(1));
    let msg = d.get("message").and_then(Value::as_str).unwrap_or("");
    format!("{path}:{}:{col_1} [{sev}] {msg}", line_0 + 1)
}

fn format_diagnostics_output(path: &str, file_text: &str, items: &[Value]) -> ToolOutput {
    if items.is_empty() {
        return ToolOutput::ok(format!("Diagnostics for `{path}`: 0 issues."));
    }
    let file_lines: Vec<&str> = file_text.lines().collect();
    let lines: Vec<String> = items
        .iter()
        .take(LSP_MAX_RESULTS)
        .map(|d| format_single_diagnostic(path, &file_lines, d))
        .collect();
    let mut out = ToolOutput::ok(format!(
        "Diagnostics for `{path}` ({} issue(s)):\n{}",
        items.len(),
        lines.join("\n")
    ));
    out.metadata = Some(json!({ "count": items.len(), "diagnostics": items }));
    out
}

const LSP_LOCATION_SNIPPET_MAX_BYTES: u64 = 256 * 1024;

async fn format_locations_output(ws_root: &Path, res: &Value, label: &str) -> ToolOutput {
    let locs: Vec<&Value> = match res {
        Value::Array(arr) => arr.iter().collect(),
        Value::Object(_) => vec![res],
        _ => Vec::new(),
    };
    if locs.is_empty() {
        return ToolOutput::ok(format!("{label}: (none)"));
    }
    let mut line_cache: HashMap<String, Vec<String>> = HashMap::new();
    let mut lines: Vec<String> = Vec::new();
    for loc in locs.iter().take(LSP_MAX_RESULTS) {
        let Some(uri) = loc
            .get("uri")
            .or_else(|| loc.get("targetUri"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(range) = loc
            .get("range")
            .or_else(|| loc.get("targetSelectionRange"))
            .or_else(|| loc.get("targetRange"))
        else {
            continue;
        };
        let line_0 = range
            .pointer("/start/line")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let utf16_col = range
            .pointer("/start/character")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let decoded_path = decode_uri_path(uri.strip_prefix("file://").unwrap_or(uri));
        if !line_cache.contains_key(&decoded_path) {
            let loaded = match tokio::fs::canonicalize(&decoded_path).await {
                Ok(canon) if canon.starts_with(ws_root) => match tokio::fs::metadata(&canon).await {
                    Ok(meta) if meta.is_file() && meta.len() <= LSP_LOCATION_SNIPPET_MAX_BYTES => {
                        tokio::fs::read_to_string(&canon)
                            .await
                            .map(|s| s.lines().map(String::from).collect())
                            .unwrap_or_default()
                    }
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
            line_cache.insert(decoded_path.clone(), loaded);
        }
        let col_1 = line_cache
            .get(&decoded_path)
            .and_then(|file_lines| file_lines.get(line_0))
            .map(|l| utf16_col_to_utf8_char(l, utf16_col) + 1)
            .unwrap_or(utf16_col + 1);
        lines.push(format!("{}:{}:{col_1}", file_uri_to_display(ws_root, uri), line_0 + 1));
    }
    let mut out = ToolOutput::ok(format!("{label} ({}):\n{}", lines.len(), lines.join("\n")));
    out.metadata = Some(json!({ "locations": lines }));
    out
}

fn format_hover_output(res: &Value) -> ToolOutput {
    let Some(contents) = res.get("contents") else {
        return ToolOutput::ok("Hover: (no type/documentation info at position)");
    };
    let text = match contents {
        Value::String(s) => s.clone(),
        Value::Object(map) => map
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        Value::Array(arr) => arr
            .iter()
            .filter_map(|item| match item {
                Value::String(s) => Some(s.as_str()),
                Value::Object(m) => m.get("value").and_then(Value::as_str),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    };
    match text.trim().is_empty() {
        true => ToolOutput::ok("Hover: (empty)"),
        false => ToolOutput::ok(text),
    }
}

fn format_symbols_output(ws_root: &Path, res: &Value, header: &str) -> ToolOutput {
    let Some(arr) = res.as_array() else {
        return ToolOutput::ok(format!("{header}: (none)"));
    };
    let mut lines = Vec::new();
    collect_symbol_lines(ws_root, arr, 0, &mut lines);
    match lines.is_empty() {
        true => ToolOutput::ok(format!("{header}: (none)")),
        false => ToolOutput::ok(format!("{header} ({}):\n{}", lines.len(), lines.join("\n"))),
    }
}

fn collect_symbol_lines(ws_root: &Path, items: &[Value], depth: usize, out: &mut Vec<String>) {
    let indent = "  ".repeat(depth.min(4));
    for item in items {
        if out.len() >= LSP_MAX_RESULTS {
            return;
        }
        let name = item.get("name").and_then(Value::as_str).unwrap_or("");
        let kind = symbol_kind_label(item.get("kind").and_then(Value::as_u64).unwrap_or(0));
        let loc_tag = if let Some(uri) = item.pointer("/location/uri").and_then(Value::as_str) {
            let line = item
                .pointer("/location/range/start/line")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                + 1;
            format!(" ({}:{line})", file_uri_to_display(ws_root, uri))
        } else if let Some(line) = item
            .pointer("/selectionRange/start/line")
            .or_else(|| item.pointer("/range/start/line"))
            .and_then(Value::as_u64)
        {
            format!(" (L{})", line + 1)
        } else {
            String::new()
        };
        out.push(format!("{indent}- [{kind}] {name}{loc_tag}"));
        if let Some(children) = item.get("children").and_then(Value::as_array) {
            collect_symbol_lines(ws_root, children, depth + 1, out);
        }
    }
}

fn format_call_hierarchy_output(ws_root: &Path, res: &Value, action: LspAction) -> ToolOutput {
    let key = match action {
        LspAction::IncomingCalls => "from",
        _ => "to",
    };
    let Some(arr) = res.as_array() else {
        return ToolOutput::ok(format!("{}: (none)", action.as_str()));
    };
    let lines: Vec<String> = arr
        .iter()
        .take(LSP_MAX_RESULTS)
        .filter_map(|entry| {
            let target = entry.get(key)?;
            let name = target.get("name").and_then(Value::as_str)?;
            let uri = target.get("uri").and_then(Value::as_str)?;
            let line = target
                .pointer("/selectionRange/start/line")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                + 1;
            Some(format!("- {name} ({}:{line})", file_uri_to_display(ws_root, uri)))
        })
        .collect();
    match lines.is_empty() {
        true => ToolOutput::ok(format!("{}: (none)", action.as_str())),
        false => ToolOutput::ok(format!(
            "{} ({}):\n{}",
            action.as_str(),
            lines.len(),
            lines.join("\n")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenko_core::LspServerOverrideConfig;

    #[tokio::test]
    async fn lsp_framing_and_utf16_position_conversion_work_without_regex() {
        let payload = json!({"jsonrpc": "2.0", "id": 7, "method": "textDocument/hover"});
        let frame = encode_lsp_frame(&payload).expect("encode");
        let mut reader = BufReader::new(&frame[..]);
        let decoded = read_lsp_frame(&mut reader)
            .await
            .expect("read")
            .expect("some");
        assert_eq!(decoded, payload);

        // Emoji `🚀` is 1 Rust char, 2 UTF-16 code units!
        let line = "let 🚀 = 42;";
        assert_eq!(utf8_char_col_to_utf16(line, 5), 6);
        assert_eq!(utf16_col_to_utf8_char(line, 6), 5);
    }

    #[tokio::test]
    async fn lsp_pool_roundtrips_real_content_length_stdio_mock_server() {
        let dir = std::env::temp_dir().join(format!("zenko_lsp_test_{}", zenko_core::new_id()));
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("main.rs"), "fn hello() -> u32 { 42 }\n").expect("write");

        let mock_py = r#"
import sys, json

def read_msg():
    cl = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        s = line.decode('utf-8', 'replace').strip()
        if not s:
            break
        if s.lower().startswith('content-length:'):
            cl = int(s.split(':', 1)[1].strip())
    if cl is None:
        return None
    body = sys.stdin.buffer.read(cl)
    return json.loads(body.decode('utf-8'))

def send_msg(obj):
    raw = json.dumps(obj).encode('utf-8')
    sys.stdout.buffer.write(f'Content-Length: {len(raw)}\r\n\r\n'.encode('ascii') + raw)
    sys.stdout.buffer.flush()

while True:
    msg = read_msg()
    if msg is None:
        break
    m = msg.get('method')
    mid = msg.get('id')
    if m == 'initialize':
        send_msg({'jsonrpc': '2.0', 'id': mid, 'result': {'capabilities': {'hoverProvider': True, 'definitionProvider': True, 'documentSymbolProvider': True}}})
    elif m == 'textDocument/didOpen':
        uri = msg['params']['textDocument']['uri']
        send_msg({'jsonrpc': '2.0', 'method': 'textDocument/publishDiagnostics', 'params': {'uri': uri, 'diagnostics': [{'range': {'start': {'line': 0, 'character': 3}, 'end': {'line': 0, 'character': 8}}, 'severity': 2, 'message': 'unused function hello'}]}})
    elif m == 'textDocument/didChange':
        uri = msg['params']['textDocument']['uri']
        txt = msg['params']['contentChanges'][0]['text']
        if 'BROKEN_TYPE' in txt:
            send_msg({'jsonrpc': '2.0', 'method': 'textDocument/publishDiagnostics', 'params': {'uri': uri, 'diagnostics': [{'range': {'start': {'line': 0, 'character': 6}, 'end': {'line': 0, 'character': 17}}, 'severity': 1, 'message': 'mismatched type BROKEN_TYPE'}]}})
        else:
            send_msg({'jsonrpc': '2.0', 'method': 'textDocument/publishDiagnostics', 'params': {'uri': uri, 'diagnostics': []}})
    elif m == 'textDocument/hover':
        send_msg({'jsonrpc': '2.0', 'id': mid, 'result': {'contents': {'kind': 'markdown', 'value': '```rust\nfn hello() -> u32\n```'}}})
    elif m in ('textDocument/definition', 'textDocument/typeDefinition', 'textDocument/implementation'):
        uri = msg['params']['textDocument']['uri']
        send_msg({'jsonrpc': '2.0', 'id': mid, 'result': [{'uri': uri, 'range': {'start': {'line': 0, 'character': 3}, 'end': {'line': 0, 'character': 8}}}]})
    elif m == 'textDocument/documentSymbol':
        send_msg({'jsonrpc': '2.0', 'id': mid, 'result': [{'name': 'hello', 'kind': 12, 'range': {'start': {'line': 0, 'character': 0}, 'end': {'line': 0, 'character': 24}}, 'selectionRange': {'start': {'line': 0, 'character': 3}, 'end': {'line': 0, 'character': 8}}}]})
    elif m == 'shutdown':
        send_msg({'jsonrpc': '2.0', 'id': mid, 'result': None})
    elif m == 'exit':
        break
"#;
        let script = dir.join("mock_lsp.py");
        std::fs::write(&script, mock_py).expect("script");

        let cfg = LspConfig {
            enabled: true,
            servers: vec![LspServerOverrideConfig {
                id: "mock-rust".into(),
                command: "python3".into(),
                args: vec!["-u".into(), script.display().to_string()],
                extensions: vec!["rs".into()],
                root_markers: vec![],
                enabled: true,
            }],
            ..LspConfig::default()
        };
        let pool = LspPoolManager::new(Arc::new(RwLock::new(cfg)));
        let tool = LspTool::new(pool.clone());
        let ctx = ToolContext {
            session_id: "s1".into(),
            workspace_dir: dir.clone(),
            data_dir: dir.clone(),
            hub_dir: dir.clone(),
            shell_timeout_secs: 10,
            auto_background_after_secs: 0,
            max_output_bytes: 64 * 1024,
            headless_no_wait: true,
            demote_bg: Arc::new(Notify::new()),
        };

        let diag = tool
            .execute(
                &ctx,
                json!({"action": "diagnostics", "path": "main.rs"}),
                CancellationToken::new(),
            )
            .await
            .expect("diag");
        assert!(diag.content.contains("unused function hello"), "diag={}", diag.content);

        let hov = tool
            .execute(
                &ctx,
                json!({"action": "hover", "path": "main.rs", "line": 1, "column": 4}),
                CancellationToken::new(),
            )
            .await
            .expect("hover");
        assert!(hov.content.contains("fn hello() -> u32"), "hov={}", hov.content);

        let syms = tool
            .execute(
                &ctx,
                json!({"action": "document_symbols", "path": "main.rs"}),
                CancellationToken::new(),
            )
            .await
            .expect("syms");
        assert!(syms.content.contains("[fn] hello"), "syms={}", syms.content);

        // TypeDefinition & Implementation actions work
        let td = tool
            .execute(
                &ctx,
                json!({"action": "type_definition", "path": "main.rs", "line": 1, "column": 4}),
                CancellationToken::new(),
            )
            .await
            .expect("type_def");
        assert!(td.content.contains("main.rs:1:4"), "td={}", td.content);

        // Stale diagnostic invalidation + Oh My Pi warm Diagnostics-on-Edit + UTF-16 -> UTF-8 column mapping!
        std::fs::write(dir.join("main.rs"), "let 🚀BROKEN_TYPE = 1;\n").expect("mutate");
        let on_edit = pool
            .warm_diagnostics_for_edited_file(&dir, &dir.join("main.rs"), CancellationToken::new())
            .await
            .expect("warm diagnostics on edit");
        assert!(on_edit.contains("mismatched type BROKEN_TYPE"), "on_edit={on_edit}");
        // UTF-16 character 6 on "let 🚀BROKEN_TYPE" -> UTF-8 char index 5 -> 1-based col 6!
        assert!(on_edit.contains("main.rs:1:6 [ERROR]"), "expected UTF-8 col 6 in {on_edit}");

        assert_eq!(pool.list_status().await.len(), 1);
        assert_eq!(pool.stop_all().await, 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
