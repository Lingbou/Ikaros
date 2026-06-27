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
pub struct ScheduleRetryPolicy {
    pub max_attempts: u32,
    pub backoff_seconds: u64,
}

impl ScheduleRetryPolicy {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

impl Default for ScheduleRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            backoff_seconds: 60,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleJobOptions {
    pub interval_seconds: Option<u64>,
    pub agent: Option<String>,
    pub deliveries: Vec<ScheduleDeliveryTarget>,
    pub retry: ScheduleRetryPolicy,
    pub grace_period_seconds: Option<u64>,
    pub timezone: Option<String>,
}

impl Default for ScheduleJobOptions {
    fn default() -> Self {
        Self {
            interval_seconds: None,
            agent: None,
            deliveries: ScheduleDeliveryTarget::default_targets(),
            retry: ScheduleRetryPolicy::default(),
            grace_period_seconds: None,
            timezone: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleRunHistoryEntry {
    pub ran_at: String,
    pub status: String,
    pub summary: String,
    pub next_run_at: Option<String>,
    pub enabled: bool,
    pub attempt: u32,
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
    #[serde(default, skip_serializing_if = "ScheduleRetryPolicy::is_default")]
    pub retry: ScheduleRetryPolicy,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub retry_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grace_period_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<ScheduleRunHistoryEntry>,
}

impl ScheduledJob {
    pub fn new(
        task: impl Into<String>,
        run_at: impl Into<String>,
        interval_seconds: Option<u64>,
        agent: Option<String>,
        deliveries: Vec<ScheduleDeliveryTarget>,
    ) -> Result<Self> {
        Self::new_with_options(
            task,
            run_at,
            ScheduleJobOptions {
                interval_seconds,
                agent,
                deliveries,
                ..ScheduleJobOptions::default()
            },
        )
    }

    pub fn new_with_options(
        task: impl Into<String>,
        run_at: impl Into<String>,
        options: ScheduleJobOptions,
    ) -> Result<Self> {
        validate_schedule_job_options(&options)?;
        let task = redact_secrets(&task.into());
        let now = now_rfc3339()?;
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            title: title_from_task(&task),
            task,
            agent: options.agent.map(|value| redact_secrets(&value)),
            run_at: run_at.into(),
            interval_seconds: options.interval_seconds,
            deliveries: options.deliveries,
            retry: options.retry,
            retry_attempts: 0,
            grace_period_seconds: options.grace_period_seconds,
            timezone: options.timezone.map(|value| redact_secrets(&value)),
            enabled: true,
            created_at: now.clone(),
            updated_at: now,
            last_run_at: None,
            last_status: None,
            last_summary: None,
            history: Vec::new(),
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

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
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

fn validate_schedule_job_options(options: &ScheduleJobOptions) -> Result<()> {
    if let Some(interval_seconds) = options.interval_seconds {
        validate_schedule_duration("schedule interval", interval_seconds)?;
    }
    if let Some(grace_period_seconds) = options.grace_period_seconds {
        validate_schedule_duration("schedule grace period", grace_period_seconds)?;
    }
    validate_schedule_duration("schedule retry backoff", options.retry.backoff_seconds)?;
    if options.retry.max_attempts == 0 {
        return Err(IkarosError::Message(
            "schedule retry max_attempts must be at least 1".into(),
        ));
    }
    Ok(())
}

fn validate_schedule_duration(label: &str, seconds: u64) -> Result<()> {
    if seconds == 0 {
        return Err(IkarosError::Message(format!("{label} must be positive")));
    }
    if i64::try_from(seconds).is_err() {
        return Err(IkarosError::Message(format!(
            "{label} exceeds supported duration range"
        )));
    }
    Ok(())
}
