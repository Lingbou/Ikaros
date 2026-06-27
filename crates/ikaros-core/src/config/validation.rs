// SPDX-License-Identifier: GPL-3.0-only

use crate::{IkarosError, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, path::Path};

use super::{CURRENT_CONFIG_SCHEMA_VERSION, IkarosConfig};

mod agent;
mod model;
mod resources;
mod safety;

pub(crate) fn load_yaml_shape_checked(raw: &str) -> Result<IkarosConfig> {
    let value: yaml_serde::Value = yaml_serde::from_str(raw)?;
    let mut report = ConfigValidationReport::default();
    validate_yaml_shape(&value, &mut report);
    if !report.is_valid() {
        return Err(IkarosError::Message(format_validation_failure(
            "configuration shape validation failed",
            &report,
        )));
    }
    Ok(yaml_serde::from_str(raw)?)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigValidationReport {
    pub errors: Vec<ConfigValidationIssue>,
    pub warnings: Vec<ConfigValidationIssue>,
}

impl ConfigValidationReport {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    fn error(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.errors.push(ConfigValidationIssue {
            path: path.into(),
            message: message.into(),
        });
    }

    fn warning(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.warnings.push(ConfigValidationIssue {
            path: path.into(),
            message: message.into(),
        });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigValidationIssue {
    pub path: String,
    pub message: String,
}

pub(crate) fn format_validation_failure(summary: &str, report: &ConfigValidationReport) -> String {
    let mut message = summary.to_owned();
    for issue in &report.errors {
        message.push_str(&format!("\nerror: {}: {}", issue.path, issue.message));
    }
    for issue in &report.warnings {
        message.push_str(&format!("\nwarning: {}: {}", issue.path, issue.message));
    }
    message
}

impl IkarosConfig {
    pub fn validate_file(path: &Path) -> Result<ConfigValidationReport> {
        if !path.exists() {
            let mut report = ConfigValidationReport::default();
            report.error(
                "config",
                format!(
                    "{} does not exist; run `ikaros init` or create config.yaml under IKAROS_HOME",
                    path.display()
                ),
            );
            return Ok(report);
        }
        let raw = fs::read_to_string(path).map_err(|source| IkarosError::io(path, source))?;
        Self::validate_yaml(&raw)
    }

    pub fn validate_yaml(raw: &str) -> Result<ConfigValidationReport> {
        let value: yaml_serde::Value = yaml_serde::from_str(raw)?;
        let mut report = ConfigValidationReport::default();
        validate_yaml_shape(&value, &mut report);
        if !report.is_valid() {
            return Ok(report);
        }

        let mut config: Self = yaml_serde::from_str(raw)?;
        config.expand_presets();
        config.validate_into(&mut report);
        Ok(report)
    }

    pub fn validate(&self) -> ConfigValidationReport {
        let mut report = ConfigValidationReport::default();
        self.validate_into(&mut report);
        report
    }

    fn validate_into(&self, report: &mut ConfigValidationReport) {
        validate_schema_version(self.schema_version, report);
        agent::validate_agent_config(
            &self.agent,
            &self.model.default,
            &self.providers.model,
            report,
        );
        model::validate_model_config(
            "model.default",
            "providers.model",
            &self.model.default,
            &self.providers.model,
            report,
        );
        resources::validate_memory_config(&self.memory, report);
        resources::validate_rag_config(&self.rag, &self.providers.embedding, report);
        resources::validate_voice_config(
            "voice.tts",
            &self.voice.tts,
            &self.providers.tts,
            true,
            report,
        );
        resources::validate_voice_config(
            "voice.asr",
            &self.voice.asr,
            &self.providers.asr,
            false,
            report,
        );
        resources::validate_mcp_config(&self.mcp, report);
        safety::validate_execution_config(&self.execution, report);
        safety::validate_self_modify_config(&self.self_modify, report);
    }
}

fn validate_yaml_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(root) = expect_mapping(value, "config", report) else {
        return;
    };
    check_allowed_fields(
        root,
        "config",
        &[
            "schema_version",
            "providers",
            "agent",
            "model",
            "policy",
            "memory",
            "rag",
            "voice",
            "mcp",
            "execution",
            "self_modify",
        ],
        report,
    );
    check_required_fields(root, "config", &["schema_version"], report);
    if let Some(providers) = mapping_get(root, "providers") {
        validate_providers_shape(providers, report);
    }
    if let Some(agent) = mapping_get(root, "agent") {
        validate_agent_shape(agent, report);
    }
    if let Some(model) = mapping_get(root, "model") {
        validate_model_shape(model, report);
    }
    if let Some(policy) = mapping_get(root, "policy") {
        check_mapping_shape(
            policy,
            "policy",
            &["workspace_writes", "network", "audit_redaction"],
            report,
        );
    }
    if let Some(memory) = mapping_get(root, "memory") {
        validate_memory_shape(memory, report);
    }
    if let Some(rag) = mapping_get(root, "rag") {
        check_mapping_shape(
            rag,
            "rag",
            &[
                "backend",
                "embedding_provider",
                "embedding_model",
                "embedding_timeout_ms",
                "embedding_max_retries",
            ],
            report,
        );
    }
    if let Some(voice) = mapping_get(root, "voice") {
        validate_voice_shape(voice, report);
    }
    if let Some(mcp) = mapping_get(root, "mcp") {
        validate_mcp_shape(mcp, report);
    }
    if let Some(execution) = mapping_get(root, "execution") {
        validate_execution_shape(execution, report);
    }
    if let Some(self_modify) = mapping_get(root, "self_modify") {
        validate_self_modify_shape(self_modify, report);
    }
}

fn validate_schema_version(schema_version: u32, report: &mut ConfigValidationReport) {
    if schema_version != CURRENT_CONFIG_SCHEMA_VERSION {
        report.error(
            "schema_version",
            format!(
                "unsupported schema version {}; current supported version is {}",
                schema_version, CURRENT_CONFIG_SCHEMA_VERSION
            ),
        );
    }
}

fn validate_providers_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(map) = check_mapping_shape(
        value,
        "providers",
        &["model", "embedding", "tts", "asr", "search"],
        report,
    ) else {
        return;
    };
    for key in ["model", "embedding", "tts", "asr", "search"] {
        if let Some(provider) = mapping_get(map, key) {
            check_mapping_shape(
                provider,
                format!("providers.{key}"),
                &["api_key", "base_url"],
                report,
            );
        }
    }
}

fn validate_agent_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(map) = check_mapping_shape(
        value,
        "agent",
        &["default", "profiles", "instances"],
        report,
    ) else {
        return;
    };
    if let Some(profiles) = mapping_get(map, "profiles") {
        validate_dynamic_mapping_shape(
            profiles,
            "agent.profiles",
            &[
                "mode",
                "description",
                "persona_overlay",
                "memory_context",
                "rag_context",
                "toolsets",
                "workspace_writes",
                "shell",
                "network",
            ],
            report,
        );
    }
    if let Some(instances) = mapping_get(map, "instances") {
        let Some(instance_map) = expect_mapping(instances, "agent.instances", report) else {
            return;
        };
        check_string_keys(instance_map, "agent.instances", report);
        for (name, instance) in string_entries(instance_map) {
            let path = format!("agent.instances.{name}");
            let Some(map) = check_mapping_shape(
                instance,
                &path,
                &[
                    "profile",
                    "workspace",
                    "state_dir",
                    "toolsets",
                    "providers",
                    "model",
                    "session_policy",
                    "auth_scope",
                    "route_bindings",
                ],
                report,
            ) else {
                continue;
            };
            if let Some(providers) = mapping_get(map, "providers") {
                if let Some(providers_map) =
                    check_mapping_shape(providers, format!("{path}.providers"), &["model"], report)
                    && let Some(model_provider) = mapping_get(providers_map, "model")
                {
                    check_mapping_shape(
                        model_provider,
                        format!("{path}.providers.model"),
                        &["api_key", "base_url"],
                        report,
                    );
                }
            }
            if let Some(model) = mapping_get(map, "model") {
                validate_model_config_shape(model, &format!("{path}.model"), report);
            }
            if let Some(policy) = mapping_get(map, "session_policy") {
                check_mapping_shape(
                    policy,
                    format!("{path}.session_policy"),
                    &[
                        "history_scope",
                        "allow_session_switch",
                        "max_parallel_subagents",
                        "max_delegation_depth",
                    ],
                    report,
                );
            }
            if let Some(auth_scope) = mapping_get(map, "auth_scope") {
                check_mapping_shape(
                    auth_scope,
                    format!("{path}.auth_scope"),
                    &["local_only", "allow_network"],
                    report,
                );
            }
            if let Some(bindings) = mapping_get(map, "route_bindings") {
                validate_sequence_mapping_shape(
                    bindings,
                    format!("{path}.route_bindings"),
                    &["channel", "account", "peer", "thread"],
                    report,
                );
            }
        }
    }
}

