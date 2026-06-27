// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn anthropic_request_body_uses_messages_api_tool_blocks() {
    let config = ModelConfig {
        provider: "anthropic".into(),
        model: "claude-sonnet-4-5".into(),
        ..ModelConfig::default()
    };
    let body = test_messages_request_body(
        &config,
        ModelRequest {
            messages: vec![
                ModelMessage::system("system token=abc123"),
                ModelMessage::user("hello"),
                ModelMessage::assistant_with_tool_calls(
                    "checking",
                    vec![ModelToolCall {
                        id: Some("toolu-token=abc123".into()),
                        name: "memory_search".into(),
                        input: serde_json::json!({"query": "hello token=abc123"}),
                        raw_arguments: None,
                    }],
                ),
                ModelMessage::tool_result(
                    "toolu-token=abc123",
                    "memory_search",
                    "Tool output token=abc123",
                ),
            ],
            options: ModelRequestOptions {
                max_tokens: Some(64),
                temperature: Some(0.1),
                ..ModelRequestOptions::default()
            },
            tools: vec![ModelToolDefinition {
                name: "memory_search".into(),
                description: "Search local memory".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }],
        },
    );

    assert_eq!(body["model"], "claude-sonnet-4-5");
    assert_eq!(body["max_tokens"], 64);
    assert_eq!(body["tools"][0]["name"], "memory_search");
    assert_eq!(body["messages"][1]["role"], "assistant");
    assert_eq!(body["messages"][1]["content"][1]["type"], "tool_use");
    assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
    assert_eq!(
        body["messages"][2]["content"][0]["tool_use_id"],
        "toolu-token=[REDACTED_SECRET]"
    );
    let raw = serde_json::to_string(&body).expect("json");
    assert!(!raw.contains("abc123"));
    assert!(raw.contains("[REDACTED_SECRET]"));
}

#[test]
fn anthropic_request_body_supports_image_content_blocks() {
    let config = ModelConfig {
        provider: "anthropic".into(),
        model: "claude-sonnet-4-5".into(),
        ..ModelConfig::default()
    };
    let body = test_messages_request_body(
        &config,
        ModelRequest {
            messages: vec![ModelMessage::user_with_content_blocks(vec![
                ModelContentBlock::text("describe"),
                ModelContentBlock::Image {
                    image_url: "data:image/png;base64,aW1n".into(),
                    mime_type: Some("image/png".into()),
                    detail: None,
                },
            ])],
            options: ModelRequestOptions {
                max_tokens: Some(64),
                ..ModelRequestOptions::default()
            },
            tools: Vec::new(),
        },
    );

    assert_eq!(body["messages"][0]["content"][0]["type"], "text");
    assert_eq!(body["messages"][0]["content"][0]["text"], "describe");
    assert_eq!(body["messages"][0]["content"][1]["type"], "image");
    assert_eq!(
        body["messages"][0]["content"][1]["source"],
        serde_json::json!({"type": "base64", "media_type": "image/png", "data": "aW1n"})
    );
}

#[test]
fn anthropic_request_body_marks_stable_system_prefix_for_prompt_cache() {
    let config = ModelConfig {
        provider: "anthropic".into(),
        model: "claude-sonnet-4-6".into(),
        ..ModelConfig::default()
    };
    let body = test_messages_request_body(
        &config,
        ModelRequest {
            messages: vec![
                ModelMessage::system("stable system policy"),
                ModelMessage::system("dynamic context bundle"),
                ModelMessage::user("hello"),
            ],
            options: ModelRequestOptions::default(),
            tools: Vec::new(),
        },
    );

    assert_eq!(
        body["system"][0],
        serde_json::json!({
            "type": "text",
            "text": "stable system policy",
            "cache_control": {"type": "ephemeral"}
        })
    );
    assert_eq!(
        body["system"][1],
        serde_json::json!({
            "type": "text",
            "text": "dynamic context bundle"
        })
    );
    assert_eq!(
        body["messages"][0]["content"][0],
        serde_json::json!({"type": "text", "text": "hello"})
    );
}

#[test]
fn anthropic_request_body_applies_modern_claude_policy() {
    let config = ModelConfig {
        provider: "anthropic".into(),
        model: "claude-opus-4-7".into(),
        ..ModelConfig::default()
    };
    let body = test_messages_request_body(
        &config,
        ModelRequest {
            messages: vec![ModelMessage::user("hello")],
            options: ModelRequestOptions {
                temperature: Some(0.2),
                top_p: Some(0.8),
                reasoning: ReasoningConfig {
                    enabled: Some(true),
                    effort: Some(ReasoningEffort::XHigh),
                },
                ..ModelRequestOptions::default()
            },
            tools: Vec::new(),
        },
    );

    assert_eq!(body["max_tokens"], 128_000);
    assert!(body.get("temperature").is_none());
    assert!(body.get("top_p").is_none());
    assert_eq!(
        body["thinking"],
        serde_json::json!({"type": "adaptive", "display": "summarized"})
    );
    assert_eq!(body["output_config"]["effort"], "xhigh");
}

