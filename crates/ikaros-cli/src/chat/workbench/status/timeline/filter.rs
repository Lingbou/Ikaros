// SPDX-License-Identifier: GPL-3.0-only

use ikaros_state::session::{AgentEventKind, SessionReplay};

use super::{
    classification::trace_event_category,
    request::{TimelineRequest, TimelineVerbosity},
};
pub(super) fn timeline_request_requires_full_replay(request: &TimelineRequest) -> bool {
    request.turn_filter.is_some() || request.kind_filter.is_some() || request.point_filter.is_some()
}

pub(super) fn replay_page_size(verbosity: TimelineVerbosity) -> usize {
    timeline_entry_limit(verbosity)
}

pub(super) fn timeline_event_limit(verbosity: TimelineVerbosity) -> usize {
    match verbosity {
        TimelineVerbosity::Timeline => 12,
        TimelineVerbosity::Replay => 16,
        TimelineVerbosity::Debug => 20,
    }
}

pub(super) fn timeline_entry_limit(verbosity: TimelineVerbosity) -> usize {
    match verbosity {
        TimelineVerbosity::Timeline => 3,
        TimelineVerbosity::Replay | TimelineVerbosity::Debug => 8,
    }
}

pub(super) fn paged_window(len: usize, limit: usize, page: usize) -> (usize, usize) {
    if len == 0 {
        return (0, 0);
    }
    let limit = limit.max(1);
    let page = page.max(1);
    let end = len.saturating_sub(limit.saturating_mul(page.saturating_sub(1)));
    let start = end.saturating_sub(limit);
    (start, end)
}

pub(super) fn filtered_entries<'a>(
    replay: &'a SessionReplay,
    turn_id: &str,
) -> Vec<&'a ikaros_state::session::SessionEntry> {
    replay
        .entries
        .iter()
        .filter(|entry| {
            entry
                .turn_id
                .as_ref()
                .map(|candidate| candidate.as_str() == turn_id)
                .unwrap_or(false)
        })
        .collect()
}

pub(super) fn filtered_events<'a>(
    replay: &'a SessionReplay,
    turn_id: &str,
) -> Vec<&'a ikaros_state::session::AgentEvent> {
    replay
        .agent_events
        .iter()
        .filter(|event| event.turn_id.as_str() == turn_id)
        .collect()
}

pub(super) fn filtered_entries_for_timeline<'a>(
    replay: &'a SessionReplay,
    turn_filter: Option<&str>,
) -> Vec<&'a ikaros_state::session::SessionEntry> {
    match turn_filter {
        Some(turn_id) => filtered_entries(replay, turn_id),
        None => replay.entries.iter().collect(),
    }
}

pub(super) fn filtered_events_for_trace<'a>(
    replay: &'a SessionReplay,
    turn_filter: Option<&str>,
) -> Vec<&'a ikaros_state::session::AgentEvent> {
    match turn_filter {
        Some(turn_id) => filtered_events(replay, turn_id),
        None => replay.agent_events.iter().collect(),
    }
}

pub(super) fn filtered_events_for_timeline<'a>(
    replay: &'a SessionReplay,
    request: &TimelineRequest,
) -> Vec<&'a ikaros_state::session::AgentEvent> {
    filtered_events_for_trace(replay, request.turn_filter.as_deref())
        .into_iter()
        .filter(|event| {
            let kind_matches = request
                .kind_filter
                .as_deref()
                .map(|kind| trace_event_category(&event.kind) == kind)
                .unwrap_or(true);
            let point_matches = request
                .point_filter
                .as_deref()
                .map(|point| timeline_point_matches(&event.kind, point))
                .unwrap_or(true);
            kind_matches && point_matches
        })
        .collect()
}

pub(in crate::chat) fn timeline_point_matches(kind: &AgentEventKind, point: &str) -> bool {
    match point {
        "failed" => {
            matches!(
                kind,
                AgentEventKind::Error
                    | AgentEventKind::ToolCallFailed
                    | AgentEventKind::ContinuationFailed
                    | AgentEventKind::ModelStream(
                        ikaros_providers::model::ModelStreamEvent::Error { .. }
                    )
            ) || matches!(
                kind,
                AgentEventKind::ModelDiagnostic(diagnostic)
                    if diagnostic.kind.contains("failed") || diagnostic.kind.contains("error")
            )
        }
        "approval" => matches!(
            kind,
            AgentEventKind::ApprovalRequested | AgentEventKind::ApprovalResolved
        ),
        _ => false,
    }
}
