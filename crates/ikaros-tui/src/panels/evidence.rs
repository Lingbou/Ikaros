// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(crate) fn screen_coding_groups_json(screen: &WorkbenchScreen) -> Vec<serde_json::Value> {
    ["progress", "diff", "test", "review"]
        .into_iter()
        .filter_map(|group| {
            let cell = latest_coding_group_cell(screen, group)?;
            Some(serde_json::json!({
                "group": group,
                "kind": cell.kind.as_str(),
                "title": terminal_inline(&cell.title),
                "detail": terminal_inline(&cell.detail),
                "actions": selected_cell_actions_json(Some(cell), &selected_cell_actions(cell)),
            }))
        })
        .collect()
}

pub(crate) fn latest_coding_group_cell<'a>(
    screen: &'a WorkbenchScreen,
    group: &str,
) -> Option<&'a WorkbenchCell> {
    let title = format!("coding {group}");
    screen
        .main
        .iter()
        .rev()
        .chain(screen.timeline.iter().rev())
        .chain(screen.side.iter().rev())
        .chain(screen.status.iter().rev())
        .find(|cell| matches!(cell.kind, WorkbenchCellKind::Coding) && cell.title == title)
}

pub(crate) fn screen_evidence_json(screen: &WorkbenchScreen) -> Vec<serde_json::Value> {
    [
        ("provider", WorkbenchCellKind::Model),
        ("context", WorkbenchCellKind::Context),
        ("memory", WorkbenchCellKind::Memory),
        ("rag", WorkbenchCellKind::Context),
        ("coding", WorkbenchCellKind::Coding),
        ("approval", WorkbenchCellKind::Approval),
        ("queue", WorkbenchCellKind::Continuation),
        ("gateway", WorkbenchCellKind::Session),
    ]
    .into_iter()
    .map(|(area, fallback_kind)| screen_evidence_area_json(screen, area, fallback_kind))
    .collect()
}

pub(crate) fn screen_evidence_area_json(
    screen: &WorkbenchScreen,
    area: &str,
    fallback_kind: WorkbenchCellKind,
) -> serde_json::Value {
    let cells = screen
        .status
        .iter()
        .chain(screen.timeline.iter())
        .chain(screen.main.iter())
        .chain(screen.side.iter())
        .filter(|cell| cell_matches_evidence_area(cell, area, fallback_kind))
        .collect::<Vec<_>>();
    let latest = cells.last().copied();
    let actions = latest
        .map(|cell| selected_cell_actions_json(Some(cell), &selected_cell_actions(cell)))
        .unwrap_or_else(|| selected_cell_actions_json(None, &[]));
    serde_json::json!({
        "area": area,
        "cell_count": cells.len(),
        "attention": cells.iter().any(|cell| evidence_cell_needs_attention(cell)),
        "latest": latest.map(screen_surface_cell_json).unwrap_or(serde_json::Value::Null),
        "actions": actions,
    })
}

pub(crate) fn cell_matches_evidence_area(
    cell: &WorkbenchCell,
    area: &str,
    fallback_kind: WorkbenchCellKind,
) -> bool {
    let title = cell.title.to_ascii_lowercase();
    let detail = cell.detail.to_ascii_lowercase();
    match area {
        "provider" => {
            matches!(cell.kind, WorkbenchCellKind::Model)
                || title.contains("provider")
                || detail.contains("provider=")
        }
        "context" => {
            matches!(cell.kind, WorkbenchCellKind::Context)
                && !title.contains("rag")
                && !detail.contains("embedding_provider=")
        }
        "memory" => matches!(cell.kind, WorkbenchCellKind::Memory) || title.contains("memory"),
        "rag" => title == "rag" || title.contains(" rag") || detail.contains("rag_top_k="),
        "coding" => matches!(cell.kind, WorkbenchCellKind::Coding),
        "approval" => matches!(cell.kind, WorkbenchCellKind::Approval),
        "queue" => {
            matches!(cell.kind, WorkbenchCellKind::Continuation)
                || title.contains("queue")
                || detail.contains("pending_inputs=")
                || detail.contains("continuations=")
        }
        "gateway" => title.contains("gateway") || detail.contains("gateway"),
        _ => cell.kind == fallback_kind,
    }
}

pub(crate) fn evidence_cell_needs_attention(cell: &WorkbenchCell) -> bool {
    matches!(
        cell.kind,
        WorkbenchCellKind::Error | WorkbenchCellKind::Approval
    ) || cell.detail.contains("status=failed")
        || cell.detail.contains("budget_status=exhausted")
        || cell.detail.contains("approval_pending")
        || cell.detail.contains("pending=")
        || cell.detail.contains("high_risk=")
}
