// SPDX-License-Identifier: GPL-3.0-only

use super::{
    client::OpenAiCompatibleProvider, request_builder::build_chat_completion_request,
    stream::parse_stream_response, tools::model_tool_calls, types::ChatCompletionResponse,
};
use crate::params::merge_request_options;
use crate::types::{
    ModelContextProfile, ModelProvider, ModelRequest, ModelRequestDiagnostic, ModelResponse,
    ModelStream,
};
use async_trait::async_trait;
use ikaros_core::{IkarosError, Result, redact_secrets};
use serde_json::Value;

#[async_trait]
impl ModelProvider for OpenAiCompatibleProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn estimate_request_tokens(&self, request: &ModelRequest) -> u32 {
        let options = merge_request_options(&self.default_options, &request.options);
        let output_tokens = options
            .max_tokens
            .or_else(|| self.profile.default_max_tokens(&self.model));
        request.estimated_tokens_with_output_limit(output_tokens)
    }

    fn context_profile(&self) -> ModelContextProfile {
        self.profile.context_profile(&self.model)
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        let key = self.api_key()?;
        let request = request.redacted();
        let prepared = build_chat_completion_request(
            &self.model,
            &self.base_url,
            self.profile,
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
            let result = self
                .client
                .post(&url)
                .bearer_auth(&key)
                .json(&body)
                .send()
                .await;
            let attempt_error = match result {
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.map_err(|source| {
                        IkarosError::Message(format!("failed to read model response: {source}"))
                    })?;
                    if !status.is_success() {
                        if !unsupported_parameter_retry_used {
                            if let Some(parameter) =
                                unsupported_parameter_to_omit(self.profile, &text, &body)
                            {
                                remove_body_parameter(&mut body, parameter);
                                diagnostics.push(unsupported_parameter_retry_diagnostic(parameter));
                                unsupported_parameter_retry_used = true;
                                continue;
                            }
                        }
                        redacted_model_http_error(status, &text)
                    } else {
                        let mut response =
                            parse_chat_completion_response(&text, &self.name, &self.model)?;
                        response.diagnostics = diagnostics;
                        return Ok(response);
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
        let key = self.api_key()?;
        let request = request.redacted();
        let prepared = build_chat_completion_request(
            &self.model,
            &self.base_url,
            self.profile,
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
                .client
                .post(&url)
                .bearer_auth(&key)
                .json(&body)
                .send()
                .await;
            let attempt_error = match result {
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.map_err(|source| {
                        IkarosError::Message(format!(
                            "failed to read model stream response: {source}"
                        ))
                    })?;
                    if !status.is_success() {
                        if !unsupported_parameter_retry_used {
                            if let Some(parameter) =
                                unsupported_parameter_to_omit(self.profile, &text, &body)
                            {
                                remove_body_parameter(&mut body, parameter);
                                diagnostics.push(unsupported_parameter_retry_diagnostic(parameter));
                                unsupported_parameter_retry_used = true;
                                continue;
                            }
                        }
                        redacted_model_http_error(status, &text)
                    } else {
                        match parse_stream_response(&text, &self.name, &self.model) {
                            Ok(mut stream) => {
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

fn unsupported_parameter_retry_diagnostic(parameter: &str) -> ModelRequestDiagnostic {
    ModelRequestDiagnostic {
        kind: "unsupported_parameter_retry".into(),
        message: "provider rejected an unsupported request parameter; retried once without it"
            .into(),
        parameter: Some(parameter.into()),
    }
}

pub(crate) fn redacted_model_http_error(status: reqwest::StatusCode, text: &str) -> String {
    format!(
        "model provider returned HTTP {status}: {}",
        redact_secrets(text)
    )
}

pub(crate) fn unsupported_parameter_to_omit(
    profile: super::profile::OpenAiCompatProfile,
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
