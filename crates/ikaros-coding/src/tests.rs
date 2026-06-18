// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use ikaros_core::{IkarosError, SelfModifyCheckProfileConfig, SelfModifyConfig};
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

#[derive(Debug)]
struct FailingRemoveFileSystem {
    fail_path: PathBuf,
    failures_left: Arc<AtomicUsize>,
}

impl FailingRemoveFileSystem {
    fn new(fail_path: PathBuf) -> Self {
        Self {
            fail_path,
            failures_left: Arc::new(AtomicUsize::new(1)),
        }
    }
}

impl ExecutionFileSystem for FailingRemoveFileSystem {
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
        LocalExecutionEnv.read_to_string(path)
    }

    fn read_bytes<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<Vec<u8>>> + Send + 'a>> {
        LocalExecutionEnv.read_bytes(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<()>> + Send + 'a>> {
        LocalExecutionEnv.write_string(path, content)
    }

    fn write_bytes<'a>(
        &'a self,
        path: &'a Path,
        content: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = ikaros_core::Result<()>> + Send + 'a>> {
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
        Box::pin(async move {
            if path == self.fail_path
                && self
                    .failures_left
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                        (remaining > 0).then_some(remaining - 1)
                    })
                    .is_ok()
            {
                return Err(IkarosError::Message("forced remove failure".into()));
            }
            LocalExecutionEnv.remove_file(path).await
        })
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
fn coding_turn_context_records_workspace_git_mode_and_redacts_secret() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join(".git/refs/heads")).expect("git refs");
    fs::write(temp.path().join(".git/HEAD"), "ref: refs/heads/main\n").expect("head");
    fs::write(
        temp.path().join(".git/refs/heads/main"),
        "0123456789abcdef0123456789abcdef01234567\n",
    )
    .expect("ref");
    fs::write(temp.path().join(".git/status_porcelain_v1"), "").expect("status");
    fs::write(temp.path().join("Cargo.toml"), "[workspace]\n").expect("cargo");

    let context = CodingTurnContext::from_workspace(CodingTurnContextInput {
        workspace_root: temp.path().to_path_buf(),
        objective: "fix token=abc123 without leaking it".into(),
        mode: CodingMode::Edit,
        session_id: Some("session-1".into()),
        turn_id: Some("turn-1".into()),
        instructions: vec!["keep the runtime event-first".into()],
        permission_profile: CodingPermissionProfile::default(),
        test_commands: vec![TestCommand {
            command: "cargo test --workspace".into(),
            reason: "workspace regression check".into(),
        }],
    })
    .expect("context");

    assert_eq!(context.mode, CodingMode::Edit);
    assert_eq!(context.session_id.as_deref(), Some("session-1"));
    assert_eq!(context.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(context.workspace_root, temp.path());
    assert_eq!(context.git.git_root.as_deref(), Some(temp.path()));
    assert_eq!(
        context.git.head.as_deref(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );
    assert_eq!(context.git.branch.as_deref(), Some("main"));
    assert!(!context.git.detached);
    assert_eq!(context.git.dirty, CodingDirtyState::Clean);
    assert!(!context.git.has_staged_changes);
    assert!(!context.git.has_unstaged_changes);
    assert!(!context.git.has_untracked_files);
    assert!(context.objective.contains("[REDACTED_SECRET]"));
    assert!(
        !serde_json::to_string(&context)
            .expect("json")
            .contains("abc123")
    );
}

#[test]
fn coding_turn_context_classifies_git_dirty_state_without_shelling_out() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join(".git/refs/heads")).expect("git refs");
    fs::write(temp.path().join(".git/HEAD"), "ref: refs/heads/main\n").expect("head");
    fs::write(
        temp.path().join(".git/refs/heads/main"),
        "1111111111111111111111111111111111111111\n",
    )
    .expect("ref");
    fs::write(temp.path().join(".git/index"), "pretend index").expect("index");
    fs::write(temp.path().join("tracked.rs"), "old\n").expect("tracked");
    fs::write(temp.path().join("new.rs"), "new\n").expect("untracked");
    fs::create_dir_all(temp.path().join(".git/refs")).expect("refs");
    fs::write(
        temp.path().join(".git/status_porcelain_v1"),
        " M tracked.rs\nA  staged.rs\n?? new.rs\n",
    )
    .expect("status");

    let context = CodingTurnContext::from_workspace(CodingTurnContextInput {
        workspace_root: temp.path().to_path_buf(),
        objective: "classify dirty state".into(),
        mode: CodingMode::Edit,
        session_id: None,
        turn_id: None,
        instructions: Vec::new(),
        permission_profile: CodingPermissionProfile::default(),
        test_commands: Vec::new(),
    })
    .expect("context");

    assert_eq!(context.git.dirty, CodingDirtyState::Dirty);
    assert_eq!(context.git.branch.as_deref(), Some("main"));
    assert!(!context.git.detached);
    assert!(context.git.has_staged_changes);
    assert!(context.git.has_unstaged_changes);
    assert!(context.git.has_untracked_files);
}

