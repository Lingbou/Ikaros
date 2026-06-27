// SPDX-License-Identifier: GPL-3.0-only

use super::types::{
    OpenAiChatMessage, OpenAiFunctionDefinition, OpenAiOutboundFunctionCall,
    OpenAiOutboundToolCall, OpenAiStreamToolCallAccumulator, OpenAiToolCall, OpenAiToolDefinition,
};
use crate::types::{ModelContentBlock, ModelMessage, ModelToolCall, ModelToolDefinition};
use ikaros_core::{redact_json, redact_secrets};

pub(super) fn openai_messages(messages: Vec<ModelMessage>) -> Vec<OpenAiChatMessage> {
    messages
        .into_iter()
        .map(|message| {
            let content = openai_message_content(&message);
            let role = message.role;
            if role == "assistant" && !message.tool_calls.is_empty() {
                return OpenAiChatMessage {
                    role,
                    content,
                    tool_calls: openai_outbound_tool_calls(message.tool_calls),
                    tool_call_id: None,
                };
            }
            if role == "tool" {
                return OpenAiChatMessage {
                    role,
                    content: Some(serde_json::Value::String(redact_secrets(&message.content))),
                    tool_calls: Vec::new(),
                    tool_call_id: message.tool_call_id.map(|id| redact_secrets(&id)),
                };
            }
            OpenAiChatMessage {
                role,
                content: content
                    .or_else(|| Some(serde_json::Value::String(redact_secrets(&message.content)))),
                tool_calls: Vec::new(),
                tool_call_id: None,
            }
        })
        .collect()
}

fn openai_message_content(message: &ModelMessage) -> Option<serde_json::Value> {
    if message.content_blocks.is_empty() {
        return (!message.content.is_empty())
            .then(|| serde_json::Value::String(redact_secrets(&message.content)));
    }
    let mut parts = Vec::new();
    let block_text = openai_content_blocks_text(&message.content_blocks);
    if !message.content.trim().is_empty() && message.content != block_text {
        parts.push(serde_json::json!({
            "type": "text",
            "text": redact_secrets(&message.content),
        }));
    }
    parts.extend(
        message
            .content_blocks
            .iter()
            .map(openai_content_part)
            .collect::<Vec<_>>(),
    );
    Some(serde_json::Value::Array(parts))
}

fn openai_content_blocks_text(blocks: &[ModelContentBlock]) -> String {
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

fn openai_content_part(block: &ModelContentBlock) -> serde_json::Value {
    match block {
        ModelContentBlock::Text { text } => serde_json::json!({
            "type": "text",
            "text": redact_secrets(text),
        }),
        ModelContentBlock::Image {
            image_url, detail, ..
        } => {
            let mut image = serde_json::Map::new();
            image.insert(
                "url".into(),
                serde_json::Value::String(redact_secrets(image_url)),
            );
            if let Some(detail) = detail {
                image.insert(
                    "detail".into(),
                    serde_json::Value::String(redact_secrets(detail)),
                );
            }
            serde_json::json!({
                "type": "image_url",
                "image_url": serde_json::Value::Object(image),
            })
        }
        ModelContentBlock::Audio {
            audio_url,
            mime_type,
        } => openai_audio_content_part(audio_url, mime_type.as_deref()),
        ModelContentBlock::File {
            file_url,
            mime_type,
            name,
        } => openai_file_content_part(file_url, mime_type.as_deref(), name.as_deref()),
        ModelContentBlock::ToolResult {
            tool_call_id,
            text,
            is_error,
        } => serde_json::json!({
            "type": "text",
            "text": redact_secrets(&format!(
                "[tool result block id={} error={}] {}",
                tool_call_id, is_error, text
            )),
        }),
    }
}

fn openai_audio_content_part(audio_url: &str, mime_type: Option<&str>) -> serde_json::Value {
    let mut audio = serde_json::Map::new();
    if let Some((data_mime, data)) = split_data_url(audio_url) {
        audio.insert(
            "data".into(),
            serde_json::Value::String(redact_secrets(data)),
        );
        audio.insert(
            "format".into(),
            serde_json::Value::String(openai_audio_format(mime_type.or(Some(data_mime)))),
        );
    } else {
        audio.insert(
            "url".into(),
            serde_json::Value::String(redact_secrets(audio_url)),
        );
        if let Some(mime_type) = mime_type {
            audio.insert(
                "mime_type".into(),
                serde_json::Value::String(redact_secrets(mime_type)),
            );
        }
    }
    serde_json::json!({
        "type": "input_audio",
        "input_audio": serde_json::Value::Object(audio),
    })
}

fn openai_file_content_part(
    file_url: &str,
    mime_type: Option<&str>,
    name: Option<&str>,
) -> serde_json::Value {
    let mut file = serde_json::Map::new();
    if let Some((data_mime, _)) = split_data_url(file_url) {
        file.insert(
            "file_data".into(),
            serde_json::Value::String(redact_secrets(file_url)),
        );
        file.insert(
            "mime_type".into(),
            serde_json::Value::String(redact_secrets(mime_type.unwrap_or(data_mime))),
        );
    } else if let Some(file_id) = file_url.strip_prefix("file_id:") {
        file.insert(
            "file_id".into(),
            serde_json::Value::String(redact_secrets(file_id)),
        );
    } else {
        file.insert(
            "file_url".into(),
            serde_json::Value::String(redact_secrets(file_url)),
        );
        if let Some(mime_type) = mime_type {
            file.insert(
                "mime_type".into(),
                serde_json::Value::String(redact_secrets(mime_type)),
            );
        }
    }
    if let Some(name) = name {
        file.insert(
            "filename".into(),
            serde_json::Value::String(redact_secrets(name)),
        );
    }
    serde_json::json!({
        "type": "file",
        "file": serde_json::Value::Object(file),
    })
}

fn split_data_url(value: &str) -> Option<(&str, &str)> {
    let rest = value.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime, data))
}

