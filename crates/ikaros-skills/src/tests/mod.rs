// SPDX-License-Identifier: GPL-3.0-only

#![cfg(test)]

pub(super) use super::*;
pub(super) use async_trait::async_trait;
pub(super) use ikaros_core::{AgentPermission, AgentProfile, ResolvedAgentProfile, Result};
pub(super) use ikaros_harness::{
    ApprovalStatus, ExecutionEnv, ExecutionSession, FileMetadata, FileSystem, LocalExecutionEnv,
    NetworkEgress, NetworkEgressRequest, NetworkEgressResponse, NetworkedExecutionEnv,
    ProcessOutput, ProcessRequest, ProcessRunner, Skill, SkillContext, SkillDescriptor,
    SkillDescriptorKind, SkillOutput, ToolExecutionMode, Toolset, ToolsetSelection,
};
pub(super) use ikaros_memory::{
    JsonlMemoryCandidateStore, JsonlMemoryJournal, JsonlWorkingMemoryStore, LocalMemoryStore,
    MemoryCandidateQuery, MemoryCandidateStatus, MemoryJournal, MemoryJournalAction, MemoryKind,
    MemoryQuery, MemoryRecord, MemoryRef, MemoryStore, WorkingMemoryRecord,
};
pub(super) use ikaros_models::{
    ModelContextProfile, ModelProvider, ModelRequest, ModelResponse, TokenUsage,
};
pub(super) use ikaros_rag::{LocalRagStore, RagQuery, RagStore};
pub(super) use ikaros_session::{
    AgentEventKind, SessionEntryKind, SessionId, SessionSource, SessionStore, SqliteSessionStore,
    TurnId,
};
pub(super) use ikaros_soul::PersonaLoader;
pub(super) use serde_json::json;
#[cfg(unix)]
pub(super) use std::os::unix::fs::PermissionsExt;
pub(super) use std::{
    fs,
    future::Future,
    path::Path,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
pub(super) use tokio::sync::Notify;

pub(super) struct HiddenExplicitOnlySkill;

#[async_trait]
impl Skill for HiddenExplicitOnlySkill {
    fn name(&self) -> &'static str {
        "hidden_explicit_only"
    }

    fn description(&self) -> &'static str {
        "Explicit invocation only test skill."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    fn risk_level(&self) -> ikaros_core::RiskLevel {
        ikaros_core::RiskLevel::SafeRead
    }

    fn descriptor(&self) -> SkillDescriptor {
        let mut descriptor = SkillDescriptor::from_skill(self);
        descriptor.disable_model_invocation = true;
        descriptor
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        Ok(SkillOutput::new("hidden explicit-only executed", json!({})))
    }
}

pub(super) struct TrackingEnv {
    reads: Arc<AtomicUsize>,
    writes: Arc<AtomicUsize>,
}

pub(super) struct TestProcessEnv {
    calls: Arc<AtomicUsize>,
}

pub(super) struct SequentialTestProcessEnv {
    calls: Arc<AtomicUsize>,
}

pub(super) struct RecordingProcessEnv {
    request: Arc<Mutex<Option<ProcessRequest>>>,
}

pub(super) struct ScriptedCodingModelProvider {
    calls: Arc<AtomicUsize>,
    responses: Vec<String>,
}

pub(super) struct BlockingCodingModelProvider {
    calls: Arc<AtomicUsize>,
    started: Arc<Notify>,
}

pub(super) struct ScriptedNetworkEnv {
    reads: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
    response: NetworkEgressResponse,
}

pub(super) struct RecordingNetwork {
    calls: Arc<AtomicUsize>,
    request: Arc<Mutex<Option<NetworkEgressRequest>>>,
    response: NetworkEgressResponse,
}

impl FileSystem for TrackingEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.path_metadata(path)
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.read_to_string(path)
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.read_bytes(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.write_string(path, content)
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.write_bytes(path, content)
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.create_dir_all(path)
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        LocalExecutionEnv.read_dir(path)
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.remove_file(path)
    }
}

impl ProcessRunner for TrackingEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        LocalExecutionEnv.run_process(request)
    }
}

impl NetworkEgress for TrackingEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        LocalExecutionEnv.send_network_request(request)
    }
}

