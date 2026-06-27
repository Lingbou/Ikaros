// SPDX-License-Identifier: GPL-3.0-only

use std::collections::BTreeSet;

use super::{
    super::{
        McpConfig, MemoryConfig, MemoryPolicyConfig, RagConfig, RemoteProviderConfig,
        VoiceProviderConfig,
    },
    ConfigValidationReport, is_remote_embedding_provider, normalize, validate_optional_url,
    validate_required, validate_required_url, validate_timeout, validate_url,
};

pub(super) fn validate_memory_config(config: &MemoryConfig, report: &mut ConfigValidationReport) {
    validate_memory_policy_config(&config.policy, report);
    let enabled = config
        .external_providers
        .iter()
        .filter(|provider| provider.enabled)
        .count();
    if enabled > 1 {
        report.error(
            "memory.external_providers",
            format!("only one external memory provider may be enabled, found {enabled}"),
        );
    }
    for (index, provider) in config.external_providers.iter().enumerate() {
        let path = format!("memory.external_providers[{index}]");
        if provider.id.trim().is_empty() {
            report.error(format!("{path}.id"), "must not be empty");
        }
        if provider.provider.trim() != "plugin" {
            report.error(format!("{path}.provider"), "only `plugin` is supported");
        }
        if provider.enabled {
            report.error(
                format!("{path}.enabled"),
                "external memory providers are descriptors only in the MVP and cannot be enabled",
            );
        }
        if let Some(endpoint) = &provider.endpoint {
            validate_url(format!("{path}.endpoint"), endpoint, report);
        }
    }
}

fn validate_memory_policy_config(config: &MemoryPolicyConfig, report: &mut ConfigValidationReport) {
    validate_threshold(
        "memory.policy.promote_threshold",
        config.promote_threshold,
        report,
    );
    validate_threshold(
        "memory.policy.demote_threshold",
        config.demote_threshold,
        report,
    );
    validate_threshold(
        "memory.policy.forget_threshold",
        config.forget_threshold,
        report,
    );
    if config.forget_threshold > config.demote_threshold {
        report.error(
            "memory.policy.forget_threshold",
            "must be less than or equal to demote_threshold",
        );
    }
    if config.demote_threshold > config.promote_threshold {
        report.error(
            "memory.policy.demote_threshold",
            "must be less than or equal to promote_threshold",
        );
    }
    if config.max_records_per_scope == 0 {
        report.error(
            "memory.policy.max_records_per_scope",
            "must be greater than zero",
        );
    }
}

fn validate_threshold(path: &str, value: f32, report: &mut ConfigValidationReport) {
    if !(0.0..=1.0).contains(&value) {
        report.error(path, "must be between 0.0 and 1.0");
    }
}

pub(super) fn validate_rag_config(
    config: &RagConfig,
    provider_settings: &RemoteProviderConfig,
    report: &mut ConfigValidationReport,
) {
    let provider = normalize(&config.embedding_provider);
    validate_timeout(
        "rag.embedding_timeout_ms",
        config.embedding_timeout_ms,
        report,
    );
    if provider == "ollama" {
        validate_required("rag.embedding_model", &config.embedding_model, report);
        validate_optional_url(
            "providers.embedding.base_url",
            &provider_settings.base_url,
            report,
        );
    } else if is_remote_embedding_provider(&provider) {
        validate_required("rag.embedding_model", &config.embedding_model, report);
        validate_required_url(
            "providers.embedding.base_url",
            &provider_settings.base_url,
            report,
        );
        validate_required(
            "providers.embedding.api_key",
            &provider_settings.api_key,
            report,
        );
    }
}

pub(super) fn validate_voice_config(
    path: &str,
    config: &VoiceProviderConfig,
    provider_settings: &RemoteProviderConfig,
    is_tts: bool,
    report: &mut ConfigValidationReport,
) {
    let provider = normalize(&config.provider);
    validate_timeout(format!("{path}.timeout_ms"), config.timeout_ms, report);
    if provider == "mock" {
        return;
    }
    validate_required(format!("{path}.model"), &config.model, report);
    let provider_path = if is_tts {
        "providers.tts"
    } else {
        "providers.asr"
    };
    validate_required_url(
        format!("{provider_path}.base_url"),
        &provider_settings.base_url,
        report,
    );
    validate_required(
        format!("{provider_path}.api_key"),
        &provider_settings.api_key,
        report,
    );
    if is_tts {
        if let Some(voice) = &config.voice {
            if voice.trim().is_empty() {
                report.error(format!("{path}.voice"), "must not be empty when set");
            }
        }
    } else if config.voice.is_some() {
        report.warning(format!("{path}.voice"), "ASR ignores the voice field");
    }
}

pub(super) fn validate_mcp_config(config: &McpConfig, report: &mut ConfigValidationReport) {
    let mut ids = BTreeSet::new();
    for (index, server) in config.servers.iter().enumerate() {
        let path = format!("mcp.servers[{index}]");
        let id = server.id.trim();
        if id.is_empty() {
            report.error(format!("{path}.id"), "must not be empty");
        } else if !ids.insert(id.to_owned()) {
            report.error(
                format!("{path}.id"),
                format!("duplicate MCP server id `{id}`"),
            );
        }
        if normalize(&server.transport) != "stdio" {
            report.error(format!("{path}.transport"), "only `stdio` is supported");
        }
        if server.command.trim().is_empty() {
            report.error(format!("{path}.command"), "must not be empty");
        }
        if contains_shell_expression(&server.command) {
            report.error(
                format!("{path}.command"),
                "must be a program name/path, not a shell expression",
            );
        }
        for (arg_index, arg) in server.args.iter().enumerate() {
            if arg.as_bytes().contains(&0) || arg.chars().any(char::is_control) {
                report.error(
                    format!("{path}.args[{arg_index}]"),
                    "must not contain control characters",
                );
            }
        }
        validate_timeout(format!("{path}.timeout_ms"), server.timeout_ms, report);
        if server.max_output_bytes == 0 {
            report.error(format!("{path}.max_output_bytes"), "must be greater than 0");
        }
        validate_mcp_tool_filter(
            &server.include_tools,
            format!("{path}.include_tools"),
            report,
        );
        validate_mcp_tool_filter(
            &server.exclude_tools,
            format!("{path}.exclude_tools"),
            report,
        );
        let include = server
            .include_tools
            .iter()
            .map(|tool| tool.trim())
            .collect::<BTreeSet<_>>();
        for tool in &server.exclude_tools {
            let tool = tool.trim();
            if !tool.is_empty() && include.contains(tool) {
                report.error(
                    format!("{path}.exclude_tools"),
                    format!("tool `{tool}` cannot be both included and excluded"),
                );
            }
        }
    }
}

fn validate_mcp_tool_filter(
    tools: &[String],
    path: impl Into<String>,
    report: &mut ConfigValidationReport,
) {
    let path = path.into();
    let mut seen = BTreeSet::new();
    for (index, tool) in tools.iter().enumerate() {
        let tool = tool.trim();
        if tool.is_empty() {
            report.error(format!("{path}[{index}]"), "must not be empty");
        } else if !seen.insert(tool.to_owned()) {
            report.error(
                format!("{path}[{index}]"),
                format!("duplicate tool `{tool}`"),
            );
        }
    }
}

fn contains_shell_expression(command: &str) -> bool {
    command
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '|' | '&' | ';' | '<' | '>'))
}
