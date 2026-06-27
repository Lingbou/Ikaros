// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::debug) async fn debug_sandbox(
    args: DebugSandboxArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let output = debug_sandbox_report(args, paths, workspace, agent_override).await?;
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

pub(crate) async fn debug_sandbox_json_line(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    probe: bool,
) -> Result<String> {
    let output =
        debug_sandbox_report(DebugSandboxArgs { probe }, paths, workspace, agent_override).await?;
    Ok(format!(
        "sandbox_json: {}",
        serde_json::to_string(&redact_json(output))?
    ))
}

pub(crate) async fn print_sandbox_status_for_human(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    probe: bool,
) -> Result<()> {
    let output =
        debug_sandbox_report(DebugSandboxArgs { probe }, paths, workspace, agent_override).await?;
    let output = redact_json(output);
    let current = output.get("current").unwrap_or(&Value::Null);
    let probe_report = output.get("probe").unwrap_or(&Value::Null);

    println!("• Sandbox");
    println!(
        "  workspace: {}",
        output
            .get("workspace")
            .and_then(Value::as_str)
            .map(redact_secrets)
            .unwrap_or_else(|| "unknown".into())
    );
    println!(
        "  backend: {} ({})",
        value_str(current, "backend").unwrap_or("unknown"),
        value_str(current, "level").unwrap_or("unknown")
    );
    println!(
        "  files: {}",
        value_str(current, "file_write_scope").unwrap_or("unknown")
    );
    println!(
        "  network: {} / {}",
        value_str(current, "network_egress").unwrap_or("unknown"),
        value_str(current, "host_allowlist_mode").unwrap_or("unknown")
    );
    println!(
        "  guards: dns_rebind={} restricted_ip={}",
        value_bool(current, "dns_rebind_block").unwrap_or(false),
        value_bool(current, "restricted_ip_literal_block").unwrap_or(false)
    );

    if probe {
        let status = value_str(probe_report, "status").unwrap_or("unknown");
        let elapsed_ms = probe_report
            .get("elapsed_ms")
            .and_then(Value::as_u64)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".into());
        println!("  probe: {status} in {elapsed_ms}ms");
        if let Some(error) = value_str(probe_report, "error") {
            println!("  error: {}", terminal_safe(error));
        }
    } else {
        println!("  probe: not run (`/sandbox --probe`)");
    }
    Ok(())
}

pub(in crate::debug) async fn debug_sandbox_report(
    args: DebugSandboxArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Value> {
    let config = IkarosConfig::load(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let current = configured_sandbox_debug_report(&config);
    let probe = if args.probe {
        sandbox_probe(&config, &agent.workspace).await?
    } else {
        json!({
            "enabled": false,
            "next": "run `ikaros debug sandbox --probe` to execute a small command through the configured ExecutionEnv",
        })
    };
    let output = json!({
        "format": "ikaros-sandbox-v1",
        "workspace": agent.workspace,
        "agent_id": agent.agent_id,
        "profile": agent.profile_name,
        "current": current,
        "probe": probe,
        "isolation_matrix": sandbox_isolation_matrix(),
    });
    Ok(output)
}

fn value_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn value_bool(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

fn terminal_safe(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

pub(in crate::debug) fn configured_sandbox_debug_report(
    config: &IkarosConfig,
) -> SandboxDebugReport {
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

pub(in crate::debug) async fn sandbox_probe(
    config: &IkarosConfig,
    workspace: &Path,
) -> Result<Value> {
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
