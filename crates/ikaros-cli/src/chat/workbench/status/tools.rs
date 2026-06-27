// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths, ResolvedAgentProfile};
use ikaros_harness::{SkillDescriptor, SkillDescriptorKind, SkillRegistry, ToolVisibility};
use ikaros_runtime::{ChatRunOptions, agent_toolset_selection};

use super::super::{WorkbenchCell, WorkbenchCellKind, path_display, terminal_inline};

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
        "• RAG".to_owned(),
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

pub(super) fn screen_rag_cell(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    options: &ChatRunOptions,
) -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Context,
        title: "rag".into(),
        detail: format!(
            "backend={} embedding_provider={} embedding_model={} top_k={} dir={} command=/rag",
            terminal_inline(&config.rag.backend),
            terminal_inline(&config.rag.embedding_provider),
            terminal_inline(&config.rag.embedding_model),
            options.rag_top_k,
            path_display(&paths.rag_dir),
        ),
    }
}

fn rag_status_json_line(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    options: &ChatRunOptions,
) -> String {
    let payload = serde_json::json!({
        "schema": "ikaros-workbench-rag-status-v1",
        "version": 1,
        "backend": terminal_inline(&config.rag.backend),
        "embedding_provider": terminal_inline(&config.rag.embedding_provider),
        "embedding_model": terminal_inline(&config.rag.embedding_model),
        "rag_top_k": options.rag_top_k,
        "default_injection": options.rag_top_k > 0,
        "rag_dir": path_display(&paths.rag_dir),
        "actions": {
            "ingest": "rag ingest <path>",
            "search": "rag search <query>",
            "context": "/context",
        },
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        r#"{"schema":"ikaros-workbench-rag-status-v1","version":1,"error":"serialization_failed"}"#
            .to_owned()
    });
    format!("rag_status_json: {encoded}")
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
        "• Tools".to_owned(),
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
        lines.push(format!("  • {}", terminal_inline(line)));
    }
    if direct.len() > 5 {
        lines.push(format!("  • ... {} more direct tools", direct.len() - 5));
    }
    if !deferred.is_empty() {
        lines.push(format!("  deferred: {}", deferred.len()));
        for line in deferred.iter().take(3) {
            lines.push(format!("  • {}", terminal_inline(line)));
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
        "• MCP".to_owned(),
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
            "  • {}: {}, {}",
            terminal_inline(&server.id),
            enabled,
            terminal_inline(&server.transport)
        ));
    }
    if total > 5 {
        lines.push(format!("  • ... {} more servers", total - 5));
    }
    lines
}

pub(super) fn screen_mcp_cell(config: &IkarosConfig) -> WorkbenchCell {
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
    WorkbenchCell {
        kind: WorkbenchCellKind::Tool,
        title: "mcp".into(),
        detail: format!(
            "servers={} enabled={} stdio={} command=/mcp status mcp=/mcp status stdio=/mcp call-stdio http=/mcp call-http probe_policy=explicit_command_required",
            total, enabled, stdio
        ),
    }
}

pub(super) fn screen_browser_cell() -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Tool,
        title: "browser".into(),
        detail: "cdp=local command=/browser status browser=/browser status launch=/browser launch supervisor=/browser supervisor-status stop=/browser stop list=/browser list navigate=/browser navigate snapshot=/browser snapshot click=/browser click type=/browser type scroll=/browser scroll screenshot=/browser screenshot cdp=/browser cdp skills=browser_status,browser_list,browser_new_target,browser_navigate,browser_snapshot,browser_click,browser_type,browser_scroll,browser_screenshot discovery=tool_search plugin_toolset policy=network_egress_for_discovery direct_cdp_websocket_for_target_commands".into(),
    }
}

pub(super) fn screen_web_cell() -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Tool,
        title: "web".into(),
        detail: "search=governed providers=duckduckgo-html,brave,bing,serpapi,tavily extract=governed command=/web help web=/web help search=/web search extract=/web extract policy=network_egress approval=network".into(),
    }
}

pub(super) fn screen_vision_cell() -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Model,
        title: "vision".into(),
        detail: "multimodal=image skill=vision_describe command=/vision describe vision=/vision describe provider=active_model input=path|url|data-url".into(),
    }
}

pub(super) fn screen_image_cell() -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Model,
        title: "image".into(),
        detail: "multimodal=image_generation skill=image_generate command=/image generate image=/image generate generate=/image generate provider=openai_compatible_endpoint output=url|b64_json".into(),
    }
}

