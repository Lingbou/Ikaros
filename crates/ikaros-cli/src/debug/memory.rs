// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::debug) fn prompt_section_metadata_only(value: &Value) -> Value {
    let Some(sections) = value.as_array() else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        sections
            .iter()
            .filter_map(|section| section.as_object())
            .map(|section| {
                let mut metadata = section.clone();
                metadata.remove("content");
                Value::Object(metadata)
            })
            .collect(),
    )
}

pub(in crate::debug) fn debug_memory_lifecycle(
    args: DebugSessionQuery,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let output = debug_memory_lifecycle_report(args, paths, workspace, agent_override)?;
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

pub(crate) fn debug_memory_lifecycle_json_line(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    session_id: &str,
    turn_id: Option<&str>,
) -> Result<String> {
    let output = debug_memory_lifecycle_report(
        DebugSessionQuery {
            session_id: session_id.to_owned(),
            turn_id: turn_id.map(ToOwned::to_owned),
        },
        paths,
        workspace,
        agent_override,
    )?;
    Ok(format!(
        "memory_lifecycle_json: {}",
        serde_json::to_string(&redact_json(output))?
    ))
}

pub(in crate::debug) fn debug_memory_lifecycle_report(
    args: DebugSessionQuery,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Value> {
    let (state_db, replay) = replay_session(paths, workspace, agent_override, &args.session_id)?;
    let events = filter_turn_events(
        &replay.agent_events,
        &args.session_id,
        args.turn_id.as_deref(),
    )?;
    let memory_event_refs = events
        .into_iter()
        .filter(|event| matches!(event.kind, AgentEventKind::MemoryLifecycle))
        .collect::<Vec<_>>();
    let journal = JsonlMemoryJournal::new(&paths.memory_dir);
    let matching_journal_entries = journal
        .list()?
        .into_iter()
        .filter(|entry| {
            source_ref_matches(
                entry.source_ref.as_ref(),
                &args.session_id,
                args.turn_id.as_deref(),
            )
        })
        .collect::<Vec<_>>();
    let mut turn_ids = memory_event_refs
        .iter()
        .map(|event| event.turn_id.to_string())
        .collect::<BTreeSet<_>>();
    for entry in &matching_journal_entries {
        if let Some((_, Some(turn_id))) = memory_ref_session_turn(entry.source_ref.as_ref()) {
            turn_ids.insert(turn_id.to_owned());
        }
    }
    let correlations = turn_correlation_map(&args.session_id, &turn_ids);
    let correlation_id =
        selected_turn_correlation_id(&args.session_id, args.turn_id.as_deref(), &turn_ids);
    let memory_events = memory_event_refs
        .into_iter()
        .map(memory_event_summary)
        .collect::<Vec<_>>();
    let mut action_counts = BTreeMap::<String, usize>::new();
    for entry in &matching_journal_entries {
        *action_counts
            .entry(memory_journal_action_name(&entry.action).to_owned())
            .or_default() += 1;
    }
    let journal_entries = matching_journal_entries
        .iter()
        .map(memory_journal_entry_summary)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let output = json!({
        "session_id": args.session_id,
        "turn_id": args.turn_id,
        "correlation_id": correlation_id,
        "correlations": correlations,
        "state_db": state_db.display().to_string(),
        "memory_lifecycle_events": memory_events,
        "memory_journal_path": journal.path().display().to_string(),
        "memory_journal_action_counts": action_counts,
        "memory_journal_entries": journal_entries,
    });
    Ok(output)
}
pub(in crate::debug) fn memory_journal_action_name(
    action: &ikaros_memory::MemoryJournalAction,
) -> &'static str {
    match action {
        ikaros_memory::MemoryJournalAction::Append => "append",
        ikaros_memory::MemoryJournalAction::Update => "update",
        ikaros_memory::MemoryJournalAction::Promote => "promote",
        ikaros_memory::MemoryJournalAction::Demote => "demote",
        ikaros_memory::MemoryJournalAction::Forget => "forget",
        ikaros_memory::MemoryJournalAction::Skip => "skip",
        ikaros_memory::MemoryJournalAction::CandidateCreated => "candidate_created",
        ikaros_memory::MemoryJournalAction::CandidateAccepted => "candidate_accepted",
        ikaros_memory::MemoryJournalAction::CandidateRejected => "candidate_rejected",
        ikaros_memory::MemoryJournalAction::ProjectionRendered => "projection_rendered",
        ikaros_memory::MemoryJournalAction::Superseded => "superseded",
        ikaros_memory::MemoryJournalAction::WorkingMemoryExpired => "working_memory_expired",
    }
}

pub(in crate::debug) fn memory_event_summary(event: &AgentEvent) -> Value {
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
        "session_id": event.session_id,
        "turn_id": event.turn_id,
        "correlation_id": trace_correlation_id(&event.session_id, &event.turn_id),
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

pub(in crate::debug) fn memory_journal_entry_summary(
    entry: &MemoryJournalEntry,
) -> serde_json::Result<Value> {
    let mut value = serde_json::to_value(entry)?;
    let correlation_id =
        memory_ref_session_turn(entry.source_ref.as_ref()).and_then(|(session_id, turn_id)| {
            turn_id.map(|turn_id| trace_correlation_id(session_id, turn_id))
        });
    if let Value::Object(object) = &mut value {
        object.insert("correlation_id".to_owned(), json!(correlation_id));
    }
    Ok(value)
}

pub(in crate::debug) fn memory_ref_session_turn(
    source_ref: Option<&MemoryRef>,
) -> Option<(&str, Option<&str>)> {
    let Some(MemoryRef::SessionTurn {
        session_id,
        turn_id,
    }) = source_ref
    else {
        return None;
    };
    Some((session_id.as_str(), turn_id.as_deref()))
}

pub(in crate::debug) fn source_ref_matches(
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
