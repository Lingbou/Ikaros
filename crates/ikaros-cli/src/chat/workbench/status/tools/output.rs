// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_agent::agent_loop::agent_toolset_selection;
use ikaros_agent::chat::ChatRunOptions;
use ikaros_core::{IkarosConfig, IkarosPaths, ResolvedAgentProfile};
use ikaros_execution::harness::{SkillRegistry, ToolVisibility};

use super::super::super::{path_display, terminal_inline};
use super::registry::{
    tool_descriptor_short_line, tool_descriptor_status_json, tool_descriptor_status_line,
};
use super::value::{json_str, mcp_status_json, rag_status_json_line, tools_status_json_line};

pub(in crate::chat) fn print_rag_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    options: &ChatRunOptions,
) {
    println!("rag_backend: {}", terminal_inline(&config.rag.backend));
    println!(
        "rag_embedding_provider: {}",
        terminal_inline(&config.rag.embedding_provider)
    );
    println!(
        "rag_embedding_model: {}",
        terminal_inline(&config.rag.embedding_model)
    );
    println!("rag_top_k: {}", options.rag_top_k);
    println!("rag_dir: {}", path_display(&paths.rag_dir));
    println!("{}", rag_status_json_line(config, paths, options));
}

pub(in crate::chat) fn print_rag_status_for_human(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    options: &ChatRunOptions,
) {
    for line in rag_status_human_lines(config, paths, options) {
        println!("{line}");
    }
}

pub(in crate::chat) fn rag_status_human_lines(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    options: &ChatRunOptions,
) -> Vec<String> {
    vec![
        "* RAG".to_owned(),
        format!("  backend: {}", terminal_inline(&config.rag.backend)),
        format!(
            "  embedding: {} ({})",
            terminal_inline(&config.rag.embedding_model),
            terminal_inline(&config.rag.embedding_provider)
        ),
        format!("  default injection: {}", options.rag_top_k > 0),
        format!("  top_k: {}", options.rag_top_k),
        format!("  directory: {}", path_display(&paths.rag_dir)),
    ]
}

pub(in crate::chat) fn print_tools_status(
    registry: &SkillRegistry,
    agent: &ResolvedAgentProfile,
) -> Result<()> {
    let selection = agent_toolset_selection(agent)?;
    let mut direct = Vec::new();
    let mut deferred = Vec::new();
    let mut disabled = Vec::new();
    let mut direct_json = Vec::new();
    let mut deferred_json = Vec::new();
    let mut disabled_json = Vec::new();
    for descriptor in registry.descriptors() {
        let visibility = registry.visibility_for(&descriptor.name, &selection);
        let line = tool_descriptor_status_line(&descriptor, visibility);
        let item = tool_descriptor_status_json(&descriptor, visibility);
        match visibility {
            Some(ToolVisibility::Direct) => {
                direct.push(line);
                direct_json.push(item);
            }
            Some(ToolVisibility::Deferred) => {
                deferred.push(line);
                deferred_json.push(item);
            }
            Some(ToolVisibility::Disabled) => {
                disabled.push(line);
                disabled_json.push(item);
            }
            Some(ToolVisibility::Hidden) | None => {}
        }
    }
    direct.sort();
    deferred.sort();
    disabled.sort();
    direct_json.sort_by(|left, right| {
        json_str(left, "name")
            .unwrap_or("")
            .cmp(json_str(right, "name").unwrap_or(""))
    });
    deferred_json.sort_by(|left, right| {
        json_str(left, "name")
            .unwrap_or("")
            .cmp(json_str(right, "name").unwrap_or(""))
    });
    disabled_json.sort_by(|left, right| {
        json_str(left, "name")
            .unwrap_or("")
            .cmp(json_str(right, "name").unwrap_or(""))
    });
    println!("tools_agent: {}", terminal_inline(&agent.name));
    println!("tools_toolsets: {}", selection.names().join(","));
    println!("tools_direct: {}", direct.len());
    for line in direct {
        println!("- direct {}", terminal_inline(&line));
    }
    println!("tools_deferred: {}", deferred.len());
    for line in deferred {
        println!("- deferred {}", terminal_inline(&line));
    }
    println!("tools_disabled: {}", disabled.len());
    for line in disabled {
        println!("- disabled {}", terminal_inline(&line));
    }
    println!(
        "{}",
        tools_status_json_line(
            agent,
            &selection.names(),
            &direct_json,
            &deferred_json,
            &disabled_json
        )
    );
    Ok(())
}

