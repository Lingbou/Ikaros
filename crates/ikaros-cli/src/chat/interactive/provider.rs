// SPDX-License-Identifier: GPL-3.0-only

use crate::config::{
    ConfigBudgetArgs, ConfigBudgetUpdate, ConfigCommand, config_command, update_budget_config,
};
use crate::debug::{DebugCommand, debug_command};
use crate::provider::{ProviderCommand, provider_command};
use crate::resolve_agent_instance;
use anyhow::{Context, Result, anyhow};
use ikaros_core::{IkarosConfig, IkarosPaths, ModelConfig, RemoteProviderConfig};
use ikaros_harness::ExecutionSession;
use ikaros_models::{
    ModelProvider, governed_provider_from_config_with_http_client,
    model_request_options_from_config,
};
use ikaros_runtime::EgressModelHttpClient;
use std::{path::Path, sync::Arc};

use super::{InteractiveChatRuntime, terminal_inline};

pub(super) async fn handle_provider_command(
    args: Vec<&str>,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    let agent_override = Some(runtime.agent_id.as_str());
    match args.as_slice() {
        [] | ["inspect"] => {
            provider_command(ProviderCommand::Inspect, paths, workspace, agent_override).await
        }
        ["health"] => {
            provider_command(
                ProviderCommand::Health { live: false },
                paths,
                workspace,
                agent_override,
            )
            .await
        }
        ["health", "--live"] => {
            provider_command(
                ProviderCommand::Health { live: true },
                paths,
                workspace,
                agent_override,
            )
            .await
        }
        ["matrix"] => {
            provider_command(
                ProviderCommand::Matrix {
                    live: false,
                    json: false,
                },
                paths,
                workspace,
                agent_override,
            )
            .await
        }
        ["matrix", "--live"] => {
            provider_command(
                ProviderCommand::Matrix {
                    live: true,
                    json: false,
                },
                paths,
                workspace,
                agent_override,
            )
            .await
        }
        ["matrix", "--json"] => {
            provider_command(
                ProviderCommand::Matrix {
                    live: false,
                    json: true,
                },
                paths,
                workspace,
                agent_override,
            )
            .await
        }
        ["matrix", "--live", "--json"] | ["matrix", "--json", "--live"] => {
            provider_command(
                ProviderCommand::Matrix {
                    live: true,
                    json: true,
                },
                paths,
                workspace,
                agent_override,
            )
            .await
        }
        ["profiles"] => {
            provider_command(ProviderCommand::Profiles, paths, workspace, agent_override).await
        }
        ["debug"] => debug_command(DebugCommand::Provider, paths, workspace, agent_override).await,
        _ => {
            println!(
                "usage: /provider [inspect|health [--live]|matrix [--live] [--json]|profiles|debug]"
            );
            Ok(())
        }
    }
}

pub(super) fn handle_budget_command(
    args: Vec<&str>,
    paths: &IkarosPaths,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    let budget_args = match args.as_slice() {
        [] | ["show"] | ["status"] => ConfigCommand::Budget(ConfigBudgetArgs {
            set: None,
            disable: false,
            json: false,
        }),
        ["disable"] | ["off"] => ConfigCommand::Budget(ConfigBudgetArgs {
            set: None,
            disable: true,
            json: false,
        }),
        ["set", value] => {
            let budget = value.parse::<u32>().with_context(|| {
                format!(
                    "invalid /budget set value `{}`; expected a positive integer token count",
                    terminal_inline(value)
                )
            })?;
            if budget == 0 {
                return Err(anyhow!(
                    "invalid /budget set value `0`; use /budget disable to remove the daily budget"
                ));
            }
            ConfigCommand::Budget(ConfigBudgetArgs {
                set: Some(budget),
                disable: false,
                json: false,
            })
        }
        ["--json"] | ["show", "--json"] | ["status", "--json"] => {
            ConfigCommand::Budget(ConfigBudgetArgs {
                set: None,
                disable: false,
                json: true,
            })
        }
        _ => {
            println!("usage: /budget [show|set <tokens>|disable] [--json]");
            return Ok(());
        }
    };
    let ConfigCommand::Budget(budget_args) = budget_args else {
        unreachable!("budget parser only returns budget commands");
    };
    if runtime.default_inline_stdout() && !budget_args.json {
        let update = update_budget_config(paths, budget_args.set, budget_args.disable)?;
        reload_interactive_model_config(paths, runtime, false)?;
        print_budget_status_for_human(runtime, &update);
    } else {
        config_command(ConfigCommand::Budget(budget_args), paths)?;
        reload_interactive_model_config(paths, runtime, true)?;
        println!("{}", budget_runtime_json_line(runtime));
    }
    if !runtime.pending_inputs.is_empty() && !runtime.default_inline_stdout() {
        println!(
            "budget_resume_hint: pending_inputs={} command=/queue run",
            runtime.pending_inputs.len()
        );
    }
    Ok(())
}

