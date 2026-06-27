// SPDX-License-Identifier: GPL-3.0-only
//! Built-in Ikaros skills, all designed to run through the harness.

mod browser;
mod coding;
mod fs;
mod mcp;
mod memory;
mod multimodal;
mod persona;
mod plugin;
mod prompt_docs;
mod rag;
mod shell;
mod support;
mod tool_bridge;
mod voice;
mod web;

pub use browser::{
    BrowserActivateTargetSkill, BrowserCdpSkill, BrowserClickSkill, BrowserCloseTargetSkill,
    BrowserListSkill, BrowserNavigateSkill, BrowserNewTargetSkill, BrowserScreenshotSkill,
    BrowserScrollSkill, BrowserSnapshotSkill, BrowserStatusSkill, BrowserTypeSkill,
};
pub use coding::{
    CodeEditGuardedSkill, CodeIterateSkill, CodeReviewSkill, CodeWorkflowSkill, RepoScanSkill,
    RunTestsSkill, TaskSummarizeSkill,
};
pub use fs::{FsReadSkill, FsWriteGuardedSkill, ListDirSkill};
use ikaros_core::{ModelConfig, RagConfig, RemoteProviderConfig};
use ikaros_harness::CancellationToken;
use ikaros_memory::LocalMemoryStore;
use ikaros_models::ModelProvider;
use ikaros_rag::LocalRagStore;
use ikaros_session::{SessionId, SessionSource, SessionStore, TurnId};
use ikaros_tools::{SkillRegistry, Toolset};
use ikaros_voice::VoiceProviderConfig;
pub use mcp::{McpStdioCallSkill, McpStdioProbeSkill};
pub use memory::{
    MemoryAppendSkill, MemoryCandidateCreateSkill, MemoryDeleteSkill, MemoryProjectionSkill,
    MemorySearchSkill, MemoryUpdateSkill, WorkingMemoryListSkill,
};
pub use multimodal::{ImageGenerateSkill, VisionDescribeSkill};
pub use persona::PersonaLoadSkill;
pub use plugin::PluginCommandRunSkill;
pub use rag::{
    RagDeletePathSkill, RagDeleteScopeSkill, RagIngestSkill, RagReindexSkill, RagSearchSkill,
    RagStaleSkill, with_execution_env_embedding_provider,
};
pub use shell::{GitDiffSkill, GitStatusSkill, ShellGuardedSkill};
use std::{path::PathBuf, sync::Arc};
pub use tool_bridge::{ToolCallSkill, ToolDescribeSkill, ToolSearchSkill};
pub use voice::{VoiceAsrSkill, VoiceTtsSkill};
pub use web::{WebExtractSkill, WebSearchSkill};

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
    pub voice_tts: VoiceProviderConfig,
    pub voice_tts_provider: RemoteProviderConfig,
    pub voice_asr: VoiceProviderConfig,
    pub voice_asr_provider: RemoteProviderConfig,
    pub web_search_provider: RemoteProviderConfig,
    pub coding_session: Option<CodingSessionConfig>,
}

