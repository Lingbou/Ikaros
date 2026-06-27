// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_session::{
    AgentEvent, AgentEventKind, SessionEntry, SessionEntryKind, SessionId, SessionReplay,
    SessionReplayPage, SessionStore, SqliteSessionStore,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use super::super::{
    WorkbenchCell, WorkbenchCellKind, agent_event_cell, coding_event_cells, path_display,
    session_entry_cell, terminal_inline,
};
use super::state_db_candidates;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::chat) enum TimelineVerbosity {
    Timeline,
    Replay,
    Debug,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::chat) struct TimelineRequest {
    pub(in crate::chat) turn_filter: Option<String>,
    pub(in crate::chat) kind_filter: Option<String>,
    pub(in crate::chat) point_filter: Option<String>,
    pub(in crate::chat) page: usize,
}

impl TimelineRequest {
    pub(in crate::chat) fn for_turn(turn_id: &str) -> Self {
        Self {
            turn_filter: Some(turn_id.to_owned()),
            kind_filter: None,
            point_filter: None,
            page: 1,
        }
    }
}

impl Default for TimelineRequest {
    fn default() -> Self {
        Self {
            turn_filter: None,
            kind_filter: None,
            point_filter: None,
            page: 1,
        }
    }
}

pub(super) fn screen_timeline_cells(
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

pub(super) fn screen_timeline_cells_from_replay(
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

const TIMELINE_CATEGORY_ORDER: &[&str] = &[
    "session",
    "model",
    "tool",
    "context",
    "memory",
    "coding",
    "audit",
    "continuation",
    "approval",
    "error",
];

fn timeline_category_cell_kind(category: &str) -> WorkbenchCellKind {
    match category {
        "model" => WorkbenchCellKind::Model,
        "tool" => WorkbenchCellKind::Tool,
        "context" => WorkbenchCellKind::Context,
        "memory" => WorkbenchCellKind::Memory,
        "coding" => WorkbenchCellKind::Coding,
        "audit" => WorkbenchCellKind::Audit,
        "continuation" => WorkbenchCellKind::Continuation,
        "approval" => WorkbenchCellKind::Approval,
        "error" => WorkbenchCellKind::Error,
        _ => WorkbenchCellKind::Session,
    }
}

pub(super) fn screen_failure_cells(
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

pub(super) fn screen_failure_cells_from_replay(replay: &SessionReplay) -> Vec<WorkbenchCell> {
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

pub(super) fn screen_coding_cells(
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

pub(super) fn screen_coding_cells_from_replay(replay: &SessionReplay) -> Vec<WorkbenchCell> {
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

pub(super) fn print_screen_trace_snapshot(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let candidates = state_db_candidates(config, paths, workspace, runtime)?;
    for state_db in &candidates {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if let Some(replay) = store.replay_session(&session_id)? {
            let mut counts = BTreeMap::<&'static str, usize>::new();
            let mut turns = BTreeSet::<String>::new();
            for event in filtered_events_for_trace(&replay, None) {
                *counts.entry(trace_event_category(&event.kind)).or_default() += 1;
                turns.insert(event.turn_id.to_string());
            }
            println!("screen_trace: found");
            println!("screen_trace_spans: {}", turns.len());
            println!("screen_trace_counts: {}", format_trace_counts(&counts));
            println!("screen_trace_hint: /trace");
            return Ok(());
        }
    }
    println!("screen_trace: not_found");
    println!("screen_trace_spans: 0");
    println!(
        "screen_trace_counts: {}",
        format_trace_counts(&BTreeMap::new())
    );
    println!("screen_trace_hint: /trace");
    Ok(())
}

pub(in crate::chat) fn print_replay_status(
    label: &str,
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    verbosity: TimelineVerbosity,
    request: TimelineRequest,
) -> Result<()> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let candidates = state_db_candidates(config, paths, workspace, runtime)?;
    for state_db in &candidates {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if !timeline_request_requires_full_replay(&request) {
            if let Some(replay) =
                store.replay_session_page(&session_id, request.page, replay_page_size(verbosity))?
            {
                let continuations = store.continuations(&session_id)?;
                let all_events = store.agent_events(&session_id)?;
                println!("{label}: found");
                println!("session: {}", terminal_inline(session_id.as_str()));
                println!("state_db: {}", path_display(state_db));
                println!("entries: {}", replay.total_entries);
                println!("agent_events: {}", replay.total_agent_events);
                println!("approvals: {}", replay.total_approvals);
                println!("continuations: {}", continuations.len());
                println!("{label}_page: {}", request.page);
                println!("{label}_page_source: session_store_page");
                println!(
                    "{label}_page_totals: entries={} agent_events={} approvals={}",
                    replay.total_entries, replay.total_agent_events, replay.total_approvals
                );
                println!(
                    "{label}_page_size: entries={} events={}",
                    timeline_entry_limit(verbosity),
                    timeline_event_limit(verbosity)
                );
                print_recent_timeline_page(&replay, verbosity, &all_events);
                return Ok(());
            }
            continue;
        }
        if let Some(replay) = store.replay_session(&session_id)? {
            let continuations = store.continuations(&session_id)?;
            println!("{label}: found");
            println!("session: {}", terminal_inline(session_id.as_str()));
            println!("state_db: {}", path_display(state_db));
            println!("entries: {}", replay.entries.len());
            println!("agent_events: {}", replay.agent_events.len());
            println!("approvals: {}", replay.approvals.len());
            println!("continuations: {}", continuations.len());
            println!("{label}_page: {}", request.page);
            println!("{label}_page_source: session_replay_full");
            println!(
                "{label}_page_size: entries={} events={}",
                timeline_entry_limit(verbosity),
                timeline_event_limit(verbosity)
            );
            if let Some(turn_id) = request.turn_filter.as_deref() {
                let filtered_entries = filtered_entries(&replay, turn_id);
                let filtered_events = filtered_events(&replay, turn_id);
                println!("{label}_turn_filter: {}", terminal_inline(turn_id));
                println!(
                    "{label}_turn: {}",
                    if filtered_entries.is_empty() && filtered_events.is_empty() {
                        "not_found"
                    } else {
                        "found"
                    }
                );
                println!("filtered_entries: {}", filtered_entries.len());
                println!("filtered_agent_events: {}", filtered_events.len());
            }
            if let Some(kind) = request.kind_filter.as_deref() {
                let filtered_events = filtered_events_for_timeline(&replay, &request);
                println!("{label}_kind_filter: {}", terminal_inline(kind));
                println!("filtered_entries: 0");
                println!("filtered_agent_events: {}", filtered_events.len());
            }
            if let Some(point) = request.point_filter.as_deref() {
                let filtered_events = filtered_events_for_timeline(&replay, &request);
                println!("{label}_point_filter: {}", terminal_inline(point));
                println!("filtered_entries: 0");
                println!("filtered_agent_events: {}", filtered_events.len());
            }
            print_recent_timeline(&replay, verbosity, &request);
            return Ok(());
        }
    }
    println!("{label}: not_found");
    println!("session: {}", terminal_inline(session_id.as_str()));
    println!("{label}_page: {}", request.page);
    if let Some(turn_id) = request.turn_filter.as_deref() {
        println!("{label}_turn_filter: {}", terminal_inline(turn_id));
    }
    if let Some(kind) = request.kind_filter.as_deref() {
        println!("{label}_kind_filter: {}", terminal_inline(kind));
    }
    if let Some(point) = request.point_filter.as_deref() {
        println!("{label}_point_filter: {}", terminal_inline(point));
    }
    println!("state_db_candidates: {}", candidates.len());
    Ok(())
}

pub(in crate::chat) fn print_replay_status_for_human(
    label: &str,
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    verbosity: TimelineVerbosity,
    request: TimelineRequest,
) -> Result<()> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let candidates = state_db_candidates(config, paths, workspace, runtime)?;
    for state_db in &candidates {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if !timeline_request_requires_full_replay(&request) {
            if let Some(replay) =
                store.replay_session_page(&session_id, request.page, replay_page_size(verbosity))?
            {
                let continuations = store.continuations(&session_id)?;
                println!("• {}", human_replay_label(label));
                println!("  session: {}", terminal_inline(session_id.as_str()));
                println!("  entries: {}", replay.total_entries);
                println!("  events: {}", replay.total_agent_events);
                println!("  approvals: {}", replay.total_approvals);
                println!("  continuations: {}", continuations.len());
                println!("  page: {}", request.page);
                print_human_recent_entries(replay.entries.iter(), timeline_entry_limit(verbosity));
                print_human_recent_events(
                    replay.agent_events.iter(),
                    timeline_event_limit(verbosity),
                );
                return Ok(());
            }
            continue;
        }
        if let Some(replay) = store.replay_session(&session_id)? {
            let continuations = store.continuations(&session_id)?;
            let entries = if request.kind_filter.is_some() || request.point_filter.is_some() {
                Vec::new()
            } else {
                filtered_entries_for_timeline(&replay, request.turn_filter.as_deref())
            };
            let events = filtered_events_for_timeline(&replay, &request);
            println!("• {}", human_replay_label(label));
            println!("  session: {}", terminal_inline(session_id.as_str()));
            println!("  entries: {}", replay.entries.len());
            println!("  events: {}", replay.agent_events.len());
            println!("  approvals: {}", replay.approvals.len());
            println!("  continuations: {}", continuations.len());
            print_human_request_filters(label, &request);
            print_human_recent_entries(entries, timeline_entry_limit(verbosity));
            print_human_recent_events(events, timeline_event_limit(verbosity));
            return Ok(());
        }
    }
    println!("• {}", human_replay_label(label));
    println!("  session: {}", terminal_inline(session_id.as_str()));
    println!("  status: not found");
    print_human_request_filters(label, &request);
    println!("  state databases checked: {}", candidates.len());
    Ok(())
}

pub(in crate::chat) fn print_trace_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    request: TimelineRequest,
) -> Result<()> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let candidates = state_db_candidates(config, paths, workspace, runtime)?;
    for state_db in &candidates {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if let Some(replay) = store.replay_session(&session_id)? {
            print_trace_command(&request);
            println!("trace: found");
            println!("session: {}", terminal_inline(session_id.as_str()));
            println!("state_db: {}", path_display(state_db));
            if let Some(turn_id) = request.turn_filter.as_deref() {
                let filtered_events = filtered_events(&replay, turn_id);
                println!("trace_turn_filter: {}", terminal_inline(turn_id));
                println!(
                    "trace_turn: {}",
                    if filtered_events.is_empty() {
                        "not_found"
                    } else {
                        "found"
                    }
                );
            }
            if let Some(kind) = request.kind_filter.as_deref() {
                println!("trace_kind_filter: {}", terminal_inline(kind));
            }
            if let Some(point) = request.point_filter.as_deref() {
                println!("trace_point_filter: {}", terminal_inline(point));
            }
            let filtered_events = filtered_events_for_timeline(&replay, &request);
            println!("filtered_agent_events: {}", filtered_events.len());
            print_trace_summary(&replay, &request);
            return Ok(());
        }
    }
    print_trace_command(&request);
    println!("trace: not_found");
    println!("session: {}", terminal_inline(session_id.as_str()));
    if let Some(turn_id) = request.turn_filter.as_deref() {
        println!("trace_turn_filter: {}", terminal_inline(turn_id));
    }
    if let Some(kind) = request.kind_filter.as_deref() {
        println!("trace_kind_filter: {}", terminal_inline(kind));
    }
    if let Some(point) = request.point_filter.as_deref() {
        println!("trace_point_filter: {}", terminal_inline(point));
    }
    println!("state_db_candidates: {}", candidates.len());
    Ok(())
}

pub(in crate::chat) fn print_trace_status_for_human(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    request: TimelineRequest,
) -> Result<()> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let candidates = state_db_candidates(config, paths, workspace, runtime)?;
    for state_db in &candidates {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if let Some(replay) = store.replay_session(&session_id)? {
            let events = filtered_events_for_timeline(&replay, &request);
            let mut counts = BTreeMap::<&'static str, usize>::new();
            let mut spans = BTreeSet::<String>::new();
            for event in &events {
                *counts.entry(trace_event_category(&event.kind)).or_default() += 1;
                spans.insert(event.turn_id.to_string());
            }
            println!("• Trace");
            println!("  session: {}", terminal_inline(session_id.as_str()));
            print_human_request_filters("trace", &request);
            println!("  spans: {}", spans.len());
            println!("  events: {}", events.len());
            println!("  categories: {}", human_trace_counts(&counts));
            print_human_recent_events(events, 8);
            return Ok(());
        }
    }
    println!("• Trace");
    println!("  session: {}", terminal_inline(session_id.as_str()));
    println!("  status: not found");
    print_human_request_filters("trace", &request);
    println!("  state databases checked: {}", candidates.len());
    Ok(())
}

fn print_trace_command(request: &TimelineRequest) {
    let mut parts = vec!["/trace".to_owned()];
    if let Some(turn_id) = request.turn_filter.as_deref() {
        parts.push(terminal_inline(turn_id));
    }
    if let Some(kind) = request.kind_filter.as_deref() {
        parts.push("--kind".into());
        parts.push(terminal_inline(kind));
    }
    if let Some(point) = request.point_filter.as_deref() {
        parts.push(match point {
            "failed" => "--failed".into(),
            "approval" => "--approval".into(),
            other => format!("--{}", terminal_inline(other)),
        });
    }
    println!("trace_command: {}", parts.join(" "));
}

fn print_trace_summary(replay: &SessionReplay, request: &TimelineRequest) {
    let mut global_counts = BTreeMap::<&'static str, usize>::new();
    let mut spans = BTreeMap::<String, BTreeMap<&'static str, usize>>::new();
    let events = filtered_events_for_timeline(replay, request);
    for event in &events {
        let category = trace_event_category(&event.kind);
        *global_counts.entry(category).or_default() += 1;
        *spans
            .entry(event.turn_id.to_string())
            .or_default()
            .entry(category)
            .or_default() += 1;
    }
    println!("trace_spans: {}", spans.len());
    println!(
        "trace_event_counts: {}",
        format_trace_counts(&global_counts)
    );
    if spans.is_empty() {
        println!("trace_cells: none");
        return;
    }
    println!("trace_cells:");
    let start = spans.len().saturating_sub(5);
    for (turn_id, counts) in spans.into_iter().skip(start) {
        let events = counts.values().sum::<usize>();
        let cell = WorkbenchCell {
            kind: WorkbenchCellKind::Session,
            title: format!("trace span turn={}", terminal_inline(&turn_id)),
            detail: format!(
                "events={events} correlation={} {}",
                workbench_correlation_id(replay.session.session_id.as_str(), &turn_id),
                format_trace_counts(&counts)
            ),
        };
        println!("- {}", cell.render());
    }
    println!("trace_events:");
    let start = events.len().saturating_sub(8);
    for cell in timeline_event_cells(&events[start..]) {
        println!("- {}", cell.render());
    }
}

fn workbench_correlation_id(session_id: &str, turn_id: &str) -> String {
    format!(
        "session:{}:turn:{}",
        terminal_inline(session_id),
        terminal_inline(turn_id)
    )
}

fn trace_event_category(kind: &AgentEventKind) -> &'static str {
    match kind {
        AgentEventKind::ModelStream(_) | AgentEventKind::ModelDiagnostic(_) => "model",
        AgentEventKind::ToolCallStarted
        | AgentEventKind::ToolCallOutputDelta
        | AgentEventKind::ToolCallCompleted
        | AgentEventKind::ToolCallFailed
        | AgentEventKind::ToolCallCancelled => "tool",
        AgentEventKind::ContextDiff | AgentEventKind::ContextCompacted => "context",
        AgentEventKind::MemoryLifecycle => "memory",
        AgentEventKind::CodingTurn => "coding",
        AgentEventKind::AuditAnchor => "audit",
        AgentEventKind::ContinuationStarted
        | AgentEventKind::ContinuationCompleted
        | AgentEventKind::ContinuationFailed
        | AgentEventKind::ContinuationCancelled => "continuation",
        AgentEventKind::ApprovalRequested | AgentEventKind::ApprovalResolved => "approval",
        AgentEventKind::Error => "error",
        AgentEventKind::SessionStart
        | AgentEventKind::TurnStart
        | AgentEventKind::UserMessage
        | AgentEventKind::TurnEnd => "session",
    }
}

