// SPDX-License-Identifier: GPL-3.0-only

use super::{
    client::OpenAiCompatibleProvider, request_builder::build_chat_completion_request,
    stream::OpenAiStreamAccumulator, tools::model_tool_calls, types::ChatCompletionResponse,
};
use crate::http::{ModelHttpRequest, ModelHttpStreamResponse};
use crate::params::merge_request_options;
use crate::types::{
    ModelContextProfile, ModelProvider, ModelProviderCapabilities, ModelRequest,
    ModelRequestDiagnostic, ModelResponse, ModelStream, ModelStreamEventSink,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use ikaros_core::{IkarosError, Result, redact_secrets};
use serde_json::Value;
use std::collections::BTreeMap;

#[async_trait]
impl ModelProvider for OpenAiCompatibleProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn estimate_request_tokens(&self, request: &ModelRequest) -> u32 {
        let options = merge_request_options(&self.default_options, &request.options);
        let output_tokens = options.max_tokens.or(self.profile.default_max_tokens);
        request.estimated_tokens_with_output_limit(output_tokens)
    }

    fn context_profile(&self) -> ModelContextProfile {
        self.profile.context.clone()
    }

    fn capabilities(&self) -> ModelProviderCapabilities {
        ModelProviderCapabilities {
            chat: true,
            streaming: true,
            tool_calls: true,
            reasoning: !matches!(self.profile.reasoning_policy, super::ReasoningPolicy::None),
            json_mode: true,
            network: self.profile.network_access,
            image_input: true,
            audio_input: true,
            file_input: true,
        }
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        let key = self.api_key()?;
        let request = request.redacted();
        let prepared = build_chat_completion_request(
            &self.model,
            &self.base_url,
            &self.profile,
            &self.default_options,
            request,
            false,
        )?;
        let mut body = prepared.body;
        let _profile_id = prepared.profile_id;
        let url = self.chat_completions_url();
        let mut attempt = 0usize;
        let max_attempts = self.max_retries as usize + 1;
        let mut unsupported_parameter_retry_used = false;
        let mut diagnostics = Vec::new();
        loop {
            let result = self.http.send(model_http_post(&url, &key, &body)?).await;
            let attempt_error = match result {
                Ok(response) => {
                    let status = response.status;
                    let headers = response.headers.clone();
                    let text = response.body;
                    if !(200..=299).contains(&status) {
                        if !unsupported_parameter_retry_used {
                            if let Some(parameter) =
                                unsupported_parameter_to_omit(&self.profile, &text, &body)
                            {
                                remove_body_parameter(&mut body, parameter);
                                diagnostics.push(unsupported_parameter_retry_diagnostic(parameter));
                                unsupported_parameter_retry_used = true;
                                continue;
                            }
                        }
                        redacted_model_http_error(status, &headers, &text)
                    } else {
                        let mut parsed =
                            parse_chat_completion_response(&text, &self.name, &self.model)?;
                        diagnostics.extend(
                            parsed
                                .diagnostics
                                .into_iter()
                                .map(ModelRequestDiagnostic::sanitized),
                        );
                        parsed.diagnostics = diagnostics;
                        return Ok(parsed);
                    }
                }
                Err(source) => format!("model request failed on attempt {attempt}: {source}"),
            };
            attempt += 1;
            if attempt >= max_attempts {
                return Err(IkarosError::Message(attempt_error));
            }
        }
    }

    async fn stream(&self, request: ModelRequest) -> Result<ModelStream> {
        let mut sink = crate::types::NoopModelStreamEventSink;
        self.stream_with_events(request, &mut sink).await
    }

    async fn stream_with_events(
        &self,
        request: ModelRequest,
        event_sink: &mut dyn ModelStreamEventSink,
    ) -> Result<ModelStream> {
        let key = self.api_key()?;
        let request = request.redacted();
        let prepared = build_chat_completion_request(
            &self.model,
            &self.base_url,
            &self.profile,
            &self.default_options,
            request,
            true,
        )?;
        let mut body = prepared.body;
        let _profile_id = prepared.profile_id;
        let url = self.chat_completions_url();
        let mut attempt = 0usize;
        let max_attempts = self.max_retries as usize + 1;
        let mut unsupported_parameter_retry_used = false;
        let mut diagnostics = Vec::new();
        loop {
            let result = self
                .http
                .send_stream(model_http_post(&url, &key, &body)?)
                .await;
            let attempt_error = match result {
                Ok(response) => {
                    let status = response.status;
                    let headers = response.headers.clone();
                    if !(200..=299).contains(&status) {
                        let text = read_model_http_stream_body(response).await?;
                        if !unsupported_parameter_retry_used {
                            if let Some(parameter) =
                                unsupported_parameter_to_omit(&self.profile, &text, &body)
                            {
                                remove_body_parameter(&mut body, parameter);
                                diagnostics.push(unsupported_parameter_retry_diagnostic(parameter));
                                unsupported_parameter_retry_used = true;
                                continue;
                            }
                        }
                        redacted_model_http_error(status, &headers, &text)
                    } else {
                        match parse_stream_response_body(
                            response,
                            &self.name,
                            &self.model,
                            event_sink,
                        )
                        .await
                        {
                            Ok(mut stream) => {
                                diagnostics.extend(
                                    stream
                                        .diagnostics
                                        .into_iter()
                                        .map(ModelRequestDiagnostic::sanitized),
                                );
                                stream.diagnostics = diagnostics;
                                return Ok(stream);
                            }
                            Err(error) => {
                                format!(
                                    "failed to parse model stream on attempt {attempt}: {error}"
                                )
                            }
                        }
                    }
                }
                Err(source) => {
                    format!("model stream request failed on attempt {attempt}: {source}")
                }
            };
            attempt += 1;
            if attempt >= max_attempts {
                return Err(IkarosError::Message(attempt_error));
            }
        }
    }
}

