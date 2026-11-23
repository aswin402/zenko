//! Streaming SSE & non-streaming completion parser supporting OpenRouter, MiniMax
//! (`reasoning_content`, `reasoning_details`, `<think>` tags, `base_resp` errors),
//! and standard OpenAI-compatible responses — zero regex.

use serde_json::{Value, json};
use std::collections::BTreeMap;
use tokio::sync::mpsc;
use zenko_core::{LlmDelta, Result, ToolCall, UsageStats, ZenkoError};

const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";
const MAX_SSE_LINE_BYTES: usize = 1024 * 1024;

/// Byte-safe SSE line framer that buffers raw TCP bytes until `\n` before
/// decoding UTF-8, preventing multi-byte characters (emojis, CJK, box-drawing)
/// split across arbitrary network chunks from turning into `\u{FFFD}` (``).
#[derive(Default)]
pub struct SseByteLineDecoder {
    raw_buf: Vec<u8>,
}

impl SseByteLineDecoder {
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            raw_buf: Vec::with_capacity(cap),
        }
    }

    /// Appends raw network bytes and yields every complete `\n`-terminated line
    /// with optional trailing `\r` stripped. Caps unterminated lines at `MAX_SSE_LINE_BYTES`.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>> {
        if self.raw_buf.len().saturating_add(bytes.len()) > MAX_SSE_LINE_BYTES
            && !bytes.contains(&b'\n')
        {
            return Err(ZenkoError::Provider(format!(
                "SSE line exceeded maximum allowed size ({MAX_SSE_LINE_BYTES} bytes)"
            )));
        }
        self.raw_buf.extend_from_slice(bytes);
        let mut lines = Vec::new();
        let mut start = 0usize;

        while let Some(rel) = self.raw_buf[start..].iter().position(|&b| b == b'\n') {
            let end = start + rel;
            if end - start > MAX_SSE_LINE_BYTES {
                return Err(ZenkoError::Provider(format!(
                    "SSE line exceeded maximum allowed size ({MAX_SSE_LINE_BYTES} bytes)"
                )));
            }
            let trimmed_end = match end > start && self.raw_buf[end - 1] == b'\r' {
                true => end - 1,
                false => end,
            };
            lines.push(String::from_utf8_lossy(&self.raw_buf[start..trimmed_end]).into_owned());
            start = end + 1;
        }

        if start > 0 {
            self.raw_buf.drain(..start);
        }
        if self.raw_buf.len() > MAX_SSE_LINE_BYTES {
            return Err(ZenkoError::Provider(format!(
                "SSE line exceeded maximum allowed size ({MAX_SSE_LINE_BYTES} bytes)"
            )));
        }
        Ok(lines)
    }

    /// Flushes any trailing unterminated bytes at EOF.
    pub fn finish(&mut self) -> Option<String> {
        if self.raw_buf.is_empty() {
            return None;
        }
        let tail = std::mem::take(&mut self.raw_buf);
        let trimmed = String::from_utf8_lossy(&tail).trim().to_string();
        match trimmed.is_empty() {
            true => None,
            false => Some(trimmed),
        }
    }
}

/// Deterministic zero-regex streaming splitter for inline `<think>...</think>` blocks.
/// Safely handles tag boundaries split across arbitrary SSE chunk sizes.
#[derive(Default)]
pub struct ThinkTagSplitter {
    in_think: bool,
    pending: String,
}

impl ThinkTagSplitter {
    pub fn push(&mut self, chunk: &str) -> Vec<(bool, String)> {
        self.pending.push_str(chunk);
        let mut out = Vec::new();

        loop {
            let target = match self.in_think {
                false => THINK_OPEN,
                true => THINK_CLOSE,
            };

            if let Some(pos) = self.pending.find(target) {
                if pos > 0 {
                    out.push((self.in_think, self.pending[..pos].to_string()));
                }
                self.pending.drain(..pos + target.len());
                self.in_think = !self.in_think;
                continue;
            }

            let keep = trailing_partial_tag_len(&self.pending, target);
            let emit_len = self.pending.len().saturating_sub(keep);
            if emit_len > 0 {
                out.push((self.in_think, self.pending[..emit_len].to_string()));
                self.pending.drain(..emit_len);
            }
            break;
        }

        out
    }