fn format_trace_counts(counts: &BTreeMap<&'static str, usize>) -> String {
    [
        "session",
        "model",
        "tool",
        "context",
        "memory",
        "coding",
        "audit",
        "continuation",
        "approval",
        "error",
    ]
    .into_iter()
    .map(|category| format!("{category}={}", counts.get(category).copied().unwrap_or(0)))
    .collect::<Vec<_>>()
    .join(" ")
}

fn print_recent_timeline(
    replay: &SessionReplay,
    verbosity: TimelineVerbosity,
    request: &TimelineRequest,
) {
    let event_limit = timeline_event_limit(verbosity);
    let entry_limit = timeline_entry_limit(verbosity);
    let entries = if request.kind_filter.is_some() || request.point_filter.is_some() {
        Vec::new()
    } else {
        filtered_entries_for_timeline(replay, request.turn_filter.as_deref())
    };
    let events = filtered_events_for_timeline(replay, request);
    if !entries.is_empty() {
        println!("recent_entries:");
        let (start, end) = paged_window(entries.len(), entry_limit, request.page);
        for entry in &entries[start..end] {
            println!("- {}", session_entry_cell(entry).render());
        }
    }
    print_coding_groups(events.iter().copied());
    if !events.is_empty() {
        println!("recent_events:");
        let (start, end) = paged_window(events.len(), event_limit, request.page);
        for cell in timeline_event_cells(&events[start..end]) {
            println!("- {}", cell.render());
        }
    }
}

