// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{Result, redact_secrets};
use ikaros_execution::harness::{ExecutionSession, SkillRegistry};
use ikaros_state::memory::{
    RelationshipMemoryNote, relationship_context_lines as memory_relationship_context_lines,
    relationship_notes_from_output,
};
use serde_json::json;

pub use ikaros_state::memory::{RelationshipMutationReport, RelationshipSnapshot};

pub type RelationshipNote = RelationshipMemoryNote;
pub type RelationshipReport = RelationshipMutationReport;

pub async fn relationship_snapshot_from_session(
    session: &ExecutionSession,
    registry: &SkillRegistry,
    scope: Option<&str>,
    limit: usize,
) -> Result<RelationshipSnapshot> {
    let mut input = json!({
        "kind": "relationship",
        "limit": limit,
        "include_pending_candidates": true,
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

pub fn relationship_context_lines(snapshot: &RelationshipSnapshot, limit: usize) -> Vec<String> {
    memory_relationship_context_lines(&snapshot.notes, limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
