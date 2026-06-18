// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use ikaros_core::Result;
use ikaros_harness::{
    ApprovalStatus, ExecutionEnv, ExecutionSession, FileMetadata, FileSystem, LocalExecutionEnv,
    NetworkEgress, NetworkEgressRequest, NetworkEgressResponse, ProcessOutput, ProcessRequest,
    ProcessRunner, Skill, SkillContext, SkillOutput,
};
use ikaros_memory::{
    JsonlMemoryCandidateStore, JsonlWorkingMemoryStore, LocalMemoryStore, MemoryCandidateQuery,
    MemoryCandidateStatus, MemoryKind, MemoryQuery, MemoryRecord, MemoryRef, MemoryStore,
    WorkingMemoryRecord,
};
use ikaros_models::{ModelContextProfile, ModelProvider, ModelRequest, ModelResponse, TokenUsage};
use ikaros_rag::{LocalRagStore, RagQuery, RagStore};
use ikaros_session::{
    AgentEventKind, SessionEntryKind, SessionId, SessionSource, SessionStore, SqliteSessionStore,
    TurnId,
};
use ikaros_soul::PersonaLoader;
use serde_json::json;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    future::Future,
    path::Path,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct TrackingEnv {
    reads: Arc<AtomicUsize>,
    writes: Arc<AtomicUsize>,
}

struct TestProcessEnv {
    calls: Arc<AtomicUsize>,
}

struct SequentialTestProcessEnv {
    calls: Arc<AtomicUsize>,
}

struct ScriptedCodingModelProvider {
    calls: Arc<AtomicUsize>,
    responses: Vec<String>,
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
        session: &'a ExecutionSession,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        Box::pin(async move {
            skill
                .execute(
                    input,
                    SkillContext {
                        session: session.clone(),
                    },
                )
                .await
        })
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
        session: &'a ExecutionSession,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        Box::pin(async move {
            skill
                .execute(
                    input,
                    SkillContext {
                        session: session.clone(),
                    },
                )
                .await
        })
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
        session: &'a ExecutionSession,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        Box::pin(async move {
            skill
                .execute(
                    input,
                    SkillContext {
                        session: session.clone(),
                    },
                )
                .await
        })
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
            },
            diagnostics: Vec::new(),
        })
    }
}

fn test_estimate_tokens(text: &str) -> u32 {
    if text.trim().is_empty() {
        return 0;
    }
    ((text.chars().count() as u32).saturating_add(3) / 4).max(1)
}

fn test_env(root: &Path, workspace: &Path) -> SkillEnvironment {
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
        persona_path: root.join("persona.md"),
        skills_dir: root.join("skills"),
        voice_tts: ikaros_voice::VoiceProviderConfig::mock_tts(),
        voice_tts_provider: ikaros_core::RemoteProviderConfig::default(),
        voice_asr: ikaros_voice::VoiceProviderConfig::mock_asr(),
        voice_asr_provider: ikaros_core::RemoteProviderConfig::default(),
        coding_session: None,
    }
}

fn write_plugin_runner(plugin_dir: &Path, unix_body: &str, windows_body: &str) -> &'static str {
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

#[test]
fn builtin_registry_contains_core_skill_groups() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let names = registry.names();

    for expected in [
        "fs_read",
        "memory_append",
        "rag_ingest",
        "voice_tts",
        "repo_scan",
        "code_edit_guarded",
        "task_summarize",
    ] {
        assert!(names.contains(&expected.to_string()), "missing {expected}");
    }
}

#[tokio::test]
async fn registry_blocks_temp_rag_ingest_through_policy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(workspace.join(".temp")).expect("mkdir");
    fs::write(workspace.join(".temp/secret.md"), "secret").expect("write");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    let result = session
        .execute_skill(&registry, "rag_ingest", json!({"path": ".temp/secret.md"}))
        .await
        .expect("skill");
    assert!(!result.ok);
    assert!(result.summary.contains(".temp"));
}

#[tokio::test]
async fn memory_skills_run_through_harness_and_reject_secret_updates() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let memory = LocalMemoryStore::new(temp.path().join("memory"), "jsonl").expect("memory");
    let env = SkillEnvironment {
        memory_store: memory.clone(),
        ..test_env(temp.path(), &workspace)
    };
    let registry = builtin_registry(env);
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let appended = session
        .execute_skill(
            &registry,
            "memory_append",
            json!({"kind": "project", "scope": "repo", "content": "remember local-first"}),
        )
        .await
        .expect("append");
    assert!(appended.ok);
    assert_eq!(memory.list(MemoryQuery::default()).expect("list").len(), 1);

    let record = memory
        .append(MemoryRecord::new(MemoryKind::Project, "repo", "old memory").expect("record"))
        .expect("append");
    let record_id = record.id.clone();
    let updated = session
        .execute_skill(
            &registry,
            "memory_update",
            json!({"id": record_id.clone(), "content": "new memory", "tags": ["edited"]}),
        )
        .await
        .expect("update");
    assert!(updated.ok);
    assert_eq!(updated.output["updated"]["content"], json!("new memory"));

    let rejected = session
        .execute_skill(
            &registry,
            "memory_update",
            json!({"id": record_id.clone(), "content": "token=abc123"}),
        )
        .await
        .expect_err("secret update rejected");
    assert!(rejected.to_string().contains("secret-like"));

    let deleted = session
        .execute_skill(&registry, "memory_delete", json!({"id": record_id}))
        .await
        .expect("delete");
    assert!(deleted.ok);
    assert_eq!(deleted.output["records_deleted"], json!(1));
}

#[tokio::test]
async fn memory_candidate_create_skill_writes_pending_inbox_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let memory_dir = temp.path().join("memory");
    let memory = LocalMemoryStore::new(&memory_dir, "jsonl").expect("memory");
    let env = SkillEnvironment {
        memory_store: memory.clone(),
        ..test_env(temp.path(), &workspace)
    };
    let registry = builtin_registry(env);
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let created = session
        .execute_skill(
            &registry,
            "memory_candidate_create",
            json!({
                "kind": "relationship",
                "scope": "default",
                "content": "User preference: concise updates",
                "reason": "preference_pattern",
                "confidence": 0.7,
                "tags": ["relationship", "chat-learned"],
                "source_ref": {
                    "type": "session_turn",
                    "data": {"session_id": "chat-1", "turn_id": "turn-1"}
                }
            }),
        )
        .await
        .expect("create candidate");

    assert!(created.ok);
    assert_eq!(created.output["created"], json!(true));
    assert!(
        memory
            .list(MemoryQuery::default())
            .expect("core memory")
            .is_empty(),
        "candidate creation must not promote into core memory"
    );

    let candidates = JsonlMemoryCandidateStore::new(&memory_dir)
        .list(MemoryCandidateQuery {
            status: Some(MemoryCandidateStatus::Pending),
            ..MemoryCandidateQuery::default()
        })
        .expect("pending candidates");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].content, "User preference: concise updates");
    assert_eq!(
        candidates[0].source_ref,
        Some(MemoryRef::SessionTurn {
            session_id: "chat-1".into(),
            turn_id: Some("turn-1".into())
        })
    );

    let duplicate = session
        .execute_skill(
            &registry,
            "memory_candidate_create",
            json!({
                "kind": "relationship",
                "scope": "default",
                "content": "User preference: concise updates",
                "reason": "preference_pattern"
            }),
        )
        .await
        .expect("duplicate candidate");
    assert!(duplicate.ok);
    assert_eq!(duplicate.output["created"], json!(false));
    assert_eq!(duplicate.output["id"], json!(candidates[0].id));
}

#[tokio::test]
async fn memory_delete_with_kind_does_not_delete_other_kinds_by_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let memory = LocalMemoryStore::new(temp.path().join("memory"), "jsonl").expect("memory");
    let env = SkillEnvironment {
        memory_store: memory.clone(),
        ..test_env(temp.path(), &workspace)
    };
    let registry = builtin_registry(env);
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    let project = memory
        .append(MemoryRecord::new(MemoryKind::Project, "repo", "project note").expect("record"))
        .expect("append project");

    let deleted = session
        .execute_skill(
            &registry,
            "memory_delete",
            json!({"id": project.id, "kind": "relationship"}),
        )
        .await
        .expect("kind guarded delete");

    assert!(deleted.ok);
    assert_eq!(deleted.output["records_deleted"], json!(0));
    assert_eq!(memory.list(MemoryQuery::default()).expect("list").len(), 1);
}