fn print_recent_timeline_page(
    replay: &SessionReplayPage,
    verbosity: TimelineVerbosity,
    all_events: &[ikaros_session::AgentEvent],
) {
    let entry_limit = timeline_entry_limit(verbosity);
    let event_limit = timeline_event_limit(verbosity);
    if !replay.entries.is_empty() {
        println!("recent_entries:");
        for entry in replay.entries.iter().take(entry_limit) {
            println!("- {}", session_entry_cell(entry).render());
        }
    }
    print_coding_groups(all_events.iter());
    if !all_events.is_empty() {
        println!("recent_events:");
        let (start, end) = paged_window(all_events.len(), event_limit, replay.page);
        let events = all_events[start..end].iter().collect::<Vec<_>>();
        for cell in timeline_event_cells(&events) {
            println!("- {}", cell.render());
        }
    }
}

fn timeline_event_cells(events: &[&AgentEvent]) -> Vec<WorkbenchCell> {
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
            ikaros_models::ModelStreamEvent::TextDelta(_) => text_delta_chunks += 1,
            ikaros_models::ModelStreamEvent::ReasoningDelta(_) => reasoning_delta_chunks += 1,
            ikaros_models::ModelStreamEvent::RefusalDelta(_) => refusal_delta_chunks += 1,
            ikaros_models::ModelStreamEvent::ToolCallStart { .. }
            | ikaros_models::ModelStreamEvent::ToolCallDelta { .. }
            | ikaros_models::ModelStreamEvent::ToolCallEnd { .. } => tool_call_events += 1,
            ikaros_models::ModelStreamEvent::Usage(_) => usage_events += 1,
            ikaros_models::ModelStreamEvent::Error { .. } => error_events += 1,
            ikaros_models::ModelStreamEvent::Done => done_events += 1,
            ikaros_models::ModelStreamEvent::Start { .. } => {}
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

fn human_replay_label(label: &str) -> &'static str {
    match label {
        "replay" => "Replay",
        "debug" => "Debug Timeline",
        _ => "Timeline",
    }
}

