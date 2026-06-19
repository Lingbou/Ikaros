// SPDX-License-Identifier: GPL-3.0-only

use crate::support::input_string;
use async_trait::async_trait;
use ikaros_core::{IkarosError, Result, RiskLevel};
use ikaros_harness::{PolicyRequest, Skill, SkillContext, SkillOutput};
use ikaros_memory::{
    JsonlMemoryCandidateStore, JsonlWorkingMemoryStore, LocalMemoryStore, MemoryCandidate,
    MemoryCandidateQuery, MemoryCandidateReason, MemoryCandidateStatus, MemoryKind,
    MemoryPerspective, MemoryProjectionInput, MemoryQuery, MemoryRecord, MemoryRef, MemoryStore,
    ProjectionRenderer, WorkingMemoryQuery,
};
use serde_json::json;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct MemoryAppendSkill {
    store: LocalMemoryStore,
}

impl MemoryAppendSkill {
    pub(crate) fn new(store: LocalMemoryStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Skill for MemoryAppendSkill {
    fn name(&self) -> &'static str {
        "memory_append"
    }

    fn description(&self) -> &'static str {
        "Append a local memory record after secret detection."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "required": ["kind", "scope", "content"], "properties": {"kind": {"type": "string"}, "scope": {"type": "string"}, "observer": {"type": "string"}, "subject": {"type": "string"}, "content": {"type": "string"}, "tags": {"type": "array", "items": {"type": "string"}}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::DatabaseWrite
    }

    fn policy_request(&self, _input: &serde_json::Value, _workspace_root: &Path) -> PolicyRequest {
        PolicyRequest {
            action: self.name().into(),
            risk: self.risk_level(),
            path: None,
            command: None,
            is_write: true,
        }
    }

    async fn execute(&self, input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        let kind_raw = input_string(&input, "kind")?;
        let kind = parse_memory_kind(&kind_raw)?;
        let scope = input_string(&input, "scope")?;
        let content = input_string(&input, "content")?;
        let tags = input
            .get("tags")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut record = MemoryRecord::new(kind, scope, content)?.with_tags(tags);
        if let Some(perspective) = optional_memory_perspective(&input)? {
            record = record.with_perspective(perspective);
        }
        let record = self.store.append(record)?;
        Ok(SkillOutput::new(
            "memory appended",
            json!({"id": record.id, "backend": self.store.backend_name(), "path": self.store.path()}),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct MemorySearchSkill {
    store: LocalMemoryStore,
}

impl MemorySearchSkill {
    pub(crate) fn new(store: LocalMemoryStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Skill for MemorySearchSkill {
    fn name(&self) -> &'static str {
        "memory_search"
    }

    fn description(&self) -> &'static str {
        "Search local memory."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "properties": {"query": {"type": "string"}, "kind": {"type": "string"}, "scope": {"type": "string"}, "observer": {"type": "string"}, "subject": {"type": "string"}, "limit": {"type": "integer"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        let kind = input
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .map(parse_memory_kind)
            .transpose()?;
        let query = MemoryQuery {
            kind,
            scope: input
                .get("scope")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            perspective: optional_memory_perspective(&input)?,
            text: input
                .get("query")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            limit: input
                .get("limit")
                .and_then(serde_json::Value::as_u64)
                .map(|value| value as usize),
        };
        let records = self.store.search(query)?;
        Ok(SkillOutput::new("memory search complete", json!(records)))
    }
}

#[derive(Debug, Clone)]
pub struct MemoryCandidateCreateSkill {
    store: JsonlMemoryCandidateStore,
}

impl MemoryCandidateCreateSkill {
    pub(crate) fn new(memory_store: LocalMemoryStore) -> Self {
        Self {
            store: JsonlMemoryCandidateStore::new(memory_store.memory_dir()),
        }
    }
}

#[async_trait]
impl Skill for MemoryCandidateCreateSkill {
    fn name(&self) -> &'static str {
        "memory_candidate_create"
    }

    fn description(&self) -> &'static str {
        "Create a pending local memory candidate without promoting it to core memory."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "required": ["kind", "scope", "content"], "properties": {"kind": {"type": "string"}, "scope": {"type": "string"}, "content": {"type": "string"}, "reason": {"type": "string"}, "confidence": {"type": "number"}, "tags": {"type": "array", "items": {"type": "string"}}, "source_ref": {"type": "object"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::DatabaseWrite
    }

    async fn execute(&self, input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        let kind = parse_memory_kind(&input_string(&input, "kind")?)?;
        let scope = input_string(&input, "scope")?;
        let content = input_string(&input, "content")?;
        let existing = self.store.list(MemoryCandidateQuery {
            status: Some(MemoryCandidateStatus::Pending),
            kind: Some(kind.clone()),
            scope: Some(scope.clone()),
            limit: Some(usize::MAX),
        })?;
        if let Some(candidate) = existing
            .into_iter()
            .find(|candidate| candidate.content == content)
        {
            return Ok(SkillOutput::new(
                "memory candidate already pending",
                json!({"created": false, "id": candidate.id, "status": candidate.status}),
            ));
        }

        let reason = input
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .map(parse_candidate_reason)
            .transpose()?
            .unwrap_or(MemoryCandidateReason::RuntimeInference);
        let confidence = input
            .get("confidence")
            .and_then(serde_json::Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(0.5);
        let tags = input
            .get("tags")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut candidate =
            MemoryCandidate::new(kind, scope, content, reason, confidence)?.with_tags(tags)?;
        if let Some(source_ref) = input.get("source_ref") {
            candidate = candidate.with_source_ref(
                serde_json::from_value::<MemoryRef>(source_ref.clone())
                    .map_err(ikaros_core::IkarosError::from)?,
            )?;
        }
        let candidate = self.store.create(candidate)?;
        Ok(SkillOutput::new(
            "memory candidate created",
            json!({"created": true, "id": candidate.id, "status": candidate.status}),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct MemoryProjectionSkill {
    store: LocalMemoryStore,
}

impl MemoryProjectionSkill {
    pub(crate) fn new(store: LocalMemoryStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Skill for MemoryProjectionSkill {
    fn name(&self) -> &'static str {
        "memory_projection"
    }

    fn description(&self) -> &'static str {
        "Render accepted core memory into prompt-safe projections."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "properties": {"user_scope": {"type": "string"}, "project_scope": {"type": "string"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        let user_scope = input
            .get("user_scope")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("default")
            .to_owned();
        let project_scope = input
            .get("project_scope")
            .and_then(serde_json::Value::as_str)
            .filter(|scope| !scope.trim().is_empty())
            .map(ToOwned::to_owned);
        let records = self.store.list(MemoryQuery {
            limit: Some(usize::MAX),
            ..MemoryQuery::default()
        })?;
        let projection = ProjectionRenderer::default().render(MemoryProjectionInput {
            user_scope,
            project_scope,
            perspective: None,
            records,
        })?;
        Ok(SkillOutput::new(
            "memory projection rendered",
            json!(projection),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct WorkingMemoryListSkill {
    store: JsonlWorkingMemoryStore,
}

impl WorkingMemoryListSkill {
    pub(crate) fn new(memory_store: LocalMemoryStore) -> Self {
        Self {
            store: JsonlWorkingMemoryStore::new(memory_store.memory_dir()),
        }
    }
}

#[async_trait]
impl Skill for WorkingMemoryListSkill {
    fn name(&self) -> &'static str {
        "working_memory_list"
    }

    fn description(&self) -> &'static str {
        "List current session working memory scratchpad records."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "properties": {"session_id": {"type": "string"}, "kind": {"type": "string"}, "scope": {"type": "string"}, "limit": {"type": "integer"}, "include_expired": {"type": "boolean"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        let kind = input
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .map(parse_memory_kind)
            .transpose()?;
        let records = self.store.list(WorkingMemoryQuery {
            session_id: input
                .get("session_id")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            kind,
            scope: input
                .get("scope")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            include_expired: input
                .get("include_expired")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            limit: input
                .get("limit")
                .and_then(serde_json::Value::as_u64)
                .map(|value| value as usize),
        })?;
        Ok(SkillOutput::new("working memory listed", json!(records)))
    }
}

#[derive(Debug, Clone)]
pub struct MemoryUpdateSkill {
    store: LocalMemoryStore,
}

impl MemoryUpdateSkill {
    pub(crate) fn new(store: LocalMemoryStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Skill for MemoryUpdateSkill {
    fn name(&self) -> &'static str {
        "memory_update"
    }

    fn description(&self) -> &'static str {
        "Update content and/or tags for one local memory record."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "required": ["id"], "properties": {"id": {"type": "string"}, "content": {"type": "string"}, "tags": {"type": "array", "items": {"type": "string"}}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::DatabaseWrite
    }

    async fn execute(&self, input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        let id = input_string(&input, "id")?;
        let content = input
            .get("content")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        let tags = input
            .get("tags")
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            });
        let updated = self.store.update(&id, content, tags)?;
        let ok = updated.is_some();
        Ok(SkillOutput::new(
            if ok {
                "memory updated"
            } else {
                "memory record not found"
            },
            json!({"updated": updated}),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct MemoryDeleteSkill {
    store: LocalMemoryStore,
}

impl MemoryDeleteSkill {
    pub(crate) fn new(store: LocalMemoryStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Skill for MemoryDeleteSkill {
    fn name(&self) -> &'static str {
        "memory_delete"
    }

    fn description(&self) -> &'static str {
        "Delete local memory by id or by scope with an optional kind filter."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "properties": {"id": {"type": "string"}, "scope": {"type": "string"}, "kind": {"type": "string"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::DatabaseWrite
    }

    async fn execute(&self, input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        if let Some(id) = input.get("id").and_then(serde_json::Value::as_str) {
            if let Some(kind) = input
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .map(parse_memory_kind)
                .transpose()?
            {
                let matches_kind = self
                    .store
                    .list(MemoryQuery {
                        kind: Some(kind),
                        scope: None,
                        perspective: None,
                        text: None,
                        limit: None,
                    })?
                    .iter()
                    .any(|record| record.id == id);
                if !matches_kind {
                    return Ok(SkillOutput::new(
                        "memory record not found",
                        json!({"mode": "id", "id": id, "records_deleted": 0usize}),
                    ));
                }
            }
            let deleted = self.store.delete_by_id(id)?;
            return Ok(SkillOutput::new(
                if deleted {
                    "memory deleted"
                } else {
                    "memory record not found"
                },
                json!({"mode": "id", "id": id, "records_deleted": usize::from(deleted)}),
            ));
        }
        let scope = input_string(&input, "scope")?;
        let kind = input
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .map(parse_memory_kind)
            .transpose()?;
        let deleted = self.store.delete_scope(kind.clone(), &scope)?;
        Ok(SkillOutput::new(
            format!("deleted {deleted} memory record(s)"),
            json!({"mode": "scope", "kind": kind, "scope": scope, "records_deleted": deleted}),
        ))
    }
}

fn parse_memory_kind(kind: &str) -> Result<MemoryKind> {
    match kind.to_ascii_lowercase().as_str() {
        "user" => Ok(MemoryKind::User),
        "project" => Ok(MemoryKind::Project),
        "task" => Ok(MemoryKind::Task),
        "persona" => Ok(MemoryKind::Persona),
        "relationship" => Ok(MemoryKind::Relationship),
        "knowledge" => Ok(MemoryKind::Knowledge),
        other => Err(IkarosError::Message(format!(
            "unsupported memory kind: {other}"
        ))),
    }
}

fn parse_candidate_reason(reason: &str) -> Result<MemoryCandidateReason> {
    match reason.to_ascii_lowercase().as_str() {
        "explicit_remember" => Ok(MemoryCandidateReason::ExplicitRemember),
        "preference_pattern" => Ok(MemoryCandidateReason::PreferencePattern),
        "task_outcome" => Ok(MemoryCandidateReason::TaskOutcome),
        "runtime_inference" => Ok(MemoryCandidateReason::RuntimeInference),
        "manual" => Ok(MemoryCandidateReason::Manual),
        "rag_reference" => Ok(MemoryCandidateReason::RagReference),
        other => Err(IkarosError::Message(format!(
            "unsupported memory candidate reason: {other}"
        ))),
    }
}

fn optional_memory_perspective(input: &serde_json::Value) -> Result<Option<MemoryPerspective>> {
    let observer = input
        .get("observer")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let subject = input
        .get("subject")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty());
    match (observer, subject) {
        (Some(observer), Some(subject)) => MemoryPerspective::new(observer, subject).map(Some),
        (None, None) => Ok(None),
        _ => Err(IkarosError::Message(
            "memory perspective requires both observer and subject".into(),
        )),
    }
}