#[test]
fn coding_turn_context_marks_detached_and_non_git_workspaces() {
    let detached = tempfile::tempdir().expect("detached tempdir");
    fs::create_dir_all(detached.path().join(".git")).expect("git");
    fs::write(
        detached.path().join(".git/HEAD"),
        "2222222222222222222222222222222222222222\n",
    )
    .expect("head");
    fs::write(detached.path().join(".git/status_porcelain_v1"), "").expect("status");
    let detached_context = CodingTurnContext::from_workspace(CodingTurnContextInput {
        workspace_root: detached.path().to_path_buf(),
        objective: "detached".into(),
        mode: CodingMode::Plan,
        session_id: None,
        turn_id: None,
        instructions: Vec::new(),
        permission_profile: CodingPermissionProfile::default(),
        test_commands: Vec::new(),
    })
    .expect("detached context");

    assert!(detached_context.git.detached);
    assert_eq!(detached_context.git.branch, None);
    assert_eq!(detached_context.git.dirty, CodingDirtyState::Clean);

    let non_git = tempfile::tempdir().expect("non git tempdir");
    let non_git_context = CodingTurnContext::from_workspace(CodingTurnContextInput {
        workspace_root: non_git.path().to_path_buf(),
        objective: "non git".into(),
        mode: CodingMode::Plan,
        session_id: None,
        turn_id: None,
        instructions: Vec::new(),
        permission_profile: CodingPermissionProfile::default(),
        test_commands: Vec::new(),
    })
    .expect("non git context");

    assert_eq!(non_git_context.git.git_root, None);
    assert_eq!(non_git_context.git.dirty, CodingDirtyState::NotGit);
}

#[test]
fn coding_mode_capabilities_define_allowed_tools_per_mode() {
    let plan = CodingModeCapabilities::for_mode(CodingMode::Plan);
    assert!(plan.can_read_repo);
    assert!(!plan.can_apply_patch);
    assert!(!plan.can_run_tests);
    assert!(!plan.can_self_modify);
    assert!(plan.validate_request(false, false).is_ok());
    assert!(plan.validate_request(true, false).is_err());

    let review = CodingModeCapabilities::for_mode(CodingMode::Review);
    assert!(review.can_read_repo);
    assert!(!review.can_apply_patch);
    assert!(!review.can_run_tests);

    let test = CodingModeCapabilities::for_mode(CodingMode::Test);
    assert!(test.can_read_repo);
    assert!(!test.can_apply_patch);
    assert!(test.can_run_tests);
    assert!(test.validate_request(false, true).is_ok());
    assert!(test.validate_request(true, true).is_err());

    let edit = CodingModeCapabilities::for_mode(CodingMode::Edit);
    assert!(edit.can_apply_patch);
    assert!(edit.can_run_tests);
    assert!(edit.validate_request(true, true).is_ok());

    let self_modify = CodingModeCapabilities::for_mode(CodingMode::SelfModify);
    assert!(self_modify.can_self_modify);
    assert!(self_modify.requires_self_modify_boundary);
    assert!(self_modify.validate_request(true, false).is_err());
}

#[test]
fn guarded_patch_delete_and_move_feed_turn_diff_tracker() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("remove.txt"), "gone\n").expect("remove");
    fs::write(temp.path().join("old.txt"), "old\n").expect("old");
    let diff = "\
diff --git a/remove.txt b/remove.txt
--- a/remove.txt
+++ /dev/null
@@ -1 +0,0 @@
-gone
diff --git a/old.txt b/new.txt
rename from old.txt
rename to new.txt
--- a/old.txt
+++ b/new.txt
@@ -1 +1 @@
-old
+new
";

    let report = GuardedPatchApplier::apply_unified_diff(temp.path(), diff).expect("apply");

    assert_eq!(report.files_changed, 2);
    assert_eq!(report.files_deleted, 1);
    assert_eq!(report.files_moved, 1);
    assert!(!temp.path().join("remove.txt").exists());
    assert!(!temp.path().join("old.txt").exists());
    assert_eq!(
        fs::read_to_string(temp.path().join("new.txt")).expect("new"),
        "new\n"
    );

    let mut tracker = TurnDiffTracker::new(temp.path());
    tracker.track_patch_report(&report).expect("track");
    let unified = tracker.unified_diff().expect("diff");
    assert!(unified.contains("diff --git a/remove.txt b/remove.txt"));
    assert!(unified.contains("--- a/remove.txt"));
    assert!(unified.contains("+++ /dev/null"));
    assert!(unified.contains("diff --git a/old.txt b/new.txt"));
    assert!(unified.contains("rename from old.txt"));
    assert!(unified.contains("rename to new.txt"));
}