fn print_human_request_filters(label: &str, request: &TimelineRequest) {
    if let Some(turn_id) = request.turn_filter.as_deref() {
        println!("  {label} turn: {}", terminal_inline(turn_id));
    }
    if let Some(kind) = request.kind_filter.as_deref() {
        println!("  {label} kind: {}", terminal_inline(kind));
    }
    if let Some(point) = request.point_filter.as_deref() {
        println!("  {label} point: {}", terminal_inline(point));
    }
}

fn print_human_recent_entries<'a>(
    entries: impl IntoIterator<Item = &'a SessionEntry>,
    limit: usize,
) {
    let entries = entries.into_iter().collect::<Vec<_>>();
    if entries.is_empty() {
        println!("  recent entries: none");
        return;
    }
    println!("  recent entries:");
    let start = entries.len().saturating_sub(limit.max(1));
    for entry in &entries[start..] {
        println!(
            "  • {}: {}",
            human_session_entry_role(entry),
            terminal_inline(&single_line_excerpt(
                entry.visible_text.as_deref().unwrap_or("none"),
                96,
            ))
        );
    }
}

fn print_human_recent_events<'a>(events: impl IntoIterator<Item = &'a AgentEvent>, limit: usize) {
    let events = events.into_iter().collect::<Vec<_>>();
    if events.is_empty() {
        println!("  recent events: none");
        return;
    }
    println!("  recent events:");
    let start = events.len().saturating_sub(limit.max(1));
    for event in &events[start..] {
        println!(
            "  • {}: turn {}",
            human_agent_event_label(&event.kind),
            terminal_inline(event.turn_id.as_str())
        );
    }
}