pub fn builtin_registry(env: SkillEnvironment) -> SkillRegistry {
    let mut registry = SkillRegistry::new();
    registry.register_with_toolset(FsReadSkill, Toolset::Workspace);
    registry.register_with_toolset(FsWriteGuardedSkill, Toolset::Workspace);
    registry.register_with_toolset(ListDirSkill, Toolset::Workspace);
    registry.register_with_toolset(ShellGuardedSkill, Toolset::Workspace);
    registry.register_with_toolset(GitStatusSkill, Toolset::Workspace);
    registry.register_with_toolset(GitDiffSkill, Toolset::Workspace);
    registry.register_with_toolset(
        MemoryAppendSkill::new(env.memory_store.clone()),
        Toolset::Memory,
    );
    registry.register_with_toolset(
        MemorySearchSkill::new(env.memory_store.clone()),
        Toolset::Memory,
    );
    registry.register_with_toolset(
        MemoryCandidateCreateSkill::new(env.memory_store.clone()),
        Toolset::Memory,
    );
    registry.register_with_toolset(
        MemoryProjectionSkill::new(env.memory_store.clone()),
        Toolset::Memory,
    );
    registry.register_with_toolset(
        WorkingMemoryListSkill::new(env.memory_store.clone()),
        Toolset::Memory,
    );
    registry.register_with_toolset(
        MemoryUpdateSkill::new(env.memory_store.clone()),
        Toolset::Memory,
    );
    registry.register_with_toolset(
        MemoryDeleteSkill::new(env.memory_store.clone()),
        Toolset::Memory,
    );
    registry.register_with_toolset(PersonaLoadSkill::new(env.persona_path), Toolset::Core);
    registry.register_with_toolset(
        VoiceTtsSkill::new(env.voice_tts.clone(), env.voice_tts_provider.clone()),
        Toolset::Voice,
    );
    registry.register_with_toolset(
        VoiceAsrSkill::new(env.voice_asr.clone(), env.voice_asr_provider.clone()),
        Toolset::Voice,
    );
    registry.register_with_toolset(TaskSummarizeSkill, Toolset::Core);
    registry.register_with_toolset(WebExtractSkill, Toolset::Core);
    registry.register_with_toolset(
        WebSearchSkill::new(env.web_search_provider.clone()),
        Toolset::Core,
    );
    registry.register_with_toolset(BrowserStatusSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserListSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserNewTargetSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserActivateTargetSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserCloseTargetSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserNavigateSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserSnapshotSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserClickSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserTypeSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserScrollSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserScreenshotSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserCdpSkill, Toolset::Plugin);
    registry.register_with_toolset(RepoScanSkill, Toolset::Coding);
    registry.register_with_toolset(RunTestsSkill, Toolset::Coding);
    registry.register_with_toolset(CodeEditGuardedSkill, Toolset::Coding);
    registry.register_with_toolset(CodeReviewSkill, Toolset::Coding);
    registry.register_with_toolset(CodeIterateSkill, Toolset::Coding);
    registry.register_with_toolset(
        CodeWorkflowSkill::new(env.coding_session.clone()),
        Toolset::Coding,
    );
    registry.register_with_toolset(
        RagIngestSkill::new(
            env.rag_index.clone(),
            env.rag_config.clone(),
            env.rag_provider.clone(),
        ),
        Toolset::Rag,
    );
    registry.register_with_toolset(
        RagSearchSkill::new(
            env.rag_index.clone(),
            env.rag_config.clone(),
            env.rag_provider.clone(),
        ),
        Toolset::Rag,
    );
    registry.register_with_toolset(RagStaleSkill::new(env.rag_index.clone()), Toolset::Rag);
    registry.register_with_toolset(
        RagDeleteScopeSkill::new(env.rag_index.clone()),
        Toolset::Rag,
    );
    registry.register_with_toolset(RagDeletePathSkill::new(env.rag_index.clone()), Toolset::Rag);
    registry.register_with_toolset(
        RagReindexSkill::new(env.rag_index, env.rag_config, env.rag_provider),
        Toolset::Rag,
    );
    registry.register_with_toolset(
        PluginCommandRunSkill::new(env.skills_dir.clone()),
        Toolset::Plugin,
    );
    registry.register_with_toolset(McpStdioProbeSkill, Toolset::Plugin);
    registry.register_with_toolset(McpStdioCallSkill, Toolset::Plugin);
    for document in prompt_docs::load_prompt_skill_documents(&env.skills_dir) {
        registry.register_prompt_skill_document(document);
    }
    let deferred_registry = registry.clone();
    registry.register_with_toolset(
        ToolSearchSkill::new(deferred_registry.clone()),
        Toolset::Core,
    );
    registry.register_with_toolset(
        ToolDescribeSkill::new(deferred_registry.clone()),
        Toolset::Core,
    );
    registry.register_with_toolset(ToolCallSkill::new(deferred_registry), Toolset::Core);
    registry
}

pub fn register_model_backed_skills(
    registry: &mut SkillRegistry,
    model: ModelConfig,
    provider: RemoteProviderConfig,
) {
    registry.register_with_toolset(
        VisionDescribeSkill::new(model.clone(), provider.clone()),
        Toolset::Core,
    );
    registry.register_with_toolset(ImageGenerateSkill::new(model, provider), Toolset::Core);
}

#[cfg(test)]
mod tests;
