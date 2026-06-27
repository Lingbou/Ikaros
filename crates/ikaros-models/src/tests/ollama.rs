// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn ollama_request_body_uses_local_chat_tool_history() {
    let config = ModelConfig {
        provider: "ollama".into(),
        model: "llama3.2".into(),
        ..ModelConfig::default()
    };
    let body = test_chat_request_body(
        &config,
        ModelRequest {
            messages: vec![
                ModelMessage::user("what is the weather?"),
                ModelMessage::assistant_with_tool_calls(
                    "",
                    vec![ModelToolCall {
                        id: Some("ignored-by-ollama".into()),
                        name: "get_weather".into(),
                        input: serde_json::json!({"city": "Tokyo token=abc123"}),
                        raw_arguments: None,
                    }],
                ),
                ModelMessage::tool_result(
                    "ignored-by-ollama",
                    "get_weather",
                    "11 degrees token=abc123",
                ),
            ],
            options: ModelRequestOptions {
                max_tokens: Some(32),
                temperature: Some(0.0),
                top_p: Some(0.8),
                seed: Some(42),
                stop: vec!["END".into()],
                ..ModelRequestOptions::default()
            },
            tools: vec![ModelToolDefinition {
                name: "get_weather".into(),
                description: "Get weather".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }],
        },
        false,
    );

    assert_eq!(body["model"], "llama3.2");
    assert_eq!(body["stream"], false);
    assert_eq!(
        body["messages"][1]["tool_calls"][0]["function"]["name"],
        "get_weather"
    );
    assert_eq!(body["messages"][2]["role"], "tool");
    assert_eq!(body["messages"][2]["tool_name"], "get_weather");
    assert_eq!(body["options"]["num_predict"], 32);
    assert!((body["options"]["top_p"].as_f64().expect("top_p") - 0.8).abs() < 1e-6);
    assert_eq!(body["options"]["seed"], 42);
    assert_eq!(body["options"]["stop"], serde_json::json!(["END"]));
    let raw = serde_json::to_string(&body).expect("json");
    assert!(!raw.contains("abc123"));
    assert!(raw.contains("[REDACTED_SECRET]"));
}

#[test]
fn ollama_request_body_supports_base64_image_content_blocks() {
    let config = ModelConfig {
        provider: "ollama".into(),
        model: "llava".into(),
        ..ModelConfig::default()
    };
    let body = test_chat_request_body(
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
            options: ModelRequestOptions::default(),
            tools: Vec::new(),
        },
        false,
    );

    assert_eq!(body["messages"][0]["content"], "describe");
    assert_eq!(body["messages"][0]["images"], serde_json::json!(["aW1n"]));
}

#[test]
fn parses_ollama_tool_call_response() {
    let text = r#"{
        "model": "llama3.2",
        "message": {
            "role": "assistant",
            "content": "",
            "tool_calls": [{
                "function": {
                    "name": "get_weather",
                    "arguments": {"city": "Tokyo token=abc123"}
                }
            }]
        },
        "prompt_eval_count": 4,
        "eval_count": 6
    }"#;

    let response = parse_ollama_chat_response(text, "ollama", "fallback").expect("response");
    assert_eq!(response.provider, "ollama");
    assert_eq!(response.model, "llama3.2");
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].name, "get_weather");
    assert_eq!(
        response.tool_calls[0].input["city"],
        "Tokyo token=[REDACTED_SECRET]"
    );
    assert_eq!(response.usage.total_tokens, Some(10));
}

#[test]
fn parses_ollama_stream_chunks_and_tool_calls() {
    let first = serde_json::json!({
        "model": "llama3.2",
        "message": {
            "role": "assistant",
            "content": "",
            "tool_calls": [{
                "function": {
                    "name": "get_weather",
                    "arguments": {"city": "Tokyo"}
                }
            }]
        },
        "done": false
    });
    let second = serde_json::json!({
        "model": "llama3.2",
        "message": {"role": "assistant", "content": "done token=abc123"},
        "done": false
    });
    let final_chunk = serde_json::json!({
        "model": "llama3.2",
        "message": {"role": "assistant", "content": ""},
        "done": true,
        "prompt_eval_count": 3,
        "eval_count": 5
    });
    let text = format!("{first}\n{second}\n{final_chunk}\n");

    let stream = parse_ollama_stream_response(&text, "ollama", "fallback").expect("stream");
    assert_eq!(stream.provider, "ollama");
    assert_eq!(stream.model, "llama3.2");
    assert_eq!(stream.tool_calls.len(), 1);
    assert_eq!(stream.tool_calls[0].name, "get_weather");
    assert!(stream.content().contains("[REDACTED_SECRET]"));
    assert_eq!(stream.usage.total_tokens, Some(8));
    assert_eq!(
        model_stream_event_kinds(&stream.events),
        vec![
            "start",
            "tool_call_start",
            "tool_call_delta",
            "tool_call_end",
            "text_delta",
            "usage",
            "done"
        ]
    );
    assert!(matches!(
        &stream.events[0],
        ModelStreamEvent::Start { provider, model }
            if provider == "ollama" && model == "llama3.2"
    ));
    assert!(
        stream
            .events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::TextDelta(text) if text.contains("[REDACTED_SECRET]")))
    );
}
