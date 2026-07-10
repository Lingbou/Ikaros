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
