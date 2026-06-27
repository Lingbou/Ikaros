// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result, bail};
use clap::Args;
use ikaros_core::{ConfigValidationReport, IkarosConfig, IkarosPaths};
use ikaros_runtime::{
    RuntimeDoctorReport, RuntimeInitReport, initialize_runtime_home,
    initialize_runtime_home_with_options, runtime_doctor_report,
};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

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

fn config_has_setup_paths(path: &Path) -> Result<bool> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config: {}", path.display()))?;
    Ok(raw.contains("\nproviders:\n")
        && raw.contains("\n  embedding:\n")
        && raw.contains("\nvoice:\n")
        && raw.contains("\n  tts:\n")
        && raw.contains("\n  asr:\n"))
}

fn prompt_setup_args(args: &mut SetupArgs) -> Result<()> {
    println!("Ikaros interactive setup");
    println!("Press Enter to keep the value in brackets.");
    args.provider = prompt_with_default(
        "Model provider (openai-compatible, anthropic, ollama, mock)",
        Some(&args.provider),
    )?
    .unwrap_or_else(|| args.provider.clone());
    let provider = normalize_provider(&args.provider)?;
    if !matches!(provider, "mock" | "ollama") && args.api_key.is_none() {
        args.api_key = prompt_required("Model API key")?;
    }
    if provider != "mock" && args.base_url.is_none() {
        let default = (provider == "ollama").then_some("http://127.0.0.1:11434");
        args.base_url = prompt_with_default("Model base URL", default)?;
    }
    if args.model.is_none() && provider != "mock" {
        args.model = prompt_required("Model id")?;
    }
    if provider == "openai-compatible" && args.compat_profile.is_none() {
        args.compat_profile = prompt_with_default(
            "OpenAI-compatible profile (auto, generic, moonshot-kimi, deepseek, gemini-openai, openrouter, qwen, local-openai-compatible)",
            Some("auto"),
        )?;
    }
    if args.daily_token_budget.is_none()
        && let Some(value) = prompt_with_default("Daily token budget (blank for none)", None)?
    {
        args.daily_token_budget = Some(value.parse().with_context(|| {
            format!("invalid daily token budget `{value}`; expected an integer")
        })?);
    }
    let model_api_key = args.api_key.clone();
    let model_base_url = args.base_url.clone();
    prompt_remote_resource(
        "Embedding",
        model_api_key.as_deref(),
        model_base_url.as_deref(),
        &mut args.embedding_api_key,
        &mut args.embedding_base_url,
        &mut args.embedding_model,
    )?;
    if prompt_remote_resource(
        "TTS",
        model_api_key.as_deref(),
        model_base_url.as_deref(),
        &mut args.tts_api_key,
        &mut args.tts_base_url,
        &mut args.tts_model,
    )? {
        args.tts_voice = prompt_with_default("TTS voice (blank for provider default)", None)?;
    }
    prompt_remote_resource(
        "ASR",
        model_api_key.as_deref(),
        model_base_url.as_deref(),
        &mut args.asr_api_key,
        &mut args.asr_base_url,
        &mut args.asr_model,
    )?;
    if args.search_api_key.is_none()
        && args.search_base_url.is_none()
        && prompt_yes_no("Configure web search provider credentials?", false)?
    {
        args.search_api_key = prompt_required("Search API key")?;
        args.search_base_url =
            prompt_with_default("Search base URL (blank for provider default)", None)?;
    }
    Ok(())
}

fn prompt_remote_resource(
    label: &str,
    model_api_key: Option<&str>,
    model_base_url: Option<&str>,
    api_key: &mut Option<String>,
    base_url: &mut Option<String>,
    model: &mut Option<String>,
) -> Result<bool> {
    if api_key.is_some() || base_url.is_some() || model.is_some() {
        return Ok(false);
    }
    if !prompt_yes_no(&format!("Configure remote {label} provider?"), false)? {
        return Ok(false);
    }
    if model_api_key.is_some_and(|value| !value.trim().is_empty())
        && model_base_url.is_some_and(|value| !value.trim().is_empty())
        && prompt_yes_no(
            &format!("Reuse model provider API key and base URL for {label}?"),
            true,
        )?
    {
        *api_key = model_api_key.map(ToOwned::to_owned);
        *base_url = model_base_url.map(ToOwned::to_owned);
    } else {
        *api_key = prompt_required(&format!("{label} API key"))?;
        *base_url = prompt_required(&format!("{label} base URL"))?;
    }
    *model = prompt_required(&format!("{label} model"))?;
    Ok(true)
}