fn validate_model_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(map) = check_mapping_shape(value, "model", &["default"], report) else {
        return;
    };
    if let Some(default) = mapping_get(map, "default") {
        let Some(default_map) = check_mapping_shape(
            default,
            "model.default",
            &[
                "preset",
                "provider",
                "runtime",
                "transport",
                "model",
                "api_key",
                "base_url",
                "compat_profile",
                "params",
                "reasoning",
                "extra_body",
                "cost",
                "timeout_ms",
                "max_retries",
                "rate_limit_per_minute",
                "daily_token_budget",
                "fallbacks",
            ],
            report,
        ) else {
            return;
        };
        if let Some(params) = mapping_get(default_map, "params") {
            check_mapping_shape(
                params,
                "model.default.params",
                &[
                    "max_tokens",
                    "temperature",
                    "top_p",
                    "n",
                    "presence_penalty",
                    "frequency_penalty",
                    "seed",
                    "stop",
                ],
                report,
            );
        }
        if let Some(reasoning) = mapping_get(default_map, "reasoning") {
            check_mapping_shape(
                reasoning,
                "model.default.reasoning",
                &["enabled", "effort"],
                report,
            );
        }
        if let Some(extra_body) = mapping_get(default_map, "extra_body") {
            expect_mapping(extra_body, "model.default.extra_body", report);
        }
        if let Some(cost) = mapping_get(default_map, "cost") {
            validate_model_cost_shape(cost, "model.default.cost", report);
        }
        if let Some(fallbacks) = mapping_get(default_map, "fallbacks") {
            validate_model_fallbacks_shape(fallbacks, "model.default.fallbacks", report);
        }
    }
}

