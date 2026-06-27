// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_agent::chat::ChatRunOptions;
use ikaros_core::redact_json;
use ikaros_protocol::context::ContextEngineRegistry;
use ikaros_state::session::{AgentEventKind, SessionId, SessionStore, SqliteSessionStore};

use super::super::super::terminal_inline;
use super::value::{json_array_len, json_bool, json_str, json_u64};

pub(in crate::chat) fn context_status_human_lines(
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Result<Vec<String>> {
    let registry = ContextEngineRegistry;
    let descriptors = registry.descriptors();
    let default_engine = descriptors
        .iter()
        .find(|descriptor| descriptor.default)
        .map(|descriptor| descriptor.id)
        .unwrap_or("none");
    let store = SqliteSessionStore::new(&runtime.state_dir);
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let replay = store.replay_session(&session_id)?;
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
    let empty_payload = serde_json::Value::Null;
    let payload = latest_context
        .map(|event| &event.payload)
        .unwrap_or(&empty_payload);

    let mut lines = vec![
        "* Context".to_owned(),
        format!("  session: {}", terminal_inline(&runtime.chat_session_id)),
        format!("  budget: {} tokens", options.context_token_budget),
        format!("  disabled: {}", options.no_context),
        format!(
            "  history: {} turns, {} summaries",
            options.history_context_limit, options.history_summary_limit
        ),
        format!(
            "  memory: {} records, search limit {}",
            options.memory_limit, options.memory_search_limit
        ),
        format!("  rag: top_k {}", options.rag_top_k),
        format!("  relationship learning: {}", options.relationship_learning),
        format!(
            "  engine: {} ({} available)",
            terminal_inline(default_engine),
            descriptors.len()
        ),
        "  latest:".to_owned(),
        format!("    events: {context_events}"),
        format!("    sections: {}", json_array_len(payload, "sections")),
        format!(
            "    prompt sections: {}",
            json_array_len(payload, "prompt_sections")
        ),
        format!("    references: {}", json_array_len(payload, "references")),
        format!("    compacted: {}", latest_compaction.is_some()),
    ];
    if let Some(turn_id) = latest_context.map(|event| event.turn_id.as_str()) {
        lines.push(format!("  turn: {}", terminal_inline(turn_id)));
    }
    Ok(lines)
}

pub(super) fn context_status_json_line(
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

pub(super) fn print_context_engine_registry() {
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

pub(super) fn print_latest_prompt_sections(runtime: &InteractiveChatRuntime) -> Result<()> {
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
