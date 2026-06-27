// SPDX-License-Identifier: GPL-3.0-only

use crate::AgentConfig;

use super::{
    super::{ModelConfig, RemoteProviderConfig},
    ConfigValidationReport, model, normalize,
};

pub(super) fn validate_agent_config(
    config: &AgentConfig,
    default_model: &ModelConfig,
    default_model_provider: &RemoteProviderConfig,
    report: &mut ConfigValidationReport,
) {
    if config.default.trim().is_empty() {
        report.error("agent.default", "must not be empty");
    } else if !config.profiles.contains_key(&config.default) {
        report.error(
            "agent.default",
            format!("references unknown profile `{}`", config.default),
        );
    }
    if config.profiles.is_empty() {
        report.error("agent.profiles", "must define at least one profile");
    }
    for (name, profile) in &config.profiles {
        let path = format!("agent.profiles.{name}");
        if name.trim().is_empty() {
            report.error(&path, "profile name must not be empty");
        }
        validate_agent_toolsets(&profile.toolsets, format!("{path}.toolsets"), report);
    }
    for (id, instance) in &config.instances {
        let path = format!("agent.instances.{id}");
        if id.trim().is_empty() {
            report.error(&path, "agent instance id must not be empty");
        }
        let profile = if instance.profile.trim().is_empty() {
            &config.default
        } else {
            &instance.profile
        };
        if !config.profiles.contains_key(profile) {
            report.error(
                format!("{path}.profile"),
                format!("references unknown profile `{profile}`"),
            );
        }
        if let Some(toolsets) = &instance.toolsets {
            validate_agent_toolsets(toolsets, format!("{path}.toolsets"), report);
        }
        if instance.session_policy.max_parallel_subagents == 0 {
            report.error(
                format!("{path}.session_policy.max_parallel_subagents"),
                "must be greater than 0",
            );
        }
        if instance.session_policy.max_delegation_depth == 0 {
            report.error(
                format!("{path}.session_policy.max_delegation_depth"),
                "must be greater than 0",
            );
        }
        let provider = instance
            .providers
            .model
            .as_ref()
            .unwrap_or(default_model_provider);
        if let Some(model_config) = &instance.model {
            model::validate_model_config(
                &format!("{path}.model"),
                &format!("{path}.providers.model"),
                model_config,
                provider,
                report,
            );
        } else if instance.providers.model.is_some() {
            let provider_kind = normalize(&default_model.provider);
            model::validate_model_provider_settings(
                &provider_kind,
                &format!("{path}.providers.model"),
                provider,
                report,
            );
        }
        for (index, binding) in instance.route_bindings.iter().enumerate() {
            if binding.channel.trim().is_empty() {
                report.error(
                    format!("{path}.route_bindings[{index}].channel"),
                    "must not be empty",
                );
            }
        }
    }
}

fn validate_agent_toolsets(
    toolsets: &[String],
    path: impl Into<String>,
    report: &mut ConfigValidationReport,
) {
    let path = path.into();
    if toolsets.is_empty() {
        report.error(path, "must list at least one toolset");
        return;
    }
    let mut has_core = false;
    let mut has_deferred = false;
    for (index, toolset) in toolsets.iter().enumerate() {
        let normalized = toolset.trim();
        let valid = matches!(
            normalized,
            "core" | "workspace" | "memory" | "rag" | "coding" | "voice" | "plugin"
        );
        if !valid {
            report.error(
                format!("{path}[{index}]"),
                format!("unsupported toolset `{toolset}`"),
            );
        }
        has_core |= normalized == "core";
        has_deferred |= matches!(normalized, "rag" | "coding" | "voice" | "plugin");
    }
    if has_deferred && !has_core {
        report.error(
            path,
            "profiles that enable deferred toolsets must also enable `core` so tool_search, tool_describe, and tool_call are available",
        );
    }
}
