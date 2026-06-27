// SPDX-License-Identifier: GPL-3.0-only

use super::SetupArgs;
use anyhow::{Result, bail};

#[derive(Debug, Clone, Copy)]
pub(super) struct SetupResource<'a> {
    pub(super) provider: &'a str,
    pub(super) api_key: &'a str,
    pub(super) base_url: &'a str,
    pub(super) model: &'a str,
}

pub(super) fn normalize_provider(provider: &str) -> Result<&'static str> {
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

pub(super) fn model_transport_for_provider(provider: &str) -> &'static str {
    match provider {
        "mock" => "mock",
        "anthropic" => "anthropic-messages",
        "ollama" => "ollama-chat",
        _ => "openai-compatible-chat-completions",
    }
}

pub(super) fn model_for_setup(provider: &str, model: Option<&str>) -> Result<String> {
    match (
        provider,
        model.map(str::trim).filter(|value| !value.is_empty()),
    ) {
        ("mock", None) => Ok("mock-ikaros".into()),
        (_, Some(model)) => Ok(model.into()),
        _ => bail!("--model is required for provider `{provider}`"),
    }
}

pub(super) fn model_api_key_for_setup<'a>(
    provider: &str,
    api_key: Option<&'a str>,
) -> Result<&'a str> {
    let api_key = api_key.map(str::trim).unwrap_or("");
    if matches!(provider, "mock" | "ollama") {
        return Ok(api_key);
    }
    if api_key.is_empty() {
        bail!("--api-key is required for provider `{provider}`");
    }
    Ok(api_key)
}

pub(super) fn model_base_url_for_setup<'a>(
    provider: &str,
    base_url: Option<&'a str>,
) -> Result<&'a str> {
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

pub(super) fn setup_compat_profile(provider: &str, compat_profile: Option<&str>) -> String {
    match (provider, compat_profile) {
        ("openai-compatible", Some(profile)) => profile.trim().to_owned(),
        ("openai-compatible", None) => "auto".into(),
        (_, Some(profile)) if matches!(profile.trim(), "auto" | "generic") => {
            profile.trim().to_owned()
        }
        _ => "generic".into(),
    }
}

pub(super) fn apply_reused_model_provider_resources(
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

pub(super) fn setup_embedding(args: &SetupArgs) -> Result<SetupResource<'_>> {
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

pub(super) fn setup_voice<'a>(
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
