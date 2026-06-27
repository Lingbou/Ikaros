// SPDX-License-Identifier: GPL-3.0-only

use super::McpStatus;
use anyhow::Result;
use ikaros_core::{IkarosPaths, redact_secrets};
use ikaros_host::mcp_servers_report;

pub(super) fn status(args: McpStatus, paths: &IkarosPaths) -> Result<()> {
    let report = mcp_servers_report(paths)?;
    if args.json {
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }
    println!("mcp_servers: {}", report.servers.len());
    for server in &report.servers {
        println!(
            "- id={} enabled={} transport={} command={} args={} include_tools={} exclude_tools={} timeout_ms={} max_output_bytes={}",
            server.id,
            server.enabled,
            server.transport,
            server.command,
            server.args_count,
            format_tool_filter(&server.include_tools),
            format_tool_filter(&server.exclude_tools),
            server.timeout_ms,
            server.max_output_bytes,
        );
    }
    Ok(())
}

fn format_tool_filter(values: &[String]) -> String {
    if values.is_empty() {
        return "all".into();
    }
    values
        .iter()
        .map(|value| safe(value))
        .collect::<Vec<_>>()
        .join(",")
}

fn safe(input: &str) -> String {
    redact_secrets(input)
        .chars()
        .map(|ch| if ch.is_control() { '_' } else { ch })
        .collect()
}
