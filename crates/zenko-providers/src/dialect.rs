//! Declarative wire-dialect dispatch for OpenRouter, MiniMax, and OpenAI-compatible APIs.
//! Pure `match` expressions over `ProviderDialect` — zero `if/else` ladders, zero hardcoded URLs.

use reqwest::RequestBuilder;
use serde_json::{Value, json};
use zenko_core::{
    ChatRequest, EventRole, MINIMAX_TEMP_MAX, MINIMAX_TEMP_MIN, PROVIDER_HTTP_REFERER,
    PROVIDER_X_TITLE, ProviderDialect, find_provider,
};

/// Resolves wire dialect from explicit provider ID, falling back to host inspection for `custom`.
pub fn resolve_dialect(provider_id: &str, base_url: &str) -> ProviderDialect {
    let desc = find_provider(provider_id);
    match desc.dialect {
        ProviderDialect::OpenAiCompat if desc.id == "custom" => {
            let lower = base_url.to_ascii_lowercase();
            match (lower.contains("openrouter.ai"), lower.contains("minimax")) {
                (true, _) => ProviderDialect::OpenRouter,
                (_, true) => ProviderDialect::MiniMax,
                _ => ProviderDialect::OpenAiCompat,
            }
        }
        explicit => explicit,
    }
}

