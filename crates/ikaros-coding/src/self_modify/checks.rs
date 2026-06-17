// SPDX-License-Identifier: GPL-3.0-only

use super::{SelfModifyChangeKind, SelfModifyCheckProfile, SelfModifyCheckReport, SelfModifyStore};
use crate::{TestFailureAnalyzer, validate_test_command};
use ikaros_core::{IkarosError, Result, SelfModifyConfig, redact_secrets};
use ikaros_harness::{FileSystem as ExecutionFileSystem, ProcessRequest, ProcessRunner};
use std::path::Path;
#[cfg(test)]
use std::process::Command;

impl SelfModifyStore {
    #[cfg(test)]
    pub fn default_check_profile(
        &self,
        change_kind: &SelfModifyChangeKind,
    ) -> SelfModifyCheckProfile {
        let has_cargo = self.workspace_root.join("Cargo.toml").exists();
        let commands = if has_cargo {
            match change_kind {
                SelfModifyChangeKind::SkillPatch
                | SelfModifyChangeKind::PersonaPatch
                | SelfModifyChangeKind::ConfigPatch
                | SelfModifyChangeKind::RuntimePatch => {
                    vec!["cargo check --workspace --all-features".into()]
                }
                SelfModifyChangeKind::DocumentationPatch => {
                    vec!["cargo fmt --all -- --check".into()]
                }
            }
        } else {
            Vec::new()
        };
        let reason = if commands.is_empty() {
            "no default self-check command matched this workspace".into()
        } else {
            match change_kind {
                SelfModifyChangeKind::SkillPatch => {
                    "skill patches default to a workspace cargo check".into()
                }
                SelfModifyChangeKind::PersonaPatch => {
                    "persona patches default to a workspace cargo check".into()
                }
                SelfModifyChangeKind::ConfigPatch => {
                    "config patches default to a workspace cargo check".into()
                }
                SelfModifyChangeKind::RuntimePatch => {
                    "runtime patches default to a workspace cargo check".into()
                }
                SelfModifyChangeKind::DocumentationPatch => {
                    "documentation patches default to a cargo formatting check".into()
                }
            }
        };
        SelfModifyCheckProfile {
            change_kind: change_kind.clone(),
            source: "default".into(),
            commands,
            reason,
        }
    }

    pub async fn default_check_profile_with_env(
        &self,
        change_kind: &SelfModifyChangeKind,
        file_system: &dyn ExecutionFileSystem,
    ) -> Result<SelfModifyCheckProfile> {
        let has_cargo = super::diff::path_metadata_with_env(
            file_system,
            &self.workspace_root.join("Cargo.toml"),
        )
        .await?
        .is_some_and(|metadata| metadata.is_file);
        Ok(self.check_profile_from_workspace_shape(change_kind, has_cargo))
    }

