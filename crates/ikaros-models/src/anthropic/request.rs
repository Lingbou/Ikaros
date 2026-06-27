// SPDX-License-Identifier: GPL-3.0-only

use super::wire::{
    AnthropicCacheControl, AnthropicContentBlock, AnthropicMessage, AnthropicMessagesRequest,
    AnthropicOutputConfig, AnthropicThinking, AnthropicToolDefinition,
};
use crate::http::ModelHttpRequest;
use crate::types::{
    ModelContentBlock, ModelMessage, ModelRequest, ModelToolCall, ModelToolDefinition,
    ReasoningEffort,
};
use ikaros_core::{
    IkarosError, RemoteProviderConfig, Result, redact_json, redact_secrets, resolve_config_value,
};
use std::{collections::BTreeMap, mem};

#[cfg(test)]
use ikaros_core::ModelConfig;

const ANTHROPIC_VERSION: &str = "2023-06-01";
const ANTHROPIC_DEFAULT_OUTPUT_LIMIT: u32 = 128_000;
const LEGACY_MANUAL_THINKING_CLAUDE_SUBSTRINGS: &[&str] = &[
    "claude-3",
    "claude-opus-4-0",
    "claude-opus-4.0",
    "claude-opus-4-1",
    "claude-opus-4.1",
    "claude-sonnet-4-0",
    "claude-sonnet-4.0",
    "claude-opus-4-2025",
    "claude-sonnet-4-2025",
    "claude-opus-4-5",
    "claude-opus-4.5",
    "claude-sonnet-4-5",
    "claude-sonnet-4.5",
    "claude-haiku-4-5",
    "claude-haiku-4.5",
];
const NO_XHIGH_CLAUDE_SUBSTRINGS: &[&str] = &[
    "claude-opus-4-6",
    "claude-opus-4.6",
    "claude-sonnet-4-6",
    "claude-sonnet-4.6",
];

pub(super) fn anthropic_http_post(
    url: &str,
    key: &str,
    body: &AnthropicMessagesRequest,
) -> Result<ModelHttpRequest> {
    let mut headers = BTreeMap::new();
    headers.insert("x-api-key".into(), key.into());
    headers.insert("anthropic-version".into(), ANTHROPIC_VERSION.into());
    headers.insert("content-type".into(), "application/json".into());
    Ok(ModelHttpRequest {
        method: "POST".into(),
        url: url.into(),
        headers,
        body: serde_json::to_string(body).map_err(|source| {
            IkarosError::Message(format!(
                "failed to serialize Anthropic request JSON: {source}"
            ))
        })?,
    })
}

pub(super) fn anthropic_messages_request_body(
    model: &str,
    request: ModelRequest,
) -> AnthropicMessagesRequest {
    let (system, messages) = anthropic_system_and_messages(request.messages);
    let mut max_tokens = request
        .options
        .max_tokens
        .unwrap_or_else(|| anthropic_default_max_tokens(model));
    let mut temperature = request.options.temperature;
    let mut top_p = request.options.top_p;
    let (thinking, output_config) = anthropic_thinking_fields(
        model,
        &request.options.reasoning,
        &mut max_tokens,
        &mut temperature,
    );
    if forbids_sampling_params(model) {
        temperature = None;
        top_p = None;
    }
    AnthropicMessagesRequest {
        model: model.into(),
        max_tokens,
        temperature,
        top_p,
        system,
        messages,
        tools: anthropic_tools(request.tools),
        thinking,
        output_config,
        stream: None,
    }
}

