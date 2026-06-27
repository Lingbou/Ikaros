// SPDX-License-Identifier: GPL-3.0-only

use super::*;

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
    assert!(matches!(
        stream.events.first(),
        Some(ModelStreamEvent::Start { provider, model })
            if provider == "mock" && model == "mock-ikaros"
    ));
    let event_text = stream
        .events
        .iter()
        .filter_map(|event| match event {
            ModelStreamEvent::TextDelta(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(event_text.contains("[REDACTED_SECRET]"));
    assert!(
        stream
            .events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::Usage(_)))
    );
    assert!(matches!(stream.events.last(), Some(ModelStreamEvent::Done)));
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
async fn openai_provider_sends_chat_request_through_injected_http_client() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let http = Arc::new(CapturingHttpClient {
        seen: seen.clone(),
        status: 200,
        headers: BTreeMap::new(),
        body: r#"{"model":"test-model","choices":[{"message":{"content":"ok"}}],"usage":{"total_tokens":2}}"#.into(),
    });
    let config = ModelConfig {
        provider: "openai-compatible".into(),
        model: "test-model".into(),
        runtime: "harness-agent-loop".into(),
        transport: "openai-compatible-chat-completions".into(),
        ..ModelConfig::default()
    };
    let provider_settings = RemoteProviderConfig {
        api_key: "sk-secret".into(),
        base_url: "https://api.example/v1".into(),
    };
    let provider = OpenAiCompatibleProvider::from_config_with_http_client(
        "openai-compatible",
        &config,
        &provider_settings,
        http,
    )
    .expect("provider");

    let response = provider
        .generate(ModelRequest::from_user_text("hello"))
        .await
        .expect("response");

    assert_eq!(response.content, "ok");
    let requests = seen.lock().expect("seen");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].url, "https://api.example/v1/chat/completions");
    assert_eq!(
        requests[0].headers.get("authorization").map(String::as_str),
        Some("Bearer sk-secret")
    );
    let body: serde_json::Value = serde_json::from_str(&requests[0].body).expect("json");
    assert_eq!(body["model"], "test-model");
    assert_eq!(body["messages"][0]["content"], "hello");
}

#[tokio::test]
async fn openai_provider_stream_with_events_emits_incremental_sse_chunks_before_done() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let (first_chunk_seen_tx, first_chunk_seen_rx) = tokio::sync::oneshot::channel();
    let (release_second_chunk_tx, release_second_chunk_rx) = tokio::sync::oneshot::channel();
    let http = Arc::new(DelayedStreamingHttpClient {
        seen: seen.clone(),
        first_chunk_seen: Mutex::new(Some(first_chunk_seen_tx)),
        release_second_chunk: Mutex::new(Some(release_second_chunk_rx)),
    });
    let config = ModelConfig {
        provider: "openai-compatible".into(),
        model: "stream-model".into(),
        runtime: "harness-agent-loop".into(),
        transport: "openai-compatible-chat-completions".into(),
        ..ModelConfig::default()
    };
    let provider_settings = RemoteProviderConfig {
        api_key: "sk-secret".into(),
        base_url: "https://api.example/v1".into(),
    };
    let provider = OpenAiCompatibleProvider::from_config_with_http_client(
        "openai-compatible",
        &config,
        &provider_settings,
        http,
    )
    .expect("provider");
    let mut sink = RecordingModelStreamEventSink {
        events: events.clone(),
    };
    let task = tokio::spawn(async move {
        provider
            .stream_with_events(ModelRequest::from_user_text("hello"), &mut sink)
            .await
    });

    first_chunk_seen_rx.await.expect("first chunk observed");
    tokio::task::yield_now().await;
    let live_events = events.lock().expect("events").clone();
    assert!(
        live_events.iter().any(
            |event| matches!(event, ModelStreamEvent::TextDelta(text) if text.contains("Hello "))
        ),
        "first text delta should be emitted before the stream completes: {live_events:?}"
    );
    assert!(
        !live_events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::Done)),
        "done must not be emitted until the delayed second chunk is released"
    );
    assert!(
        !task.is_finished(),
        "provider future should still be waiting"
    );

    release_second_chunk_tx
        .send(())
        .expect("release second chunk");
    let stream = task.await.expect("join").expect("stream");
    assert_eq!(stream.content(), "Hello world");
    assert!(matches!(
        events.lock().expect("events").last(),
        Some(ModelStreamEvent::Done)
    ));

    let requests = seen.lock().expect("seen");
    let body: serde_json::Value = serde_json::from_str(&requests[0].body).expect("json");
    assert_eq!(body["stream"], true);
}

