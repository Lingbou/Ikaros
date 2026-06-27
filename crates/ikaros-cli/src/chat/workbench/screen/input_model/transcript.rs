// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::chat::workbench::screen) fn screen_surface_cell_json(
    cell: &WorkbenchCell,
) -> serde_json::Value {
    let commands = selected_cell_actions(cell)
        .into_iter()
        .filter(|command| command != "none")
        .collect::<Vec<_>>();
    serde_json::json!({
        "kind": cell.kind.as_str(),
        "title": terminal_inline(&cell.title),
        "detail": terminal_inline(&cell.detail),
        "intents": selected_cell_intents_json(&commands),
    })
}

pub(in crate::chat::workbench::screen) fn screen_transcript_model_json(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    active_cell: &serde_json::Value,
) -> serde_json::Value {
    let committed = screen
        .timeline
        .iter()
        .map(|cell| transcript_cell_json("timeline", cell, false))
        .chain(
            screen
                .main
                .iter()
                .filter(|cell| !cell.title.starts_with("active "))
                .map(|cell| transcript_cell_json("main", cell, false)),
        )
        .chain(
            screen
                .side
                .iter()
                .filter(|cell| !cell.title.starts_with("pending "))
                .map(|cell| transcript_cell_json("side", cell, false)),
        )
        .collect::<Vec<_>>();
    let live_tail_cells = screen
        .main
        .iter()
        .filter(|cell| cell.title.starts_with("active "))
        .map(|cell| transcript_cell_json("main", cell, true))
        .collect::<Vec<_>>();
    let cache_key = transcript_cache_key(screen, &live_tail_cells);
    serde_json::json!({
        "schema": "ikaros-workbench-transcript-v1",
        "version": 1,
        "render_mode": if state.raw_mode() { "raw" } else { "rich" },
        "raw_mode_available": true,
        "reflow": {
            "resize_reflow": true,
            "cells_are_logical": true,
            "live_tail_cache_key": cache_key,
        },
        "committed_count": committed.len(),
        "live_tail_count": live_tail_cells.len(),
        "has_live_tail": !live_tail_cells.is_empty() || !active_cell.is_null(),
        "committed": committed,
        "live_tail": live_tail_cells,
        "active_cell": active_cell.clone(),
        "actions": {
            "open_transcript": "/trace",
            "timeline": "/timeline",
            "failed": "/timeline --failed",
            "approval": "/timeline --approval",
            "raw": "/screen --raw",
            "rich": "/screen --rich",
        },
    })
}

pub(in crate::chat::workbench::screen) fn transcript_cell_json(
    source_panel: &'static str,
    cell: &WorkbenchCell,
    live: bool,
) -> serde_json::Value {
    let commands = selected_cell_actions(cell)
        .into_iter()
        .filter(|command| command != "none")
        .collect::<Vec<_>>();
    let rendered_detail = render_terminal_markdown(&cell.detail);
    let rendered_lines = rendered_detail
        .lines()
        .map(terminal_inline)
        .collect::<Vec<_>>();
    serde_json::json!({
        "source_panel": source_panel,
        "kind": cell.kind.as_str(),
        "title": terminal_inline(&cell.title),
        "detail": terminal_inline(&render_cell_detail_summary(&cell.detail)),
        "raw_detail": terminal_inline(&cell.detail),
        "rendered_detail": terminal_inline(&rendered_detail),
        "rendered_lines": rendered_lines,
        "markdown_features": markdown_feature_json(&cell.detail, &rendered_detail),
        "live": live,
        "cache_key": transcript_cell_cache_key(source_panel, cell, live),
        "transcript_key": transcript_cell_transcript_key(source_panel, cell),
        "height_hint": transcript_cell_height_hint(cell),
        "hyperlinks": transcript_cell_hyperlinks(cell),
        "actions": selected_cell_actions_json(Some(cell), &commands),
    })
}

pub(in crate::chat::workbench::screen) fn transcript_cache_key(
    screen: &WorkbenchScreen,
    live_tail_cells: &[serde_json::Value],
) -> String {
    let live_keys = live_tail_cells
        .iter()
        .filter_map(|cell| cell.get("cache_key").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "timeline:{}:main:{}:side:{}:live:{}:{}",
        screen.timeline.len(),
        screen.main.len(),
        screen.side.len(),
        live_tail_cells.len(),
        terminal_inline(&live_keys)
    )
}

pub(in crate::chat::workbench::screen) fn transcript_cell_cache_key(
    source_panel: &str,
    cell: &WorkbenchCell,
    live: bool,
) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        source_panel,
        cell.kind.as_str(),
        stable_text_fingerprint(&cell.title),
        stable_text_fingerprint(&cell.detail),
        if live { "live" } else { "committed" },
    )
}

pub(in crate::chat::workbench::screen) fn transcript_cell_transcript_key(
    source_panel: &str,
    cell: &WorkbenchCell,
) -> String {
    extract_token_after(&cell.detail, "event=")
        .or_else(|| extract_token_after(&cell.detail, "turn="))
        .map(|id| format!("{source_panel}:{id}"))
        .unwrap_or_else(|| {
            format!(
                "{}:{}:{}",
                source_panel,
                cell.kind.as_str(),
                stable_text_fingerprint(&cell.title)
            )
        })
}

pub(in crate::chat::workbench::screen) fn stable_text_fingerprint(input: &str) -> String {
    let hash = input.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}

pub(in crate::chat::workbench::screen) fn transcript_cell_height_hint(
    cell: &WorkbenchCell,
) -> usize {
    let detail_lines = render_cell_detail_summary(&cell.detail)
        .lines()
        .count()
        .max(1);
    1 + detail_lines
}

pub(in crate::chat::workbench::screen) fn transcript_cell_hyperlinks(
    cell: &WorkbenchCell,
) -> Vec<serde_json::Value> {
    selected_cell_actions(cell)
        .into_iter()
        .filter(|command| command.starts_with('/') && command != "none")
        .take(8)
        .map(|command| {
            serde_json::json!({
                "label": command_action(&command),
                "target": terminal_inline(&command),
                "kind": command_intent(&command),
            })
        })
        .collect()
}
