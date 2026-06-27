// SPDX-License-Identifier: GPL-3.0-only

use crate::types::{ProviderErrorKind, ProviderHealthStatus};
use ikaros_core::{IkarosError, Result, redact_secrets};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u32>,
    pub estimated: bool,
}

#[derive(Debug, Clone)]
pub struct ModelUsageLedger {
    path: PathBuf,
    cache: Arc<Mutex<ModelUsageCache>>,
}

#[derive(Debug, Clone, Default)]
struct ModelUsageCache {
    loaded: bool,
    file_len: u64,
    records: Vec<ModelUsageRecord>,
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
    cache: Arc<Mutex<ProviderHealthCache>>,
}

#[derive(Debug, Clone, Default)]
struct ProviderHealthCache {
    loaded: bool,
    file_len: u64,
    records: Vec<ProviderHealthRecord>,
}

impl ModelUsageLedger {
    pub fn new(audit_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: audit_dir.into().join("model-usage.jsonl"),
            cache: Arc::default(),
        }
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            cache: Arc::default(),
        }
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
        writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.path, source))?;
        let file_len = file
            .metadata()
            .map_err(|source| IkarosError::io(&self.path, source))?
            .len();
        self.update_cache_after_append(record, file_len)
    }

    pub fn read_all(&self) -> Result<Vec<ModelUsageRecord>> {
        self.cached_records()
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

    fn update_cache_after_append(&self, record: ModelUsageRecord, file_len: u64) -> Result<()> {
        let mut cache = self.cache.lock().map_err(|_| {
            IkarosError::Message(format!(
                "model usage cache lock poisoned for {}",
                self.path.display()
            ))
        })?;
        if cache.loaded {
            cache.records.push(record);
        } else {
            cache.records = read_usage_records_from_file(&self.path)?.0;
        }
        cache.loaded = true;
        cache.file_len = file_len;
        Ok(())
    }

    fn cached_records(&self) -> Result<Vec<ModelUsageRecord>> {
        let file_len = match fs::metadata(&self.path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut cache = self.cache.lock().map_err(|_| {
                    IkarosError::Message(format!(
                        "model usage cache lock poisoned for {}",
                        self.path.display()
                    ))
                })?;
                cache.loaded = true;
                cache.file_len = 0;
                cache.records.clear();
                return Ok(Vec::new());
            }
            Err(source) => return Err(IkarosError::io(&self.path, source)),
        };

        {
            let cache = self.cache.lock().map_err(|_| {
                IkarosError::Message(format!(
                    "model usage cache lock poisoned for {}",
                    self.path.display()
                ))
            })?;
            if cache.loaded && cache.file_len == file_len {
                return Ok(cache.records.clone());
            }
        }

        match read_usage_records_from_file(&self.path) {
            Ok((records, loaded_len)) => {
                let mut cache = self.cache.lock().map_err(|_| {
                    IkarosError::Message(format!(
                        "model usage cache lock poisoned for {}",
                        self.path.display()
                    ))
                })?;
                cache.loaded = true;
                cache.file_len = loaded_len;
                cache.records = records.clone();
                Ok(records)
            }
            Err(error) => {
                let cache = self.cache.lock().map_err(|_| {
                    IkarosError::Message(format!(
                        "model usage cache lock poisoned for {}",
                        self.path.display()
                    ))
                })?;
                if cache.loaded {
                    Ok(cache.records.clone())
                } else {
                    Err(error)
                }
            }
        }
    }
}

fn read_usage_records_from_file(path: &Path) -> Result<(Vec<ModelUsageRecord>, u64)> {
    if !path.exists() {
        return Ok((Vec::new(), 0));
    }
    let file = fs::File::open(path).map_err(|source| IkarosError::io(path, source))?;
    let loaded_len = file
        .metadata()
        .map_err(|source| IkarosError::io(path, source))?
        .len();
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|source| IkarosError::io(path, source))?;
        if !line.trim().is_empty() {
            records.push(serde_json::from_str(&line)?);
        }
    }
    Ok((records, loaded_len))
}