#[tokio::test]
async fn openai_provider_http_error_surfaces_retry_after_header_safely() {
    let http = Arc::new(CapturingHttpClient {
        seen: Arc::new(Mutex::new(Vec::new())),
        status: 429,
        headers: BTreeMap::from([
            ("retry-after".into(), "0".into()),
            ("set-cookie".into(), "session=sk-header-secret".into()),
        ]),
        body: r#"{"error":"provider echoed token=body-secret and sk-not-real"}"#.into(),
    });
    let config = ModelConfig {
        provider: "openai-compatible".into(),
        model: "test-model".into(),
        runtime: "harness-agent-loop".into(),
        transport: "openai-compatible-chat-completions".into(),
        ..ModelConfig::default()
    };
    let provider_settings = RemoteProviderConfig {
        api_key: "sk-secret".into(),
        base_url: "https://api.example/v1".into(),
    };
    let provider = OpenAiCompatibleProvider::from_config_with_http_client(
        "openai-compatible",
        &config,
        &provider_settings,
        http,
    )
    .expect("provider");

    let error = provider
        .generate(ModelRequest::from_user_text("hello"))
        .await
        .expect_err("429 should fail");
    let message = error.to_string();

    assert!(message.contains("HTTP 429"));
    assert!(message.contains("Retry-After: 0"));
    assert!(!message.contains("set-cookie"));
    assert!(!message.contains("sk-header-secret"));
    assert!(!message.contains("body-secret"));
    assert!(!message.contains("sk-not-real"));
    assert!(message.contains("[REDACTED_SECRET]"));
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

    let error = ModelProviderKind::parse("openai").expect_err("alias rejected");
    assert!(error.contains("unsupported ModelProviderKind `openai`"));
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
fn openai_compatible_profile_auto_detects_moonshot_kimi() {
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
    assert_eq!(provider.compat_profile_id(), "moonshot-kimi");
}

#[test]
fn provider_profile_catalog_resolves_without_enum_keys() {
    let specs = ProviderProfile::catalog();
    let ids = specs.iter().map(|spec| spec.id).collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            "generic",
            "moonshot-kimi",
            "deepseek",
            "gemini-openai",
            "openrouter",
            "qwen",
            "local-openai-compatible",
        ]
    );

    let qwen = ProviderProfile::resolve_profile_id(
        "qwen",
        "https://dashscope.aliyuncs.com/compatible-mode/v1",
        "qwen3-coder",
    )
    .expect("qwen profile by id");
    assert_eq!(qwen.id, "qwen");
    assert_eq!(
        qwen.message_policy,
        MessagePolicy::QwenTextPartsWithSystemCache
    );
    assert_eq!(qwen.default_max_tokens, Some(65_536));

    let auto = ProviderProfile::resolve_configured(
        "auto",
        "https://api.moonshot.cn/v1",
        "generic-router-model",
    )
    .expect("auto profile");
    assert_eq!(auto.id, "moonshot-kimi");

    let resolved = specs
        .iter()
        .map(|spec| {
            ProviderProfile::resolve_profile_id(spec.id, "https://api.example/v1", "model")
                .expect("profile spec should resolve by id")
                .id
        })
        .collect::<Vec<_>>();
    assert_eq!(resolved, ids);
}

