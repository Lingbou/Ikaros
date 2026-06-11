// SPDX-License-Identifier: GPL-3.0-only

use super::{SelfModifyChangeKind, SelfModifyCheckProfile, SelfModifyCheckReport, SelfModifyStore};
use crate::{TestFailureAnalyzer, validate_test_command};
use ikaros_core::{IkarosError, Result, SelfModifyConfig, redact_secrets};
use std::{path::Path, process::Command};

impl SelfModifyStore {
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

    pub(super) fn run_checks(&self, commands: &[String]) -> Result<Vec<SelfModifyCheckReport>> {
        commands
            .iter()
            .map(|command| run_check(command, &self.workspace_root))
            .collect()
    }
}

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
