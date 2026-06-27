// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_state::session::{
    AgentEventKind, SessionId, SessionReplay, SessionStore, SqliteSessionStore,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use super::super::super::{
    WorkbenchCell, WorkbenchCellKind, agent_event_cell, coding_event_cells, session_entry_cell,
    terminal_inline,
};
use super::super::state_db_candidates;
use super::{
    classification::{
        TIMELINE_CATEGORY_ORDER, format_trace_counts, timeline_category_cell_kind,
        trace_event_category,
    },
    filter::timeline_point_matches,
};
pub(in crate::chat) fn screen_timeline_cells(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<Vec<WorkbenchCell>> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    for state_db in state_db_candidates(config, paths, workspace, runtime)? {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if let Some(replay) = store.replay_session(&session_id)? {
            return Ok(screen_timeline_cells_from_replay(&replay, 8));
        }
    }
    Ok(vec![WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "timeline".into(),
        detail: "/timeline shows recent entries and events; no replay found yet".into(),
    }])
}

pub(in crate::chat) fn screen_timeline_cells_from_replay(
    replay: &SessionReplay,
    limit: usize,
) -> Vec<WorkbenchCell> {
    let mut cells = replay
        .entries
        .iter()
        .map(|entry| (entry.at, session_entry_cell(entry)))
        .chain(
            replay
                .agent_events
                .iter()
                .filter(|event| !matches!(event.kind, AgentEventKind::ModelStream(_)))
                .map(|event| (event.at, agent_event_cell(event))),
        )
        .collect::<Vec<_>>();
    cells.sort_by_key(|(at, _)| *at);
    let limit = limit.max(1);
    let start = cells.len().saturating_sub(limit);
    let recent = cells
        .into_iter()
        .skip(start)
        .map(|(_, cell)| cell)
        .collect::<Vec<_>>();
    if recent.is_empty() {
        let mut output = vec![screen_timeline_navigation_cell(replay)];
        output.extend(screen_replay_navigation_cells(replay, limit));
        output.extend(screen_timeline_group_cells(replay));
        output.push(WorkbenchCell {
            kind: WorkbenchCellKind::Session,
            title: "timeline".into(),
            detail: "empty session timeline command=/timeline trace=/trace".into(),
        });
        output
    } else {
        let groups = screen_timeline_group_cells(replay);
        let replay_nav = screen_replay_navigation_cells(replay, limit);
        let mut output = Vec::with_capacity(recent.len() + groups.len() + replay_nav.len() + 1);
        output.push(screen_timeline_navigation_cell(replay));
        output.extend(replay_nav);
        output.extend(groups);
        output.extend(recent);
        output
    }
}

fn screen_timeline_navigation_cell(replay: &SessionReplay) -> WorkbenchCell {
    let mut turn_ids = replay
        .agent_events
        .iter()
        .map(|event| event.turn_id.as_str())
        .collect::<Vec<_>>();
    turn_ids.sort_unstable();
    turn_ids.dedup();
    let latest_turn = replay
        .agent_events
        .iter()
        .rev()
        .map(|event| event.turn_id.as_str())
        .next()
        .unwrap_or("none");
    let failed_turn = replay
        .agent_events
        .iter()
        .rev()
        .find(|event| matches!(event.kind, AgentEventKind::Error))
        .map(|event| event.turn_id.as_str())
        .unwrap_or("none");
    let latest_turn_commands = if latest_turn == "none" {
        "latest_timeline=none latest_trace=none".to_owned()
    } else {
        format!("latest_timeline=/timeline {latest_turn} latest_trace=/trace {latest_turn}")
    };
    let failed_turn_commands = if failed_turn == "none" {
        "failed_timeline=none failed_trace=none".to_owned()
    } else {
        format!("failed_timeline=/timeline {failed_turn} failed_trace=/trace {failed_turn}")
    };
    WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "timeline navigator".into(),
        detail: format!(
            "turns={} entries={} events={} approvals={} latest={} failed={} command=/timeline trace=/trace replay=/replay timeline=/timeline page=/timeline --page 2 failed_filter=/timeline --failed approval=/timeline --approval {} {}",
            turn_ids.len(),
            replay.entries.len(),
            replay.agent_events.len(),
            replay.approvals.len(),
            terminal_inline(latest_turn),
            terminal_inline(failed_turn),
            latest_turn_commands,
            failed_turn_commands
        ),
    }
}

