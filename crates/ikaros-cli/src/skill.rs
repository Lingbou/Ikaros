// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    print_approval_hint, print_skill_result, resolve_agent_instance, session_and_registry,
};
use anyhow::Result;
use clap::{Args, Subcommand};
use ikaros_core::{IkarosConfig, IkarosPaths, ResolvedAgentProfile};
use ikaros_harness::{
    PluginCatalog, ToolVisibility, audit_plugins, install_local_plugin, set_plugin_enabled,
    set_plugin_quarantine, uninstall_local_plugin, validate_plugin_file,
};
use ikaros_runtime::agent_toolset_selection;
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Subcommand)]
pub(crate) enum SkillCommand {
    List,
    Inspect { name: String },
    Audit,
    Validate { path: PathBuf },
    Install(SkillInstall),
    Uninstall { name: String },
    Enable { name: String },
    Disable { name: String },
    Quarantine(SkillQuarantine),
    Unquarantine { name: String },
    Run(SkillRun),
}

#[derive(Debug, Args)]
pub(crate) struct SkillInstall {
    path: PathBuf,
    #[arg(long)]
    enable: bool,
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
pub(crate) struct SkillRun {
    name: String,
    #[arg(long = "input-json", default_value = "{}")]
    input_json: String,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Args)]
pub(crate) struct SkillQuarantine {
    name: String,
    #[arg(long, default_value = "operator quarantine")]
    reason: String,
}

