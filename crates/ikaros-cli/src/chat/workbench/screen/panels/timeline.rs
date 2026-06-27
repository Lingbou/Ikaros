// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::chat::workbench::screen) fn screen_timeline_groups_json(
    screen: &WorkbenchScreen,
) -> Vec<serde_json::Value> {
    screen
        .timeline
        .iter()
        .filter_map(|cell| {
            let category = cell.title.strip_prefix("timeline ")?;
            Some(serde_json::json!({
                "category": terminal_inline(category),
                "kind": cell.kind.as_str(),
                "events": extract_token_after(&cell.detail, "events=")
                    .unwrap_or_else(|| "0".into()),
                "latest_turn": extract_token_after(&cell.detail, "latest_turn=")
                    .unwrap_or_else(|| "none".into()),
                "timeline": command_with_prefix(
                    &selected_cell_actions(cell),
                    "/timeline --kind ",
                ),
                "trace": command_with_prefix(
                    &selected_cell_actions(cell),
                    "/trace --kind ",
                ),
                "replay": command_with_prefix(
                    &selected_cell_actions(cell),
                    "/replay --kind ",
                ),
            }))
        })
        .collect()
}

pub(in crate::chat::workbench::screen) fn screen_timeline_group_model_json(
    screen: &WorkbenchScreen,
) -> serde_json::Value {
    let groups = screen_timeline_groups_json(screen);
    let coding_workflow = find_cell(screen, |cell| cell.title == "coding workflow");
    let group_lookup = |category: &str| {
        groups
            .iter()
            .find(|group| {
                group.get("category").and_then(serde_json::Value::as_str) == Some(category)
            })
            .cloned()
    };
    let required = [
        (
            "session",
            WorkbenchCellKind::Session,
            "/timeline --kind session",
        ),
        ("model", WorkbenchCellKind::Model, "/timeline --kind model"),
        ("tool", WorkbenchCellKind::Tool, "/timeline --kind tool"),
        (
            "context",
            WorkbenchCellKind::Context,
            "/timeline --kind context",
        ),
        (
            "memory",
            WorkbenchCellKind::Memory,
            "/timeline --kind memory",
        ),
        (
            "coding",
            WorkbenchCellKind::Coding,
            "/timeline --kind coding",
        ),
        ("test", WorkbenchCellKind::Coding, "/timeline --kind coding"),
        ("audit", WorkbenchCellKind::Audit, "/timeline --kind audit"),
        (
            "continuation",
            WorkbenchCellKind::Continuation,
            "/timeline --kind continuation",
        ),
        (
            "approval",
            WorkbenchCellKind::Approval,
            "/timeline --kind approval",
        ),
        ("error", WorkbenchCellKind::Error, "/timeline --kind error"),
    ];
    let items = required
        .into_iter()
        .map(|(category, kind, command)| {
            let existing = group_lookup(category);
            let mut events = existing
                .as_ref()
                .and_then(|group| group.get("events"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("0")
                .to_owned();
            if category == "test" && events == "0" {
                events = coding_phase_count(coding_workflow, "test=");
            }
            let latest_turn = existing
                .as_ref()
                .and_then(|group| group.get("latest_turn"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("none")
                .to_owned();
            serde_json::json!({
                "category": category,
                "kind": kind.as_str(),
                "visible": events != "0",
                "events": events,
                "latest_turn": latest_turn,
                "timeline": existing
                    .as_ref()
                    .and_then(|group| group.get("timeline"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!(command)),
                "trace": existing
                    .as_ref()
                    .and_then(|group| group.get("trace"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!(command.replace("/timeline", "/trace"))),
                "replay": existing
                    .as_ref()
                    .and_then(|group| group.get("replay"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!(command.replace("/timeline", "/replay"))),
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "schema": "ikaros-workbench-timeline-groups-v1",
        "version": 1,
        "layout": "stable_category_slots",
        "items": items,
        "actions": {
            "timeline": "/timeline",
            "trace": "/trace",
            "replay": "/replay",
            "failed": "/timeline --failed",
            "approval": "/timeline --approval",
        },
    })
}

#[derive(Debug, Clone)]
pub(in crate::chat::workbench::screen) struct TimelineTabModel {
    pub(in crate::chat::workbench::screen) id: &'static str,
    pub(in crate::chat::workbench::screen) label: &'static str,
    pub(in crate::chat::workbench::screen) count: usize,
    pub(in crate::chat::workbench::screen) attention: bool,
    pub(in crate::chat::workbench::screen) kind_filter: Option<&'static str>,
    pub(in crate::chat::workbench::screen) timeline_command: String,
    pub(in crate::chat::workbench::screen) trace_command: String,
    pub(in crate::chat::workbench::screen) replay_command: String,
    pub(in crate::chat::workbench::screen) focus_action: String,
    pub(in crate::chat::workbench::screen) shortcut: Option<&'static str>,
}

pub(in crate::chat::workbench::screen) fn screen_timeline_tabs_model_json(
    screen: &WorkbenchScreen,
) -> serde_json::Value {
    let tabs = screen_timeline_tabs(screen);
    let active_tab = tabs
        .iter()
        .find(|tab| tab.attention)
        .or_else(|| tabs.iter().find(|tab| tab.count > 0 && tab.id != "all"))
        .map(|tab| tab.id)
        .unwrap_or("all");
    let attention_count = tabs.iter().filter(|tab| tab.attention).count();
    let total_visible = tabs
        .iter()
        .find(|tab| tab.id == "all")
        .map(|tab| tab.count)
        .unwrap_or(0);
    serde_json::json!({
        "schema": "ikaros-workbench-timeline-tabs-v1",
        "version": 1,
        "layout": "tabbed_replay_filter",
        "active_tab": active_tab,
        "attention_count": attention_count,
        "total_visible": total_visible,
        "tabs": tabs
            .into_iter()
            .map(screen_timeline_tab_json)
            .collect::<Vec<_>>(),
        "navigation": {
            "previous": "/screen --select-prev",
            "next": "/screen --select-next",
            "all": "/screen --select-action timeline_all",
            "failed": "/screen --select-action timeline_error",
            "approval": "/screen --select-action timeline_approval",
        },
    })
}

pub(in crate::chat::workbench::screen) fn screen_timeline_tab_json(
    tab: TimelineTabModel,
) -> serde_json::Value {
    serde_json::json!({
        "id": tab.id,
        "label": tab.label,
        "count": tab.count,
        "visible": tab.count > 0 || matches!(tab.id, "all" | "error"),
        "attention": tab.attention,
        "kind_filter": tab.kind_filter,
        "focus_action": tab.focus_action,
        "commands": {
            "timeline": tab.timeline_command,
            "trace": tab.trace_command,
            "replay": tab.replay_command,
        },
        "shortcut": tab.shortcut,
    })
}

pub(in crate::chat::workbench::screen) fn screen_timeline_tabs(
    screen: &WorkbenchScreen,
) -> Vec<TimelineTabModel> {
    [
        ("all", "All", None, Some("ctrl-t")),
        ("model", "Model", Some("model"), None),
        ("provider", "Provider", Some("model"), None),
        ("context", "Context", Some("context"), None),
        ("tool", "Tool", Some("tool"), None),
        ("approval", "Approval", Some("approval"), None),
        ("coding", "Coding", Some("coding"), None),
        ("test", "Test", Some("coding"), None),
        ("audit", "Audit", Some("audit"), None),
        ("memory", "Memory", Some("memory"), None),
        ("queue", "Queue", Some("continuation"), None),
        ("gateway", "Gateway", Some("gateway"), None),
        ("error", "Errors", Some("error"), None),
    ]
    .into_iter()
    .map(|(id, label, kind_filter, shortcut)| {
        let count = timeline_tab_count(screen, id);
        let attention = timeline_tab_needs_attention(screen, id, count);
        let (timeline_command, trace_command, replay_command) =
            timeline_tab_commands(id, kind_filter);
        TimelineTabModel {
            id,
            label,
            count,
            attention,
            kind_filter,
            timeline_command,
            trace_command,
            replay_command,
            focus_action: format!("/screen --select-action timeline_{id}"),
            shortcut,
        }
    })
    .collect()
}

pub(in crate::chat::workbench::screen) fn timeline_tab_commands(
    id: &'static str,
    kind_filter: Option<&'static str>,
) -> (String, String, String) {
    match (id, kind_filter) {
        ("all", _) => ("/timeline".into(), "/trace".into(), "/replay".into()),
        ("gateway", _) => (
            "/gateway".into(),
            "/trace --kind gateway".into(),
            "/replay --kind gateway".into(),
        ),
        (_, Some(kind)) => (
            format!("/timeline --kind {kind}"),
            format!("/trace --kind {kind}"),
            format!("/replay --kind {kind}"),
        ),
        _ => ("/timeline".into(), "/trace".into(), "/replay".into()),
    }
}

pub(in crate::chat::workbench::screen) fn timeline_tab_count(
    screen: &WorkbenchScreen,
    id: &str,
) -> usize {
    match id {
        "all" => timeline_visible_item_count(screen),
        "provider" => timeline_provider_count(screen),
        "test" => timeline_group_event_count(screen, "test").max(timeline_cells_matching(
            screen,
            |cell| {
                matches!(cell.kind, WorkbenchCellKind::Coding)
                    && timeline_cell_contains(cell, &["test=", "tests=", "test "])
            },
        )),
        "queue" => timeline_group_event_count(screen, "continuation").max(timeline_cells_of_kind(
            screen,
            WorkbenchCellKind::Continuation,
        )),
        "gateway" => timeline_cells_matching(screen, |cell| {
            timeline_cell_contains(cell, &["gateway", "webhook", "outbox", "inbox"])
        }),
        "error" => timeline_group_event_count(screen, "error")
            .max(timeline_cells_of_kind(screen, WorkbenchCellKind::Error)),
        category => timeline_group_event_count(screen, category).max(timeline_cells_matching(
            screen,
            |cell| {
                cell.kind.as_str() == category
                    || cell.title.strip_prefix("timeline ") == Some(category)
            },
        )),
    }
}

pub(in crate::chat::workbench::screen) fn timeline_visible_item_count(
    screen: &WorkbenchScreen,
) -> usize {
    find_cell(screen, |cell| cell.title == "replay navigation")
        .and_then(|cell| extract_token_after(&cell.detail, "items="))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(screen.timeline.len())
}

pub(in crate::chat::workbench::screen) fn timeline_group_event_count(
    screen: &WorkbenchScreen,
    category: &str,
) -> usize {
    let title = format!("timeline {category}");
    screen
        .timeline
        .iter()
        .find(|cell| cell.title == title)
        .and_then(|cell| extract_token_after(&cell.detail, "events="))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0)
}

pub(in crate::chat::workbench::screen) fn timeline_provider_count(
    screen: &WorkbenchScreen,
) -> usize {
    timeline_group_event_count(screen, "model").max(timeline_cells_matching(screen, |cell| {
        matches!(cell.kind, WorkbenchCellKind::Model)
            || timeline_cell_contains(
                cell,
                &[
                    "provider",
                    "model budget",
                    "fallback",
                    "cooldown",
                    "token budget",
                ],
            )
    }))
}

pub(in crate::chat::workbench::screen) fn timeline_cells_of_kind(
    screen: &WorkbenchScreen,
    kind: WorkbenchCellKind,
) -> usize {
    timeline_cells_matching(screen, |cell| cell.kind == kind)
}

pub(in crate::chat::workbench::screen) fn timeline_cells_matching(
    screen: &WorkbenchScreen,
    predicate: impl Fn(&WorkbenchCell) -> bool,
) -> usize {
    all_cells(screen).filter(|cell| predicate(cell)).count()
}

pub(in crate::chat::workbench::screen) fn timeline_cell_contains(
    cell: &WorkbenchCell,
    needles: &[&str],
) -> bool {
    let title = cell.title.to_ascii_lowercase();
    let detail = cell.detail.to_ascii_lowercase();
    needles
        .iter()
        .any(|needle| title.contains(needle) || detail.contains(needle))
}

pub(in crate::chat::workbench::screen) fn timeline_tab_needs_attention(
    screen: &WorkbenchScreen,
    id: &str,
    count: usize,
) -> bool {
    if count == 0 {
        return false;
    }
    match id {
        "error" => true,
        "approval" => {
            timeline_cells_matching(screen, |cell| {
                matches!(cell.kind, WorkbenchCellKind::Approval)
                    || timeline_cell_contains(cell, &["approval_pending", "pending approval"])
            }) > 0
        }
        "queue" => {
            timeline_cells_matching(screen, |cell| {
                matches!(cell.kind, WorkbenchCellKind::Continuation)
                    && timeline_cell_contains(cell, &["failed", "cancelled", "timeout", "expired"])
            }) > 0
        }
        "provider" | "model" => {
            timeline_cells_matching(screen, |cell| {
                matches!(
                    cell.kind,
                    WorkbenchCellKind::Model | WorkbenchCellKind::Error
                ) && timeline_cell_contains(
                    cell,
                    &[
                        "budget exceeded",
                        "cooldown",
                        "fallback",
                        "provider_retry_failed",
                    ],
                )
            }) > 0
        }
        "coding" | "test" => {
            timeline_cells_matching(screen, |cell| {
                matches!(
                    cell.kind,
                    WorkbenchCellKind::Coding | WorkbenchCellKind::Error
                ) && timeline_cell_contains(
                    cell,
                    &["failed", "failure", "review_finding", "rollback"],
                )
            }) > 0
        }
        "context" => {
            timeline_cells_matching(screen, |cell| {
                matches!(
                    cell.kind,
                    WorkbenchCellKind::Context | WorkbenchCellKind::Error
                ) && timeline_cell_contains(cell, &["context_limit", "compacted", "over_budget"])
            }) > 0
        }
        "memory" => {
            timeline_cells_matching(screen, |cell| {
                matches!(cell.kind, WorkbenchCellKind::Memory)
                    && timeline_cell_contains(cell, &["skipped", "forget", "demote"])
            }) > 0
        }
        "gateway" => {
            timeline_cells_matching(screen, |cell| {
                timeline_cell_contains(
                    cell,
                    &["gateway failed", "webhook failed", "delivery failed"],
                )
            }) > 0
        }
        _ => false,
    }
}

pub(in crate::chat::workbench::screen) fn screen_timeline_panel_json(
    screen: &WorkbenchScreen,
) -> serde_json::Value {
    let navigator = find_cell(screen, |cell| cell.title == "timeline navigator");
    let replay = find_cell(screen, |cell| cell.title == "replay navigation");
    let older_page = find_cell(screen, |cell| cell.title == "older replay page");
    let failed = find_cell(screen, |cell| cell.title == "replay failed");
    let approval = find_cell(screen, |cell| cell.title == "replay approval");
    let groups = find_cell(screen, |cell| cell.title == "timeline groups");
    serde_json::json!({
        "navigator": navigator.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "replay": replay.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "older_page": older_page.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "groups_summary": groups.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "tabs": screen_timeline_tabs_model_json(screen),
        "groups": screen_timeline_groups_json(screen),
        "filters": screen_timeline_filters_json(screen),
        "quick_jumps": screen_timeline_quick_jumps_json(navigator, failed, approval),
        "turns": navigator
            .and_then(|cell| extract_token_after(&cell.detail, "turns="))
            .unwrap_or_else(|| "0".into()),
        "entries": navigator
            .and_then(|cell| extract_token_after(&cell.detail, "entries="))
            .unwrap_or_else(|| "0".into()),
        "events": navigator
            .and_then(|cell| extract_token_after(&cell.detail, "events="))
            .unwrap_or_else(|| "0".into()),
        "approvals": navigator
            .and_then(|cell| extract_token_after(&cell.detail, "approvals="))
            .unwrap_or_else(|| "0".into()),
        "latest_turn": navigator
            .and_then(|cell| extract_token_after(&cell.detail, "latest="))
            .unwrap_or_else(|| "none".into()),
        "failed_turn": navigator
            .and_then(|cell| extract_token_after(&cell.detail, "failed="))
            .unwrap_or_else(|| "none".into()),
        "has_failed_turn": failed.is_some()
            || navigator
                .and_then(|cell| extract_token_after(&cell.detail, "failed="))
                .is_some_and(|turn| turn != "none"),
        "visible_items": replay
            .and_then(|cell| extract_token_after(&cell.detail, "items="))
            .unwrap_or_else(|| "0".into()),
        "page_size": replay
            .and_then(|cell| extract_token_after(&cell.detail, "page_size="))
            .unwrap_or_else(|| "0".into()),
        "pages": replay
            .and_then(|cell| extract_token_after(&cell.detail, "pages="))
            .unwrap_or_else(|| "1".into()),
        "failed_point": failed
            .map(screen_replay_point_json)
            .unwrap_or(serde_json::Value::Null),
        "approval_point": approval
            .map(screen_replay_point_json)
            .unwrap_or(serde_json::Value::Null),
        "actions": navigator
            .or(replay)
            .map(|cell| selected_cell_actions_json(Some(cell), &selected_cell_actions(cell)))
            .unwrap_or_else(|| selected_cell_actions_json(None, &[])),
    })
}

pub(in crate::chat::workbench::screen) fn screen_timeline_filters_json(
    screen: &WorkbenchScreen,
) -> Vec<serde_json::Value> {
    let mut filters = screen
        .timeline
        .iter()
        .filter_map(|cell| {
            let category = cell.title.strip_prefix("timeline ")?;
            let commands = selected_cell_actions(cell);
            Some(serde_json::json!({
                "kind": terminal_inline(category),
                "events": extract_token_after(&cell.detail, "events=")
                    .unwrap_or_else(|| "0".into()),
                "latest_turn": extract_token_after(&cell.detail, "latest_turn=")
                    .unwrap_or_else(|| "none".into()),
                "timeline": command_with_prefix(&commands, "/timeline --kind "),
                "trace": command_with_prefix(&commands, "/trace --kind "),
                "replay": command_with_prefix(&commands, "/replay --kind "),
            }))
        })
        .collect::<Vec<_>>();
    filters.push(serde_json::json!({
        "kind": "failed",
        "events": "unknown",
        "latest_turn": "latest_failed",
        "timeline": "/timeline --failed",
        "trace": "/trace --failed",
        "replay": "/replay --failed",
    }));
    filters.push(serde_json::json!({
        "kind": "approval",
        "events": "unknown",
        "latest_turn": "latest_approval",
        "timeline": "/timeline --approval",
        "trace": "/trace --approval",
        "replay": "/replay --approval",
    }));
    filters
}

pub(in crate::chat::workbench::screen) fn screen_timeline_quick_jumps_json(
    navigator: Option<&WorkbenchCell>,
    failed: Option<&WorkbenchCell>,
    approval: Option<&WorkbenchCell>,
) -> serde_json::Value {
    let navigator_commands = navigator.map(selected_cell_actions).unwrap_or_default();
    serde_json::json!({
        "latest": {
            "turn": navigator
                .and_then(|cell| extract_token_after(&cell.detail, "latest="))
                .unwrap_or_else(|| "none".into()),
            "timeline": command_with_prefix(&navigator_commands, "/timeline ")
                .or_else(|| command_with_prefix(&navigator_commands, "/timeline")),
            "trace": command_with_prefix(&navigator_commands, "/trace "),
        },
        "failed": failed
            .map(screen_replay_point_json)
            .unwrap_or_else(|| serde_json::json!({
                "kind": "error",
                "title": "failed",
                "events": "0",
                "latest_turn": "none",
                "actions": {
                    "primary": "/timeline --failed",
                    "trace": "/trace --failed",
                    "replay": "/replay --failed",
                },
            })),
        "approval": approval
            .map(screen_replay_point_json)
            .unwrap_or_else(|| serde_json::json!({
                "kind": "approval",
                "title": "approval",
                "events": "0",
                "latest_turn": "none",
                "actions": {
                    "primary": "/timeline --approval",
                    "trace": "/trace --approval",
                    "replay": "/replay --approval",
                },
            })),
    })
}

pub(in crate::chat::workbench::screen) fn screen_replay_point_json(
    cell: &WorkbenchCell,
) -> serde_json::Value {
    serde_json::json!({
        "kind": cell.kind.as_str(),
        "title": terminal_inline(&cell.title),
        "events": extract_token_after(&cell.detail, "events=")
            .unwrap_or_else(|| "0".into()),
        "latest_turn": extract_token_after(&cell.detail, "latest_turn=")
            .unwrap_or_else(|| "none".into()),
        "actions": selected_cell_actions_json(Some(cell), &selected_cell_actions(cell)),
    })
}
