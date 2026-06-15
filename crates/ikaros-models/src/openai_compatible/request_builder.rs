// SPDX-License-Identifier: GPL-3.0-only

use super::{
    profile::{OpenAiCompatProfile, is_moonshot_model},
    schema_sanitizer::sanitize_moonshot_tool_definitions,
    tools::{openai_messages, openai_tools},
};
use crate::{
    params::merge_request_options,
    types::{ModelRequest, ModelRequestOptions},
};
use ikaros_core::{IkarosError, Result};
use serde_json::{Map, Number, Value};

#[derive(Debug, Clone)]
pub(super) struct PreparedChatCompletionRequest {
    pub(super) body: Value,
    pub(super) profile_id: &'static str,
}

pub(super) fn build_chat_completion_request(
    model: &str,
    base_url: &str,
    profile: OpenAiCompatProfile,
    default_options: &ModelRequestOptions,
    request: ModelRequest,
    stream: bool,
) -> Result<PreparedChatCompletionRequest> {
    let mut options = merge_request_options(default_options, &request.options);
    let mut body = Map::new();
    body.insert("model".into(), Value::String(model.into()));
    let mut messages =
        serde_json::to_value(openai_messages(request.messages)).map_err(|source| {
            IkarosError::Message(format!(
                "failed to serialize OpenAI-compatible messages: {source}"
            ))
        })?;
    profile.prepare_messages(&mut messages);
    body.insert("messages".into(), messages);

    let profile_default_max = profile.default_max_tokens(model);
    insert_u32(
        &mut body,
        "max_tokens",
        options.max_tokens.or(profile_default_max),
    );
    if profile.omits_temperature() {
        options.temperature = None;
    }
    insert_f32(&mut body, "temperature", options.temperature)?;
    insert_f32(&mut body, "top_p", options.top_p)?;
    insert_u32(&mut body, "n", options.n);
    insert_f32(&mut body, "presence_penalty", options.presence_penalty)?;
    insert_f32(&mut body, "frequency_penalty", options.frequency_penalty)?;
    if let Some(seed) = options.seed {
        body.insert("seed".into(), Value::Number(Number::from(seed)));
    }
    if !options.stop.is_empty() {
        body.insert(
            "stop".into(),
            Value::Array(options.stop.iter().cloned().map(Value::String).collect()),
        );
    }

    let tools = if profile == OpenAiCompatProfile::MoonshotKimi || is_moonshot_model(model) {
        sanitize_moonshot_tool_definitions(request.tools)
    } else {
        request.tools
    };
    if let Some(tools) = openai_tools(tools) {
        body.insert(
            "tools".into(),
            serde_json::to_value(tools).map_err(|source| {
                IkarosError::Message(format!(
                    "failed to serialize OpenAI-compatible tools: {source}"
                ))
            })?,
        );
    }
    if stream {
        body.insert("stream".into(), Value::Bool(true));
    }

    for (key, value) in &options.extra_body {
        body.insert(key.clone(), value.clone());
    }
    profile.apply_profile_fields(&mut body, model, base_url, &options);

    Ok(PreparedChatCompletionRequest {
        body: Value::Object(body),
        profile_id: profile.id(),
    })
}

fn insert_u32(body: &mut Map<String, Value>, key: &str, value: Option<u32>) {
    if let Some(value) = value {
        body.insert(key.into(), Value::Number(Number::from(value)));
    }
}

fn insert_f32(body: &mut Map<String, Value>, key: &str, value: Option<f32>) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    if !value.is_finite() {
        return Err(IkarosError::Message(format!(
            "model request option `{key}` must be finite"
        )));
    }
    let Some(number) = Number::from_f64(value as f64) else {
        return Err(IkarosError::Message(format!(
            "model request option `{key}` could not be represented as JSON"
        )));
    };
    body.insert(key.into(), Value::Number(number));
    Ok(())
}
