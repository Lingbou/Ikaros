// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_state::memory::{JsonlMemoryJournal, MemoryJournal};
use ikaros_state::session::AgentEventKind;

use super::super::super::{WorkbenchCell, WorkbenchCellKind, path_display, terminal_inline};
use super::cells::{
    print_memory_candidate_cells, print_memory_journal_cells, print_memory_projection_cells,
    print_memory_supersession_cells, print_working_memory_cells,
};
use super::projection::{memory_projection_explain, superseded_memory_record_count};
use super::value::{
    memory_projection_file_count, pending_memory_candidate_count, working_memory_record_count,
};

pub(in crate::chat) fn print_memory_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    println!(
        "memory_backend: {}",
        terminal_inline(&config.memory.backend)
    );
    println!("memory_dir: {}", path_display(&paths.memory_dir));
    println!(
        "memory_context_enabled: {}",
        runtime.agent.profile.memory_context
    );
    println!(
        "memory_policy: promote={} demote={} forget={} max_records_per_scope={}",
        config.memory.policy.promote_threshold,
        config.memory.policy.demote_threshold,
        config.memory.policy.forget_threshold,
        config.memory.policy.max_records_per_scope
    );
    println!(
        "memory_external_providers: {}",
        config.memory.external_providers.len()
    );
    println!(
        "- {}",
        WorkbenchCell {
            kind: WorkbenchCellKind::Memory,
            title: "memory status".into(),
            detail: format!(
                "backend={} context_enabled={} external_providers={}",
                terminal_inline(&config.memory.backend),
                runtime.agent.profile.memory_context,
                config.memory.external_providers.len()
            ),
        }
        .render()
    );
    println!("{}", memory_status_json_line(config, paths, runtime)?);
    print_memory_projection_explain(config, paths, runtime)?;
    print_memory_projection_cells(paths)?;
    print_memory_candidate_cells(paths)?;
    print_memory_supersession_cells(config, paths)?;
    print_working_memory_cells(paths, runtime)?;
    super::super::print_filtered_event_cells(runtime, "memory", |kind| {
        matches!(kind, AgentEventKind::MemoryLifecycle)
    })?;
    print_memory_journal_cells(paths)?;
    Ok(())
}

pub(in crate::chat) fn print_memory_status_for_human(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    for line in memory_status_human_lines(config, paths, runtime)? {
        println!("{line}");
    }
    Ok(())
}

pub(in crate::chat) fn memory_status_human_lines(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<Vec<String>> {
    let explain = memory_projection_explain(config, paths, runtime)?;
    let pending_candidates = pending_memory_candidate_count(paths)?;
    let working_active = working_memory_record_count(paths, runtime)?;
    let journal_entries = JsonlMemoryJournal::new(&paths.memory_dir).list()?.len();

    Ok(vec![
        "* Memory".to_owned(),
        format!("  backend: {}", terminal_inline(&config.memory.backend)),
        format!("  context: {}", runtime.agent.profile.memory_context),
        format!("  stored: {}", explain.total_records),
        format!(
            "  projected: {} included, {} excluded",
            explain.included_count, explain.excluded_count
        ),
        format!("  candidates: {pending_candidates}"),
        format!("  working: {working_active}"),
        format!("  journal: {journal_entries}"),
        format!("  directory: {}", path_display(&paths.memory_dir)),
    ])
}

fn memory_status_json_line(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<String> {
    let projection_files = memory_projection_file_count(paths)?;
    let pending_candidates = pending_memory_candidate_count(paths)?;
    let superseded_records = superseded_memory_record_count(config, paths)?;
    let working_active = working_memory_record_count(paths, runtime)?;
    let journal_entries = JsonlMemoryJournal::new(&paths.memory_dir).list()?.len();
    let projection_explain = memory_projection_explain(config, paths, runtime)?;
    let payload = serde_json::json!({
        "schema": "ikaros-workbench-memory-status-v1",
        "version": 1,
        "session_id": terminal_inline(&runtime.chat_session_id),
        "backend": terminal_inline(&config.memory.backend),
        "memory_dir": path_display(&paths.memory_dir),
        "context_enabled": runtime.agent.profile.memory_context,
        "policy": {
            "promote_threshold": config.memory.policy.promote_threshold,
            "demote_threshold": config.memory.policy.demote_threshold,
            "forget_threshold": config.memory.policy.forget_threshold,
            "max_records_per_scope": config.memory.policy.max_records_per_scope,
        },
        "external_providers": config.memory.external_providers.len(),
        "counts": {
            "projection_files": projection_files,
            "pending_candidates": pending_candidates,
            "superseded_records": superseded_records,
            "working_active": working_active,
            "journal_entries": journal_entries,
        },
        "projection_explain": projection_explain.to_json(),
        "actions": {
            "projection_render": "memory projection render",
            "candidate_list": "memory candidate list",
            "supersession": "memory supersession <memory-id>",
            "memory_lifecycle": format!("debug memory-lifecycle {}", terminal_inline(&runtime.chat_session_id)),
        },
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        r#"{"schema":"ikaros-workbench-memory-status-v1","version":1,"error":"serialization_failed"}"#
            .to_owned()
    });
    Ok(format!("memory_status_json: {encoded}"))
}

fn print_memory_projection_explain(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    let explain = memory_projection_explain(config, paths, runtime)?;
    println!(
        "memory_projection_explain: total={} included={} excluded={} user_scope={} project_scope={}",
        explain.total_records,
        explain.included_count,
        explain.excluded_count,
        terminal_inline(&explain.user_scope),
        explain
            .project_scope
            .as_deref()
            .map(terminal_inline)
            .unwrap_or_else(|| "none".into())
    );
    println!(
        "memory_projection_explain_json: {}",
        serde_json::to_string(&explain.to_json())
            .unwrap_or_else(|_| { r#"{"error":"serialization_failed"}"#.to_owned() })
    );
    Ok(())
}