/// Builds the JSON payload for `/chat/completions` according to the active provider dialect.
pub fn build_request_body(dialect: ProviderDialect, req: &ChatRequest) -> Value {
    let mut messages = Vec::with_capacity(req.messages.len() + 1);

    if !req.system_prompt.trim().is_empty() {
        let sys_msg = match (dialect, req.enable_prompt_caching) {
            (ProviderDialect::OpenRouter, true) => json!({
                "role": "system",
                "content": [{
                    "type": "text",
                    "text": req.system_prompt,
                    "cache_control": { "type": "ephemeral" }
                }]
            }),
            _ => json!({
                "role": "system",
                "content": req.system_prompt
            }),
        };
        messages.push(sys_msg);
    }

    for msg in &req.messages {
        match msg.role {
            EventRole::System => {
                messages.push(json!({ "role": "system", "content": msg.content }));
            }
            EventRole::User => {
                messages.push(json!({ "role": "user", "content": msg.content }));
            }
            EventRole::Assistant => {
                let mut obj = serde_json::Map::new();
                obj.insert("role".into(), Value::String("assistant".into()));

                match msg.tool_calls.is_empty() {
                    true => {
                        obj.insert("content".into(), Value::String(msg.content.clone()));
                    }
                    false => {
                        let calls: Vec<Value> = msg
                            .tool_calls
                            .iter()
                            .map(|tc| {
                                json!({
                                    "id": tc.id,
                                    "type": "function",
                                    "function": {
                                        "name": tc.name,
                                        "arguments": tc.arguments.to_string()
                                    }
                                })
                            })
                            .collect();
                        let content_val = match msg.content.is_empty() {
                            true => Value::Null,
                            false => Value::String(msg.content.clone()),
                        };
                        obj.insert("content".into(), content_val);
                        obj.insert("tool_calls".into(), Value::Array(calls));
                    }
                }

                // Preserve interleaved reasoning state across multi-turn tool calls
                // (required by MiniMax M3/M2.7/M2.5 and DeepSeek reasoner).
                if let Some(ref rc) = msg.reasoning_content {
                    if !rc.trim().is_empty() {
                        obj.insert("reasoning_content".into(), Value::String(rc.clone()));
                    }
                }

                messages.push(Value::Object(obj));
            }
            EventRole::Tool => {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": msg.tool_call_id.as_deref().unwrap_or(""),
                    "content": msg.content
                }));
            }
        }
    }

    let temperature = match dialect {
        ProviderDialect::MiniMax => req.temperature.clamp(MINIMAX_TEMP_MIN, MINIMAX_TEMP_MAX),
        _ => req.temperature,
    };

    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": req.enable_streaming,
        "temperature": temperature,
        "max_tokens": req.max_tokens,
    });

    if let Some(obj) = body.as_object_mut() {
        if req.enable_streaming {
            obj.insert("stream_options".into(), json!({ "include_usage": true }));
        }

        if !req.tools.is_empty() {
            let last_tool_idx = req.tools.len().saturating_sub(1);
            let attach_tool_cache =
                dialect == ProviderDialect::OpenRouter && req.enable_prompt_caching;
            let tools: Vec<Value> = req
                .tools
                .iter()
                .enumerate()
                .map(|(idx, t)| {
                    let mut tool_val = json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters
                        }
                    });
                    if attach_tool_cache && idx == last_tool_idx {
                        if let Some(m) = tool_val.as_object_mut() {
                            m.insert(
                                "cache_control".into(),
                                json!({ "type": "ephemeral" }),
                            );
                        }
                    }
                    tool_val
                })
                .collect();
            obj.insert("tools".into(), Value::Array(tools));
            obj.insert("tool_choice".into(), Value::String("auto".into()));
        }

        // OpenRouter/Anthropic multi-breakpoint KV caching: place a 3rd cache breakpoint on the
        // penultimate stable user/tool message so long multi-turn histories reuse cached prefixes.
        if dialect == ProviderDialect::OpenRouter && req.enable_prompt_caching {
            if let Some(arr) = obj.get_mut("messages").and_then(Value::as_array_mut) {
                if arr.len() >= 4 {
                    let target_idx = arr
                        .iter()
                        .enumerate()
                        .take(arr.len() - 1)
                        .skip(1)
                        .rev()
                        .find(|(_, m)| {
                            matches!(
                                m.get("role").and_then(Value::as_str),
                                Some("user" | "tool")
                            )
                        })
                        .map(|(i, _)| i);
                    if let Some(idx) = target_idx {
                        if let Some(mobj) = arr[idx].as_object_mut() {
                            if let Some(text) =
                                mobj.get("content").and_then(Value::as_str).map(String::from)
                            {
                                mobj.insert(
                                    "content".into(),
                                    json!([{
                                        "type": "text",
                                        "text": text,
                                        "cache_control": { "type": "ephemeral" }
                                    }]),
                                );
                            }
                        }
                    }
                }
            }
        }

        match dialect {
            ProviderDialect::OpenRouter => {
                obj.insert("usage".into(), json!({ "include": true }));
                if req.sticky_session_routing && !req.session_id.is_empty() {
                    obj.insert("session_id".into(), Value::String(req.session_id.clone()));
                }
                if !req.fallbacks.is_empty() {
                    let mut models = Vec::with_capacity(req.fallbacks.len() + 1);
                    models.push(Value::String(req.model.clone()));
                    for fb in &req.fallbacks {
                        if fb != &req.model {
                            models.push(Value::String(fb.clone()));
                        }
                    }
                    obj.insert("models".into(), Value::Array(models));
                }
                if !req.provider_sort.is_empty() {
                    obj.insert(
                        "provider".into(),
                        json!({
                            "sort": req.provider_sort,
                            "allow_fallbacks": true
                        }),
                    );
                }
                if let Some(effort) = req.reasoning_effort {
                    obj.insert("reasoning".into(), json!({ "effort": effort.as_str() }));
                }
            }
            ProviderDialect::MiniMax => {
                // Separate thinking into dedicated reasoning_content / reasoning_details fields.
                obj.insert("reasoning_split".into(), Value::Bool(true));
            }
            ProviderDialect::OpenAiCompat => {}
        }
    }

    body
}

/// Applies provider-specific HTTP headers onto a request builder.
pub fn apply_chat_headers(
    dialect: ProviderDialect,
    req_builder: RequestBuilder,
    chat_req: &ChatRequest,
) -> RequestBuilder {
    match dialect {
        ProviderDialect::OpenRouter => {
            let mut b = req_builder
                .header("HTTP-Referer", PROVIDER_HTTP_REFERER)
                .header("X-Title", PROVIDER_X_TITLE);
            if chat_req.sticky_session_routing && !chat_req.session_id.is_empty() {
                b = b.header("x-session-id", &chat_req.session_id);
            }
            b
        }
        ProviderDialect::MiniMax | ProviderDialect::OpenAiCompat => req_builder,
    }
}