fn human_session_entry_role(entry: &SessionEntry) -> &'static str {
    match entry.kind {
        SessionEntryKind::AssistantMessage => "assistant",
        SessionEntryKind::UserMessage => "user",
        _ => "entry",
    }
}

fn human_agent_event_label(kind: &AgentEventKind) -> &'static str {
    match kind {
        AgentEventKind::ModelStream(_) => "model stream",
        AgentEventKind::ModelDiagnostic(_) => "model diagnostic",
        AgentEventKind::ToolCallStarted => "tool started",
        AgentEventKind::ToolCallOutputDelta => "tool output",
        AgentEventKind::ToolCallCompleted => "tool completed",
        AgentEventKind::ToolCallFailed => "tool failed",
        AgentEventKind::ToolCallCancelled => "tool cancelled",
        AgentEventKind::ContextDiff => "context updated",
        AgentEventKind::ContextCompacted => "context compacted",
        AgentEventKind::MemoryLifecycle => "memory lifecycle",
        AgentEventKind::CodingTurn => "coding",
        AgentEventKind::AuditAnchor => "audit",
        AgentEventKind::ContinuationStarted => "continuation started",
        AgentEventKind::ContinuationCompleted => "continuation completed",
        AgentEventKind::ContinuationFailed => "continuation failed",
        AgentEventKind::ContinuationCancelled => "continuation cancelled",
        AgentEventKind::ApprovalRequested => "approval requested",
        AgentEventKind::ApprovalResolved => "approval resolved",
        AgentEventKind::Error => "error",
        AgentEventKind::SessionStart => "session started",
        AgentEventKind::TurnStart => "turn started",
        AgentEventKind::UserMessage => "user message",
        AgentEventKind::TurnEnd => "turn ended",
    }
}