#[tokio::test]
async fn memory_delete_with_kind_finds_records_beyond_default_search_limit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let memory = LocalMemoryStore::new(temp.path().join("memory"), "jsonl").expect("memory");
    let env = SkillEnvironment {
        memory_store: memory.clone(),
        ..test_env(temp.path(), &workspace)
    };
    let registry = builtin_registry(env);
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    let mut target =
        MemoryRecord::new(MemoryKind::Project, "repo", "old project note").expect("record");
    target.created_at = "2000-01-01T00:00:00Z".into();
    let target = memory.append(target).expect("append target");
    for index in 0..25 {
        let mut record = MemoryRecord::new(
            MemoryKind::Project,
            "repo",
            format!("new project note {index}"),
        )
        .expect("record");
        record.created_at = format!("2099-01-01T00:00:{index:02}Z");
        memory.append(record).expect("append newer");
    }

    let deleted = session
        .execute_skill(
            &registry,
            "memory_delete",
            json!({"id": target.id, "kind": "project"}),
        )
        .await
        .expect("kind guarded delete");

    assert!(deleted.ok);
    assert_eq!(deleted.output["records_deleted"], json!(1));
    assert!(
        memory
            .list(MemoryQuery {
                kind: Some(MemoryKind::Project),
                ..MemoryQuery::default()
            })
            .expect("list")
            .iter()
            .all(|record| record.content != "old project note")
    );
}

#[tokio::test]
async fn memory_projection_skill_renders_core_memory_without_task_summaries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let memory = LocalMemoryStore::new(temp.path().join("memory"), "jsonl").expect("memory");
    let env = SkillEnvironment {
        memory_store: memory.clone(),
        ..test_env(temp.path(), &workspace)
    };
    let registry = builtin_registry(env);
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    memory
        .append(
            MemoryRecord::new(MemoryKind::User, "default", "User preference: concise")
                .expect("record"),
        )
        .expect("append user");
    memory
        .append(
            MemoryRecord::new(
                MemoryKind::Project,
                "repo",
                "Working convention: local-first memory",
            )
            .expect("record"),
        )
        .expect("append project");
    memory
        .append(
            MemoryRecord::new(
                MemoryKind::Task,
                "chat-session",
                "Turn summary\nuser: do this once",
            )
            .expect("record")
            .with_tags(vec!["turn-summary".into()]),
        )
        .expect("append task");

    let projection = session
        .execute_skill(
            &registry,
            "memory_projection",
            json!({"user_scope": "default", "project_scope": "repo"}),
        )
        .await
        .expect("projection");

    assert!(projection.ok);
    assert!(
        projection.output["user"]
            .as_str()
            .expect("user")
            .contains("concise")
    );
    assert!(
        projection.output["project"]
            .as_str()
            .expect("project")
            .contains("local-first memory")
    );
    assert!(
        !projection.output.to_string().contains("do this once"),
        "projection must not expose ordinary episode summaries as core memory"
    );
}

#[tokio::test]
async fn working_memory_list_skill_reads_session_scratchpad() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let memory = LocalMemoryStore::new(temp.path().join("memory"), "jsonl").expect("memory");
    let working = JsonlWorkingMemoryStore::new(temp.path().join("memory"));
    working
        .append(
            WorkingMemoryRecord::new(
                "session-1",
                MemoryKind::Task,
                "session-1",
                "Current task goal: finish memory projection",
                Some(24),
            )
            .expect("working memory")
            .with_source_ref(MemoryRef::SessionTurn {
                session_id: "session-1".into(),
                turn_id: Some("turn-1".into()),
            })
            .expect("source ref"),
        )
        .expect("append working memory");
    let env = SkillEnvironment {
        memory_store: memory,
        ..test_env(temp.path(), &workspace)
    };
    let registry = builtin_registry(env);
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(
            &registry,
            "working_memory_list",
            json!({"session_id": "session-1", "limit": 5}),
        )
        .await
        .expect("working memory list");

    assert!(result.ok);
    let records = result.output.as_array().expect("records");
    assert_eq!(records.len(), 1);
    assert!(
        records[0]["content"]
            .as_str()
            .expect("content")
            .contains("memory projection")
    );
}

#[tokio::test]
async fn rag_maintenance_skills_run_through_harness() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let doc = workspace.join("doc.md");
    fs::write(&doc, "cleanup index entry").expect("write");
    let rag = LocalRagStore::new(temp.path().join("rag"), "jsonl").expect("rag");
    let env = SkillEnvironment {
        rag_index: rag.clone(),
        ..test_env(temp.path(), &workspace)
    };
    let registry = builtin_registry(env);
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let ingest = session
        .execute_skill(
            &registry,
            "rag_ingest",
            json!({"path": "doc.md", "scope": "cleanup"}),
        )
        .await
        .expect("ingest");
    assert!(ingest.ok);
    let search = session
        .execute_skill(
            &registry,
            "rag_search",
            json!({"query": "cleanup", "scope": "cleanup"}),
        )
        .await
        .expect("search");
    assert!(search.ok);
    assert_eq!(search.output.as_array().expect("hits").len(), 1);

    fs::remove_file(&doc).expect("remove");
    let stale = session
        .execute_skill(&registry, "rag_stale", json!({}))
        .await
        .expect("stale");
    assert!(stale.ok);
    assert_eq!(
        stale
            .output
            .get("stale_files")
            .and_then(serde_json::Value::as_array)
            .expect("stale files")
            .len(),
        1
    );

    let deleted = session
        .execute_skill(&registry, "rag_delete_path", json!({"path": "doc.md"}))
        .await
        .expect("delete path");
    assert!(deleted.ok);
    assert_eq!(deleted.output["chunks_deleted"], json!(1));
    assert!(
        rag.search(RagQuery {
            query: "cleanup".into(),
            top_k: 5,
            scope: Some("cleanup".into()),
        })
        .expect("search after delete")
        .is_empty()
    );
}

#[tokio::test]
async fn rag_ingest_reads_workspace_files_through_execution_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("doc.md"), "env routed index entry").expect("doc");
    let rag = LocalRagStore::new(temp.path().join("rag"), "jsonl").expect("rag");
    let registry = builtin_registry(SkillEnvironment {
        rag_index: rag.clone(),
        ..test_env(temp.path(), &workspace)
    });
    let reads = Arc::new(AtomicUsize::new(0));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(
        Arc::new(TrackingEnv {
            reads: reads.clone(),
            writes: Arc::new(AtomicUsize::new(0)),
        }),
    );

    let ingest = session
        .execute_skill(
            &registry,
            "rag_ingest",
            json!({"path": "doc.md", "scope": "env"}),
        )
        .await
        .expect("ingest");
    let search = session
        .execute_skill(
            &registry,
            "rag_search",
            json!({"query": "routed", "scope": "env"}),
        )
        .await
        .expect("search");

    assert!(ingest.ok);
    assert!(search.ok);
    assert_eq!(search.output.as_array().expect("hits").len(), 1);
    assert!(
        reads.load(Ordering::SeqCst) > 0,
        "rag ingest workspace reads must go through ExecutionEnv"
    );
}

#[tokio::test]
async fn rag_stale_checks_workspace_metadata_through_execution_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let doc = workspace.join("doc.md");
    fs::write(&doc, "env stale candidate").expect("doc");
    let rag = LocalRagStore::new(temp.path().join("rag"), "jsonl").expect("rag");
    let registry = builtin_registry(SkillEnvironment {
        rag_index: rag.clone(),
        ..test_env(temp.path(), &workspace)
    });
    let reads = Arc::new(AtomicUsize::new(0));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(
        Arc::new(TrackingEnv {
            reads: reads.clone(),
            writes: Arc::new(AtomicUsize::new(0)),
        }),
    );
    session
        .execute_skill(
            &registry,
            "rag_ingest",
            json!({"path": "doc.md", "scope": "env"}),
        )
        .await
        .expect("ingest");
    let reads_before_stale = reads.load(Ordering::SeqCst);
    fs::remove_file(&doc).expect("remove");

    let stale = session
        .execute_skill(&registry, "rag_stale", json!({}))
        .await
        .expect("stale");

    assert!(stale.ok);
    assert_eq!(
        stale
            .output
            .get("stale_files")
            .and_then(serde_json::Value::as_array)
            .expect("stale files")
            .len(),
        1
    );
    assert!(
        reads.load(Ordering::SeqCst) > reads_before_stale,
        "rag stale metadata checks must go through ExecutionEnv"
    );
}

