// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use clap::Subcommand;
use ikaros_core::{IkarosConfig, IkarosPaths, redact_secrets};
use ikaros_models::{
    ModelRequest, ProviderHealthLedger, ProviderRegistry,
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
}

pub(crate) async fn provider_command(
    command: ProviderCommand,
    paths: &IkarosPaths,
    workspace: &Path,
) -> Result<()> {
    match command {
        ProviderCommand::Inspect => inspect_provider(paths),
        ProviderCommand::Health { live } => provider_health(paths, workspace, live).await,
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
