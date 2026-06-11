// SPDX-License-Identifier: GPL-3.0-only

use super::{ChatHistoryRecord, ChatHistoryStore};
use ikaros_core::{IkarosError, Result};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
};

impl ChatHistoryStore {
    pub(super) fn append_jsonl(&self, record: &ChatHistoryRecord) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| IkarosError::io(&self.path, source))?;
        let encoded = serde_json::to_string(record)
            .map_err(|source| IkarosError::Message(format!("json serialize error: {source}")))?;
        writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.path, source))?;
        Ok(())
    }

    pub(super) fn read_all_jsonl(&self) -> Result<Vec<ChatHistoryRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file =
            fs::File::open(&self.path).map_err(|source| IkarosError::io(&self.path, source))?;
        let mut records = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line.map_err(|source| IkarosError::io(&self.path, source))?;
            if line.trim().is_empty() {
                continue;
            }
            let record = serde_json::from_str(&line).map_err(|source| {
                IkarosError::Message(format!(
                    "invalid chat history record at {}: {source}",
                    self.path.display()
                ))
            })?;
            records.push(record);
        }
        Ok(records)
    }

    fn write_all_jsonl(&self, records: &[ChatHistoryRecord]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)
            .map_err(|source| IkarosError::io(&self.path, source))?;
        for record in records {
            let encoded = serde_json::to_string(record).map_err(|source| {
                IkarosError::Message(format!("json serialize error: {source}"))
            })?;
            writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.path, source))?;
        }
        Ok(())
    }

    pub(super) fn delete_session_jsonl(&self, session_id: &str) -> Result<usize> {
        let records = self.read_all_jsonl()?;
        let before = records.len();
        let retained = records
            .into_iter()
            .filter(|record| record.session_id != session_id)
            .collect::<Vec<_>>();
        let deleted = before.saturating_sub(retained.len());
        if deleted > 0 {
            self.write_all_jsonl(&retained)?;
        }
        Ok(deleted)
    }

    pub(super) fn clear_jsonl(&self) -> Result<usize> {
        let count = self.read_all_jsonl()?.len();
        if count > 0 || self.path.exists() {
            self.write_all_jsonl(&[])?;
        }
        Ok(count)
    }
}