fn prompt_required(label: &str) -> Result<Option<String>> {
    let Some(value) = prompt_with_default(label, None)? else {
        bail!("{label} is required");
    };
    Ok(Some(value))
}

fn prompt_with_default(label: &str, default: Option<&str>) -> Result<Option<String>> {
    match default.filter(|value| !value.trim().is_empty()) {
        Some(default) => print!("{label} [{default}]: "),
        None => print!("{label}: "),
    }
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let value = line.trim();
    if value.is_empty() {
        return Ok(default.map(ToOwned::to_owned));
    }
    Ok(Some(value.to_owned()))
}

fn prompt_yes_no(label: &str, default: bool) -> Result<bool> {
    let default_label = if default { "Y/n" } else { "y/N" };
    loop {
        print!("{label} [{default_label}]: ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        match line.trim().to_ascii_lowercase().as_str() {
            "" => return Ok(default),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("Please answer y or n."),
        }
    }
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

fn print_init_report(report: &RuntimeInitReport) {
    println!("home: {}", report.home.display());
    println!(
        "config: {} ({})",
        report.config.display(),
        if report.config_created {
            "created"
        } else {
            "existing"
        }
    );
    println!(
        "persona_dir: {} ({})",
        report.persona_dir.display(),
        if report.persona_created {
            "created"
        } else {
            "existing"
        }
    );
    println!("persona_profile: {}", report.persona_profile.display());
    println!("memory: {}", report.memory_dir.display());
    println!("rag: {}", report.rag_dir.display());
    println!("automation: {}", report.automation_dir.display());
    println!("gateway: {}", report.gateway_dir.display());
    println!("audit: {}", report.audit_dir.display());
}

fn print_doctor_report(report: &RuntimeDoctorReport) {
    println!("home: {}", report.home.display());
    println!("workspace: {}", report.workspace.display());
    println!("config_schema_version: {}", report.config.schema_version);
    println!("config_valid: {}", report.config.valid);
    for issue in &report.config.issues {
        println!(
            "config_issue: {}: {}: {}",
            issue.severity, issue.path, issue.message
        );
    }
    println!("persona: {} ({})", report.persona.name, report.persona.role);
    println!(
        "agent: {} mode={} writes={} shell={} network={}",
        report.agent.name,
        report.agent.mode,
        report.agent.workspace_writes,
        report.agent.shell,
        report.agent.network
    );
    println!("agent_profiles: {}", report.agent_profiles.join(", "));
    println!("emotion: {}", report.emotion);
    println!(
        "model: provider={} model={} key_configured={}",
        report.model.provider, report.model.model, report.model.api_key_configured
    );
    println!(
        "model_limits: rate_limit_per_minute={:?} daily_token_budget={:?} daily_token_used_today={} daily_token_remaining_today={:?} daily_token_budget_status={}",
        report.model.rate_limit_per_minute,
        report.model.daily_token_budget,
        report.model.daily_token_used_today,
        report.model.daily_token_remaining_today,
        report.model.daily_token_budget_status
    );
    println!("model_usage: {}", report.model_usage_path.display());
    println!(
        "execution: sandbox_backend={} sandbox_image={} read_scope={} network_enabled={} allow_provider_hosts={} allowed_hosts={} network_timeout_ms={}",
        report.execution.sandbox_backend,
        display_optional_config_value(&report.execution.sandbox_image),
        report.execution.sandbox_read_scope,
        report.execution.network_enabled,
        report.execution.allow_provider_hosts,
        report.execution.allowed_hosts,
        report.execution.network_timeout_ms
    );
    println!(
        "memory: backend={} path={}",
        report.memory.backend,
        report.memory.path.display()
    );
    let active_external = report
        .memory_providers
        .active_external()
        .map(|provider| provider.id.as_str())
        .unwrap_or("none");
    println!(
        "memory_providers: local={} external_active={} external_configured={} issues={}",
        report.memory_providers.active_local.id,
        active_external,
        report.memory_providers.external.len(),
        report.memory_providers.issues.len()
    );
    for issue in &report.memory_providers.issues {
        println!("memory_provider_issue: {issue}");
    }
    println!(
        "rag: backend={} embedding_provider={} embedding_model={} embedding_key_configured={} embedding_base_url_configured={} embedding_uses_network={} embedding_egress={} path={}",
        report.rag.backend,
        report.rag.embedding_provider,
        report.rag.embedding_model,
        report.rag.embedding_api_key_configured,
        report.rag.embedding_base_url_configured,
        report.rag.embedding_uses_network,
        report.rag.embedding_egress,
        report.rag.path.display()
    );
    println!(
        "voice: tts_provider={} tts_model={} asr_provider={} asr_model={}",
        report.voice.tts_provider,
        report.voice.tts_model,
        report.voice.asr_provider,
        report.voice.asr_model
    );
    println!(
        "automation: schedules={}",
        report.automation.schedules_path.display()
    );
    println!(
        "gateway: inbox={} outbox={}",
        report.gateway.inbox_path.display(),
        report.gateway.outbox_path.display()
    );
    println!("skills: {}", report.skills.join(", "));
    println!(
        "plugins: {} plugin(s), {} enabled, {} disabled, {} active declared skill(s), {} warning(s)",
        report.plugins.plugin_count,
        report.plugins.enabled_plugin_count,
        report.plugins.disabled_plugin_count,
        report.plugins.active_declared_skill_count,
        report.plugins.warning_count
    );
    println!("audit: {}", report.audit_path.display());
}

fn display_optional_config_value(value: &str) -> &str {
    if value.trim().is_empty() {
        "none"
    } else {
        value
    }
}

#[derive(Debug, Clone, Copy)]
struct SetupResource<'a> {
    provider: &'a str,
    api_key: &'a str,
    base_url: &'a str,
    model: &'a str,
}

