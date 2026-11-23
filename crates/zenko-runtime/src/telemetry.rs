use crate::log_store::CategorizedLogSink;
use chrono::Utc;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::broadcast;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use zenko_core::{AgentEvent, LogCategory, LogEntry};

#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
    max_entries: usize,
    event_tx: broadcast::Sender<AgentEvent>,
    sink: Arc<RwLock<Option<Arc<CategorizedLogSink>>>>,
}

impl LogBuffer {
    pub fn new(max_entries: usize, event_tx: broadcast::Sender<AgentEvent>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(max_entries.min(512)))),
            max_entries,
            event_tx,
            sink: Arc::new(RwLock::new(None)),
        }
    }

    pub fn attach_sink(&self, sink: Arc<CategorizedLogSink>) {
        if let Ok(mut guard) = self.sink.write() {
            *guard = Some(sink);
        }
    }

    pub fn push(&self, entry: LogEntry) {
        if let Ok(guard) = self.sink.read() {
            if let Some(ref sink) = *guard {
                sink.enqueue(entry.clone());
            }
        }
        if let Ok(mut guard) = self.inner.lock() {
            if guard.len() >= self.max_entries {
                guard.pop_front();
            }
            guard.push_back(entry.clone());
        }
        let _ = self.event_tx.send(AgentEvent::Log(entry));
    }

    pub fn flush(&self) {
        if let Ok(guard) = self.sink.read() {
            if let Some(ref sink) = *guard {
                sink.flush();
            }
        }
    }

    pub fn recent(&self, limit: usize) -> Vec<LogEntry> {
        let Ok(guard) = self.inner.lock() else {
            return Vec::new();
        };
        let skip = guard.len().saturating_sub(limit);
        guard.iter().skip(skip).cloned().collect()
    }
}

pub struct LogBroadcastLayer {
    buffer: LogBuffer,
}

impl LogBroadcastLayer {
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: Vec<String>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => self.message = format!("{value:?}"),
            other => self.fields.push(format!("{other}={value:?}")),
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_string(),
            other => self.fields.push(format!("{other}={value}")),
        }
    }
}

fn classify_tracing_target(target: &str) -> LogCategory {
    match target {
        t if t.starts_with("zenko_gateways") => LogCategory::Gateway,
        t if t.starts_with("zenko_webui") => LogCategory::Webui,
        t if t.starts_with("zenko::setup") || t.starts_with("zenko_cli::setup") => LogCategory::Tui,
        t if t.starts_with("zenko_tools") => LogCategory::Session,
        _ => LogCategory::System,
    }
}

impl<S: Subscriber> Layer<S> for LogBroadcastLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let msg = match (visitor.fields.is_empty(), visitor.message.is_empty()) {
            (true, _) => visitor.message,
            (false, true) => visitor.fields.join(" "),
            (false, false) => format!("{} ({})", visitor.message, visitor.fields.join(", ")),
        };
        let target = meta.target();
        self.buffer.push(LogEntry {
            timestamp: Utc::now(),
            level: meta.level().as_str().to_string(),
            category: classify_tracing_target(target).as_str().to_string(),
            target: target.to_string(),
            session_id: None,
            turn_id: None,
            call_id: None,
            message: msg,
            payload: None,
        });
    }
}

/// Reads current process Resident Set Size (VmRSS) from `/proc/self/status` on Linux (0 cost, zero allocs).
pub fn current_rss_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
            return 0;
        };
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                if let Some(kb_str) = rest.split_whitespace().next() {
                    if let Ok(kb) = kb_str.parse::<u64>() {
                        return kb * 1024;
                    }
                }
            }
        }
        0
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}
