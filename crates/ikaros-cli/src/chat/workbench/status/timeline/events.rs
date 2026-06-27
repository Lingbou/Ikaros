// SPDX-License-Identifier: GPL-3.0-only

use ikaros_state::session::{AgentEvent, AgentEventKind};
use std::collections::BTreeSet;

use super::super::super::{WorkbenchCell, WorkbenchCellKind, agent_event_cell};
pub(super) fn timeline_event_cells(events: &[&AgentEvent]) -> Vec<WorkbenchCell> {
    let mut cells = Vec::new();
    if let Some(cell) = model_stream_summary_cell(events) {
        cells.push(cell);
    }
    cells.extend(
        events
            .iter()
            .copied()
            .filter(|event| !matches!(event.kind, AgentEventKind::ModelStream(_)))
            .map(agent_event_cell),
    );
    cells
}

fn model_stream_summary_cell(events: &[&AgentEvent]) -> Option<WorkbenchCell> {
    let mut turns = BTreeSet::<String>::new();
    let mut text_delta_chunks = 0usize;
    let mut reasoning_delta_chunks = 0usize;
    let mut refusal_delta_chunks = 0usize;
    let mut tool_call_events = 0usize;
    let mut usage_events = 0usize;
    let mut done_events = 0usize;
    let mut error_events = 0usize;
    let mut total = 0usize;

    for event in events {
        let AgentEventKind::ModelStream(stream_event) = &event.kind else {
            continue;
        };
        total += 1;
        turns.insert(event.turn_id.to_string());
        match stream_event {
            ikaros_providers::model::ModelStreamEvent::TextDelta(_) => text_delta_chunks += 1,
            ikaros_providers::model::ModelStreamEvent::ReasoningDelta(_) => {
                reasoning_delta_chunks += 1
            }
            ikaros_providers::model::ModelStreamEvent::RefusalDelta(_) => refusal_delta_chunks += 1,
            ikaros_providers::model::ModelStreamEvent::ToolCallStart { .. }
            | ikaros_providers::model::ModelStreamEvent::ToolCallDelta { .. }
            | ikaros_providers::model::ModelStreamEvent::ToolCallEnd { .. } => {
                tool_call_events += 1
            }
            ikaros_providers::model::ModelStreamEvent::Usage(_) => usage_events += 1,
            ikaros_providers::model::ModelStreamEvent::Error { .. } => error_events += 1,
            ikaros_providers::model::ModelStreamEvent::Done => done_events += 1,
            ikaros_providers::model::ModelStreamEvent::Start { .. } => {}
        }
    }

    (total > 0).then(|| WorkbenchCell {
        kind: WorkbenchCellKind::Model,
        title: "model stream summary".into(),
        detail: format!(
            "turns={} events={} text_delta_chunks={} reasoning_delta_chunks={} refusal_delta_chunks={} tool_call_events={} usage_events={} done_events={} error_events={} trace=/trace --kind model timeline=/timeline --kind model replay=/replay --kind model",
            turns.len(),
            total,
            text_delta_chunks,
            reasoning_delta_chunks,
            refusal_delta_chunks,
            tool_call_events,
            usage_events,
            done_events,
            error_events
        ),
    })
}