#[tokio::test]
async fn cloud_rag_search_requires_network_approval_before_provider_execution() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(SkillEnvironment {
        rag_config: ikaros_core::RagConfig {
            embedding_provider: "openai-compatible".into(),
            embedding_model: "embedding-model".into(),
            embedding_timeout_ms: 1000,
            embedding_max_retries: 0,
            ..ikaros_core::RagConfig::default()
        },
        rag_provider: ikaros_core::RemoteProviderConfig {
            api_key: "test-rag-key".into(),
            base_url: "https://example.invalid/v1".into(),
        },
        ..test_env(temp.path(), &workspace)
    });
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(&registry, "rag_search", json!({"query": "cloud retrieval"}))
        .await
        .expect("approval request");
    assert!(!result.ok);
    assert_eq!(result.output["decision"], json!("ask_user"));
    assert!(result.summary.contains("network action"));
}

#[tokio::test]
async fn run_tests_rejects_non_test_shell_commands() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(
            &registry,
            "run_tests",
            json!({"command": "echo unsafe > created.txt"}),
        )
        .await
        .expect("policy denial");
    assert!(!result.ok);
    assert_eq!(result.output["decision"], json!("deny"));
    assert!(!workspace.join("created.txt").exists());
}

#[tokio::test]
async fn shell_guarded_rejects_non_allowlisted_shell_strings() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(
            &registry,
            "shell_guarded",
            json!({"command": "echo unsafe > created.txt"}),
        )
        .await
        .expect("policy denial");

    assert!(!result.ok);
    assert_eq!(result.output["decision"], json!("deny"));
    assert!(!workspace.join("created.txt").exists());
}

#[tokio::test]
async fn command_backed_plugin_skill_runs_through_harness() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path();
    let plugin_dir = temp.path().join("skills/hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    let program = write_plugin_runner(
        &plugin_dir,
        "#!/bin/sh\ninput=$(cat)\ncase \"$input\" in *abc123*) printf 'raw-ok token=abc123\\n' ;; *) printf 'missing raw input: %s\\n' \"$input\"; exit 2 ;; esac\n",
        "@echo off\r\nfindstr /C:\"abc123\" >nul\r\nif errorlevel 1 (\r\n  echo missing raw input\r\n  exit /b 2\r\n)\r\necho raw-ok token=abc123\r\n",
    );
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Command-backed plugin."

[[skills]]
name = "echo"
description = "Echo redacted input."
risk = "safe_read"
input_schema = { type = "object", properties = { message = { type = "string" } } }

[skills.command]
program = "__PROGRAM__"
timeout_ms = 1000
"#
        .replace("__PROGRAM__", program),
    )
    .expect("manifest");
    let registry = builtin_registry(test_env(temp.path(), workspace));
    let session = ExecutionSession::new(workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(
            &registry,
            "plugin_command_run",
            json!({"name": "hello.echo", "input": {"message": "token=abc123"}}),
        )
        .await
        .expect("plugin run");

    assert!(result.ok);
    assert_eq!(result.output["plugin"], json!("hello"));
    assert_eq!(result.output["skill"], json!("echo"));
    assert_eq!(result.output["status"], json!(0));
    let stdout = result.output["stdout"].as_str().expect("stdout");
    assert!(stdout.contains("raw-ok"));
    assert!(stdout.contains("[REDACTED_SECRET]"));
    assert!(!stdout.contains("abc123"));
}

#[tokio::test]
async fn command_backed_plugin_rejects_oversized_stdin() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path();
    let plugin_dir = temp.path().join("skills/hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    let program = write_plugin_runner(
        &plugin_dir,
        "#!/bin/sh\ncat >/dev/null\n",
        "@echo off\r\nmore >nul\r\n",
    );
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Command-backed plugin."

[[skills]]
name = "echo"
description = "Echo redacted input."
risk = "safe_read"

[skills.command]
program = "__PROGRAM__"
timeout_ms = 10000
"#
        .replace("__PROGRAM__", program),
    )
    .expect("manifest");
    let registry = builtin_registry(test_env(temp.path(), workspace));
    let session = ExecutionSession::new(workspace, temp.path().join("audit"));
    let oversized = "x".repeat(ikaros_harness::PLUGIN_COMMAND_MAX_STDIN_BYTES + 1);

    let error = session
        .execute_skill(
            &registry,
            "plugin_command_run",
            json!({"name": "hello.echo", "input": {"message": oversized}}),
        )
        .await
        .expect_err("oversized stdin should fail");

    assert!(error.to_string().contains("stdin exceeds"));
}

#[tokio::test]
async fn command_backed_plugin_rejects_oversized_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path();
    let plugin_dir = temp.path().join("skills/hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    let program = write_plugin_runner(
        &plugin_dir,
        "#!/bin/sh\nprintf '%070000d' 0 | tr 0 x\n",
        concat!(
            "@echo off\r\n",
            "set \"chunk=xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\"\r\n",
            "for /L %%i in (1,1,600) do <nul set /p \"=%chunk%\"\r\n",
        ),
    );
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Command-backed plugin."

[[skills]]
name = "noisy"
description = "Emit too much output."
risk = "safe_read"

[skills.command]
program = "__PROGRAM__"
timeout_ms = 1000
"#
        .replace("__PROGRAM__", program),
    )
    .expect("manifest");
    let registry = builtin_registry(test_env(temp.path(), workspace));
    let session = ExecutionSession::new(workspace, temp.path().join("audit"));

    let error = session
        .execute_skill(
            &registry,
            "plugin_command_run",
            json!({"name": "hello.noisy", "input": {}}),
        )
        .await
        .expect_err("oversized output should fail");

    let error_text = error.to_string();
    assert!(
        error_text.contains("exceeded"),
        "unexpected oversized-output error: {error_text}"
    );
    assert!(error_text.contains(&ikaros_harness::PLUGIN_COMMAND_MAX_OUTPUT_BYTES.to_string()));
}

#[tokio::test]
async fn command_backed_plugin_timeout_is_enforced() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path();
    let plugin_dir = temp.path().join("skills/hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    let program = write_plugin_runner(
        &plugin_dir,
        "#!/bin/sh\nsleep 1\n",
        "@echo off\r\nping -n 2 127.0.0.1 >nul\r\n",
    );
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Command-backed plugin."

[[skills]]
name = "slow"
description = "Sleep too long."
risk = "safe_read"

[skills.command]
program = "__PROGRAM__"
timeout_ms = 1
"#
        .replace("__PROGRAM__", program),
    )
    .expect("manifest");
    let registry = builtin_registry(test_env(temp.path(), workspace));
    let session = ExecutionSession::new(workspace, temp.path().join("audit"));

    let error = session
        .execute_skill(
            &registry,
            "plugin_command_run",
            json!({"name": "hello.slow", "input": {}}),
        )
        .await
        .expect_err("timeout should fail");

    assert!(error.to_string().contains("timed out"));
}

#[cfg(unix)]
#[tokio::test]
async fn command_backed_plugin_rejects_symlinked_program_outside_plugin() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path();
    let plugin_dir = temp.path().join("skills/hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    let outside = temp.path().join("outside.sh");
    fs::write(&outside, "#!/bin/sh\nprintf outside\n").expect("outside");
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o755)).expect("chmod outside");
    std::os::unix::fs::symlink(&outside, plugin_dir.join("runner.sh")).expect("symlink");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Command-backed plugin."

[[skills]]
name = "escape"
description = "Escape plugin root."
risk = "safe_read"

