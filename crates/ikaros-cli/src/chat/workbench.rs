// SPDX-License-Identifier: GPL-3.0-only

mod mentions;
mod slash;
mod status;

use anyhow::Result;
use ikaros_core::{IkarosPaths, redact_secrets};
use ikaros_session::{AgentEvent, AgentEventKind, SessionEntry};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub(super) use mentions::print_context_mentions;
pub(super) use slash::{print_slash_commands, suggest_slash_command};
pub(super) use status::{
    TimelineVerbosity, print_approval_status, print_context_status, print_diff_status,
    print_gateway_status, print_memory_status, print_rag_status, print_replay_status,
    print_session_history, print_session_status, print_session_summaries, print_tasks_status,
    print_trace_status, print_workbench_status,
};

pub(super) const MULTILINE_TERMINATOR: &str = ".";

pub(super) fn workbench_history_path(paths: &IkarosPaths) -> PathBuf {
    paths.home.join("workbench").join("history.txt")
}

pub(super) fn append_workbench_history(paths: &IkarosPaths, input: &str) -> Result<PathBuf> {
    let path = workbench_history_path(paths);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    let input = redact_secrets(input.trim());
    writeln!(file, "{input}")?;
    writeln!(file, "---")?;
    Ok(path)
}

pub(super) fn format_workbench_help() -> &'static str {
    "commands: /help, /commands [query], /queue [message|clear], /agents, /agent <profile>, /status, /sessions, /session status|resume|history, /resume <session>, /new, /fork [summary], /timeline, /replay, /debug, /trace, /mentions [query], /context, /memory, /rag, /model, /provider [inspect|health [--live]|matrix [--live]], /gateway, /tasks, /approval, /diff, /clear, /code <plan|apply|test|review|rollback> ..., /multi, /quit"
}

pub(super) fn normalize_session_id(input: &str) -> String {
    redact_secrets(input)
        .trim()
        .replace(['/', '\\', ':', '\n', '\r', '\t'], "_")
}

pub(super) fn path_display(path: &Path) -> String {
    terminal_inline(&path.display().to_string())
}

pub(super) fn terminal_inline(input: &str) -> String {
    redact_secrets(input)
        .chars()
        .map(|ch| if ch.is_control() { '_' } else { ch })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkbenchCellKind {
    Session,
    Model,
    Tool,
    Context,
    Memory,
    Coding,
    Audit,
    Continuation,
    Approval,
    Error,
}

impl WorkbenchCellKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Model => "model",
            Self::Tool => "tool",
            Self::Context => "context",
            Self::Memory => "memory",
            Self::Coding => "coding",
            Self::Audit => "audit",
            Self::Continuation => "continuation",
            Self::Approval => "approval",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkbenchCell {
    pub(super) kind: WorkbenchCellKind,
    pub(super) title: String,
    pub(super) detail: String,
}

impl WorkbenchCell {
    pub(super) fn render(&self) -> String {
        format!(
            "cell kind={} title={} detail={}",
            self.kind.as_str(),
            terminal_inline(&self.title),
            terminal_inline(&self.detail)
        )
    }
}

#[cfg(test)]
pub(super) fn render_workbench_snapshot(cells: &[WorkbenchCell], width: usize) -> String {
    let width = width.max(16);
    let mut output = format!("snapshot width={width}\n");
    for cell in cells {
        let prefix = format!("[{}]", cell.kind.as_str());
        let title = terminal_inline(&cell.title);
        if prefix.chars().count() + 1 + title.chars().count() <= width {
            output.push_str(&format!("{prefix} {title}\n"));
        } else {
            output.push_str(&prefix);
            output.push('\n');
            for line in wrap_snapshot_detail(&title, width.saturating_sub(2)) {
                output.push_str("  ");
                output.push_str(&line);
                output.push('\n');
            }
        }
        for line in wrap_snapshot_detail(&terminal_inline(&cell.detail), width.saturating_sub(2)) {
            output.push_str("  ");
            output.push_str(&line);
            output.push('\n');
        }
    }
    output
}

#[cfg(test)]
fn wrap_snapshot_detail(detail: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in detail.split_whitespace() {
        if word.chars().count() > width {
            if !current.is_empty() {
                lines.push(current);
                current = String::new();
            }
            let mut chunk = String::new();
            for ch in word.chars() {
                if chunk.chars().count() == width {
                    lines.push(chunk);
                    chunk = String::new();
                }
                chunk.push(ch);
            }
            if !chunk.is_empty() {
                current = chunk;
            }
            continue;
        }
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = word.to_owned();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push("none".into());
    }
    lines
}

pub(super) fn session_entry_cell(entry: &SessionEntry) -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: format!(
            "entry {:?} turn={}",
            entry.kind,
            optional_turn(&entry.turn_id)
        ),
        detail: entry
            .visible_text
            .as_deref()
            .map(terminal_inline)
            .unwrap_or_else(|| "none".into()),
    }
}

