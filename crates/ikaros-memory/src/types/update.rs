// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};

use super::MemoryRecord;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryUpdateSnapshot {
    pub content: String,
    pub tags: Vec<String>,
}

impl From<&MemoryRecord> for MemoryUpdateSnapshot {
    fn from(record: &MemoryRecord) -> Self {
        Self {
            content: record.content.clone(),
            tags: record.tags.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryChangeReport {
    pub id: String,
    pub found: bool,
    pub content_changed: bool,
    pub tags_changed: bool,
    pub changed_fields: Vec<String>,
    pub before: Option<MemoryUpdateSnapshot>,
    pub after: Option<MemoryUpdateSnapshot>,
}

impl MemoryChangeReport {
    pub fn from_records(
        id: impl Into<String>,
        before: Option<&MemoryRecord>,
        after: Option<&MemoryRecord>,
    ) -> Self {
        let content_changed = before
            .zip(after)
            .is_some_and(|(before, after)| before.content != after.content);
        let tags_changed = before
            .zip(after)
            .is_some_and(|(before, after)| before.tags != after.tags);
        let mut changed_fields = Vec::new();
        if content_changed {
            changed_fields.push("content".into());
        }
        if tags_changed {
            changed_fields.push("tags".into());
        }
        Self {
            id: id.into(),
            found: after.is_some(),
            content_changed,
            tags_changed,
            changed_fields,
            before: before.map(MemoryUpdateSnapshot::from),
            after: after.map(MemoryUpdateSnapshot::from),
        }
    }

    pub fn not_found(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            found: false,
            content_changed: false,
            tags_changed: false,
            changed_fields: Vec::new(),
            before: None,
            after: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryUpdateReport {
    pub record: MemoryRecord,
    pub change: MemoryChangeReport,
}

impl MemoryUpdateReport {
    pub fn from_before_after(before: &MemoryRecord, after: MemoryRecord) -> Self {
        let change = MemoryChangeReport::from_records(&after.id, Some(before), Some(&after));
        Self {
            record: after,
            change,
        }
    }
}