fn validate_memory_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(map) = check_mapping_shape(
        value,
        "memory",
        &["backend", "policy", "external_providers"],
        report,
    ) else {
        return;
    };
    if let Some(policy) = mapping_get(map, "policy") {
        check_mapping_shape(
            policy,
            "memory.policy",
            &[
                "promote_threshold",
                "demote_threshold",
                "forget_threshold",
                "max_records_per_scope",
            ],
            report,
        );
    }
    if let Some(external) = mapping_get(map, "external_providers") {
        validate_sequence_mapping_shape(
            external,
            "memory.external_providers",
            &["id", "provider", "enabled", "endpoint", "api_key"],
            report,
        );
    }
}

fn validate_voice_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(map) = check_mapping_shape(value, "voice", &["tts", "asr"], report) else {
        return;
    };
    for key in ["tts", "asr"] {
        if let Some(provider) = mapping_get(map, key) {
            check_mapping_shape(
                provider,
                format!("voice.{key}"),
                &["provider", "model", "timeout_ms", "max_retries", "voice"],
                report,
            );
        }
    }
}

fn validate_mcp_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(map) = check_mapping_shape(value, "mcp", &["servers"], report) else {
        return;
    };
    if let Some(servers) = mapping_get(map, "servers") {
        validate_sequence_mapping_shape(
            servers,
            "mcp.servers",
            &[
                "id",
                "enabled",
                "transport",
                "command",
                "args",
                "include_tools",
                "exclude_tools",
                "timeout_ms",
                "max_output_bytes",
            ],
            report,
        );
    }
}

fn validate_self_modify_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(map) = check_mapping_shape(value, "self_modify", &["check_profiles"], report) else {
        return;
    };
    if let Some(check_profiles) = mapping_get(map, "check_profiles") {
        validate_dynamic_mapping_shape(
            check_profiles,
            "self_modify.check_profiles",
            &["commands", "reason"],
            report,
        );
    }
}

fn validate_execution_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(map) = check_mapping_shape(value, "execution", &["network", "sandbox"], report) else {
        return;
    };
    if let Some(network) = mapping_get(map, "network") {
        check_mapping_shape(
            network,
            "execution.network",
            &[
                "enabled",
                "allow_provider_hosts",
                "allowed_hosts",
                "timeout_ms",
            ],
            report,
        );
    }
    if let Some(sandbox) = mapping_get(map, "sandbox") {
        check_mapping_shape(
            sandbox,
            "execution.sandbox",
            &["backend", "image", "read_scope"],
            report,
        );
    }
}