#[test]
fn openai_compatible_provider_keeps_resolved_profile_decisions() {
    let config = ModelConfig {
        provider: "openai-compatible".into(),
        model: "kimi-k2.6".into(),
        compat_profile: "auto".into(),
        ..ModelConfig::default()
    };
    let provider_settings = RemoteProviderConfig {
        api_key: "test-key".into(),
        base_url: "https://api.moonshot.cn/v1".into(),
    };
    let provider =
        OpenAiCompatibleProvider::from_config("openai-compatible", &config, &provider_settings)
            .expect("provider");

    let profile = provider.resolved_profile();
    assert_eq!(profile.id, "moonshot-kimi");
    assert_eq!(profile.default_max_tokens, Some(32_000));
    assert_eq!(profile.temperature_policy, TemperaturePolicy::Omit);
    assert_eq!(profile.tool_schema_policy, ToolSchemaPolicy::MoonshotSubset);
    assert_eq!(provider.context_profile(), profile.context);
}

#[test]
fn openai_compatible_profiles_expose_structured_decisions() {
    let kimi = ProviderProfile::resolve_profile_id(
        "moonshot-kimi",
        "https://api.moonshot.cn/v1",
        "kimi-k2.6",
    )
    .expect("moonshot-kimi profile");
    assert_eq!(kimi.id, "moonshot-kimi");
    assert_eq!(kimi.default_max_tokens, Some(32_000));
    assert_eq!(kimi.temperature_policy, TemperaturePolicy::Omit);
    assert_eq!(kimi.reasoning_policy, ReasoningPolicy::MoonshotKimi);
    assert_eq!(kimi.message_policy, MessagePolicy::Plain);
    assert_eq!(kimi.tool_schema_policy, ToolSchemaPolicy::MoonshotSubset);
    assert_eq!(kimi.context.context_window, 128_000);

    let qwen = ProviderProfile::resolve_profile_id(
        "qwen",
        "https://dashscope.aliyuncs.com/compatible-mode/v1",
        "qwen3-coder",
    )
    .expect("qwen profile");
    assert_eq!(qwen.default_max_tokens, Some(65_536));
    assert_eq!(
        qwen.message_policy,
        MessagePolicy::QwenTextPartsWithSystemCache
    );
    assert_eq!(qwen.tool_schema_policy, ToolSchemaPolicy::PassThrough);

    let generic = ProviderProfile::resolve_profile_id(
        "generic",
        "https://api.example/v1",
        "generic-chat-32k",
    )
    .expect("generic profile");
    assert_eq!(generic.default_max_tokens, None);
    assert_eq!(generic.temperature_policy, TemperaturePolicy::PassThrough);
    assert_eq!(generic.context.context_window, 32_000);
}

#[test]
fn openai_compatible_profiles_resolve_directly_from_configured_value() {
    let auto = ProviderProfile::resolve_configured(
        "auto",
        "https://api.moonshot.cn/v1",
        "generic-router-model",
    )
    .expect("auto profile");
    assert_eq!(auto.id, "moonshot-kimi");
    assert_eq!(auto.temperature_policy, TemperaturePolicy::Omit);

    let explicit = ProviderProfile::resolve_configured("qwen", "https://api.example/v1", "model")
        .expect("explicit profile");
    assert_eq!(explicit.id, "qwen");
    assert_eq!(
        explicit.message_policy,
        MessagePolicy::QwenTextPartsWithSystemCache
    );

    let error = ProviderProfile::resolve_configured("missing", "https://api.example/v1", "model")
        .expect_err("unknown configured profile should fail");
    assert!(
        error
            .to_string()
            .contains("unsupported OpenAI-compatible profile")
    );
}

