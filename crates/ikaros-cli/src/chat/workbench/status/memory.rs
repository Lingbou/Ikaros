// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths, redact_secrets};
use ikaros_memory::{
    JsonlMemoryCandidateStore, JsonlMemoryJournal, JsonlWorkingMemoryStore, LocalMemoryStore,
    MemoryCandidateQuery, MemoryCandidateStatus, MemoryJournal, MemoryKind, MemoryQuery,
    MemoryRecord, MemoryStore, WorkingMemoryQuery,
};
use ikaros_session::AgentEventKind;
use std::fs;

use super::super::{WorkbenchCell, WorkbenchCellKind, path_display, terminal_inline};

pub(in crate::chat) fn print_memory_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    println!(
        "memory_backend: {}",
        terminal_inline(&config.memory.backend)
    );
    println!("memory_dir: {}", path_display(&paths.memory_dir));
    println!(
        "memory_context_enabled: {}",
        runtime.agent.profile.memory_context
    );
    println!(
        "memory_policy: promote={} demote={} forget={} max_records_per_scope={}",
        config.memory.policy.promote_threshold,
        config.memory.policy.demote_threshold,
        config.memory.policy.forget_threshold,
        config.memory.policy.max_records_per_scope
    );
    println!(
        "memory_external_providers: {}",
        config.memory.external_providers.len()
    );
    println!(
        "- {}",
        WorkbenchCell {
            kind: WorkbenchCellKind::Memory,
            title: "memory status".into(),
            detail: format!(
                "backend={} context_enabled={} external_providers={}",
                terminal_inline(&config.memory.backend),
                runtime.agent.profile.memory_context,
                config.memory.external_providers.len()
            ),
        }
        .render()
    );
    println!("{}", memory_status_json_line(config, paths, runtime)?);
    print_memory_projection_explain(config, paths, runtime)?;
    print_memory_projection_cells(paths)?;
    print_memory_candidate_cells(paths)?;
    print_memory_supersession_cells(config, paths)?;
    print_working_memory_cells(paths, runtime)?;
    super::print_filtered_event_cells(runtime, "memory", |kind| {
        matches!(kind, AgentEventKind::MemoryLifecycle)
    })?;
    print_memory_journal_cells(paths)?;
    Ok(())
}

pub(super) fn screen_memory_cell(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<WorkbenchCell> {
    let explain = memory_projection_explain(config, paths, runtime)?;
    Ok(WorkbenchCell {
        kind: WorkbenchCellKind::Memory,
        title: "memory".into(),
        detail: format!(
            "backend={} context_enabled={} projection_files={} projection_included={} projection_excluded={} pending_candidates={} working_active={} journal_entries={} command=/debug memory-lifecycle {} memory=/memory",
            terminal_inline(&config.memory.backend),
            runtime.agent.profile.memory_context,
            memory_projection_file_count(paths)?,
            explain.included_count,
            explain.excluded_count,
            pending_memory_candidate_count(paths)?,
            working_memory_record_count(paths, runtime)?,
            JsonlMemoryJournal::new(&paths.memory_dir).list()?.len(),
            terminal_inline(&runtime.chat_session_id),
        ),
    })
}

fn memory_status_json_line(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<String> {
    let projection_files = memory_projection_file_count(paths)?;
    let pending_candidates = pending_memory_candidate_count(paths)?;
    let superseded_records = superseded_memory_record_count(config, paths)?;
    let working_active = working_memory_record_count(paths, runtime)?;
    let journal_entries = JsonlMemoryJournal::new(&paths.memory_dir).list()?.len();
    let projection_explain = memory_projection_explain(config, paths, runtime)?;
    let payload = serde_json::json!({
        "schema": "ikaros-workbench-memory-status-v1",
        "version": 1,
        "session_id": terminal_inline(&runtime.chat_session_id),
        "backend": terminal_inline(&config.memory.backend),
        "memory_dir": path_display(&paths.memory_dir),
        "context_enabled": runtime.agent.profile.memory_context,
        "policy": {
            "promote_threshold": config.memory.policy.promote_threshold,
            "demote_threshold": config.memory.policy.demote_threshold,
            "forget_threshold": config.memory.policy.forget_threshold,
            "max_records_per_scope": config.memory.policy.max_records_per_scope,
        },
        "external_providers": config.memory.external_providers.len(),
        "counts": {
            "projection_files": projection_files,
            "pending_candidates": pending_candidates,
            "superseded_records": superseded_records,
            "working_active": working_active,
            "journal_entries": journal_entries,
        },
        "projection_explain": projection_explain.to_json(),
        "actions": {
            "projection_render": "memory projection render",
            "candidate_list": "memory candidate list",
            "supersession": "memory supersession <memory-id>",
            "memory_lifecycle": format!("debug memory-lifecycle {}", terminal_inline(&runtime.chat_session_id)),
        },
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        r#"{"schema":"ikaros-workbench-memory-status-v1","version":1,"error":"serialization_failed"}"#
            .to_owned()
    });
    Ok(format!("memory_status_json: {encoded}"))
}

#[derive(Debug, Clone)]
struct MemoryProjectionExplain {
    user_scope: String,
    project_scope: Option<String>,
    total_records: usize,
    included_count: usize,
    excluded_count: usize,
    included_user: usize,
    included_project: usize,
    included_general: usize,
    included: Vec<serde_json::Value>,
    excluded: Vec<serde_json::Value>,
}

impl MemoryProjectionExplain {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "user_scope": terminal_inline(&self.user_scope),
            "project_scope": self.project_scope.as_deref().map(terminal_inline),
            "total_records": self.total_records,
            "included_count": self.included_count,
            "excluded_count": self.excluded_count,
            "included_by_bucket": {
                "user": self.included_user,
                "project": self.included_project,
                "general": self.included_general,
            },
            "included": self.included,
            "excluded": self.excluded,
        })
    }
}

