//! Unified Hono-style minimal structured logger shared across HTTP (WebUI),
//! Agent reasoning/streaming, Tool execution, Approvals, Gateways, Cron, and System.
//! Zero scattered ANSI literals, zero regex, zero `if/else` chains.

use crate::{
    LOG_CRON_FILE, LOG_FIELD_DELIMITER, LOG_GATEWAY_FILE, LOG_MAX_ARG_KEYS,
    LOG_PREVIEW_ARG_VAL_CHARS, LOG_PREVIEW_COMPLEX_VAL_CHARS, LOG_SESSION_INDEX_FILE,
    LOG_SESSIONS_SUBDIR, LOG_SYSTEM_FILE, LOG_TUI_FILE, LOG_WEBUI_FILE, LogEntry,
};
use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_DIM: &str = "\x1b[90m";
const ANSI_BOLD: &str = "\x1b[1m";
const ANSI_WHITE: &str = "\x1b[97m";
const ANSI_VERMILION: &str = "\x1b[38;2;234;88;41m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_CYAN: &str = "\x1b[36m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_RED: &str = "\x1b[31m";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogDirection {
    In,
    Work,
    Out,
    Warn,
    Err,
}

impl LogDirection {
    pub const fn glyph(self) -> &'static str {
        match self {
            Self::In => "<--",
            Self::Work => "···",
            Self::Out => "-->",
            Self::Warn => "  !",
            Self::Err => "  ×",
        }
    }

    pub const fn ansi(self) -> &'static str {
        match self {
            Self::In => ANSI_DIM,
            Self::Work => ANSI_VERMILION,
            Self::Out => ANSI_VERMILION,
            Self::Warn => ANSI_YELLOW,
            Self::Err => ANSI_RED,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogCategory {
    Session,
    Gateway,
    Webui,
    Tui,
    Cron,
    System,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct LogCategoryMeta {
    pub id: &'static str,
    pub label: &'static str,
    pub file: &'static str,
    pub description: &'static str,
}

/// Declarative registry of categorized log streams (shared by disk writer, SQLite, CLI & WebUI).
pub const LOG_CATEGORIES: &[LogCategoryMeta] = &[
    LogCategoryMeta {
        id: "session",
        label: "Session",
        file: "sessions/index.jsonl",
        description: "Agent turns, prompts, reasoning, tool executions, and approvals (plus per-session files)",
    },
    LogCategoryMeta {
        id: "gateway",
        label: "Gateway",
        file: LOG_GATEWAY_FILE,
        description: "Telegram bot polling, pairing, and signed webhook traffic",
    },
    LogCategoryMeta {
        id: "webui",
        label: "WebUI",
        file: LOG_WEBUI_FILE,
        description: "Embedded WebUI HTTP requests, API routes, and OpenAI-compatible traffic",
    },
    LogCategoryMeta {
        id: "tui",
        label: "TUI",
        file: LOG_TUI_FILE,
        description: "Setup TUI wizard and interactive terminal REPL sessions",
    },
    LogCategoryMeta {
        id: "cron",
        label: "Cron",
        file: LOG_CRON_FILE,
        description: "24/7 scheduled cron jobs and autonomous heartbeat dispatches",
    },
    LogCategoryMeta {
        id: "system",
        label: "System",
        file: LOG_SYSTEM_FILE,
        description: "Daemon startup, shutdown, vault, IPC, and internal runtime diagnostics",
    },
];

impl LogCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Gateway => "gateway",
            Self::Webui => "webui",
            Self::Tui => "tui",
            Self::Cron => "cron",
            Self::System => "system",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "session" | "sessions" | "agent" | "tool" | "input" | "approve" => Self::Session,
            "gateway" | "gateways" | "telegram" | "webhook" => Self::Gateway,
            "webui" | "web" | "http" | "api" => Self::Webui,
            "tui" | "cli" | "setup" | "repl" => Self::Tui,
            "cron" | "cronjob" | "cronjobs" | "heartbeat" => Self::Cron,
            _ => Self::System,
        }
    }

    pub fn relative_file_parts(self) -> (Option<&'static str>, &'static str) {
        match self {
            Self::Session => (Some(LOG_SESSIONS_SUBDIR), LOG_SESSION_INDEX_FILE),
            Self::Gateway => (None, LOG_GATEWAY_FILE),
            Self::Webui => (None, LOG_WEBUI_FILE),
            Self::Tui => (None, LOG_TUI_FILE),
            Self::Cron => (None, LOG_CRON_FILE),
            Self::System => (None, LOG_SYSTEM_FILE),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogChannel {
    Http,
    Input,
    Agent,
    Tool,
    Approve,
    Gateway,
    Tui,
    Cron,
    System,
}

impl LogChannel {
    pub const fn category(self) -> LogCategory {
        match self {
            Self::Input | Self::Agent | Self::Tool | Self::Approve => LogCategory::Session,
            Self::Gateway => LogCategory::Gateway,
            Self::Http => LogCategory::Webui,
            Self::Tui => LogCategory::Tui,
            Self::Cron => LogCategory::Cron,
            Self::System => LogCategory::System,
        }
    }

    pub const fn tag(self) -> &'static str {
        match self {
            Self::Http => "HTTP",
            Self::Input => "INPUT",
            Self::Agent => "AGENT",
            Self::Tool => "TOOL",
            Self::Approve => "APPROVE",
            Self::Gateway => "GATEWAY",
            Self::Tui => "TUI",
            Self::Cron => "CRON",
            Self::System => "SYSTEM",
        }
    }

    pub const fn target(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Input => "input",
            Self::Agent => "agent",
            Self::Tool => "tool",
            Self::Approve => "approve",
            Self::Gateway => "gateway",
            Self::Tui => "tui",
            Self::Cron => "cron",
            Self::System => "system",
        }
    }

    pub const fn ansi(self) -> &'static str {
        match self {
            Self::Http => ANSI_DIM,
            Self::Input => ANSI_WHITE,
            Self::Agent => ANSI_VERMILION,
            Self::Tool => ANSI_CYAN,
            Self::Approve => ANSI_YELLOW,
            Self::Gateway => ANSI_GREEN,
            Self::Tui => ANSI_VERMILION,
            Self::Cron => ANSI_DIM,
            Self::System => ANSI_DIM,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStatusTone {
    Neutral,
    Ok,
    Info,
    Warn,
    Err,
}

impl LogStatusTone {
    pub const fn ansi(self) -> &'static str {
        match self {
            Self::Neutral => ANSI_DIM,
            Self::Ok => ANSI_GREEN,
            Self::Info => ANSI_CYAN,
            Self::Warn => ANSI_YELLOW,
            Self::Err => ANSI_RED,
        }
    }

    pub const fn level(self) -> &'static str {
        match self {
            Self::Err => "ERROR",
            Self::Warn => "WARN",
            _ => "INFO",
        }
    }

    pub const fn from_http_status(code: u16) -> Self {
        match code / 100 {
            2 => Self::Ok,
            3 => Self::Info,
            4 => Self::Warn,
            5 => Self::Err,
            _ => Self::Neutral,
        }
    }
}

/// Deterministic unit-scaling latency formatter (`91µs`, `2.1ms`, `1.42s`).
pub fn format_latency(d: Duration) -> String {
    let us = d.as_micros();
    match us {
        0..=999 => format!("{us}µs"),
        1_000..=999_999 => format!("{:.1}ms", us as f64 / 1_000.0),
        _ => format!("{:.2}s", d.as_secs_f64()),
    }
}

/// Compact token counter (`840`, `2.4k`, `1.1M`).
pub fn format_tokens(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1_000.0),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

/// Extracts concise model display name (`anthropic/claude-sonnet-4` -> `claude-sonnet-4`).
pub fn short_model_name(model_id: &str) -> &str {
    match model_id.rsplit_once('/') {
        Some((_, short)) if !short.is_empty() => short,
        _ => model_id,
    }
}

/// Deterministic compact single-line preview with whitespace collapsing and ellipsis.
pub fn compact_preview(raw: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(max_chars.min(raw.len()) + 4);
    let mut prev_space = true;
    let mut count = 0usize;

    for ch in raw.chars() {
        let normalized = match ch {
            '\n' | '\r' | '\t' => ' ',
            c => c,
        };
        match (normalized == ' ', prev_space) {
            (true, true) => continue,
            (true, false) => prev_space = true,
            (false, _) => prev_space = false,
        }
        if count >= max_chars {
            out.push('…');
            break;
        }
        out.push(normalized);
        count += 1;
    }

    out.trim_end().to_string()
}

/// Deterministic `(+added/-removed)` unified-diff counter (zero regex).
pub fn diff_line_stats(diff: &str) -> (usize, usize) {
    let mut added = 0usize;
    let mut removed = 0usize;
    for line in diff.lines() {
        match (
            line.starts_with("+++"),
            line.starts_with("---"),
            line.as_bytes().first(),
        ) {
            (false, false, Some(b'+')) => added += 1,
            (false, false, Some(b'-')) => removed += 1,
            _ => {}
        }
    }
    (added, removed)
}

/// Formats JSON tool arguments into concise `key="val"` pairs for minimal logs,
/// honoring `TOOL_ARG_PREVIEW_PRIORITY_KEYS` so `path`/`command`/`action` always lead ahead of `content`.
pub fn compact_tool_args(raw_json_or_preview: &str, max_chars: usize) -> String {
    match serde_json::from_str::<Value>(raw_json_or_preview) {
        Ok(Value::Object(map)) => {
            let mut ordered_keys: Vec<&str> = Vec::with_capacity(map.len());
            for &prio in crate::TOOL_ARG_PREVIEW_PRIORITY_KEYS {
                if map.contains_key(prio) {
                    ordered_keys.push(prio);
                }
            }
            for k in map.keys() {
                if !crate::TOOL_ARG_PREVIEW_PRIORITY_KEYS.contains(&k.as_str()) {
                    ordered_keys.push(k.as_str());
                }
            }
            let parts: Vec<String> = ordered_keys
                .into_iter()
                .take(LOG_MAX_ARG_KEYS)
                .filter_map(|k| {
                    let v = map.get(k)?;
                    let val_str = match v {
                        Value::String(s) => {
                            format!("\"{}\"", compact_preview(s, LOG_PREVIEW_ARG_VAL_CHARS))
                        }
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => compact_preview(&v.to_string(), LOG_PREVIEW_COMPLEX_VAL_CHARS),
                    };
                    Some(format!("{k}={val_str}"))
                })
                .collect();
            compact_preview(&parts.join(" "), max_chars)
        }
        _ => compact_preview(raw_json_or_preview, max_chars),
    }
}

fn sanitize_field(raw: &str) -> String {
    raw.chars()
        .map(|c| match c {
            '\t' | '\n' | '\r' => ' ',
            other => other,
        })
        .collect()
}

/// Pure structured `LogEntry` constructor (`glyph\ttag\tsubject\tstatus\tmeta`) for SSE & WebUI.
pub fn format_hono_entry(
    dir: LogDirection,
    channel: LogChannel,
    subject: &str,
    status: Option<(&str, LogStatusTone)>,
    meta: Option<&str>,
) -> LogEntry {
    let glyph = dir.glyph();
    let tag = channel.tag();
    let tone = status.map(|(_, t)| t).unwrap_or(match dir {
        LogDirection::Err => LogStatusTone::Err,
        LogDirection::Warn => LogStatusTone::Warn,
        _ => LogStatusTone::Neutral,
    });

    let subj_clean = sanitize_field(subject);
    let status_clean = status.map(|(s, _)| sanitize_field(s)).unwrap_or_default();
    let meta_clean = meta.map(sanitize_field).unwrap_or_default();
    let d = LOG_FIELD_DELIMITER;

    LogEntry {
        timestamp: Utc::now(),
        level: tone.level().to_string(),
        category: channel.category().as_str().to_string(),
        target: channel.target().to_string(),
        session_id: None,
        turn_id: None,
        call_id: None,
        message: format!("{glyph}{d}{tag}{d}{subj_clean}{d}{status_clean}{d}{meta_clean}"),
        payload: None,
    }
}

/// Prints one aligned Hono-style ANSI line to stdout without mutating any log buffer.
pub fn print_hono_line(
    dir: LogDirection,
    channel: LogChannel,
    subject: &str,
    status: Option<(&str, LogStatusTone)>,
    meta: Option<&str>,
) {
    let glyph = dir.glyph();
    let tag = channel.tag();

    let status_term = match status {
        Some((s, tone)) if !s.is_empty() => {
            format!(" {ANSI_BOLD}{}{s}{ANSI_RESET}", tone.ansi())
        }
        _ => String::new(),
    };
    let meta_term = match meta {
        Some(m) if !m.is_empty() => format!(" {ANSI_DIM}{m}{ANSI_RESET}"),
        _ => String::new(),
    };

    println!(
        "{}{glyph}{ANSI_RESET} {}{tag:<7}{ANSI_RESET} {subject}{status_term}{meta_term}",
        dir.ansi(),
        channel.ansi(),
    );
}

/// Prints one minimal Hono-style terminal line AND returns its structured `LogEntry`.
pub fn emit_hono_line(
    dir: LogDirection,
    channel: LogChannel,
    subject: &str,
    status: Option<(&str, LogStatusTone)>,
    meta: Option<&str>,
) -> LogEntry {
    print_hono_line(dir, channel, subject, status, meta);
    format_hono_entry(dir, channel, subject, status, meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_latencies_tokens_diffs_args_and_entries() {
        assert_eq!(format_latency(Duration::from_micros(91)), "91µs");
        assert_eq!(format_latency(Duration::from_micros(2_100)), "2.1ms");
        assert_eq!(format_latency(Duration::from_millis(1_420)), "1.42s");
        assert_eq!(format_tokens(840), "840");
        assert_eq!(format_tokens(2_410), "2.4k");
        assert_eq!(format_tokens(1_250_000), "1.2M");
        assert_eq!(short_model_name("anthropic/claude-sonnet-4"), "claude-sonnet-4");
        assert_eq!(short_model_name("gpt-4o"), "gpt-4o");
        assert_eq!(
            diff_line_stats("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new1\n+new2\n"),
            (2, 1)
        );
        assert_eq!(
            compact_tool_args(r#"{"path":"src/main.rs","start_line":10}"#, 80),
            r#"path="src/main.rs" start_line=10"#
        );
        // Priority keys (`path`) MUST lead ahead of alphabetically earlier bulk keys (`content`).
        let write_preview = compact_tool_args(
            r#"{"content":"<!DOCTYPE html><html>very long file body...","path":"index.html"}"#,
            80,
        );
        assert!(
            write_preview.starts_with(r#"path="index.html" content="#),
            "expected path before content, got: {write_preview}"
        );

        let entry = format_hono_entry(
            LogDirection::Out,
            LogChannel::Tool,
            "edit_file\tmain.rs",
            Some(("ok", LogStatusTone::Ok)),
            Some("6.4ms (+2/-1)"),
        );
        assert_eq!(entry.target, "tool");
        assert_eq!(entry.level, "INFO");
        let cols: Vec<&str> = entry.message.split(LOG_FIELD_DELIMITER).collect();
        assert_eq!(cols, vec!["-->", "TOOL", "edit_file main.rs", "ok", "6.4ms (+2/-1)"]);
    }
}
