// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use clap::Subcommand;
use ikaros_core::IkarosPaths;
use ikaros_host::{
    ProviderHealthReport, ProviderMatrixReport, ProviderMatrixRow, provider_health_report,
    provider_inspect_report, provider_matrix_report, provider_profiles_report,
};
use std::path::Path;

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
        #[arg(long)]
        json: bool,
    },
    Profiles,
}

pub(crate) async fn provider_command(
    command: ProviderCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    match command {
        ProviderCommand::Inspect => inspect_provider(paths, workspace, agent_override),
        ProviderCommand::Health { live } => {
            provider_health(paths, workspace, agent_override, live).await
        }
        ProviderCommand::Matrix { live, json } => {
            provider_matrix(paths, workspace, agent_override, live, json).await
        }
        ProviderCommand::Profiles => provider_profiles(),
    }
}

fn inspect_provider(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let report = provider_inspect_report(paths, workspace, agent_override)?;
    println!("provider: {}", report.provider);
    println!("model: {}", report.model);
    println!("configured_profile: {}", report.configured_profile);
    println!("profile: {}", report.profile);
    println!("profile_source: {}", report.profile_source);
    println!("temperature_policy: {}", report.temperature_policy);
    println!("reasoning_policy: {}", report.reasoning_policy);
    println!("message_policy: {}", report.message_policy);
    println!("tool_schema_policy: {}", report.tool_schema_policy);
    println!("request_body_policy: {}", report.request_body_policy);
    println!(
        "retry_without_parameters: {}",
        report.retry_without_parameters
    );
    println!("fallback_count: {}", report.fallback_rows.len());
    for row in report.fallback_rows {
        println!(
            "fallback_row: index={} provider={} model={} configured_profile={} profile={} live_smoke={} streaming={} tool_calls={} reasoning={} network={} image_input={} audio_input={} file_input={} context_window={} default_output_tokens={}",
            row.index,
            row.provider,
            row.model,
            row.configured_profile,
            row.profile,
            row.live_smoke,
            row.streaming,
            row.tool_calls,
            row.reasoning,
            row.network,
            row.image_input,
            row.audio_input,
            row.file_input,
            row.context_window,
            row.default_output_tokens,
        );
    }
    println!("context_window: {}", report.context_window);
    println!("default_output_tokens: {}", report.default_output_tokens);
    println!("tokenizer: {}", report.tokenizer);
    println!("streaming: {}", report.streaming);
    println!("tool_calls: {}", report.tool_calls);
    println!("reasoning: {}", report.reasoning);
    println!("json_mode: {}", report.json_mode);
    println!("network: {}", report.network);
    println!("image_input: {}", report.image_input);
    println!("audio_input: {}", report.audio_input);
    println!("file_input: {}", report.file_input);
    println!("health: {}", report.health);
    if let Some(input) = report.cost_input_per_million {
        println!("cost_input_per_million: {} {}", input, report.cost_currency);
    } else {
        println!("cost_input_per_million: unknown");
    }
    if let Some(output) = report.cost_output_per_million {
        println!(
            "cost_output_per_million: {} {}",
            output, report.cost_currency
        );
    } else {
        println!("cost_output_per_million: unknown");
    }
    if let Some(cache_read) = report.cost_cache_read_per_million {
        println!(
            "cost_cache_read_per_million: {} {}",
            cache_read, report.cost_currency
        );
    } else {
        println!("cost_cache_read_per_million: unknown");
    }
    if let Some(cache_write) = report.cost_cache_write_per_million {
        println!(
            "cost_cache_write_per_million: {} {}",
            cache_write, report.cost_currency
        );
    } else {
        println!("cost_cache_write_per_million: unknown");
    }
    Ok(())
}

