// SPDX-License-Identifier: GPL-3.0-only

use super::wrap;
use crate::{
    WorkbenchCellKind, WorkbenchScreen,
    input_model::{
        screen_input_model_json, screen_recovery_model_json, screen_turn_state_model_json,
    },
    panels::{
        all_cells, coding_phase_count, find_cell, latest_coding_failure_cell,
        screen_approval_panel_json, screen_coding_panel_json, screen_context_panel_json,
        screen_memory_panel_json, screen_provider_panel_json, screen_queue_continuation_cells,
        screen_queue_panel_json, screen_rag_panel_json, screen_surface_progress_json,
        screen_timeline_panel_json, screen_timeline_tabs,
    },
    render::json_string,
    selection::{command_with_prefix, extract_token_after, selected_cell_actions},
    terminal_inline,
};

pub(crate) fn main_dashboard_lines(
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

pub(crate) fn human_dashboard_lines(
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

pub(crate) fn screen_dashboard_model_json(screen: &WorkbenchScreen) -> serde_json::Value {
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

pub(crate) fn dashboard_item_json(
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

pub(crate) fn dashboard_json_bool(value: &serde_json::Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

pub(crate) fn dashboard_timeline_status(timeline: &serde_json::Value) -> String {
    if dashboard_json_bool(timeline, "has_failed_turn") {
        "failed".into()
    } else {
        "ok".into()
    }
}

pub(crate) fn dashboard_context_status(context: &serde_json::Value) -> String {
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

pub(crate) fn dashboard_memory_status(memory: &serde_json::Value) -> String {
    if json_string(memory, "pending_candidates", "0") != "0" {
        "pending".into()
    } else if json_string(memory, "working_active", "0") != "0" {
        "working".into()
    } else {
        "ok".into()
    }
}

pub(crate) fn dashboard_rag_status(rag: &serde_json::Value) -> String {
    if dashboard_json_bool(rag, "needs_attention") {
        "attention".into()
    } else if dashboard_json_bool(rag, "default_injection") {
        "injecting".into()
    } else {
        "idle".into()
    }
}

pub(crate) fn dashboard_approval_status(approval: &serde_json::Value) -> String {
    if json_string(approval, "pending", "0") != "0" {
        "pending".into()
    } else {
        "idle".into()
    }
}

pub(crate) fn dashboard_queue_status(queue: &serde_json::Value) -> String {
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

pub(crate) fn timeline_panel_summary(screen: &WorkbenchScreen) -> String {
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

pub(crate) fn timeline_tabs_summary(screen: &WorkbenchScreen) -> String {
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

pub(crate) fn provider_panel_summary(screen: &WorkbenchScreen) -> String {
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

pub(crate) fn context_panel_summary(screen: &WorkbenchScreen) -> String {
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

pub(crate) fn memory_panel_summary(screen: &WorkbenchScreen) -> String {
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

pub(crate) fn rag_panel_summary(screen: &WorkbenchScreen) -> String {
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

pub(crate) fn coding_panel_summary(screen: &WorkbenchScreen) -> String {
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

pub(crate) fn approval_panel_summary(screen: &WorkbenchScreen) -> String {
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

pub(crate) fn queue_panel_summary(screen: &WorkbenchScreen) -> String {
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