fn reload_interactive_model_config(
    paths: &IkarosPaths,
    runtime: &mut InteractiveChatRuntime,
    print_status: bool,
) -> Result<()> {
    let config = IkarosConfig::load_shape_checked(&paths.config)?;
    let agent_instance = resolve_agent_instance(
        &config,
        Some(runtime.agent_id.as_str()),
        &runtime.workspace,
        &paths.home,
    )?;
    let model_config = agent_instance.model_config(&config.model.default).clone();
    let model_provider = agent_instance
        .effective_model_provider_config(&config.model.default, &config.providers.model)
        .clone();
    let request_options = model_request_options_from_config(&model_config)?;
    let provider =
        build_interactive_model_provider(&model_config, &model_provider, paths, &runtime.session)?;
    runtime.model_config = model_config;
    runtime.model_provider = model_provider;
    runtime.request_options = request_options;
    runtime.provider = provider;
    if print_status {
        println!(
            "budget_runtime: reloaded model={} daily_token_budget={}",
            terminal_inline(&runtime.model_config.model),
            runtime
                .model_config
                .daily_token_budget
                .map_or_else(|| "disabled".to_owned(), |budget| budget.to_string())
        );
    }
    Ok(())
}

fn print_budget_status_for_human(runtime: &InteractiveChatRuntime, update: &ConfigBudgetUpdate) {
    println!("• Budget");
    println!("  model: {}", terminal_inline(&runtime.model_config.model));
    println!(
        "  daily token budget: {}",
        human_optional_budget(update.current)
    );
    if update.changed {
        println!("  previous: {}", human_optional_budget(update.before));
    }
    if !runtime.pending_inputs.is_empty() {
        println!(
            "  pending input: {} queued, run /queue run to continue",
            runtime.pending_inputs.len()
        );
    }
}

fn human_optional_budget(value: Option<u32>) -> String {
    value
        .map(|budget| budget.to_string())
        .unwrap_or_else(|| "disabled".to_owned())
}

fn budget_runtime_json_line(runtime: &InteractiveChatRuntime) -> String {
    let budget = runtime.model_config.daily_token_budget;
    let payload = serde_json::json!({
        "schema": "ikaros-workbench-budget-runtime-v1",
        "version": 1,
        "model": terminal_inline(&runtime.model_config.model),
        "provider": terminal_inline(&runtime.model_config.provider),
        "daily_token_budget": budget,
        "budget_status": if budget.is_some() { "bounded" } else { "disabled" },
        "pending_inputs": runtime.pending_inputs.len(),
        "actions": {
            "show": "/budget",
            "set": "/budget set <tokens>",
            "disable": "/budget disable",
            "run_pending": (!runtime.pending_inputs.is_empty()).then_some("/queue run"),
        },
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        r#"{"schema":"ikaros-workbench-budget-runtime-v1","version":1,"budget_status":"unknown","pending_inputs":0,"actions":{"show":"/budget","set":"/budget set <tokens>","disable":"/budget disable","run_pending":null}}"#
            .to_owned()
    });
    format!("budget_runtime_json: {encoded}")
}

pub(in crate::chat) fn build_interactive_model_provider(
    model_config: &ModelConfig,
    model_provider: &RemoteProviderConfig,
    paths: &IkarosPaths,
    session: &ExecutionSession,
) -> Result<Box<dyn ModelProvider>> {
    Ok(governed_provider_from_config_with_http_client(
        model_config,
        model_provider,
        &paths.audit_dir,
        Some(Arc::new(EgressModelHttpClient::new(session.env.clone()))),
    )?)
}