#[test]
fn openai_compatible_profile_catalog_is_static_and_complete() {
    let specs = ProviderProfile::catalog();
    let ids = specs.iter().map(|spec| spec.id).collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            "generic",
            "moonshot-kimi",
            "deepseek",
            "gemini-openai",
            "openrouter",
            "qwen",
            "local-openai-compatible",
        ]
    );

    let kimi = specs
        .iter()
        .find(|spec| spec.id == "moonshot-kimi")
        .expect("kimi profile spec");
    assert!(kimi.auto_base_url_markers.contains(&"api.moonshot.cn"));
    assert!(kimi.auto_model_markers.contains(&"kimi"));
    assert_eq!(kimi.default_max_tokens, Some(32_000));
    assert_eq!(kimi.temperature_policy, TemperaturePolicy::Omit);
    assert_eq!(kimi.reasoning_policy, ReasoningPolicy::MoonshotKimi);
    assert!(kimi.network_access);
    assert!(kimi.retry_without_parameters.is_empty());

    let qwen = specs
        .iter()
        .find(|spec| spec.id == "qwen")
        .expect("qwen profile spec");
    assert!(qwen.auto_base_url_markers.contains(&"dashscope"));
    assert_eq!(
        qwen.message_policy,
        MessagePolicy::QwenTextPartsWithSystemCache
    );
    assert_eq!(
        qwen.request_body_policy,
        RequestBodyPolicy::QwenHighResolutionImages
    );
    assert!(qwen.network_access);
    assert_eq!(qwen.retry_without_parameters, &["temperature"]);

    let generic = specs
        .iter()
        .find(|spec| spec.id == "generic")
        .expect("generic profile spec");
    assert_eq!(
        generic.retry_without_parameters,
        &["temperature", "max_tokens"]
    );
    assert!(generic.network_access);

    let local = specs
        .iter()
        .find(|spec| spec.id == "local-openai-compatible")
        .expect("local profile spec");
    assert!(!local.network_access);
    assert!(local.auto_base_url_markers.contains(&"127.0.0.1"));

    for spec in specs {
        if spec.id != "generic" {
            assert!(
                !spec.auto_base_url_markers.is_empty()
                    || !spec.auto_model_markers.is_empty()
                    || !spec.auto_model_tail_prefixes.is_empty(),
                "non-generic profile `{}` should declare catalog auto-detect hints",
                spec.id
            );
        }
        let resolved =
            ProviderProfile::resolve_profile_id(spec.id, "https://api.example/v1", "model")
                .expect("profile id should resolve");
        assert_eq!(resolved.id, spec.id);
        assert_eq!(resolved.temperature_policy, spec.temperature_policy);
        assert_eq!(resolved.reasoning_policy, spec.reasoning_policy);
        assert_eq!(resolved.message_policy, spec.message_policy);
        assert_eq!(resolved.request_body_policy, spec.request_body_policy);
        assert_eq!(resolved.network_access, spec.network_access);
        assert_eq!(
            resolved.retry_without_parameters,
            spec.retry_without_parameters
        );
    }
}

#[test]
fn openai_compatible_generic_profile_keeps_standard_chat_payload() {
    let provider = openai_provider_for_body(
        ModelConfig {
            provider: "openai-compatible".into(),
            model: "generic-chat".into(),
            compat_profile: "generic".into(),
            ..ModelConfig::default()
        },
        "https://api.example/v1",
    );
    let body = provider
        .test_chat_completion_body(
            ModelRequest {
                messages: vec![ModelMessage::user("hello")],
                options: ModelRequestOptions {
                    max_tokens: Some(64),
                    temperature: Some(0.4),
                    ..ModelRequestOptions::default()
                },
                tools: Vec::new(),
            },
            false,
        )
        .expect("body");

    assert_eq!(body["model"], "generic-chat");
    assert_eq!(body["max_tokens"], 64);
    assert!((body["temperature"].as_f64().expect("temperature") - 0.4).abs() < 1e-6);
    assert!(body.get("thinking").is_none());
    assert!(body.get("stream").is_none());
}

