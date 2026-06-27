// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn parses_openai_compatible_stream_chunks() {
    let text = r#"data: {"model":"stream-model","choices":[{"delta":{"content":"Hello "}}]}

data: {"model":"stream-model","choices":[{"delta":{"content":"world token=abc123"}}],"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}}

data: [DONE]
"#;
    let stream = parse_stream_response(text, "openai-compatible", "fallback").expect("stream");
    assert_eq!(stream.provider, "openai-compatible");
    assert_eq!(stream.model, "stream-model");
    assert_eq!(stream.usage.total_tokens, Some(5));
    assert_eq!(stream.content(), "Hello world token=[REDACTED_SECRET]");
    assert!(!stream.content().contains("abc123"));
    assert!(stream.content().contains("[REDACTED_SECRET]"));
    assert!(matches!(
        stream.events.first(),
        Some(ModelStreamEvent::Start {
            provider,
            model
        }) if provider == "openai-compatible" && model == "stream-model"
    ));
    assert!(stream.events.iter().any(
        |event| matches!(event, ModelStreamEvent::TextDelta(text) if text.contains("Hello "))
    ));
    assert!(stream.events.iter().any(
        |event| matches!(event, ModelStreamEvent::Usage(usage) if usage.total_tokens == Some(5))
    ));
    assert!(matches!(stream.events.last(), Some(ModelStreamEvent::Done)));
}

#[test]
fn openai_compatible_stream_fixture_emits_canonical_event_sequence() {
    let text = r#"data: {"model":"fixture-model","choices":[{"delta":{"content":"Hello ","reasoning":"thinking token=abc123"}}]}

data: {"model":"fixture-model","choices":[{"delta":{"refusal":"cannot reveal token=abc123"}}]}

data: {"model":"fixture-model","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"memory_search","arguments":"{\"query\":\"hi "}}]}}]}

data: {"model":"fixture-model","choices":[{"delta":{"content":"world","tool_calls":[{"index":0,"function":{"arguments":"token=abc123\"}"}}]}}],"usage":{"prompt_tokens":3,"completion_tokens":5,"total_tokens":8}}

data: [DONE]
"#;
    let stream = parse_stream_response(text, "openai-compatible", "fallback").expect("stream");

    assert_eq!(stream.model, "fixture-model");
    assert_eq!(stream.content(), "Hello world");
    assert_eq!(stream.tool_calls.len(), 1);
    assert_eq!(stream.tool_calls[0].name, "memory_search");
    assert_eq!(
        stream.tool_calls[0].input["query"],
        "hi token=[REDACTED_SECRET]"
    );
    assert_eq!(stream.usage.total_tokens, Some(8));
    assert_eq!(
        model_stream_event_kinds(&stream.events),
        vec![
            "start",
            "text_delta",
            "reasoning_delta",
            "refusal_delta",
            "text_delta",
            "reasoning_delta",
            "refusal_delta",
            "tool_call_start",
            "tool_call_delta",
            "tool_call_end",
            "usage",
            "done"
        ]
    );
    assert!(
        stream
            .events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::ReasoningDelta(text) if text.contains("[REDACTED_SECRET]")))
    );
    assert!(
        stream
            .events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::RefusalDelta(text) if text.contains("[REDACTED_SECRET]")))
    );
    assert!(
        stream
            .events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::ToolCallDelta { args_delta, .. } if args_delta.contains("[REDACTED_SECRET]")))
    );
}

#[test]
fn openai_compatible_http_errors_redact_response_body() {
    let error = redacted_model_http_error(
        reqwest::StatusCode::BAD_REQUEST.as_u16(),
        &BTreeMap::new(),
        r#"{"error":"provider echoed token=abc123 and sk-not-real"}"#,
    );

    assert!(error.contains("HTTP 400"));
    assert!(!error.contains("abc123"));
    assert!(!error.contains("sk-not-real"));
    assert!(error.contains("[REDACTED_SECRET]"));
}

