// SPDX-License-Identifier: GPL-3.0-only

use super::policy::{rag_path_policy_request, rag_risk_level};
use crate::support::input_path;
use async_trait::async_trait;
use ikaros_core::{RagConfig, RemoteProviderConfig, Result, RiskLevel};
use ikaros_harness::{PolicyRequest, Skill, SkillContext, SkillOutput};
use ikaros_rag::{IngestOptions, LocalRagStore};
use serde_json::json;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct RagIngestSkill {
    index: LocalRagStore,
    rag_config: RagConfig,
    provider_settings: RemoteProviderConfig,
}

impl RagIngestSkill {
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
impl Skill for RagIngestSkill {
    fn name(&self) -> &'static str {
        "rag_ingest"
    }

    fn description(&self) -> &'static str {
        "Ingest local files into the client-side RAG index."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "required": ["path"], "properties": {"path": {"type": "string"}, "scope": {"type": "string"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        rag_risk_level(&self.rag_config, true)
    }

    fn policy_request(&self, input: &serde_json::Value, workspace_root: &Path) -> PolicyRequest {
        rag_path_policy_request(self.name(), self.risk_level(), input, workspace_root, true)
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let path = input_path(&input, &ctx.session.sandbox.workspace_root)?;
        let scope = input
            .get("scope")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("project")
            .to_string();
        let report = self.index.ingest_path_with_embedding_config(
            &path,
            IngestOptions {
                scope,
                ..IngestOptions::default()
            },
            &self.rag_config,
            &self.provider_settings,
        )?;
        Ok(SkillOutput::new("rag ingest complete", json!(report)))
    }
}

#[derive(Debug, Clone)]
pub struct RagReindexSkill {
    index: LocalRagStore,
    rag_config: RagConfig,
    provider_settings: RemoteProviderConfig,
}

impl RagReindexSkill {
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
impl Skill for RagReindexSkill {
    fn name(&self) -> &'static str {
        "rag_reindex"
    }

    fn description(&self) -> &'static str {
        "Reindex a local path by replacing existing chunks for matching files."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "required": ["path"], "properties": {"path": {"type": "string"}, "scope": {"type": "string"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        rag_risk_level(&self.rag_config, true)
    }

    fn policy_request(&self, input: &serde_json::Value, workspace_root: &Path) -> PolicyRequest {
        rag_path_policy_request(self.name(), self.risk_level(), input, workspace_root, true)
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let path = input_path(&input, &ctx.session.sandbox.workspace_root)?;
        let scope = input
            .get("scope")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("project")
            .to_string();
        let report = self.index.ingest_path_with_embedding_config(
            &path,
            IngestOptions {
                scope,
                ..IngestOptions::default()
            },
            &self.rag_config,
            &self.provider_settings,
        )?;
        Ok(SkillOutput::new("rag reindex complete", json!(report)))
    }
}
