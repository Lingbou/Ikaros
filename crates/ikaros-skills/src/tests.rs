// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use ikaros_harness::{ApprovalStatus, ExecutionSession};
use ikaros_memory::{LocalMemoryStore, MemoryKind, MemoryQuery, MemoryRecord, MemoryStore};
use ikaros_rag::{LocalRagStore, RagQuery, RagStore};
use ikaros_soul::PersonaLoader;
use serde_json::json;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{fs, path::Path};

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
        persona_path: root.join("persona.md"),
        skills_dir: root.join("skills"),
        voice_tts: ikaros_voice::VoiceProviderConfig::mock_tts(),
        voice_asr: ikaros_voice::VoiceProviderConfig::mock_asr(),
    }
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
async fn cloud_rag_search_requires_network_approval_before_provider_execution() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(SkillEnvironment {
        rag_config: ikaros_core::RagConfig {
            embedding_provider: "openai-compatible".into(),
            embedding_base_url: "https://example.invalid/v1".into(),
            embedding_api_key: "test-rag-key".into(),
            embedding_model: "embedding-model".into(),
            embedding_timeout_ms: 1000,
            embedding_max_retries: 0,
            ..ikaros_core::RagConfig::default()
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
    let runner = plugin_dir.join("runner.sh");
    fs::write(
        &runner,
        "#!/bin/sh\ninput=$(cat)\ncase \"$input\" in *abc123*) printf 'raw-ok token=abc123\\n' ;; *) printf 'missing raw input: %s\\n' \"$input\"; exit 2 ;; esac\n",
    )
    .expect("runner");
    #[cfg(unix)]
    fs::set_permissions(&runner, fs::Permissions::from_mode(0o755)).expect("chmod");
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
program = "runner.sh"
timeout_ms = 1000
"#,
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
    let runner = plugin_dir.join("runner.sh");
    fs::write(&runner, "#!/bin/sh\ncat >/dev/null\n").expect("runner");
    #[cfg(unix)]
    fs::set_permissions(&runner, fs::Permissions::from_mode(0o755)).expect("chmod");
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
program = "runner.sh"
timeout_ms = 1000
"#,
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
    let runner = plugin_dir.join("runner.sh");
    fs::write(
        &runner,
        "#!/bin/sh\ni=0\nwhile [ \"$i\" -lt 70000 ]; do printf x; i=$((i + 1)); done\n",
    )
    .expect("runner");
    #[cfg(unix)]
    fs::set_permissions(&runner, fs::Permissions::from_mode(0o755)).expect("chmod");
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
program = "runner.sh"
timeout_ms = 1000
"#,
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

    assert!(error.to_string().contains("exceeded"));
    assert!(
        error
            .to_string()
            .contains(&ikaros_harness::PLUGIN_COMMAND_MAX_OUTPUT_BYTES.to_string())
    );
}

#[tokio::test]
async fn command_backed_plugin_timeout_is_enforced() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path();
    let plugin_dir = temp.path().join("skills/hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    let runner = plugin_dir.join("runner.sh");
    fs::write(&runner, "#!/bin/sh\nsleep 1\n").expect("runner");
    #[cfg(unix)]
    fs::set_permissions(&runner, fs::Permissions::from_mode(0o755)).expect("chmod");
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
program = "runner.sh"
timeout_ms = 1
"#,
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
async fn cloud_voice_tts_requires_network_approval_before_provider_execution() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(SkillEnvironment {
        voice_tts: ikaros_voice::VoiceProviderConfig {
            provider: "openai-compatible".into(),
            base_url: "https://example.invalid/v1".into(),
            api_key: "test-voice-key".into(),
            model: "tts-model".into(),
            timeout_ms: 1000,
            max_retries: 0,
            voice: Some("nova".into()),
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
            base_url: "https://example.invalid/v1".into(),
            api_key: "test-voice-key".into(),
            model: "tts-model".into(),
            timeout_ms: 1000,
            max_retries: 0,
            voice: Some("nova".into()),
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

#[test]
fn persona_loader_skill_uses_default_parser_type() {
    let persona = PersonaLoader::parse(PersonaLoader::default_markdown()).expect("persona");
    assert_eq!(persona.identity.name, "Ikaros");
}
