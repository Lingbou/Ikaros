// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(crate) fn screen_readiness_model_json(screen: &WorkbenchScreen) -> serde_json::Value {
    let readiness = find_cell(screen, |cell| cell.title == "readiness");
    let renderer = find_cell(screen, |cell| cell.title == "renderer");
    let commands = find_cell(screen, |cell| cell.title == "commands");
    let bottom = find_cell(screen, |cell| cell.title == "bottom pane");
    let state_db = find_cell(screen, |cell| cell.title == "state db");
    let observability = find_cell(screen, |cell| cell.title == "observability");
    let sandbox = find_cell(screen, |cell| cell.title == "sandbox");
    let gateway = find_cell(screen, |cell| cell.title == "gateway");
    let provider = screen_provider_panel_json(screen);
    let context = screen_context_panel_json(screen);
    let memory = screen_memory_panel_json(screen);
    let rag = screen_rag_panel_json(screen);
    let coding = screen_coding_panel_json(screen);
    let approval = screen_approval_panel_json(screen);
    let queue = screen_queue_panel_json(screen);

    let gates = vec![
        readiness_gate_json(
            "terminal_workbench",
            "Terminal workbench",
            readiness_status(
                renderer.is_some() && bottom.is_some() && commands.is_some(),
                false,
            ),
            "fullscreen screen model, retained composer, command registry, and markdown renderer",
            "/screen --fullscreen",
            vec!["/screen --palette", "/screen --rich"],
        ),
        readiness_gate_json(
            "approval",
            "Inline approval",
            readiness_status(true, dashboard_json_bool(&approval, "needs_attention")),
            "approval panel, overlay decisions, and continue-after-approval actions",
            "/approval",
            vec![
                "/screen approve-selected",
                "/screen deny-selected",
                "/trace --approval",
            ],
        ),
        readiness_gate_json(
            "timeline",
            "Timeline and replay",
            readiness_status(state_db.is_some(), false),
            "state.db backed timeline, trace, replay, and selected turn navigation",
            "/timeline",
            vec!["/trace", "/replay", "/debug state-db"],
        ),
        readiness_gate_json(
            "provider",
            "Provider visibility",
            readiness_status(
                provider.get("provider").is_some(),
                dashboard_json_bool(&provider, "needs_attention"),
            ),
            "provider profile, budget, health, fallback, cost, and recovery actions",
            "/provider",
            vec![
                "/provider health --live",
                "/provider matrix --live",
                "/budget",
            ],
        ),
        readiness_gate_json(
            "context_memory_rag",
            "Context, memory, and RAG",
            readiness_status(
                context.get("budget_status").is_some()
                    && memory.get("lifecycle").is_some()
                    && rag.get("actions_model").is_some(),
                dashboard_json_bool(&context, "needs_attention")
                    || dashboard_json_bool(&memory, "needs_attention")
                    || dashboard_json_bool(&rag, "needs_attention"),
            ),
            "context budget/sections/references, memory lifecycle, and RAG injection state",
            "/context",
            vec!["/memory", "/rag", "/debug memory-lifecycle"],
        ),
        readiness_gate_json(
            "coding",
            "Coding workflow",
            readiness_status(
                coding.get("workflow_action").is_some(),
                dashboard_json_bool(&coding, "needs_attention"),
            ),
            "plan/apply/test/review/rollback loop with coding timeline evidence",
            "/code workflow --model-loop",
            vec!["/code plan", "/code test", "/code review", "/code rollback"],
        ),
        readiness_gate_json(
            "queue_interrupt",
            "Queue and interrupt",
            readiness_status(
                queue.get("status_counts").is_some(),
                dashboard_json_bool(&queue, "needs_attention"),
            ),
            "continuation queue, pending input, attachments, cancel, retry, and recovery controls",
            "/debug continuations",
            vec!["/queue run", "/cancel all", "/queue retry <id>"],
        ),
        readiness_gate_json(
            "sandbox_debug",
            "Sandbox and debug",
            readiness_status(sandbox.is_some() && observability.is_some(), false),
            "sandbox, state db, logs, insights, dump, readiness, and trace debug surfaces",
            "/debug readiness",
            vec![
                "/debug sandbox",
                "/debug insights",
                "/debug logs",
                "/debug dump",
            ],
        ),
        readiness_gate_json(
            "gateway",
            "Gateway evidence",
            readiness_status(gateway.is_some(), false),
            "gateway queue evidence without a separate agent loop",
            "/gateway",
            vec!["/gateway daemon status", "/gateway adapter status"],
        ),
    ];
    let attention_count = gates
        .iter()
        .filter(|gate| {
            gate.get("status")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|status| status == "attention")
        })
        .count();
    let incomplete_count = gates
        .iter()
        .filter(|gate| {
            gate.get("status")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|status| status == "missing")
        })
        .count();

    serde_json::json!({
        "schema": "ikaros-mvp-readiness-v1",
        "status": if incomplete_count > 0 {
            "missing"
        } else if attention_count > 0 {
            "attention"
        } else {
            "ready"
        },
        "attention_count": attention_count,
        "incomplete_count": incomplete_count,
        "readiness_cell": readiness.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "gates": gates,
        "actions": {
            "readiness": "/debug readiness",
            "doctor": "ikaros doctor",
            "screen": "/screen --fullscreen",
            "trace": "/trace",
            "timeline": "/timeline",
            "debug_dump": "/debug dump",
        },
    })
}

pub(crate) fn readiness_status(available: bool, attention: bool) -> &'static str {
    if !available {
        "missing"
    } else if attention {
        "attention"
    } else {
        "ready"
    }
}

pub(crate) fn readiness_gate_json(
    id: &str,
    label: &str,
    status: &str,
    evidence: &str,
    primary_action: &str,
    secondary_actions: Vec<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "label": label,
        "status": status,
        "evidence": evidence,
        "primary_action": primary_action,
        "secondary_actions": secondary_actions,
        "intent": command_intent(primary_action),
        "risk": command_risk(primary_action),
        "requires_explicit_action": command_requires_explicit_action(primary_action),
    })
}