fn provider_profiles() -> Result<()> {
    let report = provider_profiles_report();
    println!("provider_profiles: {}", report.provider);
    println!("profile_count: {}", report.rows.len());
    for profile in report.rows {
        println!(
            "profile_row: provider={} profile={} auto_base_url_markers={} auto_model_markers={} auto_model_tail_prefixes={} temperature_policy={} reasoning_policy={} message_policy={} tool_schema_policy={} request_body_policy={} retry_without_parameters={} context_window={} default_output_tokens={} tokenizer={} streaming={} tool_calls={} reasoning={} json_mode={} network={} image_input={} audio_input={} file_input={}",
            profile.provider,
            profile.profile,
            profile.auto_base_url_markers,
            profile.auto_model_markers,
            profile.auto_model_tail_prefixes,
            profile.temperature_policy,
            profile.reasoning_policy,
            profile.message_policy,
            profile.tool_schema_policy,
            profile.request_body_policy,
            profile.retry_without_parameters,
            profile.context_window,
            profile.default_output_tokens,
            profile.tokenizer,
            profile.streaming,
            profile.tool_calls,
            profile.reasoning,
            profile.json_mode,
            profile.network,
            profile.image_input,
            profile.audio_input,
            profile.file_input,
        );
    }
    Ok(())
}

async fn provider_matrix(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    live: bool,
    json: bool,
) -> Result<()> {
    let report = provider_matrix_report(paths, workspace, agent_override, live).await?;
    if json {
        print_matrix_json(report)?;
        return Ok(());
    }
    println!("provider_matrix: live={live}");
    for row in &report.rows {
        print_matrix_row(row);
    }
    Ok(())
}

fn print_matrix_row(row: &ProviderMatrixRow) {
    println!(
        "matrix_row: kind={} provider={} model={} base_url_configured={} api_key_configured={} live_smoke={} live_probe={} probe_detail={} health_status={} consecutive_failures={} cooldown_until={} configured_profile={} provider_profile={} profile_source={} temperature_policy={} reasoning_policy={} message_policy={} tool_schema_policy={} request_body_policy={} retry_without_parameters={} context_window={} default_output_tokens={} tokenizer={} streaming={} tool_calls={} reasoning={} json_mode={} network={} image_input={} audio_input={} file_input={} cost_input_per_million={} cost_output_per_million={} cost_cache_read_per_million={} cost_cache_write_per_million={} cost_currency={} usage_requests_today={} usage_prompt_tokens_today={} usage_completion_tokens_today={} usage_total_tokens_today={} cache_read_tokens_today={} cache_write_tokens_today={} estimated_cost_today={} cache_accounting={} fallback_role={} fallback_count={} fallback_models={} debug_hint={}",
        row.kind,
        row.provider,
        row.model,
        row.base_url_configured,
        row.api_key_configured,
        row.live_smoke,
        row.live_probe,
        row.probe_detail,
        row.health_status,
        row.consecutive_failures,
        row.cooldown_until.as_deref().unwrap_or("none"),
        row.configured_profile,
        row.provider_profile,
        row.profile_source,
        row.temperature_policy,
        row.reasoning_policy,
        row.message_policy,
        row.tool_schema_policy,
        row.request_body_policy,
        row.retry_without_parameters,
        row.context_window,
        row.default_output_tokens,
        row.tokenizer,
        row.streaming,
        row.tool_calls,
        row.reasoning,
        row.json_mode,
        row.network,
        row.image_input,
        row.audio_input,
        row.file_input,
        row.cost_input_per_million,
        row.cost_output_per_million,
        row.cost_cache_read_per_million,
        row.cost_cache_write_per_million,
        row.cost_currency,
        row.usage_requests_today,
        row.usage_prompt_tokens_today,
        row.usage_completion_tokens_today,
        row.usage_total_tokens_today,
        row.cache_read_tokens_today,
        row.cache_write_tokens_today,
        row.estimated_cost_today,
        row.cache_accounting,
        row.fallback_role,
        row.fallback_count,
        format_fallback_models(&row.fallback_models),
        row.debug_hint,
    );
}

