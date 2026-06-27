// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::debug) fn debug_insights(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let output = debug_insights_report(paths, workspace, agent_override)?;
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

pub(crate) fn debug_insights_json_line(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<String> {
    let output = debug_insights_report(paths, workspace, agent_override)?;
    Ok(format!(
        "insights_json: {}",
        serde_json::to_string(&redact_json(output))?
    ))
}

pub(in crate::debug) fn debug_insights_report(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Value> {
    let doctor = runtime_doctor_report(paths, workspace, agent_override)?;
    let config = IkarosConfig::load_shape_checked(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let state_report = SqliteSessionStore::new(&agent.state_dir).operational_report()?;
    let logs = collect_debug_logs(paths, DebugLogSource::All)?;
    let gateway = LocalGatewayStore::new(&paths.gateway_dir);
    let gateway_summary = debug_insights_gateway_summary(&gateway)?;
    let registry = ProviderRegistry;
    let health = ProviderHealthLedger::new(&paths.audit_dir);
    let usage = ModelUsageLedger::new(&paths.audit_dir);
    let provider_rows = provider_debug_matrix(&config, &agent, &registry, &health, &usage)?;
    let recent_start = logs.entries.len().saturating_sub(5);
    let recent_logs = logs.entries[recent_start..]
        .iter()
        .map(|entry| entry.value.clone())
        .collect::<Vec<_>>();
    let alerts = debug_insights_alerts(
        doctor.config.valid,
        state_report.integrity_check.ok,
        &provider_rows,
        &gateway_summary,
    );
    let status = if alerts.is_empty() { "ok" } else { "attention" };
    Ok(json!({
        "format": "ikaros-debug-insights-v1",
        "status": status,
        "home": paths.home.display().to_string(),
        "workspace": agent.workspace.display().to_string(),
        "agent": {
            "agent_id": agent.agent_id,
            "profile": agent.profile_name,
            "mode": doctor.agent.mode,
        },
        "config": {
            "schema_version": doctor.config.schema_version,
            "valid": doctor.config.valid,
            "issue_count": doctor.config.issues.len(),
            "issues": doctor.config.issues,
        },
        "state_db": {
            "path": state_report.path.display().to_string(),
            "schema_version": state_report.schema_version,
            "integrity_ok": state_report.integrity_check.ok,
            "journal_mode": state_report.journal_mode,
            "foreign_keys": state_report.foreign_keys,
            "write_policy": state_report.write_policy,
            "wal_checkpoint": state_report.wal_checkpoint,
            "search_indexes": state_report.search_indexes,
        },
        "logs": {
            "trace_schema": STRUCTURED_TRACE_SCHEMA,
            "audit_path": logs.audit_path.display().to_string(),
            "model_usage_path": logs.model_usage_path.display().to_string(),
            "trace_path": logs.trace_path.display().to_string(),
            "audit_count": logs.audit_count,
            "model_usage_count": logs.model_usage_count,
            "trace_count": logs.trace_count,
            "total_entries": logs.entries.len(),
            "total_model_tokens": logs.total_model_tokens,
            "cache_read_tokens": logs.cache_read_tokens,
            "cache_write_tokens": logs.cache_write_tokens,
            "recent": recent_logs,
        },
        "providers": {
            "health_log": health.path().display().to_string(),
            "rows": provider_rows,
        },
        "gateway": gateway_summary,
        "alerts": alerts,
    }))
}
pub(in crate::debug) fn debug_insights_gateway_summary(store: &LocalGatewayStore) -> Result<Value> {
    let messages = store.list()?;
    let deliveries = store.deliveries()?;
    let pairings = store.pairings()?;
    let pending = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Pending)
        .count();
    let processing = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Processing)
        .count();
    let failed = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Failed)
        .count();
    let cancelled = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Cancelled)
        .count();
    let dead_lettered = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::DeadLettered)
        .count();
    let delivery_pending = deliveries
        .iter()
        .filter(|delivery| delivery.status == GatewayDeliveryStatus::Pending)
        .count();
    let delivery_processing = deliveries
        .iter()
        .filter(|delivery| delivery.status == GatewayDeliveryStatus::Processing)
        .count();
    let delivery_dead_lettered = deliveries
        .iter()
        .filter(|delivery| delivery.status == GatewayDeliveryStatus::DeadLettered)
        .count();
    let pairing_pending = pairings
        .iter()
        .filter(|pairing| pairing.status == GatewayPairingStatus::Pending)
        .count();
    let pairing_paired = pairings
        .iter()
        .filter(|pairing| pairing.status == GatewayPairingStatus::Paired)
        .count();
    Ok(json!({
        "inbox_path": store.inbox_path().display().to_string(),
        "outbox_path": store.outbox_path().display().to_string(),
        "pairings_path": store.pairings_path().display().to_string(),
        "messages_total": messages.len(),
        "pending": pending,
        "processing": processing,
        "failed": failed,
        "cancelled": cancelled,
        "dead_lettered": dead_lettered,
        "deliveries_total": deliveries.len(),
        "delivery_pending": delivery_pending,
        "delivery_processing": delivery_processing,
        "delivery_dead_lettered": delivery_dead_lettered,
        "pairings_total": pairings.len(),
        "pairing_pending": pairing_pending,
        "pairing_paired": pairing_paired,
    }))
}
pub(in crate::debug) fn debug_insights_alerts(
    config_valid: bool,
    state_integrity_ok: bool,
    provider_rows: &[Value],
    gateway: &Value,
) -> Vec<Value> {
    let mut alerts = Vec::new();
    if !config_valid {
        alerts.push(json!({
            "kind": "config_invalid",
            "severity": "error",
            "summary": "configuration validation has errors or warnings",
        }));
    }
    if !state_integrity_ok {
        alerts.push(json!({
            "kind": "state_db_integrity",
            "severity": "error",
            "summary": "state.db integrity check is not ok",
        }));
    }
    for row in provider_rows {
        let live_smoke = row
            .get("live_smoke")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if !matches!(live_smoke, "ready" | "offline" | "local-ready") {
            alerts.push(json!({
                "kind": "provider_readiness",
                "severity": "warning",
                "provider_kind": row.get("kind").and_then(Value::as_str).unwrap_or("unknown"),
                "provider": row.get("provider").and_then(Value::as_str).unwrap_or("unknown"),
                "model": row.get("model").and_then(Value::as_str).unwrap_or("unknown"),
                "live_smoke": live_smoke,
                "debug_hint": row.get("debug_hint").and_then(Value::as_str).unwrap_or("inspect-provider"),
            }));
        }
    }
    let pending = gateway.get("pending").and_then(Value::as_u64).unwrap_or(0);
    let processing = gateway
        .get("processing")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let dead_lettered = gateway
        .get("dead_lettered")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let delivery_dead_lettered = gateway
        .get("delivery_dead_lettered")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if pending > 0 {
        alerts.push(json!({
            "kind": "gateway_pending",
            "severity": "info",
            "count": pending,
            "summary": "gateway messages are waiting for a worker",
        }));
    }
    if processing > 0 {
        alerts.push(json!({
            "kind": "gateway_processing",
            "severity": "info",
            "count": processing,
            "summary": "gateway messages have active leases",
        }));
    }
    if dead_lettered > 0 || delivery_dead_lettered > 0 {
        alerts.push(json!({
            "kind": "gateway_dead_lettered",
            "severity": "warning",
            "messages": dead_lettered,
            "deliveries": delivery_dead_lettered,
            "summary": "gateway has terminal failed work",
        }));
    }
    alerts
}
