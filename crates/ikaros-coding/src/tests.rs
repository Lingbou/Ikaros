// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use ikaros_core::{SelfModifyCheckProfileConfig, SelfModifyConfig};
use ikaros_harness::{
    FileMetadata, FileSystem as ExecutionFileSystem, LocalExecutionEnv, ProcessOutput,
    ProcessRequest, ProcessRunner,
};
use std::collections::BTreeMap;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::{
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Debug, Default)]
struct SelfModifyTrackingEnv {
    read_to_string_calls: Arc<AtomicUsize>,
    read_bytes_calls: Arc<AtomicUsize>,
    write_string_calls: Arc<AtomicUsize>,
    write_bytes_calls: Arc<AtomicUsize>,
    remove_file_calls: Arc<AtomicUsize>,
    process_calls: Arc<AtomicUsize>,
}

impl SelfModifyTrackingEnv {
    fn read_bytes_count(&self) -> usize {
        self.read_bytes_calls.load(Ordering::SeqCst)
    }

    fn write_string_count(&self) -> usize {
        self.write_string_calls.load(Ordering::SeqCst)
    }

    fn write_bytes_count(&self) -> usize {
        self.write_bytes_calls.load(Ordering::SeqCst)
    }

    fn remove_file_count(&self) -> usize {
        self.remove_file_calls.load(Ordering::SeqCst)
    }

    fn process_count(&self) -> usize {
        self.process_calls.load(Ordering::SeqCst)
    }
}

impl ExecutionFileSystem for SelfModifyTrackingEnv {
    fn path_metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<FileMetadata>> + Send + 'a>> {
        LocalExecutionEnv.path_metadata(path)
    }

    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<String>> + Send + 'a>> {
        self.read_to_string_calls.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.read_to_string(path)
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<Vec<u8>>> + Send + 'a>> {
        self.read_bytes_calls.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.read_bytes(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<()>> + Send + 'a>> {
        self.write_string_calls.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.write_string(path, content)
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<()>> + Send + 'a>> {
        self.write_bytes_calls.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.write_bytes(path, content)
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<()>> + Send + 'a>> {
        LocalExecutionEnv.create_dir_all(path)
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<Vec<String>>> + Send + 'a>> {
        LocalExecutionEnv.read_dir(path)
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<()>> + Send + 'a>> {
        self.remove_file_calls.fetch_add(1, Ordering::SeqCst);
        LocalExecutionEnv.remove_file(path)
    }
}

impl ProcessRunner for SelfModifyTrackingEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<ProcessOutput>> + Send + 'a>> {
        self.process_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            assert_eq!(request.command, "cargo");
            assert_eq!(request.args, vec!["check"]);
            Ok(ProcessOutput {
                status: 0,
                stdout: "checked through execution env".into(),
                stderr: String::new(),
            })
        })
    }
}

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

#[test]
fn guarded_patch_applier_modifies_existing_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("note.txt");
    fs::write(&path, "old\nkeep\n").expect("write");
    let diff = "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n-old\n+new\n keep\n";
    let report = GuardedPatchApplier::apply_unified_diff(temp.path(), diff).expect("apply");
    assert_eq!(report.files_changed, 1);
    assert_eq!(report.insertions, 1);
    assert_eq!(report.deletions, 1);
    assert_eq!(fs::read_to_string(path).expect("read"), "new\nkeep\n");
}

#[test]
fn guarded_patch_applier_creates_file_and_rejects_temp() {
    let temp = tempfile::tempdir().expect("tempdir");
    let diff = "diff --git a/new.txt b/new.txt\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+one\n+two\n";
    let report = GuardedPatchApplier::apply_unified_diff(temp.path(), diff).expect("apply");
    assert_eq!(report.files_created, 1);
    assert_eq!(
        fs::read_to_string(temp.path().join("new.txt")).expect("read"),
        "one\ntwo\n"
    );

    let denied = "diff --git a/.temp/secret.txt b/.temp/secret.txt\n--- /dev/null\n+++ b/.temp/secret.txt\n@@ -0,0 +1 @@\n+secret\n";
    let error = GuardedPatchApplier::apply_unified_diff(temp.path(), denied).expect_err("denied");
    assert!(error.to_string().contains(".temp"));
}

