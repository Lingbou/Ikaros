// SPDX-License-Identifier: GPL-3.0-only

use super::wire::{AnthropicContentBlock, AnthropicMessagesResponse};
use crate::types::{ModelResponse, ModelToolCall};
use ikaros_core::{IkarosError, Result, redact_json, redact_secrets};

pub(crate) fn parse_messages_response(
    text: &str,
    provider: &str,
    fallback_model: &str,
) -> Result<ModelResponse> {
    let parsed: AnthropicMessagesResponse = serde_json::from_str(text).map_err(|source| {
        IkarosError::Message(format!(
            "failed to parse Anthropic model response JSON: {source}"
        ))
    })?;
    let content = parsed
        .content
        .iter()
        .filter(|block| block.kind == "text")
        .filter_map(|block| block.text.as_deref())
        .map(redact_secrets)
        .collect::<Vec<_>>()
        .join("");
    let tool_calls = parsed
        .content
        .into_iter()
        .filter(|block| block.kind == "tool_use")
        .filter_map(anthropic_model_tool_call)
        .collect();
    Ok(ModelResponse {
        provider: provider.into(),
        model: parsed.model.unwrap_or_else(|| fallback_model.into()),
        content,
        tool_calls,
        usage: parsed.usage.unwrap_or_default().into(),
        diagnostics: Vec::new(),
    })
}

fn anthropic_model_tool_call(block: AnthropicContentBlock) -> Option<ModelToolCall> {
    let name = block.name?;
    let input = block
        .input
        .map(redact_json)
        .unwrap_or_else(|| serde_json::Value::Object(Default::default()));
    Some(ModelToolCall {
        id: block.id.map(|id| redact_secrets(&id)),
        name: redact_secrets(&name),
        raw_arguments: Some(redact_secrets(&input.to_string())),
        input,
    })
}
