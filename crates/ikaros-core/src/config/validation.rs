// SPDX-License-Identifier: GPL-3.0-only

use crate::{AgentConfig, IkarosError, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, path::Path};

use super::{
    IkarosConfig, MemoryConfig, MemoryPolicyConfig, ModelConfig, RagConfig, RemoteProviderConfig,
    SelfModifyConfig, VoiceProviderConfig,
};

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

fn format_validation_failure(summary: &str, report: &ConfigValidationReport) -> String {
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

        let config: Self = yaml_serde::from_str(raw)?;
        config.validate_into(&mut report);
        Ok(report)
    }

    pub fn validate(&self) -> ConfigValidationReport {
        let mut report = ConfigValidationReport::default();
        self.validate_into(&mut report);
        report
    }

    fn validate_into(&self, report: &mut ConfigValidationReport) {
        validate_agent_config(&self.agent, report);
        validate_model_config(&self.model.default, &self.providers.model, report);
        validate_policy_config(&self.policy.workspace_writes, &self.policy.network, report);
        validate_memory_config(&self.memory, report);
        validate_local_backend("chat_history.backend", &self.chat_history.backend, report);
        validate_rag_config(&self.rag, &self.providers.embedding, report);
        validate_voice_config(
            "voice.tts",
            &self.voice.tts,
            &self.providers.tts,
            true,
            report,
        );
        validate_voice_config(
            "voice.asr",
            &self.voice.asr,
            &self.providers.asr,
            false,
            report,
        );
        validate_self_modify_config(&self.self_modify, report);
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
            "providers",
            "agent",
            "model",
            "policy",
            "memory",
            "chat_history",
            "rag",
            "voice",
            "self_modify",
        ],
        report,
    );
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
    if let Some(chat_history) = mapping_get(root, "chat_history") {
        check_mapping_shape(chat_history, "chat_history", &["backend"], report);
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
    if let Some(self_modify) = mapping_get(root, "self_modify") {
        validate_self_modify_shape(self_modify, report);
    }
}