fn mcp_status_json(config: &IkarosConfig) -> String {
    serde_json::to_string(&serde_json::json!({
        "schema": "ikaros-workbench-mcp-status-v1",
        "version": 1,
        "servers_total": config.mcp.servers.len(),
        "servers_enabled": config.mcp.servers.iter().filter(|server| server.enabled).count(),
        "probe_policy": "explicit_command_required",
        "servers": config.mcp.servers.iter().map(|server| {
            serde_json::json!({
                "id": terminal_inline(&server.id),
                "enabled": server.enabled,
                "transport": terminal_inline(&server.transport),
                "command": terminal_inline(&server.command),
                "args_count": server.args.len(),
                "include_tools": server.include_tools.iter().map(|tool| terminal_inline(tool)).collect::<Vec<_>>(),
                "exclude_tools": server.exclude_tools.iter().map(|tool| terminal_inline(tool)).collect::<Vec<_>>(),
                "timeout_ms": server.timeout_ms,
                "max_output_bytes": server.max_output_bytes,
            })
        }).collect::<Vec<_>>(),
    }))
    .unwrap_or_else(|_| {
        r#"{"schema":"ikaros-workbench-mcp-status-v1","version":1,"error":"serialization_failed"}"#
            .into()
    })
}

fn tools_status_json_line(
    agent: &ResolvedAgentProfile,
    toolsets: &[&str],
    direct: &[serde_json::Value],
    deferred: &[serde_json::Value],
    disabled: &[serde_json::Value],
) -> String {
    let payload = serde_json::json!({
        "schema": "ikaros-workbench-tools-status-v1",
        "version": 1,
        "agent": terminal_inline(&agent.name),
        "toolsets": toolsets.iter().map(|toolset| terminal_inline(toolset)).collect::<Vec<_>>(),
        "counts": {
            "direct": direct.len(),
            "deferred": deferred.len(),
            "disabled": disabled.len(),
        },
        "groups": {
            "direct": direct,
            "deferred": deferred,
            "disabled": disabled,
        },
        "actions": {
            "commands": "/commands tool",
            "screen": "/screen --focus main",
        },
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        r#"{"schema":"ikaros-workbench-tools-status-v1","version":1,"error":"serialization_failed"}"#
            .to_owned()
    });
    format!("tools_status_json: {encoded}")
}

fn tool_descriptor_status_json(
    descriptor: &SkillDescriptor,
    visibility: Option<ToolVisibility>,
) -> serde_json::Value {
    let callable = matches!(
        visibility,
        Some(ToolVisibility::Direct | ToolVisibility::Deferred)
    ) && descriptor.kind == SkillDescriptorKind::ExecutableTool
        && !descriptor.disable_model_invocation;
    serde_json::json!({
        "name": terminal_inline(&descriptor.name),
        "kind": skill_descriptor_kind_name(&descriptor.kind),
        "callable": callable,
        "toolset": descriptor.toolset.as_str(),
        "risk": format!("{:?}", descriptor.risk_level),
        "mode": descriptor.execution_mode.as_str(),
        "provenance": descriptor.provenance.as_deref().map(terminal_inline),
        "support_files": descriptor.support_files.len(),
    })
}

fn tool_descriptor_status_line(
    descriptor: &SkillDescriptor,
    visibility: Option<ToolVisibility>,
) -> String {
    let callable = matches!(
        visibility,
        Some(ToolVisibility::Direct | ToolVisibility::Deferred)
    ) && descriptor.kind == SkillDescriptorKind::ExecutableTool
        && !descriptor.disable_model_invocation;
    format!(
        "{} kind={} callable={} toolset={} risk={:?} mode={} provenance={} support_files={}",
        descriptor.name,
        skill_descriptor_kind_name(&descriptor.kind),
        callable,
        descriptor.toolset,
        descriptor.risk_level,
        descriptor.execution_mode.as_str(),
        descriptor.provenance.as_deref().unwrap_or("-"),
        descriptor.support_files.len()
    )
}

fn tool_descriptor_short_line(
    descriptor: &SkillDescriptor,
    visibility: Option<ToolVisibility>,
) -> String {
    let callable = matches!(
        visibility,
        Some(ToolVisibility::Direct | ToolVisibility::Deferred)
    ) && descriptor.kind == SkillDescriptorKind::ExecutableTool
        && !descriptor.disable_model_invocation;
    format!(
        "{} ({}, callable={})",
        descriptor.name,
        skill_descriptor_kind_name(&descriptor.kind),
        callable
    )
}

fn skill_descriptor_kind_name(kind: &SkillDescriptorKind) -> &'static str {
    match kind {
        SkillDescriptorKind::ExecutableTool => "executable_tool",
        SkillDescriptorKind::PromptSkill => "prompt_skill",
    }
}

fn json_str<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(serde_json::Value::as_str)
}
