use super::*;

#[tokio::test]
async fn coding_runtime_applies_patch_tracks_diff_reviews_and_reports_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("Cargo.toml"), "[workspace]\n").expect("cargo");
    fs::create_dir_all(temp.path().join("src")).expect("src");
    fs::write(temp.path().join("src/lib.rs"), "pub fn old() {}\n").expect("lib");
    let context = CodingTurnContext::from_workspace(CodingTurnContextInput {
        workspace_root: temp.path().to_path_buf(),
        objective: "rename old function".into(),
        mode: CodingMode::Edit,
        session_id: Some("session-code".into()),
        turn_id: Some("turn-code".into()),
        instructions: Vec::new(),
        permission_profile: CodingPermissionProfile::default(),
        test_commands: Vec::new(),
    })
    .expect("context");
    let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-pub fn old() {}\n+pub fn new() {}\n";
    let test_analysis = TestFailureAnalyzer::analyze("cargo test", 0, "test result: ok", "");

    let report = DeterministicCodingRuntime
        .run_turn_with_env(
            CodingTurnInput {
                context,
                candidate_diff: Some(diff.into()),
                apply_patch: true,
                test_matrix: Vec::new(),
                test_analysis: Some(test_analysis),
            },
            &LocalExecutionEnv,
        )
        .await
        .expect("turn");

    assert_eq!(
        fs::read_to_string(temp.path().join("src/lib.rs")).expect("lib"),
        "pub fn new() {}\n"
    );
    assert!(report.patch_apply_report.is_some());
    assert!(
        report
            .turn_diff
            .unified_diff
            .as_deref()
            .is_some_and(|diff| {
                diff.contains("-pub fn old() {}") && diff.contains("+pub fn new() {}")
            })
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| event.kind == CodingTurnEventKind::GitBaselineCaptured)
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| event.kind == CodingTurnEventKind::PatchApplied)
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| event.kind == CodingTurnEventKind::DiffUpdated)
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| event.kind == CodingTurnEventKind::ReviewCompleted)
    );
    assert!(report.final_report.contains("Coding Turn Report"));
}

#[tokio::test]
async fn coding_runtime_emits_review_started_and_finding_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::write(temp.path().join("lib.rs"), "fn check() {}\n").expect("lib");
    let context = CodingTurnContext::from_workspace(CodingTurnContextInput {
        workspace_root: temp.path().to_path_buf(),
        objective: "review risky unwrap".into(),
        mode: CodingMode::Review,
        session_id: None,
        turn_id: None,
        instructions: Vec::new(),
        permission_profile: CodingPermissionProfile::default(),
        test_commands: Vec::new(),
    })
    .expect("context");
    let diff = "diff --git a/lib.rs b/lib.rs\n--- a/lib.rs\n+++ b/lib.rs\n@@ -1 +1,2 @@\n fn check() {}\n+let value = maybe.unwrap();\n";

    let report = DeterministicCodingRuntime
        .run_turn_with_env(
            CodingTurnInput {
                context,
                candidate_diff: Some(diff.into()),
                apply_patch: false,
                test_matrix: Vec::new(),
                test_analysis: None,
            },
            &LocalExecutionEnv,
        )
        .await
        .expect("turn");

    assert!(
        report
            .events
            .iter()
            .any(|event| event.kind == CodingTurnEventKind::ReviewStarted)
    );
    assert!(
        report.events.iter().any(|event| {
            event.kind == CodingTurnEventKind::ReviewFinding
                && event.payload["detail"]
                    .as_str()
                    .is_some_and(|detail| detail.contains("unwrap"))
        }),
        "review finding events should expose individual findings"
    );
}

