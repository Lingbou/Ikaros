// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_state::memory::{
    JsonlMemoryCandidateStore, JsonlMemoryJournal, JsonlWorkingMemoryStore, MemoryCandidateQuery,
    MemoryCandidateStatus, MemoryJournal, WorkingMemoryQuery,
};
use std::fs;

use super::super::super::{WorkbenchCell, WorkbenchCellKind, terminal_inline};
use super::projection::{memory_projection_explain, superseded_memory_records};
use super::value::{
    memory_projection_file_count, memory_terminal_snippet, pending_memory_candidate_count,
    working_memory_record_count,
};

pub(in crate::chat::workbench::status) fn screen_memory_cell(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<WorkbenchCell> {
    let explain = memory_projection_explain(config, paths, runtime)?;
    Ok(WorkbenchCell {
        kind: WorkbenchCellKind::Memory,
        title: "memory".into(),
        detail: format!(
            "backend={} context_enabled={} projection_files={} projection_included={} projection_excluded={} pending_candidates={} working_active={} journal_entries={} command=/debug memory-lifecycle {} memory=/memory",
            terminal_inline(&config.memory.backend),
            runtime.agent.profile.memory_context,
            memory_projection_file_count(paths)?,
            explain.included_count,
            explain.excluded_count,
            pending_memory_candidate_count(paths)?,
            working_memory_record_count(paths, runtime)?,
            JsonlMemoryJournal::new(&paths.memory_dir).list()?.len(),
            terminal_inline(&runtime.chat_session_id),
        ),
    })
}

pub(super) fn print_memory_projection_cells(paths: &IkarosPaths) -> Result<()> {
    let projection_dir = paths.memory_dir.join("projections");
    let mut files = Vec::new();
    if projection_dir.exists() {
        for entry in fs::read_dir(&projection_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
                files.push(path);
            }
        }
    }
    files.sort();
    println!("memory_projection_files: {}", files.len());
    for path in files.iter().take(5) {
        let content = fs::read_to_string(path).unwrap_or_default();
        let detail = format!(
            "file={} snippet={}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown"),
            memory_terminal_snippet(&content)
        );
        println!(
            "- {}",
            WorkbenchCell {
                kind: WorkbenchCellKind::Memory,
                title: "projection".into(),
                detail,
            }
            .render()
        );
    }
    Ok(())
}

pub(super) fn print_memory_supersession_cells(
    config: &IkarosConfig,
    paths: &IkarosPaths,
) -> Result<()> {
    let records = superseded_memory_records(config, paths)?;
    let superseded = records
        .iter()
        .filter(|record| !record.active && record.superseded_by.is_some())
        .take(5)
        .collect::<Vec<_>>();
    println!("memory_superseded_records: {}", superseded.len());
    for record in superseded {
        let replacement_id = record
            .superseded_by
            .as_deref()
            .unwrap_or(record.id.as_str());
        let detail = format!(
            "kind={:?} scope={} replaced_by={} command=memory supersession {} snippet={}",
            record.kind,
            terminal_inline(&record.scope),
            terminal_inline(replacement_id),
            terminal_inline(replacement_id),
            memory_terminal_snippet(&record.content)
        );
        println!(
            "- {}",
            WorkbenchCell {
                kind: WorkbenchCellKind::Memory,
                title: "superseded memory".into(),
                detail,
            }
            .render()
        );
    }
    Ok(())
}

pub(super) fn print_memory_candidate_cells(paths: &IkarosPaths) -> Result<()> {
    let store = JsonlMemoryCandidateStore::new(&paths.memory_dir);
    let candidates = store.list(MemoryCandidateQuery {
        status: Some(MemoryCandidateStatus::Pending),
        limit: Some(5),
        ..MemoryCandidateQuery::default()
    })?;
    println!("memory_candidates_pending: {}", candidates.len());
    for candidate in candidates {
        let detail = format!(
            "kind={:?} scope={} confidence={} snippet={}",
            candidate.kind,
            terminal_inline(&candidate.scope),
            candidate.confidence,
            memory_terminal_snippet(&candidate.content)
        );
        println!(
            "- {}",
            WorkbenchCell {
                kind: WorkbenchCellKind::Memory,
                title: "candidate pending".into(),
                detail,
            }
            .render()
        );
    }
    Ok(())
}

pub(super) fn print_working_memory_cells(
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    let store = JsonlWorkingMemoryStore::new(&paths.memory_dir);
    let records = store.list(WorkingMemoryQuery {
        session_id: Some(runtime.chat_session_id.clone()),
        limit: Some(5),
        ..WorkingMemoryQuery::default()
    })?;
    println!("memory_working_active: {}", records.len());
    for record in records {
        let detail = format!(
            "kind={:?} scope={} snippet={}",
            record.kind,
            terminal_inline(&record.scope),
            memory_terminal_snippet(&record.content)
        );
        println!(
            "- {}",
            WorkbenchCell {
                kind: WorkbenchCellKind::Memory,
                title: "working memory".into(),
                detail,
            }
            .render()
        );
    }
    Ok(())
}

pub(super) fn print_memory_journal_cells(paths: &IkarosPaths) -> Result<()> {
    let journal = JsonlMemoryJournal::new(&paths.memory_dir);
    let entries = journal.list()?;
    println!("memory_journal_entries: {}", entries.len());
    let start = entries.len().saturating_sub(5);
    for entry in &entries[start..] {
        let title = format!("journal {:?}", entry.action);
        let detail = format!(
            "scope={} reason={} source={}",
            entry.scope.as_deref().unwrap_or("none"),
            terminal_inline(&entry.reason),
            entry
                .source_ref
                .as_ref()
                .map(|source| format!("{source:?}"))
                .unwrap_or_else(|| "none".into())
        );
        println!(
            "- {}",
            WorkbenchCell {
                kind: WorkbenchCellKind::Memory,
                title,
                detail,
            }
            .render()
        );
    }
    Ok(())
}
