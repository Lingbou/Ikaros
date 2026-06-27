// SPDX-License-Identifier: GPL-3.0-only

use super::{
    events::{HookDispatchContext, emit_agent_event, invoke_agent_loop_hook, redacted_json_value},
    evidence::{ToolAuditAnchorContext, emit_session_approval_record, emit_tool_audit_anchor},
    state::AgentLoopTurnState,
    tool_dispatch::{
        DispatchedAgentLoopToolCall, ScheduledAgentLoopToolCall, ToolBatchDispatchContext,
        dispatch_scheduled_tool_batch, scheduled_tool_batch_end,
    },
    tool_result::{
        attach_recoverable_tool_retry, tool_lifecycle_end_kind, tool_lifecycle_status,
        tool_result_waiting_for_approval,
    },
};
use crate::agent_loop::{
    dispatch::{
        model_message_for_tool_result, observe_agent_loop_tool_result, stop_reason_from_tool_result,
    },
    types::{
        AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, AgentLoopHooks,
        AgentLoopStopReason, AgentLoopToolDefinition, AgentSessionId, AgentTurnId,
    },
};
use ikaros_core::{Result, redact_secrets};
use ikaros_harness::{
    CancellationToken, ExecutionSession, GuardrailConfig, GuardrailState, SkillRegistry,
};
use ikaros_models::ModelMessage;
use serde_json::json;

pub(super) struct ToolProcessingContext<'a> {
    pub(super) session: &'a ExecutionSession,
    pub(super) registry: &'a SkillRegistry,
    pub(super) direct_manifest: &'a [AgentLoopToolDefinition],
    pub(super) cancellation: &'a CancellationToken,
    pub(super) guardrails: &'a GuardrailConfig,
    pub(super) hooks: &'a dyn AgentLoopHooks,
    pub(super) task_id: Option<&'a str>,
    pub(super) session_id: &'a AgentSessionId,
    pub(super) turn_id: &'a AgentTurnId,
    pub(super) events: &'a mut Vec<AgentEvent>,
    pub(super) event_sink: &'a dyn AgentEventSink,
}

pub(super) async fn process_scheduled_tool_calls(
    mut context: ToolProcessingContext<'_>,
    iteration: u32,
    scheduled_calls: Vec<ScheduledAgentLoopToolCall>,
    state: &mut AgentLoopTurnState,
    guardrail_state: &mut GuardrailState,
    messages: &mut Vec<ModelMessage>,
) -> Result<Option<AgentLoopStopReason>> {
    let mut scheduled_index = 0;
    while scheduled_index < scheduled_calls.len() {
        let batch_end = scheduled_tool_batch_end(&scheduled_calls, scheduled_index);
        let batch = scheduled_calls[scheduled_index..batch_end].to_vec();
        let dispatched = dispatch_scheduled_tool_batch(
            ToolBatchDispatchContext {
                session: context.session,
                registry: context.registry,
                direct_manifest: context.direct_manifest,
                cancellation: context.cancellation,
                hooks: context.hooks,
                task_id: context.task_id,
                session_id: context.session_id,
                turn_id: context.turn_id,
                events: &mut *context.events,
                event_sink: context.event_sink,
            },
            iteration,
            batch,
        )
        .await?;

        for dispatched_tool in dispatched {
            if let Some(stop_reason) = record_dispatched_tool_result(
                &mut context,
                iteration,
                dispatched_tool,
                state,
                guardrail_state,
                messages,
            )? {
                return Ok(Some(stop_reason));
            }
        }

        if context.cancellation.is_cancelled() {
            return Ok(Some(AgentLoopStopReason::Cancelled));
        }
        scheduled_index = batch_end;
    }

    Ok(None)
}