#[test]
fn openai_compatible_request_body_supports_image_content_blocks() {
    let provider = openai_provider_for_body(
        ModelConfig {
            provider: "openai-compatible".into(),
            model: "vision-chat".into(),
            compat_profile: "generic".into(),
            ..ModelConfig::default()
        },
        "https://api.example/v1",
    );
    let body = provider
        .test_chat_completion_body(
            ModelRequest {
                messages: vec![ModelMessage::user_with_content_blocks(vec![
                    ModelContentBlock::text("describe this image"),
                    ModelContentBlock::Image {
                        image_url: "https://images.example/cat.png".into(),
                        mime_type: Some("image/png".into()),
                        detail: Some("high".into()),
                    },
                ])],
                options: ModelRequestOptions::default(),
                tools: Vec::new(),
            },
            false,
        )
        .expect("body");

    assert_eq!(body["messages"][0]["content"][0]["type"], "text");
    assert_eq!(
        body["messages"][0]["content"][0]["text"],
        "describe this image"
    );
    assert_eq!(body["messages"][0]["content"][1]["type"], "image_url");
    assert_eq!(
        body["messages"][0]["content"][1]["image_url"]["url"],
        "https://images.example/cat.png"
    );
    assert_eq!(
        body["messages"][0]["content"][1]["image_url"]["detail"],
        "high"
    );
}

#[test]
fn openai_compatible_kimi_profile_omits_temperature_and_enables_thinking() {
    let provider = openai_provider_for_body(
        ModelConfig {
            provider: "openai-compatible".into(),
            model: "kimi-k2.6".into(),
            ..ModelConfig::default()
        },
        "https://api.moonshot.cn/v1",
    );
    let body = provider
        .test_chat_completion_body(
            ModelRequest {
                messages: vec![ModelMessage::user("hello")],
                options: ModelRequestOptions {
                    temperature: Some(0.4),
                    ..ModelRequestOptions::default()
                },
                tools: Vec::new(),
            },
            false,
        )
        .expect("body");

    assert_eq!(provider.compat_profile_id(), "moonshot-kimi");
    assert_eq!(body["max_tokens"], 32_000);
    assert!(body.get("temperature").is_none());
    assert_eq!(body["thinking"], serde_json::json!({"type": "enabled"}));
    assert!(body.get("reasoning_effort").is_none());
}

#[test]
fn openai_compatible_kimi_profile_uses_effort_xor_thinking() {
    let provider = openai_provider_for_body(
        ModelConfig {
            provider: "openai-compatible".into(),
            model: "kimi-k2.6".into(),
            reasoning: ModelReasoningConfig {
                enabled: Some(true),
                effort: Some("high".into()),
            },
            ..ModelConfig::default()
        },
        "https://api.moonshot.cn/v1",
    );
    let body = provider
        .test_chat_completion_body(ModelRequest::from_user_text("hello"), false)
        .expect("body");

    assert!(body.get("thinking").is_none());
    assert_eq!(body["reasoning_effort"], "high");
    assert!(body.get("temperature").is_none());
}

#[test]
fn openai_compatible_deepseek_profile_emits_thinking_for_v4_only() {
    let provider = openai_provider_for_body(
        ModelConfig {
            provider: "openai-compatible".into(),
            model: "deepseek-v4-flash".into(),
            reasoning: ModelReasoningConfig {
                enabled: Some(true),
                effort: Some("xhigh".into()),
            },
            ..ModelConfig::default()
        },
        "https://api.deepseek.com/v1",
    );
    let body = provider
        .test_chat_completion_body(ModelRequest::from_user_text("hello"), false)
        .expect("body");

    assert_eq!(provider.compat_profile_id(), "deepseek");
    assert_eq!(body["thinking"], serde_json::json!({"type": "enabled"}));
    assert_eq!(body["reasoning_effort"], "max");

    let chat_provider = openai_provider_for_body(
        ModelConfig {
            provider: "openai-compatible".into(),
            model: "deepseek-chat".into(),
            ..ModelConfig::default()
        },
        "https://api.deepseek.com/v1",
    );
    let chat_body = chat_provider
        .test_chat_completion_body(ModelRequest::from_user_text("hello"), false)
        .expect("body");
    assert!(chat_body.get("thinking").is_none());
    assert!(chat_body.get("reasoning_effort").is_none());
}

