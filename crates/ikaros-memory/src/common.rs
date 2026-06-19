// SPDX-License-Identifier: GPL-3.0-only

use crate::{MemoryKind, MemoryQuery, MemoryRecord};

pub(crate) fn filter_records(records: Vec<MemoryRecord>, query: &MemoryQuery) -> Vec<MemoryRecord> {
    let needle = query.text.as_ref().map(|text| text.to_ascii_lowercase());
    let mut filtered = records
        .into_iter()
        .filter(|record| {
            query.kind.as_ref().is_none_or(|kind| &record.kind == kind)
                && query
                    .scope
                    .as_ref()
                    .is_none_or(|scope| &record.scope == scope)
                && query
                    .perspective
                    .as_ref()
                    .is_none_or(|perspective| record.perspective.as_ref() == Some(perspective))
                && needle.as_ref().is_none_or(|text| {
                    record.content.to_ascii_lowercase().contains(text)
                        || record
                            .tags
                            .iter()
                            .any(|tag| tag.to_ascii_lowercase().contains(text))
                })
        })
        .collect::<Vec<_>>();
    filtered.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    if let Some(limit) = query.limit {
        filtered.truncate(limit);
    }
    filtered
}

pub(crate) fn memory_kind_to_str(kind: &MemoryKind) -> &'static str {
    match kind {
        MemoryKind::User => "user",
        MemoryKind::Project => "project",
        MemoryKind::Task => "task",
        MemoryKind::Persona => "persona",
        MemoryKind::Relationship => "relationship",
        MemoryKind::Knowledge => "knowledge",
    }
}

pub(crate) fn memory_kind_from_str(kind: &str) -> Option<MemoryKind> {
    match kind {
        "user" => Some(MemoryKind::User),
        "project" => Some(MemoryKind::Project),
        "task" => Some(MemoryKind::Task),
        "persona" => Some(MemoryKind::Persona),
        "relationship" => Some(MemoryKind::Relationship),
        "knowledge" => Some(MemoryKind::Knowledge),
        _ => None,
    }
}
