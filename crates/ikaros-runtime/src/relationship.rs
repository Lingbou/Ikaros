// SPDX-License-Identifier: GPL-3.0-only

use crate::session_and_registry;
use ikaros_core::{IkarosPaths, Result, ToolResult, redact_secrets};
use ikaros_harness::{ExecutionSession, SkillRegistry};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationshipNote {
    pub id: String,
    pub scope: String,
    pub content: String,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationshipSnapshot {
    pub scope: Option<String>,
    pub notes: Vec<RelationshipNote>,
    pub audit_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RelationshipMutationReport {
    pub result: ToolResult,
    pub audit_path: PathBuf,
}

pub async fn relationship_snapshot(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    scope: Option<&str>,
    limit: usize,
) -> Result<RelationshipSnapshot> {
    paths.ensure()?;
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    relationship_snapshot_from_session(&session, &registry, scope, limit).await
}

pub async fn relationship_snapshot_from_session(
    session: &ExecutionSession,
    registry: &SkillRegistry,
    scope: Option<&str>,
    limit: usize,
) -> Result<RelationshipSnapshot> {
    let mut input = json!({
        "kind": "relationship",
        "limit": limit,
    });
    if let Some(scope) = scope {
        input["scope"] = json!(scope);
    }
    let result = session
        .execute_read_skill_with_audit_input(registry, "memory_search", input.clone(), input)
        .await?;
    Ok(RelationshipSnapshot {
        scope: scope.map(redact_secrets),
        notes: relationship_notes_from_output(&result.output, limit),
        audit_path: session.audit.path().to_path_buf(),
    })
}

pub async fn remember_relationship_note(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    scope: &str,
    content: &str,
    tags: Vec<String>,
) -> Result<RelationshipMutationReport> {
    paths.ensure()?;
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let result = session
        .execute_skill(
            &registry,
            "memory_append",
            json!({
                "kind": "relationship",
                "scope": scope,
                "content": content,
                "tags": relationship_tags(tags),
            }),
        )
        .await?;
    Ok(RelationshipMutationReport {
        result,
        audit_path: session.audit.path().to_path_buf(),
    })
}

pub async fn forget_relationship_note_by_id(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    id: &str,
) -> Result<RelationshipMutationReport> {
    run_relationship_delete(
        paths,
        workspace,
        agent_override,
        json!({"id": id, "kind": "relationship"}),
    )
    .await
}

pub async fn forget_relationship_scope(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    scope: &str,
) -> Result<RelationshipMutationReport> {
    run_relationship_delete(
        paths,
        workspace,
        agent_override,
        json!({"scope": scope, "kind": "relationship"}),
    )
    .await
}

pub fn relationship_context_lines(snapshot: &RelationshipSnapshot, limit: usize) -> Vec<String> {
    snapshot
        .notes
        .iter()
        .take(limit)
        .map(|note| {
            let tags = if note.tags.is_empty() {
                String::new()
            } else {
                format!(" tags={}", note.tags.join(","))
            };
            redact_secrets(&format!(
                "[relationship/{}] {}{}",
                note.scope, note.content, tags
            ))
        })
        .collect()
}

fn relationship_notes_from_output(
    output: &serde_json::Value,
    limit: usize,
) -> Vec<RelationshipNote> {
    output
        .as_array()
        .into_iter()
        .flatten()
        .filter(|record| {
            record
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("relationship"))
        })
        .take(limit)
        .filter_map(|record| {
            let id = record.get("id").and_then(serde_json::Value::as_str)?;
            let scope = record.get("scope").and_then(serde_json::Value::as_str)?;
            let content = record.get("content").and_then(serde_json::Value::as_str)?;
            let created_at = record
                .get("created_at")
                .and_then(serde_json::Value::as_str)?;
            let updated_at = record
                .get("updated_at")
                .and_then(serde_json::Value::as_str)
                .map(redact_secrets);
            let tags = record
                .get("tags")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .map(redact_secrets)
                .collect();
            Some(RelationshipNote {
                id: redact_secrets(id),
                scope: redact_secrets(scope),
                content: redact_secrets(content),
                tags,
                created_at: redact_secrets(created_at),
                updated_at,
            })
        })
        .collect()
}

fn relationship_tags(mut tags: Vec<String>) -> Vec<String> {
    if !tags.iter().any(|tag| tag == "relationship") {
        tags.push("relationship".into());
    }
    tags
}

async fn run_relationship_delete(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    input: serde_json::Value,
) -> Result<RelationshipMutationReport> {
    paths.ensure()?;
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let result = session
        .execute_skill(&registry, "memory_delete", input)
        .await?;
    Ok(RelationshipMutationReport {
        result,
        audit_path: session.audit.path().to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relationship_context_lines_redact_notes() {
        let snapshot = RelationshipSnapshot {
            scope: Some("user".into()),
            notes: vec![RelationshipNote {
                id: "note-1".into(),
                scope: "user".into(),
                content: "prefers concise updates and token=abc123".into(),
                tags: vec!["relationship".into()],
                created_at: "2026-01-01T00:00:00Z".into(),
                updated_at: None,
            }],
            audit_path: PathBuf::from("audit.jsonl"),
        };

        let lines = relationship_context_lines(&snapshot, 5);

        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("[relationship/user]"));
        assert!(!lines[0].contains("abc123"));
        assert!(lines[0].contains("[REDACTED_SECRET]"));
    }
}
