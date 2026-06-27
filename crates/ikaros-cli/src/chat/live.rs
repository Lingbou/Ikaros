// SPDX-License-Identifier: GPL-3.0-only

use super::{
    interactive::InteractiveChatRuntime,
    interactive_chat_turn_error_actions, interactive_chat_turn_error_kind,
    terminal::{RunningTurnTerminal, normalize_raw_terminal_newlines},
    workbench,
};
use anyhow::Result;
use crossterm::terminal::size as terminal_size;
use ikaros_core::{IkarosError, redact_secrets};
use ikaros_models::ModelStreamEvent;
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, SessionId, SessionStore,
    SqliteSessionStore, TurnId,
};
use ikaros_tui::{
    TerminalStreamRenderer, ToolActivity, ToolActivityStatus,
    render_tool_activity as render_tui_tool_activity,
};
use std::{
    env,
    io::{self, IsTerminal, Write},
    sync::{Arc, Mutex},
};

pub(super) fn print_live_event_cells(
    runtime: &InteractiveChatRuntime,
    turn_id: &TurnId,
) -> Result<()> {
    if runtime.machine_stdout_quiet() {
        return Ok(());
    }
    let store = SqliteSessionStore::new(&runtime.state_dir);
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let Some(replay) = store.replay_session(&session_id)? else {
        println!("live_cells: 0");
        return Ok(());
    };
    let events = replay
        .agent_events
        .iter()
        .filter(|event| event.turn_id.as_str() == turn_id.as_str())
        .collect::<Vec<_>>();
    let include_debug_internal_events = verbose_live_cells_enabled();
    let visible_events = events
        .iter()
        .copied()
        .filter(|event| live_cell_event_visible(&event.kind, include_debug_internal_events))
        .collect::<Vec<_>>();
    let cells = compact_live_event_cells_with_debug(&events, include_debug_internal_events);
    println!(
        "live_cells: compact_cells={} total_events={} visible_events={} detail={}",
        cells.len(),
        events.len(),
        visible_events.len(),
        if verbose_live_cells_enabled() {
            "expanded"
        } else {
            "json"
        }
    );
    println!("{}", live_cell_summary_line(&events, &visible_events));
    println!("{}", live_cells_json_line(&events, &visible_events, &cells));
    if verbose_live_cells_enabled() {
        for cell in &cells {
            println!("- {}", cell.render());
        }
    }
    Ok(())
}

fn verbose_live_cells_enabled() -> bool {
    env::var_os("IKAROS_WORKBENCH_VERBOSE_LIVE_CELLS").is_some()
}

fn debug_context_activity_enabled() -> bool {
    verbose_live_cells_enabled() || env::var_os("IKAROS_WORKBENCH_CONTEXT_ACTIVITY").is_some()
}

#[derive(Clone, Default)]
pub(super) struct WorkbenchLiveEventSink {
    events: Arc<Mutex<Vec<AgentEvent>>>,
    snapshots: Arc<Mutex<Vec<String>>>,
    stdout_updates: bool,
    text_delta_stdout: bool,
    human_activity_stdout: bool,
    running_terminal: Option<RunningTurnTerminal>,
    agent_text_state: Arc<Mutex<AgentTextDeltaState>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentTextDeltaState {
    started: bool,
    activity_since_last_text: bool,
    has_pending_source: bool,
    last_delta_ended_with_newline: bool,
    pending_source: String,
    renderer: TerminalStreamRenderer,
}

impl Default for AgentTextDeltaState {
    fn default() -> Self {
        Self {
            started: false,
            activity_since_last_text: false,
            has_pending_source: false,
            last_delta_ended_with_newline: false,
            pending_source: String::new(),
            renderer: TerminalStreamRenderer::new(assistant_stream_body_width()),
        }
    }
}

impl WorkbenchLiveEventSink {
    pub(super) fn with_text_delta_stdout_and_terminal(
        text_delta_stdout: bool,
        running_terminal: Option<RunningTurnTerminal>,
    ) -> Self {
        Self {
            text_delta_stdout,
            human_activity_stdout: text_delta_stdout,
            running_terminal,
            ..Self::default()
        }
    }

    fn has_error_event_for_turn(&self, turn_id: &TurnId) -> ikaros_core::Result<bool> {
        self.events
            .lock()
            .map_err(|_| IkarosError::Message("workbench live event lock is poisoned".into()))
            .map(|events| {
                events.iter().any(|event| {
                    event.turn_id.as_str() == turn_id.as_str()
                        && matches!(event.kind, AgentEventKind::Error)
                })
            })
    }

