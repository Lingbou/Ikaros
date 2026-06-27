// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::debug) fn debug_coding_turn(
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
    let coding_events = events
        .into_iter()
        .filter(|event| matches!(event.kind, AgentEventKind::CodingTurn))
        .collect::<Vec<_>>();
    let mut event_kind_counts = BTreeMap::<String, usize>::new();
    for event in &coding_events {
        if let Some(kind) = event.payload["kind"].as_str() {
            *event_kind_counts.entry(kind.to_owned()).or_default() += 1;
        }
    }
    let entries = replay
        .entries
        .iter()
        .filter(|entry| {
            args.turn_id.as_deref().is_none_or(|turn_id| {
                entry
                    .turn_id
                    .as_ref()
                    .is_some_and(|entry_turn_id| entry_turn_id.as_str() == turn_id)
            }) && entry.payload["kind"].as_str().is_some()
        })
        .map(|entry| {
            let correlation_id = entry
                .turn_id
                .as_ref()
                .map(|turn_id| trace_correlation_id(&args.session_id, turn_id));
            json!({
                "entry_id": entry.entry_id.clone(),
                "turn_id": entry.turn_id.clone(),
                "correlation_id": correlation_id,
                "kind": entry.kind,
                "coding_kind": entry.payload["kind"].as_str(),
                "visible_text": entry.visible_text.clone(),
                "payload": entry.payload.clone(),
            })
        })
        .collect::<Vec<_>>();
    let review_findings = coding_events
        .iter()
        .filter(|event| event.payload["kind"].as_str() == Some("review_finding"))
        .map(|event| event.payload.clone())
        .collect::<Vec<_>>();
    let events = coding_events
        .into_iter()
        .map(coding_event_debug_summary)
        .collect::<Result<Vec<_>>>()?;
    let output = json!({
        "session_id": args.session_id,
        "turn_id": args.turn_id,
        "state_db": state_db.display().to_string(),
        "event_count": events.len(),
        "entry_count": entries.len(),
        "event_kind_counts": event_kind_counts,
        "review_findings": review_findings,
        "events": events,
        "entries": entries,
    });
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

pub(in crate::debug) fn coding_event_debug_summary(event: &AgentEvent) -> Result<Value> {
    let mut value = serde_json::to_value(event)?;
    if let Value::Object(object) = &mut value {
        object.insert(
            "correlation_id".to_owned(),
            json!(trace_correlation_id(&event.session_id, &event.turn_id)),
        );
    }
    Ok(value)
}

pub(in crate::debug) fn debug_context_diff(
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
    let correlations = turn_correlation_map(&args.session_id, &turn_ids);
    let correlation_id =
        selected_turn_correlation_id(&args.session_id, args.turn_id.as_deref(), &turn_ids);

    let output = json!({
        "session_id": args.session_id,
        "turn_id": args.turn_id,
        "correlation_id": correlation_id,
        "correlations": correlations,
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
        "prompt_sections": latest_context
            .and_then(|payload| payload.get("prompt_sections"))
            .map(prompt_section_metadata_only),
        "prompt_cache": latest_context
            .and_then(|payload| payload.get("prompt_cache"))
            .cloned(),
        "prompt_stable_prefix_hash": latest_context
            .and_then(|payload| payload.get("prompt_stable_prefix_hash"))
            .cloned(),
        "prompt_stable_prefix_message_count": latest_context
            .and_then(|payload| payload.get("prompt_stable_prefix_message_count"))
            .cloned(),
        "prompt_stable_prefix_estimated_tokens": latest_context
            .and_then(|payload| payload.get("prompt_stable_prefix_estimated_tokens"))
            .cloned(),
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