fn record_dispatched_tool_result(
    context: &mut ToolProcessingContext<'_>,
    iteration: u32,
    dispatched_tool: DispatchedAgentLoopToolCall,
    state: &mut AgentLoopTurnState,
    guardrail_state: &mut GuardrailState,
    messages: &mut Vec<ModelMessage>,
) -> Result<Option<AgentLoopStopReason>> {
    let tool_call_id = dispatched_tool.tool_call_id;
    let tool_call_name = dispatched_tool.tool_call_name;
    let tool_input = dispatched_tool.tool_input;
    let started_event_id = dispatched_tool.started_event_id;
    let mut tool_result = dispatched_tool.result;
    attach_recoverable_tool_retry(&mut tool_result, &tool_call_name, tool_input);

    if tool_result_waiting_for_approval(&tool_result) {
        let approval_id = tool_result
            .output
            .get("approval_id")
            .and_then(serde_json::Value::as_str);
        emit_agent_event(
            &mut *context.events,
            context.event_sink,
            context.session_id,
            context.turn_id,
            AgentEventSource::Harness,
            AgentEventKind::ApprovalRequested,
            json!({
                "iteration": iteration,
                "approval_id": approval_id,
                "tool_call_id": &tool_call_id,
                "tool_event_id": started_event_id.as_str(),
                "tool": &tool_result.name,
                "output": &tool_result.output,
            }),
        )?;
        emit_session_approval_record(
            context.event_sink,
            context.session,
            context.session_id,
            context.turn_id,
            &tool_result.output,
        )?;
    }

    let terminal_event_id = emit_agent_event(
        &mut *context.events,
        context.event_sink,
        context.session_id,
        context.turn_id,
        AgentEventSource::Tool,
        AgentEventKind::ToolCallOutputDelta,
        json!({
            "iteration": iteration,
            "tool_call_id": &tool_call_id,
            "tool_event_id": started_event_id.as_str(),
            "name": &tool_result.name,
            "summary": &tool_result.summary,
            "output": &tool_result.output,
        }),
    )?;
    emit_agent_event(
        &mut *context.events,
        context.event_sink,
        context.session_id,
        context.turn_id,
        AgentEventSource::Tool,
        tool_lifecycle_end_kind(&tool_result),
        json!({
            "iteration": iteration,
            "tool_call_id": &tool_call_id,
            "tool_event_id": started_event_id.as_str(),
            "name": &tool_result.name,
            "ok": tool_result.ok,
            "status": tool_lifecycle_status(&tool_result),
            "summary": &tool_result.summary,
            "output": &tool_result.output,
        }),
    )?;
    invoke_agent_loop_hook(
        HookDispatchContext {
            hooks: context.hooks,
            events: &mut *context.events,
            event_sink: context.event_sink,
            session_id: context.session_id,
            turn_id: context.turn_id,
            task_id: context.task_id,
            iteration,
            event_id: Some(&terminal_event_id),
            hook_name: "after_tool_call",
        },
        json!({
            "tool_call_id": &tool_call_id,
            "tool_event_id": started_event_id.as_str(),
            "name": &tool_result.name,
            "ok": tool_result.ok,
            "status": tool_lifecycle_status(&tool_result),
            "summary": redact_secrets(&tool_result.summary),
            "output": redacted_json_value(tool_result.output.clone()),
        }),
        |hooks, event| hooks.after_tool_call(event),
    )?;
    emit_tool_audit_anchor(
        ToolAuditAnchorContext {
            session: context.session,
            events: &mut *context.events,
            event_sink: context.event_sink,
            session_id: context.session_id,
            turn_id: context.turn_id,
            iteration,
            tool_call_id: &tool_call_id,
            tool_event_id: &started_event_id,
        },
        &tool_result,
    )?;

    let guardrail_stop = observe_agent_loop_tool_result(
        context.session,
        context.task_id,
        guardrail_state,
        context.guardrails,
        &tool_result,
    )?;
    messages.push(model_message_for_tool_result(
        tool_call_id,
        tool_call_name,
        &tool_result,
    ));
    let stop_reason = guardrail_stop.or_else(|| stop_reason_from_tool_result(&tool_result));
    state.tool_results.push(tool_result);
    Ok(stop_reason)
}
