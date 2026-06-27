// SPDX-License-Identifier: GPL-3.0-only

use crate::session_and_registry;
use ikaros_core::{IkarosPaths, Result, redact_secrets};
use ikaros_state::memory::relationship_notes_from_output;
pub use ikaros_state::memory::{
    RelationshipMutationReport, RelationshipNote, RelationshipSnapshot,
};
use serde_json::json;
use std::path::Path;

pub async fn relationship_snapshot(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    scope: Option<&str>,
    limit: usize,
) -> Result<RelationshipSnapshot> {
    paths.ensure()?;
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let mut input = json!({
        "kind": "relationship",
        "limit": limit,
        "include_pending_candidates": true,
    });
    if let Some(scope) = scope {
        input["scope"] = json!(scope);
    }
    let result = session
        .execute_read_skill_with_audit_input(&registry, "memory_search", input.clone(), input)
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
