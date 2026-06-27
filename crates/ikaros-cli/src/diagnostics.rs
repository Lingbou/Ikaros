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
use output::{
    display_optional_model, print_doctor_report, print_init_report, resource_reuses_model_provider,
};
use setup_prompt::prompt_setup_args;
use setup_resources::{
    apply_reused_model_provider_resources, model_api_key_for_setup, model_base_url_for_setup,
    model_for_setup, model_transport_for_provider, normalize_provider, setup_compat_profile,
    setup_embedding, setup_voice,
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
    /// Remote TTS API key. When omitted, setup configures mock TTS.
    #[arg(long)]
    tts_api_key: Option<String>,
    /// Remote TTS base URL. Required with --tts-api-key.
    #[arg(long)]
    tts_base_url: Option<String>,
    /// Remote TTS model. Required with --tts-api-key.
    #[arg(long)]
    tts_model: Option<String>,
    /// Default TTS voice.
    #[arg(long)]
    tts_voice: Option<String>,
    /// Reuse the model provider API key and base URL for remote TTS.
    #[arg(long)]
    reuse_model_provider_for_tts: bool,
    /// Remote ASR API key. When omitted, setup configures mock ASR.
    #[arg(long)]
    asr_api_key: Option<String>,
    /// Remote ASR base URL. Required with --asr-api-key.
    #[arg(long)]
    asr_base_url: Option<String>,
    /// Remote ASR model. Required with --asr-api-key.
    #[arg(long)]
    asr_model: Option<String>,
    /// Reuse the model provider API key and base URL for remote ASR.
    #[arg(long)]
    reuse_model_provider_for_asr: bool,
    /// Web search API key. Used by web_search providers such as brave, bing, serpapi, or tavily.
    #[arg(long)]
    search_api_key: Option<String>,
    /// Web search endpoint. Leave empty to use the selected provider default.
    #[arg(long)]
    search_base_url: Option<String>,
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

    let tts = setup_voice(
        "tts",
        args.tts_api_key.as_deref(),
        args.tts_base_url.as_deref(),
        args.tts_model.as_deref(),
        "mock-tts",
    )?;
    raw = set_yaml_scalar(raw, &["providers", "tts", "api_key"], tts.api_key)?;
    raw = set_yaml_scalar(raw, &["providers", "tts", "base_url"], tts.base_url)?;
    raw = set_yaml_scalar(raw, &["voice", "tts", "provider"], tts.provider)?;
    raw = set_yaml_scalar(raw, &["voice", "tts", "model"], tts.model)?;
    if let Some(voice) = args.tts_voice.as_deref() {
        raw = set_yaml_scalar(raw, &["voice", "tts", "voice"], voice)?;
    }

    let asr = setup_voice(
        "asr",
        args.asr_api_key.as_deref(),
        args.asr_base_url.as_deref(),
        args.asr_model.as_deref(),
        "mock-asr",
    )?;
    raw = set_yaml_scalar(raw, &["providers", "asr", "api_key"], asr.api_key)?;
    raw = set_yaml_scalar(raw, &["providers", "asr", "base_url"], asr.base_url)?;
    raw = set_yaml_scalar(raw, &["voice", "asr", "provider"], asr.provider)?;
    raw = set_yaml_scalar(raw, &["voice", "asr", "model"], asr.model)?;

    if let Some(search_api_key) = args.search_api_key.as_deref() {
        raw = set_yaml_scalar(raw, &["providers", "search", "api_key"], search_api_key)?;
    }
    if let Some(search_base_url) = args.search_base_url.as_deref() {
        raw = set_yaml_scalar(raw, &["providers", "search", "base_url"], search_base_url)?;
    }

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
    println!(
        "embedding_reuses_model_provider: {}",
        resource_reuses_model_provider(&embedding, &api_key, &base_url)
    );
    println!("tts_provider: {}", tts.provider);
    println!("tts_model: {}", display_optional_model(tts.model));
    println!(
        "tts_reuses_model_provider: {}",
        resource_reuses_model_provider(&tts, &api_key, &base_url)
    );
    println!("asr_provider: {}", asr.provider);
    println!("asr_model: {}", display_optional_model(asr.model));
    println!(
        "asr_reuses_model_provider: {}",
        resource_reuses_model_provider(&asr, &api_key, &base_url)
    );
    println!(
        "search_api_key_configured: {}",
        args.search_api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    );
    println!(
        "search_base_url_configured: {}",
        args.search_base_url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
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
