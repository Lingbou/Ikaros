// SPDX-License-Identifier: GPL-3.0-only

use ikaros_agent::chat::ChatRunOptions;
use ikaros_core::{IkarosConfig, IkarosPaths};

use super::super::super::{WorkbenchCell, WorkbenchCellKind, path_display, terminal_inline};

pub(in crate::chat::workbench::status) fn screen_rag_cell(
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

pub(in crate::chat::workbench::status) fn screen_mcp_cell(config: &IkarosConfig) -> WorkbenchCell {
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

pub(in crate::chat::workbench::status) fn screen_browser_cell() -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Tool,
        title: "browser".into(),
        detail: "cdp=local command=/browser status browser=/browser status launch=/browser launch supervisor=/browser supervisor-status stop=/browser stop list=/browser list navigate=/browser navigate snapshot=/browser snapshot click=/browser click type=/browser type scroll=/browser scroll screenshot=/browser screenshot cdp=/browser cdp skills=browser_status,browser_list,browser_new_target,browser_navigate,browser_snapshot,browser_click,browser_type,browser_scroll,browser_screenshot discovery=tool_search plugin_toolset policy=network_egress_for_discovery direct_cdp_websocket_for_target_commands".into(),
    }
}

pub(in crate::chat::workbench::status) fn screen_web_cell() -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Tool,
        title: "web".into(),
        detail: "search=governed providers=duckduckgo-html,brave,bing,serpapi,tavily extract=governed command=/web help web=/web help search=/web search extract=/web extract policy=network_egress approval=network".into(),
    }
}

pub(in crate::chat::workbench::status) fn screen_vision_cell() -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Model,
        title: "vision".into(),
        detail: "multimodal=image skill=vision_describe command=/vision describe vision=/vision describe provider=active_model input=path|url|data-url".into(),
    }
}

pub(in crate::chat::workbench::status) fn screen_image_cell() -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Model,
        title: "image".into(),
        detail: "multimodal=image_generation skill=image_generate command=/image generate image=/image generate generate=/image generate provider=openai_compatible_endpoint output=url|b64_json".into(),
    }
}