[skills.command]
program = "runner.sh"
"#,
    )
    .expect("manifest");
    let registry = builtin_registry(test_env(temp.path(), workspace));
    let session = ExecutionSession::new(workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(
            &registry,
            "plugin_command_run",
            json!({"name": "hello.escape", "input": {}}),
        )
        .await
        .expect("policy denial");

    assert!(!result.ok);
    assert_eq!(result.output["decision"], json!("deny"));
}

#[tokio::test]
async fn code_plan_only_is_safe_but_guarded_edit_requests_approval() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::write(workspace.join("note.txt"), "old\nkeep\n").expect("note");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let plan = session
        .execute_skill(
            &registry,
            "code_edit_guarded",
            json!({"objective": "add tests", "plan_only": true}),
        )
        .await
        .expect("plan");
    assert!(plan.ok);
    assert_eq!(plan.output["plan"]["requires_approval"], json!(true));

    let iteration = session
        .execute_skill(
            &registry,
            "code_iterate",
            json!({
                "objective": "remove panic risk",
                "diff": "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,3 @@\n old\n keep\n+let value = maybe.unwrap();\n",
            }),
        )
        .await
        .expect("iterate");
    assert!(iteration.ok);
    assert_eq!(
        iteration.output["iteration"]["requires_guarded_edit"],
        json!(true)
    );
    assert!(
        iteration.output["iteration"]["guarded_edit_objective"]
            .as_str()
            .expect("objective")
            .contains("Potential panic path added")
    );
    assert_eq!(
        fs::read_to_string(workspace.join("note.txt")).expect("note before iterate"),
        "old\nkeep\n"
    );

    let workflow = session
        .execute_skill(
            &registry,
            "code_workflow",
            json!({
                "objective": "review token=abc123 safely",
                "diff": "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,3 @@\n old\n keep\n+let leaked = \"token=abc123\";\n",
            }),
        )
        .await
        .expect("workflow");
    assert!(workflow.ok);
    assert_eq!(workflow.summary, "coding turn completed");
    assert_eq!(workflow.output["context"]["mode"], json!("plan"));
    assert_eq!(
        workflow.output["patch_apply_report"],
        serde_json::Value::Null
    );
    assert!(
        workflow.output["events"]
            .as_array()
            .expect("events")
            .iter()
            .any(|event| event["kind"] == json!("patch_skipped"))
    );
    let workflow_json = serde_json::to_string(&workflow.output).expect("workflow json");
    assert!(workflow_json.contains("[REDACTED_SECRET]"));
    assert!(!workflow_json.contains("abc123"));
    assert_eq!(
        fs::read_to_string(workspace.join("note.txt")).expect("note before workflow"),
        "old\nkeep\n"
    );

    let guarded = session
        .execute_skill(
            &registry,
            "code_edit_guarded",
            json!({
                "objective": "edit note",
                "diff": "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n-old\n+new\n keep\n",
            }),
        )
        .await
        .expect("guarded");
    assert!(!guarded.ok);
    assert_eq!(guarded.output["decision"], json!("ask_user"));
    assert_eq!(
        fs::read_to_string(workspace.join("note.txt")).expect("note before approval"),
        "old\nkeep\n"
    );

    let approval_id = guarded.output["approval_id"].as_str().expect("approval id");
    session
        .decide_approval(approval_id, ApprovalStatus::Approved, None)
        .expect("approve");
    let executed = session
        .execute_approved_skill(&registry, approval_id)
        .await
        .expect("execute approved");
    assert!(executed.ok);
    assert_eq!(executed.summary, "guarded code edit applied");
    assert_eq!(executed.output["apply_report"]["files_changed"], json!(1));
    assert_eq!(
        fs::read_to_string(workspace.join("note.txt")).expect("note after approval"),
        "new\nkeep\n"
    );
}

#[tokio::test]
async fn code_workflow_mode_matrix_rejects_disallowed_operations() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::write(workspace.join("note.txt"), "old\n").expect("note");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let plan_patch = session
        .execute_skill(
            &registry,
            "code_workflow",
            json!({
                "objective": "plan must not patch",
                "mode": "plan",
                "apply_patch": true,
                "diff": "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-old\n+new\n",
            }),
        )
        .await
        .expect("plan patch request");
    assert!(!plan_patch.ok);
    assert_eq!(plan_patch.output["decision"], json!("deny"));

    let test_patch = session
        .execute_skill(
            &registry,
            "code_workflow",
            json!({
                "objective": "test mode must not patch",
                "mode": "test",
                "run_tests": true,
                "apply_patch": true,
                "diff": "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-old\n+new\n",
                "test_commands": [{"command": "cargo test -p ikaros-coding", "reason": "focused"}],
            }),
        )
        .await
        .expect("test patch request");
    assert!(!test_patch.ok);
    assert_eq!(test_patch.output["decision"], json!("deny"));

    let self_modify = session
        .execute_skill(
            &registry,
            "code_workflow",
            json!({
                "objective": "self modify is not ordinary workflow",
                "mode": "self_modify",
                "apply_patch": true,
                "diff": "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-old\n+new\n",
            }),
        )
        .await
        .expect("self modify request");
    assert!(!self_modify.ok);
    assert_eq!(self_modify.output["decision"], json!("deny"));
    assert_eq!(
        fs::read_to_string(workspace.join("note.txt")).expect("note unchanged"),
        "old\n"
    );
}

#[tokio::test]
async fn guarded_code_edit_applies_patch_through_execution_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::write(workspace.join("note.txt"), "old\nkeep\n").expect("note");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let writes = Arc::new(AtomicUsize::new(0));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(
        Arc::new(TrackingEnv {
            reads: Arc::new(AtomicUsize::new(0)),
            writes: writes.clone(),
        }),
    );

    let requested = session
        .execute_skill(
            &registry,
            "code_edit_guarded",
            json!({
                "objective": "edit note through env",
                "diff": "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n-old\n+new\n keep\n",
            }),
        )
        .await
        .expect("guarded request");
    assert!(!requested.ok);
    assert_eq!(writes.load(Ordering::SeqCst), 0);

    let approval_id = requested.output["approval_id"]
        .as_str()
        .expect("approval id");
    session
        .decide_approval(approval_id, ApprovalStatus::Approved, None)
        .expect("approve");
    let executed = session
        .execute_approved_skill(&registry, approval_id)
        .await
        .expect("execute approved");

    assert!(executed.ok);
    assert_eq!(executed.summary, "guarded code edit applied");
    assert!(
        writes.load(Ordering::SeqCst) > 0,
        "approved guarded patch writes must go through ExecutionEnv::write_string"
    );
    assert_eq!(
        fs::read_to_string(workspace.join("note.txt")).expect("note after approval"),
        "new\nkeep\n"
    );
}

#[tokio::test]
async fn code_workflow_edit_mode_applies_patch_through_execution_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::write(workspace.join("note.txt"), "old\nkeep\n").expect("note");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let writes = Arc::new(AtomicUsize::new(0));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(
        Arc::new(TrackingEnv {
            reads: Arc::new(AtomicUsize::new(0)),
            writes: writes.clone(),
        }),
    );

    let requested = session
        .execute_skill(
            &registry,
            "code_workflow",
            json!({
                "objective": "edit note in a coding turn",
                "mode": "edit",
                "apply_patch": true,
                "diff": "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n-old\n+new\n keep\n",
                "session_id": "coding-session",
                "turn_id": "coding-turn-1",
                "test_commands": [{"command": "cargo test -p ikaros-coding", "reason": "coding crate changed"}],
            }),
        )
        .await
        .expect("coding turn request");
    assert!(!requested.ok);
    assert_eq!(requested.output["decision"], json!("ask_user"));
    assert_eq!(writes.load(Ordering::SeqCst), 0);

    let approval_id = requested.output["approval_id"]
        .as_str()
        .expect("approval id");
    session
        .decide_approval(approval_id, ApprovalStatus::Approved, None)
        .expect("approve");
    let executed = session
        .execute_approved_skill(&registry, approval_id)
        .await
        .expect("execute approved");

    assert!(executed.ok);
    assert_eq!(executed.summary, "coding turn completed");
    assert!(
        writes.load(Ordering::SeqCst) > 0,
        "coding workflow patches must go through ExecutionEnv::write_string"
    );
    assert_eq!(executed.output["context"]["mode"], json!("edit"));
    assert_eq!(
        executed.output["patch_apply_report"]["files_changed"],
        json!(1)
    );
    assert_eq!(
        executed.output["turn_diff"]["summary"]["files_changed"],
        json!(1)
    );
    assert!(
        executed.output["events"]
            .as_array()
            .expect("events")
            .iter()
            .any(|event| event["kind"] == json!("patch_applied"))
    );
    assert_eq!(
        executed.output["suggested_tests"][0]["command"],
        json!("cargo test -p ikaros-coding")
    );
    assert_eq!(
        fs::read_to_string(workspace.join("note.txt")).expect("note after coding turn"),
        "new\nkeep\n"
    );
}