fn check_mapping_shape<'a>(
    value: &'a yaml_serde::Value,
    path: impl AsRef<str>,
    allowed: &[&str],
    report: &mut ConfigValidationReport,
) -> Option<&'a yaml_serde::Mapping> {
    let path = path.as_ref();
    let map = expect_mapping(value, path, report)?;
    check_allowed_fields(map, path, allowed, report);
    Some(map)
}

fn validate_dynamic_mapping_shape(
    value: &yaml_serde::Value,
    path: impl AsRef<str>,
    allowed: &[&str],
    report: &mut ConfigValidationReport,
) {
    let path = path.as_ref();
    let Some(map) = expect_mapping(value, path, report) else {
        return;
    };
    check_string_keys(map, path, report);
    for (key, value) in string_entries(map) {
        check_mapping_shape(value, format!("{path}.{key}"), allowed, report);
    }
}

fn validate_sequence_mapping_shape(
    value: &yaml_serde::Value,
    path: impl AsRef<str>,
    allowed: &[&str],
    report: &mut ConfigValidationReport,
) {
    let path = path.as_ref();
    let yaml_serde::Value::Sequence(items) = value else {
        report.error(path, "must be a YAML sequence");
        return;
    };
    for (index, item) in items.iter().enumerate() {
        check_mapping_shape(item, format!("{path}[{index}]"), allowed, report);
    }
}

fn expect_mapping<'a>(
    value: &'a yaml_serde::Value,
    path: impl AsRef<str>,
    report: &mut ConfigValidationReport,
) -> Option<&'a yaml_serde::Mapping> {
    match value {
        yaml_serde::Value::Mapping(map) => Some(map),
        _ => {
            report.error(path.as_ref(), "must be a YAML mapping");
            None
        }
    }
}

fn check_allowed_fields(
    map: &yaml_serde::Mapping,
    path: impl AsRef<str>,
    allowed: &[&str],
    report: &mut ConfigValidationReport,
) {
    let path = path.as_ref();
    let allowed = allowed.iter().copied().collect::<BTreeSet<_>>();
    for key in map.keys() {
        let Some(key) = key.as_str() else {
            report.error(path, "mapping keys must be strings");
            continue;
        };
        if !allowed.contains(key) {
            report.error(format!("{path}.{key}"), "unknown configuration field");
        }
    }
}

fn check_required_fields(
    map: &yaml_serde::Mapping,
    path: impl AsRef<str>,
    required: &[&str],
    report: &mut ConfigValidationReport,
) {
    let path = path.as_ref();
    for key in required {
        if mapping_get(map, key).is_none() {
            report.error(format!("{path}.{key}"), "required field is missing");
        }
    }
}

fn check_string_keys(
    map: &yaml_serde::Mapping,
    path: impl AsRef<str>,
    report: &mut ConfigValidationReport,
) {
    let path = path.as_ref();
    for key in map.keys() {
        if key.as_str().is_none() {
            report.error(path, "mapping keys must be strings");
        }
    }
}

fn mapping_get<'a>(map: &'a yaml_serde::Mapping, key: &str) -> Option<&'a yaml_serde::Value> {
    map.get(yaml_serde::Value::String(key.to_owned()))
}

fn string_entries(map: &yaml_serde::Mapping) -> impl Iterator<Item = (&str, &yaml_serde::Value)> {
    map.iter()
        .filter_map(|(key, value)| key.as_str().map(|key| (key, value)))
}

fn validate_model_config_shape(
    value: &yaml_serde::Value,
    path: &str,
    report: &mut ConfigValidationReport,
) {
    let Some(map) = check_mapping_shape(
        value,
        path,
        &[
            "preset",
            "provider",
            "runtime",
            "transport",
            "model",
            "api_key",
            "base_url",
            "compat_profile",
            "params",
            "reasoning",
            "extra_body",
            "cost",
            "timeout_ms",
            "max_retries",
            "rate_limit_per_minute",
            "daily_token_budget",
            "fallbacks",
        ],
        report,
    ) else {
        return;
    };
    if let Some(params) = mapping_get(map, "params") {
        check_mapping_shape(
            params,
            format!("{path}.params"),
            &[
                "max_tokens",
                "temperature",
                "top_p",
                "n",
                "presence_penalty",
                "frequency_penalty",
                "seed",
                "stop",
            ],
            report,
        );
    }
    if let Some(reasoning) = mapping_get(map, "reasoning") {
        check_mapping_shape(
            reasoning,
            format!("{path}.reasoning"),
            &["enabled", "effort"],
            report,
        );
    }
    if let Some(extra_body) = mapping_get(map, "extra_body") {
        expect_mapping(extra_body, format!("{path}.extra_body"), report);
    }
    if let Some(cost) = mapping_get(map, "cost") {
        validate_model_cost_shape(cost, format!("{path}.cost"), report);
    }
    if let Some(fallbacks) = mapping_get(map, "fallbacks") {
        validate_model_fallbacks_shape(fallbacks, format!("{path}.fallbacks"), report);
    }
}

