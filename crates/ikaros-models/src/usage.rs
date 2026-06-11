// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result};
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