fn memory_projection_explain(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<MemoryProjectionExplain> {
    let store = LocalMemoryStore::new(&paths.memory_dir, &config.memory.backend)?;
    let mut records = store.list(MemoryQuery {
        include_inactive: true,
        limit: Some(usize::MAX),
        ..MemoryQuery::default()
    })?;
    records.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    let user_scope = "default".to_owned();
    let project_scope = runtime
        .workspace
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .filter(|scope| !scope.trim().is_empty());

    let mut report = MemoryProjectionExplain {
        user_scope: user_scope.clone(),
        project_scope: project_scope.clone(),
        total_records: records.len(),
        included_count: 0,
        excluded_count: 0,
        included_user: 0,
        included_project: 0,
        included_general: 0,
        included: Vec::new(),
        excluded: Vec::new(),
    };

    for record in &records {
        let decision = projection_record_decision(record, &user_scope, project_scope.as_deref());
        if decision.included {
            report.included_count += 1;
            match decision.bucket {
                Some("user") => report.included_user += 1,
                Some("project") => report.included_project += 1,
                Some("general") => report.included_general += 1,
                _ => {}
            }
            if report.included.len() < 10 {
                report.included.push(memory_projection_record_json(
                    record,
                    decision.reason,
                    decision.bucket,
                ));
            }
        } else {
            report.excluded_count += 1;
            if report.excluded.len() < 10 {
                report
                    .excluded
                    .push(memory_projection_record_json(record, decision.reason, None));
            }
        }
    }
    Ok(report)
}

#[derive(Debug, Clone, Copy)]
struct ProjectionRecordDecision {
    included: bool,
    reason: &'static str,
    bucket: Option<&'static str>,
}

fn projection_record_decision(
    record: &MemoryRecord,
    user_scope: &str,
    project_scope: Option<&str>,
) -> ProjectionRecordDecision {
    if !record.active {
        return projection_excluded("inactive");
    }
    if record.sensitive {
        return projection_excluded("sensitive");
    }
    if record.kind == MemoryKind::Task {
        return projection_excluded("task_memory_is_episode_history");
    }
    if record.perspective.is_some() {
        return projection_excluded("perspective_mismatch");
    }
    if record.tags.iter().any(|tag| {
        matches!(
            tag.as_str(),
            "turn-summary" | "memory-lifecycle" | "policy-demoted"
        )
    }) {
        return projection_excluded("projection_excluded_tag");
    }
    if matches!(record.kind, MemoryKind::User | MemoryKind::Relationship)
        && record.scope == user_scope
    {
        return projection_included("included_user_scope", "user");
    }
    if record.kind == MemoryKind::Project
        && project_scope.is_some_and(|scope| record.scope == scope)
    {
        return projection_included("included_project_scope", "project");
    }
    if matches!(record.kind, MemoryKind::Knowledge | MemoryKind::Persona)
        || (record.kind == MemoryKind::Project && project_scope.is_none())
    {
        return projection_included("included_general_memory", "general");
    }
    projection_excluded("scope_not_selected")
}

fn projection_included(reason: &'static str, bucket: &'static str) -> ProjectionRecordDecision {
    ProjectionRecordDecision {
        included: true,
        reason,
        bucket: Some(bucket),
    }
}

fn projection_excluded(reason: &'static str) -> ProjectionRecordDecision {
    ProjectionRecordDecision {
        included: false,
        reason,
        bucket: None,
    }
}

fn memory_projection_record_json(
    record: &MemoryRecord,
    reason: &str,
    bucket: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "id": terminal_inline(&record.id),
        "kind": format!("{:?}", record.kind),
        "scope": terminal_inline(&record.scope),
        "active": record.active,
        "sensitive": record.sensitive,
        "bucket": bucket,
        "reason": reason,
        "source": record.source.as_deref().map(terminal_inline),
        "source_ref": record.source_ref.as_ref().map(|source| terminal_inline(&format!("{source:?}"))),
        "snippet": memory_terminal_snippet(&record.content),
    })
}

