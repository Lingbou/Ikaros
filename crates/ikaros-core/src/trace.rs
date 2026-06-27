// SPDX-License-Identifier: GPL-3.0-only

use crate::{IkarosError, Result, now_rfc3339, redact_json, redact_secrets};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const STRUCTURED_TRACE_SCHEMA: &str = "ikaros-structured-trace-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredTraceEvent {
    pub schema: String,
    pub id: String,
    pub at: String,
    pub level: String,
    pub target: String,
    pub event: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub data: serde_json::Value,
}

impl StructuredTraceEvent {
    pub fn new(
        level: impl Into<String>,
        target: impl Into<String>,
        event: impl Into<String>,
        message: impl Into<String>,
        data: serde_json::Value,
    ) -> Result<Self> {
        Ok(Self {
            schema: STRUCTURED_TRACE_SCHEMA.into(),
            id: Uuid::new_v4().to_string(),
            at: now_rfc3339()?,
            level: normalize_trace_level(level),
            target: redact_secrets(&target.into()),
            event: redact_trace_label(&event.into()),
            message: redact_secrets(&message.into()),
            correlation_id: None,
            session_id: None,
            turn_id: None,
            command: None,
            data: redact_json(data),
        })
    }

    pub fn with_correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        let correlation_id = redact_secrets(&correlation_id.into());
        if !correlation_id.trim().is_empty() {
            self.correlation_id = Some(correlation_id);
        }
        self
    }

    pub fn with_session_turn(
        mut self,
        session_id: impl Into<String>,
        turn_id: impl Into<String>,
    ) -> Self {
        let session_id = redact_secrets(&session_id.into());
        let turn_id = redact_secrets(&turn_id.into());
        if !session_id.trim().is_empty() {
            self.session_id = Some(session_id);
        }
        if !turn_id.trim().is_empty() {
            self.turn_id = Some(turn_id);
        }
        self
    }

    pub fn with_command(mut self, command: impl Into<String>) -> Self {
        let command = redact_secrets(&command.into());
        if !command.trim().is_empty() {
            self.command = Some(command);
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredTraceLog {
    path: PathBuf,
}

impl StructuredTraceLog {
    pub fn new(logs_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: logs_dir.into().join("trace.jsonl"),
        }
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, event: StructuredTraceEvent) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let encoded = serde_json::to_string(&event)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| IkarosError::io(&self.path, source))?;
        writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.path, source))
    }
}

fn normalize_trace_level(level: impl Into<String>) -> String {
    let level = level.into();
    match level.trim().to_ascii_uppercase().as_str() {
        "TRACE" => "TRACE".into(),
        "DEBUG" => "DEBUG".into(),
        "INFO" => "INFO".into(),
        "WARN" | "WARNING" => "WARN".into(),
        "ERROR" => "ERROR".into(),
        _ => "INFO".into(),
    }
}

fn redact_trace_label(value: &str) -> String {
    redact_secrets(value)
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}
