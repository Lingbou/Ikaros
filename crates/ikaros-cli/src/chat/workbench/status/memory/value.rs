// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::{IkarosPaths, redact_secrets};
use ikaros_state::memory::{
    JsonlMemoryCandidateStore, JsonlWorkingMemoryStore, MemoryCandidateQuery,
    MemoryCandidateStatus, WorkingMemoryQuery,
};
use std::fs;

use super::super::super::terminal_inline;

pub(super) fn memory_projection_file_count(paths: &IkarosPaths) -> Result<usize> {
    let projection_dir = paths.memory_dir.join("projections");
    if !projection_dir.exists() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in fs::read_dir(&projection_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            count += 1;
        }
    }
    Ok(count)
}

pub(super) fn pending_memory_candidate_count(paths: &IkarosPaths) -> Result<usize> {
    let store = JsonlMemoryCandidateStore::new(&paths.memory_dir);
    Ok(store
        .list(MemoryCandidateQuery {
            status: Some(MemoryCandidateStatus::Pending),
            limit: None,
            ..MemoryCandidateQuery::default()
        })?
        .len())
}

pub(super) fn working_memory_record_count(
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<usize> {
    let store = JsonlWorkingMemoryStore::new(&paths.memory_dir);
    Ok(store
        .list(WorkingMemoryQuery {
            session_id: Some(runtime.chat_session_id.clone()),
            limit: None,
            ..WorkingMemoryQuery::default()
        })?
        .len())
}

pub(super) fn memory_terminal_snippet(content: &str) -> String {
    let snippet = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .take(3)
        .collect::<Vec<_>>()
        .join(" | ");
    let snippet = if snippet.is_empty() {
        content.trim().lines().next().unwrap_or_default().to_owned()
    } else {
        snippet
    };
    terminal_inline(&truncate_memory_snippet(&redact_secrets(&snippet)))
}

fn truncate_memory_snippet(snippet: &str) -> String {
    const MAX_CHARS: usize = 180;
    let mut truncated = String::new();
    for (index, ch) in snippet.chars().enumerate() {
        if index >= MAX_CHARS {
            truncated.push_str("...");
            return truncated;
        }
        truncated.push(ch);
    }
    truncated
}
