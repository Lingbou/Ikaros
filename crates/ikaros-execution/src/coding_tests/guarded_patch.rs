use super::*;

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
fn guarded_patch_parser_rejects_generated_malformed_corpus_without_panic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = temp.path().join("lib.rs");
    let original = "line0\nline1\nline2\n";
    fs::write(&source, original).expect("lib");
    fs::write(temp.path().join("other.rs"), "other\n").expect("other");

    let path_cases = [
        "diff --git a/../escape.rs b/../escape.rs\n--- a/../escape.rs\n+++ b/../escape.rs\n@@ -1 +1 @@\n-old\n+new\n",
        "diff --git a/lib.rs b/lib.rs\n--- a/lib.rs\n+++ b/lib.rs\n@@ -1 +1 @@\n-line0\n+lineX\ndiff --git a/other.rs b/lib.rs\n--- a/other.rs\n+++ b/lib.rs\n@@ -1 +1 @@\n-other\n+otherX\n",
        "diff --git a/lib.rs b//tmp/escape.rs\n--- a/lib.rs\n+++ /tmp/escape.rs\n@@ -1 +1 @@\n-line0\n+lineX\n",
    ];
    for diff in path_cases {
        let result = std::panic::catch_unwind(|| {
            GuardedPatchApplier::apply_unified_diff_checked(temp.path(), diff)
        });
        assert!(result.is_ok(), "parser must not panic for path corpus");
        assert!(result.expect("catch unwind").is_err());
        assert_eq!(fs::read_to_string(&source).expect("lib"), original);
    }

    for seed in 0..128 {
        fs::write(&source, original).expect("reset lib");
        let hunk = match seed % 4 {
            0 => format!("@@ -{} +{} @@\n", seed + 1000, seed + 1000),
            1 => format!("@@ -x{} +{} @@\n", seed, seed),
            2 => format!("@@ -{},{} +{},{} @@\n", seed + 5, seed + 3, seed, seed + 1),
            _ => format!("@@ -{} +abc{} @@\n", seed + 1, seed),
        };
        let diff = format!(
            "diff --git a/lib.rs b/lib.rs\n--- a/lib.rs\n+++ b/lib.rs\n{hunk}-line0\n+line{seed}\n"
        );
        let result = std::panic::catch_unwind(|| {
            GuardedPatchApplier::apply_unified_diff_checked(temp.path(), &diff)
        });
        assert!(
            result.is_ok(),
            "parser must not panic for generated corpus seed {seed}"
        );
        assert!(
            result.expect("catch unwind").is_err(),
            "seed {seed} should fail"
        );
        assert_eq!(fs::read_to_string(&source).expect("lib"), original);
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