    #[cfg(test)]
    pub(super) fn snapshots(&self) -> ikaros_core::Result<Vec<String>> {
        self.snapshots
            .lock()
            .map(|snapshots| snapshots.clone())
            .map_err(|_| IkarosError::Message("workbench live snapshot lock is poisoned".into()))
    }
}

pub(super) fn emit_interactive_chat_turn_failure_evidence(
    sink: &dyn AgentEventSink,
    live_sink: &WorkbenchLiveEventSink,
    session_id: &SessionId,
    turn_id: &TurnId,
    error: &anyhow::Error,
) -> ikaros_core::Result<()> {
    if live_sink.has_error_event_for_turn(turn_id)? {
        return Ok(());
    }
    let message = error.to_string();
    let error_kind = interactive_chat_turn_error_kind(&message);
    sink.emit(&AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        None,
        AgentEventSource::Runtime,
        AgentEventKind::Error,
        serde_json::json!({
            "phase": "interactive_chat_turn",
            "message": redact_secrets(&message),
            "error_kind": error_kind,
            "recoverable": error_kind != "unknown",
            "actions": interactive_chat_turn_error_actions(error_kind, &message),
        }),
    ))?;
    sink.emit(&AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        None,
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        serde_json::json!({
            "status": "failed",
            "phase": "interactive_chat_turn",
            "error_kind": error_kind,
        }),
    ))
}

impl AgentEventSink for WorkbenchLiveEventSink {
    fn emit(&self, event: &AgentEvent) -> ikaros_core::Result<()> {
        let snapshot = {
            let mut events = self.events.lock().map_err(|_| {
                IkarosError::Message("workbench live event lock is poisoned".into())
            })?;
            events.push(event.clone());
            live_cell_snapshot(&events)
        };
        self.snapshots
            .lock()
            .map_err(|_| IkarosError::Message("workbench live snapshot lock is poisoned".into()))?
            .push(snapshot.clone());
        if self.stdout_updates {
            println!("live_cell_update:");
            for line in snapshot.lines() {
                println!("{line}");
            }
        }
        if self.text_delta_stdout {
            match &event.kind {
                AgentEventKind::ModelStream(ModelStreamEvent::TextDelta(text)) => {
                    if let Ok(mut state) = self.agent_text_state.lock() {
                        let color_bullet = !state.started && io::stdout().is_terminal();
                        let rendered = format_agent_text_delta(text, &mut state);
                        self.print_human_output(
                            &crate::chat::output::color_assistant_bullet_for_terminal(
                                &rendered,
                                color_bullet,
                            ),
                        );
                    }
                }
                AgentEventKind::ModelStream(ModelStreamEvent::Done) => {
                    if let Ok(mut state) = self.agent_text_state.lock() {
                        let color_bullet = !state.started && io::stdout().is_terminal();
                        let rendered = finish_agent_text_delta(&mut state);
                        self.print_human_output(
                            &crate::chat::output::color_assistant_bullet_for_terminal(
                                &rendered,
                                color_bullet,
                            ),
                        );
                    }
                }
                _ => {}
            }
        }
        if self.human_activity_stdout {
            if let Some(lines) = human_activity_lines(event) {
                if let Ok(mut state) = self.agent_text_state.lock() {
                    if state.started || state.has_pending_source {
                        let color_bullet = !state.started && io::stdout().is_terminal();
                        let rendered = finish_agent_text_delta(&mut state);
                        self.print_human_output(&format!(
                            "{}\n",
                            crate::chat::output::color_assistant_bullet_for_terminal(
                                &rendered,
                                color_bullet,
                            )
                        ));
                        *state = AgentTextDeltaState::default();
                    }
                }
                self.print_human_activity_lines(&lines);
                if let Ok(mut state) = self.agent_text_state.lock() {
                    state.activity_since_last_text = true;
                }
            }
        }
        Ok(())
    }
}

impl WorkbenchLiveEventSink {
    fn print_human_output(&self, text: &str) {
        if let Some(terminal) = &self.running_terminal
            && terminal.print_output(text).is_ok()
        {
            return;
        }
        if io::stdout().is_terminal() {
            print!("{}", normalize_raw_terminal_newlines(text));
        } else {
            print!("{text}");
        }
        let _ = std::io::stdout().flush();
    }

    fn print_human_activity_lines(&self, lines: &[String]) {
        let rendered = lines
            .iter()
            .map(|line| human_activity_line_for_terminal(line, io::stdout().is_terminal()))
            .collect::<Vec<_>>();
        if let Some(terminal) = &self.running_terminal {
            let mut text = rendered.join("\n");
            text.push('\n');
            if terminal.print_output(&text).is_ok() {
                return;
            }
        }
        for line in rendered {
            println!("{line}");
        }
    }
}

