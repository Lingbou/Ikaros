// SPDX-License-Identifier: GPL-3.0-only

use crate::support::{input_path, input_string};
use async_trait::async_trait;
use ikaros_core::{Result, RiskLevel};
use ikaros_harness::{Skill, SkillContext, SkillOutput};
use ikaros_rag::{LocalRagStore, RagStore};
use serde_json::json;

#[derive(Debug, Clone)]
pub struct RagStaleSkill {
    index: LocalRagStore,
}

impl RagStaleSkill {
    pub(crate) fn new(index: LocalRagStore) -> Self {
        Self { index }
    }
}

#[async_trait]
impl Skill for RagStaleSkill {
    fn name(&self) -> &'static str {
        "rag_stale"
    }

    fn description(&self) -> &'static str {
        "List stale or deleted files referenced by the local RAG index."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, _input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let mut stale = Vec::new();
        for indexed in self.index.indexed_files()? {
            let current = ctx
                .session
                .env
                .path_metadata(&indexed.source_path)
                .await
                .ok()
                .and_then(|metadata| metadata.modified_at);
            if current != indexed.modified_at {
                stale.push(indexed.source_path);
            }
        }
        stale.sort();
        stale.dedup();
        Ok(SkillOutput::new(
            format!("{} stale RAG file(s)", stale.len()),
            json!({"stale_files": stale}),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct RagDeleteScopeSkill {
    index: LocalRagStore,
}

impl RagDeleteScopeSkill {
    pub(crate) fn new(index: LocalRagStore) -> Self {
        Self { index }
    }
}

#[async_trait]
impl Skill for RagDeleteScopeSkill {
    fn name(&self) -> &'static str {
        "rag_delete_scope"
    }

    fn description(&self) -> &'static str {
        "Delete local RAG chunks for one scope."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "required": ["scope"], "properties": {"scope": {"type": "string"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::LocalWrite
    }

    async fn execute(&self, input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        let scope = input_string(&input, "scope")?;
        let deleted = self.index.delete_scope(&scope)?;
        Ok(SkillOutput::new(
            format!("deleted {deleted} RAG chunk(s) for scope {scope}"),
            json!({"scope": scope, "chunks_deleted": deleted}),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct RagDeletePathSkill {
    index: LocalRagStore,
}

impl RagDeletePathSkill {
    pub(crate) fn new(index: LocalRagStore) -> Self {
        Self { index }
    }
}

#[async_trait]
impl Skill for RagDeletePathSkill {
    fn name(&self) -> &'static str {
        "rag_delete_path"
    }

    fn description(&self) -> &'static str {
        "Delete local RAG chunks for one source path."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "required": ["path"], "properties": {"path": {"type": "string"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::LocalWrite
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let path = input_path(&input, &ctx.session.sandbox.workspace_root)?;
        let deleted = self.index.delete_path(&path)?;
        Ok(SkillOutput::new(
            format!("deleted {deleted} RAG chunk(s) for {}", path.display()),
            json!({"path": path, "chunks_deleted": deleted}),
        ))
    }
}