fn screen_replay_navigation_cells(replay: &SessionReplay, page_size: usize) -> Vec<WorkbenchCell> {
    let page_size = page_size.max(1);
    let visible_event_count = replay
        .agent_events
        .iter()
        .filter(|event| !matches!(event.kind, AgentEventKind::ModelStream(_)))
        .count();
    let visible_items = replay.entries.len() + visible_event_count;
    let total_pages = if visible_items == 0 {
        1
    } else {
        (visible_items + page_size - 1) / page_size
    };
    let mut cells = vec![WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "replay navigation".into(),
        detail: format!(
            "items={} page_size={} pages={} timeline=/timeline replay=/replay trace=/trace page=/timeline --page 2 replay=/replay --page 2",
            visible_items, page_size, total_pages,
        ),
    }];
    if total_pages > 2 {
        cells.push(WorkbenchCell {
            kind: WorkbenchCellKind::Session,
            title: "older replay page".into(),
            detail: format!(
                "page={} timeline=/timeline --page {} replay=/replay --page {} debug=/debug --page {}",
                total_pages,
                total_pages,
                total_pages,
                total_pages,
            ),
        });
    }
    if let Some(cell) = screen_replay_point_cell(replay, "failed") {
        cells.push(cell);
    }
    if let Some(cell) = screen_replay_point_cell(replay, "approval") {
        cells.push(cell);
    }
    cells
}

fn screen_replay_point_cell(replay: &SessionReplay, point: &str) -> Option<WorkbenchCell> {
    let matching_events = replay
        .agent_events
        .iter()
        .filter(|event| timeline_point_matches(&event.kind, point))
        .collect::<Vec<_>>();
    let approval_record_count = if point == "approval" {
        replay.approvals.len()
    } else {
        0
    };
    let count = matching_events.len() + approval_record_count;
    if count == 0 {
        return None;
    }
    let latest_turn = matching_events
        .iter()
        .rev()
        .map(|event| event.turn_id.as_str())
        .next()
        .unwrap_or("none");
    let kind = match point {
        "failed" => WorkbenchCellKind::Error,
        "approval" => WorkbenchCellKind::Approval,
        _ => WorkbenchCellKind::Session,
    };
    let point_action = match point {
        "failed" => "failed_filter=/timeline --failed",
        "approval" => "approval=/timeline --approval",
        _ => "timeline=/timeline",
    };
    Some(WorkbenchCell {
        kind,
        title: format!("replay {point}"),
        detail: format!(
            "events={} latest_turn={} timeline=/timeline --{} trace=/trace --{} replay=/replay --{} {}",
            count,
            terminal_inline(latest_turn),
            point,
            point,
            point,
            point_action,
        ),
    })
}

fn screen_timeline_group_cells(replay: &SessionReplay) -> Vec<WorkbenchCell> {
    let mut counts = BTreeMap::<&'static str, usize>::new();
    let mut latest_turns = BTreeMap::<&'static str, String>::new();
    for event in &replay.agent_events {
        let category = trace_event_category(&event.kind);
        *counts.entry(category).or_default() += 1;
        latest_turns.insert(category, event.turn_id.to_string());
    }
    if counts.is_empty() {
        return Vec::new();
    }
    let mut cells = vec![WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "timeline groups".into(),
        detail: format!(
            "{} timeline=/timeline trace=/trace failed_filter=/timeline --failed approval=/timeline --approval",
            format_trace_counts(&counts)
        ),
    }];
    for category in TIMELINE_CATEGORY_ORDER {
        let count = counts.get(category).copied().unwrap_or(0);
        if count == 0 {
            continue;
        }
        let latest_turn = latest_turns
            .get(category)
            .map(String::as_str)
            .unwrap_or("none");
        cells.push(WorkbenchCell {
            kind: timeline_category_cell_kind(category),
            title: format!("timeline {category}"),
            detail: format!(
                "events={} latest_turn={} timeline=/timeline --kind {} trace=/trace --kind {} replay=/replay --kind {}",
                count,
                terminal_inline(latest_turn),
                category,
                category,
                category
            ),
        });
    }
    cells
}

pub(in crate::chat) fn screen_failure_cells(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<Vec<WorkbenchCell>> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    for state_db in state_db_candidates(config, paths, workspace, runtime)? {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if let Some(replay) = store.replay_session(&session_id)? {
            return Ok(screen_failure_cells_from_replay(&replay));
        }
    }
    Ok(Vec::new())
}

pub(in crate::chat) fn screen_failure_cells_from_replay(
    replay: &SessionReplay,
) -> Vec<WorkbenchCell> {
    let Some(event) = replay
        .agent_events
        .iter()
        .rev()
        .find(|event| matches!(event.kind, AgentEventKind::Error))
    else {
        return Vec::new();
    };
    let mut cell = agent_event_cell(event);
    cell.title = format!(
        "latest error turn={}",
        terminal_inline(event.turn_id.as_str())
    );
    vec![cell]
}