fn terminal_width() -> usize {
    terminal_size()
        .map(|(width, _)| usize::from(width).max(24))
        .unwrap_or(80)
}

fn activity_answer_separator_line(width: usize) -> String {
    "─".repeat(width.max(24))
}

fn assistant_stream_body_width() -> usize {
    terminal_width().saturating_sub(2).max(1)
}

fn format_agent_text_delta(text: &str, state: &mut AgentTextDeltaState) -> String {
    let text = workbench::terminal_message(text);
    if text.is_empty() {
        return String::new();
    }

    state.has_pending_source = true;
    state.last_delta_ended_with_newline = text.ends_with('\n');
    state.pending_source.push_str(&text);
    let stable_source = drain_stable_agent_stream_source(state);
    if stable_source.is_empty() {
        return String::new();
    }
    let lines = state.renderer.push_delta(&stable_source);
    render_agent_stream_lines(&lines, state, true)
}

fn finish_agent_text_delta(state: &mut AgentTextDeltaState) -> String {
    if !state.started && !state.has_pending_source {
        return String::new();
    }

    let terminate_last_line = state.last_delta_ended_with_newline;
    let pending_source = std::mem::take(&mut state.pending_source);
    let mut lines = if pending_source.is_empty() {
        Vec::new()
    } else {
        state.renderer.push_delta(&pending_source)
    };
    let finish = state.renderer.finish_with_source();
    lines.extend(finish.lines);
    let _raw_markdown_source = finish.raw_markdown;
    let output = render_agent_stream_lines(&lines, state, terminate_last_line);
    state.started = false;
    state.has_pending_source = false;
    state.last_delta_ended_with_newline = false;
    output
}

fn render_agent_stream_lines(
    lines: &[String],
    state: &mut AgentTextDeltaState,
    terminate_last_line: bool,
) -> String {
    if lines.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    let mut first_stream_line = !state.started;
    if first_stream_line {
        if state.activity_since_last_text {
            output.push('\n');
            output.push_str(&activity_answer_separator_line(terminal_width()));
            output.push_str("\n\n");
            state.activity_since_last_text = false;
        }
        state.started = true;
    }

    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() {
            if first_stream_line {
                output.push_str("• ");
            }
        } else if first_stream_line {
            output.push_str("• ");
        } else {
            output.push_str("  ");
        }
        output.push_str(line);

        if index + 1 < lines.len() || terminate_last_line {
            output.push('\n');
        }
        first_stream_line = false;
    }

    output
}

fn drain_stable_agent_stream_source(state: &mut AgentTextDeltaState) -> String {
    let stable_len = stable_agent_stream_source_len(&state.pending_source);
    if stable_len == 0 {
        return String::new();
    }
    state.pending_source.drain(..stable_len).collect()
}