#[tokio::test]
async fn code_workflow_persists_coding_turn_timeline_to_session_store() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::write(workspace.join("note.txt"), "old\nkeep\n").expect("note");
    let store = Arc::new(SqliteSessionStore::new(temp.path().join("agent-state")));
    let session_id = SessionId::from("coding-session");
    let turn_id = TurnId::from("coding-turn");
    let registry = builtin_registry(SkillEnvironment {
        coding_session: Some(CodingSessionConfig {
            store: store.clone(),
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            source: SessionSource::Test,
            agent_id: Some("coding-agent".into()),
            workspace: Some(workspace.clone()),
            model_provider: None,
            cancellation: ikaros_harness::CancellationToken::new(),
        }),
        ..test_env(temp.path(), &workspace)
    });
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let requested = session
        .execute_skill(
            &registry,
            "code_workflow",
            json!({
                "objective": "edit note and persist coding timeline",
                "mode": "edit",
                "apply_patch": true,
                "diff": "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n-old\n+new\n keep\n",
            }),
        )
        .await
        .expect("coding workflow");
    assert!(!requested.ok);
    let approval_id = requested.output["approval_id"]
        .as_str()
        .expect("approval id");
    session
        .decide_approval(approval_id, ApprovalStatus::Approved, None)
        .expect("approve");
    let result = session
        .execute_approved_skill(&registry, approval_id)
        .await
        .expect("execute approved coding workflow");

    assert!(result.ok);
    let replay = store
        .replay_session(&session_id)
        .expect("replay")
        .expect("session replay");
    assert_eq!(replay.session.agent_id.as_deref(), Some("coding-agent"));
    assert_eq!(
        replay.session.workspace.as_deref(),
        Some(workspace.as_path())
    );
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| event.turn_id == turn_id
                && matches!(event.kind, AgentEventKind::CodingTurn)
                && event.payload["kind"] == json!("patch_applied")),
        "patch-applied coding event should be replayable"
    );
    assert!(
        replay.entries.iter().any(|entry| {
            entry.turn_id.as_ref() == Some(&turn_id)
                && entry.kind == SessionEntryKind::Custom
                && entry.payload["kind"] == json!("final_report_prepared")
        }),
        "final report coding entry should be replayable"
    );
}

#[tokio::test]
async fn code_workflow_replay_fixture_preserves_coding_turn_event_order() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::write(workspace.join("note.txt"), "old\n").expect("note");
    let store = Arc::new(SqliteSessionStore::new(temp.path().join("agent-state")));
    let session_id = SessionId::from("coding-replay-session");
    let turn_id = TurnId::from("coding-replay-turn");
    let registry = builtin_registry(SkillEnvironment {
        coding_session: Some(CodingSessionConfig {
            store: store.clone(),
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            source: SessionSource::Test,
            agent_id: Some("coding-agent".into()),
            workspace: Some(workspace.clone()),
            model_provider: None,
            cancellation: ikaros_harness::CancellationToken::new(),
        }),
        ..test_env(temp.path(), &workspace)
    });
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let requested = session
        .execute_skill(
            &registry,
            "code_workflow",
            json!({
                "objective": "replay a deterministic coding turn",
                "mode": "edit",
                "apply_patch": true,
                "diff": "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-old\n+new\n",
            }),
        )
        .await
        .expect("coding workflow request");
    assert!(!requested.ok);
    let approval_id = requested.output["approval_id"]
        .as_str()
        .expect("approval id");
    session
        .decide_approval(approval_id, ApprovalStatus::Approved, None)
        .expect("approve");
    session
        .execute_approved_skill(&registry, approval_id)
        .await
        .expect("execute approved coding workflow");

    let replay = store
        .replay_session(&session_id)
        .expect("replay")
        .expect("session replay");
    let kinds = replay
        .agent_events
        .iter()
        .filter(|event| {
            event.turn_id == turn_id && matches!(event.kind, AgentEventKind::CodingTurn)
        })
        .map(|event| {
            event.payload["kind"]
                .as_str()
                .unwrap_or("<missing>")
                .to_owned()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        kinds,
        vec![
            "context_prepared",
            "git_baseline_captured",
            "loop_iteration_started",
            "repo_scanned",
            "plan_prepared",
            "patch_applied",
            "diff_updated",
            "review_started",
            "review_finding",
            "review_completed",
            "iteration_planned",
            "loop_terminated",
            "final_report_prepared",
        ]
    );
}

#[test]
fn mock_model_coding_replay_fixture_persists_two_iteration_loop() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::write(workspace.join("lib.rs"), "pub fn value() -> i32 { 1 }\n").expect("lib");
    let store = Arc::new(SqliteSessionStore::new(temp.path().join("agent-state")));
    let session_id = SessionId::from("mock-replay-session");
    let turn_id = TurnId::from("mock-replay-turn");
    let context =
        ikaros_coding::CodingTurnContext::from_workspace(ikaros_coding::CodingTurnContextInput {
            workspace_root: workspace.clone(),
            objective: "mock model replay should preserve coding loop".into(),
            mode: ikaros_coding::CodingMode::Edit,
            session_id: Some(session_id.as_str().to_owned()),
            turn_id: Some(turn_id.as_str().to_owned()),
            instructions: Vec::new(),
            permission_profile: ikaros_coding::CodingPermissionProfile::default(),
            test_commands: Vec::new(),
        })
        .expect("context");
    let first_diff = "\
diff --git a/lib.rs b/lib.rs
--- a/lib.rs
+++ b/lib.rs
@@ -1 +1 @@
-pub fn value() -> i32 { 1 }
+pub fn value() -> i32 { 2 }
";
    let second_diff = "\
diff --git a/lib.rs b/lib.rs
--- a/lib.rs
+++ b/lib.rs
@@ -1 +1 @@
-pub fn value() -> i32 { 2 }
+pub fn value() -> i32 { 3 }
";
    let report = ikaros_coding::MockModelCodingRuntime::default()
        .run_scripted_turns(ikaros_coding::MockModelCodingInput {
            context,
            turns: vec![
                ikaros_coding::MockModelCodingTurn {
                    candidate_diff: Some(first_diff.into()),
                    test_matrix: vec![ikaros_coding::TestFailureAnalyzer::analyze(
                        "cargo test",
                        101,
                        "test result: FAILED",
                        "",
                    )],
                },
                ikaros_coding::MockModelCodingTurn {
                    candidate_diff: Some(second_diff.into()),
                    test_matrix: vec![ikaros_coding::TestFailureAnalyzer::analyze(
                        "cargo test",
                        0,
                        "test result: ok",
                        "",
                    )],
                },
            ],
        })
        .expect("mock runtime");
    crate::coding::persist_coding_turn_report(
        Some(&CodingSessionConfig {
            store: store.clone(),
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            source: SessionSource::Test,
            agent_id: Some("mock-coding-agent".into()),
            workspace: Some(workspace.clone()),
            model_provider: None,
            cancellation: ikaros_harness::CancellationToken::new(),
        }),
        &report,
    )
    .expect("persist report");

    let replay = store
        .replay_session(&session_id)
        .expect("replay")
        .expect("session replay");
    let patch_events = replay
        .agent_events
        .iter()
        .filter(|event| {
            event.turn_id == turn_id
                && matches!(event.kind, AgentEventKind::CodingTurn)
                && event.payload["kind"] == json!("patch_applied")
        })
        .count();
    let test_events = replay
        .agent_events
        .iter()
        .filter(|event| {
            event.turn_id == turn_id
                && matches!(event.kind, AgentEventKind::CodingTurn)
                && event.payload["kind"] == json!("test_evidence_recorded")
        })
        .count();
    let loop_event = replay
        .agent_events
        .iter()
        .find(|event| {
            event.turn_id == turn_id
                && matches!(event.kind, AgentEventKind::CodingTurn)
                && event.payload["kind"] == json!("loop_terminated")
        })
        .expect("loop terminated");

    assert_eq!(patch_events, 2);
    assert_eq!(test_events, 2);
    assert_eq!(loop_event.payload["payload"]["status"], json!("passed"));
    assert_eq!(loop_event.payload["payload"]["iterations"], json!(2));
}