#[cfg(unix)]
#[test]
fn guarded_patch_applier_rejects_symlink_targets() {
    let temp = tempfile::tempdir().expect("tempdir");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).expect("outside dir");
    fs::write(outside.join("note.txt"), "old\n").expect("outside file");
    symlink(outside.join("note.txt"), temp.path().join("note.txt")).expect("file symlink");
    let diff = "diff --git a/note.txt b/note.txt\n--- a/note.txt\n+++ b/note.txt\n@@ -1 +1 @@\n-old\n+new\n";

    let error =
        GuardedPatchApplier::apply_unified_diff(temp.path(), diff).expect_err("symlink rejected");

    assert!(error.to_string().contains("symlink"));
    assert_eq!(
        fs::read_to_string(outside.join("note.txt")).expect("outside file"),
        "old\n"
    );
}

#[cfg(unix)]
#[test]
fn guarded_patch_applier_rejects_symlink_parent_directories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).expect("outside dir");
    symlink(&outside, temp.path().join("linked")).expect("dir symlink");
    let diff = "diff --git a/linked/new.txt b/linked/new.txt\n--- /dev/null\n+++ b/linked/new.txt\n@@ -0,0 +1 @@\n+new\n";

    let error = GuardedPatchApplier::apply_unified_diff(temp.path(), diff)
        .expect_err("symlink parent rejected");

    assert!(error.to_string().contains("symlink"));
    assert!(!outside.join("new.txt").exists());
}

#[test]
fn guarded_patch_applier_does_not_partially_apply_stale_multifile_diff() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first = temp.path().join("first.txt");
    let second = temp.path().join("second.txt");
    fs::write(&first, "old\n").expect("first");
    fs::write(&second, "current\n").expect("second");
    let diff = "\
diff --git a/first.txt b/first.txt
--- a/first.txt
+++ b/first.txt
@@ -1 +1 @@
-old
+new
diff --git a/second.txt b/second.txt
--- a/second.txt
+++ b/second.txt
@@ -1 +1 @@
-stale
+updated
";

    let error = GuardedPatchApplier::apply_unified_diff(temp.path(), diff).expect_err("stale hunk");

    assert!(error.to_string().contains("hunk mismatch"));
    assert_eq!(fs::read_to_string(first).expect("first unchanged"), "old\n");
    assert_eq!(
        fs::read_to_string(second).expect("second unchanged"),
        "current\n"
    );
}

#[test]
fn self_modify_proposal_stores_dry_run_and_rollback_snapshot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let store_dir = temp.path().join("self-modify");
    fs::create_dir_all(workspace.join("src")).expect("workspace");
    fs::write(workspace.join("src/lib.rs"), "pub fn old() {}\n").expect("source");
    let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-pub fn old() {}\n+pub fn new() {}\n";
    let store = SelfModifyStore::new(&workspace, &store_dir);

    let proposal = store
        .propose(
            SelfModifyChangeKind::RuntimePatch,
            "src/lib.rs",
            diff,
            Some("task-1".into()),
        )
        .expect("proposal");

    assert!(!proposal.dry_run_report.enabled);
    assert!(!proposal.dry_run_report.apply_available);
    assert!(proposal.dry_run_report.manual_apply_available);
    assert!(proposal.dry_run_report.ok_to_request_approval);
    assert_eq!(proposal.dry_run_report.diff_summary.files_changed, 1);
    assert!(proposal.rollback_plan.snapshot_path.exists());
    assert_eq!(
        fs::read_to_string(&proposal.rollback_plan.snapshot_path).expect("snapshot"),
        "pub fn old() {}\n"
    );
    assert_eq!(store.list().expect("list").len(), 1);
}