pub(crate) async fn skill_command(
    command: SkillCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    paths.ensure()?;
    let config = IkarosConfig::load(&paths.config)?;
    let agent_instance = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let plugins = PluginCatalog::load(&paths.skills_dir)?;
    let agent = ResolvedAgentProfile {
        name: agent_instance.profile_name.clone(),
        profile: agent_instance.profile.clone(),
    };
    let toolsets = agent_toolset_selection(&agent)?;
    match command {
        SkillCommand::List => {
            println!("builtins:");
            for descriptor in registry.descriptors() {
                let visibility = model_visibility(&registry, &descriptor.name, &toolsets);
                println!(
                    "- {} [{:?} toolset={} visibility={}]",
                    descriptor.name, descriptor.risk_level, descriptor.toolset, visibility
                );
            }
            println!("plugins:");
            if plugins.plugins.is_empty() {
                println!("- none");
            } else {
                for plugin in &plugins.plugins {
                    let state = plugin_marketplace_state(
                        plugin.marketplace.enabled,
                        plugin.marketplace.quarantined,
                    );
                    println!(
                        "- {} {} [{} priority={} source={}]: {}",
                        plugin.manifest.name,
                        plugin.manifest.version,
                        state,
                        plugin.marketplace.priority,
                        plugin.marketplace.source,
                        plugin.manifest.description
                    );
                    if plugin.marketplace.enabled && !plugin.marketplace.quarantined {
                        for skill in &plugin.manifest.skills {
                            let execution = if skill.command.is_some() {
                                "command"
                            } else {
                                "declaration"
                            };
                            println!(
                                "  - {}.{} [{:?} {}]",
                                plugin.manifest.name, skill.name, skill.risk, execution
                            );
                        }
                    } else if plugin.marketplace.quarantined {
                        println!("  - skills quarantined by marketplace metadata");
                    } else {
                        println!("  - skills disabled by marketplace metadata");
                    }
                }
            }
            print_plugin_warnings(&plugins);
        }
        SkillCommand::Inspect { name } => {
            if let Some(skill) = registry.get(&name) {
                let descriptor = registry
                    .descriptors()
                    .into_iter()
                    .find(|descriptor| descriptor.name == name)
                    .ok_or_else(|| anyhow::anyhow!("skill descriptor missing: {name}"))?;
                println!("name: {}", skill.name());
                println!("kind: builtin");
                println!("description: {}", skill.description());
                println!("risk: {:?}", skill.risk_level());
                println!("toolset: {}", descriptor.toolset);
                println!(
                    "model_visibility: {}",
                    model_visibility(&registry, &descriptor.name, &toolsets)
                );
                println!(
                    "input_schema: {}",
                    serde_json::to_string_pretty(&skill.input_schema())?
                );
            } else if let Some((plugin, skill)) = plugins.find_declared_skill(&name) {
                println!("name: {}.{}", plugin.manifest.name, skill.name);
                println!("kind: plugin-manifest");
                println!("plugin: {}", plugin.manifest.name);
                println!("version: {}", plugin.manifest.version);
                println!("enabled: {}", plugin.marketplace.enabled);
                println!("quarantined: {}", plugin.marketplace.quarantined);
                if let Some(reason) = &plugin.marketplace.quarantine_reason {
                    println!("quarantine_reason: {reason}");
                }
                println!("priority: {}", plugin.marketplace.priority);
                println!("source: {}", plugin.marketplace.source);
                if let Some(path) = &plugin.marketplace.path {
                    println!("marketplace_path: {}", path.display());
                }
                if let Some(repository) = &plugin.marketplace.repository {
                    println!("repository: {repository}");
                }
                if let Some(homepage) = &plugin.marketplace.homepage {
                    println!("homepage: {homepage}");
                }
                if let Some(license) = &plugin.marketplace.license {
                    println!("license: {license}");
                }
                if !plugin.marketplace.tags.is_empty() {
                    println!("tags: {}", plugin.marketplace.tags.join(", "));
                }
                println!("path: {}", plugin.path.display());
                println!("description: {}", skill.description);
                println!("risk: {:?}", skill.risk);
                println!(
                    "input_schema: {}",
                    serde_json::to_string_pretty(&skill.input_schema)?
                );
                if !skill.permissions.is_empty() {
                    println!(
                        "permissions: {}",
                        serde_json::to_string_pretty(&skill.permissions)?
                    );
                }
                if let Some(command) = &skill.command {
                    println!("command: {}", command.program.display());
                    if !command.args.is_empty() {
                        println!("command_args: {}", command.args.join(" "));
                    }
                    if let Some(timeout_ms) = command.timeout_ms {
                        println!("command_timeout_ms: {timeout_ms}");
                    }
                } else {
                    println!("command: none");
                }
            } else {
                print_plugin_warnings(&plugins);
                anyhow::bail!("skill not found: {name}");
            }
        }
        SkillCommand::Audit => {
            let report = audit_plugins(&paths.skills_dir)?;
            println!("plugins: {}", report.plugin_count);
            println!("enabled: {}", report.enabled_plugin_count);
            println!("disabled: {}", report.disabled_plugin_count);
            println!("quarantined: {}", report.quarantined_plugin_count);
            println!("skills: {}", report.skill_count);
            println!("enabled_skills: {}", report.enabled_skill_count);
            println!("command_skills: {}", report.command_skill_count);
            println!("warnings: {}", report.warning_count);
            println!("missing_commands: {}", report.missing_command_count);
            if report.plugins.is_empty() {
                println!("plugin_details: none");
            } else {
                println!("plugin_details:");
                for plugin in &report.plugins {
                    let state = plugin_marketplace_state(plugin.enabled, plugin.quarantined);
                    let risks = plugin
                        .risk_levels
                        .iter()
                        .map(|risk| format!("{risk:?}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!(
                        "- {} {} [{} priority={} source={} skills={} enabled_skills={} command_skills={} missing_commands={}]",
                        plugin.name,
                        plugin.version,
                        state,
                        plugin.priority,
                        plugin.source,
                        plugin.skill_count,
                        plugin.enabled_skill_count,
                        plugin.command_skill_count,
                        plugin.missing_commands.len()
                    );
                    if let Some(reason) = &plugin.quarantine_reason {
                        println!("  quarantine_reason: {reason}");
                    }
                    println!("  manifest: {}", plugin.manifest_path.display());
                    if let Some(path) = &plugin.marketplace_path {
                        println!("  marketplace_path: {}", path.display());
                    }
                    println!(
                        "  risk_levels: {}",
                        if risks.is_empty() {
                            "none".to_owned()
                        } else {
                            risks
                        }
                    );
                    for missing in &plugin.missing_commands {
                        println!(
                            "  missing_command: {} -> {} ({})",
                            missing.skill_name,
                            missing.program.display(),
                            missing.resolved_path.display()
                        );
                    }
                }
            }
            if !report.warnings.is_empty() {
                println!("warning_details:");
                for warning in &report.warnings {
                    println!("- {}: {}", warning.path.display(), warning.message);
                }
            }
        }
        SkillCommand::Validate { path } => {
            let report = validate_plugin_file(&path)?;
            println!("plugin: {}", report.name);
            println!("version: {}", report.version);
            println!("path: {}", report.path.display());
            println!("skills: {}", report.skill_count);
            println!("command_skills: {}", report.command_skill_count);
            println!(
                "risk_levels: {}",
                report
                    .risk_levels
                    .iter()
                    .map(|risk| format!("{risk:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if report.missing_command_paths.is_empty() {
                println!("missing_commands: none");
            } else {
                println!("missing_commands:");
                for path in report.missing_command_paths {
                    println!("- {}", path.display());
                }
            }
        }
        SkillCommand::Install(args) => {
            let report =
                install_local_plugin(&paths.skills_dir, &args.path, args.enable, args.force)?;
            println!("plugin: {}", report.name);
            println!("version: {}", report.version);
            println!("source: {}", report.source_path.display());
            println!("target: {}", report.target_dir.display());
            println!("enabled: {}", report.enabled);
            println!("replaced: {}", report.replaced);
            println!("skills: {}", report.skill_count);
            println!("command_skills: {}", report.command_skill_count);
            println!("marketplace: {}", report.marketplace_path.display());
        }
        SkillCommand::Uninstall { name } => {
            let report = uninstall_local_plugin(&paths.skills_dir, &name)?;
            println!("plugin: {}", report.name);
            println!("manifest: {}", report.manifest_path.display());
            println!("removed: {}", report.removed_path.display());
            println!(
                "marketplace_entry_removed: {}",
                report.marketplace_entry_removed
            );
            println!("marketplace: {}", report.marketplace_path.display());
        }
        SkillCommand::Enable { name } => {
            let update = set_plugin_enabled(&paths.skills_dir, &name, true)?;
            println!("plugin: {}", update.name);
            println!("enabled: {}", update.enabled);
            println!("quarantined: {}", update.quarantined);
            if let Some(reason) = update.quarantine_reason {
                println!("quarantine_reason: {reason}");
            }
            println!("entry_added: {}", update.entry_added);
            println!("marketplace: {}", update.marketplace_path.display());
        }
        SkillCommand::Disable { name } => {
            let update = set_plugin_enabled(&paths.skills_dir, &name, false)?;
            println!("plugin: {}", update.name);
            println!("enabled: {}", update.enabled);
            println!("quarantined: {}", update.quarantined);
            if let Some(reason) = update.quarantine_reason {
                println!("quarantine_reason: {reason}");
            }
            println!("entry_added: {}", update.entry_added);
            println!("marketplace: {}", update.marketplace_path.display());
        }
        SkillCommand::Quarantine(args) => {
            let update =
                set_plugin_quarantine(&paths.skills_dir, &args.name, true, Some(&args.reason))?;
            println!("plugin: {}", update.name);
            println!("enabled: {}", update.enabled);
            println!("quarantined: {}", update.quarantined);
            if let Some(reason) = update.quarantine_reason {
                println!("quarantine_reason: {reason}");
            }
            println!("entry_added: {}", update.entry_added);
            println!("marketplace: {}", update.marketplace_path.display());
        }
        SkillCommand::Unquarantine { name } => {
            let update = set_plugin_quarantine(&paths.skills_dir, &name, false, None)?;
            println!("plugin: {}", update.name);
            println!("enabled: {}", update.enabled);
            println!("quarantined: {}", update.quarantined);
            println!("entry_added: {}", update.entry_added);
            println!("marketplace: {}", update.marketplace_path.display());
        }
        SkillCommand::Run(args) => {
            let input = serde_json::from_str::<serde_json::Value>(&args.input_json)?;
            if !input.is_object() {
                anyhow::bail!("skill run --input-json must be a JSON object");
            }
            let session = session.with_dry_run(args.dry_run);
            let result = if registry.get(&args.name).is_some() {
                session.execute_skill(&registry, &args.name, input).await?
            } else {
                session
                    .execute_skill(
                        &registry,
                        "plugin_command_run",
                        json!({"name": args.name, "input": input}),
                    )
                    .await?
            };
            print_skill_result(&result)?;
            print_approval_hint(&result);
        }
    }
    Ok(())
}

fn model_visibility(
    registry: &ikaros_harness::SkillRegistry,
    name: &str,
    toolsets: &ikaros_harness::ToolsetSelection,
) -> &'static str {
    match registry.visibility_for(name, toolsets) {
        Some(ToolVisibility::Direct) => "direct",
        Some(ToolVisibility::Deferred) => "deferred",
        Some(ToolVisibility::Disabled) | Some(ToolVisibility::Hidden) | None => "disabled",
    }
}

fn plugin_marketplace_state(enabled: bool, quarantined: bool) -> &'static str {
    if quarantined {
        "quarantined"
    } else if enabled {
        "enabled"
    } else {
        "disabled"
    }
}

fn print_plugin_warnings(plugins: &PluginCatalog) {
    if plugins.warnings.is_empty() {
        return;
    }
    eprintln!("plugin warnings:");
    for warning in &plugins.warnings {
        eprintln!("- {}: {}", warning.path.display(), warning.message);
    }
}
