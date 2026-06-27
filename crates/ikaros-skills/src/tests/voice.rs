// SPDX-License-Identifier: GPL-3.0-only

use super::*;

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