async fn parse_stream_response_body(
    mut response: ModelHttpStreamResponse,
    provider: &str,
    fallback_model: &str,
    event_sink: &mut dyn ModelStreamEventSink,
) -> Result<ModelStream> {
    let mut accumulator = OpenAiStreamAccumulator::new(provider, fallback_model);
    let mut pending_utf8 = Vec::<u8>::new();
    while let Some(chunk) = response.body.next().await {
        if let Some(text) = decode_utf8_stream_chunk(&mut pending_utf8, chunk?)? {
            accumulator.push_text(&text, event_sink)?;
        }
    }
    if !pending_utf8.is_empty() {
        let text = String::from_utf8(std::mem::take(&mut pending_utf8)).map_err(|source| {
            IkarosError::Message(format!(
                "failed to decode model stream response as UTF-8: {source}"
            ))
        })?;
        accumulator.push_text(&text, event_sink)?;
    }
    accumulator.finish(event_sink)
}

async fn read_model_http_stream_body(mut response: ModelHttpStreamResponse) -> Result<String> {
    let mut body = String::new();
    let mut pending_utf8 = Vec::<u8>::new();
    while let Some(chunk) = response.body.next().await {
        if let Some(text) = decode_utf8_stream_chunk(&mut pending_utf8, chunk?)? {
            body.push_str(&text);
        }
    }
    if !pending_utf8.is_empty() {
        let text = String::from_utf8(std::mem::take(&mut pending_utf8)).map_err(|source| {
            IkarosError::Message(format!(
                "failed to decode model stream error response as UTF-8: {source}"
            ))
        })?;
        body.push_str(&text);
    }
    Ok(body)
}

