// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::chat::workbench::screen) fn screen_coding_panel_json(
    screen: &WorkbenchScreen,
) -> serde_json::Value {
    let workflow = find_cell(screen, |cell| cell.title == "coding workflow");
    let progress = latest_coding_group_cell(screen, "progress");
    let diff = latest_coding_group_cell(screen, "diff");
    let test = latest_coding_group_cell(screen, "test");
    let review = latest_coding_group_cell(screen, "review");
    let latest_failure = latest_coding_failure_cell(screen);
    let recovery = latest_failure
        .or(diff)
        .or(test)
        .or(review)
        .or(progress)
        .or(workflow);
    let status = workflow
        .and_then(|cell| extract_token_after(&cell.detail, "status="))
        .unwrap_or_else(|| "idle".into());
    let diff_count = coding_phase_count(workflow, "diff=");
    let test_count = coding_phase_count(workflow, "test=");
    let review_count = coding_phase_count(workflow, "review=");
    let progress_count = coding_phase_count(workflow, "progress=");
    let has_diff = diff.is_some() || diff_count != "0";
    let has_test_evidence = test.is_some() || test_count != "0";
    let has_review = review.is_some() || review_count != "0";
    let needs_attention = status == "attention" || latest_failure.is_some();
    serde_json::json!({
        "workflow": workflow.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "status": status,
        "events": workflow
            .and_then(|cell| extract_token_after(&cell.detail, "events="))
            .unwrap_or_else(|| "0".into()),
        "turns": workflow
            .and_then(|cell| extract_token_after(&cell.detail, "turns="))
            .unwrap_or_else(|| "0".into()),
        "latest_turn": workflow
            .and_then(|cell| extract_token_after(&cell.detail, "latest_turn="))
            .unwrap_or_else(|| "none".into()),
        "phase_counts": {
            "progress": progress_count,
            "diff": diff_count,
            "test": test_count,
            "review": review_count,
        },
        "latest_progress": progress.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "latest_diff": diff.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "latest_test": test.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "latest_review": review.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "latest_failure": latest_failure
            .map(panel_cell_json)
            .unwrap_or(serde_json::Value::Null),
        "has_diff": has_diff,
        "has_test_evidence": has_test_evidence,
        "has_review": has_review,
        "needs_attention": needs_attention,
        "can_rollback": has_diff || has_test_evidence || has_review || latest_failure.is_some(),
        "plan_action": "/code plan",
        "workflow_action": "/code workflow --model-loop",
        "apply_action": "/code apply",
        "test_action": "/code test",
        "review_action": "/code review",
        "rollback_action": "/code rollback",
        "diff_action": "/diff",
        "trace_action": "/trace",
        "timeline_action": "/timeline",
        "groups": screen_coding_groups_json(screen),
        "recovery": recovery
            .map(|cell| selected_cell_actions_json(Some(cell), &selected_cell_actions(cell)))
            .unwrap_or_else(|| selected_cell_actions_json(None, &[])),
        "actions": workflow
            .map(|cell| selected_cell_actions_json(Some(cell), &selected_cell_actions(cell)))
            .unwrap_or_else(|| selected_cell_actions_json(None, &[])),
    })
}