fn print_memory_projection_explain(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    let explain = memory_projection_explain(config, paths, runtime)?;
    println!(
        "memory_projection_explain: total={} included={} excluded={} user_scope={} project_scope={}",
        explain.total_records,
        explain.included_count,
        explain.excluded_count,
        terminal_inline(&explain.user_scope),
        explain
            .project_scope
            .as_deref()
            .map(terminal_inline)
            .unwrap_or_else(|| "none".into())
    );
    println!(
        "memory_projection_explain_json: {}",
        serde_json::to_string(&explain.to_json())
            .unwrap_or_else(|_| { r#"{"error":"serialization_failed"}"#.to_owned() })
    );
    Ok(())
}

fn memory_projection_file_count(paths: &IkarosPaths) -> Result<usize> {
    let projection_dir = paths.memory_dir.join("projections");
    if !projection_dir.exists() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in fs::read_dir(&projection_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            count += 1;
        }
    }
    Ok(count)
}

fn pending_memory_candidate_count(paths: &IkarosPaths) -> Result<usize> {
    let store = JsonlMemoryCandidateStore::new(&paths.memory_dir);
    Ok(store
        .list(MemoryCandidateQuery {
            status: Some(MemoryCandidateStatus::Pending),
            limit: None,
            ..MemoryCandidateQuery::default()
        })?
        .len())
}

fn superseded_memory_record_count(config: &IkarosConfig, paths: &IkarosPaths) -> Result<usize> {
    Ok(superseded_memory_records(config, paths)?
        .into_iter()
        .filter(|record| !record.active && record.superseded_by.is_some())
        .count())
}

fn working_memory_record_count(
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<usize> {
    let store = JsonlWorkingMemoryStore::new(&paths.memory_dir);
    Ok(store
        .list(WorkingMemoryQuery {
            session_id: Some(runtime.chat_session_id.clone()),
            limit: None,
            ..WorkingMemoryQuery::default()
        })?
        .len())
}

fn superseded_memory_records(
    config: &IkarosConfig,
    paths: &IkarosPaths,
) -> Result<Vec<MemoryRecord>> {
    let store = LocalMemoryStore::new(&paths.memory_dir, &config.memory.backend)?;
    let mut records = store.list(MemoryQuery {
        include_inactive: true,
        limit: Some(usize::MAX),
        ..MemoryQuery::default()
    })?;
    records.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    Ok(records)
}

fn print_memory_projection_cells(paths: &IkarosPaths) -> Result<()> {
    let projection_dir = paths.memory_dir.join("projections");
    let mut files = Vec::new();
    if projection_dir.exists() {
        for entry in fs::read_dir(&projection_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
                files.push(path);
            }
        }
    }
    files.sort();
    println!("memory_projection_files: {}", files.len());
    for path in files.iter().take(5) {
        let content = fs::read_to_string(path).unwrap_or_default();
        let detail = format!(
            "file={} snippet={}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown"),
            memory_terminal_snippet(&content)
        );
        println!(
            "- {}",
            WorkbenchCell {
                kind: WorkbenchCellKind::Memory,
                title: "projection".into(),
                detail,
            }
            .render()
        );
    }
    Ok(())
}

