// SPDX-License-Identifier: GPL-3.0-only

use super::MemoryWorkingCommand;
use super::journal::append_working_memory_expired_journal;
use super::parse::parse_memory_kind;
use anyhow::Result;
use ikaros_core::IkarosPaths;
use ikaros_state::memory::{JsonlWorkingMemoryStore, WorkingMemoryQuery};
use serde_json::json;

pub(super) fn memory_working_command(
    command: MemoryWorkingCommand,
    paths: &IkarosPaths,
) -> Result<()> {
    let store = JsonlWorkingMemoryStore::new(&paths.memory_dir);
    match command {
        MemoryWorkingCommand::List(args) => {
            let kind = args.kind.as_deref().map(parse_memory_kind).transpose()?;
            let records = store.list(WorkingMemoryQuery {
                session_id: args.session,
                kind,
                scope: args.scope,
                include_expired: args.include_expired,
                limit: Some(args.limit),
            })?;
            println!("{}", serde_json::to_string_pretty(&records)?);
        }
        MemoryWorkingCommand::Prune => {
            let expired = store.prune_expired()?;
            for record in &expired {
                append_working_memory_expired_journal(paths, record)?;
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "summary": "working memory pruned",
                    "expired_count": expired.len(),
                    "expired": expired,
                }))?
            );
        }
    }
    Ok(())
}
