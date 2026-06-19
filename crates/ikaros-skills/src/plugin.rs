// SPDX-License-Identifier: GPL-3.0-only

use async_trait::async_trait;
use ikaros_core::{IkarosError, Result, RiskLevel, redact_secrets};
use ikaros_harness::{
    LoadedPluginManifest, PLUGIN_COMMAND_MAX_OUTPUT_BYTES, PLUGIN_COMMAND_MAX_STDIN_BYTES,
    PLUGIN_COMMAND_MAX_TIMEOUT_MS, PluginCatalog, PluginCommandManifest, PluginSkillManifest,
    PolicyRequest, ProcessOutput, ProcessRequest, Skill, SkillContext, SkillOutput,
};
use serde_json::json;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct PluginCommandRunSkill {
    skills_dir: PathBuf,
}

impl PluginCommandRunSkill {
    pub fn new(skills_dir: impl Into<PathBuf>) -> Self {
        Self {
            skills_dir: skills_dir.into(),
        }
    }
}

#[async_trait]
impl Skill for PluginCommandRunSkill {
    fn name(&self) -> &'static str {
        "plugin_command_run"
    }

    fn description(&self) -> &'static str {
        "Run an enabled command-backed plugin skill through harness policy."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["name", "input"],
            "properties": {
                "name": {"type": "string"},
                "input": {"type": "object"}
            }
        })
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::ShellRead
    }

    fn policy_request(&self, input: &serde_json::Value, workspace_root: &Path) -> PolicyRequest {
        let request = self
            .resolve_plugin_command(input)
            .map(|resolved| PolicyRequest {
                action: format!("plugin:{}", resolved.qualified_name),
                risk: resolved.skill.risk.clone(),
                path: Some(resolve_policy_path(&resolved.program, workspace_root)),
                command: Some(resolved.command_display()),
                is_write: matches!(
                    resolved.skill.risk,
                    RiskLevel::LocalWrite | RiskLevel::ShellWrite | RiskLevel::DatabaseWrite
                ),
            });
        request.unwrap_or_else(|error| PolicyRequest {
            action: format!(
                "plugin:{}",
                input
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown")
            ),
            risk: RiskLevel::Destructive,
            path: None,
            command: Some(redact_secrets(&error.to_string())),
            is_write: true,
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let plugin_input = input.get("input").cloned().unwrap_or_else(|| json!({}));
        let resolved = self.resolve_plugin_command(&input)?;
        let output = run_plugin_command(&resolved, plugin_input, &ctx).await?;
        Ok(SkillOutput::new(
            format!("plugin command executed: {}", resolved.qualified_name),
            json!({
                "plugin": resolved.plugin.manifest.name,
                "skill": resolved.skill.name,
                "status": output.status,
                "stdout": output.stdout,
                "stderr": output.stderr,
            }),
        ))
    }
}

fn resolve_policy_path(path: &Path, workspace_root: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

#[derive(Clone)]
struct ResolvedPluginCommand {
    plugin: LoadedPluginManifest,
    skill: PluginSkillManifest,
    command: PluginCommandManifest,
    qualified_name: String,
    program: PathBuf,
}

impl ResolvedPluginCommand {
    fn command_display(&self) -> String {
        std::iter::once(self.program.display().to_string())
            .chain(self.command.args.iter().cloned())
            .map(|part| redact_secrets(&part))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl PluginCommandRunSkill {
    fn resolve_plugin_command(&self, input: &serde_json::Value) -> Result<ResolvedPluginCommand> {
        let name = input
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| IkarosError::Message("plugin skill name is required".into()))?;
        let catalog = PluginCatalog::load(&self.skills_dir)?;
        let (plugin, skill) = catalog.find_skill(name).ok_or_else(|| {
            IkarosError::Message(format!("enabled plugin skill not found: {name}"))
        })?;
        let command = skill.command.clone().ok_or_else(|| {
            IkarosError::Message(format!(
                "plugin skill is declaration-only and has no command: {}.{}",
                plugin.manifest.name, skill.name
            ))
        })?;
        let plugin_root = plugin.path.parent().ok_or_else(|| {
            IkarosError::Message(format!(
                "plugin manifest has no parent directory: {}",
                plugin.path.display()
            ))
        })?;
        let program = resolve_plugin_program(plugin_root, &command.program)?;
        Ok(ResolvedPluginCommand {
            plugin: plugin.clone(),
            skill: skill.clone(),
            command,
            qualified_name: format!("{}.{}", plugin.manifest.name, skill.name),
            program,
        })
    }
}

fn resolve_plugin_program(plugin_root: &Path, program: &Path) -> Result<PathBuf> {
    if program.as_os_str().is_empty()
        || program.is_absolute()
        || program.components().any(|component| {
            matches!(component, Component::ParentDir) || component.as_os_str() == ".temp"
        })
    {
        return Err(IkarosError::Message(
            "plugin command program must be relative and must not target .temp".into(),
        ));
    }
    let plugin_root =
        fs::canonicalize(plugin_root).map_err(|source| IkarosError::io(plugin_root, source))?;
    let program_path = plugin_root.join(program);
    let program = fs::canonicalize(&program_path).map_err(|source| {
        IkarosError::Message(format!(
            "plugin command program is missing or inaccessible: {}: {source}",
            program.display()
        ))
    })?;
    if !program.starts_with(&plugin_root) {
        return Err(IkarosError::Message(
            "plugin command program must resolve under the plugin directory".into(),
        ));
    }
    if !program.is_file() {
        return Err(IkarosError::Message(format!(
            "plugin command program must be a file: {}",
            program.display()
        )));
    }
    Ok(program)
}

async fn run_plugin_command(
    resolved: &ResolvedPluginCommand,
    plugin_input: serde_json::Value,
    ctx: &SkillContext,
) -> Result<ProcessOutput> {
    let stdin = serde_json::to_string(&plugin_input)?;
    if stdin.len() > PLUGIN_COMMAND_MAX_STDIN_BYTES {
        return Err(IkarosError::Message(format!(
            "plugin command stdin exceeds {PLUGIN_COMMAND_MAX_STDIN_BYTES} bytes"
        )));
    }
    let timeout_ms = resolved
        .command
        .timeout_ms
        .unwrap_or(PLUGIN_COMMAND_MAX_TIMEOUT_MS)
        .min(PLUGIN_COMMAND_MAX_TIMEOUT_MS);
    let request = ProcessRequest::program(
        resolved.program.display().to_string(),
        resolved.command.args.clone(),
        &ctx.session.sandbox.workspace_root,
    )
    .with_stdin(stdin)
    .with_timeout_ms(timeout_ms)
    .with_max_output_bytes(PLUGIN_COMMAND_MAX_OUTPUT_BYTES);
    let output = ctx
        .session
        .env
        .run_process(request)
        .await
        .map_err(|source| {
            IkarosError::Message(format!(
                "failed to run plugin command {}: {source}",
                resolved.qualified_name
            ))
        })?;
    Ok(ProcessOutput {
        status: output.status,
        stdout: redact_secrets(&output.stdout),
        stderr: redact_secrets(&output.stderr),
    })
}
