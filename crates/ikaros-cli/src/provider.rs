// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use clap::Subcommand;
use ikaros_core::{IkarosConfig, IkarosPaths, redact_secrets};
use ikaros_models::{
    ModelProviderDescriptor, ModelRequest, ProviderHealthLedger, ProviderRegistry,
    governed_provider_from_config_with_http_client,
};
use ikaros_runtime::{EgressModelHttpClient, runtime_execution_env};
use std::{path::Path, sync::Arc};

#[derive(Debug, Subcommand)]
pub(crate) enum ProviderCommand {
    Inspect,
    Health {
        #[arg(long)]
        live: bool,
    },
    Matrix {
        #[arg(long)]
        live: bool,
    },
}

pub(crate) async fn provider_command(
    command: ProviderCommand,
    paths: &IkarosPaths,
    workspace: &Path,
) -> Result<()> {
    match command {
        ProviderCommand::Inspect => inspect_provider(paths),
        ProviderCommand::Health { live } => provider_health(paths, workspace, live).await,
        ProviderCommand::Matrix { live } => provider_matrix(paths, workspace, live).await,
    }
}

fn inspect_provider(paths: &IkarosPaths) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let model = &config.model.default;
    let descriptor = ProviderRegistry.descriptor(
        &model.provider,
        &config.providers.model.base_url,
        &model.model,
    )?;

    println!("provider: {}", descriptor.provider);
    println!("model: {}", redact_secrets(&descriptor.model));
    println!("profile: {}", descriptor.profile);
    println!("context_window: {}", descriptor.context.context_window);
    println!(
        "default_output_tokens: {}",
        descriptor.context.default_output_tokens
    );
    println!("tokenizer: {:?}", descriptor.context.tokenizer);
    println!("streaming: {}", descriptor.capabilities.streaming);
    println!("tool_calls: {}", descriptor.capabilities.tool_calls);
    println!("reasoning: {}", descriptor.capabilities.reasoning);
    println!("json_mode: {}", descriptor.capabilities.json_mode);
    println!("network: {}", descriptor.capabilities.network);
    println!("health: {:?}", descriptor.health.status);
    if let Some(input) = descriptor.cost.input_per_million {
        println!(
            "cost_input_per_million: {} {}",
            input, descriptor.cost.currency
        );
    } else {
        println!("cost_input_per_million: unknown");
    }
    if let Some(output) = descriptor.cost.output_per_million {
        println!(
            "cost_output_per_million: {} {}",
            output, descriptor.cost.currency
        );
    } else {
        println!("cost_output_per_million: unknown");
    }
    Ok(())
}

async fn provider_matrix(paths: &IkarosPaths, workspace: &Path, live: bool) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let registry = ProviderRegistry;
    let model_live_probe = if live {
        provider_live_probe(paths, workspace, &config).await
    } else {
        "not-run".into()
    };
    let other_live_probe = if live { "not-supported" } else { "not-run" };
    println!("provider_matrix: live={live}");
    print_matrix_row(
        &registry,
        "model",
        &config.model.default.provider,
        &config.model.default.model,
        &config.providers.model.base_url,
        &config.providers.model.api_key,
        &model_live_probe,
    );
    print_matrix_row(
        &registry,
        "embedding",
        &config.rag.embedding_provider,
        &config.rag.embedding_model,
        &config.providers.embedding.base_url,
        &config.providers.embedding.api_key,
        other_live_probe,
    );
    print_matrix_row(
        &registry,
        "tts",
        &config.voice.tts.provider,
        &config.voice.tts.model,
        &config.providers.tts.base_url,
        &config.providers.tts.api_key,
        other_live_probe,
    );
    print_matrix_row(
        &registry,
        "asr",
        &config.voice.asr.provider,
        &config.voice.asr.model,
        &config.providers.asr.base_url,
        &config.providers.asr.api_key,
        other_live_probe,
    );
    Ok(())
}

async fn provider_live_probe(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
) -> String {
    let Ok(env) = runtime_execution_env(config, workspace) else {
        return "failed".into();
    };
    let provider = governed_provider_from_config_with_http_client(
        &config.model.default,
        &config.providers.model,
        &paths.audit_dir,
        Some(Arc::new(EgressModelHttpClient::new(env))),
    );
    let Ok(provider) = provider else {
        return "failed".into();
    };
    match provider
        .generate(ModelRequest::from_user_text(
            "Ikaros provider matrix live probe. Reply with ok.",
        ))
        .await
    {
        Ok(_) => "ok".into(),
        Err(_) => "failed".into(),
    }
}