    pub fn flush(&mut self) -> Option<(bool, String)> {
        match self.pending.is_empty() {
            true => None,
            false => Some((self.in_think, std::mem::take(&mut self.pending))),
        }
    }
}

fn trailing_partial_tag_len(haystack: &str, tag: &str) -> usize {
    let hb = haystack.as_bytes();
    let tb = tag.as_bytes();
    let max_check = hb.len().min(tb.len().saturating_sub(1));
    let mut best = 0usize;
    for len in 1..=max_check {
        if hb[hb.len() - len..] == tb[..len] {
            best = len;
        }
    }
    best
}

#[derive(Default)]
struct PendingToolCall {
    id: String,
    name: String,
    args_buf: String,
}

#[derive(Default)]
pub struct SseAccumulator {
    tool_calls: BTreeMap<usize, PendingToolCall>,
    pub finish_reason: Option<String>,
    flushed_tools: bool,
    think_splitter: ThinkTagSplitter,
}

impl SseAccumulator {
    pub async fn flush_all(&mut self, tx: &mpsc::Sender<Result<LlmDelta>>) {
        if let Some((is_thinking, tail)) = self.think_splitter.flush() {
            emit_text_segment(is_thinking, tail, tx).await;
        }
        self.flush_tool_calls(tx).await;
    }

    pub async fn flush_tool_calls(&mut self, tx: &mpsc::Sender<Result<LlmDelta>>) {
        if self.flushed_tools || self.tool_calls.is_empty() {
            return;
        }
        self.flushed_tools = true;
        let calls = std::mem::take(&mut self.tool_calls);
        for (idx, pending) in calls {
            if pending.name.trim().is_empty() {
                continue;
            }
            let parsed_args = match pending.args_buf.trim().is_empty() {
                true => json!({}),
                false => serde_json::from_str(&pending.args_buf)
                    .unwrap_or_else(|_| json!({ "_raw": pending.args_buf })),
            };
            let id = match pending.id.is_empty() {
                true => format!("call_{idx}"),
                false => pending.id,
            };
            let _ = tx
                .send(Ok(LlmDelta::ToolCallComplete(ToolCall {
                    id,
                    name: pending.name,
                    arguments: parsed_args,
                })))
                .await;
        }
    }
}

/// Checks both OpenAI/OpenRouter `error` and MiniMax `base_resp` failure envelopes.
pub fn check_provider_error(val: &Value) -> Result<()> {
    if let Some(err_obj) = val.get("error") {
        let msg = err_obj
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| err_obj.as_str())
            .unwrap_or("provider stream error");
        return Err(ZenkoError::Provider(msg.to_string()));
    }

    if let Some(base_resp) = val.get("base_resp").and_then(Value::as_object) {
        let code = base_resp
            .get("status_code")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if code != 0 {
            let msg = base_resp
                .get("status_msg")
                .and_then(Value::as_str)
                .unwrap_or("MiniMax API error");
            return Err(ZenkoError::Provider(format!("MiniMax error ({code}): {msg}")));
        }
    }

    Ok(())
}

async fn emit_text_segment(
    is_thinking: bool,
    segment: String,
    tx: &mpsc::Sender<Result<LlmDelta>>,
) {
    if segment.is_empty() {
        return;
    }
    let delta = match is_thinking {
        true => LlmDelta::Thinking(segment),
        false => LlmDelta::Text(segment),
    };
    let _ = tx.send(Ok(delta)).await;
}

fn extract_reasoning_text(container: &Value) -> Option<String> {
    if let Some(direct) = container
        .get("reasoning")
        .and_then(Value::as_str)
        .or_else(|| container.get("reasoning_content").and_then(Value::as_str))
    {
        if !direct.is_empty() {
            return Some(direct.to_string());
        }
    }

    if let Some(details) = container.get("reasoning_details").and_then(Value::as_array) {
        let joined: String = details
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect();
        if !joined.is_empty() {
            return Some(joined);
        }
    }

    None
}