fn stable_agent_stream_source_len(source: &str) -> usize {
    let mut offset = 0usize;
    let mut stable_end = 0usize;
    let mut fence: Option<AgentStreamFenceState> = None;

    for raw_line in source.split_inclusive('\n') {
        if !raw_line.ends_with('\n') {
            break;
        }

        let line = raw_line.trim_end_matches(['\r', '\n']);
        advance_agent_stream_fence(&mut fence, line);
        offset = offset.saturating_add(raw_line.len());
        if fence.is_none() {
            stable_end = offset;
        }
    }

    stable_end
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AgentStreamFenceState {
    marker: char,
    len: usize,
}

fn advance_agent_stream_fence(fence: &mut Option<AgentStreamFenceState>, line: &str) {
    let leading_spaces = line
        .as_bytes()
        .iter()
        .take_while(|byte| **byte == b' ')
        .count();
    if leading_spaces > 3 {
        return;
    }

    let trimmed = line[leading_spaces..].trim_start_matches('>');
    let Some((marker, len)) = parse_agent_stream_fence_marker(trimmed.trim_start()) else {
        return;
    };

    if let Some(open) = fence {
        let closes_open_fence = marker == open.marker
            && len >= open.len
            && trimmed.trim_start()[len..].trim().is_empty();
        if closes_open_fence {
            *fence = None;
        }
    } else {
        *fence = Some(AgentStreamFenceState { marker, len });
    }
}

fn parse_agent_stream_fence_marker(line: &str) -> Option<(char, usize)> {
    let marker = line.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let len = line.chars().take_while(|ch| *ch == marker).count();
    (len >= 3).then_some((marker, len))
}

pub(in crate::chat) fn human_activity_lines(event: &AgentEvent) -> Option<Vec<String>> {
    human_activity_lines_with_debug_context(event, debug_context_activity_enabled())
}

pub(super) fn human_activity_lines_with_debug_context(
    event: &AgentEvent,
    include_debug_context: bool,
) -> Option<Vec<String>> {
    match &event.kind {
        AgentEventKind::ToolCallCompleted => {
            let name = event_payload_str(&event.payload, "name").unwrap_or("tool");
            let activity = ToolActivity::new(name, ToolActivityStatus::Completed);
            Some(render_tui_tool_activity(&activity_with_payload_detail(
                activity,
                &event.payload,
            )))
        }
        AgentEventKind::ToolCallFailed => {
            let name = event_payload_str(&event.payload, "name").unwrap_or("tool");
            let activity = ToolActivity::new(name, ToolActivityStatus::Failed);
            Some(render_tui_tool_activity(&activity_with_payload_detail(
                activity,
                &event.payload,
            )))
        }
        AgentEventKind::ToolCallCancelled => {
            let name = event_payload_str(&event.payload, "name").unwrap_or("tool");
            let activity = ToolActivity::new(name, ToolActivityStatus::Cancelled);
            Some(render_tui_tool_activity(&activity))
        }
        AgentEventKind::ContextDiff if include_debug_context => {
            let sections = json_array_len(&event.payload, "sections");
            let references = json_array_len(&event.payload, "references");
            if sections == 0 && references == 0 {
                return None;
            }
            Some(vec![
                "• Gathered context".into(),
                format!("  • {sections} section(s), {references} reference(s)"),
            ])
        }
        AgentEventKind::ContextCompacted if include_debug_context => {
            let sections = json_array_len(&event.payload, "compressed_sections");
            Some(vec![
                "• Compacted context".into(),
                format!("  • {sections} section(s) compressed"),
            ])
        }
        AgentEventKind::CodingTurn => {
            let summary = event_payload_str(&event.payload, "summary")
                .or_else(|| event_payload_str(&event.payload, "status"))
                .unwrap_or("coding workflow updated");
            Some(vec![
                "• Updated code".into(),
                format!("  • {}", workbench::terminal_inline(summary)),
            ])
        }
        AgentEventKind::ApprovalRequested => Some(vec!["• Waiting for approval".into()]),
        AgentEventKind::Error => human_error_activity_lines(event),
        _ => None,
    }
}

fn human_error_activity_lines(event: &AgentEvent) -> Option<Vec<String>> {
    if event_payload_str(&event.payload, "phase") == Some("interactive_chat_turn") {
        return None;
    }
    let detail = event_payload_str(&event.payload, "message")
        .or_else(|| event_payload_str(&event.payload, "error"))
        .or_else(|| event_payload_str(&event.payload, "summary"))
        .or_else(|| event_payload_str(&event.payload, "detail"))
        .map(workbench::terminal_inline)
        .filter(|message| !message.trim().is_empty());
    let mut lines = vec!["• Error".to_owned()];
    if let Some(detail) = detail {
        lines.push(format!("  └ {detail}"));
    }
    Some(lines)
}

fn human_activity_line_for_terminal(line: &str, color: bool) -> String {
    if !color {
        return line.to_owned();
    }
    if let Some(rest) = line.strip_prefix("• ") {
        format!("\x1b[32m•\x1b[0m {rest}")
    } else if let Some(rest) = line.strip_prefix("  • ") {
        format!("  \x1b[36m•\x1b[0m {rest}")
    } else if let Some(rest) = line.strip_prefix("  └ ") {
        format!("  \x1b[36m└\x1b[0m {rest}")
    } else {
        line.to_owned()
    }
}

fn activity_with_payload_detail(
    activity: ToolActivity,
    payload: &serde_json::Value,
) -> ToolActivity {
    if let Some(summary) = event_payload_str(payload, "summary")
        .or_else(|| event_payload_str(payload, "error"))
        .filter(|summary| !summary.trim().is_empty())
    {
        activity.with_detail(summary)
    } else {
        activity
    }
}

fn event_payload_str<'a>(payload: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    payload.get(key).and_then(serde_json::Value::as_str)
}

fn live_cell_snapshot(events: &[AgentEvent]) -> String {
    let event_refs = events.iter().collect::<Vec<_>>();
    let visible_events = event_refs
        .iter()
        .copied()
        .filter(|event| default_live_cell_event(&event.kind))
        .collect::<Vec<_>>();
    let cells = compact_live_event_cells_with_debug(&event_refs, false);
    let mut lines = vec![
        format!("live_cells: {} total_events={}", cells.len(), events.len()),
        live_cell_summary_line(&event_refs, &visible_events),
        live_cells_json_line(&event_refs, &visible_events, &cells),
    ];
    lines.extend(cells.iter().map(|cell| format!("- {}", cell.render())));
    lines.join("\n")
}

