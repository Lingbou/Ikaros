// SPDX-License-Identifier: GPL-3.0-only

use super::MemorySupersession;
use anyhow::Result;
use ikaros_core::{IkarosPaths, redact_json};
use ikaros_host::local_memory_store;
use ikaros_state::memory::{MemoryQuery, MemoryRecord, MemoryStore};
use serde_json::json;

pub(super) fn memory_supersession_command(
    args: MemorySupersession,
    paths: &IkarosPaths,
) -> Result<()> {
    let store = local_memory_store(paths)?;
    let records = store.list(MemoryQuery {
        include_inactive: true,
        limit: Some(usize::MAX),
        ..MemoryQuery::default()
    })?;
    let record = records
        .iter()
        .find(|record| record.id == args.id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("memory record not found: {}", args.id))?;
    let replaces = record
        .supersedes
        .iter()
        .filter_map(|id| memory_record_by_id(&records, id))
        .cloned()
        .collect::<Vec<_>>();
    let replaced_by = record
        .superseded_by
        .as_deref()
        .and_then(|id| memory_record_by_id(&records, id))
        .cloned();
    let status = memory_supersession_status(&record, replaced_by.as_ref());
    let chain = memory_supersession_chain(&record, &replaces, replaced_by.as_ref());
    let output = json!({
        "summary": "memory supersession explained",
        "query_id": args.id,
        "status": status,
        "record": record,
        "replaces": replaces,
        "replaced_by": replaced_by,
        "chain": chain,
    });
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

fn memory_record_by_id<'a>(records: &'a [MemoryRecord], id: &str) -> Option<&'a MemoryRecord> {
    records.iter().find(|record| record.id == id)
}

fn memory_supersession_status(
    record: &MemoryRecord,
    replaced_by: Option<&MemoryRecord>,
) -> &'static str {
    if record.active {
        "active"
    } else if replaced_by.is_some() {
        "superseded"
    } else {
        "inactive"
    }
}

fn memory_supersession_chain(
    record: &MemoryRecord,
    replaces: &[MemoryRecord],
    replaced_by: Option<&MemoryRecord>,
) -> Vec<serde_json::Value> {
    let mut chain = Vec::with_capacity(replaces.len() + 1 + usize::from(replaced_by.is_some()));
    for replaced in replaces {
        chain.push(json!({
            "role": "replaces",
            "id": replaced.id,
            "active": replaced.active,
            "valid_from": replaced.valid_from,
            "valid_until": replaced.valid_until,
            "content": replaced.content,
        }));
    }
    chain.push(json!({
        "role": "record",
        "id": record.id,
        "active": record.active,
        "valid_from": record.valid_from,
        "valid_until": record.valid_until,
        "content": record.content,
    }));
    if let Some(replacement) = replaced_by {
        chain.push(json!({
            "role": "replaced_by",
            "id": replacement.id,
            "active": replacement.active,
            "valid_from": replacement.valid_from,
            "valid_until": replacement.valid_until,
            "content": replacement.content,
        }));
    }
    chain
}
