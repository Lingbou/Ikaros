// SPDX-License-Identifier: GPL-3.0-only

use super::types::{
    AgentLoopModelEnvelope, AgentLoopToolCall, AgentLoopToolCallDiagnostic,
    AgentLoopToolCallParseStrategy,
};
use ikaros_core::{redact_json, redact_secrets};
use ikaros_models::{ModelResponse, ModelToolCall};
use serde_json::json;

pub(super) fn agent_loop_model_envelope_from_response(
    response: &ModelResponse,
) -> Option<AgentLoopModelEnvelope> {
    if !response.tool_calls.is_empty() {
        return Some(AgentLoopModelEnvelope {
            final_answer: non_empty_string(&response.content),
            tool_calls: response
                .tool_calls
                .iter()
                .map(agent_loop_tool_call_from_model_tool_call)
                .collect(),
            parse_strategy: Some(AgentLoopToolCallParseStrategy::ProviderNativeToolCalls),
        });
    }
    parse_agent_loop_model_envelope(&response.content)
}

pub(super) fn parse_agent_loop_model_envelope(content: &str) -> Option<AgentLoopModelEnvelope> {
    parse_envelope_json(content)
}

fn parse_envelope_json(content: &str) -> Option<AgentLoopModelEnvelope> {
    let value = serde_json::from_str::<serde_json::Value>(content.trim()).ok()?;
    agent_loop_envelope_from_json_value(value)
}

fn agent_loop_envelope_from_json_value(value: serde_json::Value) -> Option<AgentLoopModelEnvelope> {
    match value {
        serde_json::Value::Object(object) => {
            if !object
                .keys()
                .all(|key| matches!(key.as_str(), "final_answer" | "tool_calls"))
            {
                return None;
            }
            let final_answer = optional_canonical_non_empty_string(object.get("final_answer"))?;
            let tool_calls = canonical_fallback_tool_calls(object.get("tool_calls"))?;
            if final_answer.is_none() && tool_calls.is_empty() {
                return None;
            }
            Some(AgentLoopModelEnvelope {
                final_answer,
                tool_calls,
                parse_strategy: Some(AgentLoopToolCallParseStrategy::JsonFallback),
            })
        }
        _ => None,
    }
}

pub(super) fn agent_loop_tool_call_diagnostic(
    iteration: u32,
    response: &ModelResponse,
    envelope: Option<&AgentLoopModelEnvelope>,
) -> AgentLoopToolCallDiagnostic {
    let strategy = envelope
        .and_then(|envelope| envelope.parse_strategy)
        .unwrap_or(AgentLoopToolCallParseStrategy::PlainText);
    AgentLoopToolCallDiagnostic {
        iteration,
        strategy,
        repaired: strategy.is_repaired(),
        native_tool_call_count: response.tool_calls.len(),
        tool_call_count: envelope
            .map(|envelope| envelope.tool_calls.len())
            .unwrap_or_default(),
        has_final_answer: envelope
            .and_then(|envelope| envelope.final_answer.as_ref())
            .is_some(),
    }
}

fn agent_loop_tool_call_from_model_tool_call(call: &ModelToolCall) -> AgentLoopToolCall {
    AgentLoopToolCall {
        id: call.id.clone().map(|id| redact_secrets(&id)),
        name: redact_secrets(&call.name),
        input: normalize_tool_input(&call.input, call.raw_arguments.as_deref()),
    }
}

fn agent_loop_tool_call_from_json(value: &serde_json::Value) -> Option<AgentLoopToolCall> {
    let object = value.as_object()?;
    if !object
        .keys()
        .all(|key| matches!(key.as_str(), "id" | "name" | "input"))
    {
        return None;
    }
    let id = optional_canonical_non_empty_string(object.get("id"))?;
    let name = object
        .get("name")
        .and_then(serde_json::Value::as_str)
        .and_then(non_empty_string)?;
    let input = canonical_fallback_tool_input(object.get("input"))?;
    Some(AgentLoopToolCall { id, name, input })
}

fn canonical_fallback_tool_calls(
    value: Option<&serde_json::Value>,
) -> Option<Vec<AgentLoopToolCall>> {
    let Some(value) = value else {
        return Some(Vec::new());
    };
    let calls = value.as_array()?;
    calls
        .iter()
        .map(agent_loop_tool_call_from_json)
        .collect::<Option<Vec<_>>>()
}

fn optional_canonical_non_empty_string(
    value: Option<&serde_json::Value>,
) -> Option<Option<String>> {
    match value {
        None => Some(None),
        Some(serde_json::Value::String(value)) => non_empty_string(value).map(Some),
        Some(_) => None,
    }
}

fn canonical_fallback_tool_input(value: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    let value = value?;
    match value {
        serde_json::Value::Object(_) => Some(redact_json(value.clone())),
        _ => None,
    }
}

fn normalize_tool_input(
    value: &serde_json::Value,
    raw_arguments: Option<&str>,
) -> serde_json::Value {
    match value {
        serde_json::Value::String(raw) => parse_tool_arguments(raw).unwrap_or_else(|| {
            json!({
                "raw_arguments": redact_secrets(raw),
            })
        }),
        serde_json::Value::Null => raw_arguments
            .and_then(parse_tool_arguments)
            .unwrap_or_else(empty_object),
        _ => redact_json(value.clone()),
    }
}

fn parse_tool_arguments(raw: &str) -> Option<serde_json::Value> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Some(empty_object());
    }
    serde_json::from_str(raw).ok().map(redact_json)
}

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(Default::default())
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = redact_secrets(value.trim());
    (!value.is_empty()).then_some(value)
}
