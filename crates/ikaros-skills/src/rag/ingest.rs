// SPDX-License-Identifier: GPL-3.0-only

use super::{
    egress_embedding::with_execution_env_embedding_provider,
    policy::{rag_approval_context, rag_path_policy_request, rag_risk_level},
};
use crate::support::input_path;
use async_trait::async_trait;
use ikaros_core::{IkarosError, RagConfig, RemoteProviderConfig, Result, RiskLevel};
use ikaros_rag::{IngestOptions, IngestSourceFile, LocalRagStore};
use ikaros_tools::{FileSystem, PolicyRequest, Skill, SkillContext, SkillOutput};
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
            true,
            true,
        ))
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let path = input_path(&input, &ctx.session.sandbox.workspace_root)?;
        let scope = input
            .get("scope")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("project")
            .to_string();
        let sources = collect_ingest_sources(ctx.session.env.as_ref(), &path).await?;
        let options = IngestOptions {
            scope,
            ..IngestOptions::default()
        };
        let report = with_execution_env_embedding_provider(
            &self.rag_config,
            &self.provider_settings,
            ctx.session.env.clone(),
            |provider| {
                self.index
                    .ingest_sources_with_embedding(sources, options, provider)
            },
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
            true,
            true,
        ))
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let path = input_path(&input, &ctx.session.sandbox.workspace_root)?;
        let scope = input
            .get("scope")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("project")
            .to_string();
        let sources = collect_ingest_sources(ctx.session.env.as_ref(), &path).await?;
        let options = IngestOptions {
            scope,
            ..IngestOptions::default()
        };
        let report = with_execution_env_embedding_provider(
            &self.rag_config,
            &self.provider_settings,
            ctx.session.env.clone(),
            |provider| {
                self.index
                    .ingest_sources_with_embedding(sources, options, provider)
            },
        )?;
        Ok(SkillOutput::new("rag reindex complete", json!(report)))
    }
}

async fn collect_ingest_sources(
    file_system: &dyn FileSystem,
    root: &Path,
) -> Result<Vec<IngestSourceFile>> {
    let mut pending = vec![(root.to_path_buf(), true)];
    let mut files = Vec::new();
    while let Some((path, is_root)) = pending.pop() {
        let metadata = file_system.path_metadata(&path).await?;
        if metadata.is_symlink {
            if is_root {
                return Err(IkarosError::Message(format!(
                    "RAG ingest rejects symlink path: {}",
                    path.display()
                )));
            }
            continue;
        }
        if metadata.is_file {
            if is_indexable(&path) {
                let content = file_system.read_to_string(&path).await?;
                files.push(IngestSourceFile {
                    source_path: path,
                    content,
                    modified_at: metadata.modified_at,
                });
            }
            continue;
        }
        if !metadata.is_dir {
            return Err(IkarosError::Message(format!(
                "RAG ingest path does not exist: {}",
                path.display()
            )));
        }
        for name in file_system.read_dir(&path).await? {
            let child = path.join(name);
            if should_skip(&child) {
                continue;
            }
            pending.push((child, false));
        }
    }
    files.sort_by(|left, right| left.source_path.cmp(&right.source_path));
    Ok(files)
}

fn should_skip(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name == ".git"
                || name == "target"
                || name == "node_modules"
                || name == ".temp"
                || name.starts_with('.')
        })
}

fn is_indexable(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some(
            "md" | "txt"
                | "rs"
                | "toml"
                | "json"
                | "yaml"
                | "yml"
                | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "py"
        )
    )
}
