// SPDX-License-Identifier: GPL-3.0-only

#![cfg(test)]

pub(super) use super::*;
pub(super) use crate::anthropic::{
    parse_messages_response, parse_stream_response as parse_anthropic_stream_response,
    test_messages_request_body, test_model_stream_events_from_response,
};
pub(super) use crate::ollama::{
    parse_chat_response as parse_ollama_chat_response,
    parse_stream_response as parse_ollama_stream_response, test_chat_request_body,
};
pub(super) use crate::openai_compatible::{
    MessagePolicy, ProviderProfile, ReasoningPolicy, RequestBodyPolicy, TemperaturePolicy,
    ToolSchemaPolicy, parse_chat_completion_response, parse_stream_response,
    redacted_model_http_error, unsupported_parameter_to_omit,
};
pub(super) use async_trait::async_trait;
pub(super) use futures_util::stream;
pub(super) use ikaros_core::{
    IkarosError, ModelConfig, ModelProviderKind, ModelReasoningConfig, RemoteProviderConfig, Result,
};
pub(super) use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};
pub(super) use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
pub(super) use tracing_subscriber::{
    Layer, Registry,
    layer::{Context as TraceLayerContext, SubscriberExt},
};

#[derive(Debug, Clone, Default)]
pub(super) struct RecordingTracingLayer {
    events: Arc<Mutex<Vec<RecordedTracingEvent>>>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RecordedTracingEvent {
    target: String,
    name: String,
    fields: BTreeMap<String, String>,
}

impl RecordingTracingLayer {
    fn events(&self) -> Arc<Mutex<Vec<RecordedTracingEvent>>> {
        self.events.clone()
    }
}

impl<S> Layer<S> for RecordingTracingLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: TraceLayerContext<'_, S>) {
        let mut visitor = RecordingFieldVisitor::default();
        event.record(&mut visitor);
        self.events
            .lock()
            .expect("trace events")
            .push(RecordedTracingEvent {
                target: event.metadata().target().to_owned(),
                name: event.metadata().name().to_owned(),
                fields: visitor.fields,
            });
    }
}

#[derive(Default)]
pub(super) struct RecordingFieldVisitor {
    fields: BTreeMap<String, String>,
}

static TRACE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static TRACE_EVENTS: OnceLock<Arc<Mutex<Vec<RecordedTracingEvent>>>> = OnceLock::new();

pub(super) fn install_test_tracing_recorder() -> Arc<Mutex<Vec<RecordedTracingEvent>>> {
    TRACE_EVENTS
        .get_or_init(|| {
            let layer = RecordingTracingLayer::default();
            let events = layer.events();
            tracing::subscriber::set_global_default(Registry::default().with(layer))
                .expect("install test tracing subscriber");
            tracing::callsite::rebuild_interest_cache();
            events
        })
        .clone()
}

impl Visit for RecordingFieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), value.to_owned());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }
}

pub(super) struct CapturingProvider {
    seen: Arc<Mutex<Option<ModelRequest>>>,
}

pub(super) struct FlakyProvider {
    attempts: Arc<AtomicUsize>,
    first_error: String,
}

pub(super) struct AlwaysFailProvider;

#[derive(Clone)]
pub(super) struct CapturingHttpClient {
    seen: Arc<Mutex<Vec<ModelHttpRequest>>>,
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

pub(super) struct DelayedStreamingHttpClient {
    pub(super) seen: Arc<Mutex<Vec<ModelHttpRequest>>>,
    pub(super) first_chunk_seen: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    pub(super) release_second_chunk: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

pub(super) struct RecordingModelStreamEventSink {
    pub(super) events: Arc<Mutex<Vec<ModelStreamEvent>>>,
}

pub(super) fn model_stream_event_kinds(events: &[ModelStreamEvent]) -> Vec<&'static str> {
    events
        .iter()
        .map(|event| match event {
            ModelStreamEvent::Start { .. } => "start",
            ModelStreamEvent::TextDelta(_) => "text_delta",
            ModelStreamEvent::ReasoningDelta(_) => "reasoning_delta",
            ModelStreamEvent::ToolCallStart { .. } => "tool_call_start",
            ModelStreamEvent::ToolCallDelta { .. } => "tool_call_delta",
            ModelStreamEvent::ToolCallEnd { .. } => "tool_call_end",
            ModelStreamEvent::RefusalDelta(_) => "refusal_delta",
            ModelStreamEvent::Usage(_) => "usage",
            ModelStreamEvent::Error { .. } => "error",
            ModelStreamEvent::Done => "done",
        })
        .collect()
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
                ..TokenUsage::default()
            },
            diagnostics: Vec::new(),
        })
    }
}

