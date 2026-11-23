//! Local Unix Domain Socket JSON-RPC client for talking to a running `zenko web` daemon.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use zenko_core::{
    IPC_METHOD_JOBS_FOLLOW, IPC_METHOD_LOGS_FOLLOW, IPC_METHOD_SESSION_ATTACH, LogEntry,
    LogFilter, ZenkoConfig,
};

#[derive(Debug, Clone)]
pub struct DaemonOfflineError(pub String);

impl std::fmt::Display for DaemonOfflineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DaemonOfflineError {}

/// Returns `true` ONLY when connecting to the local `zenko web` daemon socket failed because
/// no daemon is listening (`NotFound` / `ConnectionRefused`, or non-Unix). Distinguishes an
/// offline daemon from an in-flight RPC timeout/transport error so non-idempotent commands never
/// double-execute after a timeout!
pub fn is_daemon_offline(err: &anyhow::Error) -> bool {
    err.downcast_ref::<DaemonOfflineError>().is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Text,
    Json,
    StreamJson,
}

#[cfg(unix)]
pub async fn ipc_call(cfg: &ZenkoConfig, method: &str, params: Value) -> Result<Value> {
    ipc_call_timeout(cfg, method, params, None).await
}

#[cfg(unix)]
pub async fn ipc_call_timeout(
    cfg: &ZenkoConfig,
    method: &str,
    params: Value,
    timeout: Option<Duration>,
) -> Result<Value> {
    let sock = cfg.socket_path();
    let stream = match tokio::net::UnixStream::connect(&sock).await {
        Ok(s) => s,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Err(anyhow::Error::new(DaemonOfflineError(format!(
                "daemon socket {} offline: {e}",
                sock.display()
            ))));
        }
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!(
                "failed to connect to daemon socket {}",
                sock.display()
            )));
        }
    };
    let (reader, mut writer) = stream.into_split();
    let payload = json!({ "method": method, "params": params });
    let mut line = serde_json::to_string(&payload)?;
    line.push('\n');
    writer.write_all(line.as_bytes()).await?;
    writer.flush().await?;

    let mut lines = BufReader::new(reader).lines();
    let next_fut = lines.next_line();
    let resp_line = match timeout {
        Some(dur) => tokio::time::timeout(dur, next_fut)
            .await
            .context("IPC call timed out")??
            .context("empty response from daemon")?,
        None => next_fut.await?.context("empty response from daemon")?,
    };
    Ok(serde_json::from_str(&resp_line)?)
}

#[cfg(not(unix))]
pub async fn ipc_call(_cfg: &ZenkoConfig, _method: &str, _params: Value) -> Result<Value> {
    Err(anyhow::Error::new(DaemonOfflineError(
        "Unix IPC not available on this OS".into(),
    )))
}

#[cfg(not(unix))]
pub async fn ipc_call_timeout(
    _cfg: &ZenkoConfig,
    _method: &str,
    _params: Value,
    _timeout: Option<Duration>,
) -> Result<Value> {
    Err(anyhow::Error::new(DaemonOfflineError(
        "Unix IPC not available on this OS".into(),
    )))
}

pub fn unwrap_ipc_result(resp: Value) -> Result<Value> {
    match resp.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(resp.get("result").cloned().unwrap_or(Value::Null)),
        _ => {
            let msg = resp
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown daemon error");
            anyhow::bail!("{msg}");
        }
    }
}

