// SPDX-License-Identifier: GPL-3.0-only

use super::*;

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

#[tokio::test]
async fn mock_model_coding_replay_fixture_persists_two_iteration_loop() {
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
        .run_scripted_turns_with_env(
            ikaros_coding::MockModelCodingInput {
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
            },
            &LocalExecutionEnv,
        )
        .await
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
    assert_eq!(
        requested.output["approval_context"]["operations"]["provider_call"],
        json!(true)
    );
    assert_eq!(
        requested.output["approval_context"]["operations"]["workspace_write"],
        json!(true)
    );
    assert_eq!(
        requested.output["approval_context"]["operations"]["shell"],
        json!(false)
    );
    assert_eq!(
        requested.output["approval_context"]["operations"]["shell_commands"],
        json!([])
    );
    assert_eq!(
        requested.output["approval_context"]["operations"]["shell_commands_inferred"],
        json!(false)
    );
    assert_eq!(
        requested.output["approval_context"]["provider"]["name"],
        json!("scripted-coding-model")
    );
    assert_eq!(
        requested.output["approval_context"]["session"]["session_id"],
        json!("model-approval-session")
    );
    assert_eq!(
        requested.output["approval_context"]["session"]["turn_id"],
        json!("model-approval-turn")
    );
    let approval_id = requested.output["approval_id"]
        .as_str()
        .expect("approval id");
    let approval = session
        .approvals
        .get(approval_id)
        .expect("approval lookup")
        .expect("approval record");
    assert_eq!(
        approval.request.context.as_ref().expect("approval context")["operations"]["provider_call"],
        json!(true)
    );
    assert_eq!(model_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fs::read_to_string(workspace.join("lib.rs")).expect("lib unchanged"),
        "pub fn value() -> i32 { 1 }\n"
    );
}

#[tokio::test]
async fn code_workflow_model_loop_cancels_while_waiting_for_provider() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::write(workspace.join("lib.rs"), "pub fn value() -> i32 { 1 }\n").expect("lib");
    let store = Arc::new(SqliteSessionStore::new(temp.path().join("agent-state")));
    let session_id = SessionId::from("model-wait-cancel-session");
    let turn_id = TurnId::from("model-wait-cancel-turn");
    let model_calls = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(Notify::new());
    let cancellation = ikaros_harness::CancellationToken::new();
    let model = Arc::new(BlockingCodingModelProvider {
        calls: model_calls.clone(),
        started: started.clone(),
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
            cancellation: cancellation.clone(),
        }),
        ..test_env(temp.path(), &workspace)
    });
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    let requested = session
        .execute_skill(
            &registry,
            "code_workflow",
            json!({
                "objective": "cancel while provider is running",
                "mode": "edit",
                "model_loop": true,
                "apply_patch": true,
                "run_tests": true,
                "max_iterations": 1,
                "test_commands": [{"command": "cargo test", "reason": "should not run after cancel"}],
                "session_id": session_id.as_str(),
                "turn_id": turn_id.as_str(),
            }),
        )
        .await
        .expect("request");
    let approval_id = requested.output["approval_id"]
        .as_str()
        .expect("approval id")
        .to_owned();
    session
        .decide_approval(&approval_id, ApprovalStatus::Approved, None)
        .expect("approve");

    let execute = tokio::spawn({
        let session = session.clone();
        let registry = registry.clone();
        async move {
            session
                .execute_approved_skill(&registry, &approval_id)
                .await
        }
    });
    started.notified().await;
    cancellation.cancel();
    let executed = tokio::time::timeout(std::time::Duration::from_millis(500), execute)
        .await
        .expect("provider wait should observe cancellation")
        .expect("join")
        .expect("execute approved");

    assert!(executed.ok);
    assert_eq!(model_calls.load(Ordering::SeqCst), 1);
    assert_eq!(executed.output["loop_report"]["status"], json!("cancelled"));
    assert!(
        executed.output["events"]
            .as_array()
            .expect("events")
            .iter()
            .any(|event| event["kind"] == json!("coding_loop_cancelled")
                && event["payload"]["phase"] == json!("awaiting_model_response"))
    );
    assert_eq!(
        fs::read_to_string(workspace.join("lib.rs")).expect("lib unchanged"),
        "pub fn value() -> i32 { 1 }\n"
    );
    let replay = store
        .replay_session(&session_id)
        .expect("replay")
        .expect("session replay");
    assert!(replay.agent_events.iter().any(|event| {
        event.turn_id == turn_id
            && matches!(event.kind, AgentEventKind::CodingTurn)
            && event.payload["kind"] == json!("coding_loop_cancelled")
            && event.payload["payload"]["phase"] == json!("awaiting_model_response")
    }));
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