fn human_trace_counts(counts: &BTreeMap<&'static str, usize>) -> String {
    let parts = [
        "session",
        "model",
        "tool",
        "context",
        "memory",
        "coding",
        "audit",
        "continuation",
        "approval",
        "error",
    ]
    .into_iter()
    .filter_map(|category| {
        let count = counts.get(category).copied().unwrap_or_default();
        (count > 0).then(|| format!("{category} {count}"))
    })
    .collect::<Vec<_>>();
    if parts.is_empty() {
        "none".into()
    } else {
        parts.join(", ")
    }
}

fn single_line_excerpt(input: &str, max_chars: usize) -> String {
    let normalized = input.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut output = String::new();
    for (index, ch) in normalized.chars().enumerate() {
        if index >= max_chars {
            output.push_str("...");
            return output;
        }
        output.push(ch);
    }
    if output.is_empty() {
        "none".into()
    } else {
        output
    }
}

fn timeline_request_requires_full_replay(request: &TimelineRequest) -> bool {
    request.turn_filter.is_some() || request.kind_filter.is_some() || request.point_filter.is_some()
}

fn replay_page_size(verbosity: TimelineVerbosity) -> usize {
    timeline_entry_limit(verbosity)
}

fn timeline_event_limit(verbosity: TimelineVerbosity) -> usize {
    match verbosity {
        TimelineVerbosity::Timeline => 12,
        TimelineVerbosity::Replay => 16,
        TimelineVerbosity::Debug => 20,
    }
}

