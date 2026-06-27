// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    JsonlWorkingMemoryStore, LocalMemoryStore, MemoryKind, MemoryQuery, MemoryRecord, MemoryRef,
    MemoryStore, MemoryUpdateReport, WorkingMemoryRecord,
};
use ikaros_core::{Result, redact_secrets};
use std::path::{Path, PathBuf};

use super::{
    MemoryDelegationObservation, MemoryLifecycleRecordRef, MemoryLifecycleReport,
    MemoryPreCompressInput, MemoryPrefetchInput, MemoryProvider, MemoryProviderDescriptor,
    MemorySessionSwitch, MemoryTurnRecord, MemoryTurnStart,
};

impl MemoryProvider for LocalMemoryStore {
    fn descriptor(&self) -> MemoryProviderDescriptor {
        MemoryProviderDescriptor::active_local(self.backend_name(), self.path().to_path_buf())
    }

    fn turn_start(&self, input: MemoryTurnStart) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport {
            phase: "turn_start".into(),
            records_read: 0,
            records_written: 0,
            source_ref: None,
            records: Vec::new(),
            notes: vec![format!(
                "session={} agent={}",
                input.session_id.as_deref().unwrap_or("none"),
                input.agent_id.as_deref().unwrap_or("none")
            )],
        })
    }

    fn prefetch(&self, input: MemoryPrefetchInput) -> Result<Vec<MemoryRecord>> {
        MemoryStore::search(self, input.query)
    }

    fn sync_turn(&self, turn: MemoryTurnRecord) -> Result<MemoryLifecycleReport> {
        let Some(session_id) = turn.session_id.clone() else {
            return Ok(MemoryLifecycleReport {
                phase: "sync_turn".into(),
                records_read: 0,
                records_written: 0,
                source_ref: None,
                records: Vec::new(),
                notes: vec!["skipped: missing session id".into()],
            });
        };
        let source_ref = MemoryRef::SessionTurn {
            session_id: session_id.clone(),
            turn_id: turn.turn_id.clone(),
        };
        let content = turn_summary_content(&turn);
        if content.trim().is_empty() {
            return Ok(MemoryLifecycleReport {
                phase: "sync_turn".into(),
                records_read: 0,
                records_written: 0,
                source_ref: Some(source_ref),
                records: Vec::new(),
                notes: vec!["skipped: empty turn".into()],
            });
        }
        if content.contains("[REDACTED_SECRET]") {
            return Ok(MemoryLifecycleReport {
                phase: "sync_turn".into(),
                records_read: 0,
                records_written: 0,
                source_ref: Some(source_ref),
                records: Vec::new(),
                notes: vec!["skipped: redacted secret marker present".into()],
            });
        }
        let working = JsonlWorkingMemoryStore::new(memory_dir_for_store_path(self.path()));
        let record = WorkingMemoryRecord::new(
            session_id.clone(),
            MemoryKind::Task,
            session_id.clone(),
            content,
            Some(24),
        )?
        .with_tags(vec!["turn-summary".into(), "memory-lifecycle".into()])?
        .with_source_ref(source_ref.clone())?;
        let record = working.append(record)?;
        Ok(MemoryLifecycleReport {
            phase: "sync_turn".into(),
            records_read: 0,
            records_written: 1,
            source_ref: Some(source_ref),
            records: vec![MemoryLifecycleRecordRef {
                id: record.id,
                kind: record.kind,
                scope: record.scope,
                source_ref: record.source_ref,
                confidence: None,
            }],
            notes: vec!["working_memory_written".into()],
        })
    }

    fn pre_compress(&self, input: MemoryPreCompressInput) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport {
            phase: "pre_compress".into(),
            records_read: 0,
            records_written: 0,
            source_ref: None,
            records: Vec::new(),
            notes: vec![format!("budget_tokens={}", input.budget_tokens)],
        })
    }

    fn session_switch(&self, input: MemorySessionSwitch) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport {
            phase: "session_switch".into(),
            records_read: 0,
            records_written: 0,
            source_ref: None,
            records: Vec::new(),
            notes: vec![format!(
                "from={} to={}",
                input.from_session_id.as_deref().unwrap_or("none"),
                input.to_session_id.as_deref().unwrap_or("none")
            )],
        })
    }

    fn delegation_observation(
        &self,
        input: MemoryDelegationObservation,
    ) -> Result<MemoryLifecycleReport> {
        if input.summary.trim().is_empty() {
            return Ok(MemoryLifecycleReport {
                phase: "delegation_observation".into(),
                records_read: 0,
                records_written: 0,
                source_ref: None,
                records: Vec::new(),
                notes: vec!["skipped: empty summary".into()],
            });
        }
        let scope = input
            .parent_agent_id
            .clone()
            .unwrap_or_else(|| "delegation".into());
        let record = MemoryRecord::new(
            MemoryKind::Task,
            scope,
            redact_secrets(&format!(
                "Delegation observation from child_agent={}: {}",
                input.child_agent_id.as_deref().unwrap_or("unknown"),
                input.summary
            )),
        )?
        .with_tags(vec![
            "delegation-observation".into(),
            "memory-lifecycle".into(),
        ])
        .with_source("memory_lifecycle");
        let record = MemoryStore::append(self, record)?;
        Ok(MemoryLifecycleReport {
            phase: "delegation_observation".into(),
            records_read: 0,
            records_written: 1,
            source_ref: None,
            records: vec![MemoryLifecycleRecordRef::from(&record)],
            notes: Vec::new(),
        })
    }

    fn append(&self, record: MemoryRecord) -> Result<MemoryRecord> {
        MemoryStore::append(self, record)
    }

    fn search(&self, query: MemoryQuery) -> Result<Vec<MemoryRecord>> {
        MemoryStore::search(self, query)
    }

    fn update(
        &self,
        id: &str,
        content: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<Option<MemoryUpdateReport>> {
        MemoryStore::update(self, id, content, tags)
    }

    fn delete_by_id(&self, id: &str) -> Result<bool> {
        MemoryStore::delete_by_id(self, id)
    }

    fn delete_scope(&self, query_kind: Option<MemoryKind>, scope: &str) -> Result<usize> {
        MemoryStore::delete_scope(self, query_kind, scope)
    }
}

fn memory_dir_for_store_path(path: &Path) -> PathBuf {
    path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn turn_summary_content(turn: &MemoryTurnRecord) -> String {
    let user = truncate_memory_text(&redact_secrets(&turn.user_input), 1_200);
    let assistant = truncate_memory_text(&redact_secrets(&turn.assistant_output), 1_200);
    format!("Turn summary\nuser: {user}\nassistant: {assistant}")
}

fn truncate_memory_text(text: &str, max_chars: usize) -> String {
    let mut output = text.chars().take(max_chars).collect::<String>();
    if text.chars().count() > max_chars {
        output.push_str("... [truncated]");
    }
    output
}
