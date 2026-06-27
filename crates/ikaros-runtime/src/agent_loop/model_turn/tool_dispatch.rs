// SPDX-License-Identifier: GPL-3.0-only

use super::events::{
    HookDispatchContext, emit_agent_event, invoke_agent_loop_hook, redacted_json_value,
};
use crate::agent_loop::{
    dispatch::dispatch_agent_loop_tool_call,
    types::{
        AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, AgentLoopHooks,
        AgentLoopToolCall, AgentLoopToolDefinition, AgentLoopToolResult, AgentSessionId,
        AgentTurnId,
    },
};
use ikaros_core::{Result, redact_secrets};
use ikaros_harness::{CancellationToken, ExecutionSession, SkillRegistry, ToolExecutionMode};
use ikaros_session::AgentEventId;
use serde_json::json;

#[derive(Clone)]
pub(super) struct ScheduledAgentLoopToolCall {
    model_order: usize,
    call: AgentLoopToolCall,
    pub(super) tool_call_id: Option<String>,
    tool_call_name: String,
    execution_mode: ToolExecutionMode,
    timeout_ms: Option<u64>,
}

pub(super) struct DispatchedAgentLoopToolCall {
    pub(super) model_order: usize,
    pub(super) tool_call_id: Option<String>,
    pub(super) tool_call_name: String,
    pub(super) tool_input: serde_json::Value,
    pub(super) started_event_id: AgentEventId,
    pub(super) result: AgentLoopToolResult,
}

pub(super) struct ToolBatchDispatchContext<'a> {
    pub(super) session: &'a ExecutionSession,
    pub(super) registry: &'a SkillRegistry,
    pub(super) cancellation: &'a CancellationToken,
    pub(super) hooks: &'a dyn AgentLoopHooks,
    pub(super) task_id: Option<&'a str>,
    pub(super) session_id: &'a AgentSessionId,
    pub(super) turn_id: &'a AgentTurnId,
    pub(super) events: &'a mut Vec<AgentEvent>,
    pub(super) event_sink: &'a dyn AgentEventSink,
}

pub(super) fn schedule_agent_loop_tool_calls(
    calls: Vec<AgentLoopToolCall>,
    definitions: &[AgentLoopToolDefinition],
) -> Vec<ScheduledAgentLoopToolCall> {
    calls
        .into_iter()
        .enumerate()
        .map(|(model_order, call)| {
            let definition = definitions
                .iter()
                .find(|definition| definition.name == call.name);
            let execution_mode = definition
                .map(|definition| definition.execution_mode)
                .unwrap_or(ToolExecutionMode::Sequential);
            let timeout_ms = definition.and_then(|definition| definition.timeout_ms);
            ScheduledAgentLoopToolCall {
                model_order,
                tool_call_id: call.id.clone(),
                tool_call_name: call.name.clone(),
                call,
                execution_mode,
                timeout_ms,
            }
        })
        .collect()
}

pub(super) fn scheduled_tool_batch_end(
    calls: &[ScheduledAgentLoopToolCall],
    start: usize,
) -> usize {
    if calls[start].execution_mode == ToolExecutionMode::Sequential {
        return start + 1;
    }
    calls[start..]
        .iter()
        .position(|call| call.execution_mode == ToolExecutionMode::Sequential)
        .map(|offset| start + offset)
        .unwrap_or(calls.len())
        .max(start + 1)
}

