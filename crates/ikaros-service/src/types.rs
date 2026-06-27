// SPDX-License-Identifier: GPL-3.0-only

use crate::render::{render_launchd, render_systemd};
use ikaros_core::redact_secrets;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ServiceManager {
    Systemd,
    Launchd,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ServiceKind {
    ScheduleWorker,
    MessageWorker,
    MessageWebhook,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceTemplateConfig {
    pub manager: ServiceManager,
    pub kind: ServiceKind,
    pub label: String,
    pub binary_path: PathBuf,
    pub ikaros_home: PathBuf,
    pub workspace: PathBuf,
    pub agent: Option<String>,
    pub host: String,
    pub port: u16,
    pub interval_seconds: u64,
    pub limit: usize,
}

impl ServiceTemplateConfig {
    pub fn default_label(kind: ServiceKind) -> &'static str {
        match kind {
            ServiceKind::ScheduleWorker => "ikaros-schedule-worker",
            ServiceKind::MessageWorker => "ikaros-message-worker",
            ServiceKind::MessageWebhook => "ikaros-message-webhook",
        }
    }

    pub fn command_args(&self) -> Vec<String> {
        let mut args = self.base_command_args();
        match self.kind {
            ServiceKind::ScheduleWorker => {
                args.extend([
                    "schedule".into(),
                    "worker".into(),
                    "--interval-seconds".into(),
                    self.interval_seconds.to_string(),
                    "--limit".into(),
                    self.limit.to_string(),
                ]);
            }
            ServiceKind::MessageWorker => {
                args.extend([
                    "message".into(),
                    "worker".into(),
                    "--interval-seconds".into(),
                    self.interval_seconds.to_string(),
                    "--limit".into(),
                    self.limit.to_string(),
                ]);
            }
            ServiceKind::MessageWebhook => {
                args.extend([
                    "message".into(),
                    "webhook".into(),
                    "--host".into(),
                    redact_secrets(&self.host),
                    "--port".into(),
                    self.port.to_string(),
                ]);
            }
        }
        args
    }

    pub(crate) fn base_command_args(&self) -> Vec<String> {
        let mut args = vec![
            self.binary_path.display().to_string(),
            "--ikaros-home".into(),
            self.ikaros_home.display().to_string(),
            self.workspace.display().to_string(),
        ];
        if let Some(agent) = self
            .agent
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            args.push("--agent".into());
            args.push(redact_secrets(agent));
        }
        args
    }

    pub fn render(&self) -> String {
        match self.manager {
            ServiceManager::Systemd => render_systemd(self),
            ServiceManager::Launchd => render_launchd(self),
        }
    }
}