#[test]
fn openai_compatible_http_errors_include_retry_after_without_leaking_headers() {
    let headers = BTreeMap::from([
        ("retry-after".into(), "0".into()),
        ("set-cookie".into(), "session=sk-header-secret".into()),
        ("x-api-key".into(), "token=header-secret".into()),
    ]);
    let error = redacted_model_http_error(
        reqwest::StatusCode::TOO_MANY_REQUESTS.as_u16(),
        &headers,
        r#"{"error":"provider echoed token=body-secret and sk-not-real"}"#,
    );

    assert!(error.contains("HTTP 429"));
    assert!(error.contains("Retry-After: 0"));
    assert!(!error.contains("set-cookie"));
    assert!(!error.contains("x-api-key"));
    assert!(!error.contains("sk-header-secret"));
    assert!(!error.contains("header-secret"));
    assert!(!error.contains("body-secret"));
    assert!(!error.contains("sk-not-real"));
    assert!(error.contains("[REDACTED_SECRET]"));
}

#[test]
fn parses_openai_compatible_stream_tool_call_deltas() {
    let first_chunk = serde_json::json!({
        "model": "stream-tool-model",
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": "call-token=abc123",
                    "function": {
                        "name": "memory_",
                        "arguments": "{"
                    }
                }]
            }
        }]
    });
    let second_chunk = serde_json::json!({
        "model": "stream-tool-model",
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "function": {
                        "name": "search",
                        "arguments": "\"query\":\"hello token=abc123\",\"limit\":2}"
                    }
                }]
            }
        }],
        "usage": {"prompt_tokens": 2, "completion_tokens": 4, "total_tokens": 6}
    });
    let text = format!("data: {first_chunk}\n\ndata: {second_chunk}\n\ndata: [DONE]\n");
    let stream = parse_stream_response(&text, "openai-compatible", "fallback").expect("stream");

    assert_eq!(stream.provider, "openai-compatible");
    assert_eq!(stream.model, "stream-tool-model");
    assert!(stream.chunks.is_empty());
    assert_eq!(stream.tool_calls.len(), 1);
    assert_eq!(stream.tool_calls[0].name, "memory_search");
    assert_eq!(stream.tool_calls[0].input["limit"], 2);
    assert_eq!(
        stream.tool_calls[0].input["query"],
        "hello token=[REDACTED_SECRET]"
    );
    assert!(
        stream.tool_calls[0]
            .id
            .as_deref()
            .is_some_and(|id| id.contains("[REDACTED_SECRET]"))
    );
    assert_eq!(stream.usage.total_tokens, Some(6));
    assert!(stream.events.iter().any(
        |event| matches!(event, ModelStreamEvent::ToolCallStart { name, .. } if name == "memory_search")
    ));
    assert!(
        stream
            .events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::ToolCallDelta { args_delta, .. } if args_delta.contains("[REDACTED_SECRET]")))
    );
    assert!(
        stream
            .events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::ToolCallEnd { id } if id.contains("[REDACTED_SECRET]")))
    );
}

#[test]
fn openai_compatible_stream_redacts_split_tool_argument_secrets() {
    let first_chunk = serde_json::json!({
        "model": "stream-tool-model",
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": "call-1",
                    "function": {
                        "name": "memory_search",
                        "arguments": "{\"query\":\"sk-"
                    }
                }]
            }
        }]
    });
    let second_chunk = serde_json::json!({
        "model": "stream-tool-model",
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "function": {
                        "arguments": "abc\"}"
                    }
                }]
            }
        }]
    });
    let text = format!("data: {first_chunk}\n\ndata: {second_chunk}\n\ndata: [DONE]\n");
    let stream = parse_stream_response(&text, "openai-compatible", "fallback").expect("stream");
    let rendered_events = serde_json::to_string(&stream.events).expect("events");

    assert!(!rendered_events.contains("sk-"));
    assert!(!rendered_events.contains("abc"));
    assert!(rendered_events.contains("[REDACTED_SECRET]"));
    assert_eq!(
        stream.tool_calls[0].input["query"],
        serde_json::json!("[REDACTED_SECRET]")
    );
}

