// SPDX-License-Identifier: GPL-3.0-only

use crate::types::{ProviderErrorKind, ProviderHealthStatus};
use ikaros_core::{IkarosError, Result, redact_secrets};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelUsageRecord {
    pub id: String,
    pub at: String,
    pub provider: String,
    pub model: String,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: u32,
    pub estimated: bool,
}

#[derive(Debug, Clone)]
pub struct ModelUsageLedger {
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderHealthRecord {
    pub at: String,
    pub provider: String,
    pub model: String,
    pub status: ProviderHealthStatus,
    pub consecutive_failures: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_kind: Option<ProviderErrorKind>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_error_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_until: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderHealthLedger {
    path: PathBuf,
}

impl ModelUsageLedger {
    pub fn new(audit_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: audit_dir.into().join("model-usage.jsonl"),
        }
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, record: ModelUsageRecord) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let encoded = serde_json::to_string(&record)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| IkarosError::io(&self.path, source))?;
        writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.path, source))
    }

    pub fn read_all(&self) -> Result<Vec<ModelUsageRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file =
            fs::File::open(&self.path).map_err(|source| IkarosError::io(&self.path, source))?;
        let reader = BufReader::new(file);
        let mut records = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|source| IkarosError::io(&self.path, source))?;
            if !line.trim().is_empty() {
                records.push(serde_json::from_str(&line)?);
            }
        }
        Ok(records)
    }

    pub fn total_for_day(&self, day: &str) -> Result<u32> {
        Ok(self
            .read_all()?
            .into_iter()
            .filter(|record| record.at.starts_with(day))
            .map(|record| record.total_tokens)
            .sum())
    }

    pub fn requests_for_minute(&self, minute: &str) -> Result<usize> {
        Ok(self
            .read_all()?
            .into_iter()
            .filter(|record| record.at.starts_with(minute))
            .count())
    }
}

impl ProviderHealthLedger {
    pub fn new(audit_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: audit_dir.into().join("provider-health.jsonl"),
        }
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn for_usage_ledger(ledger: &ModelUsageLedger) -> Self {
        let path = ledger
            .path()
            .parent()
            .map(|parent| parent.join("provider-health.jsonl"))
            .unwrap_or_else(|| PathBuf::from("provider-health.jsonl"));
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, mut record: ProviderHealthRecord) -> Result<()> {
        record.last_error_summary = redact_secrets(&record.last_error_summary);
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let encoded = serde_json::to_string(&record)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| IkarosError::io(&self.path, source))?;
        writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.path, source))
    }

    pub fn read_all(&self) -> Result<Vec<ProviderHealthRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file =
            fs::File::open(&self.path).map_err(|source| IkarosError::io(&self.path, source))?;
        let reader = BufReader::new(file);
        let mut records = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|source| IkarosError::io(&self.path, source))?;
            if !line.trim().is_empty() {
                records.push(serde_json::from_str(&line)?);
            }
        }
        Ok(records)
    }

    pub fn latest(&self, provider: &str, model: &str) -> Result<Option<ProviderHealthRecord>> {
        Ok(self
            .read_all()?
            .into_iter()
            .rev()
            .find(|record| record.provider == provider && record.model == model))
    }
}