#[tokio::test]
async fn guarded_patch_move_rolls_back_written_target_when_source_remove_fails() {
    let temp = tempfile::tempdir().expect("tempdir");
    let old_path = temp.path().join("old.txt");
    let new_path = temp.path().join("new.txt");
    fs::write(&old_path, "old\n").expect("old");
    let file_system = FailingRemoveFileSystem::new(old_path.clone());
    let diff = "\
diff --git a/old.txt b/new.txt
rename from old.txt
rename to new.txt
--- a/old.txt
+++ b/new.txt
@@ -1 +1 @@
-old
+new
";

    let failure =
        GuardedPatchApplier::apply_unified_diff_with_env_checked(temp.path(), diff, &file_system)
            .await
            .expect_err("forced remove failure");

    assert_eq!(failure.kind, PatchFailureKind::Unsupported);
    assert_eq!(
        fs::read_to_string(&old_path).expect("old restored"),
        "old\n"
    );
    assert!(
        !new_path.exists(),
        "target written before failed source remove must be rolled back"
    );
}

#[test]
fn guarded_patch_reports_structured_context_drift() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("lib.rs"), "actual\n").expect("lib");
    let diff = "\
diff --git a/lib.rs b/lib.rs
--- a/lib.rs
+++ b/lib.rs
@@ -1 +1 @@
-expected
+updated
";

    let failure =
        GuardedPatchApplier::apply_unified_diff_checked(temp.path(), diff).expect_err("drift");

    assert_eq!(failure.kind, PatchFailureKind::HunkMismatch);
    assert_eq!(failure.path.as_deref(), Some(Path::new("lib.rs")));
    assert_eq!(failure.source_line, Some(1));
    assert_eq!(failure.expected.as_deref(), Some("expected"));
    assert_eq!(failure.actual.as_deref(), Some("actual"));
    assert_eq!(
        fs::read_to_string(temp.path().join("lib.rs")).expect("lib"),
        "actual\n"
    );
}

#[test]
fn guarded_patch_reports_already_applied_drift() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("lib.rs"), "new\n").expect("lib");
    let diff = "\
diff --git a/lib.rs b/lib.rs
--- a/lib.rs
+++ b/lib.rs
@@ -1 +1 @@
-old
+new
";

    let failure = GuardedPatchApplier::apply_unified_diff_checked(temp.path(), diff)
        .expect_err("already applied");

    assert_eq!(failure.kind, PatchFailureKind::AlreadyApplied);
    assert!(failure.already_applied);
    assert_eq!(failure.path.as_deref(), Some(Path::new("lib.rs")));
}

#[test]
fn guarded_patch_reports_ambiguous_anchor_candidates() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("lib.rs"), "same\nkeep\nsame\nkeep\n").expect("lib");
    let diff = "\
diff --git a/lib.rs b/lib.rs
--- a/lib.rs
+++ b/lib.rs
@@ -1,2 +1,2 @@
-same
-keep
+changed
+keep
";

    let failure = GuardedPatchApplier::apply_unified_diff_checked(temp.path(), diff)
        .expect_err("ambiguous patch anchor");

    assert_eq!(failure.kind, PatchFailureKind::AmbiguousAnchor);
    assert_eq!(failure.candidate_lines.as_ref(), &[1, 3]);
    assert!(failure.message.contains("ambiguous hunk anchor in lib.rs"));
}

#[test]
fn guarded_patch_malformed_diff_does_not_mutate_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("lib.rs"), "old\n").expect("lib");
    let malformed = [
        "diff --git a/lib.rs b/lib.rs\n--- a/lib.rs\n+++ b/lib.rs\n@@ nope @@\n-old\n+new\n",
        "diff --git a/lib.rs b/lib.rs\n--- a/lib.rs\n+++ b/lib.rs\n@@ -1 +1 @@\n*old\n+new\n",
        "diff --git a/lib.rs b/lib.rs\n--- a/lib.rs\n+++ b/lib.rs\n@@ -3 +3 @@\n-old\n+new\n",
    ];

    for diff in malformed {
        let failure =
            GuardedPatchApplier::apply_unified_diff_checked(temp.path(), diff).expect_err("fail");
        assert!(
            matches!(
                failure.kind,
                PatchFailureKind::Unsupported | PatchFailureKind::HunkOutOfRange
            ),
            "unexpected failure kind: {:?}",
            failure.kind
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("lib.rs")).expect("lib"),
            "old\n"
        );
    }
}