#[async_trait]
impl ModelProvider for FlakyProvider {
    fn name(&self) -> &str {
        "flaky"
    }

    fn model_id(&self) -> &str {
        "flaky-model"
    }

    async fn generate(&self, _request: ModelRequest) -> Result<ModelResponse> {
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
        if attempt == 0 {
            return Err(IkarosError::Message(self.first_error.clone()));
        }
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "test".into(),
            content: "ok".into(),
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
            diagnostics: Vec::new(),
        })
    }
}

#[async_trait]
impl ModelProvider for AlwaysFailProvider {
    fn name(&self) -> &str {
        "always-fail"
    }

    fn model_id(&self) -> &str {
        "fail-model"
    }

    async fn generate(&self, _request: ModelRequest) -> Result<ModelResponse> {
        Err(IkarosError::Message(
            "provider returned 503 sk-secret".into(),
        ))
    }
}

impl ModelHttpClient for CapturingHttpClient {
    fn send<'a>(
        &'a self,
        request: ModelHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ModelHttpResponse>> + Send + 'a>> {
        let seen = self.seen.clone();
        let status = self.status;
        let headers = self.headers.clone();
        let body = self.body.clone();
        Box::pin(async move {
            seen.lock()
                .map_err(|_| IkarosError::Message("http capture lock poisoned".into()))?
                .push(request);
            Ok(ModelHttpResponse {
                status,
                headers,
                body,
            })
        })
    }
}

impl ModelStreamEventSink for RecordingModelStreamEventSink {
    fn emit(&mut self, event: ModelStreamEvent) -> Result<()> {
        self.events
            .lock()
            .map_err(|_| IkarosError::Message("stream event lock poisoned".into()))?
            .push(event);
        Ok(())
    }
}

impl ModelHttpClient for DelayedStreamingHttpClient {
    fn send<'a>(
        &'a self,
        request: ModelHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ModelHttpResponse>> + Send + 'a>> {
        let seen = self.seen.clone();
        Box::pin(async move {
            seen.lock()
                .map_err(|_| IkarosError::Message("http capture lock poisoned".into()))?
                .push(request);
            Ok(ModelHttpResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: [
                    r#"data: {"model":"stream-model","choices":[{"delta":{"content":"Hello "}}]}"#,
                    "",
                    r#"data: {"model":"stream-model","choices":[{"delta":{"content":"world"}}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
                    "",
                    "data: [DONE]",
                    "",
                ]
                .join("\n"),
            })
        })
    }

    fn send_stream<'a>(
        &'a self,
        request: ModelHttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ModelHttpStreamResponse>> + Send + 'a>> {
        let seen = self.seen.clone();
        let first_chunk_seen = self
            .first_chunk_seen
            .lock()
            .expect("first chunk sender")
            .take();
        let release_second_chunk = self
            .release_second_chunk
            .lock()
            .expect("release second chunk receiver")
            .take();
        Box::pin(async move {
            seen.lock()
                .map_err(|_| IkarosError::Message("http capture lock poisoned".into()))?
                .push(request);
            let first =
                r#"data: {"model":"stream-model","choices":[{"delta":{"content":"Hello "}}]}

"#
                .as_bytes()
                .to_vec();
            let second = r#"data: {"model":"stream-model","choices":[{"delta":{"content":"world"}}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}

data: [DONE]

"#
            .as_bytes()
            .to_vec();
            let body = stream::unfold(
                (
                    0usize,
                    Some(first),
                    Some(second),
                    first_chunk_seen,
                    release_second_chunk,
                ),
                |(step, mut first, mut second, first_chunk_seen, mut release_second_chunk)| async move {
                    match step {
                        0 => {
                            if let Some(sender) = first_chunk_seen {
                                let _ = sender.send(());
                            }
                            Some((
                                Ok(first.take().expect("first chunk")),
                                (1, None, second, None, release_second_chunk),
                            ))
                        }
                        1 => {
                            if let Some(receiver) = release_second_chunk.take() {
                                let _ = receiver.await;
                            }
                            Some((
                                Ok(second.take().expect("second chunk")),
                                (2, None, None, None, None),
                            ))
                        }
                        _ => None,
                    }
                },
            );
            Ok(ModelHttpStreamResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: Box::pin(body),
            })
        })
    }
}

mod ollama;
mod provider_profiles;
mod request_bodies;
mod retry_fallback;
mod streaming;
mod usage_health;
