// SPDX-License-Identifier: GPL-3.0-only

mod output;
mod setup_prompt;
mod setup_resources;
mod setup_yaml;

use anyhow::{Context, Result, bail};
use clap::Args;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_host::{
    initialize_runtime_home, initialize_runtime_home_with_options, runtime_doctor_report,
};
use output::{display_optional_model, print_doctor_report, print_init_report};
use setup_prompt::prompt_setup_args;
use setup_resources::{
    apply_reused_model_provider_resources, model_api_key_for_setup, model_base_url_for_setup,
    model_for_setup, model_transport_for_provider, normalize_provider, setup_compat_profile,
    setup_embedding,
};
use setup_yaml::{
    config_has_setup_paths, format_setup_validation_failure, set_yaml_scalar, set_yaml_scalar_raw,
};
use std::{fs, path::Path};

#[derive(Debug, Args, Default)]
pub(crate) struct DoctorArgs {}

#[derive(Debug, Args, Default)]
pub(crate) struct InitArgs {
    /// Write a complete default config instead of the minimal starter config.
    #[arg(long)]
    pub(crate) full: bool,
}

#[derive(Debug, Args)]
pub(crate) struct SetupArgs {
    /// Prompt for missing setup fields interactively.
    #[arg(long)]
    interactive: bool,
    /// Model provider family: openai-compatible, anthropic, ollama, or mock.
    #[arg(long, default_value = "openai-compatible")]
    provider: String,
    /// API key for remote model providers. The value is written to IKAROS_HOME/config.yaml and never printed.
    #[arg(long)]
    api_key: Option<String>,
    /// Base URL for remote model providers.
    #[arg(long)]
    base_url: Option<String>,
    /// Model id to send to the provider.
    #[arg(long)]
    model: Option<String>,
    /// OpenAI-compatible profile: auto, generic, moonshot-kimi, deepseek, gemini-openai, openrouter, qwen, or local-openai-compatible.
    #[arg(long)]
    compat_profile: Option<String>,
    /// Optional daily token budget. Omit to keep the generated null budget.
    #[arg(long)]
    daily_token_budget: Option<u64>,
    /// Remote embedding API key. When omitted, setup configures local hash embeddings.
    #[arg(long)]
    embedding_api_key: Option<String>,
    /// Remote embedding base URL. Required with --embedding-api-key.
    #[arg(long)]
    embedding_base_url: Option<String>,
    /// Remote embedding model. Required with --embedding-api-key.
    #[arg(long)]
    embedding_model: Option<String>,
    /// Reuse the model provider API key and base URL for remote embeddings.
    #[arg(long)]
    reuse_model_provider_for_embedding: bool,
}

pub(crate) fn init(args: InitArgs, paths: &IkarosPaths) -> Result<()> {
    let report = initialize_runtime_home_with_options(paths, args.full)?;
    println!("Ikaros initialized");
    print_init_report(&report);
    Ok(())
}

pub(crate) fn setup(mut args: SetupArgs, paths: &IkarosPaths) -> Result<()> {
    if args.interactive {
        prompt_setup_args(&mut args)?;
    }
    let init_report = initialize_runtime_home(paths)?;
    if init_report.config_created || !config_has_setup_paths(&paths.config)? {
        IkarosConfig::write_full_config(&paths.config)?;
    }
    let mut raw = fs::read_to_string(&paths.config)
        .with_context(|| format!("failed to read config: {}", paths.config.display()))?;
    let provider = normalize_provider(&args.provider)?;
    let transport = model_transport_for_provider(provider);
    let model = model_for_setup(provider, args.model.as_deref())?;
    let api_key = model_api_key_for_setup(provider, args.api_key.as_deref())?.to_owned();
    let base_url = model_base_url_for_setup(provider, args.base_url.as_deref())?.to_owned();
    let compat_profile = setup_compat_profile(provider, args.compat_profile.as_deref());
    apply_reused_model_provider_resources(&mut args, &api_key, &base_url)?;

    raw = set_yaml_scalar(raw, &["providers", "model", "api_key"], &api_key)?;
    raw = set_yaml_scalar(raw, &["providers", "model", "base_url"], &base_url)?;
    raw = set_yaml_scalar(raw, &["model", "default", "provider"], provider)?;
    raw = set_yaml_scalar(raw, &["model", "default", "transport"], transport)?;
    raw = set_yaml_scalar(raw, &["model", "default", "model"], &model)?;
    raw = set_yaml_scalar(
        raw,
        &["model", "default", "compat_profile"],
        &compat_profile,
    )?;
    if let Some(budget) = args.daily_token_budget {
        raw = set_yaml_scalar_raw(
            raw,
            &["model", "default", "daily_token_budget"],
            &budget.to_string(),
        )?;
    }

    let embedding = setup_embedding(&args)?;
    raw = set_yaml_scalar(
        raw,
        &["providers", "embedding", "api_key"],
        embedding.api_key,
    )?;
    raw = set_yaml_scalar(
        raw,
        &["providers", "embedding", "base_url"],
        embedding.base_url,
    )?;
    raw = set_yaml_scalar(raw, &["rag", "embedding_provider"], embedding.provider)?;
    raw = set_yaml_scalar(raw, &["rag", "embedding_model"], embedding.model)?;

    let report = IkarosConfig::validate_yaml(&raw)?;
    if !report.is_valid() {
        bail!(
            "{}",
            format_setup_validation_failure("setup produced invalid configuration", &report)
        );
    }
    fs::write(&paths.config, raw)
        .with_context(|| format!("failed to write config: {}", paths.config.display()))?;

    println!("Ikaros setup");
    println!("home: {}", paths.home.display());
    println!("config: {}", paths.config.display());
    println!("config_created: {}", init_report.config_created);
    println!("model_provider: {provider}");
    println!("model_transport: {transport}");
    println!("model_model: {model}");
    println!("model_base_url_configured: {}", !base_url.is_empty());
    println!("model_api_key_configured: {}", !api_key.is_empty());
    println!("embedding_provider: {}", embedding.provider);
    println!(
        "embedding_model: {}",
        display_optional_model(embedding.model)
    );
    println!("next: ikaros config validate");
    println!("next: ikaros doctor");
    Ok(())
}

pub(crate) fn doctor(
    _args: DoctorArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let report = runtime_doctor_report(paths, workspace, agent_override)?;
    println!("Ikaros doctor");
    print_doctor_report(&report);
    Ok(())
}