fn print_matrix_row(
    registry: &ProviderRegistry,
    kind: &str,
    provider: &str,
    model: &str,
    base_url: &str,
    api_key: &str,
    live_probe: &str,
) {
    let descriptor = registry.descriptor(provider, base_url, model).ok();
    let provider = redact_secrets(provider);
    let model = redact_secrets(model);
    let base_url_configured = !base_url.trim().is_empty();
    let api_key_configured = !api_key.trim().is_empty();
    let live_smoke = live_smoke_state(&provider, &model, base_url_configured, api_key_configured);
    println!(
        "matrix_row: kind={} provider={} model={} base_url_configured={} api_key_configured={} live_smoke={} live_probe={} provider_profile={} context_window={} default_output_tokens={} tokenizer={} streaming={} tool_calls={} reasoning={} json_mode={} network={} cost_input_per_million={} cost_output_per_million={} cost_currency={}",
        redact_secrets(kind),
        provider,
        model,
        base_url_configured,
        api_key_configured,
        live_smoke,
        redact_secrets(live_probe),
        matrix_profile(&descriptor),
        matrix_context_window(&descriptor),
        matrix_default_output_tokens(&descriptor),
        matrix_tokenizer(&descriptor),
        matrix_capability(&descriptor, |descriptor| descriptor.capabilities.streaming),
        matrix_capability(&descriptor, |descriptor| descriptor.capabilities.tool_calls),
        matrix_capability(&descriptor, |descriptor| descriptor.capabilities.reasoning),
        matrix_capability(&descriptor, |descriptor| descriptor.capabilities.json_mode),
        matrix_capability(&descriptor, |descriptor| descriptor.capabilities.network),
        matrix_cost(&descriptor, |descriptor| descriptor.cost.input_per_million),
        matrix_cost(&descriptor, |descriptor| descriptor.cost.output_per_million),
        descriptor
            .as_ref()
            .map(|descriptor| descriptor.cost.currency.as_str())
            .unwrap_or("unknown")
    );
}

fn matrix_profile(descriptor: &Option<ModelProviderDescriptor>) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| redact_secrets(&descriptor.profile))
        .unwrap_or_else(|| "unknown".into())
}

fn matrix_context_window(descriptor: &Option<ModelProviderDescriptor>) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| descriptor.context.context_window.to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn matrix_default_output_tokens(descriptor: &Option<ModelProviderDescriptor>) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| descriptor.context.default_output_tokens.to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn matrix_tokenizer(descriptor: &Option<ModelProviderDescriptor>) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| format!("{:?}", descriptor.context.tokenizer))
        .unwrap_or_else(|| "unknown".into())
}

fn matrix_capability(
    descriptor: &Option<ModelProviderDescriptor>,
    read: impl FnOnce(&ModelProviderDescriptor) -> bool,
) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| read(descriptor).to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn matrix_cost(
    descriptor: &Option<ModelProviderDescriptor>,
    read: impl FnOnce(&ModelProviderDescriptor) -> Option<f64>,
) -> String {
    descriptor
        .as_ref()
        .and_then(read)
        .map(|cost| cost.to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn live_smoke_state(
    provider: &str,
    model: &str,
    base_url_configured: bool,
    api_key_configured: bool,
) -> &'static str {
    match provider {
        "mock" | "hash" => "offline",
        "ollama" => {
            if model.trim().is_empty() {
                "missing-model"
            } else {
                "local-ready"
            }
        }
        _ if model.trim().is_empty() => "missing-model",
        _ if !base_url_configured => "missing-base-url",
        _ if !api_key_configured => "missing-api-key",
        _ => "ready",
    }
}

async fn provider_health(paths: &IkarosPaths, workspace: &Path, live: bool) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    if live {
        let env = runtime_execution_env(&config, workspace)?;
        let provider = governed_provider_from_config_with_http_client(
            &config.model.default,
            &config.providers.model,
            &paths.audit_dir,
            Some(Arc::new(EgressModelHttpClient::new(env))),
        )?;
        match provider
            .generate(ModelRequest::from_user_text(
                "Ikaros provider health probe. Reply with a short ok.",
            ))
            .await
        {
            Ok(response) => {
                println!("live: ok");
                println!("provider: {}", response.provider);
                println!("model: {}", redact_secrets(&response.model));
                println!(
                    "usage_total: {}",
                    response.usage.total_or_prompt_completion()
                );
                return Ok(());
            }
            Err(error) => {
                println!("live: failed");
                println!("error: {}", redact_secrets(&error.to_string()));
                return Ok(());
            }
        }
    }

    let ledger = ProviderHealthLedger::new(&paths.audit_dir);
    let model = &config.model.default;
    let latest = ledger.latest(&model.provider, &model.model)?;
    println!("provider: {}", model.provider);
    println!("model: {}", redact_secrets(&model.model));
    if let Some(record) = latest {
        println!("health: {:?}", record.status);
        println!("consecutive_failures: {}", record.consecutive_failures);
        if let Some(kind) = record.last_error_kind {
            println!("last_error_kind: {:?}", kind);
        }
        if !record.last_error_summary.is_empty() {
            println!("last_error: {}", redact_secrets(&record.last_error_summary));
        }
        if let Some(cooldown_until) = record.cooldown_until {
            println!("cooldown_until: {cooldown_until}");
        }
    } else {
        println!("health: Unknown");
    }
    println!("health_log: {}", ledger.path().display());
    Ok(())
}
