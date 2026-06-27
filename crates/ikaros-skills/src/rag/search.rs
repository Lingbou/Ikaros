// SPDX-License-Identifier: GPL-3.0-only

use super::{
    egress_embedding::with_execution_env_embedding_provider,
    policy::{rag_approval_context, rag_risk_level},
};
use crate::support::input_string;
use async_trait::async_trait;
use ikaros_core::{RagConfig, RemoteProviderConfig, Result, RiskLevel};
use ikaros_rag::{LocalRagStore, RagHit, RagQuery};
use ikaros_tools::{PolicyRequest, Skill, SkillContext, SkillOutput};
use serde_json::json;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct RagSearchSkill {
    index: LocalRagStore,
    rag_config: RagConfig,
    provider_settings: RemoteProviderConfig,
}

impl RagSearchSkill {
    pub(crate) fn new(
        index: LocalRagStore,
        rag_config: RagConfig,
        provider_settings: RemoteProviderConfig,
    ) -> Self {
        Self {
            index,
            rag_config,
            provider_settings,
        }
    }
}

#[async_trait]
impl Skill for RagSearchSkill {
    fn name(&self) -> &'static str {
        "rag_search"
    }

    fn description(&self) -> &'static str {
        "Search the client-side RAG index."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "required": ["query"], "properties": {"query": {"type": "string"}, "top_k": {"type": "integer"}, "scope": {"type": "string"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        rag_risk_level(&self.rag_config, false)
    }

    fn policy_request(&self, _input: &serde_json::Value, _workspace_root: &Path) -> PolicyRequest {
        PolicyRequest {
            action: self.name().into(),
            risk: self.risk_level(),
            path: None,
            command: None,
            is_write: false,
        }
    }

    fn approval_context(
        &self,
        input: &serde_json::Value,
        workspace_root: &Path,
    ) -> Option<serde_json::Value> {
        Some(rag_approval_context(
            self.name(),
            &self.rag_config,
            &self.provider_settings,
            input,
            workspace_root,
            false,
            false,
        ))
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let query = input_string(&input, "query")?;
        let query = RagQuery {
            query,
            top_k: input
                .get("top_k")
                .and_then(serde_json::Value::as_u64)
                .map(|value| value as usize)
                .unwrap_or(5),
            scope: input
                .get("scope")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
        };
        let hits = with_execution_env_embedding_provider(
            &self.rag_config,
            &self.provider_settings,
            ctx.session.env.clone(),
            |provider| self.index.search_with_embedding(query, provider),
        )?;
        Ok(SkillOutput::new(
            "rag search complete",
            json!(render_rag_hits(hits)),
        ))
    }
}

fn render_rag_hits(hits: Vec<RagHit>) -> Vec<serde_json::Value> {
    hits.into_iter()
        .map(|hit| {
            json!({
                "chunk": {
                    "id": hit.chunk.id,
                    "document_id": hit.chunk.document_id,
                    "scope": hit.chunk.scope,
                    "source_path": hit.chunk.source_path,
                    "line_start": hit.chunk.line_start,
                    "line_end": hit.chunk.line_end,
                    "content": hit.chunk.content,
                    "indexed_at": hit.chunk.indexed_at,
                    "modified_at": hit.chunk.modified_at,
                    "embedding_provider": hit.chunk.embedding_provider,
                },
                "citation": hit.citation,
                "score": hit.score,
            })
        })
        .collect()
}
