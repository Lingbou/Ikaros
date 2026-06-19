// SPDX-License-Identifier: GPL-3.0-only
//! Built-in Ikaros skills, all designed to run through the harness.

mod coding;
mod fs;
mod memory;
mod persona;
mod plugin;
mod rag;
mod shell;
mod support;
mod voice;

pub use coding::{
    CodeEditGuardedSkill, CodeIterateSkill, CodeReviewSkill, CodeWorkflowSkill, RepoScanSkill,
    RunTestsSkill, TaskSummarizeSkill,
};
pub use fs::{FsReadSkill, FsWriteGuardedSkill, ListDirSkill};
use ikaros_core::{RagConfig, RemoteProviderConfig};
use ikaros_harness::SkillRegistry;
use ikaros_memory::LocalMemoryStore;
use ikaros_rag::LocalRagStore;
use ikaros_session::{SessionId, SessionSource, SessionStore, TurnId};
use ikaros_voice::VoiceProviderConfig;
pub use memory::{
    MemoryAppendSkill, MemoryCandidateCreateSkill, MemoryDeleteSkill, MemoryProjectionSkill,
    MemorySearchSkill, MemoryUpdateSkill, WorkingMemoryListSkill,
};
pub use persona::PersonaLoadSkill;
pub use plugin::PluginCommandRunSkill;
pub use rag::{
    RagDeletePathSkill, RagDeleteScopeSkill, RagIngestSkill, RagReindexSkill, RagSearchSkill,
    RagStaleSkill,
};
pub use shell::{GitDiffSkill, GitStatusSkill, ShellGuardedSkill};
use std::{path::PathBuf, sync::Arc};
pub use voice::{VoiceAsrSkill, VoiceTtsSkill};

#[derive(Clone)]
pub struct CodingSessionConfig {
    pub store: Arc<dyn SessionStore>,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub source: SessionSource,
    pub agent_id: Option<String>,
    pub workspace: Option<PathBuf>,
}

impl std::fmt::Debug for CodingSessionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodingSessionConfig")
            .field("session_id", &self.session_id)
            .field("turn_id", &self.turn_id)
            .field("source", &self.source)
            .field("agent_id", &self.agent_id)
            .field("workspace", &self.workspace)
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
    pub coding_session: Option<CodingSessionConfig>,
}

pub fn builtin_registry(env: SkillEnvironment) -> SkillRegistry {
    let mut registry = SkillRegistry::new();
    registry.register(FsReadSkill);
    registry.register(FsWriteGuardedSkill);
    registry.register(ListDirSkill);
    registry.register(ShellGuardedSkill);
    registry.register(GitStatusSkill);
    registry.register(GitDiffSkill);
    registry.register(MemoryAppendSkill::new(env.memory_store.clone()));
    registry.register(MemorySearchSkill::new(env.memory_store.clone()));
    registry.register(MemoryCandidateCreateSkill::new(env.memory_store.clone()));
    registry.register(MemoryProjectionSkill::new(env.memory_store.clone()));
    registry.register(WorkingMemoryListSkill::new(env.memory_store.clone()));
    registry.register(MemoryUpdateSkill::new(env.memory_store.clone()));
    registry.register(MemoryDeleteSkill::new(env.memory_store.clone()));
    registry.register(PersonaLoadSkill::new(env.persona_path));
    registry.register(VoiceTtsSkill::new(
        env.voice_tts.clone(),
        env.voice_tts_provider.clone(),
    ));
    registry.register(VoiceAsrSkill::new(
        env.voice_asr.clone(),
        env.voice_asr_provider.clone(),
    ));
    registry.register(TaskSummarizeSkill);
    registry.register(RepoScanSkill);
    registry.register(RunTestsSkill);
    registry.register(CodeEditGuardedSkill);
    registry.register(CodeReviewSkill);
    registry.register(CodeIterateSkill);
    registry.register(CodeWorkflowSkill::new(env.coding_session.clone()));
    registry.register(RagIngestSkill::new(
        env.rag_index.clone(),
        env.rag_config.clone(),
        env.rag_provider.clone(),
    ));
    registry.register(RagSearchSkill::new(
        env.rag_index.clone(),
        env.rag_config.clone(),
        env.rag_provider.clone(),
    ));
    registry.register(RagStaleSkill::new(env.rag_index.clone()));
    registry.register(RagDeleteScopeSkill::new(env.rag_index.clone()));
    registry.register(RagDeletePathSkill::new(env.rag_index.clone()));
    registry.register(RagReindexSkill::new(
        env.rag_index,
        env.rag_config,
        env.rag_provider,
    ));
    registry.register(PluginCommandRunSkill::new(env.skills_dir));
    registry
}

#[cfg(test)]
mod tests;