#[test]
fn openai_compatible_stream_redacts_split_text_reasoning_and_refusal_secrets() {
    let first_chunk = serde_json::json!({
        "model": "stream-redaction-model",
        "choices": [{
            "delta": {
                "content": "visible sk-",
                "reasoning_content": "thinking token=",
                "refusal": "cannot reveal api_key="
            }
        }]
    });
    let second_chunk = serde_json::json!({
        "model": "stream-redaction-model",
        "choices": [{
            "delta": {
                "content": "splitcontent42 done",
                "reasoning_content": "splitreason43 ",
                "refusal": "splitrefusal44 "
            }
        }]
    });
    let text = format!("data: {first_chunk}\n\ndata: {second_chunk}\n\ndata: [DONE]\n");
    let stream = parse_stream_response(&text, "openai-compatible", "fallback").expect("stream");
    let rendered_events = serde_json::to_string(&stream.events).expect("events");
    let rendered_chunks = serde_json::to_string(&stream.chunks).expect("chunks");

    assert!(!rendered_events.contains("sk-"));
    assert!(!rendered_events.contains("splitcontent42"));
    assert!(!rendered_events.contains("splitreason43"));
    assert!(!rendered_events.contains("splitrefusal44"));
    assert!(!rendered_chunks.contains("sk-"));
    assert!(!rendered_chunks.contains("splitcontent42"));
    assert!(rendered_events.contains("[REDACTED_SECRET]"));
    assert!(rendered_chunks.contains("[REDACTED_SECRET]"));
}

#[test]
fn parses_openai_compatible_stream_reasoning_and_refusal_events() {
    let first_chunk = serde_json::json!({
        "model": "stream-reasoning-model",
        "choices": [{
            "delta": {
                "reasoning_content": "thinking token=abc123",
                "refusal": "cannot reveal token=abc123"
            }
        }]
    });
    let text = format!("data: {first_chunk}\n\ndata: [DONE]\n");
    let stream = parse_stream_response(&text, "openai-compatible", "fallback").expect("stream");

    assert!(
        stream
            .events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::ReasoningDelta(text) if text.contains("[REDACTED_SECRET]")))
    );
    assert!(
        stream
            .events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::RefusalDelta(text) if text.contains("[REDACTED_SECRET]")))
    );
}

#[test]
fn parses_openai_compatible_native_tool_calls() {
    let text = r#"{
        "model": "tool-model",
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call-token=abc123",
                    "type": "function",
                    "function": {
                        "name": "memory_search",
                        "arguments": "{\"query\":\"hello token=abc123\",\"limit\":2}"
                    }
                }]
            }
        }],
        "usage": {"prompt_tokens": 3, "completion_tokens": 4, "total_tokens": 7}
    }"#;

    let response =
        parse_chat_completion_response(text, "openai-compatible", "fallback").expect("response");

    assert_eq!(response.provider, "openai-compatible");
    assert_eq!(response.model, "tool-model");
    assert!(response.content.is_empty());
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].name, "memory_search");
    assert_eq!(response.tool_calls[0].input["limit"], 2);
    assert_eq!(
        response.tool_calls[0].input["query"],
        "hello token=[REDACTED_SECRET]"
    );
    assert!(
        response.tool_calls[0]
            .id
            .as_deref()
            .is_some_and(|id| id.contains("[REDACTED_SECRET]"))
    );
    assert_eq!(response.usage.total_tokens, Some(7));
}

#[test]
fn provider_context_profiles_are_provider_aware() {
    let kimi = ProviderProfile::resolve_profile_id(
        "moonshot-kimi",
        "https://api.moonshot.cn/v1",
        "kimi-k2.6",
    )
    .expect("moonshot-kimi profile")
    .context;
    assert_eq!(kimi.context_window, 128_000);
    assert_eq!(kimi.default_output_tokens, 32_000);
    assert_eq!(kimi.tokenizer, ModelTokenizerKind::OpenAiCompatible);

    let gemini = ProviderProfile::resolve_profile_id(
        "gemini-openai",
        "https://generativelanguage.googleapis.com/v1beta/openai",
        "gemini-2.5-pro",
    )
    .expect("gemini-openai profile")
    .context;
    assert_eq!(gemini.context_window, 1_048_576);
    assert_eq!(gemini.default_output_tokens, 8_192);

    let mock = MockModelProvider::default().context_profile();
    assert_eq!(mock.context_window, 8_192);
    assert_eq!(mock.default_output_tokens, 1_024);
    assert_eq!(mock.tokenizer, ModelTokenizerKind::Mock);
}