fn anthropic_thinking_fields(
    model: &str,
    reasoning: &crate::types::ReasoningConfig,
    max_tokens: &mut u32,
    temperature: &mut Option<f32>,
) -> (Option<AnthropicThinking>, Option<AnthropicOutputConfig>) {
    let configured = reasoning.enabled.is_some() || reasoning.effort.is_some();
    if !configured
        || reasoning.enabled == Some(false)
        || matches!(reasoning.effort, Some(ReasoningEffort::None))
        || model.to_ascii_lowercase().contains("haiku")
    {
        return (None, None);
    }

    let effort = reasoning.effort.unwrap_or(ReasoningEffort::Medium);
    if supports_adaptive_thinking(model) {
        let mut adaptive_effort = anthropic_adaptive_effort(effort);
        if adaptive_effort == "xhigh" && !supports_xhigh_effort(model) {
            adaptive_effort = "max";
        }
        return (
            Some(AnthropicThinking {
                kind: "adaptive".into(),
                display: Some("summarized".into()),
                budget_tokens: None,
            }),
            Some(AnthropicOutputConfig {
                effort: adaptive_effort.into(),
            }),
        );
    }

    let budget = anthropic_manual_thinking_budget(effort);
    *temperature = Some(1.0);
    *max_tokens = (*max_tokens).max(budget.saturating_add(4096));
    (
        Some(AnthropicThinking {
            kind: "enabled".into(),
            display: None,
            budget_tokens: Some(budget),
        }),
        None,
    )
}

fn anthropic_system_and_messages(
    messages: Vec<ModelMessage>,
) -> (Option<Vec<AnthropicContentBlock>>, Vec<AnthropicMessage>) {
    let mut system = Vec::new();
    let mut anthropic_messages = Vec::new();
    let mut pending_tool_results = Vec::new();

    for message in messages {
        if message.role == "system" {
            if !message.content.trim().is_empty() {
                let mut block = anthropic_text_block(message.content);
                if system.is_empty() {
                    block.cache_control = Some(AnthropicCacheControl {
                        kind: "ephemeral".into(),
                    });
                }
                system.push(block);
            }
            continue;
        }

        if message.role == "tool" {
            if let Some(tool_call_id) = message.tool_call_id {
                pending_tool_results.push(anthropic_tool_result_block(
                    tool_call_id,
                    message.content,
                    false,
                ));
                continue;
            }
        }

        if !pending_tool_results.is_empty() {
            anthropic_messages.push(AnthropicMessage {
                role: "user".into(),
                content: mem::take(&mut pending_tool_results),
            });
        }

        match message.role.as_str() {
            "assistant" => anthropic_messages.push(AnthropicMessage {
                role: "assistant".into(),
                content: anthropic_assistant_content(message),
            }),
            "user" => anthropic_messages.push(AnthropicMessage {
                role: "user".into(),
                content: anthropic_user_content(message),
            }),
            _ => anthropic_messages.push(AnthropicMessage {
                role: "user".into(),
                content: anthropic_user_content(message),
            }),
        }
    }

    if !pending_tool_results.is_empty() {
        anthropic_messages.push(AnthropicMessage {
            role: "user".into(),
            content: pending_tool_results,
        });
    }

    let system = (!system.is_empty()).then_some(system);
    (system, anthropic_messages)
}

fn anthropic_assistant_content(message: ModelMessage) -> Vec<AnthropicContentBlock> {
    let mut content = Vec::new();
    let block_text = anthropic_content_blocks_text(&message.content_blocks);
    if !message.content.trim().is_empty() && message.content != block_text {
        content.push(anthropic_text_block(message.content));
    }
    content.extend(
        message
            .content_blocks
            .into_iter()
            .map(anthropic_content_block),
    );
    content.extend(message.tool_calls.into_iter().map(anthropic_tool_use_block));
    if content.is_empty() {
        content.push(anthropic_text_block(""));
    }
    content
}

fn anthropic_user_content(message: ModelMessage) -> Vec<AnthropicContentBlock> {
    if message.content_blocks.is_empty() {
        return vec![anthropic_text_block(message.content)];
    }
    let mut content = Vec::new();
    let block_text = anthropic_content_blocks_text(&message.content_blocks);
    if !message.content.trim().is_empty() && message.content != block_text {
        content.push(anthropic_text_block(message.content));
    }
    content.extend(
        message
            .content_blocks
            .into_iter()
            .map(anthropic_content_block),
    );
    if content.is_empty() {
        content.push(anthropic_text_block(""));
    }
    content
}

