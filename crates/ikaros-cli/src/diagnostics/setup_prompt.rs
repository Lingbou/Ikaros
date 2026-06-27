// SPDX-License-Identifier: GPL-3.0-only

use super::{SetupArgs, setup_resources::normalize_provider};
use anyhow::{Context, Result, bail};
use std::io::{self, Write};

pub(super) fn prompt_setup_args(args: &mut SetupArgs) -> Result<()> {
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
