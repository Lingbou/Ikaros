// SPDX-License-Identifier: GPL-3.0-only

use super::types::{
    OpenAiChatMessage, OpenAiFunctionDefinition, OpenAiOutboundFunctionCall,
    OpenAiOutboundToolCall, OpenAiStreamToolCallAccumulator, OpenAiToolCall, OpenAiToolDefinition,
};
use crate::types::{ModelMessage, ModelToolCall, ModelToolDefinition};
use ikaros_core::{redact_json, redact_secrets};

pub(super) fn openai_messages(messages: Vec<ModelMessage>) -> Vec<OpenAiChatMessage> {
    messages
        .into_iter()
        .map(|message| {
            let role = message.role;
            let content = (!message.content.is_empty()).then(|| redact_secrets(&message.content));
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
                    content: Some(redact_secrets(&message.content)),
                    tool_calls: Vec::new(),
                    tool_call_id: message.tool_call_id.map(|id| redact_secrets(&id)),
                };
            }
            OpenAiChatMessage {
                role,
                content: Some(redact_secrets(&message.content)),
                tool_calls: Vec::new(),
                tool_call_id: None,
            }
        })
        .collect()
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