pub(in crate::chat) fn print_tools_status_for_human(
    registry: &SkillRegistry,
    agent: &ResolvedAgentProfile,
) -> Result<()> {
    for line in tools_status_human_lines(registry, agent)? {
        println!("{line}");
    }
    Ok(())
}

pub(in crate::chat) fn tools_status_human_lines(
    registry: &SkillRegistry,
    agent: &ResolvedAgentProfile,
) -> Result<Vec<String>> {
    let selection = agent_toolset_selection(agent)?;
    let mut direct = Vec::new();
    let mut deferred = Vec::new();
    let mut disabled = Vec::new();
    for descriptor in registry.descriptors() {
        let visibility = registry.visibility_for(&descriptor.name, &selection);
        let line = tool_descriptor_short_line(&descriptor, visibility);
        match visibility {
            Some(ToolVisibility::Direct) => direct.push(line),
            Some(ToolVisibility::Deferred) => deferred.push(line),
            Some(ToolVisibility::Disabled) => disabled.push(line),
            Some(ToolVisibility::Hidden) | None => {}
        }
    }
    direct.sort();
    deferred.sort();
    disabled.sort();

    let mut lines = vec![
        "* Tools".to_owned(),
        format!("  agent: {}", terminal_inline(&agent.name)),
        format!("  toolsets: {}", selection.names().join(",")),
        format!(
            "  available: {} direct, {} deferred, {} disabled",
            direct.len(),
            deferred.len(),
            disabled.len()
        ),
    ];
    for line in direct.iter().take(5) {
        lines.push(format!("  * {}", terminal_inline(line)));
    }
    if direct.len() > 5 {
        lines.push(format!("  * ... {} more direct tools", direct.len() - 5));
    }
    if !deferred.is_empty() {
        lines.push(format!("  deferred: {}", deferred.len()));
        for line in deferred.iter().take(3) {
            lines.push(format!("  * {}", terminal_inline(line)));
        }
    }
    Ok(lines)
}

pub(in crate::chat) fn print_mcp_status(config: &IkarosConfig) {
    let total = config.mcp.servers.len();
    let enabled = config
        .mcp
        .servers
        .iter()
        .filter(|server| server.enabled)
        .count();
    let stdio = config
        .mcp
        .servers
        .iter()
        .filter(|server| server.transport.trim() == "stdio")
        .count();
    println!("mcp_servers: {total}");
    println!("mcp_enabled: {enabled}");
    println!("mcp_stdio: {stdio}");
    println!("mcp_probe_policy: explicit_command_required");
    println!("mcp_http_call: /mcp call-http <url> <tool> --arguments-json {{...}}");
    println!("mcp_status_json: {}", mcp_status_json(config));
    for server in &config.mcp.servers {
        println!(
            "- id={} enabled={} transport={} command={} args={} include_tools={} exclude_tools={} timeout_ms={} max_output_bytes={}",
            terminal_inline(&server.id),
            server.enabled,
            terminal_inline(&server.transport),
            terminal_inline(&server.command),
            server.args.len(),
            terminal_inline(&server.include_tools.join(",")),
            terminal_inline(&server.exclude_tools.join(",")),
            server.timeout_ms,
            server.max_output_bytes,
        );
    }
}

pub(in crate::chat) fn print_mcp_status_for_human(config: &IkarosConfig) {
    for line in mcp_status_human_lines(config) {
        println!("{line}");
    }
}

pub(in crate::chat) fn mcp_status_human_lines(config: &IkarosConfig) -> Vec<String> {
    let total = config.mcp.servers.len();
    let enabled = config
        .mcp
        .servers
        .iter()
        .filter(|server| server.enabled)
        .count();
    let stdio = config
        .mcp
        .servers
        .iter()
        .filter(|server| server.transport.trim() == "stdio")
        .count();
    let mut lines = vec![
        "* MCP".to_owned(),
        format!("  servers: {enabled}/{total} enabled"),
        format!("  stdio: {stdio}"),
        "  probe policy: explicit command required".to_owned(),
        "  call: /mcp call-stdio ... or /mcp call-http ...".to_owned(),
    ];
    for server in config.mcp.servers.iter().take(5) {
        let enabled = if server.enabled {
            "enabled"
        } else {
            "disabled"
        };
        lines.push(format!(
            "  * {}: {}, {}",
            terminal_inline(&server.id),
            enabled,
            terminal_inline(&server.transport)
        ));
    }
    if total > 5 {
        lines.push(format!("  * ... {} more servers", total - 5));
    }
    lines
}
