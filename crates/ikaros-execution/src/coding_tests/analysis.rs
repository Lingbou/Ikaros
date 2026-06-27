use super::*;

#[test]
fn scans_rust_repo_without_temp() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("Cargo.toml"), "[workspace]").expect("write");
    fs::create_dir(temp.path().join(".temp")).expect("temp dir");
    fs::write(temp.path().join(".temp/ignore.rs"), "ignored").expect("write ignored");
    let repo = RepoScanner::new(temp.path()).scan().expect("scan");
    assert_eq!(repo.files.len(), 1);
    assert_eq!(repo.package_files.len(), 1);
}

#[cfg(unix)]
#[test]
fn repo_scan_skips_symlinked_directories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(workspace.join("Cargo.toml"), "[workspace]").expect("workspace cargo");
    fs::write(
        outside.join("Cargo.toml"),
        "[package]\nname = \"outside\"\n",
    )
    .expect("outside cargo");
    symlink(&outside, workspace.join("linked")).expect("dir symlink");

    let repo = RepoScanner::new(&workspace).scan().expect("scan");

    assert_eq!(repo.package_files, vec![workspace.join("Cargo.toml")]);
    assert!(
        repo.files
            .iter()
            .all(|file| !file.path.starts_with(&outside))
    );
}

#[test]
fn summarizes_diff() {
    let summary = DiffSummarizer::summarize("diff --git a/a b/a\n+++ b/a\n--- a/a\n+new\n-old\n");
    assert_eq!(summary.files_changed, 1);
    assert_eq!(summary.insertions_hint, 1);
    assert_eq!(summary.deletions_hint, 1);
}

#[test]
fn analyzes_compile_failure_without_leaking_secrets() {
    let analysis = TestFailureAnalyzer::analyze(
        "cargo test",
        101,
        "",
        "error[E0425]: cannot find value `x`\nsecret token=abc123",
    );
    assert_eq!(analysis.category, TestFailureCategory::Compile);
    assert!(analysis.summary.contains("compile"));
    assert!(!analysis.summary.contains("abc123"));
    assert!(analysis.suggested_next_steps[0].contains("compiler error"));
}

#[test]
fn analyzes_failed_rust_tests() {
    let output = "\
running 2 tests
test tests::passes ... ok
test tests::fails ... FAILED

failures:

    tests::fails

test result: FAILED. 1 passed; 1 failed
";
    let analysis = TestFailureAnalyzer::analyze("cargo test", 101, output, "");
    assert_eq!(analysis.category, TestFailureCategory::TestFailure);
    assert_eq!(analysis.failed_tests, vec!["tests::fails"]);
    assert!(analysis.summary.contains("tests::fails"));
}

#[test]
fn analyzes_successful_test_command() {
    let analysis = TestFailureAnalyzer::analyze("cargo test", 0, "test result: ok", "");
    assert_eq!(analysis.category, TestFailureCategory::Passed);
    assert!(analysis.likely_causes.is_empty());
}

#[test]
fn rejects_allowlisted_test_commands_with_workspace_escape_paths() {
    for command in [
        "cargo test --manifest-path ../outside/Cargo.toml",
        "pytest /tmp/outside/tests",
        "npm run test -- --config=../outside/config.js",
    ] {
        let error = validate_test_command(command)
            .expect_err("path escapes must be rejected even for allowlisted test runners");
        assert!(
            error.to_string().contains("outside the allowed"),
            "{command}: {error}"
        );
    }
}

#[test]
fn rejects_windows_rooted_test_command_paths_on_all_platforms() {
    for command in [r"pytest \outside\tests", r"pytest \\server\share\tests"] {
        let error = validate_test_command(command)
            .expect_err("Windows-rooted paths must be rejected on every CI platform");
        assert!(
            error.to_string().contains("outside the allowed"),
            "{command}: {error}"
        );
    }
}

#[test]
fn review_assistant_flags_diff_and_test_risks() {
    let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,2 @@\n old\n+let value = maybe.unwrap();\n";
    let test_analysis = TestFailureAnalyzer::analyze(
        "cargo test",
        101,
        "test tests::fails ... FAILED\n\nfailures:\n    tests::fails\n",
        "",
    );
    let report = CodeReviewAssistant::review(diff, Some(test_analysis));
    assert_eq!(report.diff_summary.files_changed, 1);
    assert!(report.findings.iter().any(|finding| {
        finding.severity == ReviewSeverity::Medium && finding.title == "Potential panic path added"
    }));
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.severity == ReviewSeverity::High
                && finding.title == "Tests are not passing")
    );
    assert!(report.markdown.contains("Review Notes"));
}