fn print_matrix_json(report: ProviderMatrixReport) -> Result<()> {
    let rows = report.rows.iter().map(matrix_row_json).collect::<Vec<_>>();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema": "ikaros-provider-matrix-v1",
            "version": 1,
            "live": report.live,
            "rows": rows,
        }))?
    );
    Ok(())
}

fn matrix_row_json(row: &ProviderMatrixRow) -> serde_json::Value {
    serde_json::json!({
        "kind": row.kind,
        "provider": row.provider,
        "model": row.model,
        "configured": {
            "base_url": row.base_url_configured,
            "api_key": row.api_key_configured,
            "profile": row.configured_profile,
        },
        "profile": {
            "resolved": row.provider_profile,
            "source": row.profile_source,
            "temperature_policy": row.temperature_policy,
            "reasoning_policy": row.reasoning_policy,
            "message_policy": row.message_policy,
            "tool_schema_policy": row.tool_schema_policy,
            "request_body_policy": row.request_body_policy,
            "retry_without_parameters": row.retry_without_parameters,
        },
        "context": {
            "context_window": row.context_window,
            "default_output_tokens": row.default_output_tokens,
            "tokenizer": row.tokenizer,
        },
        "capabilities": {
            "streaming": row.streaming,
            "tool_calls": row.tool_calls,
            "reasoning": row.reasoning,
            "json_mode": row.json_mode,
            "network": row.network,
            "image_input": row.image_input,
            "audio_input": row.audio_input,
            "file_input": row.file_input,
        },
        "health": {
            "status": row.health_status,
            "consecutive_failures": row.consecutive_failures,
            "cooldown_until": row.cooldown_until,
        },
        "live": {
            "local_readiness": row.live_smoke,
            "probe_status": row.live_probe,
            "probe_detail": row.probe_detail,
            "debug_hint": row.debug_hint,
        },
        "cost": {
            "input_per_million": row.cost_input_per_million,
            "output_per_million": row.cost_output_per_million,
            "cache_read_per_million": row.cost_cache_read_per_million,
            "cache_write_per_million": row.cost_cache_write_per_million,
            "currency": row.cost_currency,
        },
        "usage_today": {
            "requests": row.usage_requests_today,
            "prompt_tokens": row.usage_prompt_tokens_today,
            "completion_tokens": row.usage_completion_tokens_today,
            "total_tokens": row.usage_total_tokens_today,
            "cache_read_tokens": row.cache_read_tokens_today,
            "cache_write_tokens": row.cache_write_tokens_today,
            "estimated_cost": row.estimated_cost_today,
            "cache_accounting": row.cache_accounting,
        },
        "fallback": {
            "role": row.fallback_role,
            "count": row.fallback_count,
            "models": row.fallback_models,
        },
    })
}

fn format_fallback_models(models: &[String]) -> String {
    if models.is_empty() {
        "none".into()
    } else {
        models.join(",")
    }
}

async fn provider_health(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    live: bool,
) -> Result<()> {
    match provider_health_report(paths, workspace, agent_override, live).await? {
        ProviderHealthReport::LiveOk {
            provider,
            model,
            usage_total,
        } => {
            println!("live: ok");
            println!("provider: {provider}");
            println!("model: {model}");
            println!("usage_total: {usage_total}");
        }
        ProviderHealthReport::LiveFailed { error } => {
            println!("live: failed");
            println!("error: {error}");
        }
        ProviderHealthReport::Local {
            provider,
            model,
            health,
            consecutive_failures,
            last_error_kind,
            last_error_summary,
            cooldown_until,
            health_log,
        } => {
            println!("provider: {provider}");
            println!("model: {model}");
            println!("health: {health}");
            println!("consecutive_failures: {consecutive_failures}");
            if let Some(kind) = last_error_kind {
                println!("last_error_kind: {kind}");
            }
            if let Some(error) = last_error_summary {
                println!("last_error: {error}");
            }
            if let Some(cooldown_until) = cooldown_until {
                println!("cooldown_until: {cooldown_until}");
            }
            println!("health_log: {}", health_log.display());
        }
    }
    Ok(())
}
