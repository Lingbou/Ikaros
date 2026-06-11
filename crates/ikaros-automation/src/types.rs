// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result, now_rfc3339, redact_secrets};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleDeliveryTarget {
    LocalFile,
    GatewayOutbox,
}

impl ScheduleDeliveryTarget {
    pub fn default_targets() -> Vec<Self> {
        vec![Self::LocalFile]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LocalFile => "local_file",
            Self::GatewayOutbox => "gateway_outbox",
        }
    }
}

impl FromStr for ScheduleDeliveryTarget {
    type Err = IkarosError;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "local-file" | "local_file" | "file" => Ok(Self::LocalFile),
            "gateway-outbox" | "gateway_outbox" | "outbox" => Ok(Self::GatewayOutbox),
            other => Err(IkarosError::Message(format!(
                "unsupported schedule delivery target: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduledJob {
    pub id: String,
    pub title: String,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub run_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deliveries: Vec<ScheduleDeliveryTarget>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_summary: Option<String>,
}

impl ScheduledJob {
    pub fn new(
        task: impl Into<String>,
        run_at: impl Into<String>,
        interval_seconds: Option<u64>,
        agent: Option<String>,
        deliveries: Vec<ScheduleDeliveryTarget>,
    ) -> Result<Self> {
        let task = redact_secrets(&task.into());
        let now = now_rfc3339()?;
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            title: title_from_task(&task),
            task,
            agent: agent.map(|value| redact_secrets(&value)),
            run_at: run_at.into(),
            interval_seconds,
            deliveries,
            enabled: true,
            created_at: now.clone(),
            updated_at: now,
            last_run_at: None,
            last_status: None,
            last_summary: None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleRunUpdate {
    pub ran_at: String,
    pub status: String,
    pub summary: String,
    pub next_run_at: Option<String>,
    pub enabled: bool,
}

fn title_from_task(task: &str) -> String {
    let mut title = task
        .lines()
        .next()
        .unwrap_or("scheduled task")
        .trim()
        .to_string();
    if title.is_empty() {
        title = "scheduled task".into();
    }
    if title.len() > 80 {
        title.truncate(77);
        title.push_str("...");
    }
    title
}