pub(super) async fn dispatch_scheduled_tool_batch(
    context: ToolBatchDispatchContext<'_>,
    iteration: u32,
    batch: Vec<ScheduledAgentLoopToolCall>,
) -> Result<Vec<DispatchedAgentLoopToolCall>> {
    let mut started = Vec::with_capacity(batch.len());
    for scheduled in batch {
        let started_event_id = emit_agent_event(
            context.events,
            context.event_sink,
            context.session_id,
            context.turn_id,
            AgentEventSource::Tool,
            AgentEventKind::ToolCallStarted,
            json!({
                "iteration": iteration,
                "id": &scheduled.tool_call_id,
                "name": &scheduled.tool_call_name,
                "input": redacted_json_value(scheduled.call.input.clone()),
                "execution_mode": scheduled.execution_mode.as_str(),
                "timeout_ms": scheduled.timeout_ms,
            }),
        )?;
        invoke_agent_loop_hook(
            HookDispatchContext {
                hooks: context.hooks,
                events: context.events,
                event_sink: context.event_sink,
                session_id: context.session_id,
                turn_id: context.turn_id,
                task_id: context.task_id,
                iteration,
                event_id: Some(&started_event_id),
                hook_name: "before_tool_call",
            },
            json!({
                "tool_call_id": &scheduled.tool_call_id,
                "tool_event_id": started_event_id.as_str(),
                "name": &scheduled.tool_call_name,
                "input": redacted_json_value(scheduled.call.input.clone()),
                "execution_mode": scheduled.execution_mode.as_str(),
                "timeout_ms": scheduled.timeout_ms,
            }),
            |hooks, event| hooks.before_tool_call(event),
        )?;
        started.push((scheduled, started_event_id));
    }

    if started.len() == 1
        || started
            .iter()
            .any(|(scheduled, _)| scheduled.execution_mode == ToolExecutionMode::Sequential)
    {
        let mut dispatched = Vec::with_capacity(started.len());
        for (scheduled, started_event_id) in started {
            let result = dispatch_scheduled_tool_call_with_cancellation(
                &context,
                iteration,
                scheduled.clone(),
            )
            .await;
            dispatched.push(DispatchedAgentLoopToolCall {
                model_order: scheduled.model_order,
                tool_call_id: scheduled.tool_call_id,
                tool_call_name: scheduled.tool_call_name,
                tool_input: redacted_json_value(scheduled.call.input.clone()),
                started_event_id,
                result,
            });
        }
        return Ok(dispatched);
    }

    let (sender, mut receiver) = tokio::sync::mpsc::channel(started.len());
    let mut handles = Vec::with_capacity(started.len());
    let mut pending = Vec::with_capacity(started.len());
    for (scheduled, started_event_id) in started {
        let session = context.session.clone();
        let registry = context.registry.clone();
        let timeout_ms = scheduled.timeout_ms;
        let pending_call = PendingAgentLoopToolCall {
            model_order: scheduled.model_order,
            tool_call_id: scheduled.tool_call_id.clone(),
            tool_call_name: scheduled.tool_call_name.clone(),
            tool_input: redacted_json_value(scheduled.call.input.clone()),
            started_event_id,
        };
        pending.push(pending_call.clone());
        let sender = sender.clone();
        handles.push(tokio::spawn(async move {
            let result = dispatch_agent_loop_tool_call(
                &session,
                &registry,
                iteration,
                scheduled.call,
                timeout_ms,
            )
            .await;
            let _ = sender.send((pending_call, result)).await;
        }));
    }
    drop(sender);

    let mut dispatched = Vec::with_capacity(handles.len());
    while !pending.is_empty() {
        tokio::select! {
            received = receiver.recv() => {
                match received {
                    Some((pending_call, result)) => {
                        pending.retain(|call| call.started_event_id != pending_call.started_event_id);
                        dispatched.push(DispatchedAgentLoopToolCall {
                            model_order: pending_call.model_order,
                            tool_call_id: pending_call.tool_call_id,
                            tool_call_name: pending_call.tool_call_name,
                            tool_input: pending_call.tool_input,
                            started_event_id: pending_call.started_event_id,
                            result,
                        });
                    }
                    None => {
                        for pending_call in pending.drain(..) {
                            dispatched.push(DispatchedAgentLoopToolCall {
                                model_order: pending_call.model_order,
                                tool_call_id: pending_call.tool_call_id,
                                tool_call_name: pending_call.tool_call_name.clone(),
                                tool_input: pending_call.tool_input,
                                started_event_id: pending_call.started_event_id,
                                result: AgentLoopToolResult {
                                    iteration,
                                    name: redact_secrets(&pending_call.tool_call_name),
                                    harness_call_id: None,
                                    ok: false,
                                    summary: "tool task ended without reporting a result".into(),
                                    output: json!({"error": "tool task ended without reporting a result"}),
                                    recoverable: false,
                                },
                            });
                        }
                    }
                }
            }
            _ = context.cancellation.cancelled() => {
                for handle in &handles {
                    handle.abort();
                }
                for pending_call in pending.drain(..) {
                    dispatched.push(DispatchedAgentLoopToolCall {
                        model_order: pending_call.model_order,
                        tool_call_id: pending_call.tool_call_id,
                        tool_call_name: pending_call.tool_call_name.clone(),
                        tool_input: pending_call.tool_input,
                        started_event_id: pending_call.started_event_id,
                        result: cancelled_agent_loop_tool_result(
                            iteration,
                            &pending_call.tool_call_name,
                            "tool call cancelled during execution",
                        ),
                    });
                }
                dispatched.sort_by_key(|call| call.model_order);
                return Ok(dispatched);
            }
        }
    }
    dispatched.sort_by_key(|call| call.model_order);
    Ok(dispatched)
}