fn extract_usage(val: &Value) -> Option<UsageStats> {
    let usage = val.get("usage").and_then(Value::as_object)?;
    let prompt_tokens = usage.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0);
    let completion_tokens = usage
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_read_tokens = usage
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(Value::as_u64)
        .or_else(|| usage.get("prompt_cache_hit_tokens").and_then(Value::as_u64))
        .or_else(|| usage.get("cache_read_input_tokens").and_then(Value::as_u64))
        .unwrap_or(0);
    let cost_usd = usage.get("cost").and_then(Value::as_f64).unwrap_or(0.0);
    Some(UsageStats {
        prompt_tokens,
        completion_tokens,
        cache_read_tokens,
        cost_usd,
    })
}

/// Returns `Ok(true)` when `[DONE]` is received so callers can terminate keep-alive streams promptly.
pub async fn process_sse_line(
    line: &str,
    acc: &mut SseAccumulator,
    tx: &mpsc::Sender<Result<LlmDelta>>,
) -> Result<bool> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with(':') {
        return Ok(false);
    }
    let Some(data) = trimmed.strip_prefix("data:") else {
        return Ok(false);
    };
    let payload = data.trim();
    if payload == "[DONE]" {
        acc.flush_all(tx).await;
        return Ok(true);
    }

    let val: Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(_) => return Ok(false),
    };

    check_provider_error(&val)?;

    if let Some(usage) = extract_usage(&val) {
        let _ = tx.send(Ok(LlmDelta::Usage(usage))).await;
    }

    if let Some(choice) = val
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
    {
        if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
            acc.finish_reason = Some(fr.to_string());
        }

        if let Some(delta) = choice.get("delta") {
            if let Some(reasoning) = extract_reasoning_text(delta) {
                let _ = tx.send(Ok(LlmDelta::Thinking(reasoning))).await;
            }

            if let Some(text) = delta.get("content").and_then(Value::as_str) {
                if !text.is_empty() {
                    for (is_thinking, seg) in acc.think_splitter.push(text) {
                        emit_text_segment(is_thinking, seg, tx).await;
                    }
                }
            }

            if let Some(tcs) = delta.get("tool_calls").and_then(Value::as_array) {
                for tc in tcs {
                    let incoming_id = tc.get("id").and_then(Value::as_str).unwrap_or("");
                    let idx = match tc.get("index").and_then(Value::as_u64) {
                        Some(n) => n as usize,
                        None => {
                            // Some providers (e.g. Ollama) omit `index` on parallel tool calls;
                            // match existing call by ID or allocate a fresh slot instead of colliding on 0.
                            if !incoming_id.is_empty() {
                                acc.tool_calls
                                    .iter()
                                    .find(|(_, p)| p.id == incoming_id)
                                    .map(|(&k, _)| k)
                                    .unwrap_or_else(|| {
                                        acc.tool_calls
                                            .keys()
                                            .next_back()
                                            .map(|k| k + 1)
                                            .unwrap_or(0)
                                    })
                            } else {
                                acc.tool_calls.keys().next_back().copied().unwrap_or(0)
                            }
                        }
                    };
                    let entry = acc.tool_calls.entry(idx).or_default();
                    if !incoming_id.is_empty() {
                        entry.id = incoming_id.to_string();
                    }
                    if let Some(func) = tc.get("function") {
                        if let Some(name) = func.get("name").and_then(Value::as_str) {
                            if !name.is_empty() {
                                entry.name.push_str(name);
                            }
                        }
                        if let Some(args) = func.get("arguments").and_then(Value::as_str) {
                            entry.args_buf.push_str(args);
                        }
                    }
                }
            }
        }
    }

    Ok(false)
}