pub(in crate::chat::workbench::screen) fn screen_coding_loop_model_json(
    screen: &WorkbenchScreen,
) -> serde_json::Value {
    let panel = screen_coding_panel_json(screen);
    let status = json_string(&panel, "status", "idle");
    let has_diff = dashboard_json_bool(&panel, "has_diff");
    let has_test_evidence = dashboard_json_bool(&panel, "has_test_evidence");
    let has_review = dashboard_json_bool(&panel, "has_review");
    let needs_attention = dashboard_json_bool(&panel, "needs_attention");
    let can_rollback = dashboard_json_bool(&panel, "can_rollback");
    let latest_failure = panel
        .get("latest_failure")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let next_action = if needs_attention {
        "/code review"
    } else if !has_diff {
        "/code plan"
    } else if !has_test_evidence {
        "/code test"
    } else if !has_review {
        "/code review"
    } else {
        "/code workflow --model-loop"
    };

    serde_json::json!({
        "schema": "ikaros-coding-loop-v1",
        "status": status,
        "needs_attention": needs_attention,
        "can_rollback": can_rollback,
        "next_action": next_action,
        "requires_explicit_confirmation": command_requires_explicit_action(next_action),
        "latest_turn": panel
            .get("latest_turn")
            .cloned()
            .unwrap_or_else(|| serde_json::json!("none")),
        "latest_failure": latest_failure,
        "steps": [
            coding_loop_step_json(
                "plan",
                "Plan",
                "/code plan",
                true,
                has_diff || has_test_evidence || has_review,
                false,
                "Analyze scope, risks, and test plan",
            ),
            coding_loop_step_json(
                "apply",
                "Apply",
                "/code apply",
                true,
                has_diff,
                !has_diff,
                "Apply a guarded patch after explicit confirmation",
            ),
            coding_loop_step_json(
                "approval",
                "Approval",
                "/screen approve-selected",
                screen_modal_cell(screen).is_some(),
                !screen_modal_cell(screen).is_some() && has_diff,
                screen_modal_cell(screen).is_some(),
                "Approve or deny the current write/shell request inline",
            ),
            coding_loop_step_json(
                "test",
                "Test",
                "/code test",
                has_diff,
                has_test_evidence,
                has_diff && !has_test_evidence,
                "Run allowlisted tests and capture evidence",
            ),
            coding_loop_step_json(
                "review",
                "Review",
                "/code review",
                has_diff || has_test_evidence,
                has_review,
                needs_attention,
                "Review diff, failures, and safety findings",
            ),
            coding_loop_step_json(
                "rollback",
                "Rollback",
                "/code rollback",
                can_rollback,
                false,
                needs_attention && can_rollback,
                "Rollback the current coding turn when needed",
            ),
        ],
        "actions": {
            "plan": "/code plan",
            "workflow": "/code workflow --model-loop",
            "apply": "/code apply",
            "test": "/code test",
            "review": "/code review",
            "rollback": "/code rollback",
            "trace": "/trace --kind coding",
            "timeline": "/timeline --kind coding",
        },
        "approval": {
            "visible": screen_modal_cell(screen).is_some(),
            "approve": "/screen approve-selected",
            "deny": "/screen deny-selected",
            "inspect": "/approval",
        },
    })
}

pub(in crate::chat::workbench::screen) fn coding_loop_step_json(
    id: &str,
    label: &str,
    command: &str,
    available: bool,
    complete: bool,
    attention: bool,
    description: &str,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "label": label,
        "command": command,
        "available": available,
        "complete": complete,
        "attention": attention,
        "description": description,
        "intent": command_intent(command),
        "risk": command_risk(command),
        "requires_explicit_action": command_requires_explicit_action(command),
    })
}

pub(in crate::chat::workbench::screen) fn latest_coding_failure_cell(
    screen: &WorkbenchScreen,
) -> Option<&WorkbenchCell> {
    screen
        .main
        .iter()
        .rev()
        .chain(screen.timeline.iter().rev())
        .chain(screen.side.iter().rev())
        .chain(screen.status.iter().rev())
        .find(|cell| {
            matches!(
                cell.kind,
                WorkbenchCellKind::Coding | WorkbenchCellKind::Error
            ) && (cell.detail.contains("kind=patch_failed")
                || cell.detail.contains("status=failed")
                || cell.detail.contains("status=attention"))
        })
}

pub(in crate::chat::workbench::screen) fn coding_phase_count(
    workflow: Option<&WorkbenchCell>,
    key: &str,
) -> String {
    workflow
        .and_then(|cell| extract_token_after(&cell.detail, key))
        .unwrap_or_else(|| "0".into())
}