#[cfg(test)]
pub(super) fn compact_live_event_cells(events: &[&AgentEvent]) -> Vec<workbench::WorkbenchCell> {
    compact_live_event_cells_with_debug(events, false)
}

pub(super) fn compact_live_event_cells_with_debug(
    events: &[&AgentEvent],
    include_debug_internal_events: bool,
) -> Vec<workbench::WorkbenchCell> {
    let mut cells = Vec::new();
    if let Some(cell) = model_stream_summary_cell(events) {
        cells.push(cell);
    }
    if let Some(cell) = tool_progress_summary_cell(events) {
        cells.push(cell);
    }
    if include_debug_internal_events {
        if let Some(cell) = context_progress_summary_cell(events) {
            cells.push(cell);
        }
    }
    let visible_events = events
        .iter()
        .copied()
        .filter(|event| live_cell_event_visible(&event.kind, include_debug_internal_events))
        .collect::<Vec<_>>();
    let remaining = 8usize.saturating_sub(cells.len());
    let start = visible_events.len().saturating_sub(remaining);
    for event in &visible_events[start..] {
        cells.push(workbench::agent_event_cell(event));
    }
    cells
}

fn model_stream_summary_cell(events: &[&AgentEvent]) -> Option<workbench::WorkbenchCell> {
    let mut provider = "unknown".to_owned();
    let mut model = "unknown".to_owned();
    let mut text_delta_chunks = 0usize;
    let mut reasoning_delta_chunks = 0usize;
    let mut refusal_delta_chunks = 0usize;
    let mut tool_call_events = 0usize;
    let mut errors = 0usize;
    let mut usage_total = None;
    let mut done = false;
    let mut seen = false;

    for event in events {
        let AgentEventKind::ModelStream(stream_event) = &event.kind else {
            continue;
        };
        seen = true;
        match stream_event {
            ModelStreamEvent::Start {
                provider: event_provider,
                model: event_model,
            } => {
                provider = workbench::terminal_inline(event_provider);
                model = workbench::terminal_inline(event_model);
            }
            ModelStreamEvent::TextDelta(_) => text_delta_chunks += 1,
            ModelStreamEvent::ReasoningDelta(_) => reasoning_delta_chunks += 1,
            ModelStreamEvent::RefusalDelta(_) => refusal_delta_chunks += 1,
            ModelStreamEvent::ToolCallStart { .. }
            | ModelStreamEvent::ToolCallDelta { .. }
            | ModelStreamEvent::ToolCallEnd { .. } => tool_call_events += 1,
            ModelStreamEvent::Usage(usage) => {
                usage_total = Some(usage.total_or_prompt_completion());
            }
            ModelStreamEvent::Error { .. } => errors += 1,
            ModelStreamEvent::Done => done = true,
        }
    }

    seen.then(|| workbench::WorkbenchCell {
        kind: workbench::WorkbenchCellKind::Model,
        title: "model stream summary".into(),
        detail: format!(
            "provider={} model={} text_delta_chunks={} reasoning_delta_chunks={} refusal_delta_chunks={} tool_call_events={} usage_total={} done={} errors={}",
            provider,
            model,
            text_delta_chunks,
            reasoning_delta_chunks,
            refusal_delta_chunks,
            tool_call_events,
            usage_total
                .map(|tokens| tokens.to_string())
                .unwrap_or_else(|| "unknown".into()),
            done,
            errors
        ),
    })
}

fn tool_progress_summary_cell(events: &[&AgentEvent]) -> Option<workbench::WorkbenchCell> {
    let mut started = 0usize;
    let mut output = 0usize;
    let mut completed = 0usize;
    let mut failed = 0usize;
    let mut cancelled = 0usize;
    let mut latest_tool = "unknown".to_owned();
    let mut latest_status = "unknown".to_owned();

    for event in events {
        let status = match &event.kind {
            AgentEventKind::ToolCallStarted => {
                started += 1;
                "started"
            }
            AgentEventKind::ToolCallOutputDelta => {
                output += 1;
                "output"
            }
            AgentEventKind::ToolCallCompleted => {
                completed += 1;
                "completed"
            }
            AgentEventKind::ToolCallFailed => {
                failed += 1;
                "failed"
            }
            AgentEventKind::ToolCallCancelled => {
                cancelled += 1;
                "cancelled"
            }
            _ => continue,
        };
        latest_status = event
            .payload
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(status)
            .to_owned();
        latest_tool = event
            .payload
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(workbench::terminal_inline)
            .unwrap_or_else(|| "unknown".into());
    }

    let total = started + output + completed + failed + cancelled;
    (total > 0).then(|| workbench::WorkbenchCell {
        kind: workbench::WorkbenchCellKind::Tool,
        title: "tool progress summary".into(),
        detail: format!(
            "started={} output={} completed={} failed={} cancelled={} latest_tool={} latest_status={} trace=/trace --kind tool tools=/tools",
            started,
            output,
            completed,
            failed,
            cancelled,
            latest_tool,
            workbench::terminal_inline(&latest_status)
        ),
    })
}