#[tokio::test]
async fn code_workflow_run_tests_records_process_evidence_in_coding_timeline() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::write(workspace.join("note.txt"), "old\n").expect("note");
    let store = Arc::new(SqliteSessionStore::new(temp.path().join("agent-state")));
    let session_id = SessionId::from("coding-test-session");
    let turn_id = TurnId::from("coding-test-turn");
    let registry = builtin_registry(SkillEnvironment {
        coding_session: Some(CodingSessionConfig {
            store: store.clone(),
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            source: SessionSource::Test,
            agent_id: None,
            workspace: Some(workspace.clone()),
            model_provider: None,
            cancellation: ikaros_harness::CancellationToken::new(),
        }),
        ..test_env(temp.path(), &workspace)
    });
    let process_calls = Arc::new(AtomicUsize::new(0));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(
        Arc::new(TestProcessEnv {
            calls: process_calls.clone(),
        }),
    );

    let result = session
        .execute_skill(
            &registry,
            "code_workflow",
            json!({
                "objective": "run focused coding tests",
                "mode": "test",
                "run_tests": true,
                "test_commands": [{"command": "cargo test -p ikaros-coding", "reason": "focused coding check"}],
            }),
        )
        .await
        .expect("coding workflow");

    assert!(result.ok);
    assert_eq!(process_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        result.output["test_analysis"]["command"],
        json!("cargo test -p ikaros-coding")
    );
    assert_eq!(result.output["test_analysis"]["status"], json!(0));
    let replay = store
        .replay_session(&session_id)
        .expect("replay")
        .expect("session replay");
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| event.turn_id == turn_id
                && matches!(event.kind, AgentEventKind::CodingTurn)
                && event.payload["kind"] == json!("test_evidence_recorded")
                && event.payload["payload"]["status"] == json!(0)),
        "test evidence should be durable coding event"
    );
}

#[tokio::test]
async fn code_workflow_run_tests_executes_test_matrix_and_persists_each_result() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::write(workspace.join("note.txt"), "old\n").expect("note");
    let store = Arc::new(SqliteSessionStore::new(temp.path().join("agent-state")));
    let session_id = SessionId::from("coding-matrix-session");
    let turn_id = TurnId::from("coding-matrix-turn");
    let registry = builtin_registry(SkillEnvironment {
        coding_session: Some(CodingSessionConfig {
            store: store.clone(),
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            source: SessionSource::Test,
            agent_id: None,
            workspace: Some(workspace.clone()),
            model_provider: None,
            cancellation: ikaros_harness::CancellationToken::new(),
        }),
        ..test_env(temp.path(), &workspace)
    });
    let process_calls = Arc::new(AtomicUsize::new(0));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(
        Arc::new(TestProcessEnv {
            calls: process_calls.clone(),
        }),
    );

    let result = session
        .execute_skill(
            &registry,
            "code_workflow",
            json!({
                "objective": "run coding test matrix",
                "mode": "test",
                "run_tests": true,
                "test_commands": [
                    {"command": "cargo test -p ikaros-coding", "reason": "focused coding check"},
                    {"command": "cargo fmt --all -- --check", "reason": "format check"}
                ],
            }),
        )
        .await
        .expect("coding workflow");

    assert!(result.ok);
    assert_eq!(process_calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        result.output["test_matrix"]
            .as_array()
            .expect("matrix")
            .len(),
        2
    );
    assert_eq!(result.output["test_matrix"][0]["category"], json!("Passed"));
    assert_eq!(result.output["test_matrix"][1]["category"], json!("Format"));
    let replay = store
        .replay_session(&session_id)
        .expect("replay")
        .expect("session replay");
    let test_events = replay
        .agent_events
        .iter()
        .filter(|event| {
            event.turn_id == turn_id
                && matches!(event.kind, AgentEventKind::CodingTurn)
                && event.payload["kind"] == json!("test_evidence_recorded")
        })
        .count();
    assert_eq!(test_events, 2);
}

#[tokio::test]
async fn code_workflow_model_loop_requires_approval_before_calling_provider() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::write(workspace.join("lib.rs"), "pub fn value() -> i32 { 1 }\n").expect("lib");
    let model_calls = Arc::new(AtomicUsize::new(0));
    let model = Arc::new(ScriptedCodingModelProvider {
        calls: model_calls.clone(),
        responses: vec![r#"{"candidate_diff": "diff --git a/lib.rs b/lib.rs\n--- a/lib.rs\n+++ b/lib.rs\n@@ -1 +1 @@\n-pub fn value() -> i32 { 1 }\n+pub fn value() -> i32 { 2 }\n", "final_answer": "patch ready", "stop": false}"#.into()],
    });
    let registry = builtin_registry(SkillEnvironment {
        coding_session: Some(CodingSessionConfig {
            store: Arc::new(SqliteSessionStore::new(temp.path().join("agent-state"))),
            session_id: SessionId::from("model-approval-session"),
            turn_id: TurnId::from("model-approval-turn"),
            source: SessionSource::Test,
            agent_id: Some("coding-agent".into()),
            workspace: Some(workspace.clone()),
            model_provider: Some(model),
            cancellation: ikaros_harness::CancellationToken::new(),
        }),
        ..test_env(temp.path(), &workspace)
    });
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let requested = session
        .execute_skill(
            &registry,
            "code_workflow",
            json!({
                "objective": "model loop must not run before approval",
                "mode": "edit",
                "model_loop": true,
                "apply_patch": true,
                "max_iterations": 1,
            }),
        )
        .await
        .expect("request");

    assert!(!requested.ok);
    assert_eq!(requested.output["decision"], json!("ask_user"));
    assert_eq!(model_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fs::read_to_string(workspace.join("lib.rs")).expect("lib unchanged"),
        "pub fn value() -> i32 { 1 }\n"
    );
}

