// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{MemoryPolicyConfig, Result};
use ikaros_memory::{
    MemoryJournal, MemoryJournalAction, MemoryJournalEntry, MemoryKind, MemoryLifecycleRecordRef,
    MemoryLifecycleReport, MemoryPolicy, MemoryPolicyDecision, MemoryPolicyEngine, MemoryProvider,
    MemoryQuery, MemoryRecord, add_policy_tag,
};

pub fn memory_policy_from_config(config: &MemoryPolicyConfig) -> MemoryPolicy {
    MemoryPolicy {
        promote_threshold: config.promote_threshold,
        demote_threshold: config.demote_threshold,
        forget_threshold: config.forget_threshold,
        max_records_per_scope: config.max_records_per_scope,
    }
}

pub fn apply_runtime_memory_policy(
    provider: &dyn MemoryProvider,
    journal: &dyn MemoryJournal,
    policy: &MemoryPolicy,
    report: &MemoryLifecycleReport,
) -> Result<Vec<MemoryJournalEntry>> {
    let mut entries = append_memory_sync_journal(provider, journal, policy, report)?;
    if report.phase != "sync_turn" {
        return Ok(entries);
    }
    let engine = MemoryPolicyEngine::new(policy.clone());
    let trigger_ref = report.source_ref.clone();
    let mut affected_scopes = Vec::<(MemoryKind, String)>::new();

    for record_ref in &report.records {
        push_affected_scope(
            &mut affected_scopes,
            record_ref.kind.clone(),
            &record_ref.scope,
        );
    }
    for (kind, scope) in affected_scopes {
        let mut records_in_scope = scope_records(provider, kind.clone(), &scope)?;
        for record in records_in_scope.clone() {
            if let Some(decision) = engine.classify_record(&record, &records_in_scope) {
                if decision.action == MemoryJournalAction::Promote
                    || decision.action == MemoryJournalAction::Demote
                {
                    let tag = match decision.action {
                        MemoryJournalAction::Promote => "policy-promoted",
                        MemoryJournalAction::Demote => "policy-demoted",
                        _ => unreachable!(),
                    };
                    provider.update(&record.id, None, Some(add_policy_tag(&record.tags, tag)))?;
                } else if decision.action == MemoryJournalAction::Forget {
                    provider.delete_by_id(&record.id)?;
                }
                entries.push(append_memory_decision_journal(
                    journal,
                    &record,
                    decision,
                    trigger_ref.clone().or_else(|| record.source_ref.clone()),
                )?);
            }
        }

        records_in_scope = scope_records(provider, kind, &scope)?;
        for (record, score) in engine.quota_victims(&records_in_scope) {
            provider.delete_by_id(&record.id)?;
            entries.push(append_memory_decision_journal(
                journal,
                &record,
                MemoryPolicyDecision {
                    action: MemoryJournalAction::Forget,
                    score,
                    reason: "quota removed lower score memory".into(),
                },
                trigger_ref.clone().or_else(|| record.source_ref.clone()),
            )?);
        }
    }

    Ok(entries)
}

fn append_memory_sync_journal(
    provider: &dyn MemoryProvider,
    journal: &dyn MemoryJournal,
    policy: &MemoryPolicy,
    report: &MemoryLifecycleReport,
) -> Result<Vec<MemoryJournalEntry>> {
    if report.phase != "sync_turn" {
        return Ok(Vec::new());
    }
    let skipped_note = report
        .notes
        .iter()
        .find(|note| note.to_ascii_lowercase().contains("skipped"));
    if report.records_written > 0 {
        let mut entries = Vec::new();
        if report
            .notes
            .iter()
            .any(|note| note == "working_memory_written")
        {
            let mut entry = MemoryJournalEntry::new(
                MemoryJournalAction::Append,
                "sync_turn wrote working memory",
            )?;
            if let Some(source_ref) = report.source_ref.clone() {
                entry = entry.with_source_ref(source_ref)?;
            }
            entries.push(journal.append(entry)?);
        }
        for record_ref in &report.records {
            let Some(record) = find_memory_record(provider, record_ref)? else {
                continue;
            };
            let scope_records = scope_records(provider, record.kind.clone(), &record.scope)?;
            let score =
                MemoryPolicyEngine::new(policy.clone()).score_record(&record, &scope_records);
            entries.push(append_memory_entry_journal(
                journal,
                MemoryJournalAction::Append,
                "sync_turn wrote core memory record",
                &record,
                Some(score),
                record
                    .source_ref
                    .clone()
                    .or_else(|| report.source_ref.clone()),
            )?);
        }
        return Ok(entries);
    }
    if let Some(note) = skipped_note {
        let reason = if note.to_ascii_lowercase().contains("redacted")
            || note.to_ascii_lowercase().contains("secret")
        {
            "sync_turn skipped because redaction marker was present".to_owned()
        } else {
            format!("sync_turn {note}")
        };
        let mut entry = MemoryJournalEntry::new(MemoryJournalAction::Skip, reason)?;
        if let Some(source_ref) = report.source_ref.clone() {
            entry = entry.with_source_ref(source_ref)?;
        }
        return journal.append(entry).map(|entry| vec![entry]);
    }
    Ok(Vec::new())
}

fn append_memory_decision_journal(
    journal: &dyn MemoryJournal,
    record: &MemoryRecord,
    decision: MemoryPolicyDecision,
    source_ref: Option<ikaros_memory::MemoryRef>,
) -> Result<MemoryJournalEntry> {
    append_memory_entry_journal(
        journal,
        decision.action,
        decision.reason,
        record,
        Some(decision.score),
        source_ref,
    )
}

fn append_memory_entry_journal(
    journal: &dyn MemoryJournal,
    action: MemoryJournalAction,
    reason: impl Into<String>,
    record: &MemoryRecord,
    score: Option<ikaros_memory::MemoryScore>,
    source_ref: Option<ikaros_memory::MemoryRef>,
) -> Result<MemoryJournalEntry> {
    let mut entry = MemoryJournalEntry::new(action, reason)?.with_memory(
        &record.id,
        record.kind.clone(),
        &record.scope,
    )?;
    if let Some(score) = score {
        entry = entry.with_score(score);
    }
    if let Some(source_ref) = source_ref {
        entry = entry.with_source_ref(source_ref)?;
    }
    journal.append(entry)
}

fn find_memory_record(
    provider: &dyn MemoryProvider,
    record_ref: &MemoryLifecycleRecordRef,
) -> Result<Option<MemoryRecord>> {
    Ok(
        scope_records(provider, record_ref.kind.clone(), &record_ref.scope)?
            .into_iter()
            .find(|record| record.id == record_ref.id),
    )
}

fn scope_records(
    provider: &dyn MemoryProvider,
    kind: MemoryKind,
    scope: &str,
) -> Result<Vec<MemoryRecord>> {
    provider.search(MemoryQuery {
        kind: Some(kind),
        scope: Some(scope.to_owned()),
        perspective: None,
        text: None,
        limit: Some(usize::MAX),
        include_inactive: false,
    })
}

fn push_affected_scope(scopes: &mut Vec<(MemoryKind, String)>, kind: MemoryKind, scope: &str) {
    let item = (kind, scope.to_owned());
    if !scopes.contains(&item) {
        scopes.push(item);
    }
}