fn validate_model_fallbacks_shape(
    value: &yaml_serde::Value,
    path: impl AsRef<str>,
    report: &mut ConfigValidationReport,
) {
    let path = path.as_ref();
    let yaml_serde::Value::Sequence(items) = value else {
        report.error(path, "must be a YAML sequence");
        return;
    };
    for (index, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        let Some(map) = check_mapping_shape(
            item,
            &item_path,
            &[
                "provider",
                "runtime",
                "transport",
                "model",
                "preset",
                "compat_profile",
                "params",
                "reasoning",
                "extra_body",
                "cost",
                "timeout_ms",
                "max_retries",
                "rate_limit_per_minute",
                "daily_token_budget",
                "api_key",
                "base_url",
            ],
            report,
        ) else {
            continue;
        };
        if let Some(params) = mapping_get(map, "params") {
            check_mapping_shape(
                params,
                format!("{item_path}.params"),
                &[
                    "max_tokens",
                    "temperature",
                    "top_p",
                    "n",
                    "presence_penalty",
                    "frequency_penalty",
                    "seed",
                    "stop",
                ],
                report,
            );
        }
        if let Some(reasoning) = mapping_get(map, "reasoning") {
            check_mapping_shape(
                reasoning,
                format!("{item_path}.reasoning"),
                &["enabled", "effort"],
                report,
            );
        }
        if let Some(extra_body) = mapping_get(map, "extra_body") {
            expect_mapping(extra_body, format!("{item_path}.extra_body"), report);
        }
        if let Some(cost) = mapping_get(map, "cost") {
            validate_model_cost_shape(cost, format!("{item_path}.cost"), report);
        }
    }
}

fn validate_model_cost_shape(
    value: &yaml_serde::Value,
    path: impl AsRef<str>,
    report: &mut ConfigValidationReport,
) {
    check_mapping_shape(
        value,
        path,
        &[
            "currency",
            "input_per_million",
            "output_per_million",
            "cache_read_per_million",
            "cache_write_per_million",
        ],
        report,
    );
}

fn validate_timeout(path: impl Into<String>, timeout_ms: u64, report: &mut ConfigValidationReport) {
    if timeout_ms == 0 {
        report.error(path.into(), "must be greater than 0");
    }
}

fn validate_optional_positive(
    path: impl Into<String>,
    value: Option<u32>,
    report: &mut ConfigValidationReport,
) {
    if value == Some(0) {
        report.error(path.into(), "must be greater than 0 when set");
    }
}

fn validate_float_range(
    path: impl Into<String>,
    value: f32,
    min: f32,
    max: f32,
    report: &mut ConfigValidationReport,
) {
    let path = path.into();
    if !value.is_finite() {
        report.error(path, "must be finite");
        return;
    }
    if value < min || value > max {
        report.error(path, format!("must be between {min} and {max}"));
    }
}

fn validate_required(path: impl Into<String>, value: &str, report: &mut ConfigValidationReport) {
    if value.trim().is_empty() {
        report.error(path.into(), "must not be empty");
    }
}

fn validate_required_url(
    path: impl Into<String>,
    value: &str,
    report: &mut ConfigValidationReport,
) {
    let path = path.into();
    if value.trim().is_empty() {
        report.error(path, "must not be empty");
        return;
    }
    validate_url(path, value, report);
}

fn validate_optional_url(
    path: impl Into<String>,
    value: &str,
    report: &mut ConfigValidationReport,
) {
    if value.trim().is_empty() {
        return;
    }
    validate_url(path, value, report);
}

fn validate_url(path: impl Into<String>, value: &str, report: &mut ConfigValidationReport) {
    let path = path.into();
    let trimmed = value.trim();
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        report.error(path, "must start with http:// or https://");
    }
}

fn is_remote_embedding_provider(provider: &str) -> bool {
    provider == "openai-compatible"
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}