pub(super) fn agent_event_cell(event: &AgentEvent) -> WorkbenchCell {
    WorkbenchCell {
        kind: agent_event_cell_kind(&event.kind),
        title: format!(
            "event {} turn={}",
            agent_event_label(&event.kind),
            terminal_inline(event.turn_id.as_str())
        ),
        detail: terminal_inline(event.event_id.as_str()),
    }
}

pub(super) fn coding_event_cells<'a>(
    events: impl IntoIterator<Item = &'a AgentEvent>,
) -> Vec<(&'static str, WorkbenchCell)> {
    events
        .into_iter()
        .filter(|event| matches!(event.kind, AgentEventKind::CodingTurn))
        .filter_map(|event| {
            let kind = event
                .payload
                .get("kind")
                .and_then(serde_json::Value::as_str)?;
            let group = coding_event_group(kind);
            let summary = event
                .payload
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(kind);
            Some((
                group,
                WorkbenchCell {
                    kind: WorkbenchCellKind::Coding,
                    title: format!("coding {group}"),
                    detail: format!(
                        "turn={} kind={} summary={}",
                        terminal_inline(event.turn_id.as_str()),
                        terminal_inline(kind),
                        terminal_inline(summary)
                    ),
                },
            ))
        })
        .collect()
}

fn coding_event_group(kind: &str) -> &'static str {
    match kind {
        "patch_applied" | "patch_failed" | "patch_skipped" | "diff_updated" => "diff",
        "test_evidence_recorded" => "test",
        "review_started" | "review_finding" | "review_completed" | "iteration_planned" => "review",
        _ => "progress",
    }
}

fn optional_turn(turn_id: &Option<ikaros_session::TurnId>) -> String {
    turn_id
        .as_ref()
        .map(|turn_id| terminal_inline(turn_id.as_str()))
        .unwrap_or_else(|| "none".into())
}

fn agent_event_cell_kind(kind: &AgentEventKind) -> WorkbenchCellKind {
    match kind {
        AgentEventKind::ModelStream(_) => WorkbenchCellKind::Model,
        AgentEventKind::ToolCallStarted
        | AgentEventKind::ToolCallOutputDelta
        | AgentEventKind::ToolCallCompleted
        | AgentEventKind::ToolCallFailed
        | AgentEventKind::ToolCallCancelled => WorkbenchCellKind::Tool,
        AgentEventKind::ContextDiff | AgentEventKind::ContextCompacted => {
            WorkbenchCellKind::Context
        }
        AgentEventKind::MemoryLifecycle => WorkbenchCellKind::Memory,
        AgentEventKind::CodingTurn => WorkbenchCellKind::Coding,
        AgentEventKind::AuditAnchor => WorkbenchCellKind::Audit,
        AgentEventKind::ContinuationStarted
        | AgentEventKind::ContinuationCompleted
        | AgentEventKind::ContinuationFailed
        | AgentEventKind::ContinuationCancelled => WorkbenchCellKind::Continuation,
        AgentEventKind::ApprovalRequested | AgentEventKind::ApprovalResolved => {
            WorkbenchCellKind::Approval
        }
        AgentEventKind::Error => WorkbenchCellKind::Error,
        AgentEventKind::SessionStart
        | AgentEventKind::TurnStart
        | AgentEventKind::UserMessage
        | AgentEventKind::TurnEnd => WorkbenchCellKind::Session,
    }
}

