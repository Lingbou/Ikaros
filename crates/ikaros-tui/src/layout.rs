// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use super::{input_model::*, panels::*, render::*, selection::*};

pub(super) fn panel_title<'a>(
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

pub(super) fn panel_paragraph<'a>(
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

pub(super) fn main_dashboard_lines(
    screen: &WorkbenchScreen,
    width: usize,
    raw_mode: bool,
) -> Vec<String> {
    let dashboard = screen_dashboard_model_json(screen);
    if !raw_mode {
        return human_dashboard_lines(screen, &dashboard, width);
    }
    let attention_count = dashboard
        .get("attention_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let primary_attention = dashboard
        .get("primary_attention")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none");
    let recovery_status = dashboard
        .get("recovery")
        .map(|value| json_string(value, "status", "idle"))
        .unwrap_or_else(|| "idle".into());
    let next_action = dashboard
        .get("recovery")
        .and_then(|value| value.get("primary"))
        .and_then(|value| value.get("command"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none");
    let mut lines = Vec::new();
    let mut raw_lines = vec![format!(
        "[dashboard] attention={} first_attention={} recovery_status={} next={} {} provider=/provider context=/context memory=/memory rag=/rag code=/code plan approval=/approval queue=/debug continuations",
        attention_count,
        primary_attention,
        recovery_status,
        next_action,
        timeline_tabs_summary(screen),
    )];
    if let Some(items) = dashboard.get("items").and_then(serde_json::Value::as_array) {
        for item in items {
            let id = json_string(item, "id", "unknown");
            let status = json_string(item, "status", "unknown");
            let summary = json_string(item, "summary", "none");
            let primary = json_string(item, "primary_action", "none");
            let focus = json_string(item, "focus_action", "none");
            let marker = if item
                .get("attention")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                "!"
            } else {
                ""
            };
            raw_lines.push(format!(
                "[{id}{marker}] status={status} {summary} open={primary} focus={focus}"
            ));
        }
    }
    raw_lines.push("---- main cells below; selection applies to these rows ----".to_owned());
    for line in raw_lines {
        lines.extend(wrap(&line, width));
    }
    lines
}

pub(super) fn human_dashboard_lines(
    screen: &WorkbenchScreen,
    dashboard: &serde_json::Value,
    width: usize,
) -> Vec<String> {
    let attention_count = dashboard
        .get("attention_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let mut lines = Vec::new();
    if attention_count == 0 {
        lines.extend(wrap("Ready. Type a message below.", width));
    } else {
        let areas = dashboard
            .get("items")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| {
                        item.get("attention")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false)
                    })
                    .map(|item| json_string(item, "label", "item"))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        lines.extend(wrap(
            &format!("Needs attention: {}", areas.join(", ")),
            width,
        ));
    }

    let provider = screen_provider_panel_json(screen);
    let provider_status = json_string(&provider, "health_status", "unknown");
    if provider_status != "ok" && provider_status != "unknown" {
        lines.extend(wrap(
            &format!("Provider status: {provider_status}. Press F5 for actions."),
            width,
        ));
    }

    let queue = screen_queue_panel_json(screen);
    let queued = json_string(&queue, "queued", "0");
    let running = json_string(&queue, "running", "0");
    if queued != "0" || running != "0" {
        lines.extend(wrap(
            &format!("Queue: {queued} waiting, {running} running."),
            width,
        ));
    }

    lines.extend(wrap("F5 commands. F1 help. Ctrl+C exits when idle.", width));
    lines
}

pub(super) fn screen_dashboard_model_json(screen: &WorkbenchScreen) -> serde_json::Value {
    let timeline = screen_timeline_panel_json(screen);
    let provider = screen_provider_panel_json(screen);
    let context = screen_context_panel_json(screen);
    let memory = screen_memory_panel_json(screen);
    let rag = screen_rag_panel_json(screen);
    let coding = screen_coding_panel_json(screen);
    let approval = screen_approval_panel_json(screen);
    let queue = screen_queue_panel_json(screen);
    let progress = screen_surface_progress_json(screen);
    let input_model = screen_input_model_json(screen, &serde_json::Value::Null);
    let turn_state = screen_turn_state_model_json(screen, &progress, &input_model);
    let recovery = screen_recovery_model_json(screen, &turn_state);

    let items = vec![
        dashboard_item_json(
            "timeline",
            "Timeline",
            "session",
            timeline_panel_summary(screen),
            dashboard_timeline_status(&timeline),
            dashboard_json_bool(&timeline, "has_failed_turn"),
            "/timeline",
            "/screen --focus timeline",
            "/trace",
            vec!["/replay", "/timeline --failed", "/trace --approval"],
        ),
        dashboard_item_json(
            "provider",
            "Provider",
            "model",
            provider_panel_summary(screen),
            json_string(&provider, "health_status", "unknown"),
            dashboard_json_bool(&provider, "needs_attention"),
            "/provider health",
            "/screen --focus main --select-action provider",
            "/provider matrix --live",
            vec!["/provider debug", "/budget", "/trace --kind model"],
        ),
        dashboard_item_json(
            "context",
            "Context",
            "context",
            context_panel_summary(screen),
            dashboard_context_status(&context),
            dashboard_json_bool(&context, "needs_attention"),
            "/context",
            "/screen --focus main --select-action context",
            "/trace --kind context",
            vec!["/timeline --kind context"],
        ),
        dashboard_item_json(
            "memory",
            "Memory",
            "memory",
            memory_panel_summary(screen),
            dashboard_memory_status(&memory),
            dashboard_json_bool(&memory, "needs_attention"),
            "/memory",
            "/screen --focus main --select-action memory",
            "/debug memory-lifecycle",
            vec!["/trace --kind memory", "memory projection render"],
        ),
        dashboard_item_json(
            "rag",
            "RAG",
            "retrieval",
            rag_panel_summary(screen),
            dashboard_rag_status(&rag),
            dashboard_json_bool(&rag, "needs_attention"),
            "/rag",
            "/screen --focus main --select-action rag",
            "rag search <query>",
            vec!["rag ingest <path>", "rag reindex", "rag stale"],
        ),
        dashboard_item_json(
            "coding",
            "Coding",
            "coding",
            coding_panel_summary(screen),
            json_string(&coding, "status", "idle"),
            dashboard_json_bool(&coding, "needs_attention"),
            "/code workflow --model-loop",
            "/screen --focus main --select-action code",
            "/code plan",
            vec!["/code test", "/code review", "/code rollback"],
        ),
        dashboard_item_json(
            "approval",
            "Approval",
            "approval",
            approval_panel_summary(screen),
            dashboard_approval_status(&approval),
            dashboard_json_bool(&approval, "needs_attention"),
            "/screen approve-selected",
            "/screen --focus side --select-action approval",
            "/approval",
            vec!["/screen deny-selected", "/trace --approval"],
        ),
        dashboard_item_json(
            "queue",
            "Queue",
            "continuation",
            queue_panel_summary(screen),
            dashboard_queue_status(&queue),
            dashboard_json_bool(&queue, "needs_attention"),
            "/debug continuations",
            "/screen --focus side --select-action queue",
            "/queue run",
            vec!["/cancel all", "/queue retry <id>"],
        ),
    ];
    let attention_count = items
        .iter()
        .filter(|item| {
            item.get("attention")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        })
        .count();
    let primary_attention = items
        .iter()
        .find(|item| {
            item.get("attention")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        })
        .and_then(|item| item.get("id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none")
        .to_owned();

    serde_json::json!({
        "schema": "ikaros-workbench-dashboard-v1",
        "layout": "status_cards",
        "attention_count": attention_count,
        "primary_attention": primary_attention,
        "turn_state": turn_state,
        "recovery": recovery,
        "items": items,
        "navigation": {
            "next": "/screen --down",
            "previous": "/screen --up",
            "open": "/screen open-selected",
            "confirm": "/screen confirm-selected",
            "focus_main": "/screen --focus main",
            "focus_side": "/screen --focus side",
        },
    })
}

pub(super) fn dashboard_item_json(
    id: &str,
    label: &str,
    category: &str,
    summary: String,
    status: String,
    attention: bool,
    primary_action: &str,
    focus_action: &str,
    open_action: &str,
    secondary_actions: Vec<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "label": label,
        "category": category,
        "summary": terminal_inline(&summary),
        "status": status,
        "attention": attention,
        "primary_action": primary_action,
        "focus_action": focus_action,
        "open_action": open_action,
        "secondary_actions": secondary_actions,
        "selection": {
            "enter": open_action,
            "alt_enter": primary_action,
            "focus": focus_action,
        },
    })
}