/// Primary endpoint used to verify API key connectivity for a given dialect.
pub fn verify_endpoint(dialect: ProviderDialect, base_url: &str) -> String {
    match dialect {
        ProviderDialect::OpenRouter => format!("{base_url}/key"),
        ProviderDialect::MiniMax | ProviderDialect::OpenAiCompat => format!("{base_url}/models"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenko_core::{ChatMessage, ToolCall};

    fn sample_request(temp: f32) -> ChatRequest {
        ChatRequest {
            session_id: "sess_1".into(),
            model: "MiniMax-M3.1-Flash".into(),
            fallbacks: vec!["MiniMax-M2.7".into()],
            system_prompt: "You are Zenko.".into(),
            messages: vec![
                ChatMessage {
                    role: EventRole::User,
                    content: "check files".into(),
                    reasoning_content: None,
                    tool_calls: vec![],
                    tool_call_id: None,
                    name: None,
                },
                ChatMessage {
                    role: EventRole::Assistant,
                    content: String::new(),
                    reasoning_content: Some("Need to read README first".into()),
                    tool_calls: vec![ToolCall {
                        id: "call_1".into(),
                        name: "read_file".into(),
                        arguments: json!({"path": "README.md"}),
                    }],
                    tool_call_id: None,
                    name: None,
                },
            ],
            tools: vec![],
            temperature: temp,
            max_tokens: 1024,
            enable_streaming: true,
            enable_prompt_caching: true,
            sticky_session_routing: true,
            provider_sort: "price".into(),
            reasoning_effort: Some(zenko_core::ReasoningEffort::High),
        }
    }

    #[test]
    fn minimax_dialect_sets_reasoning_split_clamps_temp_and_keeps_thinking() {
        let body = build_request_body(ProviderDialect::MiniMax, &sample_request(0.0));
        assert_eq!(body["reasoning_split"], Value::Bool(true));
        let temp = body["temperature"].as_f64().expect("temp");
        assert!((temp - MINIMAX_TEMP_MIN as f64).abs() < 1e-4);
        assert!(body.get("provider").is_none());
        assert!(body.get("models").is_none());
        assert_eq!(
            body["messages"][2]["reasoning_content"],
            Value::String("Need to read README first".into())
        );

        let high = build_request_body(ProviderDialect::MiniMax, &sample_request(1.9));
        let high_temp = high["temperature"].as_f64().expect("high temp");
        assert!((high_temp - MINIMAX_TEMP_MAX as f64).abs() < 1e-4);
    }

    #[test]
    fn openrouter_dialect_adds_cache_control_fallbacks_and_routing() {
        let mut req = sample_request(0.2);
        req.tools.push(zenko_core::ToolSpec {
            name: "read_file".into(),
            description: "Read file".into(),
            parameters: json!({"type": "object"}),
            risk: zenko_core::RiskLevel::Safe,
        });
        req.messages.push(ChatMessage {
            role: EventRole::Tool,
            content: "README contents".into(),
            reasoning_content: None,
            tool_calls: vec![],
            tool_call_id: Some("call_1".into()),
            name: Some("read_file".into()),
        });
        req.messages.push(ChatMessage {
            role: EventRole::User,
            content: "next step".into(),
            reasoning_content: None,
            tool_calls: vec![],
            tool_call_id: None,
            name: None,
        });
        let body = build_request_body(ProviderDialect::OpenRouter, &req);
        assert!(body["messages"][0]["content"].is_array());
        assert_eq!(body["tools"][0]["cache_control"]["type"], "ephemeral");
        assert!(body["messages"][3]["content"].is_array());
        assert_eq!(
            body["messages"][3]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        assert_eq!(body["session_id"], Value::String("sess_1".into()));
        assert_eq!(body["models"][1], Value::String("MiniMax-M2.7".into()));
        assert_eq!(body["provider"]["sort"], Value::String("price".into()));
        assert_eq!(body["reasoning"]["effort"], Value::String("high".into()));
        assert!(body.get("reasoning_split").is_none());
    }

    #[test]
    fn resolves_custom_minimax_host_dialect() {
        assert_eq!(
            resolve_dialect("minimax", "https://api.minimax.io/v1"),
            ProviderDialect::MiniMax
        );
        assert_eq!(
            resolve_dialect("custom", "https://api.minimaxi.com/v1"),
            ProviderDialect::MiniMax
        );
        assert_eq!(
            resolve_dialect("openrouter", "https://openrouter.ai/api/v1"),
            ProviderDialect::OpenRouter
        );
    }
}