#[tokio::test]
async fn code_workflow_model_loop_applies_followup_patch_and_persists_replay() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::write(workspace.join("lib.rs"), "pub fn value() -> i32 { 1 }\n").expect("lib");
    let store = Arc::new(SqliteSessionStore::new(temp.path().join("agent-state")));
    let session_id = SessionId::from("model-loop-session");
    let turn_id = TurnId::from("model-loop-turn");
    let model_calls = Arc::new(AtomicUsize::new(0));
    let process_calls = Arc::new(AtomicUsize::new(0));
    let model = Arc::new(ScriptedCodingModelProvider {
        calls: model_calls.clone(),
        responses: vec![
            r#"{"candidate_diff": "diff --git a/lib.rs b/lib.rs\n--- a/lib.rs\n+++ b/lib.rs\n@@ -1 +1 @@\n-pub fn value() -> i32 { 1 }\n+pub fn value() -> i32 { 2 }\n", "final_answer": "first patch", "stop": false}"#.into(),
            r#"{"candidate_diff": "diff --git a/lib.rs b/lib.rs\n--- a/lib.rs\n+++ b/lib.rs\n@@ -1 +1 @@\n-pub fn value() -> i32 { 2 }\n+pub fn value() -> i32 { 3 }\n", "final_answer": "follow-up patch", "stop": false}"#.into(),
        ],
    });
    let registry = builtin_registry(SkillEnvironment {
        coding_session: Some(CodingSessionConfig {
            store: store.clone(),
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            source: SessionSource::Test,
            agent_id: Some("coding-agent".into()),
            workspace: Some(workspace.clone()),
            model_provider: Some(model),
            cancellation: ikaros_harness::CancellationToken::new(),
        }),
        ..test_env(temp.path(), &workspace)
    });
    let session = ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(
        Arc::new(SequentialTestProcessEnv {
            calls: process_calls.clone(),
        }),
    );

    let requested = session
        .execute_skill(
            &registry,
            "code_workflow",
            json!({
                "objective": "make value return three",
                "mode": "edit",
                "model_loop": true,
                "apply_patch": true,
                "run_tests": true,
                "max_iterations": 2,
                "test_commands": [{"command": "cargo test", "reason": "scripted focused test"}],
                "session_id": session_id.as_str(),
                "turn_id": turn_id.as_str(),
            }),
        )
        .await
        .expect("request");
    assert!(!requested.ok);
    let approval_id = requested.output["approval_id"]
        .as_str()
        .expect("approval id");
    session
        .decide_approval(approval_id, ApprovalStatus::Approved, None)
        .expect("approve");

    let executed = session
        .execute_approved_skill(&registry, approval_id)
        .await
        .expect("execute approved");

    assert!(executed.ok);
    assert_eq!(model_calls.load(Ordering::SeqCst), 2);
    assert_eq!(process_calls.load(Ordering::SeqCst), 2);
    assert_eq!(executed.output["loop_report"]["status"], json!("passed"));
    assert_eq!(executed.output["loop_report"]["iterations"], json!(2));
    assert_eq!(
        fs::read_to_string(workspace.join("lib.rs")).expect("lib updated"),
        "pub fn value() -> i32 { 3 }\n"
    );

    let replay = store
        .replay_session(&session_id)
        .expect("replay")
        .expect("session replay");
    let event_kinds = replay
        .agent_events
        .iter()
        .filter(|event| {
            event.turn_id == turn_id && matches!(event.kind, AgentEventKind::CodingTurn)
        })
        .map(|event| event.payload["kind"].as_str().unwrap_or("").to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        event_kinds
            .iter()
            .filter(|kind| kind.as_str() == "model_request_prepared")
            .count(),
        2
    );
    assert_eq!(
        event_kinds
            .iter()
            .filter(|kind| kind.as_str() == "model_response_received")
            .count(),
        2
    );
    assert_eq!(
        event_kinds
            .iter()
            .filter(|kind| kind.as_str() == "patch_applied")
            .count(),
        2
    );
    assert_eq!(
        event_kinds
            .iter()
            .filter(|kind| kind.as_str() == "test_evidence_recorded")
            .count(),
        2
    );
    assert!(replay.agent_events.iter().any(|event| {
        event.turn_id == turn_id
            && matches!(event.kind, AgentEventKind::CodingTurn)
            && event.payload["kind"] == json!("loop_terminated")
            && event.payload["payload"]["status"] == json!("passed")
    }));
}

#[tokio::test]
async fn code_workflow_model_loop_budget_limit_stops_before_provider_call() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::write(workspace.join("lib.rs"), "pub fn value() -> i32 { 1 }\n").expect("lib");
    let store = Arc::new(SqliteSessionStore::new(temp.path().join("agent-state")));
    let session_id = SessionId::from("model-budget-session");
    let turn_id = TurnId::from("model-budget-turn");
    let model_calls = Arc::new(AtomicUsize::new(0));
    let model = Arc::new(ScriptedCodingModelProvider {
        calls: model_calls.clone(),
        responses: vec![
            r#"{"candidate_diff": null, "final_answer": "unused", "stop": true}"#.into(),
        ],
    });
    let registry = builtin_registry(SkillEnvironment {
        coding_session: Some(CodingSessionConfig {
            store: store.clone(),
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            source: SessionSource::Test,
            agent_id: Some("coding-agent".into()),
            workspace: Some(workspace.clone()),
            model_provider: Some(model),
            cancellation: ikaros_harness::CancellationToken::new(),
        }),
        ..test_env(temp.path(), &workspace)
    });
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let requested = session
        .execute_skill(
            &registry,
            "code_workflow",
            json!({
                "objective": "budget must stop model call",
                "mode": "plan",
                "model_loop": true,
                "max_iterations": 1,
                "model_token_budget": 1,
                "session_id": session_id.as_str(),
                "turn_id": turn_id.as_str(),
            }),
        )
        .await
        .expect("request");
    assert!(!requested.ok);
    let approval_id = requested.output["approval_id"]
        .as_str()
        .expect("network approval");
    session
        .decide_approval(approval_id, ApprovalStatus::Approved, None)
        .expect("approve");
    let executed = session
        .execute_approved_skill(&registry, approval_id)
        .await
        .expect("execute approved");

    assert!(executed.ok);
    assert_eq!(model_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        executed.output["loop_report"]["status"],
        json!("budget_exceeded")
    );
    let replay = store
        .replay_session(&session_id)
        .expect("replay")
        .expect("session replay");
    assert!(replay.agent_events.iter().any(|event| {
        event.turn_id == turn_id
            && matches!(event.kind, AgentEventKind::CodingTurn)
            && event.payload["kind"] == json!("model_budget_exceeded")
    }));
}

#[tokio::test]
async fn code_workflow_model_loop_honors_cancelled_token_before_provider_call() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::write(workspace.join("lib.rs"), "pub fn value() -> i32 { 1 }\n").expect("lib");
    let store = Arc::new(SqliteSessionStore::new(temp.path().join("agent-state")));
    let session_id = SessionId::from("model-cancel-session");
    let turn_id = TurnId::from("model-cancel-turn");
    let model_calls = Arc::new(AtomicUsize::new(0));
    let cancellation = ikaros_harness::CancellationToken::new();
    cancellation.cancel();
    let model = Arc::new(ScriptedCodingModelProvider {
        calls: model_calls.clone(),
        responses: vec![
            r#"{"candidate_diff": null, "final_answer": "unused", "stop": true}"#.into(),
        ],
    });
    let registry = builtin_registry(SkillEnvironment {
        coding_session: Some(CodingSessionConfig {
            store: store.clone(),
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            source: SessionSource::Test,
            agent_id: Some("coding-agent".into()),
            workspace: Some(workspace.clone()),
            model_provider: Some(model),
            cancellation,
        }),
        ..test_env(temp.path(), &workspace)
    });
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let requested = session
        .execute_skill(
            &registry,
            "code_workflow",
            json!({
                "objective": "cancel before provider",
                "mode": "plan",
                "model_loop": true,
                "max_iterations": 1,
                "session_id": session_id.as_str(),
                "turn_id": turn_id.as_str(),
            }),
        )
        .await
        .expect("request");
    assert!(!requested.ok);
    let approval_id = requested.output["approval_id"]
        .as_str()
        .expect("network approval");
    session
        .decide_approval(approval_id, ApprovalStatus::Approved, None)
        .expect("approve");
    let executed = session
        .execute_approved_skill(&registry, approval_id)
        .await
        .expect("execute approved");

    assert!(executed.ok);
    assert_eq!(model_calls.load(Ordering::SeqCst), 0);
    assert_eq!(executed.output["loop_report"]["status"], json!("cancelled"));
    let replay = store
        .replay_session(&session_id)
        .expect("replay")
        .expect("session replay");
    assert!(replay.agent_events.iter().any(|event| {
        event.turn_id == turn_id
            && matches!(event.kind, AgentEventKind::CodingTurn)
            && event.payload["kind"] == json!("coding_loop_cancelled")
    }));
}