#[cfg(unix)]
pub async fn ipc_follow_logs<F>(
    cfg: &ZenkoConfig,
    filter: &LogFilter,
    mut on_entry: F,
) -> Result<()>
where
    F: FnMut(LogEntry),
{
    let sock = cfg.socket_path();
    let stream = tokio::net::UnixStream::connect(&sock)
        .await
        .with_context(|| format!("failed to connect to daemon socket {}", sock.display()))?;
    let (reader, mut writer) = stream.into_split();
    let payload = json!({
        "method": IPC_METHOD_LOGS_FOLLOW,
        "params": filter,
    });
    let mut req_line = serde_json::to_string(&payload)?;
    req_line.push('\n');
    writer.write_all(req_line.as_bytes()).await?;
    writer.flush().await?;

    let mut lines = BufReader::new(reader).lines();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            line = lines.next_line() => match line? {
                Some(raw) => {
                    if let Ok(val) = serde_json::from_str::<Value>(&raw)
                        && let Some(entry_val) = val.get("entry")
                            && let Ok(entry) = serde_json::from_value::<LogEntry>(entry_val.clone()) {
                                on_entry(entry);
                            }
                }
                None => break,
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub async fn ipc_follow_logs<F>(
    _cfg: &ZenkoConfig,
    _filter: &LogFilter,
    _on_entry: F,
) -> Result<()>
where
    F: FnMut(LogEntry),
{
    anyhow::bail!("Unix IPC live log streaming is not available on this OS")
}

#[cfg(unix)]
pub async fn ipc_attach_session<F>(
    cfg: &ZenkoConfig,
    session_id: &str,
    replay: usize,
    mut on_frame: F,
) -> Result<()>
where
    F: FnMut(Value),
{
    let sock = cfg.socket_path();
    let stream = tokio::net::UnixStream::connect(&sock)
        .await
        .with_context(|| format!("failed to connect to daemon socket {}", sock.display()))?;
    let (reader, mut writer) = stream.into_split();
    let payload = json!({
        "method": IPC_METHOD_SESSION_ATTACH,
        "params": {
            "session_id": session_id,
            "replay": replay,
        },
    });
    let mut req_line = serde_json::to_string(&payload)?;
    req_line.push('\n');
    writer.write_all(req_line.as_bytes()).await?;
    writer.flush().await?;

    let mut lines = BufReader::new(reader).lines();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            line = lines.next_line() => match line? {
                Some(raw) => {
                    if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                        if val.get("ok").and_then(Value::as_bool) == Some(false) {
                            let msg = val
                                .get("error")
                                .and_then(Value::as_str)
                                .unwrap_or("attach error");
                            anyhow::bail!("{msg}");
                        }
                        on_frame(val);
                    }
                }
                None => break,
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub async fn ipc_attach_session<F>(
    _cfg: &ZenkoConfig,
    _session_id: &str,
    _replay: usize,
    _on_frame: F,
) -> Result<()>
where
    F: FnMut(Value),
{
    anyhow::bail!("Unix IPC session attach is not available on this OS")
}

#[cfg(unix)]
pub async fn ipc_follow_job<F>(
    cfg: &ZenkoConfig,
    job_id: &str,
    offset: u64,
    mut on_frame: F,
) -> Result<()>
where
    F: FnMut(Value),
{
    let sock = cfg.socket_path();
    let stream = tokio::net::UnixStream::connect(&sock)
        .await
        .with_context(|| format!("failed to connect to daemon socket {}", sock.display()))?;
    let (reader, mut writer) = stream.into_split();
    let payload = json!({
        "method": IPC_METHOD_JOBS_FOLLOW,
        "params": {
            "job_id": job_id,
            "offset": offset,
        },
    });
    let mut req_line = serde_json::to_string(&payload)?;
    req_line.push('\n');
    writer.write_all(req_line.as_bytes()).await?;
    writer.flush().await?;

    let mut lines = BufReader::new(reader).lines();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            line = lines.next_line() => match line? {
                Some(raw) => {
                    if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                        if val.get("ok").and_then(Value::as_bool) == Some(false) {
                            let msg = val
                                .get("error")
                                .and_then(Value::as_str)
                                .unwrap_or("job follow error");
                            anyhow::bail!("{msg}");
                        }
                        on_frame(val);
                    }
                }
                None => break,
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub async fn ipc_follow_job<F>(
    _cfg: &ZenkoConfig,
    _job_id: &str,
    _offset: u64,
    _on_frame: F,
) -> Result<()>
where
    F: FnMut(Value),
{
    anyhow::bail!("Unix IPC live job streaming is not available on this OS")
}
