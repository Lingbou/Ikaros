// SPDX-License-Identifier: GPL-3.0-only
//! Built-in Ikaros skills, all designed to run through the harness.

mod coding;
mod fs;
pub mod groups;
mod mcp;
mod memory;
mod persona;
mod plugin;
mod prompt_docs;
mod rag;
mod shell;
mod support;
mod tool_bridge;

pub use coding::{
    CodeEditGuardedSkill, CodeIterateSkill, CodeReviewSkill, CodeWorkflowSkill, RepoScanSkill,
    RunTestsSkill, TaskSummarizeSkill,
};
pub use fs::{FsReadSkill, FsWriteGuardedSkill, ListDirSkill};
pub use groups::{BuiltinRegistryBuilder, BuiltinSkillGroup};
use ikaros_core::{RagConfig, RemoteProviderConfig};
use ikaros_execution::harness::CancellationToken;
use ikaros_execution::toolkit::SkillRegistry;
use ikaros_providers::model::ModelProvider;
use ikaros_state::memory::LocalMemoryStore;
use ikaros_state::rag::LocalRagStore;
use ikaros_state::session::{SessionId, SessionSource, SessionStore, TurnId};
pub use mcp::{McpStdioCallSkill, McpStdioProbeSkill};
pub use memory::{
    MemoryAppendSkill, MemoryCandidateCreateSkill, MemoryDeleteSkill, MemoryProjectionSkill,
    MemorySearchSkill, MemoryUpdateSkill, WorkingMemoryListSkill,
};
pub use persona::PersonaLoadSkill;
pub use plugin::PluginCommandRunSkill;
pub use rag::{
    RagDeletePathSkill, RagDeleteScopeSkill, RagIngestSkill, RagReindexSkill, RagSearchSkill,
    RagStaleSkill, with_execution_env_embedding_provider,
};
pub use shell::{GitDiffSkill, GitStatusSkill, ShellGuardedSkill};
use std::{path::PathBuf, sync::Arc};
pub use tool_bridge::{ToolCallSkill, ToolDescribeSkill, ToolSearchSkill};

#[derive(Clone)]
pub struct CodingSessionConfig {
    pub store: Arc<dyn SessionStore>,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub source: SessionSource,
    pub agent_id: Option<String>,
    pub workspace: Option<PathBuf>,
    pub model_provider: Option<Arc<dyn ModelProvider>>,
    pub cancellation: CancellationToken,
}

impl std::fmt::Debug for CodingSessionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodingSessionConfig")
            .field("session_id", &self.session_id)
            .field("turn_id", &self.turn_id)
            .field("source", &self.source)
            .field("agent_id", &self.agent_id)
            .field("workspace", &self.workspace)
            .field(
                "model_provider",
                &self.model_provider.as_ref().map(|p| p.name()),
            )
            .field("cancelled", &self.cancellation.is_cancelled())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct SkillEnvironment {
    pub workspace_root: PathBuf,
    pub memory_store: LocalMemoryStore,
    pub rag_index: LocalRagStore,
    pub rag_config: RagConfig,
    pub rag_provider: RemoteProviderConfig,
    pub persona_path: PathBuf,
    pub skills_dir: PathBuf,
    pub coding_session: Option<CodingSessionConfig>,
}

pub fn builtin_registry(env: SkillEnvironment) -> SkillRegistry {
    BuiltinRegistryBuilder::new(env).build()
}

#[cfg(test)]
mod tests;