#[tokio::test]
async fn coding_runtime_plan_mode_does_not_apply_candidate_patch() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("Cargo.toml"), "[workspace]\n").expect("cargo");
    fs::write(temp.path().join("lib.rs"), "old\n").expect("lib");
    let context = CodingTurnContext::from_workspace(CodingTurnContextInput {
        workspace_root: temp.path().to_path_buf(),
        objective: "plan only".into(),
        mode: CodingMode::Plan,
        session_id: None,
        turn_id: None,
        instructions: Vec::new(),
        permission_profile: CodingPermissionProfile::default(),
        test_commands: Vec::new(),
    })
    .expect("context");
    let diff =
        "diff --git a/lib.rs b/lib.rs\n--- a/lib.rs\n+++ b/lib.rs\n@@ -1 +1 @@\n-old\n+new\n";

    let report = DeterministicCodingRuntime
        .run_turn_with_env(
            CodingTurnInput {
                context,
                candidate_diff: Some(diff.into()),
                apply_patch: true,
                test_matrix: Vec::new(),
                test_analysis: None,
            },
            &LocalExecutionEnv,
        )
        .await
        .expect("turn");

    assert_eq!(
        fs::read_to_string(temp.path().join("lib.rs")).expect("lib"),
        "old\n"
    );
    assert!(report.patch_apply_report.is_none());
    assert!(
        report
            .events
            .iter()
            .any(|event| event.kind == CodingTurnEventKind::PatchSkipped)
    );
}

#[tokio::test]
async fn coding_runtime_marks_loop_passed_when_patch_and_tests_pass() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("Cargo.toml"), "[workspace]\n").expect("cargo");
    fs::write(temp.path().join("lib.rs"), "old\n").expect("lib");
    let context = CodingTurnContext::from_workspace(CodingTurnContextInput {
        workspace_root: temp.path().to_path_buf(),
        objective: "complete one coding loop".into(),
        mode: CodingMode::Edit,
        session_id: None,
        turn_id: None,
        instructions: Vec::new(),
        permission_profile: CodingPermissionProfile::default(),
        test_commands: Vec::new(),
    })
    .expect("context");
    let diff =
        "diff --git a/lib.rs b/lib.rs\n--- a/lib.rs\n+++ b/lib.rs\n@@ -1 +1 @@\n-old\n+new\n";
    let test_analysis = TestFailureAnalyzer::analyze("cargo test", 0, "test result: ok", "");

    let report = DeterministicCodingRuntime
        .run_turn_with_env(
            CodingTurnInput {
                context,
                candidate_diff: Some(diff.into()),
                apply_patch: true,
                test_matrix: vec![test_analysis],
                test_analysis: None,
            },
            &LocalExecutionEnv,
        )
        .await
        .expect("turn");

    assert_eq!(report.loop_report.status, CodingLoopStatus::Passed);
    assert_eq!(report.loop_report.iterations, 1);
    assert!(
        report
            .events
            .iter()
            .any(|event| event.kind == CodingTurnEventKind::LoopTerminated
                && event.payload["status"].as_str() == Some("passed"))
    );
}

#[tokio::test]
async fn coding_runtime_marks_loop_waiting_for_followup_patch_after_test_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("Cargo.toml"), "[workspace]\n").expect("cargo");
    fs::write(temp.path().join("lib.rs"), "old\n").expect("lib");
    let context = CodingTurnContext::from_workspace(CodingTurnContextInput {
        workspace_root: temp.path().to_path_buf(),
        objective: "needs one more patch".into(),
        mode: CodingMode::Edit,
        session_id: None,
        turn_id: None,
        instructions: Vec::new(),
        permission_profile: CodingPermissionProfile::default(),
        test_commands: Vec::new(),
    })
    .expect("context");
    let diff =
        "diff --git a/lib.rs b/lib.rs\n--- a/lib.rs\n+++ b/lib.rs\n@@ -1 +1 @@\n-old\n+new\n";
    let test_analysis = TestFailureAnalyzer::analyze("cargo test", 101, "test result: FAILED", "");

    let report = DeterministicCodingRuntime
        .run_turn_with_env(
            CodingTurnInput {
                context,
                candidate_diff: Some(diff.into()),
                apply_patch: true,
                test_matrix: vec![test_analysis],
                test_analysis: None,
            },
            &LocalExecutionEnv,
        )
        .await
        .expect("turn");

    assert_eq!(
        report.loop_report.status,
        CodingLoopStatus::AwaitingFollowUpPatch
    );
    assert!(
        report
            .loop_report
            .reason
            .contains("test evidence still has failures")
    );
}