fn timeline_entry_limit(verbosity: TimelineVerbosity) -> usize {
    match verbosity {
        TimelineVerbosity::Timeline => 3,
        TimelineVerbosity::Replay | TimelineVerbosity::Debug => 8,
    }
}

fn paged_window(len: usize, limit: usize, page: usize) -> (usize, usize) {
    if len == 0 {
        return (0, 0);
    }
    let limit = limit.max(1);
    let page = page.max(1);
    let end = len.saturating_sub(limit.saturating_mul(page.saturating_sub(1)));
    let start = end.saturating_sub(limit);
    (start, end)
}

fn print_coding_groups<'a>(events: impl IntoIterator<Item = &'a ikaros_session::AgentEvent>) {
    let cells = coding_event_cells(events);
    if cells.is_empty() {
        return;
    }
    for group in ["progress", "diff", "test", "review"] {
        let group_cells = cells
            .iter()
            .filter(|(candidate, _)| *candidate == group)
            .collect::<Vec<_>>();
        if group_cells.is_empty() {
            continue;
        }
        println!("coding_group: {group} count={}", group_cells.len());
        let start = group_cells.len().saturating_sub(3);
        for (_, cell) in &group_cells[start..] {
            println!("- {}", cell.render());
        }
    }
}

fn filtered_entries<'a>(
    replay: &'a SessionReplay,
    turn_id: &str,
) -> Vec<&'a ikaros_session::SessionEntry> {
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

fn filtered_events<'a>(
    replay: &'a SessionReplay,
    turn_id: &str,
) -> Vec<&'a ikaros_session::AgentEvent> {
    replay
        .agent_events
        .iter()
        .filter(|event| event.turn_id.as_str() == turn_id)
        .collect()
}

fn filtered_entries_for_timeline<'a>(
    replay: &'a SessionReplay,
    turn_filter: Option<&str>,
) -> Vec<&'a ikaros_session::SessionEntry> {
    match turn_filter {
        Some(turn_id) => filtered_entries(replay, turn_id),
        None => replay.entries.iter().collect(),
    }
}

fn filtered_events_for_trace<'a>(
    replay: &'a SessionReplay,
    turn_filter: Option<&str>,
) -> Vec<&'a ikaros_session::AgentEvent> {
    match turn_filter {
        Some(turn_id) => filtered_events(replay, turn_id),
        None => replay.agent_events.iter().collect(),
    }
}

fn filtered_events_for_timeline<'a>(
    replay: &'a SessionReplay,
    request: &TimelineRequest,
) -> Vec<&'a ikaros_session::AgentEvent> {
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

pub(super) fn timeline_point_matches(kind: &AgentEventKind, point: &str) -> bool {
    match point {
        "failed" => {
            matches!(
                kind,
                AgentEventKind::Error
                    | AgentEventKind::ToolCallFailed
                    | AgentEventKind::ContinuationFailed
                    | AgentEventKind::ModelStream(ikaros_models::ModelStreamEvent::Error { .. })
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