pub(super) fn dashboard_json_bool(value: &serde_json::Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

pub(super) fn dashboard_timeline_status(timeline: &serde_json::Value) -> String {
    if dashboard_json_bool(timeline, "has_failed_turn") {
        "failed".into()
    } else {
        "ok".into()
    }
}

pub(super) fn dashboard_context_status(context: &serde_json::Value) -> String {
    if context.get("limit").is_some_and(|value| !value.is_null()) {
        "limit".into()
    } else if context
        .get("compaction")
        .is_some_and(|value| !value.is_null())
    {
        "compacted".into()
    } else {
        "ok".into()
    }
}

pub(super) fn dashboard_memory_status(memory: &serde_json::Value) -> String {
    if json_string(memory, "pending_candidates", "0") != "0" {
        "pending".into()
    } else if json_string(memory, "working_active", "0") != "0" {
        "working".into()
    } else {
        "ok".into()
    }
}

pub(super) fn dashboard_rag_status(rag: &serde_json::Value) -> String {
    if dashboard_json_bool(rag, "needs_attention") {
        "attention".into()
    } else if dashboard_json_bool(rag, "default_injection") {
        "injecting".into()
    } else {
        "idle".into()
    }
}

pub(super) fn dashboard_approval_status(approval: &serde_json::Value) -> String {
    if json_string(approval, "pending", "0") != "0" {
        "pending".into()
    } else {
        "idle".into()
    }
}

pub(super) fn dashboard_queue_status(queue: &serde_json::Value) -> String {
    if json_string(queue, "failed", "0") != "0" {
        "failed".into()
    } else if json_string(queue, "running", "0") != "0" {
        "running".into()
    } else if json_string(queue, "queued", "0") != "0" {
        "queued".into()
    } else {
        "idle".into()
    }
}

pub(super) fn timeline_panel_summary(screen: &WorkbenchScreen) -> String {
    let navigator = find_cell(screen, |cell| cell.title == "timeline navigator");
    let replay = find_cell(screen, |cell| cell.title == "replay navigation");
    format!(
        "turns={} events={} approvals={} latest={} failed={} pages={} timeline=/timeline trace=/trace replay=/replay",
        navigator
            .and_then(|cell| extract_token_after(&cell.detail, "turns="))
            .unwrap_or_else(|| "0".into()),
        navigator
            .and_then(|cell| extract_token_after(&cell.detail, "events="))
            .unwrap_or_else(|| "0".into()),
        navigator
            .and_then(|cell| extract_token_after(&cell.detail, "approvals="))
            .unwrap_or_else(|| "0".into()),
        navigator
            .and_then(|cell| extract_token_after(&cell.detail, "latest="))
            .unwrap_or_else(|| "none".into()),
        navigator
            .and_then(|cell| extract_token_after(&cell.detail, "failed="))
            .unwrap_or_else(|| "none".into()),
        replay
            .and_then(|cell| extract_token_after(&cell.detail, "pages="))
            .unwrap_or_else(|| "1".into()),
    )
}

pub(super) fn timeline_tabs_summary(screen: &WorkbenchScreen) -> String {
    let tabs = screen_timeline_tabs(screen)
        .into_iter()
        .filter(|tab| tab.count > 0 || tab.attention)
        .map(|tab| {
            let marker = if tab.attention { "!" } else { "" };
            format!("{}:{}{}", tab.id, tab.count, marker)
        })
        .collect::<Vec<_>>();
    if tabs.is_empty() {
        "tabs=none".into()
    } else {
        format!("tabs={}", tabs.join(","))
    }
}

pub(super) fn provider_panel_summary(screen: &WorkbenchScreen) -> String {
    let matrix = find_cell(screen, |cell| cell.title == "provider matrix");
    let budget = find_cell(screen, |cell| cell.title == "model budget");
    let recovery = find_cell(screen, |cell| cell.title == "provider recovery");
    format!(
        "provider={} model={} context_window={} budget_status={} recovery={} matrix=/provider matrix health=/provider health debug=/provider debug",
        matrix
            .and_then(|cell| extract_token_after(&cell.detail, "provider="))
            .unwrap_or_else(|| "unknown".into()),
        matrix
            .and_then(|cell| extract_token_after(&cell.detail, "model="))
            .unwrap_or_else(|| "unknown".into()),
        matrix
            .and_then(|cell| extract_token_after(&cell.detail, "context_window="))
            .unwrap_or_else(|| "unknown".into()),
        budget
            .and_then(|cell| extract_token_after(&cell.detail, "budget_status="))
            .unwrap_or_else(|| "unknown".into()),
        recovery
            .and_then(|cell| extract_token_after(&cell.detail, "status="))
            .unwrap_or_else(|| "unknown".into()),
    )
}

pub(super) fn context_panel_summary(screen: &WorkbenchScreen) -> String {
    let budget = find_cell(screen, |cell| cell.title == "context budget");
    let current = find_cell(screen, |cell| cell.title == "context current");
    format!(
        "budget={} sections={} references={} disabled={} context=/context trace=/trace --kind context",
        budget
            .and_then(|cell| extract_token_after(&cell.detail, "used_tokens="))
            .unwrap_or_else(|| "unknown".into()),
        all_cells(screen)
            .filter(|cell| cell.title.starts_with("section "))
            .count(),
        all_cells(screen)
            .filter(|cell| cell.title.starts_with("reference "))
            .count(),
        current
            .and_then(|cell| extract_token_after(&cell.detail, "disabled="))
            .unwrap_or_else(|| "unknown".into()),
    )
}

pub(super) fn memory_panel_summary(screen: &WorkbenchScreen) -> String {
    let memory = find_cell(screen, |cell| cell.title == "memory");
    format!(
        "backend={} candidates={} working={} journal={} memory=/memory lifecycle=/debug memory-lifecycle",
        memory
            .and_then(|cell| extract_token_after(&cell.detail, "backend="))
            .unwrap_or_else(|| "unknown".into()),
        memory
            .and_then(|cell| extract_token_after(&cell.detail, "pending_candidates="))
            .unwrap_or_else(|| "0".into()),
        memory
            .and_then(|cell| extract_token_after(&cell.detail, "working_active="))
            .unwrap_or_else(|| "0".into()),
        memory
            .and_then(|cell| extract_token_after(&cell.detail, "journal_entries="))
            .unwrap_or_else(|| "0".into()),
    )
}

pub(super) fn rag_panel_summary(screen: &WorkbenchScreen) -> String {
    let rag = find_cell(screen, |cell| cell.title == "rag");
    format!(
        "backend={} embedding={} top_k={} rag=/rag search=rag search ingest=rag ingest",
        rag.and_then(|cell| extract_token_after(&cell.detail, "backend="))
            .unwrap_or_else(|| "unknown".into()),
        rag.and_then(|cell| extract_token_after(&cell.detail, "embedding_provider="))
            .unwrap_or_else(|| "unknown".into()),
        rag.and_then(|cell| extract_token_after(&cell.detail, "top_k="))
            .unwrap_or_else(|| "0".into()),
    )
}

pub(super) fn coding_panel_summary(screen: &WorkbenchScreen) -> String {
    let workflow = find_cell(screen, |cell| cell.title == "coding workflow");
    let diff_count = coding_phase_count(workflow, "diff=");
    let test_count = coding_phase_count(workflow, "test=");
    let review_count = coding_phase_count(workflow, "review=");
    let needs_attention = workflow
        .and_then(|cell| extract_token_after(&cell.detail, "status="))
        .is_some_and(|status| status == "attention")
        || latest_coding_failure_cell(screen).is_some();
    format!(
        "status={} events={} latest_turn={} diff={} test={} review={} attention={} plan=/code plan workflow=/code workflow --model-loop test=/code test review=/code review rollback=/code rollback",
        workflow
            .and_then(|cell| extract_token_after(&cell.detail, "status="))
            .unwrap_or_else(|| "idle".into()),
        workflow
            .and_then(|cell| extract_token_after(&cell.detail, "events="))
            .unwrap_or_else(|| "0".into()),
        workflow
            .and_then(|cell| extract_token_after(&cell.detail, "latest_turn="))
            .unwrap_or_else(|| "none".into()),
        diff_count,
        test_count,
        review_count,
        if needs_attention { "yes" } else { "no" },
    )
}

pub(super) fn approval_panel_summary(screen: &WorkbenchScreen) -> String {
    let controls = find_cell(screen, |cell| cell.title == "approval controls");
    let pending = controls
        .and_then(|cell| extract_token_after(&cell.detail, "pending="))
        .unwrap_or_else(|| {
            all_cells(screen)
                .filter(|cell| {
                    matches!(cell.kind, WorkbenchCellKind::Approval)
                        && cell.title.starts_with("pending ")
                })
                .count()
                .to_string()
        });
    format!(
        "pending={} high_risk={} provider={} write={} shell={} network={} approve=/screen approve-selected deny=/screen deny-selected list=/approval trace=/trace --approval",
        pending,
        controls
            .and_then(|cell| extract_token_after(&cell.detail, "high_risk="))
            .unwrap_or_else(|| "0".into()),
        controls
            .and_then(|cell| extract_token_after(&cell.detail, "provider="))
            .unwrap_or_else(|| "0".into()),
        controls
            .and_then(|cell| extract_token_after(&cell.detail, "write="))
            .unwrap_or_else(|| "0".into()),
        controls
            .and_then(|cell| extract_token_after(&cell.detail, "shell="))
            .unwrap_or_else(|| "0".into()),
        controls
            .and_then(|cell| extract_token_after(&cell.detail, "network="))
            .unwrap_or_else(|| "0".into()),
    )
}

pub(super) fn queue_panel_summary(screen: &WorkbenchScreen) -> String {
    let queue = find_cell(screen, |cell| cell.title == "queue");
    let bottom = find_cell(screen, |cell| cell.title == "bottom pane");
    let active = screen_queue_continuation_cells(screen)
        .into_iter()
        .find(|cell| {
            cell.detail.contains("status=running") || cell.detail.contains("status=queued")
        });
    let failed = screen_queue_continuation_cells(screen)
        .into_iter()
        .find(|cell| cell.detail.contains("status=failed"));
    format!(
        "queued={} running={} failed={} pending_inputs={} active={} retry={} run=/queue run cancel=/cancel all debug=/debug continuations",
        queue
            .and_then(|cell| extract_token_after(&cell.detail, "queued="))
            .unwrap_or_else(|| "0".into()),
        queue
            .and_then(|cell| extract_token_after(&cell.detail, "running="))
            .unwrap_or_else(|| "0".into()),
        queue
            .and_then(|cell| extract_token_after(&cell.detail, "failed="))
            .unwrap_or_else(|| "0".into()),
        bottom
            .and_then(|cell| extract_token_after(&cell.detail, "pending_inputs="))
            .unwrap_or_else(|| "0".into()),
        active
            .and_then(|cell| extract_token_after(&cell.detail, "id="))
            .unwrap_or_else(|| "none".into()),
        failed
            .map(selected_cell_actions)
            .and_then(|commands| command_with_prefix(&commands, "/queue retry "))
            .unwrap_or_else(|| "none".into()),
    )
}

pub(super) fn buffer_snapshot(buffer: &Buffer) -> String {
    let width = buffer.area.width as usize;
    buffer
        .content()
        .chunks(width)
        .map(|cells| {
            cells
                .iter()
                .filter(|cell| !cell.skip)
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn border_title(title: &str, width: usize) -> String {
    let title = format!(" {} ", terminal_inline(title).to_ascii_uppercase());
    if title.chars().count() + 2 >= width {
        return framed_line(&title, width);
    }
    let side = width.saturating_sub(title.chars().count() + 2);
    let left = side / 2;
    let right = side.saturating_sub(left);
    format!("+{}{}{}+", "-".repeat(left), title, "-".repeat(right))
}

pub(super) fn separator(width: usize) -> String {
    format!("+{}+", "-".repeat(width.saturating_sub(2)))
}

pub(super) fn framed_line(text: &str, width: usize) -> String {
    let inside = fit(terminal_inline(text), width.saturating_sub(2));
    format!("|{inside}|")
}

pub(super) fn three_column_row(
    left: impl AsRef<str>,
    main: impl AsRef<str>,
    side: impl AsRef<str>,
    width: usize,
) -> String {
    let (left_width, main_width, side_width) = column_widths(width);
    format!(
        "|{}|{}|{}|",
        fit(terminal_inline(left.as_ref()), left_width),
        fit(terminal_inline(main.as_ref()), main_width),
        fit(terminal_inline(side.as_ref()), side_width)
    )
}

pub(super) fn column_widths(width: usize) -> (usize, usize, usize) {
    let inner = width.saturating_sub(4).max(36);
    let left = (inner / 4).max(10);
    let side = (inner / 3).max(12);
    let main = inner.saturating_sub(left + side).max(10);
    (left, main, side)
}

pub(super) fn panel_lines(
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
            "› "
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

pub(super) fn inline_cell_summary<'a>(
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

pub(super) fn human_cell_detail_lines(cell: &WorkbenchCell, width: usize) -> Vec<String> {
    wrap_markdown_detail(&human_cell_detail(cell), width)
}

pub(super) fn human_cell_detail(cell: &WorkbenchCell) -> String {
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

pub(super) fn evidence_attention_summary(screen: &WorkbenchScreen) -> String {
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

pub(super) fn wrap(input: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines = Vec::new();
    for raw_line in input.lines() {
        let line = terminal_inline(raw_line);
        if line.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        lines.extend(wrap_single_line(&line, width));
    }
    if input.is_empty() {
        lines.push("none".into());
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        lines.push("none".into());
    }
    lines
}

pub(super) fn wrap_markdown_detail(input: &str, width: usize) -> Vec<String> {
    wrap(&render_terminal_markdown(input), width)
}

pub(super) fn render_cell_detail_summary(input: &str) -> String {
    render_terminal_markdown(input)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" / ")
}

pub(super) fn markdown_feature_json(raw: &str, rendered: &str) -> serde_json::Value {
    let has_code = rendered.contains("[code") || rendered.contains("[diff]");
    let has_diff = rendered.contains("[diff]")
        || raw.lines().any(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("+ ") || trimmed.starts_with("- ") || trimmed.starts_with("@@")
        });
    let has_table = rendered.contains("[table]") || raw.lines().any(is_markdown_table_line);
    let has_heading = raw
        .lines()
        .any(|line| markdown_heading(line.trim()).is_some());
    let has_list = raw.lines().any(|line| {
        let trimmed = line.trim();
        markdown_unordered_item_like(trimmed) || markdown_ordered_item_like(trimmed)
    });
    serde_json::json!({
        "has_code": has_code,
        "has_diff": has_diff,
        "has_table": has_table,
        "has_heading": has_heading,
        "has_list": has_list,
        "render_kind": markdown_render_kind(raw, rendered),
        "diff_stats": markdown_diff_stats_json(rendered),
    })
}

pub(super) fn markdown_render_kind(raw: &str, rendered: &str) -> &'static str {
    if rendered.contains("[diff]") || raw.contains("kind=diff_updated") {
        "diff"
    } else if rendered.contains("[code") {
        "code"
    } else if rendered.contains("[table]") {
        "table"
    } else if raw.contains("status=failed") || raw.contains("error=") {
        "error"
    } else if raw.contains("kind=review") || raw.contains("finding") {
        "review"
    } else {
        "markdown"
    }
}

pub(super) fn markdown_diff_stats_json(rendered: &str) -> serde_json::Value {
    let mut additions = 0usize;
    let mut deletions = 0usize;
    let mut hunks = 0usize;
    for line in rendered.lines() {
        if line.starts_with("+") {
            additions += 1;
        } else if line.starts_with("-") {
            deletions += 1;
        } else if line.trim_start().starts_with("@@") {
            hunks += 1;
        }
    }
    serde_json::json!({
        "additions": additions,
        "deletions": deletions,
        "hunks": hunks,
    })
}

pub(super) fn markdown_unordered_item_like(trimmed: &str) -> bool {
    ["- ", "* ", "+ "]
        .into_iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

pub(super) fn markdown_ordered_item_like(trimmed: &str) -> bool {
    let Some(dot) = trimmed.find('.') else {
        return false;
    };
    let (number, rest) = trimmed.split_at(dot);
    number.chars().all(|ch| ch.is_ascii_digit()) && rest.starts_with(". ")
}

pub(super) fn wrap_single_line(input: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in input.split_whitespace() {
        if word.chars().count() > width {
            if !current.is_empty() {
                lines.push(current);
                current = String::new();
            }
            let mut chunk = String::new();
            for ch in word.chars() {
                if chunk.chars().count() == width {
                    lines.push(chunk);
                    chunk = String::new();
                }
                chunk.push(ch);
            }
            if !chunk.is_empty() {
                current = chunk;
            }
        } else if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = word.to_owned();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push("none".into());
    }
    lines
}

pub(super) fn fit(input: String, width: usize) -> String {
    let mut output = input.chars().take(width).collect::<String>();
    let padding = width.saturating_sub(output.chars().count());
    output.push_str(&" ".repeat(padding));
    output
}
