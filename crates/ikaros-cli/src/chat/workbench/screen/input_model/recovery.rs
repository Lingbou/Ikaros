// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::chat::workbench::screen) fn screen_recovery_model_json(
    screen: &WorkbenchScreen,
    turn_state: &serde_json::Value,
) -> serde_json::Value {
    let state = json_string(turn_state, "state", "idle");
    let blocking_reason = json_string(turn_state, "blocking_reason", "none");
    let approval_panel = screen_approval_panel_json(screen);
    let queue_panel = screen_queue_panel_json(screen);
    let provider_panel = screen_provider_panel_json(screen);
    let coding_loop = screen_coding_loop_model_json(screen);
    let debug_model = screen_debug_model_json(screen);
    let provider_needs_attention = dashboard_json_bool(&provider_panel, "needs_attention");
    let queue_needs_attention = dashboard_json_bool(&queue_panel, "needs_attention");
    let coding_needs_attention = dashboard_json_bool(&coding_loop, "needs_attention")
        || !coding_loop
            .get("latest_failure")
            .unwrap_or(&serde_json::Value::Null)
            .is_null();
    let approval_pending = dashboard_json_bool(&approval_panel, "needs_attention");

    let (status, primary, secondary) = match state.as_str() {
        "approval_pending" => (
            "blocked",
            recovery_action_json(
                "approve_selected",
                "Review approval",
                "/approval",
                "approval_required",
                false,
            ),
            vec![
                recovery_action_json(
                    "approve_inline",
                    "Approve selected",
                    "/screen approve-selected",
                    "approval_decision",
                    true,
                ),
                recovery_action_json(
                    "deny_inline",
                    "Deny selected",
                    "/screen deny-selected",
                    "approval_decision",
                    true,
                ),
                recovery_action_json(
                    "trace_approval",
                    "Trace approval",
                    "/trace --approval",
                    "inspect",
                    false,
                ),
            ],
        ),
        "provider_limited" => (
            "recoverable",
            recovery_action_json(
                "inspect_budget",
                "Inspect model budget",
                "/budget",
                "provider_limited",
                false,
            ),
            vec![
                recovery_action_json(
                    "provider_health",
                    "Check provider health",
                    "/provider health --live",
                    "provider_limited",
                    false,
                ),
                recovery_action_json(
                    "provider_matrix",
                    "Check provider matrix",
                    "/provider matrix --live",
                    "provider_limited",
                    false,
                ),
                recovery_action_json(
                    "trace_failed",
                    "Trace failed turn",
                    "/trace --failed",
                    "inspect",
                    false,
                ),
            ],
        ),
        "failed" if coding_needs_attention => (
            "recoverable",
            recovery_action_json(
                "review_coding_failure",
                "Review coding failure",
                "/code review",
                "coding_failure",
                false,
            ),
            vec![
                recovery_action_json(
                    "run_tests",
                    "Run tests",
                    "/code test",
                    "coding_failure",
                    true,
                ),
                recovery_action_json(
                    "rollback",
                    "Rollback coding turn",
                    "/code rollback",
                    "coding_failure",
                    true,
                ),
                recovery_action_json(
                    "trace_failed",
                    "Trace failed turn",
                    "/trace --failed",
                    "inspect",
                    false,
                ),
            ],
        ),
        "failed" => (
            "recoverable",
            recovery_action_json(
                "trace_failed",
                "Trace failed turn",
                "/trace --failed",
                "failure",
                false,
            ),
            vec![
                recovery_action_json(
                    "timeline_failed",
                    "Open failed timeline",
                    "/timeline --failed",
                    "failure",
                    false,
                ),
                recovery_action_json(
                    "debug_dump",
                    "Collect debug dump",
                    "/debug dump",
                    "inspect",
                    false,
                ),
            ],
        ),
        "queued" => (
            "ready",
            recovery_action_json(
                "run_queue",
                "Run queued continuation",
                "/queue run",
                "queued_work",
                false,
            ),
            vec![
                recovery_action_json(
                    "debug_continuations",
                    "Inspect continuations",
                    "/debug continuations",
                    "queued_work",
                    false,
                ),
                recovery_action_json(
                    "cancel_all",
                    "Cancel queued work",
                    "/cancel all",
                    "queued_work",
                    true,
                ),
            ],
        ),
        "running" => (
            "running",
            recovery_action_json(
                "cancel_all",
                "Cancel active work",
                "/cancel all",
                "running_work",
                true,
            ),
            vec![
                recovery_action_json("trace", "Trace active turn", "/trace", "inspect", false),
                recovery_action_json("timeline", "Open timeline", "/timeline", "inspect", false),
            ],
        ),
        "composing" => (
            "ready",
            recovery_action_json("submit", "Submit input", "enter", "composer", false),
            vec![recovery_action_json(
                "cancel_input",
                "Cancel input",
                "esc",
                "composer",
                false,
            )],
        ),
        _ => (
            "idle",
            recovery_action_json("none", "No recovery needed", "none", "idle", false),
            vec![
                recovery_action_json(
                    "readiness",
                    "Open readiness",
                    "/debug readiness",
                    "inspect",
                    false,
                ),
                recovery_action_json("timeline", "Open timeline", "/timeline", "inspect", false),
            ],
        ),
    };

    serde_json::json!({
        "schema": "ikaros-workbench-recovery-v1",
        "status": status,
        "turn_state": state,
        "blocking_reason": blocking_reason,
        "needs_attention": approval_pending
            || provider_needs_attention
            || queue_needs_attention
            || coding_needs_attention
            || matches!(status, "blocked" | "recoverable"),
        "primary": primary,
        "secondary": secondary,
        "evidence": {
            "approval": approval_panel,
            "provider": provider_panel,
            "queue": queue_panel,
            "coding": coding_loop,
            "debug": debug_model,
        },
    })
}

pub(in crate::chat::workbench::screen) fn recovery_action_json(
    id: &str,
    label: &str,
    command: &str,
    reason: &str,
    destructive_or_sensitive: bool,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "label": label,
        "command": command,
        "reason": reason,
        "intent": command_intent(command),
        "scope": command_scope(command),
        "risk": command_risk(command),
        "requires_explicit_action": destructive_or_sensitive
            || command_requires_explicit_action(command),
    })
}