impl ExecutionEnv for TrackingEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        context: SkillContext,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        Box::pin(async move { skill.execute(input, context).await })
    }
}

impl FileSystem for RecordingProcessEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>> {
        LocalExecutionEnv.path_metadata(path)
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        LocalExecutionEnv.read_to_string(path)
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        LocalExecutionEnv.read_bytes(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.write_string(path, content)
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.write_bytes(path, content)
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.create_dir_all(path)
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        LocalExecutionEnv.read_dir(path)
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.remove_file(path)
    }
}

impl ProcessRunner for RecordingProcessEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        *self.request.lock().expect("record request") = Some(request);
        Box::pin(async {
            Ok(ProcessOutput {
                status: 0,
                stdout: "ok".into(),
                stderr: String::new(),
            })
        })
    }
}

impl NetworkEgress for RecordingProcessEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        LocalExecutionEnv.send_network_request(request)
    }
}

impl ExecutionEnv for RecordingProcessEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        context: SkillContext,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        Box::pin(async move { skill.execute(input, context).await })
    }
}

impl FileSystem for ScriptedNetworkEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.path_metadata(path)
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.read_to_string(path)
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.read_bytes(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.write_string(path, content)
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.write_bytes(path, content)
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.create_dir_all(path)
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        LocalExecutionEnv.read_dir(path)
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.remove_file(path)
    }
}

impl ProcessRunner for ScriptedNetworkEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        LocalExecutionEnv.run_process(request)
    }
}

impl NetworkEgress for ScriptedNetworkEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.method, "POST");
        assert!(request.url.ends_with("/embeddings"));
        assert!(request.headers.contains_key("authorization"));
        let response = self.response.clone();
        Box::pin(async move { Ok(response) })
    }
}

impl NetworkEgress for RecordingNetwork {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.request.lock().expect("request lock") = Some(request);
        let response = self.response.clone();
        Box::pin(async move { Ok(response) })
    }
}

impl ExecutionEnv for ScriptedNetworkEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        context: SkillContext,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        Box::pin(async move { skill.execute(input, context).await })
    }
}

impl FileSystem for TestProcessEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>> {
        LocalExecutionEnv.path_metadata(path)
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        LocalExecutionEnv.read_to_string(path)
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        LocalExecutionEnv.read_bytes(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.write_string(path, content)
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.write_bytes(path, content)
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.create_dir_all(path)
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        LocalExecutionEnv.read_dir(path)
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.remove_file(path)
    }
}

impl ProcessRunner for TestProcessEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            assert_eq!(request.command, "cargo");
            assert!(
                request.args == vec!["test", "-p", "ikaros-coding"]
                    || request.args == vec!["fmt", "--all", "--", "--check"],
                "unexpected test command args: {:?}",
                request.args
            );
            let status = if request.args.first().map(String::as_str) == Some("fmt") {
                1
            } else {
                0
            };
            Ok(ProcessOutput {
                status,
                stdout: if status == 0 {
                    "test result: ok. 1 passed".into()
                } else {
                    String::new()
                },
                stderr: if status == 0 {
                    String::new()
                } else {
                    "Diff in src/lib.rs".into()
                },
            })
        })
    }
}

impl NetworkEgress for TestProcessEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        LocalExecutionEnv.send_network_request(request)
    }
}

impl ExecutionEnv for TestProcessEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        context: SkillContext,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        Box::pin(async move { skill.execute(input, context).await })
    }
}

impl FileSystem for SequentialTestProcessEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata>> + Send + 'a>> {
        LocalExecutionEnv.path_metadata(path)
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        LocalExecutionEnv.read_to_string(path)
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + 'a>> {
        LocalExecutionEnv.read_bytes(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.write_string(path, content)
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.write_bytes(path, content)
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.create_dir_all(path)
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        LocalExecutionEnv.read_dir(path)
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.remove_file(path)
    }
}

impl ProcessRunner for SequentialTestProcessEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            assert_eq!(request.command, "cargo");
            assert_eq!(request.args, vec!["test"]);
            let status = if call == 0 { 101 } else { 0 };
            Ok(ProcessOutput {
                status,
                stdout: if status == 0 {
                    "test result: ok. 1 passed".into()
                } else {
                    "test result: FAILED. expected 3".into()
                },
                stderr: String::new(),
            })
        })
    }
}

