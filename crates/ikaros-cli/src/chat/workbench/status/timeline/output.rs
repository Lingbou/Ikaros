// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_state::session::{
    SessionId, SessionReplay, SessionReplayPage, SessionStore, SqliteSessionStore,
};
use std::path::Path;

use super::super::super::{coding_event_cells, path_display, session_entry_cell, terminal_inline};
use super::super::state_db_candidates;
use super::{
    events::timeline_event_cells,
    filter::{
        filtered_entries, filtered_entries_for_timeline, filtered_events,
        filtered_events_for_timeline, paged_window, replay_page_size, timeline_entry_limit,
        timeline_event_limit, timeline_request_requires_full_replay,
    },
    human::{
        human_replay_label, print_human_recent_entries, print_human_recent_events,
        print_human_request_filters,
    },
    request::{TimelineRequest, TimelineVerbosity},
};
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
                println!("* {}", human_replay_label(label));
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
            println!("* {}", human_replay_label(label));
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
    println!("* {}", human_replay_label(label));
    println!("  session: {}", terminal_inline(session_id.as_str()));
    println!("  status: not found");
    print_human_request_filters(label, &request);
    println!("  state databases checked: {}", candidates.len());
    Ok(())
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
    all_events: &[ikaros_state::session::AgentEvent],
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

fn print_coding_groups<'a>(
    events: impl IntoIterator<Item = &'a ikaros_state::session::AgentEvent>,
) {
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
