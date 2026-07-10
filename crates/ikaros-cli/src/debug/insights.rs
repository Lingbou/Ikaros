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
    let host = host_agent_context_shape_checked(paths, workspace, agent_override)?;
    let config = &host.config;
    let agent = &host.agent_instance;
    let state_report = SqliteSessionStore::new(&agent.state_dir).operational_report()?;
    let logs = collect_debug_logs(paths, DebugLogSource::All)?;
    let provider_matrix = provider_debug_matrix_report(config, agent, &paths.audit_dir)?;
    let provider_rows = provider_matrix.rows;
    let recent_start = logs.entries.len().saturating_sub(5);
    let recent_logs = logs.entries[recent_start..]
        .iter()
        .map(|entry| entry.value.clone())
        .collect::<Vec<_>>();
    let alerts = debug_insights_alerts(
        doctor.config.valid,
        state_report.integrity_check.ok,
        &provider_rows,
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
            "health_log": provider_matrix.health_log.display().to_string(),
            "rows": provider_rows,
        },
        "alerts": alerts,
    }))
}

pub(in crate::debug) fn debug_insights_alerts(
    config_valid: bool,
    state_integrity_ok: bool,
    provider_rows: &[Value],
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
    alerts
}
