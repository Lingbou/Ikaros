// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::debug) fn debug_dump(
    args: DebugDumpArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let output = debug_dump_report(&args, paths, workspace, agent_override)?;
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

pub(crate) fn debug_dump_json_line(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    recent_logs: usize,
) -> Result<String> {
    let args = DebugDumpArgs {
        output: None,
        recent_logs,
    };
    let output = debug_dump_report(&args, paths, workspace, agent_override)?;
    Ok(format!(
        "dump_json: {}",
        serde_json::to_string(&redact_json(output))?
    ))
}

pub(in crate::debug) fn debug_dump_report(
    args: &DebugDumpArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Value> {
    let config = IkarosConfig::load(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let state_db = SqliteSessionStore::new(&agent.state_dir);
    let state_report = state_db.operational_report()?;
    let logs = collect_debug_logs(paths, DebugLogSource::All)?;
    let recent_log_count = args.recent_logs.max(1);
    let start = logs.entries.len().saturating_sub(recent_log_count);
    let recent_logs = logs.entries[start..]
        .iter()
        .map(|entry| entry.value.clone())
        .collect::<Vec<_>>();
    let sandbox = configured_sandbox_debug_report(&config);
    let mut output = json!({
        "format": "ikaros-debug-dump-v1",
        "redacted": true,
        "created_at": time::OffsetDateTime::now_utc(),
        "home": paths.home.display().to_string(),
        "config": paths.config.display().to_string(),
        "agent": {
            "agent_id": agent.agent_id,
            "profile": agent.profile_name,
            "workspace": agent.workspace,
        },
        "state_db": state_report,
        "logs": {
            "trace_schema": STRUCTURED_TRACE_SCHEMA,
            "audit_path": logs.audit_path.display().to_string(),
            "model_usage_path": logs.model_usage_path.display().to_string(),
            "trace_path": logs.trace_path.display().to_string(),
            "counts": {
                "audit": logs.audit_count,
                "model_usage": logs.model_usage_count,
                "trace": logs.trace_count,
                "total": logs.entries.len(),
            },
            "recent_limit": recent_log_count,
            "recent": recent_logs,
        },
        "sandbox": {
            "current": sandbox,
            "isolation_matrix": sandbox_isolation_matrix(),
        },
        "mcp": mcp_debug_summary(&config),
        "memory": memory_debug_summary(&config, paths)?,
        "rag": rag_debug_summary(&config, paths),
        "export": null,
    });
    if let Some(path) = args.output.as_ref() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        output["export"] = json!({
            "created": true,
            "path": path.display().to_string(),
        });
        let artifact = redact_json(output.clone());
        fs::write(path, serde_json::to_vec_pretty(&artifact)?)?;
        output["export"]["created"] = json!(path.is_file());
    }
    Ok(output)
}