fn print_memory_supersession_cells(config: &IkarosConfig, paths: &IkarosPaths) -> Result<()> {
    let records = superseded_memory_records(config, paths)?;
    let superseded = records
        .iter()
        .filter(|record| !record.active && record.superseded_by.is_some())
        .take(5)
        .collect::<Vec<_>>();
    println!("memory_superseded_records: {}", superseded.len());
    for record in superseded {
        let replacement_id = record
            .superseded_by
            .as_deref()
            .unwrap_or(record.id.as_str());
        let detail = format!(
            "kind={:?} scope={} replaced_by={} command=memory supersession {} snippet={}",
            record.kind,
            terminal_inline(&record.scope),
            terminal_inline(replacement_id),
            terminal_inline(replacement_id),
            memory_terminal_snippet(&record.content)
        );
        println!(
            "- {}",
            WorkbenchCell {
                kind: WorkbenchCellKind::Memory,
                title: "superseded memory".into(),
                detail,
            }
            .render()
        );
    }
    Ok(())
}

fn print_memory_candidate_cells(paths: &IkarosPaths) -> Result<()> {
    let store = JsonlMemoryCandidateStore::new(&paths.memory_dir);
    let candidates = store.list(MemoryCandidateQuery {
        status: Some(MemoryCandidateStatus::Pending),
        limit: Some(5),
        ..MemoryCandidateQuery::default()
    })?;
    println!("memory_candidates_pending: {}", candidates.len());
    for candidate in candidates {
        let detail = format!(
            "kind={:?} scope={} confidence={} snippet={}",
            candidate.kind,
            terminal_inline(&candidate.scope),
            candidate.confidence,
            memory_terminal_snippet(&candidate.content)
        );
        println!(
            "- {}",
            WorkbenchCell {
                kind: WorkbenchCellKind::Memory,
                title: "candidate pending".into(),
                detail,
            }
            .render()
        );
    }
    Ok(())
}

fn print_working_memory_cells(paths: &IkarosPaths, runtime: &InteractiveChatRuntime) -> Result<()> {
    let store = JsonlWorkingMemoryStore::new(&paths.memory_dir);
    let records = store.list(WorkingMemoryQuery {
        session_id: Some(runtime.chat_session_id.clone()),
        limit: Some(5),
        ..WorkingMemoryQuery::default()
    })?;
    println!("memory_working_active: {}", records.len());
    for record in records {
        let detail = format!(
            "kind={:?} scope={} snippet={}",
            record.kind,
            terminal_inline(&record.scope),
            memory_terminal_snippet(&record.content)
        );
        println!(
            "- {}",
            WorkbenchCell {
                kind: WorkbenchCellKind::Memory,
                title: "working memory".into(),
                detail,
            }
            .render()
        );
    }
    Ok(())
}

fn memory_terminal_snippet(content: &str) -> String {
    let snippet = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .take(3)
        .collect::<Vec<_>>()
        .join(" | ");
    let snippet = if snippet.is_empty() {
        content.trim().lines().next().unwrap_or_default().to_owned()
    } else {
        snippet
    };
    terminal_inline(&truncate_memory_snippet(&redact_secrets(&snippet)))
}

fn truncate_memory_snippet(snippet: &str) -> String {
    const MAX_CHARS: usize = 180;
    let mut truncated = String::new();
    for (index, ch) in snippet.chars().enumerate() {
        if index >= MAX_CHARS {
            truncated.push_str("...");
            return truncated;
        }
        truncated.push(ch);
    }
    truncated
}

fn print_memory_journal_cells(paths: &IkarosPaths) -> Result<()> {
    let journal = JsonlMemoryJournal::new(&paths.memory_dir);
    let entries = journal.list()?;
    println!("memory_journal_entries: {}", entries.len());
    let start = entries.len().saturating_sub(5);
    for entry in &entries[start..] {
        let title = format!("journal {:?}", entry.action);
        let detail = format!(
            "scope={} reason={} source={}",
            entry.scope.as_deref().unwrap_or("none"),
            terminal_inline(&entry.reason),
            entry
                .source_ref
                .as_ref()
                .map(|source| format!("{source:?}"))
                .unwrap_or_else(|| "none".into())
        );
        println!(
            "- {}",
            WorkbenchCell {
                kind: WorkbenchCellKind::Memory,
                title,
                detail,
            }
            .render()
        );
    }
    Ok(())
}
