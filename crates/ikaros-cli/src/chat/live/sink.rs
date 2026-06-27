// SPDX-License-Identifier: GPL-3.0-only

use super::super::{interactive_chat_turn_error_actions, interactive_chat_turn_error_kind};
use super::activity::{human_activity_line_for_terminal, human_activity_lines};
use super::cells::live_cell_snapshot;
use ikaros_core::{IkarosError, redact_secrets};
use ikaros_providers::model::ModelStreamEvent;
use ikaros_state::session::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, SessionId, TurnId,
};
use ikaros_terminal::{
    AgentTextDeltaState, RunningTurnTerminal, color_assistant_bullet_for_terminal,
    finish_agent_text_delta, format_agent_text_delta, normalize_raw_terminal_newlines,
};
use std::{
    io::{self, IsTerminal, Write},
    sync::{Arc, Mutex},
};
#[derive(Clone, Default)]
pub(in crate::chat) struct WorkbenchLiveEventSink {
    events: Arc<Mutex<Vec<AgentEvent>>>,
    snapshots: Arc<Mutex<Vec<String>>>,
    stdout_updates: bool,
    text_delta_stdout: bool,
    human_activity_stdout: bool,
    running_terminal: Option<RunningTurnTerminal>,
    agent_text_state: Arc<Mutex<AgentTextDeltaState>>,
}

impl WorkbenchLiveEventSink {
    pub(in crate::chat) fn with_text_delta_stdout_and_terminal(
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
    pub(in crate::chat) fn snapshots(&self) -> ikaros_core::Result<Vec<String>> {
        self.snapshots
            .lock()
            .map(|snapshots| snapshots.clone())
            .map_err(|_| IkarosError::Message("workbench live snapshot lock is poisoned".into()))
    }
}

pub(in crate::chat) fn emit_interactive_chat_turn_failure_evidence(
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
                        self.print_human_output(&color_assistant_bullet_for_terminal(
                            &rendered,
                            color_bullet,
                        ));
                    }
                }
                AgentEventKind::ModelStream(ModelStreamEvent::Done) => {
                    if let Ok(mut state) = self.agent_text_state.lock() {
                        let color_bullet = !state.started && io::stdout().is_terminal();
                        let rendered = finish_agent_text_delta(&mut state);
                        self.print_human_output(&color_assistant_bullet_for_terminal(
                            &rendered,
                            color_bullet,
                        ));
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
                            color_assistant_bullet_for_terminal(&rendered, color_bullet)
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
