// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct TokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u32>,
}

impl TokenUsage {
    pub fn total_or_prompt_completion(&self) -> u32 {
        self.total_tokens.unwrap_or_else(|| {
            self.prompt_tokens
                .unwrap_or_default()
                .saturating_add(self.completion_tokens.unwrap_or_default())
        })
    }
}

impl<'de> Deserialize<'de> for TokenUsage {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize, Default)]
        struct TokenUsageWire {
            prompt_tokens: Option<u32>,
            completion_tokens: Option<u32>,
            total_tokens: Option<u32>,
            #[serde(default, alias = "cache_read_input_tokens")]
            cache_read_tokens: Option<u32>,
            #[serde(default, alias = "cache_creation_input_tokens")]
            cache_write_tokens: Option<u32>,
            #[serde(default)]
            prompt_tokens_details: Option<PromptTokensDetailsWire>,
        }

        #[derive(Deserialize, Default)]
        struct PromptTokensDetailsWire {
            cached_tokens: Option<u32>,
        }

        let wire = TokenUsageWire::deserialize(deserializer)?;
        Ok(Self {
            prompt_tokens: wire.prompt_tokens,
            completion_tokens: wire.completion_tokens,
            total_tokens: wire.total_tokens,
            cache_read_tokens: wire.cache_read_tokens.or_else(|| {
                wire.prompt_tokens_details
                    .and_then(|details| details.cached_tokens)
            }),
            cache_write_tokens: wire.cache_write_tokens,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ModelStreamEvent {
    Start { provider: String, model: String },
    TextDelta(String),
    ReasoningDelta(String),
    ToolCallStart { id: String, name: String },
    ToolCallDelta { id: String, args_delta: String },
    ToolCallEnd { id: String },
    RefusalDelta(String),
    Usage(TokenUsage),
    Error { message: String },
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelRequestDiagnostic {
    pub kind: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter: Option<String>,
}

pub const MODEL_REQUEST_DIAGNOSTIC_KIND_MAX_CHARS: usize = 96;
pub const MODEL_REQUEST_DIAGNOSTIC_MESSAGE_MAX_CHARS: usize = 512;
pub const MODEL_REQUEST_DIAGNOSTIC_PARAMETER_MAX_CHARS: usize = 128;

impl ModelRequestDiagnostic {
    pub fn new(
        kind: impl Into<String>,
        message: impl Into<String>,
        parameter: Option<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
            parameter,
        }
        .sanitized()
    }

    pub fn sanitized(mut self) -> Self {
        self.kind = sanitize_diagnostic_field(&self.kind, MODEL_REQUEST_DIAGNOSTIC_KIND_MAX_CHARS);
        self.message =
            sanitize_diagnostic_field(&self.message, MODEL_REQUEST_DIAGNOSTIC_MESSAGE_MAX_CHARS);
        self.parameter = self
            .parameter
            .take()
            .map(|parameter| {
                sanitize_diagnostic_field(&parameter, MODEL_REQUEST_DIAGNOSTIC_PARAMETER_MAX_CHARS)
            })
            .filter(|parameter| !parameter.is_empty());
        self
    }
}

fn sanitize_diagnostic_field(value: &str, max_chars: usize) -> String {
    let redacted = redact_secrets(value);
    let normalized = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&normalized, max_chars)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    const MARKER: &str = "...[truncated]";
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let marker_len = MARKER.chars().count();
    if max_chars <= marker_len {
        return MARKER.chars().take(max_chars).collect();
    }
    let keep = max_chars - marker_len;
    let mut truncated = value.chars().take(keep).collect::<String>();
    truncated.push_str(MARKER);
    truncated
}

fn redact_secrets(input: &str) -> String {
    let mut output = String::new();
    let mut token = String::new();
    for ch in input.chars() {
        if ch.is_whitespace() {
            push_redacted_token(&mut output, &token);
            token.clear();
            output.push(ch);
        } else {
            token.push(ch);
        }
    }
    push_redacted_token(&mut output, &token);
    output
}

fn push_redacted_token(output: &mut String, token: &str) {
    if token.is_empty() {
        return;
    }
    let is_assignment_secret = token
        .split_once('=')
        .is_some_and(|(key, _)| is_secret_assignment_key(key));
    if token.contains("sk-") {
        output.push_str("[REDACTED_SECRET]");
    } else if is_assignment_secret {
        let key = token.split_once('=').map_or(token, |(key, _)| key);
        let _ = write!(output, "{key}=[REDACTED_SECRET]");
    } else {
        output.push_str(token);
    }
}

fn is_secret_assignment_key(key: &str) -> bool {
    let normalized: String = key
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    matches!(
        normalized.as_str(),
        "apikey" | "accesstoken" | "authtoken" | "token" | "password" | "secret" | "privatekey"
    ) || normalized.ends_with("apikey")
        || normalized.ends_with("accesstoken")
        || normalized.ends_with("authtoken")
        || normalized.ends_with("token")
        || normalized.ends_with("password")
        || normalized.ends_with("secret")
        || normalized.ends_with("privatekey")
}