fn agent_event_label(kind: &AgentEventKind) -> &'static str {
    match kind {
        AgentEventKind::SessionStart => "session_start",
        AgentEventKind::TurnStart => "turn_start",
        AgentEventKind::UserMessage => "user_message",
        AgentEventKind::ModelStream(_) => "model_stream",
        AgentEventKind::ToolCallStarted => "tool_call_started",
        AgentEventKind::ToolCallOutputDelta => "tool_call_output_delta",
        AgentEventKind::ToolCallCompleted => "tool_call_completed",
        AgentEventKind::ToolCallFailed => "tool_call_failed",
        AgentEventKind::ToolCallCancelled => "tool_call_cancelled",
        AgentEventKind::ContextDiff => "context_diff",
        AgentEventKind::ContextCompacted => "context_compacted",
        AgentEventKind::MemoryLifecycle => "memory_lifecycle",
        AgentEventKind::CodingTurn => "coding_turn",
        AgentEventKind::AuditAnchor => "audit_anchor",
        AgentEventKind::ContinuationStarted => "continuation_started",
        AgentEventKind::ContinuationCompleted => "continuation_completed",
        AgentEventKind::ContinuationFailed => "continuation_failed",
        AgentEventKind::ContinuationCancelled => "continuation_cancelled",
        AgentEventKind::ApprovalRequested => "approval_requested",
        AgentEventKind::ApprovalResolved => "approval_resolved",
        AgentEventKind::TurnEnd => "turn_end",
        AgentEventKind::Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikaros_session::{EventId, SessionEntryKind, SessionId, TurnId};
    use tempfile::tempdir;

    #[test]
    fn workbench_history_redacts_secret_like_input() {
        let temp = tempdir().expect("tempdir");
        let paths = IkarosPaths::from_home(temp.path());

        let path = append_workbench_history(&paths, "token sk-secret-value").expect("append");

        let raw = fs::read_to_string(path).expect("history");
        assert!(!raw.contains("sk-secret-value"));
        assert!(raw.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn terminal_inline_redacts_and_replaces_control_characters() {
        let rendered = terminal_inline("token sk-secret-value\nnext\tcell\r");

        assert!(!rendered.contains("sk-secret-value"));
        assert!(!rendered.chars().any(char::is_control));
        assert_eq!(rendered, "token [REDACTED_SECRET]_next_cell_");
    }

    #[test]
    fn session_ids_are_normalized_for_terminal_resume() {
        assert_eq!(
            normalize_session_id("gateway/thread:one\n"),
            "gateway_thread_one"
        );
    }

    #[test]
    fn session_entry_cells_redact_visible_text() {
        let mut entry = SessionEntry::new(
            SessionId::from("session-one"),
            SessionEntryKind::UserMessage,
        );
        entry.turn_id = Some(TurnId::from("turn-one"));
        entry.visible_text = Some("use key sk-secret-value".into());

        let rendered = session_entry_cell(&entry).render();

        assert!(rendered.contains("kind=session"));
        assert!(rendered.contains("turn=turn-one"));
        assert!(!rendered.contains("sk-secret-value"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn agent_event_cells_group_coding_events() {
        let event = AgentEvent {
            event_id: EventId::from("event-one"),
            session_id: SessionId::from("session-one"),
            turn_id: TurnId::from("turn-one"),
            parent_event_id: None,
            at: time::OffsetDateTime::now_utc(),
            source: ikaros_session::AgentEventSource::Runtime,
            kind: AgentEventKind::CodingTurn,
            payload: serde_json::Value::Null,
        };

        let rendered = agent_event_cell(&event).render();

        assert!(rendered.contains("kind=coding"));
        assert!(rendered.contains("coding_turn"));
        assert!(rendered.contains("turn=turn-one"));
    }

    #[test]
    fn coding_event_cells_group_diff_test_review_and_progress() {
        let events = [
            "diff_updated",
            "test_evidence_recorded",
            "review_completed",
            "plan_prepared",
        ]
        .into_iter()
        .map(|kind| AgentEvent {
            event_id: EventId::new(),
            session_id: SessionId::from("session-one"),
            turn_id: TurnId::from("turn-one"),
            parent_event_id: None,
            at: time::OffsetDateTime::now_utc(),
            source: ikaros_session::AgentEventSource::Tool,
            kind: AgentEventKind::CodingTurn,
            payload: serde_json::json!({
                "kind": kind,
                "summary": format!("summary for {kind}"),
            }),
        })
        .collect::<Vec<_>>();

        let cells = coding_event_cells(&events);
        let groups = cells.iter().map(|(group, _)| *group).collect::<Vec<_>>();

        assert_eq!(groups, vec!["diff", "test", "review", "progress"]);
        assert!(
            cells
                .iter()
                .any(|(_, cell)| cell.render().contains("title=coding diff"))
        );
    }

    #[test]
    fn workbench_snapshot_wraps_cells_and_redacts_secret_text() {
        let snapshot = render_workbench_snapshot(
            &[
                WorkbenchCell {
                    kind: WorkbenchCellKind::Coding,
                    title: "coding diff".into(),
                    detail: "turn=turn-one kind=diff_updated summary=changed src/lib.rs with sk-secret-value".into(),
                },
                WorkbenchCell {
                    kind: WorkbenchCellKind::Approval,
                    title: "approval pending".into(),
                    detail: "provider_call=true workspace_write=true shell=false".into(),
                },
            ],
            42,
        );

        assert_eq!(
            snapshot,
            "snapshot width=42\n[coding] coding diff\n  turn=turn-one kind=diff_updated\n  summary=changed src/lib.rs with\n  [REDACTED_SECRET]\n[approval] approval pending\n  provider_call=true workspace_write=true\n  shell=false\n"
        );
    }

    #[test]
    fn workbench_snapshot_splits_long_tokens_at_width() {
        let snapshot = render_workbench_snapshot(
            &[WorkbenchCell {
                kind: WorkbenchCellKind::Context,
                title: "context reference".into(),
                detail: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            }],
            18,
        );

        for line in snapshot.lines().skip(1) {
            assert!(
                line.chars().count() <= 18,
                "snapshot line exceeds width: {line}"
            );
        }
    }
}