fn context_progress_summary_cell(events: &[&AgentEvent]) -> Option<workbench::WorkbenchCell> {
    let mut diffs = 0usize;
    let mut compacted = 0usize;
    let mut latest_sections = 0usize;
    let mut latest_references = 0usize;
    let mut latest_used = None;
    let mut latest_max = None;
    let mut latest_window = None;
    let mut latest_estimator = "unknown".to_owned();

    for event in events {
        match &event.kind {
            AgentEventKind::ContextDiff => {
                diffs += 1;
                latest_sections = json_array_len(&event.payload, "sections");
                latest_references = json_array_len(&event.payload, "references");
                if let Some(budget) = event.payload.get("budget") {
                    latest_used = json_u64(budget, "used_tokens");
                    latest_max = json_u64(budget, "max_tokens");
                    latest_window = json_u64(budget, "context_window");
                    latest_estimator = budget
                        .get("estimator")
                        .and_then(serde_json::Value::as_str)
                        .map(workbench::terminal_inline)
                        .unwrap_or_else(|| "unknown".into());
                }
            }
            AgentEventKind::ContextCompacted => {
                compacted += 1;
                latest_sections = json_array_len(&event.payload, "compressed_sections");
            }
            _ => continue,
        }
    }

    let total = diffs + compacted;
    (total > 0).then(|| workbench::WorkbenchCell {
        kind: workbench::WorkbenchCellKind::Context,
        title: "context progress summary".into(),
        detail: format!(
            "diffs={} compacted={} latest_sections={} latest_references={} used={} max={} context_window={} estimator={} trace=/trace --kind context context=/context",
            diffs,
            compacted,
            latest_sections,
            latest_references,
            latest_used
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
            latest_max
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
            latest_window
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
            latest_estimator,
        ),
    })
}

fn live_cell_summary_line(events: &[&AgentEvent], visible_events: &[&AgentEvent]) -> String {
    let model_stream_suppressed = events
        .iter()
        .filter(|event| matches!(event.kind, AgentEventKind::ModelStream(_)))
        .count();
    format!(
        "live_cell_summary: session={} model={} tool={} context={} memory={} coding={} approval={} continuation={} audit={} error={} model_stream_suppressed={}",
        count_live_events(visible_events, "session"),
        count_live_events(events, "model"),
        count_live_events(visible_events, "tool"),
        count_live_events(visible_events, "context"),
        count_live_events(visible_events, "memory"),
        count_live_events(visible_events, "coding"),
        count_live_events(visible_events, "approval"),
        count_live_events(visible_events, "continuation"),
        count_live_events(visible_events, "audit"),
        count_live_events(visible_events, "error"),
        model_stream_suppressed
    )
}

pub(super) fn live_cells_json_line(
    events: &[&AgentEvent],
    visible_events: &[&AgentEvent],
    cells: &[workbench::WorkbenchCell],
) -> String {
    format!(
        "live_cells_json: {}",
        serde_json::json!({
            "schema": "ikaros-workbench-live-cells-v1",
            "version": 1,
            "total_events": events.len(),
            "visible_events": visible_events.len(),
            "model_stream_suppressed": events
                .iter()
                .filter(|event| matches!(event.kind, AgentEventKind::ModelStream(_)))
                .count(),
            "counts": {
                "session": count_live_events(visible_events, "session"),
                "model": count_live_events(events, "model"),
                "tool": count_live_events(visible_events, "tool"),
                "context": count_live_events(visible_events, "context"),
                "memory": count_live_events(visible_events, "memory"),
                "coding": count_live_events(visible_events, "coding"),
                "approval": count_live_events(visible_events, "approval"),
                "continuation": count_live_events(visible_events, "continuation"),
                "audit": count_live_events(visible_events, "audit"),
                "error": count_live_events(visible_events, "error"),
            },
            "cells": cells.iter().map(live_cell_json).collect::<Vec<_>>(),
        })
    )
}

fn live_cell_json(cell: &workbench::WorkbenchCell) -> serde_json::Value {
    serde_json::json!({
        "kind": cell.kind.as_str(),
        "title": workbench::terminal_inline(&cell.title),
        "detail": workbench::terminal_inline(&cell.detail),
    })
}

