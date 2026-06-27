// SPDX-License-Identifier: GPL-3.0-only

use super::{render_cell_detail_summary, wrap_markdown_detail};
use crate::{WorkbenchCell, WorkbenchCellKind, selection::extract_token_after};

pub(crate) fn human_cell_detail_lines(cell: &WorkbenchCell, width: usize) -> Vec<String> {
    wrap_markdown_detail(&human_cell_detail(cell), width)
}

pub(crate) fn human_cell_detail(cell: &WorkbenchCell) -> String {
    if detail_is_human_text(&cell.detail) {
        return render_cell_detail_summary(&cell.detail);
    }
    match cell.kind {
        WorkbenchCellKind::Session => human_session_detail(cell),
        WorkbenchCellKind::Model => human_model_detail(cell),
        WorkbenchCellKind::Tool => human_tool_detail(cell),
        WorkbenchCellKind::Context => human_context_detail(cell),
        WorkbenchCellKind::Memory => human_memory_detail(cell),
        WorkbenchCellKind::Coding => human_coding_detail(cell),
        WorkbenchCellKind::Audit => "Audit event recorded.".into(),
        WorkbenchCellKind::Continuation => human_queue_detail(cell),
        WorkbenchCellKind::Approval => human_approval_detail(cell),
        WorkbenchCellKind::Error => human_error_detail(cell),
    }
}

fn detail_is_human_text(detail: &str) -> bool {
    let trimmed = detail.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.contains('\n') {
        return true;
    }
    if trimmed.starts_with('/') {
        return !trimmed.contains('=');
    }
    let assignments = trimmed
        .split_whitespace()
        .filter(|part| part.contains('='))
        .count();
    assignments <= 1 && !trimmed.contains(" command=")
}

fn human_session_detail(cell: &WorkbenchCell) -> String {
    if cell.title == "timeline" {
        return "Recent activity appears here after a turn.".into();
    }
    if cell.title == "progress" {
        return human_progress_from_detail(&cell.detail);
    }
    "Session activity.".into()
}

fn human_model_detail(cell: &WorkbenchCell) -> String {
    let status = extract_token_after(&cell.detail, "status=")
        .or_else(|| extract_token_after(&cell.detail, "health_status="))
        .or_else(|| extract_token_after(&cell.detail, "recovery="))
        .unwrap_or_else(|| "configured".into());
    if matches!(status.as_str(), "ok" | "configured" | "unbounded") {
        "Model is configured.".into()
    } else {
        format!("Model/provider needs attention: {status}.")
    }
}

fn human_context_detail(cell: &WorkbenchCell) -> String {
    if cell.title.contains("rag") || cell.detail.contains("rag=") {
        return "RAG is available from the command palette.".into();
    }
    let sections = extract_token_after(&cell.detail, "sections=").unwrap_or_else(|| "0".into());
    let references = extract_token_after(&cell.detail, "references=").unwrap_or_else(|| "0".into());
    format!("Context loaded: {sections} sections, {references} references.")
}

fn human_memory_detail(cell: &WorkbenchCell) -> String {
    let candidates = extract_token_after(&cell.detail, "candidates=").unwrap_or_else(|| "0".into());
    let working = extract_token_after(&cell.detail, "working=").unwrap_or_else(|| "0".into());
    format!("Memory: {working} working notes, {candidates} candidates.")
}

fn human_coding_detail(cell: &WorkbenchCell) -> String {
    if cell.detail.contains("failed") || cell.kind == WorkbenchCellKind::Error {
        "Coding work needs review.".into()
    } else {
        "Coding workflow status.".into()
    }
}

fn human_queue_detail(cell: &WorkbenchCell) -> String {
    let queued = extract_token_after(&cell.detail, "queued=").unwrap_or_else(|| "0".into());
    let running = extract_token_after(&cell.detail, "running=").unwrap_or_else(|| "0".into());
    let failed = extract_token_after(&cell.detail, "failed=").unwrap_or_else(|| "0".into());
    format!("Queue: {queued} waiting, {running} running, {failed} failed.")
}

fn human_approval_detail(cell: &WorkbenchCell) -> String {
    if cell.detail.contains("approve=") || cell.title.contains("pending") {
        "Approval required. Enter opens details; Alt+A approves; Alt+D denies.".into()
    } else {
        "No approvals pending.".into()
    }
}

fn human_tool_detail(cell: &WorkbenchCell) -> String {
    if cell.title.contains("tools") {
        "Tool status and availability.".into()
    } else {
        "Tool event.".into()
    }
}

fn human_error_detail(cell: &WorkbenchCell) -> String {
    let kind = extract_token_after(&cell.detail, "kind=").unwrap_or_else(|| "error".into());
    format!("Error: {kind}. Press F5 for recovery actions.")
}

fn human_progress_from_detail(detail: &str) -> String {
    let status = extract_token_after(detail, "status=").unwrap_or_else(|| "idle".into());
    let phase = extract_token_after(detail, "phase=").unwrap_or_else(|| "idle".into());
    if status == "idle" {
        "Idle.".into()
    } else {
        format!("{status}: {phase}.")
    }
}
