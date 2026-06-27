// SPDX-License-Identifier: GPL-3.0-only

use crate::resolve_agent_instance;
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use ikaros_core::{
    AgentInstance, IkarosConfig, IkarosPaths, ModelConfig, RemoteProviderConfig,
    StructuredTraceEvent, StructuredTraceLog, redact_secrets,
};
use ikaros_harness::{AuditEvent, AuditLog, NetworkEgressRequest};
use ikaros_models::{
    ModelContentBlock, ModelHttpClient, ModelHttpRequest, ModelMessage, ModelRequest,
    ModelRequestOptions, ModelStream, ModelToolCall, ModelToolDefinition, TokenUsage,
    governed_provider_from_config_with_http_client, model_request_options_from_config,
};
use ikaros_runtime::{EgressModelHttpClient, session_and_registry_for_instance};
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, IKAROS_PROTOCOL_NAME,
    IKAROS_PROTOCOL_VERSION, PersistingAgentTurnSink, SessionEntry, SessionEntryKind, SessionId,
    SessionSource, SessionStore, SqliteSessionStore, TurnId,
};
use ikaros_skills::with_execution_env_embedding_provider;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use time::OffsetDateTime;

mod audit;
mod evidence;
mod health;
mod http;
mod openai;
mod routing;
mod server;

use self::{audit::*, evidence::*, health::*, http::*, openai::*, routing::*, server::*};

#[derive(Debug, Subcommand)]
pub(crate) enum ApiCommand {
    /// Serve a local OpenAI-compatible API surface.
    Serve(ApiServe),
}

#[derive(Debug, Args)]
pub(crate) struct ApiServe {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 8003)]
    port: u16,
    #[arg(long, default_value_t = 64 * 1024)]
    max_body_bytes: usize,
    /// Optional bearer token required for /v1/* routes. Repeat to allow key rotation.
    #[arg(long, value_name = "TOKEN")]
    bearer_token: Vec<String>,
    /// Per-process request limit per minute. Use 0 to disable.
    #[arg(long, default_value_t = 120)]
    rate_limit_per_minute: u32,
    #[arg(long)]
    once: bool,
}