fn count_live_events(events: &[&AgentEvent], category: &str) -> usize {
    events
        .iter()
        .filter(|event| live_event_category(&event.kind) == category)
        .count()
}

fn live_event_category(kind: &AgentEventKind) -> &'static str {
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

fn json_u64(payload: &serde_json::Value, key: &str) -> Option<u64> {
    payload.get(key).and_then(serde_json::Value::as_u64)
}

fn json_array_len(payload: &serde_json::Value, key: &str) -> usize {
    payload
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

pub(super) fn default_live_cell_event(kind: &AgentEventKind) -> bool {
    matches!(
        kind,
        AgentEventKind::ToolCallCompleted
            | AgentEventKind::ToolCallFailed
            | AgentEventKind::ToolCallCancelled
            | AgentEventKind::CodingTurn
            | AgentEventKind::ApprovalRequested
            | AgentEventKind::Error
    )
}

fn live_cell_event_visible(kind: &AgentEventKind, include_debug_internal_events: bool) -> bool {
    default_live_cell_event(kind)
        || (include_debug_internal_events && !matches!(kind, AgentEventKind::ModelStream(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_text_delta_uses_codex_like_prefixes_across_chunks() {
        let mut state = AgentTextDeltaState::default();

        assert_eq!(format_agent_text_delta("hello", &mut state), "");
        assert_eq!(
            format_agent_text_delta(" world\n", &mut state),
            "• hello world\n"
        );
        assert_eq!(format_agent_text_delta("next line", &mut state), "");
        assert_eq!(finish_agent_text_delta(&mut state), "  next line");
    }

    #[test]
    fn agent_text_delta_preserves_blank_lines_and_redacts_secrets() {
        let mut state = AgentTextDeltaState::default();
        let mut rendered =
            format_agent_text_delta("first\n\nsecond token=sk-secret-value", &mut state);
        rendered.push_str(&finish_agent_text_delta(&mut state));

        assert_eq!(rendered, "• first\n\n  second [REDACTED_SECRET]");
    }

    #[test]
    fn agent_text_delta_collapses_excess_blank_lines_between_paragraphs() {
        let mut state = AgentTextDeltaState::default();
        let input = "你好。很高兴又见到你。\n\n\n\n上次我们聊到过你喜欢菠萝和凤梨。";
        let mut rendered = format_agent_text_delta(input, &mut state);
        rendered.push_str(&finish_agent_text_delta(&mut state));

        assert_eq!(
            rendered,
            "• 你好。很高兴又见到你。\n\n  上次我们聊到过你喜欢菠萝和凤梨。"
        );
        assert!(!rendered.contains("\n\n\n"));
    }

    #[test]
    fn agent_text_delta_trims_excess_trailing_blank_lines() {
        let mut state = AgentTextDeltaState::default();
        let mut rendered = format_agent_text_delta("answer\n\n\n", &mut state);
        rendered.push_str(&finish_agent_text_delta(&mut state));

        assert_eq!(rendered, "• answer\n");
        assert!(!rendered.contains("\n\n"));
    }

    #[test]
    fn agent_text_delta_does_not_add_wide_paragraph_indent() {
        let mut state = AgentTextDeltaState::default();
        let input = "你好。我是 Ikaros，很高兴见到你。\n\n有什么我可以帮你的吗？无论是整理信息、处理文件、写代码，还是只是想聊聊，我都在。";
        let mut rendered = format_agent_text_delta(input, &mut state);
        rendered.push_str(&finish_agent_text_delta(&mut state));

        assert!(
            rendered.contains("\n\n  有什么我可以帮你的吗？"),
            "{rendered}"
        );
        assert!(
            !rendered.contains("\n\n                                     有什么"),
            "{rendered}"
        );
    }

    #[test]
    fn agent_text_delta_simplifies_markdown_line_starts() {
        let mut state = AgentTextDeltaState::default();
        let mut rendered =
            format_agent_text_delta("### Summary\n- first\n> quoted\nplain", &mut state);
        rendered.push_str(&finish_agent_text_delta(&mut state));

        assert_eq!(rendered, "• Summary\n  • first\n  │ quoted\n  plain");
    }

    #[test]
    fn agent_text_delta_keeps_blank_line_before_markdown_heading() {
        let mut state = AgentTextDeltaState::default();
        let mut rendered = format_agent_text_delta("intro\n\n### Summary", &mut state);
        rendered.push_str(&finish_agent_text_delta(&mut state));

        assert_eq!(rendered, "• intro\n\n  Summary");
    }

    #[test]
    fn agent_text_delta_simplifies_markdown_prefixes_split_across_chunks() {
        let mut state = AgentTextDeltaState::default();
        let mut rendered = String::new();

        rendered.push_str(&format_agent_text_delta("##", &mut state));
        rendered.push_str(&format_agent_text_delta("# Summary\n-", &mut state));
        rendered.push_str(&format_agent_text_delta(" item", &mut state));
        rendered.push_str(&finish_agent_text_delta(&mut state));

        assert_eq!(rendered, "• Summary\n  • item");
        assert!(!rendered.contains('└'));
    }

    #[test]
    fn agent_text_delta_cleans_inline_markers_across_chunks() {
        let mut state = AgentTextDeltaState::default();
        let mut rendered = String::new();

        rendered.push_str(&format_agent_text_delta("Plain **bo", &mut state));
        rendered.push_str(&format_agent_text_delta("ld** and `in", &mut state));
        rendered.push_str(&format_agent_text_delta("line code`.", &mut state));
        rendered.push_str(&finish_agent_text_delta(&mut state));

        assert_eq!(rendered, "• Plain bold and inline code.");
        assert!(!rendered.contains("**"));
        assert!(!rendered.contains('`'));
    }

    #[test]
    fn agent_text_delta_renders_fenced_code_without_raw_fence_markers() {
        let mut state = AgentTextDeltaState::default();

        let rendered = format_agent_text_delta("```rust\nfn main() {}\n```\n", &mut state);

        assert_eq!(rendered, "• ╭─ rust\n  │ fn main() {}\n  ╰─\n");
        assert!(!rendered.contains("```"));
    }

    #[test]
    fn agent_text_delta_renders_table_rows_without_raw_table_markers() {
        let mut state = AgentTextDeltaState::default();
        let rendered = format_agent_text_delta(
            "| File | Status |\n| --- | --- |\n| src/lib.rs | changed |",
            &mut state,
        );
        assert!(rendered.is_empty());
        let rendered = finish_agent_text_delta(&mut state);

        assert!(rendered.contains("• File"));
        assert!(rendered.contains("Status"));
        assert!(rendered.contains("src/lib.rs │ changed"));
        assert!(!rendered.contains("| --- |"));
    }

    #[test]
    fn agent_text_delta_matches_final_assistant_markdown_for_terminal_shapes() {
        let input = "### 总结\n\n- 中文项目\n1. 第一项\n\n```rust\nfn main() {}\n```\n\n| File | Status |\n| --- | --- |\n| src/lib.rs | changed |";
        let mut state = AgentTextDeltaState::default();
        let mut rendered = String::new();

        let chars = input.chars().collect::<Vec<_>>();
        for chunk in chars.chunks(5) {
            let chunk = chunk.iter().collect::<String>();
            rendered.push_str(&format_agent_text_delta(&chunk, &mut state));
        }
        rendered.push_str(&finish_agent_text_delta(&mut state));

        assert_eq!(
            rendered,
            crate::chat::output::render_assistant_markdown_transcript(input)
        );
        assert!(!rendered.contains("###"));
        assert!(!rendered.contains("```"));
        assert!(!rendered.contains("| --- |"));
        assert!(!rendered.contains('└'));
    }

    #[test]
    fn agent_text_delta_separates_activity_from_answer() {
        let mut state = AgentTextDeltaState {
            activity_since_last_text: true,
            ..Default::default()
        };

        let mut rendered = format_agent_text_delta("answer", &mut state);
        assert!(rendered.is_empty());
        rendered.push_str(&finish_agent_text_delta(&mut state));

        assert!(rendered.starts_with('\n'));
        assert!(rendered.ends_with("• answer"));
        let separator = rendered.lines().nth(1).expect("separator line");
        assert!(separator.chars().all(|ch| ch == '─'));
        assert!(separator.chars().count() >= 24);
        assert!(!state.activity_since_last_text);
    }

    #[test]
    fn human_activity_line_keeps_plain_text_when_not_terminal() {
        assert_eq!(
            human_activity_line_for_terminal("• Explored", false),
            "• Explored"
        );
        assert_eq!(
            human_activity_line_for_terminal("  └ Read SKILL.md", false),
            "  └ Read SKILL.md"
        );
    }

    #[test]
    fn human_activity_line_colors_bullet_when_terminal() {
        assert_eq!(
            human_activity_line_for_terminal("• Explored", true),
            "\x1b[32m•\x1b[0m Explored"
        );
        assert_eq!(
            human_activity_line_for_terminal("  └ Read SKILL.md", true),
            "  \x1b[36m└\x1b[0m Read SKILL.md"
        );
    }
}
