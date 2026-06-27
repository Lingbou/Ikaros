// SPDX-License-Identifier: GPL-3.0-only

use super::{fit, human_cell_detail_lines, main_dashboard_lines, wrap};
use crate::{
    WorkbenchCell, WorkbenchCellKind, WorkbenchScreen, WorkbenchScreenPanel, WorkbenchScreenState,
    panels::{cell_matches_evidence_area, evidence_cell_needs_attention},
};
use ratatui::{
    style::{Modifier, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
};

pub(crate) fn panel_title<'a>(
    label: &'a str,
    panel: WorkbenchScreenPanel,
    state: &WorkbenchScreenState,
) -> std::borrow::Cow<'a, str> {
    if state.focused_panel() == panel {
        std::borrow::Cow::Owned(format!("{label}*"))
    } else {
        std::borrow::Cow::Borrowed(label)
    }
}

pub(crate) fn panel_paragraph<'a>(
    label: &'a str,
    panel: WorkbenchScreenPanel,
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> Paragraph<'a> {
    let (cells, width) = match panel {
        WorkbenchScreenPanel::Status => (&screen.status, 80),
        WorkbenchScreenPanel::Timeline => (&screen.timeline, 28),
        WorkbenchScreenPanel::Main => (&screen.main, 44),
        WorkbenchScreenPanel::Side => (&screen.side, 34),
    };
    let mut lines = if panel == WorkbenchScreenPanel::Main {
        main_dashboard_lines(screen, width, state.raw_mode())
    } else {
        Vec::new()
    };
    lines.extend(panel_lines(
        cells,
        width,
        state.scroll_for(panel),
        Some(state.selection_for(panel)),
        state.raw_mode(),
    ));
    let text = lines.join("\n");
    let title = panel_title(label, panel, state).into_owned();
    let style = if state.focused_panel() == panel {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    Paragraph::new(text)
        .block(Block::default().title(title).borders(Borders::ALL))
        .style(style)
        .wrap(Wrap { trim: false })
}

pub(crate) fn panel_lines(
    cells: &[WorkbenchCell],
    width: usize,
    scroll: usize,
    selected_index: Option<usize>,
    raw_mode: bool,
) -> Vec<String> {
    let visible_cells = cells.iter().skip(scroll).collect::<Vec<_>>();
    if visible_cells.is_empty() {
        return vec!["none".into()];
    }
    let mut lines = Vec::new();
    for (visible_index, cell) in visible_cells.iter().enumerate() {
        let absolute_index = scroll + visible_index;
        let marker = if selected_index == Some(absolute_index) {
            "> "
        } else {
            ""
        };
        lines.extend(wrap(
            &format!("{marker}[{}] {}", cell.kind.as_str(), cell.title),
            width,
        ));
        if raw_mode {
            lines.extend(wrap(&cell.detail, width));
        } else {
            lines.extend(human_cell_detail_lines(cell, width));
        }
    }
    lines
}

pub(crate) fn inline_cell_summary<'a>(
    cells: impl Iterator<Item = &'a WorkbenchCell>,
    width: usize,
) -> String {
    let mut parts = cells
        .filter(|cell| evidence_cell_needs_attention(cell))
        .map(|cell| format!("{}:{}", cell.kind.as_str(), cell.title))
        .collect::<Vec<_>>();
    if parts.is_empty() {
        parts.push("ready".into());
    }
    fit(parts.join(" "), width)
}

pub(crate) fn evidence_attention_summary(screen: &WorkbenchScreen) -> String {
    let mut areas = [
        "provider", "context", "memory", "rag", "coding", "approval", "queue", "gateway",
    ]
    .into_iter()
    .filter(|area| {
        screen
            .status
            .iter()
            .chain(screen.timeline.iter())
            .chain(screen.main.iter())
            .chain(screen.side.iter())
            .any(|cell| {
                cell_matches_evidence_area(cell, area, WorkbenchCellKind::Session)
                    && evidence_cell_needs_attention(cell)
            })
    })
    .collect::<Vec<_>>();
    areas.sort_unstable();
    areas.dedup();
    if areas.is_empty() {
        "attention=none".into()
    } else {
        format!("attention={}", areas.join(","))
    }
}
