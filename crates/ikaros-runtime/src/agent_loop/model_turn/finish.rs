// SPDX-License-Identifier: GPL-3.0-only

use super::events::emit_agent_event;
use crate::agent_loop::{
    report::finish_agent_loop,
    types::{
        AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, AgentLoopFinish,
        AgentLoopReport, AgentSessionId, AgentTurnId,
    },
};
use ikaros_core::Result;
use ikaros_harness::ExecutionSession;
use ikaros_models::TokenUsage;
use serde_json::json;

pub(super) fn finish_agent_loop_turn(
    session: &ExecutionSession,
    task_id: Option<String>,
    session_id: &AgentSessionId,
    turn_id: &AgentTurnId,
    events: &mut Vec<AgentEvent>,
    event_sink: &dyn AgentEventSink,
    mut finish: AgentLoopFinish,
) -> Result<AgentLoopReport> {
    emit_agent_event(
        events,
        event_sink,
        session_id,
        turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        json!({
            "stop_reason": &finish.stop_reason,
            "iterations": finish.iterations,
            "tool_result_count": finish.tool_results.len(),
        }),
    )?;
    finish.events = std::mem::take(events);
    finish_agent_loop(session, task_id, finish)
}

pub(super) fn merge_token_usage(mut total: TokenUsage, usage: &TokenUsage) -> TokenUsage {
    total.prompt_tokens = merge_token_count(total.prompt_tokens, usage.prompt_tokens);
    total.completion_tokens = merge_token_count(total.completion_tokens, usage.completion_tokens);
    total.total_tokens = merge_token_count(total.total_tokens, usage.total_tokens);
    total
}

fn merge_token_count(left: Option<u32>, right: Option<u32>) -> Option<u32> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}