fn decode_utf8_stream_chunk(pending: &mut Vec<u8>, chunk: Vec<u8>) -> Result<Option<String>> {
    pending.extend(chunk);
    match String::from_utf8(std::mem::take(pending)) {
        Ok(text) => Ok((!text.is_empty()).then_some(text)),
        Err(error) => {
            let utf8_error = error.utf8_error();
            let valid_up_to = utf8_error.valid_up_to();
            if utf8_error.error_len().is_some() {
                return Err(IkarosError::Message(format!(
                    "failed to decode model stream response as UTF-8: {utf8_error}"
                )));
            }
            let bytes = error.into_bytes();
            let valid = if valid_up_to > 0 {
                Some(
                    String::from_utf8(bytes[..valid_up_to].to_vec()).map_err(|source| {
                        IkarosError::Message(format!(
                            "failed to decode model stream response as UTF-8: {source}"
                        ))
                    })?,
                )
            } else {
                None
            };
            pending.extend_from_slice(&bytes[valid_up_to..]);
            Ok(valid.filter(|text| !text.is_empty()))
        }
    }
}

fn model_http_post(url: &str, key: &str, body: &Value) -> Result<ModelHttpRequest> {
    let mut headers = BTreeMap::new();
    headers.insert("authorization".into(), format!("Bearer {key}"));
    headers.insert("content-type".into(), "application/json".into());
    Ok(ModelHttpRequest {
        method: "POST".into(),
        url: url.into(),
        headers,
        body: serde_json::to_string(body).map_err(|source| {
            IkarosError::Message(format!("failed to serialize model request JSON: {source}"))
        })?,
    })
}

fn unsupported_parameter_retry_diagnostic(parameter: &str) -> ModelRequestDiagnostic {
    ModelRequestDiagnostic::new(
        "unsupported_parameter_retry",
        "provider rejected an unsupported request parameter; retried once without it",
        Some(parameter.into()),
    )
}

pub(crate) fn redacted_model_http_error(
    status: u16,
    headers: &BTreeMap<String, String>,
    text: &str,
) -> String {
    let retry_after = retry_after_error_context(headers)
        .map(|value| format!(" {value}"))
        .unwrap_or_default();
    format!(
        "model provider returned HTTP {status}{retry_after}: {}",
        redact_secrets(text)
    )
}

fn retry_after_error_context(headers: &BTreeMap<String, String>) -> Option<String> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
        .map(|(_, value)| format!("Retry-After: {}", sanitized_header_value(value)))
}

fn sanitized_header_value(value: &str) -> String {
    let one_line = value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>();
    redact_secrets(one_line.trim())
}

pub(crate) fn unsupported_parameter_to_omit(
    profile: &super::profile::ProviderProfile,
    text: &str,
    body: &Value,
) -> Option<&'static str> {
    ["temperature", "max_tokens"].into_iter().find(|parameter| {
        body.get(parameter).is_some()
            && profile.can_retry_without_parameter(parameter)
            && response_reports_unsupported_parameter(text, parameter)
    })
}

fn response_reports_unsupported_parameter(text: &str, parameter: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if !lower.contains(parameter) {
        return false;
    }
    [
        "unsupported_parameter",
        "unsupported parameter",
        "unsupported",
        "unknown parameter",
        "unknown field",
        "unrecognized",
        "not support",
        "does not support",
        "extra inputs are not permitted",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn remove_body_parameter(body: &mut Value, parameter: &str) {
    if let Value::Object(object) = body {
        object.remove(parameter);
    }
}

pub(crate) fn parse_chat_completion_response(
    text: &str,
    provider: &str,
    fallback_model: &str,
) -> Result<ModelResponse> {
    let parsed: ChatCompletionResponse = serde_json::from_str(text).map_err(|source| {
        IkarosError::Message(format!("failed to parse model response JSON: {source}"))
    })?;
    let content = parsed
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone())
        .unwrap_or_default();
    let tool_calls = parsed
        .choices
        .first()
        .map(|choice| model_tool_calls(&choice.message.tool_calls))
        .unwrap_or_default();
    Ok(ModelResponse {
        provider: provider.into(),
        model: parsed.model.unwrap_or_else(|| fallback_model.into()),
        content: redact_secrets(&content),
        tool_calls,
        usage: parsed.usage.unwrap_or_default(),
        diagnostics: Vec::new(),
    })
}