impl ProviderHealthLedger {
    pub fn new(audit_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: audit_dir.into().join("provider-health.jsonl"),
            cache: Arc::default(),
        }
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            cache: Arc::default(),
        }
    }

    pub fn for_usage_ledger(ledger: &ModelUsageLedger) -> Self {
        let path = ledger
            .path()
            .parent()
            .map(|parent| parent.join("provider-health.jsonl"))
            .unwrap_or_else(|| PathBuf::from("provider-health.jsonl"));
        Self {
            path,
            cache: Arc::default(),
        }
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
        writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.path, source))?;
        let file_len = file
            .metadata()
            .map_err(|source| IkarosError::io(&self.path, source))?
            .len();
        self.update_cache_after_append(record, file_len)
    }

    pub fn read_all(&self) -> Result<Vec<ProviderHealthRecord>> {
        self.cached_records()
    }

    pub fn latest(&self, provider: &str, model: &str) -> Result<Option<ProviderHealthRecord>> {
        Ok(self
            .read_all()?
            .into_iter()
            .rev()
            .find(|record| record.provider == provider && record.model == model))
    }

    fn update_cache_after_append(&self, record: ProviderHealthRecord, file_len: u64) -> Result<()> {
        let mut cache = self.cache.lock().map_err(|_| {
            IkarosError::Message(format!(
                "provider health cache lock poisoned for {}",
                self.path.display()
            ))
        })?;
        if cache.loaded {
            cache.records.push(record);
        } else {
            cache.records = read_health_records_from_file(&self.path)?.0;
        }
        cache.loaded = true;
        cache.file_len = file_len;
        Ok(())
    }

    fn cached_records(&self) -> Result<Vec<ProviderHealthRecord>> {
        let file_len = match fs::metadata(&self.path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut cache = self.cache.lock().map_err(|_| {
                    IkarosError::Message(format!(
                        "provider health cache lock poisoned for {}",
                        self.path.display()
                    ))
                })?;
                cache.loaded = true;
                cache.file_len = 0;
                cache.records.clear();
                return Ok(Vec::new());
            }
            Err(source) => return Err(IkarosError::io(&self.path, source)),
        };

        {
            let cache = self.cache.lock().map_err(|_| {
                IkarosError::Message(format!(
                    "provider health cache lock poisoned for {}",
                    self.path.display()
                ))
            })?;
            if cache.loaded && cache.file_len == file_len {
                return Ok(cache.records.clone());
            }
        }

        match read_health_records_from_file(&self.path) {
            Ok((records, loaded_len)) => {
                let mut cache = self.cache.lock().map_err(|_| {
                    IkarosError::Message(format!(
                        "provider health cache lock poisoned for {}",
                        self.path.display()
                    ))
                })?;
                cache.loaded = true;
                cache.file_len = loaded_len;
                cache.records = records.clone();
                Ok(records)
            }
            Err(error) => {
                let cache = self.cache.lock().map_err(|_| {
                    IkarosError::Message(format!(
                        "provider health cache lock poisoned for {}",
                        self.path.display()
                    ))
                })?;
                if cache.loaded {
                    Ok(cache.records.clone())
                } else {
                    Err(error)
                }
            }
        }
    }
}

fn read_health_records_from_file(path: &Path) -> Result<(Vec<ProviderHealthRecord>, u64)> {
    if !path.exists() {
        return Ok((Vec::new(), 0));
    }
    let file = fs::File::open(path).map_err(|source| IkarosError::io(path, source))?;
    let loaded_len = file
        .metadata()
        .map_err(|source| IkarosError::io(path, source))?
        .len();
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|source| IkarosError::io(path, source))?;
        if !line.trim().is_empty() {
            records.push(serde_json::from_str(&line)?);
        }
    }
    Ok((records, loaded_len))
}
