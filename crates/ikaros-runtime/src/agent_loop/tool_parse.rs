// SPDX-License-Identifier: GPL-3.0-only

use super::{
    tool_repair::extract_json_candidates,
    types::{
        AgentLoopModelEnvelope, AgentLoopToolCall, AgentLoopToolCallDiagnostic,
        AgentLoopToolCallParseStrategy,
    },
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
    parse_envelope_json(content, None).or_else(|| {
        extract_json_candidates(content)
            .into_iter()
            .find_map(|candidate| parse_envelope_json(&candidate.json, Some(candidate.strategy)))
    })
}

fn parse_envelope_json(
    content: &str,
    strategy: Option<AgentLoopToolCallParseStrategy>,
) -> Option<AgentLoopModelEnvelope> {
    let value = serde_json::from_str::<serde_json::Value>(content.trim()).ok()?;
    let inferred = strategy.unwrap_or_else(|| direct_json_strategy(&value));
    agent_loop_envelope_from_json_value(value, inferred)
}

fn direct_json_strategy(value: &serde_json::Value) -> AgentLoopToolCallParseStrategy {
    match value {
        serde_json::Value::Array(_) => AgentLoopToolCallParseStrategy::DirectJsonArray,
        _ => AgentLoopToolCallParseStrategy::DirectJsonObject,
    }
}

fn agent_loop_envelope_from_json_value(
    value: serde_json::Value,
    strategy: AgentLoopToolCallParseStrategy,
) -> Option<AgentLoopModelEnvelope> {
    match value {
        serde_json::Value::Object(object) => {
            let final_answer = object
                .get("final_answer")
                .or_else(|| object.get("answer"))
                .or_else(|| object.get("response"))
                .and_then(serde_json::Value::as_str)
                .map(redact_secrets);
            let mut tool_calls = Vec::new();
            for key in ["tool_calls", "tools", "calls"] {
                if let Some(calls) = object.get(key).and_then(serde_json::Value::as_array) {
                    tool_calls.extend(calls.iter().filter_map(agent_loop_tool_call_from_json));
                }
            }
            for key in ["tool_call", "function_call", "call"] {
                if let Some(call) = object.get(key).and_then(agent_loop_tool_call_from_json) {
                    tool_calls.push(call);
                }
            }
            if tool_calls.is_empty() {
                if let Some(call) =
                    agent_loop_tool_call_from_json(&serde_json::Value::Object(object.clone()))
                {
                    tool_calls.push(call);
                }
            }
            if final_answer.is_none() && tool_calls.is_empty() {
                return None;
            }
            Some(AgentLoopModelEnvelope {
                final_answer,
                tool_calls,
                parse_strategy: Some(strategy),
            })
        }
        serde_json::Value::Array(items) => {
            let tool_calls = items
                .iter()
                .filter_map(agent_loop_tool_call_from_json)
                .collect::<Vec<_>>();
            (!tool_calls.is_empty()).then_some(AgentLoopModelEnvelope {
                final_answer: None,
                tool_calls,
                parse_strategy: Some(strategy),
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
    if let Some(function) = object
        .get("function")
        .and_then(serde_json::Value::as_object)
    {
        let name = function
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(redact_secrets)?;
        let input = function
            .get("arguments")
            .or_else(|| function.get("input"))
            .or_else(|| function.get("parameters"))
            .map(|value| normalize_tool_input(value, value.as_str()))
            .unwrap_or_else(empty_object);
        let id = object
            .get("id")
            .or_else(|| object.get("tool_call_id"))
            .and_then(serde_json::Value::as_str)
            .map(redact_secrets);
        return Some(AgentLoopToolCall { id, name, input });
    }
    let id = object
        .get("id")
        .or_else(|| object.get("tool_call_id"))
        .and_then(serde_json::Value::as_str)
        .map(redact_secrets);
    let name = object
        .get("name")
        .or_else(|| object.get("tool"))
        .or_else(|| object.get("tool_name"))
        .and_then(serde_json::Value::as_str)
        .map(redact_secrets)?;
    let input = object
        .get("input")
        .or_else(|| object.get("arguments"))
        .or_else(|| object.get("args"))
        .or_else(|| object.get("parameters"))
        .map(|value| normalize_tool_input(value, value.as_str()))
        .unwrap_or_else(empty_object);
    Some(AgentLoopToolCall { id, name, input })
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