#[tokio::test]
async fn code_workflow_model_loop_loads_ikaros_instruction_sources() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(workspace.join(".ikaros")).expect("mkdir");
    fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::write(workspace.join("lib.rs"), "pub fn value() -> i32 { 1 }\n").expect("lib");
    fs::write(
        workspace.join("IKAROS.md"),
        "Use focused tests. Never expose token=abc123.",
    )
    .expect("instructions");
    fs::write(
        workspace.join(".ikaros/instructions.md"),
        "Prefer guarded patches.",
    )
    .expect("local instructions");
    let model_calls = Arc::new(AtomicUsize::new(0));
    let model = Arc::new(ScriptedCodingModelProvider {
        calls: model_calls.clone(),
        responses: vec![
            r#"{"candidate_diff": null, "final_answer": "noted", "stop": true}"#.into(),
        ],
    });
    let registry = builtin_registry(SkillEnvironment {
        coding_session: Some(CodingSessionConfig {
            store: Arc::new(SqliteSessionStore::new(temp.path().join("agent-state"))),
            session_id: SessionId::from("instruction-session"),
            turn_id: TurnId::from("instruction-turn"),
            source: SessionSource::Test,
            agent_id: Some("coding-agent".into()),
            workspace: Some(workspace.clone()),
            model_provider: Some(model),
            cancellation: ikaros_harness::CancellationToken::new(),
        }),
        ..test_env(temp.path(), &workspace)
    });
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let requested = session
        .execute_skill(
            &registry,
            "code_workflow",
            json!({
                "objective": "load instruction sources",
                "mode": "plan",
                "model_loop": true,
                "max_iterations": 1,
            }),
        )
        .await
        .expect("request");
    let approval_id = requested.output["approval_id"]
        .as_str()
        .expect("network approval");
    session
        .decide_approval(approval_id, ApprovalStatus::Approved, None)
        .expect("approve");
    let executed = session
        .execute_approved_skill(&registry, approval_id)
        .await
        .expect("execute approved");

    assert!(executed.ok);
    let instructions = executed.output["context"]["instructions"]
        .as_array()
        .expect("instructions");
    assert_eq!(instructions.len(), 2);
    let rendered = serde_json::to_string(instructions).expect("instructions json");
    assert!(rendered.contains("IKAROS.md"));
    assert!(rendered.contains(".ikaros/instructions.md"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
    assert!(!rendered.contains("abc123"));
}

#[tokio::test]
async fn voice_tts_redacts_text_and_audit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(
            &registry,
            "voice_tts",
            json!({"text": "say sk-not-real", "format": "wav", "language": "en"}),
        )
        .await
        .expect("tts");
    assert!(result.ok);
    assert_eq!(result.output["provider"], json!("mock-tts"));
    assert!(result.output["bytes_len"].as_u64().expect("bytes") > 0);
    assert!(
        !result.output["redacted_text_preview"]
            .as_str()
            .expect("preview")
            .contains("sk-not-real")
    );

    let audit = fs::read_to_string(session.audit.path()).expect("audit");
    assert!(!audit.contains("sk-not-real"));
    assert!(audit.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn voice_tts_output_path_requires_approval_then_writes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let requested = session
        .execute_skill(
            &registry,
            "voice_tts",
            json!({"text": "hello voice", "format": "wav", "path": "voice/out.mock.wav"}),
        )
        .await
        .expect("approval request");
    assert!(!requested.ok);
    assert_eq!(requested.output["decision"], json!("ask_user"));
    assert!(!workspace.join("voice/out.mock.wav").exists());

    let approval_id = requested.output["approval_id"]
        .as_str()
        .expect("approval id");
    session
        .decide_approval(approval_id, ApprovalStatus::Approved, None)
        .expect("approve");
    let executed = session
        .execute_approved_skill(&registry, approval_id)
        .await
        .expect("execute approved");
    assert!(executed.ok);
    let audio = fs::read_to_string(workspace.join("voice/out.mock.wav")).expect("audio");
    assert!(audio.contains("IKAROS_MOCK_TTS"));
    assert!(audio.contains("hello voice"));
}

#[tokio::test]
async fn voice_tts_output_path_writes_through_execution_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let writes = Arc::new(AtomicUsize::new(0));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(
        Arc::new(TrackingEnv {
            reads: Arc::new(AtomicUsize::new(0)),
            writes: writes.clone(),
        }),
    );

    let requested = session
        .execute_skill(
            &registry,
            "voice_tts",
            json!({"text": "hello voice", "format": "wav", "path": "voice/out.mock.wav"}),
        )
        .await
        .expect("approval request");
    let approval_id = requested.output["approval_id"]
        .as_str()
        .expect("approval id");
    session
        .decide_approval(approval_id, ApprovalStatus::Approved, None)
        .expect("approve");
    let executed = session
        .execute_approved_skill(&registry, approval_id)
        .await
        .expect("execute approved");

    assert!(executed.ok);
    assert!(
        writes.load(Ordering::SeqCst) > 0,
        "voice output writes must go through ExecutionEnv"
    );
}

#[tokio::test]
async fn cloud_voice_tts_requires_network_approval_before_provider_execution() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(SkillEnvironment {
        voice_tts: ikaros_voice::VoiceProviderConfig {
            provider: "openai-compatible".into(),
            model: "tts-model".into(),
            timeout_ms: 1000,
            max_retries: 0,
            voice: Some("nova".into()),
        },
        voice_tts_provider: ikaros_core::RemoteProviderConfig {
            api_key: "test-voice-key".into(),
            base_url: "https://example.invalid/v1".into(),
        },
        ..test_env(temp.path(), &workspace)
    });
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(
            &registry,
            "voice_tts",
            json!({"text": "hello cloud voice", "format": "mp3"}),
        )
        .await
        .expect("approval request");
    assert!(!result.ok);
    assert_eq!(result.output["decision"], json!("ask_user"));
    assert!(result.summary.contains("network action"));
}

#[tokio::test]
async fn cloud_voice_tts_with_output_path_requires_network_approval_before_file_write() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(SkillEnvironment {
        voice_tts: ikaros_voice::VoiceProviderConfig {
            provider: "openai-compatible".into(),
            model: "tts-model".into(),
            timeout_ms: 1000,
            max_retries: 0,
            voice: Some("nova".into()),
        },
        voice_tts_provider: ikaros_core::RemoteProviderConfig {
            api_key: "test-voice-key".into(),
            base_url: "https://example.invalid/v1".into(),
        },
        ..test_env(temp.path(), &workspace)
    });
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(
            &registry,
            "voice_tts",
            json!({
                "text": "hello cloud voice",
                "format": "mp3",
                "path": "voice/out.mp3",
            }),
        )
        .await
        .expect("approval request");
    assert!(!result.ok);
    assert_eq!(result.output["decision"], json!("ask_user"));
    assert!(result.summary.contains("network action"));
    assert!(!workspace.join("voice/out.mp3").exists());
}

#[tokio::test]
async fn voice_asr_reads_workspace_audio_without_path_transcript() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("sample.wav"), b"mock audio").expect("audio");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(
            &registry,
            "voice_asr",
            json!({
                "path": "sample.wav",
                "format": "wav",
                "sample_rate_hz": 16000,
                "language": "en"
            }),
        )
        .await
        .expect("asr");
    assert!(result.ok);
    assert_eq!(result.output["provider"], json!("mock-asr"));
    assert_eq!(result.output["audio"]["format"], json!("wav"));
    assert_eq!(result.output["audio"]["sample_rate_hz"], json!(16000));
    assert_eq!(result.output["audio"]["language"], json!("en"));
    assert_eq!(
        result.output["transcript"]["text"],
        json!("mock transcript")
    );
    assert!(
        !result.output["transcript"]["text"]
            .as_str()
            .expect("transcript")
            .contains("sample.wav")
    );
}

#[tokio::test]
async fn voice_asr_reads_audio_through_execution_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("sample.wav"), b"mock audio").expect("audio");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let reads = Arc::new(AtomicUsize::new(0));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(
        Arc::new(TrackingEnv {
            reads: reads.clone(),
            writes: Arc::new(AtomicUsize::new(0)),
        }),
    );

    let result = session
        .execute_skill(
            &registry,
            "voice_asr",
            json!({
                "path": "sample.wav",
                "format": "wav",
                "sample_rate_hz": 16000,
                "language": "en"
            }),
        )
        .await
        .expect("asr");

    assert!(result.ok);
    assert!(
        reads.load(Ordering::SeqCst) > 0,
        "voice ASR audio reads must go through ExecutionEnv"
    );
}

#[test]
fn persona_loader_skill_uses_default_parser_type() {
    let persona = PersonaLoader::parse(PersonaLoader::default_markdown()).expect("persona");
    assert_eq!(persona.identity.name, "Ikaros");
}