impl NetworkEgress for SequentialTestProcessEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        LocalExecutionEnv.send_network_request(request)
    }
}

impl ExecutionEnv for SequentialTestProcessEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        context: SkillContext,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        Box::pin(async move { skill.execute(input, context).await })
    }
}

#[async_trait::async_trait]
impl ModelProvider for ScriptedCodingModelProvider {
    fn name(&self) -> &str {
        "scripted-coding-model"
    }

    fn context_profile(&self) -> ModelContextProfile {
        ModelContextProfile::default()
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        let content = self.responses.get(index).cloned().unwrap_or_else(|| {
            r#"{"candidate_diff": null, "final_answer": "no more patches", "stop": true}"#.into()
        });
        let prompt_tokens = request
            .messages
            .iter()
            .map(|message| test_estimate_tokens(&message.content))
            .sum::<u32>();
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "scripted".into(),
            content: content.clone(),
            tool_calls: Vec::new(),
            usage: TokenUsage {
                prompt_tokens: Some(prompt_tokens),
                completion_tokens: Some(test_estimate_tokens(&content)),
                total_tokens: None,
                ..TokenUsage::default()
            },
            diagnostics: Vec::new(),
        })
    }
}

#[async_trait::async_trait]
impl ModelProvider for BlockingCodingModelProvider {
    fn name(&self) -> &str {
        "blocking-coding-model"
    }

    fn context_profile(&self) -> ModelContextProfile {
        ModelContextProfile::default()
    }

    async fn generate(&self, _request: ModelRequest) -> Result<ModelResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.notify_waiters();
        std::future::pending::<Result<ModelResponse>>().await
    }
}

pub(super) fn test_estimate_tokens(text: &str) -> u32 {
    if text.trim().is_empty() {
        return 0;
    }
    ((text.chars().count() as u32).saturating_add(3) / 4).max(1)
}

pub(super) fn test_env(root: &Path, workspace: &Path) -> SkillEnvironment {
    let rag_config = ikaros_core::RagConfig {
        embedding_provider: "hash".into(),
        embedding_model: "text-embedding-3-small".into(),
        ..ikaros_core::RagConfig::default()
    };

    SkillEnvironment {
        workspace_root: workspace.to_path_buf(),
        memory_store: LocalMemoryStore::new(root.join("memory"), "jsonl").expect("memory"),
        rag_index: LocalRagStore::new(root.join("rag"), "jsonl").expect("rag"),
        rag_config,
        rag_provider: ikaros_core::RemoteProviderConfig::default(),
        persona_path: root.join("persona"),
        skills_dir: root.join("skills"),
        voice_tts: ikaros_voice::VoiceProviderConfig::mock_tts(),
        voice_tts_provider: ikaros_core::RemoteProviderConfig::default(),
        voice_asr: ikaros_voice::VoiceProviderConfig::mock_asr(),
        voice_asr_provider: ikaros_core::RemoteProviderConfig::default(),
        web_search_provider: ikaros_core::RemoteProviderConfig::default(),
        coding_session: None,
    }
}

pub(super) fn write_plugin_runner(
    plugin_dir: &Path,
    unix_body: &str,
    windows_body: &str,
) -> &'static str {
    let (file_name, body) = if cfg!(windows) {
        ("runner.cmd", windows_body)
    } else {
        ("runner.sh", unix_body)
    };
    let runner = plugin_dir.join(file_name);
    fs::write(&runner, body).expect("runner");
    #[cfg(unix)]
    fs::set_permissions(&runner, fs::Permissions::from_mode(0o755)).expect("chmod");
    file_name
}

pub(super) fn plugin_write_agent() -> ResolvedAgentProfile {
    let mut profile = AgentProfile::build();
    profile.workspace_writes = AgentPermission::Allow;
    profile.shell = AgentPermission::Allow;
    profile.network = AgentPermission::Allow;
    ResolvedAgentProfile {
        name: "plugin-write".into(),
        profile,
    }
}

mod coding;
mod memory;
mod plugin;
mod rag;
mod registry;
mod voice;
mod web;