fn validate_providers_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(map) = check_mapping_shape(
        value,
        "providers",
        &["model", "embedding", "tts", "asr"],
        report,
    ) else {
        return;
    };
    for key in ["model", "embedding", "tts", "asr"] {
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
                    "session_policy",
                    "auth_scope",
                    "route_bindings",
                ],
                report,
            ) else {
                continue;
            };
            if let Some(policy) = mapping_get(map, "session_policy") {
                check_mapping_shape(
                    policy,
                    format!("{path}.session_policy"),
                    &[
                        "history_scope",
                        "allow_session_switch",
                        "max_parallel_subagents",
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
                "provider",
                "runtime",
                "transport",
                "model",
                "compat_profile",
                "params",
                "reasoning",
                "extra_body",
                "timeout_ms",
                "max_retries",
                "rate_limit_per_minute",
                "daily_token_budget",
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

fn validate_agent_config(config: &AgentConfig, report: &mut ConfigValidationReport) {
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
    for name in config.profiles.keys() {
        let path = format!("agent.profiles.{name}");
        if name.trim().is_empty() {
            report.error(&path, "profile name must not be empty");
        }
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
        if instance.session_policy.max_parallel_subagents == 0 {
            report.error(
                format!("{path}.session_policy.max_parallel_subagents"),
                "must be greater than 0",
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

fn validate_model_config(
    config: &ModelConfig,
    provider_settings: &RemoteProviderConfig,
    report: &mut ConfigValidationReport,
) {
    let provider = normalize(&config.provider);
    if !is_allowed_model_provider(&provider) {
        report.error(
            "model.default.provider",
            format!("unsupported model provider `{}`", config.provider),
        );
        return;
    }
    if config.runtime.trim() != "harness-agent-loop" {
        report.error(
            "model.default.runtime",
            "only `harness-agent-loop` is supported",
        );
    }
    validate_model_transport(&provider, &config.transport, report);
    validate_timeout("model.default.timeout_ms", config.timeout_ms, report);
    validate_optional_positive(
        "model.default.rate_limit_per_minute",
        config.rate_limit_per_minute,
        report,
    );
    validate_optional_positive(
        "model.default.daily_token_budget",
        config.daily_token_budget,
        report,
    );
    validate_model_profile(&provider, &config.compat_profile, report);
    validate_model_params(config, report);
    if provider == "mock" {
        if config.model.trim().is_empty() {
            report.warning(
                "model.default.model",
                "mock provider is selected with an empty model name",
            );
        }
        return;
    }
    validate_required("model.default.model", &config.model, report);
    if provider == "ollama" {
        validate_optional_url(
            "providers.model.base_url",
            &provider_settings.base_url,
            report,
        );
    } else {
        validate_required_url(
            "providers.model.base_url",
            &provider_settings.base_url,
            report,
        );
        validate_required(
            "providers.model.api_key",
            &provider_settings.api_key,
            report,
        );
    }
}

fn validate_model_profile(
    provider: &str,
    compat_profile: &str,
    report: &mut ConfigValidationReport,
) {
    let profile = normalize(compat_profile);
    let allowed = [
        "auto",
        "generic",
        "moonshot-kimi",
        "deepseek",
        "gemini-openai",
        "openrouter",
        "qwen",
        "local-openai-compatible",
    ];
    if !allowed.contains(&profile.as_str()) {
        report.error(
            "model.default.compat_profile",
            "must be one of: auto, generic, moonshot-kimi, deepseek, gemini-openai, openrouter, qwen, local-openai-compatible",
        );
    }
    if provider != "openai-compatible" && !matches!(profile.as_str(), "auto" | "generic") {
        report.error(
            "model.default.compat_profile",
            "provider-specific compatibility profiles are only valid with `openai-compatible`",
        );
    }
}

fn validate_model_params(config: &ModelConfig, report: &mut ConfigValidationReport) {
    validate_optional_positive(
        "model.default.params.max_tokens",
        config.params.max_tokens,
        report,
    );
    validate_optional_positive("model.default.params.n", config.params.n, report);
    if let Some(temperature) = config.params.temperature {
        validate_float_range(
            "model.default.params.temperature",
            temperature,
            0.0,
            2.0,
            report,
        );
    }
    if let Some(top_p) = config.params.top_p {
        validate_float_range("model.default.params.top_p", top_p, 0.0, 1.0, report);
    }
    if let Some(presence_penalty) = config.params.presence_penalty {
        validate_float_range(
            "model.default.params.presence_penalty",
            presence_penalty,
            -2.0,
            2.0,
            report,
        );
    }
    if let Some(frequency_penalty) = config.params.frequency_penalty {
        validate_float_range(
            "model.default.params.frequency_penalty",
            frequency_penalty,
            -2.0,
            2.0,
            report,
        );
    }
    for (index, stop) in config.params.stop.iter().enumerate() {
        if stop.is_empty() {
            report.error(
                format!("model.default.params.stop[{index}]"),
                "must not be empty",
            );
        }
    }
    if let Some(effort) = &config.reasoning.effort {
        let effort = normalize(effort);
        if !matches!(
            effort.as_str(),
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
        ) {
            report.error(
                "model.default.reasoning.effort",
                "must be one of: none, minimal, low, medium, high, xhigh, max",
            );
        }
    }
}

fn validate_model_transport(provider: &str, transport: &str, report: &mut ConfigValidationReport) {
    let transport = transport.trim();
    if transport.is_empty() {
        report.error("model.default.transport", "must not be empty");
        return;
    }
    let expected = match provider {
        "mock" => "mock",
        "openai-compatible" => "openai-compatible-chat-completions",
        "anthropic" => "anthropic-messages",
        "ollama" => "ollama-chat",
        _ => return,
    };
    if transport != expected {
        report.error(
            "model.default.transport",
            format!("provider `{provider}` requires transport `{expected}`"),
        );
    }
}

fn validate_policy_config(
    workspace_writes: &str,
    network: &str,
    report: &mut ConfigValidationReport,
) {
    validate_policy_value("policy.workspace_writes", workspace_writes, report);
    validate_policy_value("policy.network", network, report);
}

fn validate_memory_config(config: &MemoryConfig, report: &mut ConfigValidationReport) {
    validate_local_backend("memory.backend", &config.backend, report);
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

fn validate_rag_config(
    config: &RagConfig,
    provider_settings: &RemoteProviderConfig,
    report: &mut ConfigValidationReport,
) {
    validate_local_backend("rag.backend", &config.backend, report);
    let provider = normalize(&config.embedding_provider);
    if !is_allowed_embedding_provider(&provider) {
        report.error(
            "rag.embedding_provider",
            format!(
                "unsupported embedding provider `{}`",
                config.embedding_provider
            ),
        );
        return;
    }
    validate_timeout(
        "rag.embedding_timeout_ms",
        config.embedding_timeout_ms,
        report,
    );
    if is_remote_embedding_provider(&provider) {
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

fn validate_voice_config(
    path: &str,
    config: &VoiceProviderConfig,
    provider_settings: &RemoteProviderConfig,
    is_tts: bool,
    report: &mut ConfigValidationReport,
) {
    let provider = normalize(&config.provider);
    if !is_allowed_voice_provider(&provider) {
        report.error(
            format!("{path}.provider"),
            format!("unsupported voice provider `{}`", config.provider),
        );
        return;
    }
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

fn validate_self_modify_config(config: &SelfModifyConfig, report: &mut ConfigValidationReport) {
    for (name, profile) in &config.check_profiles {
        let path = format!("self_modify.check_profiles.{name}");
        if name.trim().is_empty() {
            report.error(&path, "check profile name must not be empty");
        }
        if profile.commands.is_empty() {
            report.error(
                format!("{path}.commands"),
                "must contain at least one command",
            );
        }
        for (index, command) in profile.commands.iter().enumerate() {
            if command.trim().is_empty() {
                report.error(format!("{path}.commands[{index}]"), "must not be empty");
            }
        }
    }
}

fn validate_policy_value(
    path: impl Into<String>,
    value: &str,
    report: &mut ConfigValidationReport,
) {
    let path = path.into();
    if !matches!(normalize(value).as_str(), "allow" | "ask" | "deny") {
        report.error(path, "must be one of: allow, ask, deny");
    }
}

fn validate_local_backend(
    path: impl Into<String>,
    backend: &str,
    report: &mut ConfigValidationReport,
) {
    let path = path.into();
    if !matches!(normalize(backend).as_str(), "jsonl" | "sqlite") {
        report.error(path, "must be one of: jsonl, sqlite");
    }
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

fn is_allowed_model_provider(provider: &str) -> bool {
    matches!(
        provider,
        "mock" | "openai-compatible" | "anthropic" | "ollama"
    )
}

fn is_allowed_embedding_provider(provider: &str) -> bool {
    matches!(provider, "hash" | "sparse" | "mock" | "openai-compatible")
}

fn is_remote_embedding_provider(provider: &str) -> bool {
    provider == "openai-compatible"
}

fn is_allowed_voice_provider(provider: &str) -> bool {
    matches!(provider, "mock" | "openai-compatible")
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}