#[test]
fn openai_compatible_gemini_profile_maps_reasoning_for_gemini_models_only() {
    let mut extra_body = serde_json::Map::new();
    extra_body.insert(
        "extra_body".into(),
        serde_json::json!({
            "google": {"cached_content": "cache-1"},
            "session_id": "session-1"
        }),
    );
    let provider = openai_provider_for_body(
        ModelConfig {
            provider: "openai-compatible".into(),
            model: "gemini-3-flash".into(),
            reasoning: ModelReasoningConfig {
                enabled: Some(true),
                effort: Some("high".into()),
            },
            extra_body,
            ..ModelConfig::default()
        },
        "https://generativelanguage.googleapis.com/v1beta/openai",
    );
    let body = provider
        .test_chat_completion_body(ModelRequest::from_user_text("hello"), false)
        .expect("body");

    assert_eq!(provider.compat_profile_id(), "gemini-openai");
    assert_eq!(
        body["extra_body"]["google"]["thinking_config"],
        serde_json::json!({"include_thoughts": true, "thinking_level": "high"})
    );
    assert_eq!(
        body["extra_body"]["google"]["cached_content"],
        serde_json::json!("cache-1")
    );
    assert_eq!(
        body["extra_body"]["session_id"],
        serde_json::json!("session-1")
    );

    let gemma_provider = openai_provider_for_body(
        ModelConfig {
            provider: "openai-compatible".into(),
            model: "gemma-3".into(),
            reasoning: ModelReasoningConfig {
                enabled: Some(true),
                effort: Some("high".into()),
            },
            ..ModelConfig::default()
        },
        "https://generativelanguage.googleapis.com/v1beta/openai",
    );
    let gemma_body = gemma_provider
        .test_chat_completion_body(ModelRequest::from_user_text("hello"), false)
        .expect("body");
    assert!(gemma_body.get("extra_body").is_none());
}

#[test]
fn openai_compatible_openrouter_profile_avoids_invalid_claude_reasoning() {
    let mut extra_body = serde_json::Map::new();
    extra_body.insert(
        "reasoning".into(),
        serde_json::json!({"enabled": true, "effort": "high"}),
    );
    let provider = openai_provider_for_body(
        ModelConfig {
            provider: "openai-compatible".into(),
            model: "anthropic/claude-fable-4.6".into(),
            compat_profile: "openrouter".into(),
            reasoning: ModelReasoningConfig {
                enabled: Some(true),
                effort: Some("high".into()),
            },
            extra_body,
            ..ModelConfig::default()
        },
        "https://openrouter.ai/api/v1",
    );
    let body = provider
        .test_chat_completion_body(ModelRequest::from_user_text("hello"), false)
        .expect("body");

    assert_eq!(provider.compat_profile_id(), "openrouter");
    assert!(body.get("reasoning").is_none());
    assert_eq!(body["verbosity"], "high");
}

#[test]
fn openai_compatible_qwen_and_local_profiles_are_detected() {
    let qwen = openai_provider_for_body(
        ModelConfig {
            provider: "openai-compatible".into(),
            model: "qwen3-coder".into(),
            ..ModelConfig::default()
        },
        "https://dashscope.aliyuncs.com/compatible-mode/v1",
    );
    let qwen_body = qwen
        .test_chat_completion_body(
            ModelRequest {
                messages: vec![
                    ModelMessage::system("follow policy"),
                    ModelMessage::user("hello"),
                ],
                options: ModelRequestOptions::default(),
                tools: Vec::new(),
            },
            false,
        )
        .expect("body");
    assert_eq!(qwen.compat_profile_id(), "qwen");
    assert_eq!(qwen_body["max_tokens"], 65_536);
    assert_eq!(qwen_body["vl_high_resolution_images"], true);
    assert_eq!(
        qwen_body["messages"][0]["content"][0],
        serde_json::json!({
            "type": "text",
            "text": "follow policy",
            "cache_control": {"type": "ephemeral"}
        })
    );
    assert_eq!(
        qwen_body["messages"][1]["content"][0],
        serde_json::json!({"type": "text", "text": "hello"})
    );

    let local = openai_provider_for_body(
        ModelConfig {
            provider: "openai-compatible".into(),
            model: "local-model".into(),
            ..ModelConfig::default()
        },
        "http://127.0.0.1:1234/v1",
    );
    let local_body = local
        .test_chat_completion_body(ModelRequest::from_user_text("hello"), false)
        .expect("body");
    assert_eq!(local.compat_profile_id(), "local-openai-compatible");
    assert_eq!(local_body["max_tokens"], 65_536);
}

