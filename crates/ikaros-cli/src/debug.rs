// SPDX-License-Identifier: GPL-3.0-only

use crate::resolve_agent_instance;
use anyhow::{Result, anyhow};
use clap::{Args, Subcommand};
use ikaros_core::{IkarosConfig, IkarosPaths, redact_json};
use ikaros_memory::{JsonlMemoryJournal, MemoryJournal, MemoryRef};
use ikaros_session::{
    AgentEvent, AgentEventKind, SessionId, SessionReplay, SessionStore, SqliteSessionStore,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Subcommand)]
pub(crate) enum DebugCommand {
    ContextDiff(DebugSessionQuery),
    MemoryLifecycle(DebugSessionQuery),
}

#[derive(Debug, Args)]
pub(crate) struct DebugSessionQuery {
    session_id: String,
    #[arg(long)]
    turn_id: Option<String>,
}

pub(crate) fn debug_command(
    command: DebugCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    match command {
        DebugCommand::ContextDiff(args) => {
            debug_context_diff(args, paths, workspace, agent_override)
        }
        DebugCommand::MemoryLifecycle(args) => {
            debug_memory_lifecycle(args, paths, workspace, agent_override)
        }
    }
}

fn debug_context_diff(
    args: DebugSessionQuery,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let (state_db, replay) = replay_session(paths, workspace, agent_override, &args.session_id)?;
    let events = filter_turn_events(
        &replay.agent_events,
        &args.session_id,
        args.turn_id.as_deref(),
    )?;
    let context_events = events
        .iter()
        .filter(|event| matches!(event.kind, AgentEventKind::ContextDiff))
        .collect::<Vec<_>>();
    let compacted_events = events
        .iter()
        .filter(|event| matches!(event.kind, AgentEventKind::ContextCompacted))
        .collect::<Vec<_>>();
    let context_errors = events
        .iter()
        .filter(|event| matches!(event.kind, AgentEventKind::Error))
        .filter(|event| {
            event.payload["phase"].as_str() == Some("context_assemble")
                || event.payload["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("context limit exceeded"))
        })
        .collect::<Vec<_>>();
    let turn_ids = context_events
        .iter()
        .chain(compacted_events.iter())
        .chain(context_errors.iter())
        .map(|event| event.turn_id.to_string())
        .collect::<BTreeSet<_>>();
    let latest_context = context_events.last().map(|event| &event.payload);
    let latest_compaction = compacted_events.last().map(|event| &event.payload);

    let output = json!({
        "session_id": args.session_id,
        "turn_id": args.turn_id,
        "state_db": state_db.display().to_string(),
        "turns": turn_ids,
        "context_diff_events": context_events.len(),
        "context_compacted": !compacted_events.is_empty(),
        "context_limit_error": context_errors.last().map(|event| &event.payload),
        "budget": latest_context.and_then(|payload| payload.get("budget")).cloned(),
        "estimator": latest_context
            .and_then(|payload| payload.pointer("/budget/estimator"))
            .and_then(Value::as_str),
        "context_window": latest_context
            .and_then(|payload| payload.pointer("/budget/context_window"))
            .and_then(Value::as_u64),
        "sections": latest_context.and_then(|payload| payload.get("sections")).cloned(),
        "diff": latest_context.and_then(|payload| payload.get("diff")).cloned(),
        "references": latest_context.and_then(|payload| payload.get("references")).cloned(),
        "compressed_sections": latest_context
            .and_then(|payload| payload.get("compressed_sections"))
            .cloned(),
        "protected_sections": latest_context
            .and_then(|payload| payload.get("protected_sections"))
            .cloned(),
        "compression_summary": latest_context
            .and_then(|payload| payload.get("compression_summary"))
            .cloned(),
        "continuation_prompt": latest_context
            .and_then(|payload| payload.get("continuation_prompt"))
            .cloned(),
        "compaction_event": latest_compaction.cloned(),
    });
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

fn debug_memory_lifecycle(
    args: DebugSessionQuery,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let (state_db, replay) = replay_session(paths, workspace, agent_override, &args.session_id)?;
    let events = filter_turn_events(
        &replay.agent_events,
        &args.session_id,
        args.turn_id.as_deref(),
    )?;
    let memory_events = events
        .into_iter()
        .filter(|event| matches!(event.kind, AgentEventKind::MemoryLifecycle))
        .map(memory_event_summary)
        .collect::<Vec<_>>();
    let journal = JsonlMemoryJournal::new(&paths.memory_dir);
    let journal_entries = journal
        .list()?
        .into_iter()
        .filter(|entry| {
            source_ref_matches(
                entry.source_ref.as_ref(),
                &args.session_id,
                args.turn_id.as_deref(),
            )
        })
        .map(serde_json::to_value)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let output = json!({
        "session_id": args.session_id,
        "turn_id": args.turn_id,
        "state_db": state_db.display().to_string(),
        "memory_lifecycle_events": memory_events,
        "memory_journal_path": journal.path().display().to_string(),
        "memory_journal_entries": journal_entries,
    });
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

fn replay_session(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    session_id: &str,
) -> Result<(PathBuf, SessionReplay)> {
    let session_id = SessionId::from(session_id);
    for state_db in state_db_candidates(paths, workspace, agent_override)? {
        let store = SqliteSessionStore::from_file(&state_db);
        if let Some(replay) = store.replay_session(&session_id)? {
            return Ok((state_db, replay));
        }
    }
    Err(anyhow!("session not found in state.db files: {session_id}"))
}

fn state_db_candidates(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Vec<PathBuf>> {
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    let config = IkarosConfig::load(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    push_state_db_candidate(&mut candidates, &mut seen, agent.state_dir.join("state.db"));
    if agent_override.is_none() {
        let agents_dir = paths.home.join("agents");
        if agents_dir.is_dir() {
            for entry in fs::read_dir(&agents_dir)? {
                let entry = entry?;
                let state_db = entry.path().join("state.db");
                push_state_db_candidate(&mut candidates, &mut seen, state_db);
            }
        }
    }
    Ok(candidates)
}

fn push_state_db_candidate(
    candidates: &mut Vec<PathBuf>,
    seen: &mut BTreeSet<PathBuf>,
    state_db: PathBuf,
) {
    if state_db.is_file() && seen.insert(state_db.clone()) {
        candidates.push(state_db);
    }
}

fn filter_turn_events<'a>(
    events: &'a [AgentEvent],
    session_id: &str,
    turn_id: Option<&str>,
) -> Result<Vec<&'a AgentEvent>> {
    let filtered = events
        .iter()
        .filter(|event| turn_id.is_none_or(|turn_id| event.turn_id.as_str() == turn_id))
        .collect::<Vec<_>>();
    if let Some(turn_id) = turn_id
        && filtered.is_empty()
    {
        return Err(anyhow!("turn not found in session {session_id}: {turn_id}"));
    }
    Ok(filtered)
}

fn memory_event_summary(event: &AgentEvent) -> Value {
    let notes = event.payload["report"]["notes"]
        .as_array()
        .or_else(|| event.payload["notes"].as_array())
        .cloned()
        .unwrap_or_default();
    let skipped = notes.iter().any(|note| {
        note.as_str()
            .is_some_and(|note| note.to_ascii_lowercase().contains("skipped"))
    });
    let redaction_related = notes.iter().any(|note| {
        note.as_str().is_some_and(|note| {
            let note = note.to_ascii_lowercase();
            note.contains("redacted") || note.contains("secret")
        })
    });
    json!({
        "event_id": event.event_id,
        "turn_id": event.turn_id,
        "phase": event.payload["phase"].as_str()
            .or_else(|| event.payload.pointer("/report/phase").and_then(Value::as_str)),
        "records_read": event.payload["records_read"].as_u64()
            .or_else(|| event.payload.pointer("/report/records_read").and_then(Value::as_u64)),
        "records_written": event.payload["records_written"].as_u64()
            .or_else(|| event.payload.pointer("/report/records_written").and_then(Value::as_u64)),
        "source_ref": event.payload.get("source_ref")
            .cloned()
            .or_else(|| event.payload.pointer("/report/source_ref").cloned()),
        "notes": notes,
        "skipped": skipped,
        "redaction_related": redaction_related,
        "payload": event.payload,
    })
}

fn source_ref_matches(
    source_ref: Option<&MemoryRef>,
    session_id: &str,
    turn_id: Option<&str>,
) -> bool {
    match source_ref {
        Some(MemoryRef::SessionTurn {
            session_id: source_session_id,
            turn_id: source_turn_id,
        }) => {
            source_session_id == session_id
                && turn_id.is_none_or(|turn_id| source_turn_id.as_deref() == Some(turn_id))
        }
        _ => false,
    }
}
