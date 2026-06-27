// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_context::ContextEngineRegistry;
use ikaros_core::{IkarosConfig, IkarosPaths, redact_json};
use ikaros_runtime::ChatRunOptions;
use ikaros_session::{
    AgentEventKind, SessionId, SessionReplay, SessionStore, SqliteSessionStore, TurnId,
};
use std::path::Path;

use super::super::{WorkbenchCell, WorkbenchCellKind, terminal_inline};

pub(in crate::chat) fn print_context_status(
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Result<()> {
    println!(
        "context_session: {}",
        terminal_inline(&runtime.chat_session_id)
    );
    println!("context_token_budget: {}", options.context_token_budget);
    println!("context_history_limit: {}", options.history_context_limit);
    println!(
        "context_history_summary_limit: {}",
        options.history_summary_limit
    );
    println!("context_memory_limit: {}", options.memory_limit);
    println!(
        "context_memory_search_limit: {}",
        options.memory_search_limit
    );
    println!("context_rag_top_k: {}", options.rag_top_k);
    println!(
        "context_relationship_learning: {}",
        options.relationship_learning
    );
    println!("context_disabled: {}", options.no_context);
    print_context_engine_registry();
    println!("{}", context_status_json_line(runtime, options)?);
    super::print_filtered_event_cells(runtime, "context", |kind| {
        matches!(
            kind,
            AgentEventKind::ContextDiff | AgentEventKind::ContextCompacted
        )
    })?;
    print_latest_prompt_sections(runtime)?;
    Ok(())
}

pub(super) fn screen_context_cells(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Result<Vec<WorkbenchCell>> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    for state_db in super::state_db_candidates(config, paths, workspace, runtime)? {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if let Some(replay) = store.replay_session(&session_id)? {
            if let Some(cells) = screen_context_cells_from_replay(&replay) {
                return Ok(cells);
            }
            return Ok(screen_context_current_cells(runtime, options));
        }
    }
    Ok(screen_context_current_cells(runtime, options))
}

pub(super) fn screen_context_cells_from_replay(
    replay: &SessionReplay,
) -> Option<Vec<WorkbenchCell>> {
    let context_event = replay
        .agent_events
        .iter()
        .rev()
        .find(|event| matches!(event.kind, AgentEventKind::ContextDiff))?;

    let mut cells = vec![screen_context_budget_cell(
        &context_event.payload,
        &context_event.turn_id,
    )];
    cells.push(screen_context_overview_cell(
        &context_event.payload,
        &context_event.turn_id,
    ));
    if let Some(cell) =
        screen_context_source_coverage_cell(&context_event.payload, &context_event.turn_id)
    {
        cells.push(cell);
    }
    if let Some(cell) =
        screen_context_prompt_cache_cell(&context_event.payload, &context_event.turn_id)
    {
        cells.push(cell);
    }
    if let Some(cell) = screen_context_limit_cell(&context_event.payload, &context_event.turn_id) {
        cells.push(cell);
    }
    cells.extend(screen_context_section_cells(
        &context_event.payload,
        &context_event.turn_id,
    ));
    cells.extend(screen_context_reference_cells(
        &context_event.payload,
        &context_event.turn_id,
    ));
    if let Some(compacted) = replay
        .agent_events
        .iter()
        .rev()
        .find(|event| matches!(event.kind, AgentEventKind::ContextCompacted))
    {
        cells.push(screen_context_compaction_cell(
            &compacted.payload,
            &compacted.turn_id,
        ));
    } else if context_event.payload.get("compression_summary").is_some()
        || context_event.payload.get("continuation_prompt").is_some()
    {
        cells.push(screen_context_compaction_cell(
            &context_event.payload,
            &context_event.turn_id,
        ));
    }
    Some(cells)
}

fn screen_context_current_cells(
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Vec<WorkbenchCell> {
    let registry = ContextEngineRegistry;
    let descriptors = registry.descriptors();
    let default_engine = descriptors
        .iter()
        .find(|descriptor| descriptor.default)
        .map(|descriptor| descriptor.id)
        .unwrap_or("none");
    let llm_summary = descriptors
        .iter()
        .any(|descriptor| descriptor.id == "llm-summary");
    vec![
        WorkbenchCell {
            kind: WorkbenchCellKind::Context,
            title: "context current".into(),
            detail: format!(
                "session={} disabled={} token_budget={} history_limit={} history_summary_limit={} memory_limit={} memory_search_limit={} rag_top_k={} relationship_learning={} command=/context",
                terminal_inline(&runtime.chat_session_id),
                options.no_context,
                options.context_token_budget,
                options.history_context_limit,
                options.history_summary_limit,
                options.memory_limit,
                options.memory_search_limit,
                options.rag_top_k,
                options.relationship_learning,
            ),
        },
        WorkbenchCell {
            kind: WorkbenchCellKind::Context,
            title: "context engines".into(),
            detail: format!(
                "available={} default={} llm_summary={} command=/context readiness=/debug readiness",
                descriptors.len(),
                terminal_inline(default_engine),
                llm_summary,
            ),
        },
    ]
}

fn context_status_json_line(
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Result<String> {
    let store = SqliteSessionStore::new(&runtime.state_dir);
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let replay = store.replay_session(&session_id)?;
    let context_events = replay
        .as_ref()
        .map(|replay| {
            replay
                .agent_events
                .iter()
                .filter(|event| {
                    matches!(
                        event.kind,
                        AgentEventKind::ContextDiff | AgentEventKind::ContextCompacted
                    )
                })
                .count()
        })
        .unwrap_or(0);
    let latest_context = replay.as_ref().and_then(|replay| {
        replay
            .agent_events
            .iter()
            .rev()
            .find(|event| matches!(event.kind, AgentEventKind::ContextDiff))
    });
    let latest_compaction = replay.as_ref().and_then(|replay| {
        replay
            .agent_events
            .iter()
            .rev()
            .find(|event| matches!(event.kind, AgentEventKind::ContextCompacted))
    });
    let context_payload = latest_context
        .map(|event| &event.payload)
        .unwrap_or(&serde_json::Value::Null);
    let prompt_sections = prompt_sections_json(context_payload);
    let sections = context_payload
        .get("sections")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let references = context_payload
        .get("references")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let budget = context_payload
        .get("budget")
        .cloned()
        .map(redact_json)
        .unwrap_or(serde_json::Value::Null);
    let compaction_payload = latest_compaction.map(|event| &event.payload).or_else(|| {
        (context_payload.get("compression_summary").is_some()
            || context_payload.get("continuation_prompt").is_some())
        .then_some(context_payload)
    });
    let registry = ContextEngineRegistry;
    let engines = registry
        .descriptors()
        .into_iter()
        .map(|descriptor| {
            serde_json::json!({
                "id": terminal_inline(descriptor.id),
                "kind": format!("{:?}", descriptor.kind),
                "default": descriptor.default,
                "requires_model_provider": descriptor.requires_model_provider,
                "summary": terminal_inline(descriptor.summary),
            })
        })
        .collect::<Vec<_>>();
    let payload = serde_json::json!({
        "schema": "ikaros-workbench-context-status-v1",
        "version": 1,
        "session_id": terminal_inline(&runtime.chat_session_id),
        "options": {
            "context_token_budget": options.context_token_budget,
            "history_context_limit": options.history_context_limit,
            "history_summary_limit": options.history_summary_limit,
            "memory_limit": options.memory_limit,
            "memory_search_limit": options.memory_search_limit,
            "rag_top_k": options.rag_top_k,
            "relationship_learning": options.relationship_learning,
            "disabled": options.no_context,
        },
        "engines": engines,
        "timeline": {
            "context_events": context_events,
            "has_context_diff": latest_context.is_some(),
            "has_context_compacted": latest_compaction.is_some(),
            "turn_id": latest_context.map(|event| terminal_inline(event.turn_id.as_str())),
        },
        "latest": {
            "budget": budget,
            "section_count": sections,
            "reference_count": references,
            "prompt_section_count": prompt_sections.len(),
            "prompt_sections": prompt_sections,
            "prompt_cache": prompt_cache_json(context_payload),
            "compaction": compaction_payload.map(context_compaction_json),
        },
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        r#"{"schema":"ikaros-workbench-context-status-v1","version":1,"error":"serialization_failed"}"#
            .to_owned()
    });
    Ok(format!("context_status_json: {encoded}"))
}

fn screen_context_overview_cell(payload: &serde_json::Value, turn_id: &TurnId) -> WorkbenchCell {
    let sections = json_array_len(payload, "sections");
    let prompt_sections = json_array_len(payload, "prompt_sections");
    let references = json_array_len(payload, "references");
    let diff = payload.get("diff").unwrap_or(&serde_json::Value::Null);
    WorkbenchCell {
        kind: WorkbenchCellKind::Context,
        title: "context overview".into(),
        detail: format!(
            "turn={} sections={} prompt_sections={} references={} added={} removed={} compressed={} context=/context trace=/trace {} timeline=/timeline --kind context",
            terminal_inline(turn_id.as_str()),
            sections,
            prompt_sections,
            references,
            json_array_len(diff, "added"),
            json_array_len(diff, "removed"),
            json_array_len(diff, "compressed"),
            terminal_inline(turn_id.as_str()),
        ),
    }
}

fn screen_context_source_coverage_cell(
    payload: &serde_json::Value,
    turn_id: &TurnId,
) -> Option<WorkbenchCell> {
    let sections = payload.get("sections")?.as_array()?;
    let mut source_counts = std::collections::BTreeMap::<String, usize>::new();
    let mut trust_counts = std::collections::BTreeMap::<String, usize>::new();
    let mut reason_counts = std::collections::BTreeMap::<String, usize>::new();
    for section in sections {
        *source_counts
            .entry(json_str(section, "source_kind").unwrap_or("unknown").into())
            .or_default() += 1;
        *trust_counts
            .entry(json_str(section, "trust_level").unwrap_or("unknown").into())
            .or_default() += 1;
        *reason_counts
            .entry(
                json_str(section, "injection_reason")
                    .unwrap_or("unknown")
                    .into(),
            )
            .or_default() += 1;
    }
    Some(WorkbenchCell {
        kind: WorkbenchCellKind::Context,
        title: "context source coverage".into(),
        detail: format!(
            "turn={} sources={} trust={} reasons={} context=/context trace=/trace {}",
            terminal_inline(turn_id.as_str()),
            format_named_counts(&source_counts),
            format_named_counts(&trust_counts),
            format_named_counts(&reason_counts),
            terminal_inline(turn_id.as_str()),
        ),
    })
}

fn screen_context_prompt_cache_cell(
    payload: &serde_json::Value,
    turn_id: &TurnId,
) -> Option<WorkbenchCell> {
    let prompt_cache = payload.get("prompt_cache");
    let has_flat_cache = payload.get("prompt_stable_prefix_hash").is_some()
        || payload.get("prompt_stable_prefix_message_count").is_some()
        || payload
            .get("prompt_stable_prefix_estimated_tokens")
            .is_some();
    if prompt_cache.is_none() && !has_flat_cache {
        return None;
    }
    Some(WorkbenchCell {
        kind: WorkbenchCellKind::Context,
        title: "prompt cache".into(),
        detail: format!(
            "turn={} policy={} eligible={} hash={} messages={} estimated_tokens={} debug=/provider debug context=/context trace=/trace {}",
            terminal_inline(turn_id.as_str()),
            prompt_cache_field(payload, "provider_policy").unwrap_or_else(|| "unknown".into()),
            prompt_cache_field(payload, "eligible").unwrap_or_else(|| "unknown".into()),
            json_str(payload, "prompt_stable_prefix_hash")
                .map(terminal_inline)
                .unwrap_or_else(|| "unknown".into()),
            json_u64(payload, "prompt_stable_prefix_message_count")
                .map(|count| count.to_string())
                .unwrap_or_else(|| "unknown".into()),
            json_u64(payload, "prompt_stable_prefix_estimated_tokens")
                .map(|tokens| tokens.to_string())
                .unwrap_or_else(|| "unknown".into()),
            terminal_inline(turn_id.as_str()),
        ),
    })
}

fn screen_context_limit_cell(
    payload: &serde_json::Value,
    turn_id: &TurnId,
) -> Option<WorkbenchCell> {
    let limit = payload
        .get("limit")
        .or_else(|| payload.get("context_limit"))
        .or_else(|| payload.get("limit_report"))?;
    Some(WorkbenchCell {
        kind: WorkbenchCellKind::Error,
        title: "context limit".into(),
        detail: format!(
            "turn={} required={} max={} protected={} estimator={} reason={} context=/context trace=/trace {} debug=/debug --kind context",
            terminal_inline(turn_id.as_str()),
            json_u64(limit, "required_tokens")
                .or_else(|| json_u64(limit, "required"))
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
            json_u64(limit, "max_tokens")
                .or_else(|| json_u64(limit, "max"))
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
            json_u64(limit, "protected_tokens")
                .or_else(|| json_u64(limit, "protected"))
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
            terminal_inline(json_str(limit, "estimator").unwrap_or("unknown")),
            terminal_inline(json_str(limit, "reason").unwrap_or("context_limit")),
            terminal_inline(turn_id.as_str()),
        ),
    })
}

fn screen_context_budget_cell(payload: &serde_json::Value, turn_id: &TurnId) -> WorkbenchCell {
    let budget = payload.get("budget").unwrap_or(&serde_json::Value::Null);
    WorkbenchCell {
        kind: WorkbenchCellKind::Context,
        title: "context budget".into(),
        detail: format!(
            "turn={} estimator={} used={} max={} context_window={} reserved_output={} source={} prompt_cache_policy={} prompt_cache_eligible={} prompt_cache_hash={} prompt_cache_messages={} prompt_cache_estimated={} command=/context trace=/trace {}",
            terminal_inline(turn_id.as_str()),
            json_str(budget, "estimator").unwrap_or("unknown"),
            json_u64(budget, "used_tokens").unwrap_or(0),
            json_u64(budget, "max_tokens").unwrap_or(0),
            json_u64(budget, "context_window")
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
            json_u64(budget, "reserved_output_tokens")
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
            json_str(budget, "source").unwrap_or("unknown"),
            prompt_cache_field(payload, "provider_policy").unwrap_or_else(|| "unknown".into()),
            prompt_cache_field(payload, "eligible").unwrap_or_else(|| "unknown".into()),
            json_str(payload, "prompt_stable_prefix_hash")
                .map(terminal_inline)
                .unwrap_or_else(|| "unknown".into()),
            json_u64(payload, "prompt_stable_prefix_message_count")
                .map(|count| count.to_string())
                .unwrap_or_else(|| "unknown".into()),
            json_u64(payload, "prompt_stable_prefix_estimated_tokens")
                .map(|tokens| tokens.to_string())
                .unwrap_or_else(|| "unknown".into()),
            terminal_inline(turn_id.as_str()),
        ),
    }
}

fn prompt_cache_field(payload: &serde_json::Value, key: &str) -> Option<String> {
    let value = payload.get("prompt_cache")?.get(key)?;
    match value {
        serde_json::Value::String(text) => Some(terminal_inline(text)),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn screen_context_section_cells(
    payload: &serde_json::Value,
    turn_id: &TurnId,
) -> Vec<WorkbenchCell> {
    payload
        .get("sections")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .take(4)
        .map(|section| {
            let label = json_str(section, "label")
                .or_else(|| json_str(section, "kind"))
                .unwrap_or("unknown");
            WorkbenchCell {
                kind: WorkbenchCellKind::Context,
                title: format!("section {}", terminal_inline(label)),
                detail: format!(
                    "turn={} section {} estimated={} source={} trust={} freshness={} scope={} reason={} trace=/trace {}",
                    terminal_inline(turn_id.as_str()),
                    terminal_inline(label),
                    json_u64(section, "estimated_tokens").unwrap_or(0),
                    terminal_inline(json_str(section, "source_kind").unwrap_or("unknown")),
                    terminal_inline(json_str(section, "trust_level").unwrap_or("unknown")),
                    terminal_inline(json_str(section, "freshness").unwrap_or("unknown")),
                    terminal_inline(json_str(section, "scope").unwrap_or("unknown")),
                    terminal_inline(json_str(section, "injection_reason").unwrap_or("unknown")),
                    terminal_inline(turn_id.as_str()),
                ),
            }
        })
        .collect()
}

fn screen_context_reference_cells(
    payload: &serde_json::Value,
    turn_id: &TurnId,
) -> Vec<WorkbenchCell> {
    payload
        .get("references")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .take(3)
        .map(|reference| {
            let raw = json_str(reference, "raw").unwrap_or("reference");
            WorkbenchCell {
                kind: WorkbenchCellKind::Context,
                title: format!("reference {}", terminal_inline(raw)),
                detail: format!(
                    "turn={} reference {} path={} estimated={} trace=/trace {}",
                    terminal_inline(turn_id.as_str()),
                    terminal_inline(raw),
                    terminal_inline(json_str(reference, "resolved_path").unwrap_or("unknown")),
                    json_u64(reference, "estimated_tokens").unwrap_or(0),
                    terminal_inline(turn_id.as_str()),
                ),
            }
        })
        .collect()
}

fn screen_context_compaction_cell(payload: &serde_json::Value, turn_id: &TurnId) -> WorkbenchCell {
    let compressed_sections = payload
        .get("compressed_sections")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let summary = json_str(payload, "summary")
        .or_else(|| json_str(payload, "compression_summary"))
        .unwrap_or("none");
    let has_continuation = payload
        .get("continuation_prompt")
        .and_then(serde_json::Value::as_str)
        .map(|prompt| !prompt.is_empty())
        .unwrap_or(false);
    WorkbenchCell {
        kind: WorkbenchCellKind::Context,
        title: "context compacted".into(),
        detail: format!(
            "turn={} compressed_sections={} continuation_prompt={} summary={} trace=/trace {}",
            terminal_inline(turn_id.as_str()),
            compressed_sections,
            if has_continuation { "yes" } else { "no" },
            terminal_inline(summary),
            terminal_inline(turn_id.as_str()),
        ),
    }
}

fn prompt_sections_json(context_payload: &serde_json::Value) -> Vec<serde_json::Value> {
    context_payload
        .get("prompt_sections")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|section| {
            serde_json::json!({
                "kind": terminal_inline(json_str(section, "kind").unwrap_or("unknown")),
                "source": terminal_inline(json_str(section, "source").unwrap_or("unknown")),
                "title": terminal_inline(json_str(section, "title").unwrap_or("unknown")),
                "priority": json_u64(section, "priority").unwrap_or(0),
                "estimated_tokens": json_u64(section, "estimated_tokens").unwrap_or(0),
                "redaction": terminal_inline(json_str(section, "redaction").unwrap_or("unknown")),
                "cache_stable_prefix": json_bool(section, "cache_stable_prefix")
                    .unwrap_or(false),
            })
        })
        .collect()
}

fn prompt_cache_json(context_payload: &serde_json::Value) -> serde_json::Value {
    if let Some(prompt_cache) = context_payload.get("prompt_cache") {
        return prompt_cache.clone();
    }
    serde_json::json!({
        "stable_prefix_hash": json_str(context_payload, "prompt_stable_prefix_hash")
            .map(terminal_inline),
        "stable_prefix_message_count": json_u64(
            context_payload,
            "prompt_stable_prefix_message_count"
        ),
        "stable_prefix_estimated_tokens": json_u64(
            context_payload,
            "prompt_stable_prefix_estimated_tokens"
        ),
    })
}

fn context_compaction_json(payload: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "summary": json_str(payload, "summary")
            .or_else(|| json_str(payload, "compression_summary"))
            .map(terminal_inline),
        "continuation_prompt": payload
            .get("continuation_prompt")
            .and_then(serde_json::Value::as_str)
            .map(|prompt| !prompt.is_empty())
            .unwrap_or(false),
        "compressed_sections": payload
            .get("compressed_sections")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len)
            .unwrap_or(0),
    })
}

fn print_context_engine_registry() {
    let registry = ContextEngineRegistry;
    for descriptor in registry.descriptors() {
        println!(
            "context_engine: {} kind={:?} default={} requires_model_provider={} summary={}",
            terminal_inline(descriptor.id),
            descriptor.kind,
            descriptor.default,
            descriptor.requires_model_provider,
            terminal_inline(descriptor.summary)
        );
    }
}

fn print_latest_prompt_sections(runtime: &InteractiveChatRuntime) -> Result<()> {
    let store = SqliteSessionStore::new(&runtime.state_dir);
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let Some(replay) = store.replay_session(&session_id)? else {
        println!("context_prompt_sections: 0");
        return Ok(());
    };
    let Some(context_event) = replay
        .agent_events
        .iter()
        .rev()
        .find(|event| matches!(event.kind, AgentEventKind::ContextDiff))
    else {
        println!("context_prompt_sections: 0");
        return Ok(());
    };
    let sections = context_event
        .payload
        .get("prompt_sections")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    println!(
        "context_prompt_cache: hash={} messages={} tokens={}",
        json_str(&context_event.payload, "prompt_stable_prefix_hash")
            .map(terminal_inline)
            .unwrap_or_else(|| "unknown".into()),
        json_u64(&context_event.payload, "prompt_stable_prefix_message_count")
            .map(|count| count.to_string())
            .unwrap_or_else(|| "unknown".into()),
        json_u64(
            &context_event.payload,
            "prompt_stable_prefix_estimated_tokens"
        )
        .map(|tokens| tokens.to_string())
        .unwrap_or_else(|| "unknown".into()),
    );
    println!("context_prompt_sections: {}", sections.len());
    for section in sections {
        let kind = section
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let source = section
            .get("source")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let title = section
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let priority = section
            .get("priority")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let estimated_tokens = section
            .get("estimated_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let redaction = section
            .get("redaction")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let cache_stable = section
            .get("cache_stable_prefix")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        println!(
            "- prompt_section kind={} source={} priority={} tokens={} redaction={} cache_stable_prefix={} title={}",
            terminal_inline(kind),
            terminal_inline(source),
            priority,
            estimated_tokens,
            terminal_inline(redaction),
            cache_stable,
            terminal_inline(title)
        );
    }
    Ok(())
}

fn json_str<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(serde_json::Value::as_str)
}

fn json_u64(value: &serde_json::Value, key: &str) -> Option<u64> {
    value.get(key).and_then(serde_json::Value::as_u64)
}

fn json_bool(value: &serde_json::Value, key: &str) -> Option<bool> {
    value.get(key).and_then(serde_json::Value::as_bool)
}

fn json_array_len(value: &serde_json::Value, key: &str) -> usize {
    value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn format_named_counts(counts: &std::collections::BTreeMap<String, usize>) -> String {
    if counts.is_empty() {
        return "none".into();
    }
    counts
        .iter()
        .map(|(key, value)| format!("{}={}", terminal_inline(key), value))
        .collect::<Vec<_>>()
        .join(",")
}