#[test]
fn openai_compatible_unsupported_parameter_retry_is_narrow() {
    let body = serde_json::json!({
        "model": "example",
        "messages": [],
        "temperature": 0.4,
        "max_tokens": 64
    });
    let generic =
        ProviderProfile::resolve_profile_id("generic", "https://api.example/v1", "example")
            .expect("generic profile");
    let moonshot = ProviderProfile::resolve_profile_id(
        "moonshot-kimi",
        "https://api.moonshot.cn/v1",
        "kimi-k2.6",
    )
    .expect("moonshot-kimi profile");
    assert_eq!(
        unsupported_parameter_to_omit(
            &generic,
            r#"{"error":{"code":"unsupported_parameter","message":"Unsupported parameter: 'temperature'"}}"#,
            &body,
        ),
        Some("temperature")
    );
    assert_eq!(
        unsupported_parameter_to_omit(
            &moonshot,
            r#"{"error":{"code":"unsupported_parameter","message":"Unsupported parameter: 'max_tokens'"}}"#,
            &body,
        ),
        None
    );
    assert_eq!(
        unsupported_parameter_to_omit(
            &generic,
            r#"{"error":{"message":"temperature must be between 0 and 2"}}"#,
            &body,
        ),
        None
    );
}

#[test]
fn openai_compatible_kimi_sanitizes_tool_schema_without_mutating_registry_shape() {
    let provider = openai_provider_for_body(
        ModelConfig {
            provider: "openai-compatible".into(),
            model: "kimi-k2.6".into(),
            ..ModelConfig::default()
        },
        "https://api.moonshot.cn/v1",
    );
    let original_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "query": {
                "anyOf": [
                    {"type": "string", "enum": ["news", null, ""]},
                    {"type": "null"}
                ],
                "nullable": true
            },
            "limit": {
                "title": "Limit",
                "description": "missing type",
                "minimum": 1,
                "maximum": 20,
                "format": "int32"
            },
            "mode": {
                "oneOf": [
                    {"type": "string", "enum": ["fast", "safe"]},
                    {"type": "integer", "minimum": 0}
                ]
            }
        }
    });
    let body = provider
        .test_chat_completion_body(
            ModelRequest {
                messages: vec![ModelMessage::user("search")],
                options: ModelRequestOptions::default(),
                tools: vec![ModelToolDefinition {
                    name: "memory_search".into(),
                    description: "Search".into(),
                    input_schema: original_schema.clone(),
                }],
            },
            false,
        )
        .expect("body");
    let params = &body["tools"][0]["function"]["parameters"];

    assert_eq!(params["properties"]["query"]["type"], "string");
    assert_eq!(
        params["properties"]["query"]["enum"],
        serde_json::json!(["news"])
    );
    assert_eq!(params["properties"]["limit"]["type"], "string");
    assert!(params["properties"]["limit"].get("title").is_none());
    assert!(params["properties"]["limit"].get("minimum").is_none());
    assert!(params["properties"]["limit"].get("maximum").is_none());
    assert!(params["properties"]["limit"].get("format").is_none());
    assert!(params["properties"]["mode"].get("oneOf").is_none());
    assert!(params["properties"]["mode"].get("anyOf").is_some());
    assert!(
        params["properties"]["mode"]["anyOf"][1]
            .get("minimum")
            .is_none()
    );
    assert!(original_schema["properties"]["limit"].get("type").is_none());
    assert!(
        original_schema["properties"]["limit"]
            .get("minimum")
            .is_some()
    );
    assert!(original_schema["properties"]["mode"].get("oneOf").is_some());
}

fn openai_provider_for_body(config: ModelConfig, base_url: &str) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::from_config(
        "openai-compatible",
        &config,
        &RemoteProviderConfig {
            api_key: "test-key".into(),
            base_url: base_url.into(),
        },
    )
    .expect("provider")
}
