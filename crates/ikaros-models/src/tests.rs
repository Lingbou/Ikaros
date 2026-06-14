// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use crate::anthropic::{parse_messages_response, test_messages_request_body};
use crate::ollama::{
    parse_chat_response as parse_ollama_chat_response,
    parse_stream_response as parse_ollama_stream_response, test_chat_request_body,
};
use crate::openai_compatible::{
    parse_chat_completion_response, parse_stream_response, redacted_model_http_error,
};
use async_trait::async_trait;
use ikaros_core::{IkarosError, ModelConfig, RemoteProviderConfig, Result};
use std::{
    fs,
    sync::{Arc, Mutex},
};

struct CapturingProvider {
    seen: Arc<Mutex<Option<ModelRequest>>>,
}

#[async_trait]
impl ModelProvider for CapturingProvider {
    fn name(&self) -> &str {
        "capturing"
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        *self
            .seen
            .lock()
            .map_err(|_| IkarosError::Message("capture lock poisoned".into()))? = Some(request);
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "capture".into(),
            content: "ok".into(),
            tool_calls: Vec::new(),
            usage: TokenUsage {
                prompt_tokens: Some(1),
                completion_tokens: Some(1),
                total_tokens: None,
            },
        })
    }
}

#[tokio::test]
async fn mock_provider_redacts_secret_like_input() {
    let provider = MockModelProvider::default();
    let response = provider
        .generate(ModelRequest::from_user_text("please use sk-not-real"))
        .await
        .expect("mock");
    assert!(!response.content.contains("sk-not-real"));
    assert!(response.content.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn mock_provider_streams_redacted_chunks() {
    let provider = MockModelProvider::default();
    let stream = provider
        .stream(ModelRequest::from_user_text(
            "please stream a long answer while hiding token=abc123 from every chunk",
        ))
        .await
        .expect("stream");
    assert_eq!(stream.provider, "mock");
    assert!(stream.chunks.len() > 1);
    assert!(!stream.content().contains("abc123"));
    assert!(stream.content().contains("[REDACTED_SECRET]"));
}

#[test]
fn model_transport_descriptor_separates_runtime_and_wire_format() {
    let config = ModelConfig {
        provider: "openai-compatible".into(),
        model: "chat-model".into(),
        runtime: "harness-agent-loop".into(),
        transport: "openai-compatible-chat-completions".into(),
        ..ModelConfig::default()
    };
    let provider_settings = RemoteProviderConfig {
        api_key: "test-key".into(),
        base_url: "https://api.example/v1".into(),
    };
    let descriptor = model_transport_descriptor_from_config(&config, &provider_settings);

    assert_eq!(descriptor.provider, "openai-compatible");
    assert_eq!(descriptor.model, "chat-model");
    assert_eq!(descriptor.runtime, "harness-agent-loop");
    assert_eq!(descriptor.transport, "openai-compatible-chat-completions");
    assert_eq!(
        descriptor.base_url.as_deref(),
        Some("https://api.example/v1")
    );
    assert!(descriptor.supports_streaming);
    assert!(descriptor.normalizes_tool_calls);
}

#[tokio::test]
async fn mock_provider_returns_concise_code_review_notes() {
    let provider = MockModelProvider::default();
    let response = provider
        .generate(ModelRequest::from_user_text(
            "Heuristic review report:\nsecret token=abc123\n\nRedacted diff excerpt:\n+let x = 1;\n\nGuarded Patch Iteration",
        ))
        .await
        .expect("generate");
    assert!(response.content.contains("Residual Risks"));
    assert!(response.content.contains("Focused Tests"));
    assert!(response.content.contains("Guarded Patch Iteration"));
    assert!(!response.content.contains("Heuristic review report"));
    assert!(!response.content.contains("abc123"));
}

#[test]
fn model_factory_accepts_only_canonical_provider_names() {
    let provider_settings = RemoteProviderConfig {
        api_key: "test-key".into(),
        base_url: "https://api.example/v1".into(),
    };
    let config = ModelConfig {
        provider: "openai-compatible".into(),
        model: "example-model".into(),
        ..ModelConfig::default()
    };
    let provider = provider_from_config(&config, &provider_settings).expect("provider");
    assert_eq!(provider.name(), "openai-compatible");

    let alias = ModelConfig {
        provider: "openai".into(),
        model: "example-model".into(),
        ..ModelConfig::default()
    };
    let error = provider_from_config(&alias, &provider_settings)
        .err()
        .expect("alias rejected");
    assert!(
        error
            .to_string()
            .contains("unsupported model provider: openai")
    );
}

#[test]
fn anthropic_and_ollama_canonical_providers_are_supported() {
    let model_settings = RemoteProviderConfig {
        api_key: "test-key".into(),
        base_url: "https://api.anthropic.com/v1".into(),
    };
    let anthropic = provider_from_config(
        &ModelConfig {
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            ..ModelConfig::default()
        },
        &model_settings,
    )
    .expect("anthropic provider");
    assert_eq!(anthropic.name(), "anthropic");

    let ollama_settings = RemoteProviderConfig {
        api_key: String::new(),
        base_url: String::new(),
    };
    let ollama_config = ModelConfig {
        provider: "ollama".into(),
        model: "llama3.2".into(),
        ..ModelConfig::default()
    };
    let ollama = provider_from_config(&ollama_config, &ollama_settings).expect("ollama provider");
    assert_eq!(ollama.name(), "ollama");
    let ollama =
        OllamaProvider::from_config("ollama", &ollama_config, &ollama_settings).expect("ollama");
    assert_eq!(
        ollama.transport_descriptor().base_url.as_deref(),
        Some("http://127.0.0.1:11434")
    );
}

#[test]
fn openai_compatible_temperature_is_passthrough() {
    let config = ModelConfig {
        provider: "openai-compatible".into(),
        model: "kimi-k2.6".into(),
        ..ModelConfig::default()
    };
    let provider_settings = RemoteProviderConfig {
        api_key: "test-key".into(),
        base_url: "https://api.example/v1".into(),
    };
    let provider =
        OpenAiCompatibleProvider::from_config("openai-compatible", &config, &provider_settings)
            .expect("provider");
    assert_eq!(provider.compatible_temperature(Some(0.0)), Some(0.0));
    assert_eq!(provider.compatible_temperature(Some(0.4)), Some(0.4));
    assert_eq!(provider.compatible_temperature(None), None);
}

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
            max_tokens: Some(64),
            temperature: Some(0.1),
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
            max_tokens: Some(32),
            temperature: Some(0.0),
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
    let raw = serde_json::to_string(&body).expect("json");
    assert!(!raw.contains("abc123"));
    assert!(raw.contains("[REDACTED_SECRET]"));
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
}

#[tokio::test]
async fn governed_provider_logs_usage_without_prompt_content() {
    let temp = tempfile::tempdir().expect("tempdir");
    let provider = GovernedModelProvider::new(
        Box::new(MockModelProvider::default()),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::default(),
    );
    provider
        .generate(ModelRequest::from_user_text("remember token=abc123"))
        .await
        .expect("generate");
    let raw = fs::read_to_string(provider.ledger().path()).expect("usage log");
    assert!(!raw.contains("remember"));
    assert!(!raw.contains("abc123"));
    let records = provider.ledger().read_all().expect("records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].provider, "mock");
    assert!(records[0].total_tokens > 0);
}

#[tokio::test]
async fn governed_provider_stream_logs_usage_without_prompt_content() {
    let temp = tempfile::tempdir().expect("tempdir");
    let provider = GovernedModelProvider::new(
        Box::new(MockModelProvider::default()),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::default(),
    );
    let stream = provider
        .stream(ModelRequest::from_user_text("stream token=abc123"))
        .await
        .expect("stream");
    assert!(stream.content().contains("[REDACTED_SECRET]"));
    let raw = fs::read_to_string(provider.ledger().path()).expect("usage log");
    assert!(!raw.contains("stream"));
    assert!(!raw.contains("abc123"));
    let records = provider.ledger().read_all().expect("records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].provider, "mock");
    assert!(records[0].total_tokens > 0);
}

#[tokio::test]
async fn governed_provider_redacts_request_before_inner_provider() {
    let temp = tempfile::tempdir().expect("tempdir");
    let seen = Arc::new(Mutex::new(None));
    let provider = GovernedModelProvider::new(
        Box::new(CapturingProvider { seen: seen.clone() }),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::default(),
    );
    provider
        .generate(ModelRequest::from_user_text("never forward token=abc123"))
        .await
        .expect("generate");
    let captured = seen
        .lock()
        .expect("capture lock")
        .clone()
        .expect("captured request");
    let raw = serde_json::to_string(&captured).expect("json");
    assert!(!raw.contains("abc123"));
    assert!(raw.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn governed_provider_enforces_rate_limit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let provider = GovernedModelProvider::new(
        Box::new(MockModelProvider::default()),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits {
            rate_limit_per_minute: Some(1),
            daily_token_budget: None,
        },
    );
    provider
        .generate(ModelRequest::from_user_text("first"))
        .await
        .expect("first");
    let err = provider
        .generate(ModelRequest::from_user_text("second"))
        .await
        .expect_err("rate limited");
    assert!(err.to_string().contains("rate limit"));
}

#[tokio::test]
async fn governed_provider_enforces_daily_budget() {
    let temp = tempfile::tempdir().expect("tempdir");
    let provider = GovernedModelProvider::new(
        Box::new(MockModelProvider::default()),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits {
            rate_limit_per_minute: None,
            daily_token_budget: Some(5),
        },
    );
    let err = provider
        .generate(ModelRequest {
            messages: vec![ModelMessage::user("this request should exceed budget")],
            max_tokens: Some(128),
            temperature: None,
            tools: Vec::new(),
        })
        .await
        .expect_err("budget limited");
    assert!(err.to_string().contains("budget"));
}

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
    assert_eq!(stream.chunks.len(), 2);
    assert!(stream.content().contains("Hello world"));
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
fn openai_compatible_http_errors_redact_response_body() {
    let error = redacted_model_http_error(
        reqwest::StatusCode::BAD_REQUEST,
        r#"{"error":"provider echoed token=abc123 and sk-not-real"}"#,
    );

    assert!(error.contains("HTTP 400"));
    assert!(!error.contains("abc123"));
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
        |event| matches!(event, ModelStreamEvent::ToolCallStart { name, .. } if name == "memory_")
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