fn anthropic_content_blocks_text(blocks: &[ModelContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ModelContentBlock::Text { text } => Some(text.as_str()),
            ModelContentBlock::ToolResult { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn anthropic_content_block(block: ModelContentBlock) -> AnthropicContentBlock {
    match block {
        ModelContentBlock::Text { text } => anthropic_text_block(text),
        ModelContentBlock::Image {
            image_url,
            mime_type,
            ..
        } => anthropic_image_block(image_url, mime_type),
        ModelContentBlock::Audio {
            audio_url,
            mime_type,
        } => anthropic_text_block(format!(
            "[audio block omitted url={} mime_type={}]",
            audio_url,
            mime_type.as_deref().unwrap_or("unknown")
        )),
        ModelContentBlock::File {
            file_url,
            mime_type,
            name,
        } => anthropic_text_block(format!(
            "[file block omitted url={} name={} mime_type={}]",
            file_url,
            name.as_deref().unwrap_or("unnamed"),
            mime_type.as_deref().unwrap_or("unknown")
        )),
        ModelContentBlock::ToolResult {
            tool_call_id,
            text,
            is_error,
        } => anthropic_tool_result_block(tool_call_id, text, is_error),
    }
}

fn anthropic_image_block(image_url: String, mime_type: Option<String>) -> AnthropicContentBlock {
    AnthropicContentBlock {
        kind: "image".into(),
        text: None,
        id: None,
        name: None,
        input: None,
        source: Some(anthropic_image_source(&image_url, mime_type.as_deref())),
        tool_use_id: None,
        content: None,
        is_error: None,
        cache_control: None,
    }
}

fn anthropic_image_source(image_url: &str, mime_type: Option<&str>) -> serde_json::Value {
    if let Some((media_type, data)) = parse_data_url_image(image_url) {
        return serde_json::json!({
            "type": "base64",
            "media_type": redact_secrets(mime_type.unwrap_or(media_type)),
            "data": redact_secrets(data),
        });
    }
    serde_json::json!({
        "type": "url",
        "url": redact_secrets(image_url),
    })
}

fn parse_data_url_image(image_url: &str) -> Option<(&str, &str)> {
    let rest = image_url.strip_prefix("data:")?;
    let (metadata, data) = rest.split_once(',')?;
    let media_type = metadata.strip_suffix(";base64")?;
    Some((media_type, data))
}

fn anthropic_text_block(text: impl Into<String>) -> AnthropicContentBlock {
    AnthropicContentBlock {
        kind: "text".into(),
        text: Some(redact_secrets(&text.into())),
        id: None,
        name: None,
        input: None,
        source: None,
        tool_use_id: None,
        content: None,
        is_error: None,
        cache_control: None,
    }
}

fn anthropic_tool_use_block(call: ModelToolCall) -> AnthropicContentBlock {
    AnthropicContentBlock {
        kind: "tool_use".into(),
        text: None,
        id: Some(redact_secrets(
            &call.id.unwrap_or_else(|| format!("toolu_{}", call.name)),
        )),
        name: Some(redact_secrets(&call.name)),
        input: Some(redact_json(call.input)),
        source: None,
        tool_use_id: None,
        content: None,
        is_error: None,
        cache_control: None,
    }
}

fn anthropic_tool_result_block(
    tool_call_id: impl Into<String>,
    content: impl Into<String>,
    is_error: bool,
) -> AnthropicContentBlock {
    AnthropicContentBlock {
        kind: "tool_result".into(),
        text: None,
        id: None,
        name: None,
        input: None,
        source: None,
        tool_use_id: Some(redact_secrets(&tool_call_id.into())),
        content: Some(redact_secrets(&content.into())),
        is_error: is_error.then_some(true),
        cache_control: None,
    }
}

fn anthropic_tools(tools: Vec<ModelToolDefinition>) -> Option<Vec<AnthropicToolDefinition>> {
    (!tools.is_empty()).then(|| {
        tools
            .into_iter()
            .map(|tool| AnthropicToolDefinition {
                name: redact_secrets(&tool.name),
                description: redact_secrets(&tool.description),
                input_schema: tool.input_schema,
            })
            .collect()
    })
}

pub(super) fn anthropic_default_max_tokens(model: &str) -> u32 {
    let model = model.to_ascii_lowercase().replace('.', "-");
    let limits = [
        ("claude-fable", 128_000),
        ("claude-opus-4-8", 128_000),
        ("claude-opus-4-7", 128_000),
        ("claude-opus-4-6", 128_000),
        ("claude-sonnet-4-6", 64_000),
        ("claude-opus-4-5", 64_000),
        ("claude-sonnet-4-5", 64_000),
        ("claude-haiku-4-5", 64_000),
        ("claude-opus-4", 32_000),
        ("claude-sonnet-4", 64_000),
        ("claude-3-7-sonnet", 128_000),
        ("claude-3-5-sonnet", 8_192),
        ("claude-3-5-haiku", 8_192),
        ("claude-3-opus", 4_096),
        ("claude-3-sonnet", 4_096),
        ("claude-3-haiku", 4_096),
        ("minimax", 131_072),
        ("qwen3", 65_536),
    ];
    limits
        .iter()
        .filter(|(needle, _)| model.contains(needle))
        .max_by_key(|(needle, _)| needle.len())
        .map(|(_, limit)| *limit)
        .unwrap_or(ANTHROPIC_DEFAULT_OUTPUT_LIMIT)
}

pub(super) fn anthropic_context_window(model: &str) -> u32 {
    let model = model.to_ascii_lowercase().replace('.', "-");
    if model.contains("claude") {
        return 200_000;
    }
    if model.contains("minimax") {
        return 1_000_000;
    }
    if model.contains("qwen3") {
        return 128_000;
    }
    200_000
}

fn supports_adaptive_thinking(model: &str) -> bool {
    if !is_claude_model(model) {
        return false;
    }
    !LEGACY_MANUAL_THINKING_CLAUDE_SUBSTRINGS
        .iter()
        .any(|needle| model.to_ascii_lowercase().contains(needle))
}

fn supports_xhigh_effort(model: &str) -> bool {
    supports_adaptive_thinking(model)
        && !NO_XHIGH_CLAUDE_SUBSTRINGS
            .iter()
            .any(|needle| model.to_ascii_lowercase().contains(needle))
}

fn forbids_sampling_params(model: &str) -> bool {
    if !is_claude_model(model) {
        return false;
    }
    let model = model.to_ascii_lowercase();
    if NO_XHIGH_CLAUDE_SUBSTRINGS
        .iter()
        .any(|needle| model.contains(needle))
    {
        return false;
    }
    !LEGACY_MANUAL_THINKING_CLAUDE_SUBSTRINGS
        .iter()
        .any(|needle| model.contains(needle))
}

fn is_claude_model(model: &str) -> bool {
    model.to_ascii_lowercase().contains("claude")
}

fn anthropic_adaptive_effort(effort: ReasoningEffort) -> &'static str {
    match effort {
        ReasoningEffort::None => "medium",
        ReasoningEffort::Minimal | ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
        ReasoningEffort::High => "high",
        ReasoningEffort::XHigh => "xhigh",
        ReasoningEffort::Max => "max",
    }
}

fn anthropic_manual_thinking_budget(effort: ReasoningEffort) -> u32 {
    match effort {
        ReasoningEffort::Low | ReasoningEffort::Minimal => 4_000,
        ReasoningEffort::High => 16_000,
        ReasoningEffort::XHigh | ReasoningEffort::Max => 32_000,
        ReasoningEffort::None | ReasoningEffort::Medium => 8_000,
    }
}

pub(super) fn provider_base_url(provider_settings: &RemoteProviderConfig) -> Result<String> {
    Ok(resolve_config_value(
        &provider_settings.base_url,
        "providers.model.base_url for Anthropic-compatible model provider",
    )?
    .trim_end_matches('/')
    .into())
}

#[cfg(test)]
pub(crate) fn test_messages_request_body(
    config: &ModelConfig,
    request: ModelRequest,
) -> serde_json::Value {
    let body = anthropic_messages_request_body(&config.model, request.redacted());
    serde_json::to_value(body).expect("serialize Anthropic messages request")
}
