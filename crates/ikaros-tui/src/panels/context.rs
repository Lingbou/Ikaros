// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(crate) fn screen_context_panel_json(screen: &WorkbenchScreen) -> serde_json::Value {
    let budget = find_cell(screen, |cell| cell.title == "context budget");
    let current = find_cell(screen, |cell| cell.title == "context current");
    let overview = find_cell(screen, |cell| cell.title == "context overview");
    let source_coverage = find_cell(screen, |cell| cell.title == "context source coverage");
    let prompt_cache = find_cell(screen, |cell| cell.title == "prompt cache");
    let limit = find_cell(screen, |cell| cell.title == "context limit");
    let compaction = find_cell(screen, |cell| cell.title.contains("compaction"));
    let sections = all_cells(screen)
        .filter(|cell| cell.title.starts_with("section "))
        .map(context_section_item_json)
        .collect::<Vec<_>>();
    let references = all_cells(screen)
        .filter(|cell| cell.title.starts_with("reference "))
        .map(context_reference_item_json)
        .collect::<Vec<_>>();
    serde_json::json!({
        "budget": budget.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "current": current.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "overview": overview.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "source_coverage": source_coverage.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "prompt_cache": prompt_cache.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "limit": limit.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "compaction": compaction.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "section_count": sections.len(),
        "reference_count": references.len(),
        "sections": sections,
        "references": references,
        "coverage": context_coverage_json(source_coverage),
        "budget_status": context_budget_status_json(budget.or(current)),
        "compaction_state": context_compaction_state_json(compaction),
        "needs_attention": limit.is_some() || compaction.is_some(),
        "actions": budget
            .or(current)
            .map(|cell| selected_cell_actions_json(Some(cell), &selected_cell_actions(cell)))
            .unwrap_or_else(|| selected_cell_actions_json(None, &[])),
    })
}

pub(crate) fn context_section_item_json(cell: &WorkbenchCell) -> serde_json::Value {
    let commands = selected_cell_actions(cell);
    serde_json::json!({
        "title": terminal_inline(&cell.title),
        "label": cell.title
            .strip_prefix("section ")
            .map(terminal_inline)
            .unwrap_or_else(|| "unknown".into()),
        "turn": extract_token_after(&cell.detail, "turn=")
            .unwrap_or_else(|| "none".into()),
        "estimated_tokens": extract_token_after(&cell.detail, "estimated=")
            .unwrap_or_else(|| "0".into()),
        "source": extract_token_after(&cell.detail, "source=")
            .unwrap_or_else(|| "unknown".into()),
        "trust": extract_token_after(&cell.detail, "trust=")
            .unwrap_or_else(|| "unknown".into()),
        "freshness": extract_token_after(&cell.detail, "freshness=")
            .unwrap_or_else(|| "unknown".into()),
        "scope": extract_token_after(&cell.detail, "scope=")
            .unwrap_or_else(|| "unknown".into()),
        "reason": extract_token_after(&cell.detail, "reason=")
            .unwrap_or_else(|| "unknown".into()),
        "protected": context_section_is_protected(cell),
        "actions": selected_cell_actions_json(Some(cell), &commands),
    })
}

pub(crate) fn context_section_is_protected(cell: &WorkbenchCell) -> bool {
    let detail = cell.detail.to_ascii_lowercase();
    detail.contains("trust=explicit")
        || detail.contains("source=reference")
        || detail.contains("reason=explicit")
        || detail.contains("@file")
}

pub(crate) fn context_reference_item_json(cell: &WorkbenchCell) -> serde_json::Value {
    let commands = selected_cell_actions(cell);
    serde_json::json!({
        "title": terminal_inline(&cell.title),
        "raw": cell.title
            .strip_prefix("reference ")
            .map(terminal_inline)
            .unwrap_or_else(|| "unknown".into()),
        "turn": extract_token_after(&cell.detail, "turn=")
            .unwrap_or_else(|| "none".into()),
        "path": extract_token_after(&cell.detail, "path=")
            .unwrap_or_else(|| "unknown".into()),
        "estimated_tokens": extract_token_after(&cell.detail, "estimated=")
            .unwrap_or_else(|| "0".into()),
        "actions": selected_cell_actions_json(Some(cell), &commands),
    })
}

pub(crate) fn context_coverage_json(source_coverage: Option<&WorkbenchCell>) -> serde_json::Value {
    serde_json::json!({
        "sources": source_coverage
            .and_then(|cell| extract_token_after(&cell.detail, "sources="))
            .unwrap_or_else(|| "none".into()),
        "trust": source_coverage
            .and_then(|cell| extract_token_after(&cell.detail, "trust="))
            .unwrap_or_else(|| "none".into()),
        "reasons": source_coverage
            .and_then(|cell| extract_token_after(&cell.detail, "reasons="))
            .unwrap_or_else(|| "none".into()),
    })
}

pub(crate) fn context_budget_status_json(cell: Option<&WorkbenchCell>) -> serde_json::Value {
    serde_json::json!({
        "turn": cell
            .and_then(|cell| extract_token_after(&cell.detail, "turn="))
            .unwrap_or_else(|| "none".into()),
        "estimator": cell
            .and_then(|cell| extract_token_after(&cell.detail, "estimator="))
            .unwrap_or_else(|| "unknown".into()),
        "used_tokens": cell
            .and_then(|cell| extract_token_after(&cell.detail, "used="))
            .or_else(|| cell.and_then(|cell| extract_token_after(&cell.detail, "token_budget=")))
            .unwrap_or_else(|| "0".into()),
        "max_tokens": cell
            .and_then(|cell| extract_token_after(&cell.detail, "max="))
            .unwrap_or_else(|| "0".into()),
        "context_window": cell
            .and_then(|cell| extract_token_after(&cell.detail, "context_window="))
            .unwrap_or_else(|| "unknown".into()),
        "reserved_output": cell
            .and_then(|cell| extract_token_after(&cell.detail, "reserved_output="))
            .unwrap_or_else(|| "unknown".into()),
        "source": cell
            .and_then(|cell| extract_token_after(&cell.detail, "source="))
            .unwrap_or_else(|| "unknown".into()),
    })
}

pub(crate) fn context_compaction_state_json(
    compaction: Option<&WorkbenchCell>,
) -> serde_json::Value {
    serde_json::json!({
        "active": compaction.is_some(),
        "compressed_sections": compaction
            .and_then(|cell| extract_token_after(&cell.detail, "compressed_sections="))
            .unwrap_or_else(|| "0".into()),
        "continuation_prompt": compaction
            .and_then(|cell| extract_token_after(&cell.detail, "continuation_prompt="))
            .unwrap_or_else(|| "no".into()),
        "summary": compaction
            .and_then(|cell| extract_assignment_span(
                &cell.detail,
                "summary=",
                &[" trace="],
            ))
            .unwrap_or_else(|| "none".into()),
    })
}