#[cfg(unix)]
#[test]
fn self_modify_rejects_symlink_targets_before_snapshot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    let store_dir = temp.path().join("self-modify");
    fs::create_dir_all(workspace.join("src")).expect("workspace");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(outside.join("lib.rs"), "pub fn outside() {}\n").expect("outside source");
    symlink(outside.join("lib.rs"), workspace.join("src/lib.rs")).expect("file symlink");
    let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-pub fn outside() {}\n+pub fn new() {}\n";
    let store = SelfModifyStore::new(&workspace, &store_dir);

    let error = store
        .propose(SelfModifyChangeKind::RuntimePatch, "src/lib.rs", diff, None)
        .expect_err("symlink rejected");

    assert!(error.to_string().contains("symlink"));
    assert!(!store.proposal_path().exists());
    assert_eq!(
        fs::read_to_string(outside.join("lib.rs")).expect("outside unchanged"),
        "pub fn outside() {}\n"
    );
}

#[test]
fn self_modify_apply_and_rollback_use_snapshot_gate() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let store_dir = temp.path().join("self-modify");
    fs::create_dir_all(workspace.join("src")).expect("workspace");
    fs::write(workspace.join("src/lib.rs"), "pub fn old() {}\n").expect("source");
    let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-pub fn old() {}\n+pub fn new() {}\n";
    let store = SelfModifyStore::new(&workspace, &store_dir);
    let proposal = store
        .propose(SelfModifyChangeKind::RuntimePatch, "src/lib.rs", diff, None)
        .expect("proposal");

    let report = store
        .apply_approved(&proposal.id, "approval-1")
        .expect("apply");

    assert!(!report.operation_id.is_empty());
    assert_eq!(report.patch_report.files_changed, 1);
    assert!(report.post_checks_passed);
    assert_eq!(report.check_profile.source, "default");
    assert!(report.check_profile.commands.is_empty());
    assert!(report.auto_rollback.is_none());
    assert_eq!(
        fs::read_to_string(workspace.join("src/lib.rs")).expect("updated"),
        "pub fn new() {}\n"
    );

    let rollback = store.rollback(&proposal.id).expect("rollback");
    assert!(!rollback.operation_id.is_empty());
    assert!(rollback.restored_snapshot);
    assert_eq!(
        fs::read_to_string(workspace.join("src/lib.rs")).expect("restored"),
        "pub fn old() {}\n"
    );
    let operations = store.operations().expect("operations");
    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0].kind, SelfModifyOperationKind::Apply);
    assert_eq!(operations[0].proposal_id, proposal.id);
    assert_eq!(operations[0].post_checks_passed, Some(true));
    assert_eq!(operations[1].kind, SelfModifyOperationKind::Rollback);
}

#[tokio::test]
async fn self_modify_propose_apply_and_rollback_use_execution_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let store_dir = temp.path().join("self-modify");
    fs::create_dir_all(workspace.join("src")).expect("workspace");
    fs::write(workspace.join("src/lib.rs"), "pub fn old() {}\n").expect("source");
    let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-pub fn old() {}\n+pub fn new() {}\n";
    let store = SelfModifyStore::new(&workspace, &store_dir);
    let env = SelfModifyTrackingEnv::default();

    let proposal = store
        .propose_with_env(
            SelfModifyChangeKind::RuntimePatch,
            "src/lib.rs",
            diff,
            None,
            &env,
        )
        .await
        .expect("proposal");
    assert!(env.read_bytes_count() > 0);

    let report = store
        .apply_approved_with_checks_and_env(
            &proposal.id,
            "approval-1",
            &["cargo check".into()],
            &env,
            &env,
        )
        .await
        .expect("apply through env");

    assert!(report.post_checks_passed);
    assert_eq!(env.process_count(), 2);
    assert!(env.write_string_count() > 0);
    assert_eq!(
        fs::read_to_string(workspace.join("src/lib.rs")).expect("updated"),
        "pub fn new() {}\n"
    );

    let rollback = store
        .rollback_with_env(&proposal.id, &env)
        .await
        .expect("rollback through env");

    assert!(rollback.restored_snapshot);
    assert!(env.write_bytes_count() > 0);
    assert_eq!(env.remove_file_count(), 0);
    assert_eq!(
        fs::read_to_string(workspace.join("src/lib.rs")).expect("restored"),
        "pub fn old() {}\n"
    );
}

