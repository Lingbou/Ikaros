// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_state::session::{SessionId, SessionReplay, SessionStore, SqliteSessionStore};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use super::super::super::{WorkbenchCell, WorkbenchCellKind, path_display, terminal_inline};
use super::super::state_db_candidates;
use super::{
    classification::{format_trace_counts, trace_event_category},
    events::timeline_event_cells,
    filter::{filtered_events, filtered_events_for_timeline, filtered_events_for_trace},
    human::{human_trace_counts, print_human_recent_events, print_human_request_filters},
    request::TimelineRequest,
};
pub(in crate::chat) fn print_screen_trace_snapshot(
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
            println!("* Trace");
            println!("  session: {}", terminal_inline(session_id.as_str()));
            print_human_request_filters("trace", &request);
            println!("  spans: {}", spans.len());
            println!("  events: {}", events.len());
            println!("  categories: {}", human_trace_counts(&counts));
            print_human_recent_events(events, 8);
            return Ok(());
        }
    }
    println!("* Trace");
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
