// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::chat::workbench::screen) fn screen_provider_panel_json(
    screen: &WorkbenchScreen,
) -> serde_json::Value {
    let matrix = find_cell(screen, |cell| cell.title == "provider matrix");
    let cost = find_cell(screen, |cell| cell.title == "provider cost");
    let budget = find_cell(screen, |cell| cell.title == "model budget");
    let cache = find_cell(screen, |cell| cell.title == "provider cache policy");
    let health = find_cell(screen, |cell| cell.title == "provider health");
    let recovery = find_cell(screen, |cell| cell.title == "provider recovery");
    let fallback = find_cell(screen, |cell| cell.title == "provider fallback");
    let last_error_kind = recovery
        .and_then(|cell| extract_token_after(&cell.detail, "last_error_kind="))
        .or_else(|| health.and_then(|cell| extract_token_after(&cell.detail, "last_error_kind=")))
        .unwrap_or_else(|| "none".into());
    let cooldown_until = recovery
        .and_then(|cell| extract_token_after(&cell.detail, "cooldown_until="))
        .or_else(|| health.and_then(|cell| extract_token_after(&cell.detail, "cooldown_until=")))
        .unwrap_or_else(|| "none".into());
    let budget_status = budget
        .and_then(|cell| extract_token_after(&cell.detail, "budget_status="))
        .unwrap_or_else(|| "unknown".into());
    let health_status = health
        .and_then(|cell| extract_token_after(&cell.detail, "health_status="))
        .or_else(|| recovery.and_then(|cell| extract_token_after(&cell.detail, "status=")))
        .unwrap_or_else(|| "unknown".into());
    serde_json::json!({
        "provider": matrix
            .and_then(|cell| extract_token_after(&cell.detail, "provider="))
            .unwrap_or_else(|| "unknown".into()),
        "model": matrix
            .and_then(|cell| extract_token_after(&cell.detail, "model="))
            .unwrap_or_else(|| "unknown".into()),
        "profile": matrix
            .and_then(|cell| extract_token_after(&cell.detail, "profile="))
            .unwrap_or_else(|| "unknown".into()),
        "context_window": matrix
            .and_then(|cell| extract_token_after(&cell.detail, "context_window="))
            .unwrap_or_else(|| "unknown".into()),
        "tokenizer": matrix
            .and_then(|cell| extract_token_after(&cell.detail, "tokenizer="))
            .unwrap_or_else(|| "unknown".into()),
        "budget_status": budget_status.clone(),
        "daily_token_budget": budget
            .and_then(|cell| extract_token_after(&cell.detail, "daily_token_budget="))
            .unwrap_or_else(|| "unknown".into()),
        "used_today": budget
            .and_then(|cell| extract_token_after(&cell.detail, "used_today="))
            .unwrap_or_else(|| "unknown".into()),
        "remaining_today": budget
            .and_then(|cell| extract_token_after(&cell.detail, "remaining_today="))
            .unwrap_or_else(|| "unknown".into()),
        "currency": cost
            .and_then(|cell| extract_token_after(&cell.detail, "currency="))
            .unwrap_or_else(|| "unknown".into()),
        "estimated_cost_today": cost
            .and_then(|cell| extract_token_after(&cell.detail, "estimated_cost_today="))
            .unwrap_or_else(|| "unknown".into()),
        "cache_read_tokens_today": cost
            .and_then(|cell| extract_token_after(&cell.detail, "cache_read_tokens_today="))
            .unwrap_or_else(|| "0".into()),
        "cache_write_tokens_today": cost
            .and_then(|cell| extract_token_after(&cell.detail, "cache_write_tokens_today="))
            .unwrap_or_else(|| "0".into()),
        "cache_accounting": cost
            .and_then(|cell| extract_token_after(&cell.detail, "cache_accounting="))
            .unwrap_or_else(|| "unknown".into()),
        "prompt_cache_policy": cache
            .and_then(|cell| extract_token_after(&cell.detail, "prompt_cache_policy="))
            .unwrap_or_else(|| "unknown".into()),
        "retry_without_parameters": cache
            .and_then(|cell| extract_token_after(&cell.detail, "retry_without_parameters="))
            .unwrap_or_else(|| "unknown".into()),
        "health_status": health_status.clone(),
        "consecutive_failures": health
            .and_then(|cell| extract_token_after(&cell.detail, "consecutive_failures="))
            .unwrap_or_else(|| "0".into()),
        "last_error_kind": last_error_kind.clone(),
        "cooldown_until": cooldown_until.clone(),
        "needs_attention": provider_panel_needs_attention(
            &health_status,
            &budget_status,
            &last_error_kind,
            &cooldown_until,
        ),
        "budget": budget.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "cost": cost.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "cache_policy": cache.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "health": health.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "recovery": recovery.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "fallback": fallback.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "fallback_count": fallback
            .and_then(|cell| extract_token_after(&cell.detail, "fallback_count="))
            .unwrap_or_else(|| "0".into()),
        "fallback_rows": all_cells(screen)
            .filter(|cell| cell.title.starts_with("fallback "))
            .map(panel_cell_json)
            .collect::<Vec<_>>(),
        "recovery_actions": provider_recovery_actions_json(
            recovery.or(health).or(budget),
            &budget_status,
            &last_error_kind,
            &cooldown_until,
        ),
        "actions": matrix
            .map(|cell| selected_cell_actions_json(Some(cell), &selected_cell_actions(cell)))
            .unwrap_or_else(|| selected_cell_actions_json(None, &[])),
    })
}

pub(in crate::chat::workbench::screen) fn provider_panel_needs_attention(
    health_status: &str,
    budget_status: &str,
    last_error_kind: &str,
    cooldown_until: &str,
) -> bool {
    !matches!(health_status, "Healthy" | "Unknown" | "ok" | "unknown")
        || matches!(budget_status, "near_limit" | "exhausted")
        || last_error_kind != "none"
        || cooldown_until != "none"
}

pub(in crate::chat::workbench::screen) fn provider_recovery_actions_json(
    cell: Option<&WorkbenchCell>,
    budget_status: &str,
    last_error_kind: &str,
    cooldown_until: &str,
) -> serde_json::Value {
    let commands = cell.map(selected_cell_actions).unwrap_or_default();
    serde_json::json!({
        "primary": if budget_status == "exhausted" {
            Some("/budget disable".to_owned())
        } else if cooldown_until != "none" || last_error_kind != "none" {
            command_with_prefix(&commands, "/provider health --live")
                .or_else(|| Some("/provider health --live".to_owned()))
        } else {
            command_with_prefix(&commands, "/provider health")
                .or_else(|| Some("/provider health".to_owned()))
        },
        "live_probe": command_with_prefix(&commands, "/provider health --live")
            .or_else(|| Some("/provider health --live".to_owned())),
        "fallback_matrix": command_with_prefix(&commands, "/provider matrix")
            .or_else(|| Some("/provider matrix --live".to_owned())),
        "debug": command_with_prefix(&commands, "/provider debug")
            .or_else(|| Some("/provider debug".to_owned())),
        "trace": command_with_prefix(&commands, "/trace --kind model")
            .or_else(|| Some("/trace --kind model".to_owned())),
        "budget": command_with_prefix(&commands, "/budget")
            .or_else(|| Some("/budget".to_owned())),
        "disable_budget": (budget_status == "exhausted" || budget_status == "near_limit")
            .then_some("/budget disable"),
        "selected": selected_cell_actions_json(cell, &commands),
    })
}
