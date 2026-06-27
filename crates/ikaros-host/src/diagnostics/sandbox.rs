// SPDX-License-Identifier: GPL-3.0-only

use crate::{host_agent_context, provider_egress_allowed_hosts, runtime_execution_env};
use ikaros_core::{IkarosConfig, IkarosPaths, Result, redact_secrets};
use ikaros_execution::harness::{
    ProcessRequest, SandboxDebugReport, local_sandbox_debug_report, sandbox_isolation_matrix,
};
use serde_json::{Value, json};
use std::path::Path;

pub async fn debug_sandbox_report(
    probe: bool,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Value> {
    let host = host_agent_context(paths, workspace, agent_override)?;
    let config = &host.config;
    let agent = &host.agent_instance;
    let current = configured_sandbox_debug_report(config);
    let probe = if probe {
        sandbox_probe(config, &agent.workspace).await?
    } else {
        json!({
            "enabled": false,
            "next": "run `ikaros debug sandbox --probe` to execute a small command through the configured ExecutionEnv",
        })
    };
    Ok(json!({
        "format": "ikaros-sandbox-v1",
        "workspace": &agent.workspace,
        "agent_id": &agent.agent_id,
        "profile": &agent.profile_name,
        "current": current,
        "probe": probe,
        "isolation_matrix": sandbox_isolation_matrix(),
    }))
}

pub fn configured_sandbox_debug_report(config: &IkarosConfig) -> SandboxDebugReport {
    let mut current = local_sandbox_debug_report(
        config.execution.sandbox.backend.as_str(),
        config.execution.network.enabled,
        Some(&config.execution.sandbox.image),
    );
    current.allow_provider_hosts = config.execution.network.allow_provider_hosts;
    current.configured_allowed_host_count = config.execution.network.allowed_hosts.len();
    current.effective_allowed_host_count = if config.execution.network.enabled {
        provider_egress_allowed_hosts(config).len()
    } else {
        0
    };
    current.host_allowlist_mode = if !config.execution.network.enabled {
        "deny_by_default".into()
    } else if config.execution.network.allow_provider_hosts {
        "provider_hosts_plus_configured_hosts".into()
    } else {
        "configured_hosts_only".into()
    };
    current
}

pub async fn sandbox_probe(config: &IkarosConfig, workspace: &Path) -> Result<Value> {
    let env = runtime_execution_env(config, workspace)?;
    let started = std::time::Instant::now();
    let request = ProcessRequest::shell("printf ikaros-sandbox-probe", workspace)
        .with_timeout_ms(2_000)
        .with_max_output_bytes(4_096);
    let result = env.run_process(request).await;
    let elapsed_ms = started.elapsed().as_millis();
    match result {
        Ok(output) => Ok(json!({
            "enabled": true,
            "status": "ok",
            "sandbox_backend": config.execution.sandbox.backend,
            "sandbox_image_configured": !config.execution.sandbox.image.trim().is_empty(),
            "network_enabled": config.execution.network.enabled,
            "elapsed_ms": elapsed_ms,
            "process_status": output.status,
            "stdout": redact_secrets(output.stdout.trim()),
            "stderr": redact_secrets(output.stderr.trim()),
        })),
        Err(error) => Ok(json!({
            "enabled": true,
            "status": "failed",
            "sandbox_backend": config.execution.sandbox.backend,
            "sandbox_image_configured": !config.execution.sandbox.image.trim().is_empty(),
            "network_enabled": config.execution.network.enabled,
            "elapsed_ms": elapsed_ms,
            "error": redact_secrets(&error.to_string()),
        })),
    }
}
