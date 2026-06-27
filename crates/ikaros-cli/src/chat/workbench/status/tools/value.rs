// SPDX-License-Identifier: GPL-3.0-only

use ikaros_agent::chat::ChatRunOptions;
use ikaros_core::{IkarosConfig, IkarosPaths, ResolvedAgentProfile};
use ikaros_host::mcp_servers_report_from_config;

use super::super::super::{path_display, terminal_inline};

pub(super) fn rag_status_json_line(
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

pub(super) fn mcp_status_json(config: &IkarosConfig) -> String {
    let report = mcp_servers_report_from_config(config);
    let servers_enabled = report
        .servers
        .iter()
        .filter(|server| server.enabled)
        .count();
    serde_json::to_string(&serde_json::json!({
        "schema": "ikaros-workbench-mcp-status-v1",
        "version": 1,
        "servers_total": report.servers.len(),
        "servers_enabled": servers_enabled,
        "probe_policy": "explicit_command_required",
        "servers": report.servers,
    }))
    .unwrap_or_else(|_| {
        r#"{"schema":"ikaros-workbench-mcp-status-v1","version":1,"error":"serialization_failed"}"#
            .into()
    })
}

pub(super) fn tools_status_json_line(
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

pub(super) fn json_str<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(serde_json::Value::as_str)
}