#[test]
fn guarded_patch_rejects_malformed_hunk_new_range_without_mutating() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("lib.rs"), "old\n").expect("lib");
    let diff = "\
diff --git a/lib.rs b/lib.rs
--- a/lib.rs
+++ b/lib.rs
@@ -1 +abc @@
-old
+new
";

    let failure =
        GuardedPatchApplier::apply_unified_diff_checked(temp.path(), diff).expect_err("bad range");

    assert_eq!(failure.kind, PatchFailureKind::Unsupported);
    assert_eq!(
        fs::read_to_string(temp.path().join("lib.rs")).expect("lib"),
        "old\n"
    );
}

#[test]
fn guarded_patch_rejects_quoted_or_space_paths_without_truncation() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("file name.txt"), "old\n").expect("file");
    let diffs = [
        "diff --git a/file name.txt b/file name.txt\n--- a/file name.txt\n+++ b/file name.txt\n@@ -1 +1 @@\n-old\n+new\n",
        "diff --git \"a/file name.txt\" \"b/file name.txt\"\n--- \"a/file name.txt\"\n+++ \"b/file name.txt\"\n@@ -1 +1 @@\n-old\n+new\n",
    ];

    for diff in diffs {
        let failure = GuardedPatchApplier::apply_unified_diff_checked(temp.path(), diff)
            .expect_err("ambiguous quoted/space path");
        assert_eq!(failure.kind, PatchFailureKind::PathRejected);
        assert_eq!(
            fs::read_to_string(temp.path().join("file name.txt")).expect("file"),
            "old\n"
        );
        assert!(!temp.path().join("file").exists());
        assert!(!temp.path().join("\"a/file").exists());
    }
}

#[test]
fn guarded_patch_generated_line_update_roundtrips_without_context_loss() {
    let temp = tempfile::tempdir().expect("tempdir");
    for seed in 0..32usize {
        let file_name = format!("case_{seed}.txt");
        let old = format!(
            "prefix-{seed}\nanchor-{seed}\nsuffix-{}\n",
            seed.wrapping_mul(17)
        );
        let new_anchor = format!("anchor-{}-updated", seed.wrapping_mul(31));
        fs::write(temp.path().join(&file_name), &old).expect("case file");
        let diff = format!(
            "diff --git a/{file_name} b/{file_name}\n--- a/{file_name}\n+++ b/{file_name}\n@@ -1,3 +1,3 @@\n prefix-{seed}\n-anchor-{seed}\n+{new_anchor}\n suffix-{}\n",
            seed.wrapping_mul(17)
        );

        let report = GuardedPatchApplier::apply_unified_diff_checked(temp.path(), &diff)
            .expect("generated patch applies");

        assert_eq!(report.files_changed, 1);
        assert_eq!(
            fs::read_to_string(temp.path().join(&file_name)).expect("updated"),
            format!(
                "prefix-{seed}\n{new_anchor}\nsuffix-{}\n",
                seed.wrapping_mul(17)
            )
        );
    }
}

#[test]
fn coding_runtime_applies_patch_tracks_diff_reviews_and_reports_events() {
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
        .run_turn(CodingTurnInput {
            context,
            candidate_diff: Some(diff.into()),
            apply_patch: true,
            test_matrix: Vec::new(),
            test_analysis: Some(test_analysis),
        })
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

#[test]
fn coding_runtime_emits_review_started_and_finding_events() {
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
        .run_turn(CodingTurnInput {
            context,
            candidate_diff: Some(diff.into()),
            apply_patch: false,
            test_matrix: Vec::new(),
            test_analysis: None,
        })
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

#[test]
fn coding_runtime_plan_mode_does_not_apply_candidate_patch() {
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
        .run_turn(CodingTurnInput {
            context,
            candidate_diff: Some(diff.into()),
            apply_patch: true,
            test_matrix: Vec::new(),
            test_analysis: None,
        })
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

#[test]
fn coding_runtime_marks_loop_passed_when_patch_and_tests_pass() {
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
        .run_turn(CodingTurnInput {
            context,
            candidate_diff: Some(diff.into()),
            apply_patch: true,
            test_matrix: vec![test_analysis],
            test_analysis: None,
        })
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

#[test]
fn coding_runtime_marks_loop_waiting_for_followup_patch_after_test_failure() {
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
        .run_turn(CodingTurnInput {
            context,
            candidate_diff: Some(diff.into()),
            apply_patch: true,
            test_matrix: vec![test_analysis],
            test_analysis: None,
        })
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

#[test]
fn mock_model_coding_runtime_applies_followup_patch_until_tests_pass() {
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
        .run_scripted_turns(MockModelCodingInput {
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
        })
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