#[derive(Clone)]
struct PendingAgentLoopToolCall {
    model_order: usize,
    tool_call_id: Option<String>,
    tool_call_name: String,
    tool_input: serde_json::Value,
    started_event_id: AgentEventId,
}

async fn dispatch_scheduled_tool_call_with_cancellation(
    context: &ToolBatchDispatchContext<'_>,
    iteration: u32,
    scheduled: ScheduledAgentLoopToolCall,
) -> AgentLoopToolResult {
    tokio::select! {
        result = dispatch_agent_loop_tool_call(
            context.session,
            context.registry,
            iteration,
            scheduled.call,
            scheduled.timeout_ms,
        ) => result,
        _ = context.cancellation.cancelled() => {
            cancelled_agent_loop_tool_result(
                iteration,
                &scheduled.tool_call_name,
                "tool call cancelled during execution",
            )
        }
    }
}

fn cancelled_agent_loop_tool_result(
    iteration: u32,
    tool_call_name: &str,
    summary: &str,
) -> AgentLoopToolResult {
    AgentLoopToolResult {
        iteration,
        name: redact_secrets(tool_call_name),
        harness_call_id: None,
        ok: false,
        summary: summary.into(),
        output: json!({
            "cancelled": true,
            "reason": "agent loop cancellation requested",
        }),
        recoverable: false,
    }
}

pub(super) fn emit_cancelled_tool_call_events(
    context: ToolBatchDispatchContext<'_>,
    iteration: u32,
    scheduled_calls: &[ScheduledAgentLoopToolCall],
) -> Result<Vec<AgentLoopToolResult>> {
    let mut results = Vec::with_capacity(scheduled_calls.len());
    for scheduled in scheduled_calls {
        let summary = "tool call cancelled before execution".to_string();
        let output = json!({
            "cancelled": true,
            "reason": "agent loop cancellation requested",
        });
        let event_id = emit_agent_event(
            context.events,
            context.event_sink,
            context.session_id,
            context.turn_id,
            AgentEventSource::Tool,
            AgentEventKind::ToolCallCancelled,
            json!({
                "iteration": iteration,
                "tool_call_id": &scheduled.tool_call_id,
                "name": &scheduled.tool_call_name,
                "input": redacted_json_value(scheduled.call.input.clone()),
                "execution_mode": scheduled.execution_mode.as_str(),
                "timeout_ms": scheduled.timeout_ms,
                "status": "cancelled",
                "summary": &summary,
                "output": &output,
            }),
        )?;
        invoke_agent_loop_hook(
            HookDispatchContext {
                hooks: context.hooks,
                events: context.events,
                event_sink: context.event_sink,
                session_id: context.session_id,
                turn_id: context.turn_id,
                task_id: context.task_id,
                iteration,
                event_id: Some(&event_id),
                hook_name: "after_tool_call",
            },
            json!({
                "tool_call_id": &scheduled.tool_call_id,
                "name": &scheduled.tool_call_name,
                "input": redacted_json_value(scheduled.call.input.clone()),
                "execution_mode": scheduled.execution_mode.as_str(),
                "timeout_ms": scheduled.timeout_ms,
                "status": "cancelled",
                "summary": &summary,
                "output": &output,
            }),
            |hooks, event| hooks.after_tool_call(event),
        )?;
        results.push(AgentLoopToolResult {
            iteration,
            name: redact_secrets(&scheduled.tool_call_name),
            harness_call_id: None,
            ok: false,
            summary,
            output,
            recoverable: false,
        });
    }
    Ok(results)
}