fn normalize_provider(provider: &str) -> Result<&'static str> {
    match provider.trim() {
        "mock" => Ok("mock"),
        "openai-compatible" => Ok("openai-compatible"),
        "anthropic" => Ok("anthropic"),
        "ollama" => Ok("ollama"),
        other => bail!(
            "unsupported setup provider `{other}`; expected openai-compatible, anthropic, ollama, or mock"
        ),
    }
}

fn model_transport_for_provider(provider: &str) -> &'static str {
    match provider {
        "mock" => "mock",
        "anthropic" => "anthropic-messages",
        "ollama" => "ollama-chat",
        _ => "openai-compatible-chat-completions",
    }
}

fn model_for_setup(provider: &str, model: Option<&str>) -> Result<String> {
    match (
        provider,
        model.map(str::trim).filter(|value| !value.is_empty()),
    ) {
        ("mock", None) => Ok("mock-ikaros".into()),
        (_, Some(model)) => Ok(model.into()),
        _ => bail!("--model is required for provider `{provider}`"),
    }
}

fn model_api_key_for_setup<'a>(provider: &str, api_key: Option<&'a str>) -> Result<&'a str> {
    let api_key = api_key.map(str::trim).unwrap_or("");
    if matches!(provider, "mock" | "ollama") {
        return Ok(api_key);
    }
    if api_key.is_empty() {
        bail!("--api-key is required for provider `{provider}`");
    }
    Ok(api_key)
}

fn model_base_url_for_setup<'a>(provider: &str, base_url: Option<&'a str>) -> Result<&'a str> {
    let base_url = base_url.map(str::trim).unwrap_or("");
    if provider == "mock" {
        return Ok(base_url);
    }
    if provider == "ollama" {
        return Ok(if base_url.is_empty() {
            "http://127.0.0.1:11434"
        } else {
            base_url
        });
    }
    if base_url.is_empty() {
        bail!("--base-url is required for provider `{provider}`");
    }
    Ok(base_url)
}

fn setup_compat_profile(provider: &str, compat_profile: Option<&str>) -> String {
    match (provider, compat_profile) {
        ("openai-compatible", Some(profile)) => profile.trim().to_owned(),
        ("openai-compatible", None) => "auto".into(),
        (_, Some(profile)) if matches!(profile.trim(), "auto" | "generic") => {
            profile.trim().to_owned()
        }
        _ => "generic".into(),
    }
}