#[test]
fn self_modify_default_runtime_profile_runs_cargo_check() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let store_dir = temp.path().join("self-modify");
    fs::create_dir_all(workspace.join("src")).expect("workspace");
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname = \"self-modify-default-check\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("cargo toml");
    fs::write(
        workspace.join("src/lib.rs"),
        "pub fn value() -> i32 { 1 }\n",
    )
    .expect("source");
    let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-pub fn value() -> i32 { 1 }\n+pub fn value() -> i32 { 2 }\n";
    let store = SelfModifyStore::new(&workspace, &store_dir);
    let profile = store.default_check_profile(&SelfModifyChangeKind::RuntimePatch);
    assert_eq!(profile.source, "default");
    assert_eq!(
        profile.commands,
        vec!["cargo check --workspace --all-features"]
    );
    let proposal = store
        .propose(SelfModifyChangeKind::RuntimePatch, "src/lib.rs", diff, None)
        .expect("proposal");

    let report = store
        .apply_approved(&proposal.id, "approval-1")
        .expect("apply");

    assert_eq!(report.check_profile.commands, profile.commands);
    assert_eq!(report.pre_checks.len(), 1);
    assert_eq!(report.post_checks.len(), 1);
    assert!(report.pre_checks[0].passed);
    assert!(report.post_checks[0].passed);
}

#[test]
fn self_modify_configured_check_profile_overrides_default_and_records_operation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let store_dir = temp.path().join("self-modify");
    fs::create_dir_all(workspace.join("src")).expect("workspace");
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname = \"self-modify-config-check\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("cargo toml");
    fs::write(
        workspace.join("src/lib.rs"),
        "pub fn value() -> i32 { 1 }\n",
    )
    .expect("source");
    let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-pub fn value() -> i32 { 1 }\n+pub fn value() -> i32 { 2 }\n";
    let store = SelfModifyStore::new(&workspace, &store_dir);
    let proposal = store
        .propose(SelfModifyChangeKind::RuntimePatch, "src/lib.rs", diff, None)
        .expect("proposal");
    let mut check_profiles = BTreeMap::new();
    check_profiles.insert(
        "runtime_patch".into(),
        SelfModifyCheckProfileConfig {
            commands: vec!["cargo check".into()],
            reason: Some("Configured runtime checks stay narrow in this test.".into()),
        },
    );
    let config = SelfModifyConfig { check_profiles };

    let report = store
        .apply_approved_with_config(&proposal.id, "approval-1", &config)
        .expect("apply");

    assert_eq!(report.check_profile.source, "config:runtime_patch");
    assert_eq!(report.check_profile.commands, vec!["cargo check"]);
    assert_eq!(
        report.check_profile.reason,
        "Configured runtime checks stay narrow in this test."
    );
    assert_eq!(report.pre_checks.len(), 1);
    assert_eq!(report.post_checks.len(), 1);
    let operations = store.operations().expect("operations");
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].id, report.operation_id);
    assert_eq!(operations[0].kind, SelfModifyOperationKind::Apply);
    assert_eq!(
        operations[0]
            .check_profile
            .as_ref()
            .expect("check profile")
            .source,
        "config:runtime_patch"
    );
}

