// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_state::memory::{LocalMemoryStore, MemoryKind, MemoryQuery, MemoryRecord, MemoryStore};

use super::super::super::terminal_inline;
use super::value::memory_terminal_snippet;

#[derive(Debug, Clone)]
pub(super) struct MemoryProjectionExplain {
    pub(super) user_scope: String,
    pub(super) project_scope: Option<String>,
    pub(super) total_records: usize,
    pub(super) included_count: usize,
    pub(super) excluded_count: usize,
    included_user: usize,
    included_project: usize,
    included_general: usize,
    included: Vec<serde_json::Value>,
    excluded: Vec<serde_json::Value>,
}

impl MemoryProjectionExplain {
    pub(super) fn to_json(&self) -> serde_json::Value {
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

pub(super) fn memory_projection_explain(
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

pub(super) fn superseded_memory_record_count(
    config: &IkarosConfig,
    paths: &IkarosPaths,
) -> Result<usize> {
    Ok(superseded_memory_records(config, paths)?
        .into_iter()
        .filter(|record| !record.active && record.superseded_by.is_some())
        .count())
}

pub(super) fn superseded_memory_records(
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