fn apply_reused_model_provider_resources(
    args: &mut SetupArgs,
    model_api_key: &str,
    model_base_url: &str,
) -> Result<()> {
    if args.reuse_model_provider_for_embedding {
        reuse_model_provider_resource(
            "embedding",
            model_api_key,
            model_base_url,
            &mut args.embedding_api_key,
            &mut args.embedding_base_url,
        )?;
    }
    if args.reuse_model_provider_for_tts {
        reuse_model_provider_resource(
            "tts",
            model_api_key,
            model_base_url,
            &mut args.tts_api_key,
            &mut args.tts_base_url,
        )?;
    }
    if args.reuse_model_provider_for_asr {
        reuse_model_provider_resource(
            "asr",
            model_api_key,
            model_base_url,
            &mut args.asr_api_key,
            &mut args.asr_base_url,
        )?;
    }
    Ok(())
}

fn reuse_model_provider_resource(
    label: &str,
    model_api_key: &str,
    model_base_url: &str,
    api_key: &mut Option<String>,
    base_url: &mut Option<String>,
) -> Result<()> {
    if model_api_key.trim().is_empty() || model_base_url.trim().is_empty() {
        bail!(
            "--reuse-model-provider-for-{label} requires a configured model --api-key and --base-url"
        );
    }
    if api_key
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || base_url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        bail!(
            "--reuse-model-provider-for-{label} cannot be combined with explicit --{label}-api-key or --{label}-base-url"
        );
    }
    *api_key = Some(model_api_key.trim().to_owned());
    *base_url = Some(model_base_url.trim().to_owned());
    Ok(())
}

fn setup_embedding(args: &SetupArgs) -> Result<SetupResource<'_>> {
    match (
        args.embedding_api_key.as_deref(),
        args.embedding_base_url.as_deref(),
        args.embedding_model.as_deref(),
    ) {
        (None, None, None) => Ok(SetupResource {
            provider: "hash",
            api_key: "",
            base_url: "",
            model: "",
        }),
        (Some(api_key), Some(base_url), Some(model))
            if !api_key.trim().is_empty()
                && !base_url.trim().is_empty()
                && !model.trim().is_empty() =>
        {
            Ok(SetupResource {
                provider: "openai-compatible",
                api_key: api_key.trim(),
                base_url: base_url.trim(),
                model: model.trim(),
            })
        }
        _ => bail!(
            "--embedding-api-key, --embedding-base-url, and --embedding-model must be provided together"
        ),
    }
}

fn setup_voice<'a>(
    label: &str,
    api_key: Option<&'a str>,
    base_url: Option<&'a str>,
    model: Option<&'a str>,
    mock_model: &'a str,
) -> Result<SetupResource<'a>> {
    match (api_key, base_url, model) {
        (None, None, None) => Ok(SetupResource {
            provider: "mock",
            api_key: "",
            base_url: "",
            model: mock_model,
        }),
        (Some(api_key), Some(base_url), Some(model))
            if !api_key.trim().is_empty()
                && !base_url.trim().is_empty()
                && !model.trim().is_empty() =>
        {
            Ok(SetupResource {
                provider: "openai-compatible",
                api_key: api_key.trim(),
                base_url: base_url.trim(),
                model: model.trim(),
            })
        }
        _ => bail!(
            "--{label}-api-key, --{label}-base-url, and --{label}-model must be provided together"
        ),
    }
}

fn display_optional_model(model: &str) -> &str {
    if model.is_empty() { "none" } else { model }
}

fn resource_reuses_model_provider(
    resource: &SetupResource<'_>,
    model_api_key: &str,
    model_base_url: &str,
) -> bool {
    !resource.api_key.trim().is_empty()
        && resource.api_key == model_api_key.trim()
        && resource.base_url == model_base_url.trim()
}

fn format_setup_validation_failure(summary: &str, report: &ConfigValidationReport) -> String {
    let mut message = summary.to_owned();
    for issue in &report.errors {
        message.push_str(&format!("\nerror: {}: {}", issue.path, issue.message));
    }
    for issue in &report.warnings {
        message.push_str(&format!("\nwarning: {}: {}", issue.path, issue.message));
    }
    message
}

fn set_yaml_scalar(raw: String, path: &[&str], value: &str) -> Result<String> {
    set_yaml_scalar_raw(raw, path, &serde_json::to_string(value)?)
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