fn openai_audio_format(mime_type: Option<&str>) -> String {
    match mime_type
        .unwrap_or("audio/wav")
        .to_ascii_lowercase()
        .as_str()
    {
        "audio/mpeg" | "audio/mp3" => "mp3".into(),
        "audio/mp4" | "audio/m4a" => "m4a".into(),
        "audio/ogg" => "ogg".into(),
        "audio/opus" => "opus".into(),
        "audio/flac" => "flac".into(),
        "audio/wav" | "audio/x-wav" => "wav".into(),
        other => other
            .strip_prefix("audio/")
            .map(str::to_owned)
            .unwrap_or_else(|| other.to_owned()),
    }
}

fn openai_outbound_tool_calls(calls: Vec<ModelToolCall>) -> Vec<OpenAiOutboundToolCall> {
    calls
        .into_iter()
        .enumerate()
        .map(|(index, call)| {
            let arguments = call.raw_arguments.unwrap_or_else(|| call.input.to_string());
            OpenAiOutboundToolCall {
                id: call
                    .id
                    .map(|id| redact_secrets(&id))
                    .unwrap_or_else(|| format!("call_{index}")),
                r#type: "function",
                function: OpenAiOutboundFunctionCall {
                    name: redact_secrets(&call.name),
                    arguments: redact_secrets(&arguments),
                },
            }
        })
        .collect()
}

pub(super) fn openai_tools(tools: Vec<ModelToolDefinition>) -> Option<Vec<OpenAiToolDefinition>> {
    if tools.is_empty() {
        return None;
    }
    Some(
        tools
            .into_iter()
            .map(|tool| OpenAiToolDefinition {
                r#type: "function",
                function: OpenAiFunctionDefinition {
                    name: redact_secrets(&tool.name),
                    description: redact_secrets(&tool.description),
                    parameters: tool.input_schema,
                },
            })
            .collect(),
    )
}

pub(super) fn model_tool_calls(calls: &[OpenAiToolCall]) -> Vec<ModelToolCall> {
    calls
        .iter()
        .map(|call| {
            let arguments = call.function.arguments.trim();
            let input = if arguments.is_empty() {
                serde_json::Value::Object(Default::default())
            } else {
                serde_json::from_str(arguments)
                    .map(redact_json)
                    .unwrap_or_else(|_| serde_json::Value::String(redact_secrets(arguments)))
            };
            ModelToolCall {
                id: call.id.clone().map(|id| redact_secrets(&id)),
                name: redact_secrets(&call.function.name),
                input,
                raw_arguments: (!arguments.is_empty()).then(|| redact_secrets(arguments)),
            }
        })
        .collect()
}

pub(super) fn model_tool_calls_from_stream_accumulators(
    accumulators: Vec<OpenAiStreamToolCallAccumulator>,
) -> Vec<ModelToolCall> {
    accumulators
        .into_iter()
        .filter(|call| !call.name.trim().is_empty())
        .map(|call| {
            let arguments = call.arguments.trim();
            let input = if arguments.is_empty() {
                serde_json::Value::Object(Default::default())
            } else {
                serde_json::from_str(arguments)
                    .map(redact_json)
                    .unwrap_or_else(|_| serde_json::Value::String(redact_secrets(arguments)))
            };
            ModelToolCall {
                id: call.id,
                name: redact_secrets(call.name.trim()),
                input,
                raw_arguments: (!arguments.is_empty()).then(|| redact_secrets(arguments)),
            }
        })
        .collect()
}
