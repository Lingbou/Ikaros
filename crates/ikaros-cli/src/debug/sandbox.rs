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

    println!("* Sandbox");
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
    Ok(host_debug_sandbox_report(args.probe, paths, workspace, agent_override).await?)
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