pub(in crate::chat) fn screen_coding_cells(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<Vec<WorkbenchCell>> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    for state_db in state_db_candidates(config, paths, workspace, runtime)? {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if let Some(replay) = store.replay_session(&session_id)? {
            return Ok(screen_coding_cells_from_replay(&replay));
        }
    }
    Ok(screen_coding_placeholder_cells())
}

pub(in crate::chat) fn screen_coding_cells_from_replay(
    replay: &SessionReplay,
) -> Vec<WorkbenchCell> {
    let mut cells = Vec::new();
    let coding_cells = coding_event_cells(&replay.agent_events);
    if !coding_cells.is_empty() {
        cells.push(screen_coding_workflow_cell(replay, &coding_cells));
    }
    for group in ["progress", "diff", "test", "review"] {
        if let Some((_, latest)) = coding_cells
            .iter()
            .rev()
            .find(|(candidate, _)| *candidate == group)
        {
            let mut cell = latest.clone();
            cell.detail = format!(
                "{} {}",
                terminal_inline(&cell.detail),
                coding_group_actions(group)
            );
            cells.push(cell);
        }
    }
    if cells.is_empty() {
        screen_coding_placeholder_cells()
    } else {
        cells
    }
}

fn screen_coding_workflow_cell(
    replay: &SessionReplay,
    coding_cells: &[(&'static str, WorkbenchCell)],
) -> WorkbenchCell {
    let summary = coding_workflow_summary(replay);
    let mut group_counts = BTreeMap::<&'static str, usize>::new();
    for (group, _) in coding_cells {
        *group_counts.entry(*group).or_default() += 1;
    }
    let group_summary = ["progress", "diff", "test", "review"]
        .into_iter()
        .map(|group| format!("{group}={}", group_counts.get(group).copied().unwrap_or(0)))
        .collect::<Vec<_>>()
        .join(" ");
    let latest_turn = summary.latest_turn.as_deref().unwrap_or("none");
    WorkbenchCell {
        kind: WorkbenchCellKind::Coding,
        title: "coding workflow".into(),
        detail: format!(
            "events={} turns={} latest_turn={} status={} {} command=/diff plan=/code plan apply=/code apply test=/code test review=/code review rollback=/code rollback workflow=/code workflow --model-loop trace=/trace timeline=/timeline",
            summary.event_count,
            summary.turn_count,
            terminal_inline(latest_turn),
            summary.status,
            group_summary,
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodingWorkflowSummary {
    event_count: usize,
    turn_count: usize,
    latest_turn: Option<String>,
    status: &'static str,
}

fn coding_workflow_summary(replay: &SessionReplay) -> CodingWorkflowSummary {
    let mut turn_ids = BTreeSet::new();
    let mut latest_turn = None;
    let mut has_failure = false;
    let mut has_review = false;
    let mut has_test = false;
    let mut has_diff = false;
    let mut event_count = 0;
    for event in replay
        .agent_events
        .iter()
        .filter(|event| matches!(event.kind, AgentEventKind::CodingTurn))
    {
        event_count += 1;
        turn_ids.insert(event.turn_id.as_str().to_owned());
        latest_turn = Some(event.turn_id.as_str().to_owned());
        let kind = event
            .payload
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        match coding_event_status_group(kind) {
            "failed" => has_failure = true,
            "review" => has_review = true,
            "test" => has_test = true,
            "diff" => has_diff = true,
            _ => {}
        }
    }
    let status = if has_failure {
        "attention"
    } else if has_review {
        "review"
    } else if has_test {
        "test"
    } else if has_diff {
        "diff"
    } else {
        "planning"
    };
    CodingWorkflowSummary {
        event_count,
        turn_count: turn_ids.len(),
        latest_turn,
        status,
    }
}

fn coding_event_status_group(kind: &str) -> &'static str {
    match kind {
        "patch_failed" => "failed",
        "review_started" | "review_finding" | "review_completed" | "iteration_planned" => "review",
        "test_evidence_recorded" => "test",
        "patch_applied" | "patch_skipped" | "diff_updated" => "diff",
        _ => "progress",
    }
}

fn screen_coding_placeholder_cells() -> Vec<WorkbenchCell> {
    vec![WorkbenchCell {
        kind: WorkbenchCellKind::Coding,
        title: "coding".into(),
        detail:
            "command=/diff plan=/code plan test=/code test review=/code review rollback=/code rollback workflow=/code workflow --model-loop"
                .into(),
    }]
}

fn coding_group_actions(group: &str) -> &'static str {
    match group {
        "diff" => "command=/diff apply=/code apply rollback=/code rollback",
        "test" => "command=/diff test=/code test review=/code review",
        "review" => "command=/diff review=/code review rollback=/code rollback",
        _ => "command=/diff plan=/code plan test=/code test review=/code review",
    }
}
