// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::chat::workbench::screen) fn screen_memory_panel_json(
    screen: &WorkbenchScreen,
) -> serde_json::Value {
    let memory = find_cell(screen, |cell| cell.title == "memory");
    let pending_candidates = memory
        .and_then(|cell| extract_token_after(&cell.detail, "pending_candidates="))
        .unwrap_or_else(|| "0".into());
    let working_active = memory
        .and_then(|cell| extract_token_after(&cell.detail, "working_active="))
        .unwrap_or_else(|| "0".into());
    let journal_entries = memory
        .and_then(|cell| extract_token_after(&cell.detail, "journal_entries="))
        .unwrap_or_else(|| "0".into());
    let projection_included = memory
        .and_then(|cell| extract_token_after(&cell.detail, "projection_included="))
        .unwrap_or_else(|| "0".into());
    let projection_excluded = memory
        .and_then(|cell| extract_token_after(&cell.detail, "projection_excluded="))
        .unwrap_or_else(|| "0".into());
    serde_json::json!({
        "summary": memory.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "backend": memory
            .and_then(|cell| extract_token_after(&cell.detail, "backend="))
            .unwrap_or_else(|| "unknown".into()),
        "context_enabled": memory
            .and_then(|cell| extract_token_after(&cell.detail, "context_enabled="))
            .unwrap_or_else(|| "unknown".into()),
        "projection_files": memory
            .and_then(|cell| extract_token_after(&cell.detail, "projection_files="))
            .unwrap_or_else(|| "0".into()),
        "projection_included": projection_included.clone(),
        "projection_excluded": projection_excluded.clone(),
        "pending_candidates": pending_candidates.clone(),
        "working_active": working_active.clone(),
        "journal_entries": journal_entries.clone(),
        "needs_attention": pending_candidates != "0",
        "lifecycle": {
            "candidate_pending": pending_candidates,
            "working_active": working_active,
            "journal_entries": journal_entries,
            "projection_included": projection_included,
            "projection_excluded": projection_excluded,
        },
        "actions_model": {
            "projection_render": "memory projection render",
            "candidate_list": "memory candidate list",
            "lifecycle": memory
                .and_then(|cell| command_with_prefix(&selected_cell_actions(cell), "/debug memory-lifecycle"))
                .unwrap_or_else(|| "/debug memory-lifecycle".into()),
            "memory": "/memory",
            "trace": "/trace --kind memory",
            "timeline": "/timeline --kind memory",
        },
        "actions": memory
            .map(|cell| selected_cell_actions_json(Some(cell), &selected_cell_actions(cell)))
            .unwrap_or_else(|| selected_cell_actions_json(None, &[])),
    })
}

pub(in crate::chat::workbench::screen) fn screen_rag_panel_json(
    screen: &WorkbenchScreen,
) -> serde_json::Value {
    let rag = find_cell(screen, |cell| cell.title == "rag");
    let top_k = rag
        .and_then(|cell| extract_token_after(&cell.detail, "top_k="))
        .unwrap_or_else(|| "0".into());
    let embedding_provider = rag
        .and_then(|cell| extract_token_after(&cell.detail, "embedding_provider="))
        .unwrap_or_else(|| "unknown".into());
    serde_json::json!({
        "summary": rag.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "backend": rag
            .and_then(|cell| extract_token_after(&cell.detail, "backend="))
            .unwrap_or_else(|| "unknown".into()),
        "embedding_provider": embedding_provider.clone(),
        "embedding_model": rag
            .and_then(|cell| extract_token_after(&cell.detail, "embedding_model="))
            .unwrap_or_else(|| "unknown".into()),
        "top_k": top_k.clone(),
        "default_injection": top_k != "0",
        "egress_managed": embedding_provider != "mock" && embedding_provider != "hash",
        "needs_attention": top_k != "0" && embedding_provider == "mock",
        "actions_model": {
            "ingest": "rag ingest <path>",
            "search": "rag search <query>",
            "reindex": "rag reindex",
            "stale": "rag stale",
            "context": "/context",
            "trace": "/trace --kind context",
        },
        "actions": rag
            .map(|cell| selected_cell_actions_json(Some(cell), &selected_cell_actions(cell)))
            .unwrap_or_else(|| selected_cell_actions_json(None, &[])),
    })
}