#[test]
fn review_assistant_handles_empty_diff() {
    let report = CodeReviewAssistant::review("", None);
    assert_eq!(report.changed_files.len(), 0);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.title == "No diff detected")
    );
}

#[test]
fn patch_iteration_planner_prioritizes_blockers_and_tests() {
    let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,3 @@\n fn x() {}\n+let leaked = \"token=abc123\";\n+let value = maybe.unwrap();\n";
    let report = CodeReviewAssistant::review(diff, None);
    let repo = RepoMap {
        root: PathBuf::from("."),
        files: vec![RepoFile {
            path: PathBuf::from("src/lib.rs"),
            kind: RepoFileKind::Rust,
        }],
        package_files: vec![PathBuf::from("Cargo.toml")],
    };
    let plan = PatchIterationPlanner::plan("clean review findings", &report, &repo);
    assert_eq!(plan.priority, ReviewSeverity::High);
    assert!(plan.requires_guarded_edit);
    assert!(!plan.ready_for_approval);
    assert!(
        plan.blockers
            .iter()
            .any(|finding| finding.title == "Secret-like content added")
    );
    assert!(
        plan.guarded_edit_objective
            .contains("Potential panic path added")
    );
    assert!(
        plan.suggested_tests
            .iter()
            .any(|test| test.command.contains("cargo clippy"))
    );
    assert!(plan.markdown.contains("Patch Iteration Plan"));
    assert!(!plan.markdown.contains("abc123"));
}

#[test]
fn coding_workflow_orders_codex_style_steps_and_redacts_secret() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::create_dir_all(temp.path().join("src")).expect("src");
    fs::write(temp.path().join("src/lib.rs"), "pub fn old() {}\n").expect("lib");
    let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1,2 @@\n pub fn old() {}\n+let leaked = \"token=abc123\";\n";
    let test_analysis = TestFailureAnalyzer::analyze("cargo test", 0, "test result: ok", "");

    let report = CodingWorkflow::new(temp.path())
        .run(CodingWorkflowInput {
            objective: "fix token=abc123 safely".into(),
            diff: Some(diff.into()),
            test_analysis: Some(test_analysis),
        })
        .expect("workflow");

    let kinds = report
        .steps
        .iter()
        .map(|step| step.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            CodingWorkflowStepKind::ReadRepo,
            CodingWorkflowStepKind::Plan,
            CodingWorkflowStepKind::Patch,
            CodingWorkflowStepKind::Test,
            CodingWorkflowStepKind::Review,
            CodingWorkflowStepKind::FinalReport,
        ]
    );
    assert_eq!(report.steps[2].status, CodingWorkflowStepStatus::Completed);
    assert_eq!(report.steps[3].status, CodingWorkflowStepStatus::Completed);
    assert!(report.requires_guarded_edit);
    assert!(!report.ready_for_approval);
    assert!(
        report
            .suggested_tests
            .iter()
            .any(|command| command.command.contains("cargo test"))
    );
    let workflow_json = serde_json::to_string(&report).expect("workflow json");
    assert!(workflow_json.contains("[REDACTED_SECRET]"));
    assert!(!workflow_json.contains("abc123"));
    assert!(report.final_report.contains("Final Report"));
}

#[test]
fn coding_workflow_without_diff_blocks_patch_step_without_mutating_repo() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("Cargo.toml"), "[workspace]").expect("cargo");
    fs::create_dir_all(temp.path().join("src")).expect("src");
    let source = temp.path().join("src/lib.rs");
    fs::write(&source, "pub fn unchanged() {}\n").expect("lib");

    let report = CodingWorkflow::new(temp.path())
        .run(CodingWorkflowInput {
            objective: "plan a safe change".into(),
            diff: None,
            test_analysis: None,
        })
        .expect("workflow");

    assert_eq!(report.steps[2].kind, CodingWorkflowStepKind::Patch);
    assert_eq!(report.steps[2].status, CodingWorkflowStepStatus::Blocked);
    assert_eq!(report.steps[3].status, CodingWorkflowStepStatus::Planned);
    assert!(
        report
            .review
            .findings
            .iter()
            .any(|finding| finding.title == "No diff detected")
    );
    assert_eq!(
        fs::read_to_string(source).expect("source"),
        "pub fn unchanged() {}\n"
    );
}