#[test]
fn self_modify_post_check_failure_auto_rolls_back() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let store_dir = temp.path().join("self-modify");
    fs::create_dir_all(workspace.join("src")).expect("workspace");
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname = \"self-modify-check-smoke\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("cargo toml");
    fs::write(
        workspace.join("src/lib.rs"),
        "pub fn value() -> i32 { 1 }\n",
    )
    .expect("source");
    let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-pub fn value() -> i32 { 1 }\n+pub fn value() -> i32 { \"bad\" }\n";
    let store = SelfModifyStore::new(&workspace, &store_dir);
    let proposal = store
        .propose(SelfModifyChangeKind::RuntimePatch, "src/lib.rs", diff, None)
        .expect("proposal");

    let report = store
        .apply_approved_with_checks(&proposal.id, "approval-1", &["cargo check".into()])
        .expect("apply with rollback");

    assert!(!report.post_checks_passed);
    assert_eq!(report.check_profile.source, "override");
    assert!(report.auto_rollback.is_some());
    let auto_rollback = report.auto_rollback.as_ref().expect("auto rollback");
    assert_eq!(auto_rollback.proposal_id, proposal.id);
    assert_eq!(
        fs::read_to_string(workspace.join("src/lib.rs")).expect("restored"),
        "pub fn value() -> i32 { 1 }\n"
    );
    assert!(
        report
            .post_checks
            .iter()
            .any(|check| !check.passed && check.analysis.summary.contains("compile"))
    );
    let operations = store.operations().expect("operations");
    assert!(operations.iter().any(|operation| {
        operation.kind == SelfModifyOperationKind::AutoRollback
            && operation.id == auto_rollback.operation_id
    }));
    assert!(operations.iter().any(|operation| {
        operation.kind == SelfModifyOperationKind::Apply
            && operation.auto_rollback_operation_id.as_deref()
                == Some(auto_rollback.operation_id.as_str())
    }));
}

#[test]
fn self_modify_apply_rejects_target_drift() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let store_dir = temp.path().join("self-modify");
    fs::create_dir_all(workspace.join("src")).expect("workspace");
    fs::write(workspace.join("src/lib.rs"), "pub fn old() {}\n").expect("source");
    let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-pub fn old() {}\n+pub fn new() {}\n";
    let store = SelfModifyStore::new(&workspace, &store_dir);
    let proposal = store
        .propose(SelfModifyChangeKind::RuntimePatch, "src/lib.rs", diff, None)
        .expect("proposal");
    fs::write(workspace.join("src/lib.rs"), "pub fn drifted() {}\n").expect("drift");

    let error = store
        .apply_approved(&proposal.id, "approval-1")
        .expect_err("drift rejected");

    assert!(error.to_string().contains("changed since proposal"));
}

#[test]
fn self_modify_proposal_blocks_temp_and_secret_like_diff() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let store_dir = temp.path().join("self-modify");
    fs::create_dir_all(workspace.join("src")).expect("workspace");
    let store = SelfModifyStore::new(&workspace, &store_dir);

    let denied = store
        .propose(
            SelfModifyChangeKind::RuntimePatch,
            ".temp/hidden.rs",
            "diff --git a/.temp/hidden.rs b/.temp/hidden.rs\n",
            None,
        )
        .expect_err("temp denied");
    assert!(denied.to_string().contains(".temp"));

    fs::write(workspace.join("src/lib.rs"), "pub fn old() {}\n").expect("source");
    let secret_diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1,2 @@\n pub fn old() {}\n+const TOKEN: &str = \"token=abc123\";\n";
    let proposal = store
        .propose(
            SelfModifyChangeKind::RuntimePatch,
            "src/lib.rs",
            secret_diff,
            None,
        )
        .expect("proposal");

    assert!(!proposal.dry_run_report.ok_to_request_approval);
    assert!(proposal.unified_diff.contains("[REDACTED_SECRET]"));
    assert!(!proposal.unified_diff.contains("abc123"));
}

#[test]
fn self_modify_heartbeat_reports_proposal_only_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let store_dir = temp.path().join("self-modify");
    fs::create_dir_all(&workspace).expect("workspace");
    let store = SelfModifyStore::new(&workspace, &store_dir);

    let heartbeat = store.heartbeat().expect("heartbeat");

    assert_eq!(heartbeat.status, "manual_apply_only");
    assert_eq!(heartbeat.proposal_count, 0);
    assert!(heartbeat.proposal_store.ends_with("proposals.jsonl"));
    assert!(
        heartbeat
            .checks
            .iter()
            .any(|check| check.contains("autonomous self-modify apply path disabled"))
    );
}