pub(crate) async fn api_command(
    command: ApiCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    match command {
        ApiCommand::Serve(args) => serve_api(args, paths, workspace, agent_override),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_message_parts_map_text_and_image_content_blocks() {
        let (content, blocks) =
            api_message_content_to_model_content(ApiMessageContent::Parts(vec![
                ApiContentPart {
                    kind: Some("text".into()),
                    text: Some("hello".into()),
                    image_url: None,
                    audio_url: None,
                    input_audio: None,
                    file_url: None,
                    file: None,
                },
                ApiContentPart {
                    kind: Some("image_url".into()),
                    text: None,
                    image_url: Some(ApiImageUrl::Object {
                        url: "https://images.example/cat.png".into(),
                        detail: Some("high".into()),
                    }),
                    audio_url: None,
                    input_audio: None,
                    file_url: None,
                    file: None,
                },
                ApiContentPart {
                    kind: None,
                    text: Some("world".into()),
                    image_url: None,
                    audio_url: None,
                    input_audio: None,
                    file_url: None,
                    file: None,
                },
            ]))
            .expect("content");
        assert_eq!(content, "hello\nworld");
        assert_eq!(
            blocks,
            vec![
                ModelContentBlock::text("hello"),
                ModelContentBlock::Image {
                    image_url: "https://images.example/cat.png".into(),
                    mime_type: None,
                    detail: Some("high".into()),
                },
                ModelContentBlock::text("world"),
            ]
        );
    }

    #[test]
    fn api_message_content_rejects_unsupported_parts() {
        let error =
            api_message_content_to_model_content(ApiMessageContent::Parts(vec![ApiContentPart {
                kind: Some("input_audio".into()),
                text: None,
                image_url: None,
                audio_url: None,
                input_audio: None,
                file_url: None,
                file: None,
            }]))
            .expect_err("invalid audio part");
        assert!(error.to_string().contains("requires"));
    }

    #[test]
    fn api_responses_input_maps_text_and_image_parts() {
        let messages = api_responses_input_to_model_messages(ApiResponsesInput::Items(vec![
            ApiResponseInputItem {
                role: Some("user".into()),
                content: Some(ApiResponseInputContent::Parts(vec![
                    ApiResponseContentPart {
                        kind: Some("input_text".into()),
                        text: Some("hello".into()),
                        image_url: None,
                        audio_url: None,
                        input_audio: None,
                        file_url: None,
                        file_id: None,
                        file_data: None,
                        filename: None,
                        detail: None,
                    },
                    ApiResponseContentPart {
                        kind: Some("input_image".into()),
                        text: None,
                        image_url: Some("https://images.example/cat.png".into()),
                        audio_url: None,
                        input_audio: None,
                        file_url: None,
                        file_id: None,
                        file_data: None,
                        filename: None,
                        detail: Some("high".into()),
                    },
                ])),
            },
        ]))
        .expect("responses input");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "hello");
        assert_eq!(
            messages[0].content_blocks,
            vec![
                ModelContentBlock::text("hello"),
                ModelContentBlock::Image {
                    image_url: "https://images.example/cat.png".into(),
                    mime_type: None,
                    detail: Some("high".into()),
                },
            ]
        );
    }

    #[test]
    fn api_response_tool_maps_function_definition() {
        let tool = api_response_tool_to_model_tool(ApiResponseToolDefinition {
            kind: Some("function".into()),
            name: Some("search".into()),
            description: Some("Search local context".into()),
            parameters: Some(json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"}
                },
                "required": ["query"]
            })),
        })
        .expect("response tool");

        assert_eq!(tool.name, "search");
        assert_eq!(tool.description, "Search local context");
        assert_eq!(tool.input_schema["properties"]["query"]["type"], "string");
    }

    #[test]
    fn api_usage_uses_total_or_prompt_completion() {
        let usage = openai_usage_json(&TokenUsage {
            prompt_tokens: Some(2),
            completion_tokens: Some(3),
            cache_read_tokens: Some(1),
            ..TokenUsage::default()
        });
        assert_eq!(usage["total_tokens"], 5);
        assert_eq!(usage["prompt_tokens_details"]["cached_tokens"], 1);
    }

    #[test]
    fn api_responses_response_body_includes_output_usage_and_session() {
        let body = responses_response_body(
            ApiResponsesBody {
                content: "hello world".into(),
                model: "mock-ikaros".into(),
                provider: "mock".into(),
                tool_calls: vec![ModelToolCall {
                    id: Some("call_1".into()),
                    name: "lookup".into(),
                    input: json!({"query": "docs"}),
                    raw_arguments: None,
                }],
                usage: TokenUsage {
                    prompt_tokens: Some(2),
                    completion_tokens: Some(3),
                    ..TokenUsage::default()
                },
                diagnostics: Vec::new(),
                created: 42,
            },
            Some(&ApiSessionIds {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
            }),
        );

        assert_eq!(body["object"], "response");
        assert_eq!(body["status"], "completed");
        assert_eq!(body["output_text"], "hello world");
        assert_eq!(body["usage"]["input_tokens"], 2);
        assert_eq!(body["usage"]["output_tokens"], 3);
        assert_eq!(body["usage"]["total_tokens"], 5);
        assert_eq!(body["output"][1]["type"], "function_call");
        assert_eq!(body["output"][1]["name"], "lookup");
        assert_eq!(body["ikaros"]["session_id"], "session-1");
        assert_eq!(body["ikaros"]["turn_id"], "turn-1");
    }

    #[test]
    fn api_models_response_lists_chat_and_embedding_capabilities() {
        let body = openai_models_response_body([
            ApiModelRow {
                id: "chat-model".into(),
                provider: "openai-compatible".into(),
                capabilities: vec!["chat.completions"],
            },
            ApiModelRow {
                id: "embedding-model".into(),
                provider: "hash".into(),
                capabilities: vec!["embeddings"],
            },
        ]);
        assert_eq!(body["object"], "list");
        assert_eq!(body["data"][0]["id"], "chat-model");
        assert_eq!(
            body["data"][0]["ikaros"]["capabilities"][0],
            "chat.completions"
        );
        assert_eq!(body["data"][1]["id"], "embedding-model");
        assert_eq!(body["data"][1]["ikaros"]["capabilities"][0], "embeddings");
    }

    #[test]
    fn api_models_response_merges_capabilities_for_same_model_id() {
        let body = openai_models_response_body([
            ApiModelRow {
                id: "shared-model".into(),
                provider: "openai-compatible".into(),
                capabilities: vec!["chat.completions"],
            },
            ApiModelRow {
                id: "shared-model".into(),
                provider: "openai-compatible".into(),
                capabilities: vec!["embeddings"],
            },
        ]);
        assert_eq!(body["data"].as_array().expect("data").len(), 1);
        assert_eq!(body["data"][0]["id"], "shared-model");
        assert_eq!(
            body["data"][0]["ikaros"]["capabilities"],
            json!(["chat.completions", "embeddings"])
        );
    }

    #[test]
    fn api_embedding_input_accepts_string_and_string_array_only() {
        let single = ApiEmbeddingInput::One("hello".into())
            .values()
            .expect("single");
        assert_eq!(single, vec!["hello"]);
        let many = ApiEmbeddingInput::Many(vec!["hello".into(), "world".into()])
            .values()
            .expect("many");
        assert_eq!(many, vec!["hello", "world"]);
        assert!(
            ApiEmbeddingInput::Other(json!({"text": "hello"}))
                .values()
                .is_err()
        );
    }

    #[test]
    fn api_embedding_response_uses_openai_shape_and_usage() {
        let body = openai_embedding_response_body(
            "embedding-model".into(),
            vec![vec![0.1, 0.2], vec![0.3, 0.4]],
            7,
            ApiEmbeddingEncoding::Float,
            Some(&ApiSessionIds {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
            }),
        );
        assert_eq!(body["object"], "list");
        assert_eq!(body["model"], "embedding-model");
        assert_eq!(body["data"][0]["object"], "embedding");
        assert_eq!(body["data"][1]["index"], 1);
        assert_eq!(body["usage"]["prompt_tokens"], 7);
        assert_eq!(body["usage"]["total_tokens"], 7);
        assert_eq!(body["ikaros"]["session_id"], "session-1");
        assert_eq!(body["ikaros"]["turn_id"], "turn-1");
    }

    #[test]
    fn api_embedding_response_supports_base64_encoding() {
        let body = openai_embedding_response_body(
            "embedding-model".into(),
            vec![vec![1.0, -2.5]],
            3,
            ApiEmbeddingEncoding::Base64,
            None,
        );
        assert_eq!(body["data"][0]["embedding"], "AACAPwAAIMA=");
    }

    #[test]
    fn api_embedding_encoding_accepts_float_and_base64_only() {
        assert_eq!(
            ApiEmbeddingEncoding::parse(None).expect("default"),
            ApiEmbeddingEncoding::Float
        );
        assert_eq!(
            ApiEmbeddingEncoding::parse(Some("base64")).expect("base64"),
            ApiEmbeddingEncoding::Base64
        );
        assert!(ApiEmbeddingEncoding::parse(Some("binary")).is_err());
    }

    #[test]
    fn api_embedding_token_estimate_is_nonzero() {
        assert_eq!(estimate_embedding_tokens(""), 1);
        assert!(estimate_embedding_tokens("hello world") >= 2);
    }

    #[test]
    fn api_tool_definition_maps_openai_function_tool() {
        let tool = api_tool_definition_to_model_tool(ApiToolDefinition {
            kind: Some("function".into()),
            function: ApiToolFunctionDefinition {
                name: "get_weather".into(),
                description: Some("Fetch weather".into()),
                parameters: Some(json!({
                    "type": "object",
                    "properties": {
                        "city": {"type": "string"}
                    }
                })),
            },
        })
        .expect("tool");
        assert_eq!(tool.name, "get_weather");
        assert_eq!(tool.description, "Fetch weather");
        assert_eq!(tool.input_schema["properties"]["city"]["type"], "string");
    }

    #[test]
    fn api_message_maps_assistant_tool_calls_and_tool_results() {
        let assistant = api_message_to_model_message(ApiChatMessage {
            role: "assistant".into(),
            content: None,
            name: None,
            tool_call_id: None,
            tool_calls: vec![ApiToolCall {
                id: Some("call_1".into()),
                kind: Some("function".into()),
                function: ApiToolCallFunction {
                    name: "get_weather".into(),
                    arguments: Some(Value::String("{\"city\":\"Paris\"}".into())),
                },
            }],
        })
        .expect("assistant");
        assert_eq!(assistant.role, "assistant");
        assert_eq!(assistant.tool_calls.len(), 1);
        assert_eq!(assistant.tool_calls[0].name, "get_weather");
        assert_eq!(assistant.tool_calls[0].input["city"], "Paris");
        assert_eq!(
            assistant.tool_calls[0].raw_arguments.as_deref(),
            Some("{\"city\":\"Paris\"}")
        );

        let tool = api_message_to_model_message(ApiChatMessage {
            role: "tool".into(),
            content: Some(ApiMessageContent::Text("rainy".into())),
            name: None,
            tool_call_id: Some("call_1".into()),
            tool_calls: Vec::new(),
        })
        .expect("tool");
        assert_eq!(tool.role, "tool");
        assert_eq!(tool.tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(tool.content, "rainy");
    }

    #[test]
    fn api_tool_calls_project_to_openai_assistant_message() {
        let calls = vec![ModelToolCall {
            id: Some("call_1".into()),
            name: "get_weather".into(),
            input: json!({"city": "Paris"}),
            raw_arguments: None,
        }];
        let message = openai_assistant_message_json("", &calls);
        assert_eq!(message["role"], "assistant");
        assert!(message["content"].is_null());
        assert_eq!(message["tool_calls"][0]["id"], "call_1");
        assert_eq!(message["tool_calls"][0]["type"], "function");
        assert_eq!(message["tool_calls"][0]["function"]["name"], "get_weather");
        assert_eq!(openai_finish_reason(&calls), "tool_calls");
    }

    #[test]
    fn api_stream_body_uses_openai_sse_shape() {
        let body = openai_stream_body(
            &ModelStream {
                provider: "mock".into(),
                model: "mock-ikaros".into(),
                chunks: vec!["hello".into(), " world".into()],
                tool_calls: Vec::new(),
                usage: TokenUsage {
                    prompt_tokens: Some(1),
                    completion_tokens: Some(2),
                    total_tokens: Some(3),
                    ..TokenUsage::default()
                },
                events: Vec::new(),
                diagnostics: Vec::new(),
            },
            42,
            Some(&ApiSessionIds {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
            }),
        )
        .expect("stream body");
        assert!(body.contains("\"object\":\"chat.completion.chunk\""));
        assert!(body.contains("\"content\":\"hello\""));
        assert!(body.contains("\"finish_reason\":\"stop\""));
        assert!(body.contains("\"session_id\":\"session-1\""));
        assert!(body.contains("\"turn_id\":\"turn-1\""));
        assert!(body.ends_with("data: [DONE]\n\n"));
    }

    #[test]
    fn api_stream_body_projects_tool_calls_in_final_chunk() {
        let body = openai_stream_body(
            &ModelStream {
                provider: "mock".into(),
                model: "mock-ikaros".into(),
                chunks: Vec::new(),
                tool_calls: vec![ModelToolCall {
                    id: Some("call_1".into()),
                    name: "get_weather".into(),
                    input: json!({"city": "Paris"}),
                    raw_arguments: None,
                }],
                usage: TokenUsage::default(),
                events: Vec::new(),
                diagnostics: Vec::new(),
            },
            42,
            None,
        )
        .expect("stream body");
        assert!(body.contains("\"finish_reason\":\"tool_calls\""));
        assert!(body.contains("\"tool_calls\""));
        assert!(body.contains("\"name\":\"get_weather\""));
    }

    #[test]
    fn api_responses_stream_body_emits_responses_events() {
        let body = responses_stream_body(
            &ModelStream {
                provider: "mock".into(),
                model: "mock-ikaros".into(),
                chunks: vec!["hel".into(), "lo".into()],
                tool_calls: Vec::new(),
                usage: TokenUsage {
                    prompt_tokens: Some(1),
                    completion_tokens: Some(2),
                    total_tokens: Some(3),
                    ..TokenUsage::default()
                },
                events: Vec::new(),
                diagnostics: Vec::new(),
            },
            42,
            Some(&ApiSessionIds {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
            }),
        )
        .expect("responses stream");

        assert!(body.contains("event: response.created"));
        assert!(body.contains("event: response.output_text.delta"));
        assert!(body.contains("\"delta\":\"hel\""));
        assert!(body.contains("event: response.output_text.done"));
        assert!(body.contains("\"text\":\"hello\""));
        assert!(body.contains("event: response.completed"));
        assert!(body.contains("\"session_id\":\"session-1\""));
        assert!(body.ends_with("data: [DONE]\n\n"));
    }

    #[test]
    fn api_response_observability_headers_include_correlation_id() {
        let response = ApiHttpResponse::json_error(200, "OK", "ok").with_session(ApiSessionIds {
            session_id: "session-1".into(),
            turn_id: "turn-1".into(),
        });
        let headers = response.observability_headers();
        assert_eq!(
            headers
                .iter()
                .find(|header| header.name == "X-Ikaros-Session-Id")
                .map(|header| header.value.as_str()),
            Some("session-1")
        );
        assert_eq!(
            headers
                .iter()
                .find(|header| header.name == "X-Ikaros-Turn-Id")
                .map(|header| header.value.as_str()),
            Some("turn-1")
        );
        assert_eq!(
            headers
                .iter()
                .find(|header| header.name == "X-Ikaros-Correlation-Id")
                .map(|header| header.value.as_str()),
            Some("session:session-1:turn:turn-1")
        );
    }

    #[test]
    fn api_bad_request_errors_are_structured_json_without_config() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = IkarosPaths::from_home(temp.path());
        let chat = api_http_response(
            "POST",
            "/v1/chat/completions",
            b"{bad-json",
            &ApiHeaders::default(),
            &paths,
            temp.path(),
            None,
        )
        .expect("chat response");
        assert_eq!(chat.status_code, 400);
        assert!(chat.body.contains("invalid chat completion JSON body"));

        let embedding = api_http_response(
            "POST",
            "/v1/embeddings",
            br#"{"input":{"text":"hello"}}"#,
            &ApiHeaders::default(),
            &paths,
            temp.path(),
            None,
        )
        .expect("embedding response");
        assert_eq!(embedding.status_code, 400);
        assert!(embedding.body.contains("embedding input must be a string"));
    }

    #[test]
    fn api_internal_error_response_is_structured_and_redacted() {
        let response = ApiHttpResponse::internal_error(anyhow::anyhow!(
            "provider failed with sk-secret-value"
        ));
        assert_eq!(response.status_code, 500);
        assert!(response.body.contains("ikaros_api_error"));
        assert!(!response.body.contains("sk-secret-value"));
    }

    #[test]
    fn api_rejects_non_loopback_host() {
        assert!(require_loopback_host("127.0.0.1").is_ok());
        assert!(require_loopback_host("localhost").is_ok());
        assert!(require_loopback_host("0.0.0.0").is_err());
    }

    #[test]
    fn api_requires_bearer_token_for_v1_routes() {
        let state = ApiServerState::new(vec!["secret-token".into()], 0);
        let response = state
            .security_response("/v1/models", &ApiHeaders::default())
            .expect("unauthorized");
        assert_eq!(response.status_code, 401);
        assert!(response.body.contains("bearer token"));
        assert!(
            response
                .extra_headers
                .iter()
                .any(|header| header.name == "WWW-Authenticate")
        );
    }

    #[test]
    fn api_accepts_correct_bearer_token_and_leaves_health_open() {
        let state = ApiServerState::new(vec!["old-token".into(), "secret-token".into()], 0);
        let headers = ApiHeaders {
            authorization: Some("Bearer secret-token".into()),
            ..ApiHeaders::default()
        };
        assert!(state.security_response("/v1/models", &headers).is_none());
        for route in ["/healthz", "/health", "/ready"] {
            assert!(
                state
                    .security_response(route, &ApiHeaders::default())
                    .is_none()
            );
        }
    }

    #[test]
    fn api_accepts_rotated_bearer_tokens() {
        let state = ApiServerState::new(vec!["old-token".into(), "new-token".into()], 0);
        for token in ["old-token", "new-token"] {
            let headers = ApiHeaders {
                authorization: Some(format!("Bearer {token}")),
                ..ApiHeaders::default()
            };
            assert!(state.security_response("/v1/models", &headers).is_none());
        }
    }

    #[test]
    fn api_rejects_wrong_bearer_token_without_echoing_secret() {
        let state = ApiServerState::new(vec!["secret-token".into()], 0);
        let headers = ApiHeaders {
            authorization: Some("Bearer wrong-token".into()),
            ..ApiHeaders::default()
        };
        let response = state
            .security_response("/v1/chat/completions", &headers)
            .expect("unauthorized");
        assert_eq!(response.status_code, 401);
        assert!(!response.body.contains("secret-token"));
        assert!(!response.body.contains("wrong-token"));
    }

    #[test]
    fn api_rate_limit_returns_retry_after() {
        let state = ApiServerState::new(Vec::new(), 1);
        assert!(
            state
                .security_response("/v1/models", &ApiHeaders::default())
                .is_none()
        );
        let response = state
            .security_response("/v1/models", &ApiHeaders::default())
            .expect("rate limited");
        assert_eq!(response.status_code, 429);
        assert!(
            response
                .extra_headers
                .iter()
                .any(|header| header.name == "Retry-After")
        );
    }

    #[test]
    fn api_request_audit_does_not_record_authorization_value() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = IkarosPaths::from_home(temp.path());
        let headers = ApiHeaders {
            authorization: Some("Bearer sk-secret-value".into()),
            client_id: Some("client-sk-secret-value\nterminal".into()),
            ..ApiHeaders::default()
        };
        let response = ApiHttpResponse::json_error(401, "Unauthorized", "invalid token")
            .with_session(ApiSessionIds {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
            });

        audit_api_request(&paths, None, "GET", "/v1/models", &response, Some(&headers));

        let raw = std::fs::read_to_string(paths.audit_dir.join("audit.jsonl")).expect("audit");
        assert!(raw.contains("\"kind\":\"api_request\""));
        assert!(raw.contains("\"authorization_present\":true"));
        assert!(raw.contains("\"client_id\":\"[REDACTED_SECRET] terminal\""));
        assert!(raw.contains("\"session_id\":\"session-1\""));
        assert!(raw.contains("\"turn_id\":\"turn-1\""));
        assert!(raw.contains("\"correlation_id\":\"session:session-1:turn:turn-1\""));
        assert!(!raw.contains("sk-secret-value"));
    }
}
