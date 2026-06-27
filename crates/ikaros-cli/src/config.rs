// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use ikaros_core::{ConfigValidationReport, IkarosConfig, IkarosPaths};
use serde_json::json;
use std::fs;

#[derive(Debug, Subcommand)]
pub(crate) enum ConfigCommand {
    /// Validate IKAROS_HOME/config.yaml without printing secret values.
    Validate {
        /// Print a machine-readable JSON report.
        #[arg(long)]
        json: bool,
    },
    /// Print a redacted summary of the active runtime config.
    Show {
        /// Print a machine-readable JSON summary.
        #[arg(long)]
        json: bool,
    },
    /// Show or update model.default.daily_token_budget in config.yaml.
    Budget(ConfigBudgetArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ConfigBudgetArgs {
    /// Set model.default.daily_token_budget to this token count.
    #[arg(long, conflicts_with = "disable")]
    pub(crate) set: Option<u32>,
    /// Disable model.default.daily_token_budget by writing null.
    #[arg(long)]
    pub(crate) disable: bool,
    /// Print a machine-readable JSON summary.
    #[arg(long)]
    pub(crate) json: bool,
}

pub(crate) struct ConfigBudgetUpdate {
    pub(crate) before: Option<u32>,
    pub(crate) current: Option<u32>,
    pub(crate) changed: bool,
}

pub(crate) fn config_command(command: ConfigCommand, paths: &IkarosPaths) -> Result<()> {
    match command {
        ConfigCommand::Validate { json } => validate_config(paths, json),
        ConfigCommand::Show { json } => show_config(paths, json),
        ConfigCommand::Budget(args) => budget_config(paths, args),
    }
}

fn validate_config(paths: &IkarosPaths, json: bool) -> Result<()> {
    let report = IkarosConfig::validate_file(&paths.config)?;
    if json {
        print_json_report(&paths.config.display().to_string(), &report)?;
        if report.is_valid() {
            return Ok(());
        }
        std::process::exit(1);
    }
    print_report(&report);
    if report.is_valid() {
        println!("config valid: {}", paths.config.display());
        Ok(())
    } else {
        bail!(
            "configuration validation failed: {}",
            paths.config.display()
        )
    }
}

fn print_json_report(path: &str, report: &ConfigValidationReport) -> Result<()> {
    let json = json!({
        "path": path,
        "valid": report.is_valid(),
        "errors": &report.errors,
        "warnings": &report.warnings,
    });
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}

fn print_report(report: &ConfigValidationReport) {
    for warning in &report.warnings {
        println!("warning: {}: {}", warning.path, warning.message);
    }
    for error in &report.errors {
        println!("error: {}: {}", error.path, error.message);
    }
}

fn show_config(paths: &IkarosPaths, json: bool) -> Result<()> {
    let config = IkarosConfig::load_shape_checked(&paths.config)?;
    if json {
        let report = json!({
            "path": paths.config.display().to_string(),
            "schema_version": config.schema_version,
            "model": {
                "provider": config.model.default.provider,
                "runtime": config.model.default.runtime,
                "transport": config.model.default.transport,
                "model": config.model.default.model,
                "compat_profile": config.model.default.compat_profile,
                "api_key_configured": !config.providers.model.api_key.trim().is_empty(),
                "base_url_configured": !config.providers.model.base_url.trim().is_empty(),
                "daily_token_budget": config.model.default.daily_token_budget,
                "rate_limit_per_minute": config.model.default.rate_limit_per_minute,
                "fallback_count": config.model.default.fallbacks.len(),
            },
            "memory": {
                "backend": config.memory.backend,
                "external_provider_count": config.memory.external_providers.len(),
            },
            "rag": {
                "backend": config.rag.backend,
                "embedding_provider": config.rag.embedding_provider,
                "embedding_model": config.rag.embedding_model,
                "embedding_api_key_configured": !config.providers.embedding.api_key.trim().is_empty(),
                "embedding_base_url_configured": !config.providers.embedding.base_url.trim().is_empty(),
            },
            "voice": {
                "tts": {
                    "provider": config.voice.tts.provider,
                    "model": config.voice.tts.model,
                    "api_key_configured": !config.providers.tts.api_key.trim().is_empty(),
                    "base_url_configured": !config.providers.tts.base_url.trim().is_empty(),
                },
                "asr": {
                    "provider": config.voice.asr.provider,
                    "model": config.voice.asr.model,
                    "api_key_configured": !config.providers.asr.api_key.trim().is_empty(),
                    "base_url_configured": !config.providers.asr.base_url.trim().is_empty(),
                },
            },
            "web": {
                "search_api_key_configured": !config.providers.search.api_key.trim().is_empty(),
                "search_base_url_configured": !config.providers.search.base_url.trim().is_empty(),
            },
            "execution": {
                "network_enabled": config.execution.network.enabled,
                "allow_provider_hosts": config.execution.network.allow_provider_hosts,
                "allowed_host_count": config.execution.network.allowed_hosts.len(),
                "network_timeout_ms": config.execution.network.timeout_ms,
                "sandbox_backend": config.execution.sandbox.backend,
                "sandbox_image": config.execution.sandbox.image,
                "sandbox_read_scope": config.execution.sandbox.read_scope,
            },
            "agent": {
                "default": config.agent.default,
                "profile_count": config.agent.profiles.len(),
                "instance_count": config.agent.instances.len(),
            },
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("config: {}", paths.config.display());
    println!("schema_version: {}", config.schema_version);
    println!("agent_default: {}", config.agent.default);
    println!("agent_profiles: {}", config.agent.profiles.len());
    println!("agent_instances: {}", config.agent.instances.len());
    println!("model_provider: {}", config.model.default.provider);
    println!("model_runtime: {}", config.model.default.runtime);
    println!("model_transport: {}", config.model.default.transport);
    println!("model_model: {}", config.model.default.model);
    println!(
        "model_compat_profile: {}",
        config.model.default.compat_profile
    );
    println!(
        "model_api_key_configured: {}",
        !config.providers.model.api_key.trim().is_empty()
    );
    println!(
        "model_base_url_configured: {}",
        !config.providers.model.base_url.trim().is_empty()
    );
    println!(
        "model_daily_token_budget: {}",
        config
            .model
            .default
            .daily_token_budget
            .map_or_else(|| "none".to_owned(), |budget| budget.to_string())
    );
    println!(
        "model_rate_limit_per_minute: {}",
        config
            .model
            .default
            .rate_limit_per_minute
            .map_or_else(|| "none".to_owned(), |limit| limit.to_string())
    );
    println!("model_fallbacks: {}", config.model.default.fallbacks.len());
    println!("memory_backend: {}", config.memory.backend);
    println!(
        "memory_external_providers: {}",
        config.memory.external_providers.len()
    );
    println!("rag_backend: {}", config.rag.backend);
    println!("rag_embedding_provider: {}", config.rag.embedding_provider);
    println!(
        "rag_embedding_model: {}",
        display_optional(&config.rag.embedding_model)
    );
    println!(
        "rag_embedding_api_key_configured: {}",
        !config.providers.embedding.api_key.trim().is_empty()
    );
    println!(
        "rag_embedding_base_url_configured: {}",
        !config.providers.embedding.base_url.trim().is_empty()
    );
    println!("voice_tts_provider: {}", config.voice.tts.provider);
    println!(
        "voice_tts_model: {}",
        display_optional(&config.voice.tts.model)
    );
    println!(
        "voice_tts_api_key_configured: {}",
        !config.providers.tts.api_key.trim().is_empty()
    );
    println!(
        "voice_tts_base_url_configured: {}",
        !config.providers.tts.base_url.trim().is_empty()
    );
    println!("voice_asr_provider: {}", config.voice.asr.provider);
    println!(
        "voice_asr_model: {}",
        display_optional(&config.voice.asr.model)
    );
    println!(
        "voice_asr_api_key_configured: {}",
        !config.providers.asr.api_key.trim().is_empty()
    );
    println!(
        "voice_asr_base_url_configured: {}",
        !config.providers.asr.base_url.trim().is_empty()
    );
    println!(
        "web_search_api_key_configured: {}",
        !config.providers.search.api_key.trim().is_empty()
    );
    println!(
        "web_search_base_url_configured: {}",
        !config.providers.search.base_url.trim().is_empty()
    );
    println!(
        "execution_network_enabled: {}",
        config.execution.network.enabled
    );
    println!(
        "execution_allow_provider_hosts: {}",
        config.execution.network.allow_provider_hosts
    );
    println!(
        "execution_allowed_hosts: {}",
        config.execution.network.allowed_hosts.len()
    );
    println!(
        "execution_network_timeout_ms: {}",
        config.execution.network.timeout_ms
    );
    println!(
        "execution_sandbox_backend: {}",
        config.execution.sandbox.backend
    );
    println!(
        "execution_sandbox_image: {}",
        config.execution.sandbox.image
    );
    println!(
        "execution_sandbox_read_scope: {}",
        config.execution.sandbox.read_scope
    );
    Ok(())
}

fn display_optional(value: &str) -> &str {
    if value.trim().is_empty() {
        "none"
    } else {
        value
    }
}

fn budget_config(paths: &IkarosPaths, args: ConfigBudgetArgs) -> Result<()> {
    let update = update_budget_config(paths, args.set, args.disable)?;
    if args.json {
        let payload = json!({
            "path": paths.config.display().to_string(),
            "changed": update.changed,
            "daily_token_budget_before": update.before,
            "daily_token_budget": update.current,
            "config_key": "model.default.daily_token_budget",
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("config: {}", paths.config.display());
        println!("config_key: model.default.daily_token_budget");
        println!(
            "daily_token_budget_before: {}",
            display_optional_budget(update.before)
        );
        println!(
            "daily_token_budget: {}",
            display_optional_budget(update.current)
        );
        println!("changed: {}", update.changed);
        if update.current.is_none() {
            println!("model_budget: disabled");
        }
    }
    Ok(())
}

pub(crate) fn update_budget_config(
    paths: &IkarosPaths,
    set: Option<u32>,
    disable: bool,
) -> Result<ConfigBudgetUpdate> {
    if set == Some(0) {
        bail!("--set 0 is not valid; use --disable to remove the daily token budget");
    }
    let config = IkarosConfig::load_shape_checked(&paths.config)?;
    let before = config.model.default.daily_token_budget;
    let changed = set.is_some() || disable;
    let after = if disable { None } else { set.or(before) };
    if changed {
        let raw = fs::read_to_string(&paths.config)
            .with_context(|| format!("failed to read config: {}", paths.config.display()))?;
        let value = after
            .map(|budget| budget.to_string())
            .unwrap_or_else(|| "null".into());
        let updated =
            set_yaml_scalar_raw(raw, &["model", "default", "daily_token_budget"], &value)?;
        IkarosConfig::load_yaml_shape_checked(&updated)
            .with_context(|| "budget update produced invalid configuration shape")?;
        fs::write(&paths.config, updated)
            .with_context(|| format!("failed to write config: {}", paths.config.display()))?;
    }
    Ok(ConfigBudgetUpdate {
        before,
        current: if changed { after } else { before },
        changed,
    })
}

fn display_optional_budget(value: Option<u32>) -> String {
    value.map_or_else(|| "disabled".into(), |budget| budget.to_string())
}

fn set_yaml_scalar_raw(raw: String, path: &[&str], value: &str) -> Result<String> {
    let mut found = false;
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut output = String::new();
    for line in raw.lines() {
        let mut next = line.to_owned();
        if let Some((indent, key, colon_index)) = yaml_key(line) {
            while stack.last().is_some_and(|(level, _)| *level >= indent) {
                stack.pop();
            }
            stack.push((indent, key.to_owned()));
            if stack
                .iter()
                .map(|(_, key)| key.as_str())
                .eq(path.iter().copied())
            {
                next = format!("{} {}", &line[..=colon_index], value);
                found = true;
            }
        }
        output.push_str(&next);
        output.push('\n');
    }
    if !found {
        bail!(
            "config path `{}` was not found in config.yaml",
            path.join(".")
        );
    }
    Ok(output)
}

fn yaml_key(line: &str) -> Option<(usize, &str, usize)> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
        return None;
    }
    let indent = line.len() - trimmed.len();
    let colon_relative = trimmed.find(':')?;
    let key = trimmed[..colon_relative].trim();
    if key.is_empty() || key.contains(' ') || key.contains('\t') {
        return None;
    }
    Some((indent, key, indent + colon_relative))
}