    pub fn configured_check_profile(
        &self,
        change_kind: &SelfModifyChangeKind,
        config: &SelfModifyConfig,
    ) -> Result<Option<SelfModifyCheckProfile>> {
        let Some(profile) = config.check_profiles.get(change_kind.as_config_key()) else {
            return Ok(None);
        };
        if profile.commands.is_empty() {
            return Err(IkarosError::Message(format!(
                "self-modify check profile `{}` must include at least one command",
                change_kind.as_config_key()
            )));
        }
        let commands = profile
            .commands
            .iter()
            .map(|command| {
                validate_test_command(command)?;
                Ok(redact_secrets(command))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(SelfModifyCheckProfile {
            change_kind: change_kind.clone(),
            source: format!("config:{}", change_kind.as_config_key()),
            commands,
            reason: profile.reason.clone().unwrap_or_else(|| {
                format!(
                    "configured self-modify check profile for {}",
                    change_kind.as_config_key()
                )
            }),
        }))
    }

    #[cfg(test)]
    pub(super) fn run_checks(&self, commands: &[String]) -> Result<Vec<SelfModifyCheckReport>> {
        commands
            .iter()
            .map(|command| run_check(command, &self.workspace_root))
            .collect()
    }

    pub(super) async fn run_checks_with_env(
        &self,
        commands: &[String],
        process_runner: &dyn ProcessRunner,
    ) -> Result<Vec<SelfModifyCheckReport>> {
        let mut reports = Vec::with_capacity(commands.len());
        for command in commands {
            reports.push(run_check_with_env(command, &self.workspace_root, process_runner).await?);
        }
        Ok(reports)
    }

    fn check_profile_from_workspace_shape(
        &self,
        change_kind: &SelfModifyChangeKind,
        has_cargo: bool,
    ) -> SelfModifyCheckProfile {
        let commands = if has_cargo {
            match change_kind {
                SelfModifyChangeKind::SkillPatch
                | SelfModifyChangeKind::PersonaPatch
                | SelfModifyChangeKind::ConfigPatch
                | SelfModifyChangeKind::RuntimePatch => {
                    vec!["cargo check --workspace --all-features".into()]
                }
                SelfModifyChangeKind::DocumentationPatch => {
                    vec!["cargo fmt --all -- --check".into()]
                }
            }
        } else {
            Vec::new()
        };
        let reason = if commands.is_empty() {
            "no default self-check command matched this workspace".into()
        } else {
            match change_kind {
                SelfModifyChangeKind::SkillPatch => {
                    "skill patches default to a workspace cargo check".into()
                }
                SelfModifyChangeKind::PersonaPatch => {
                    "persona patches default to a workspace cargo check".into()
                }
                SelfModifyChangeKind::ConfigPatch => {
                    "config patches default to a workspace cargo check".into()
                }
                SelfModifyChangeKind::RuntimePatch => {
                    "runtime patches default to a workspace cargo check".into()
                }
                SelfModifyChangeKind::DocumentationPatch => {
                    "documentation patches default to a cargo formatting check".into()
                }
            }
        };
        SelfModifyCheckProfile {
            change_kind: change_kind.clone(),
            source: "default".into(),
            commands,
            reason,
        }
    }
}

#[cfg(test)]
fn run_check(command: &str, cwd: &Path) -> Result<SelfModifyCheckReport> {
    validate_test_command(command)?;
    #[cfg(windows)]
    let output = Command::new("cmd")
        .arg("/C")
        .arg(command)
        .current_dir(cwd)
        .output()
        .map_err(|source| IkarosError::Message(format!("failed to run command: {source}")))?;

    #[cfg(not(windows))]
    let output = Command::new("sh")
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .output()
        .map_err(|source| IkarosError::Message(format!("failed to run command: {source}")))?;

    let status = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let analysis = TestFailureAnalyzer::analyze(command, status, &stdout, &stderr);
    Ok(SelfModifyCheckReport {
        command: redact_secrets(command),
        status,
        passed: status == 0,
        analysis,
    })
}

async fn run_check_with_env(
    command: &str,
    cwd: &Path,
    process_runner: &dyn ProcessRunner,
) -> Result<SelfModifyCheckReport> {
    validate_test_command(command)?;
    let request = process_request_from_check_command(command, cwd)?;
    let output = process_runner.run_process(request).await?;
    let analysis =
        TestFailureAnalyzer::analyze(command, output.status, &output.stdout, &output.stderr);
    Ok(SelfModifyCheckReport {
        command: redact_secrets(command),
        status: output.status,
        passed: output.status == 0,
        analysis,
    })
}

fn process_request_from_check_command(command: &str, cwd: &Path) -> Result<ProcessRequest> {
    let mut parts = command.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| IkarosError::Message("self-check command is empty".into()))?;
    let args = parts.map(ToOwned::to_owned).collect::<Vec<_>>();
    Ok(ProcessRequest::program(program, args, cwd.to_path_buf())
        .with_timeout_ms(600_000)
        .with_max_output_bytes(1_048_576))
}
