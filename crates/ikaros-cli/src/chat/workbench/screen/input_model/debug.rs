// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::chat::workbench::screen) fn screen_debug_model_json(
    screen: &WorkbenchScreen,
) -> serde_json::Value {
    let trace_panel = screen_timeline_panel_json(screen);
    let readiness = find_cell(screen, |cell| cell.title == "readiness");
    let sandbox = find_cell(screen, |cell| cell.title == "sandbox");
    let observability = find_cell(screen, |cell| cell.title == "observability");
    let state_db = find_cell(screen, |cell| cell.title == "state db");
    let progress = screen_surface_progress_json(screen);
    let error_kind = json_string(&progress, "error_kind", "none");
    let failed_turn = json_string(&trace_panel, "failed_turn", "none");
    serde_json::json!({
        "schema": "ikaros-workbench-debug-v1",
        "primary": if error_kind != "none" || failed_turn != "none" {
            "/trace --failed"
        } else {
            "/debug readiness"
        },
        "latest_error_kind": error_kind,
        "failed_turn": failed_turn,
        "surfaces": [
            debug_surface_json("readiness", "MVP readiness", readiness, "/debug readiness", vec!["/debug insights", "/debug dump"]),
            debug_surface_json("timeline", "Timeline", None, "/timeline", vec!["/trace", "/replay", "/timeline --failed"]),
            debug_surface_json("state_db", "State DB", state_db, "/debug state-db", vec!["/debug dump", "/debug logs"]),
            debug_surface_json("continuations", "Continuations", None, "/debug continuations", vec!["/queue run", "/cancel all"]),
            debug_surface_json("memory", "Memory lifecycle", None, "/debug memory-lifecycle", vec!["/memory", "/trace --kind memory"]),
            debug_surface_json("provider", "Provider diagnostics", None, "/provider debug", vec!["/provider health --live", "/provider matrix --live"]),
            debug_surface_json("sandbox", "Sandbox", sandbox, "/debug sandbox", vec!["/sandbox", "/sandbox --probe"]),
            debug_surface_json("observability", "Logs and insights", observability, "/debug insights", vec!["/debug logs", "/debug dump"]),
        ],
        "failure_actions": {
            "trace_failed": "/trace --failed",
            "timeline_failed": "/timeline --failed",
            "dump": "/debug dump",
            "logs": "/debug logs",
            "readiness": "/debug readiness",
        },
    })
}

pub(in crate::chat::workbench::screen) fn debug_surface_json(
    id: &str,
    label: &str,
    cell: Option<&WorkbenchCell>,
    primary_action: &str,
    secondary_actions: Vec<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "label": label,
        "available": cell.is_some() || primary_action.starts_with('/'),
        "summary": cell.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "primary_action": primary_action,
        "secondary_actions": secondary_actions,
        "intent": command_intent(primary_action),
        "risk": command_risk(primary_action),
    })
}

pub(in crate::chat::workbench::screen) fn screen_navigation_json() -> Vec<serde_json::Value> {
    [
        ("status", "status", "/screen --focus status"),
        ("timeline", "timeline", "/screen --focus timeline"),
        ("main", "main", "/screen --focus main"),
        ("side", "side", "/screen --focus side"),
        (
            "provider",
            "evidence",
            "/screen --focus main --select-action provider",
        ),
        (
            "context",
            "evidence",
            "/screen --focus main --select-action context",
        ),
        (
            "memory",
            "evidence",
            "/screen --focus main --select-action memory",
        ),
        (
            "rag",
            "evidence",
            "/screen --focus main --select-action rag",
        ),
        (
            "coding",
            "evidence",
            "/screen --focus main --select-action code",
        ),
        (
            "approval",
            "evidence",
            "/screen --focus side --select-action approval",
        ),
        (
            "queue",
            "evidence",
            "/screen --focus side --select-action queue",
        ),
        (
            "gateway",
            "evidence",
            "/screen --focus status --select-action gateway",
        ),
        ("failed", "replay", "/timeline --failed"),
        ("approval_trace", "replay", "/trace --approval"),
    ]
    .into_iter()
    .map(|(target, kind, command)| {
        serde_json::json!({
            "target": target,
            "kind": kind,
            "command": command,
            "intent": command_intent(command),
            "scope": command_scope(command),
            "risk": command_risk(command),
        })
    })
    .collect()
}