#[tokio::test]
async fn mock_model_coding_runtime_applies_followup_patch_until_tests_pass() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("Cargo.toml"), "[workspace]\n").expect("cargo");
    fs::write(temp.path().join("lib.rs"), "pub fn value() -> i32 { 1 }\n").expect("lib");
    let context = CodingTurnContext::from_workspace(CodingTurnContextInput {
        workspace_root: temp.path().to_path_buf(),
        objective: "make value pass tests".into(),
        mode: CodingMode::Edit,
        session_id: Some("mock-coding-session".into()),
        turn_id: Some("mock-coding-turn".into()),
        instructions: Vec::new(),
        permission_profile: CodingPermissionProfile::default(),
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

    let report = MockModelCodingRuntime::default()
        .run_scripted_turns_with_env(
            MockModelCodingInput {
                context,
                turns: vec![
                    MockModelCodingTurn {
                        candidate_diff: Some(first_diff.into()),
                        test_matrix: vec![TestFailureAnalyzer::analyze(
                            "cargo test",
                            101,
                            "test result: FAILED",
                            "",
                        )],
                    },
                    MockModelCodingTurn {
                        candidate_diff: Some(second_diff.into()),
                        test_matrix: vec![TestFailureAnalyzer::analyze(
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
        .expect("mock coding loop");

    assert_eq!(
        fs::read_to_string(temp.path().join("lib.rs")).expect("lib"),
        "pub fn value() -> i32 { 3 }\n"
    );
    assert_eq!(report.loop_report.status, CodingLoopStatus::Passed);
    assert_eq!(report.loop_report.iterations, 2);
    assert_eq!(
        report
            .events
            .iter()
            .filter(|event| event.kind == CodingTurnEventKind::PatchApplied)
            .count(),
        2
    );
    assert_eq!(
        report
            .events
            .iter()
            .filter(|event| event.kind == CodingTurnEventKind::TestEvidenceRecorded)
            .count(),
        2
    );
    assert!(report.events.iter().any(|event| {
        event.kind == CodingTurnEventKind::LoopIterationStarted
            && event.payload["iteration"].as_u64() == Some(2)
    }));
}

#[tokio::test]
async fn mock_model_coding_runtime_applies_patches_through_execution_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("Cargo.toml"), "[workspace]\n").expect("cargo");
    fs::write(temp.path().join("lib.rs"), "pub fn value() -> i32 { 1 }\n").expect("lib");
    let context = CodingTurnContext::from_workspace(CodingTurnContextInput {
        workspace_root: temp.path().to_path_buf(),
        objective: "make value pass tests".into(),
        mode: CodingMode::Edit,
        session_id: Some("mock-coding-session".into()),
        turn_id: Some("mock-coding-turn".into()),
        instructions: Vec::new(),
        permission_profile: CodingPermissionProfile::default(),
        test_commands: Vec::new(),
    })
    .expect("context");
    let diff = "\
diff --git a/lib.rs b/lib.rs
--- a/lib.rs
+++ b/lib.rs
@@ -1 +1 @@
-pub fn value() -> i32 { 1 }
+pub fn value() -> i32 { 2 }
";
    let file_system = SelfModifyTrackingEnv::default();

    let report = MockModelCodingRuntime::default()
        .run_scripted_turns_with_env(
            MockModelCodingInput {
                context,
                turns: vec![MockModelCodingTurn {
                    candidate_diff: Some(diff.into()),
                    test_matrix: vec![TestFailureAnalyzer::analyze(
                        "cargo test",
                        0,
                        "test result: ok",
                        "",
                    )],
                }],
            },
            &file_system,
        )
        .await
        .expect("mock coding loop");

    assert_eq!(report.loop_report.status, CodingLoopStatus::Passed);
    assert_eq!(
        fs::read_to_string(temp.path().join("lib.rs")).expect("lib"),
        "pub fn value() -> i32 { 2 }\n"
    );
    assert!(
        file_system.write_string_count() > 0,
        "mock-model coding runtime must apply patches through ExecutionEnv"
    );
}
