use super::*;

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