/// Parses a non-streaming JSON chat completion response into the same `LlmDelta` stream.
pub async fn emit_non_streaming_response(
    val: &Value,
    tx: &mpsc::Sender<Result<LlmDelta>>,
) -> Result<()> {
    check_provider_error(val)?;

    let mut finish_reason = None;
    if let Some(choice) = val
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
    {
        finish_reason = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(String::from);

        if let Some(msg) = choice.get("message") {
            if let Some(reasoning) = extract_reasoning_text(msg) {
                let _ = tx.send(Ok(LlmDelta::Thinking(reasoning))).await;
            }

            if let Some(text) = msg.get("content").and_then(Value::as_str) {
                let mut splitter = ThinkTagSplitter::default();
                for (is_thinking, seg) in splitter.push(text) {
                    emit_text_segment(is_thinking, seg, tx).await;
                }
                if let Some((is_thinking, tail)) = splitter.flush() {
                    emit_text_segment(is_thinking, tail, tx).await;
                }
            }

            if let Some(tcs) = msg.get("tool_calls").and_then(Value::as_array) {
                for (idx, tc) in tcs.iter().enumerate() {
                    let id = tc
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .unwrap_or_else(|| format!("call_{idx}"));
                    let name = tc
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let raw_args = tc
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(Value::as_str)
                        .unwrap_or("{}");
                    let parsed_args = serde_json::from_str(raw_args)
                        .unwrap_or_else(|_| json!({ "_raw": raw_args }));
                    if !name.is_empty() {
                        let _ = tx
                            .send(Ok(LlmDelta::ToolCallComplete(ToolCall {
                                id,
                                name,
                                arguments: parsed_args,
                            })))
                            .await;
                    }
                }
            }
        }
    }

    if let Some(usage) = extract_usage(val) {
        let _ = tx.send(Ok(LlmDelta::Usage(usage))).await;
    }

    let _ = tx.send(Ok(LlmDelta::Done { finish_reason })).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sse_parser_handles_thinking_text_tools_and_usage() {
        let (tx, mut rx) = mpsc::channel(32);
        let mut acc = SseAccumulator::default();

        let lines = [
            r#"data: {"choices":[{"delta":{"reasoning":"Plan step 1"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"Hello "}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"README.md\"}"}}]},"finish_reason":"tool_calls"}]}"#,
            r#"data: {"usage":{"prompt_tokens":100,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":80},"cost":0.0012}}"#,
            r#"data: [DONE]"#,
        ];

        for line in lines {
            process_sse_line(line, &mut acc, &tx).await.expect("parse");
        }
        drop(tx);

        let mut events = Vec::new();
        while let Some(ev) = rx.recv().await {
            events.push(ev.expect("ok"));
        }

        assert!(matches!(events[0], LlmDelta::Thinking(ref s) if s == "Plan step 1"));
        assert!(matches!(events[1], LlmDelta::Text(ref s) if s == "Hello "));
        assert!(matches!(events[2], LlmDelta::Usage(ref u) if u.cache_read_tokens == 80 && u.cost_usd > 0.0));
        assert!(matches!(events[3], LlmDelta::ToolCallComplete(ref tc) if tc.name == "read_file" && tc.arguments["path"] == "README.md"));
    }

    #[tokio::test]
    async fn sse_parser_handles_minimax_reasoning_details_split_think_tags_and_base_resp() {
        let (tx, mut rx) = mpsc::channel(32);
        let mut acc = SseAccumulator::default();

        let lines = [
            r#"data: {"choices":[{"delta":{"reasoning_details":[{"type":"reasoning.text","text":"Checking repo"}]}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"<thi"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"nk>deep thought</th"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"ink>Final answer"}}]}"#,
            r#"data: [DONE]"#,
        ];

        for line in lines {
            process_sse_line(line, &mut acc, &tx).await.expect("parse");
        }
        drop(tx);

        let mut thinking = String::new();
        let mut text = String::new();
        while let Some(Ok(ev)) = rx.recv().await {
            match ev {
                LlmDelta::Thinking(t) => thinking.push_str(&t),
                LlmDelta::Text(t) => text.push_str(&t),
                _ => {}
            }
        }
        assert_eq!(thinking, "Checking repodeep thought");
        assert_eq!(text, "Final answer");

        // MiniMax base_resp error envelope
        let err_val = json!({"base_resp": {"status_code": 1004, "status_msg": "invalid api key"}});
        let err = check_provider_error(&err_val).expect_err("must flag MiniMax base_resp error");
        assert!(err.to_string().contains("1004"));
    }

    #[tokio::test]
    async fn non_streaming_parser_extracts_reasoning_text_and_tools() {
        let (tx, mut rx) = mpsc::channel(16);
        let payload = json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "reasoning_content": "Internal plan",
                    "content": "Done!",
                    "tool_calls": [{
                        "id": "call_mm_1",
                        "type": "function",
                        "function": { "name": "list_dir", "arguments": "{\"path\":\".\"}" }
                    }]
                }
            }],
            "usage": { "prompt_tokens": 50, "completion_tokens": 12 }
        });

        emit_non_streaming_response(&payload, &tx).await.expect("ok");
        drop(tx);

        let mut events = Vec::new();
        while let Some(Ok(ev)) = rx.recv().await {
            events.push(ev);
        }
        assert!(matches!(events[0], LlmDelta::Thinking(ref s) if s == "Internal plan"));
        assert!(matches!(events[1], LlmDelta::Text(ref s) if s == "Done!"));
        assert!(matches!(events[2], LlmDelta::ToolCallComplete(ref tc) if tc.name == "list_dir"));
        assert!(matches!(events[3], LlmDelta::Usage(ref u) if u.prompt_tokens == 50));
    }

    #[tokio::test]
    async fn sse_byte_decoder_preserves_multibyte_emojis_split_across_chunks() {
        let raw = "data: {\"choices\":[{\"delta\":{\"content\":\"Hey! 👋🦊✨\"}}]}\r\ndata: [DONE]\n";
        let bytes = raw.as_bytes();
        let mut decoder = SseByteLineDecoder::with_capacity(64);
        let (tx, mut rx) = mpsc::channel(16);
        let mut acc = SseAccumulator::default();

        // Feed 1 byte at a time so every 4-byte UTF-8 emoji is split across chunks.
        for chunk in bytes.chunks(1) {
            for line in decoder.push(chunk).expect("push") {
                process_sse_line(&line, &mut acc, &tx).await.expect("parse");
            }
        }
        if let Some(tail) = decoder.finish() {
            process_sse_line(&tail, &mut acc, &tx).await.expect("tail");
        }
        acc.flush_all(&tx).await;
        drop(tx);

        let mut text = String::new();
        while let Some(Ok(ev)) = rx.recv().await {
            if let LlmDelta::Text(s) = ev {
                text.push_str(&s);
            }
        }
        assert_eq!(text, "Hey! 👋🦊✨");
        assert!(!text.contains('\u{FFFD}'));
    }

    #[tokio::test]
    async fn sse_done_terminates_extracts_cache_tokens_and_separates_unindexed_tools() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut acc = SseAccumulator::default();

        // Parallel tool calls omitting `index` must allocate separate slots by ID
        let line1 = r#"data: {"choices":[{"delta":{"tool_calls":[{"id":"call_a","function":{"name":"read_file","arguments":"{\"path\":\"a.rs\"}"}},{"id":"call_b","function":{"name":"read_file","arguments":"{\"path\":\"b.rs\"}"}}]}}]}"#;
        assert!(!process_sse_line(line1, &mut acc, &tx).await.expect("line1"));

        // DeepSeek prompt_cache_hit_tokens extraction
        let line2 = r#"data: {"usage":{"prompt_tokens":200,"completion_tokens":40,"prompt_cache_hit_tokens":150}}"#;
        assert!(!process_sse_line(line2, &mut acc, &tx).await.expect("line2"));

        // [DONE] signals immediate stream termination
        assert!(process_sse_line("data: [DONE]", &mut acc, &tx).await.expect("done"));
        acc.flush_all(&tx).await;
        drop(tx);

        let mut tool_ids = Vec::new();
        let mut cache_hits = 0u64;
        while let Some(Ok(ev)) = rx.recv().await {
            match ev {
                LlmDelta::ToolCallComplete(tc) => tool_ids.push(tc.id),
                LlmDelta::Usage(u) => cache_hits = u.cache_read_tokens,
                _ => {}
            }
        }
        assert_eq!(tool_ids, vec!["call_a", "call_b"]);
        assert_eq!(cache_hits, 150);

        // Anthropic cache_read_input_tokens extraction
        let anth = json!({"usage": {"prompt_tokens": 100, "completion_tokens": 20, "cache_read_input_tokens": 80}});
        assert_eq!(extract_usage(&anth).expect("usage").cache_read_tokens, 80);
    }
}