#[test]
fn anthropic_request_body_uses_manual_thinking_for_legacy_claude() {
    let config = ModelConfig {
        provider: "anthropic".into(),
        model: "claude-3-7-sonnet".into(),
        ..ModelConfig::default()
    };
    let body = test_messages_request_body(
        &config,
        ModelRequest {
            messages: vec![ModelMessage::user("hello")],
            options: ModelRequestOptions {
                max_tokens: Some(1024),
                temperature: Some(0.2),
                top_p: Some(0.8),
                reasoning: ReasoningConfig {
                    enabled: Some(true),
                    effort: Some(ReasoningEffort::High),
                },
                ..ModelRequestOptions::default()
            },
            tools: Vec::new(),
        },
    );

    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["thinking"]["budget_tokens"], 16_000);
    assert_eq!(body["temperature"], 1.0);
    assert!((body["top_p"].as_f64().expect("top_p") - 0.8).abs() < 1e-6);
    assert_eq!(body["max_tokens"], 20_096);
    assert!(body.get("output_config").is_none());
}

#[test]
fn parses_anthropic_tool_use_response() {
    let text = r#"{
        "model": "claude-sonnet-4-5",
        "content": [
            {"type": "text", "text": "I'll search."},
            {
                "type": "tool_use",
                "id": "toolu-token=abc123",
                "name": "memory_search",
                "input": {"query": "hello token=abc123", "limit": 2}
            }
        ],
        "usage": {"input_tokens": 5, "output_tokens": 7}
    }"#;

    let response = parse_messages_response(text, "anthropic", "fallback").expect("response");
    assert_eq!(response.provider, "anthropic");
    assert_eq!(response.model, "claude-sonnet-4-5");
    assert_eq!(response.content, "I'll search.");
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].name, "memory_search");
    assert_eq!(response.tool_calls[0].input["limit"], 2);
    assert_eq!(
        response.tool_calls[0].input["query"],
        "hello token=[REDACTED_SECRET]"
    );
    assert_eq!(response.usage.prompt_tokens, Some(5));
    assert_eq!(response.usage.completion_tokens, Some(7));
    assert_eq!(response.usage.total_tokens, Some(12));
}

#[test]
fn anthropic_generate_backed_stream_events_are_normalized() {
    let tool_call = ModelToolCall {
        id: Some("toolu-token=[REDACTED_SECRET]".into()),
        name: "memory_search".into(),
        input: serde_json::json!({"query": "hello token=[REDACTED_SECRET]"}),
        raw_arguments: Some(r#"{"query":"hello token=[REDACTED_SECRET]"}"#.into()),
    };
    let usage = TokenUsage {
        prompt_tokens: Some(5),
        completion_tokens: Some(7),
        total_tokens: Some(12),
        ..TokenUsage::default()
    };
    let events = test_model_stream_events_from_response(
        "anthropic",
        "claude-sonnet-4-5",
        &["I'll search.".into()],
        &[tool_call],
        &usage,
    );

    assert_eq!(
        model_stream_event_kinds(&events),
        vec![
            "start",
            "text_delta",
            "tool_call_start",
            "tool_call_delta",
            "tool_call_end",
            "usage",
            "done"
        ]
    );
    assert!(matches!(
        &events[0],
        ModelStreamEvent::Start { provider, model }
            if provider == "anthropic" && model == "claude-sonnet-4-5"
    ));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::ToolCallDelta { args_delta, .. } if args_delta.contains("[REDACTED_SECRET]")))
    );
}

#[test]
fn parses_anthropic_native_sse_stream_fixture() {
    let text = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[],"usage":{"input_tokens":7,"cache_read_input_tokens":3}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello "}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"world token=abc123"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"memory_search","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"query\":\"hi "}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"token=abc123\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":5}}

event: message_stop
data: {"type":"message_stop"}
"#;

    let stream =
        parse_anthropic_stream_response(text, "anthropic", "fallback").expect("anthropic stream");

    assert_eq!(stream.provider, "anthropic");
    assert_eq!(stream.model, "claude-sonnet-4-5");
    assert_eq!(stream.content(), "Hello world token=[REDACTED_SECRET]");
    assert_eq!(stream.usage.prompt_tokens, Some(7));
    assert_eq!(stream.usage.completion_tokens, Some(5));
    assert_eq!(stream.usage.cache_read_tokens, Some(3));
    assert_eq!(stream.tool_calls.len(), 1);
    assert_eq!(stream.tool_calls[0].id.as_deref(), Some("toolu_1"));
    assert_eq!(stream.tool_calls[0].name, "memory_search");
    assert_eq!(
        stream.tool_calls[0].input["query"],
        serde_json::json!("hi token=[REDACTED_SECRET]")
    );
    assert_eq!(
        model_stream_event_kinds(&stream.events),
        vec![
            "start",
            "text_delta",
            "tool_call_start",
            "tool_call_delta",
            "tool_call_end",
            "usage",
            "done"
        ]
    );
    let rendered = serde_json::to_string(&stream).expect("stream json");
    assert!(!rendered.contains("abc123"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
}
