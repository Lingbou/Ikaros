// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use std::fs;

#[test]
fn schedule_store_adds_lists_and_filters_due_jobs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalScheduleStore::new(temp.path());
    let job = store
        .add(
            "summarize local context",
            "2000-01-01T00:00:00Z",
            None,
            Some("build".into()),
        )
        .expect("add");

    assert_eq!(store.list().expect("list").len(), 1);
    assert_eq!(store.due_now().expect("due"), vec![job]);
}

#[test]
fn schedule_store_defaults_to_local_delivery_target() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalScheduleStore::new(temp.path());
    let job = store
        .add("deliver locally", "2000-01-01T00:00:00Z", None, None)
        .expect("add");

    assert_eq!(job.deliveries, vec![ScheduleDeliveryTarget::LocalFile]);
}

#[test]
fn schedule_store_accepts_explicit_delivery_targets() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalScheduleStore::new(temp.path());
    let job = store
        .add_with_deliveries(
            "deliver through gateway",
            "2000-01-01T00:00:00Z",
            None,
            None,
            vec![ScheduleDeliveryTarget::GatewayOutbox],
        )
        .expect("add");

    assert_eq!(job.deliveries, vec![ScheduleDeliveryTarget::GatewayOutbox]);
}

#[test]
fn schedule_store_records_one_shot_run_as_disabled() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalScheduleStore::new(temp.path());
    let job = store
        .add("one shot", "2000-01-01T00:00:00Z", None, None)
        .expect("add");
    let update = store
        .record_run(&job.id, "Completed", "ran sk-not-real")
        .expect("record")
        .expect("update");

    assert!(!update.enabled);
    assert!(update.next_run_at.is_none());
    let jobs = store.list().expect("list");
    assert!(!jobs[0].enabled);
    assert!(
        !jobs[0]
            .last_summary
            .as_deref()
            .expect("summary")
            .contains("sk-not-real")
    );
}

#[test]
fn schedule_store_advances_recurring_jobs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalScheduleStore::new(temp.path());
    let job = store
        .add("recurring", "2000-01-01T00:00:00Z", Some(60), None)
        .expect("add");
    let update = store
        .record_run(&job.id, "Completed", "ok")
        .expect("record")
        .expect("update");

    assert!(update.enabled);
    assert!(update.next_run_at.is_some());
    assert!(store.list().expect("list")[0].enabled);
}

#[test]
fn schedule_store_retries_failed_one_shot_jobs_with_backoff() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalScheduleStore::new(temp.path());
    let job = store
        .add_with_options(
            "retry once",
            "2000-01-01T00:00:00Z",
            ScheduleJobOptions {
                retry: ScheduleRetryPolicy {
                    max_attempts: 3,
                    backoff_seconds: 60,
                },
                ..ScheduleJobOptions::default()
            },
        )
        .expect("add");

    let first = store
        .record_run(&job.id, "Failed", "temporary failure")
        .expect("record first failure")
        .expect("first update");
    let after_first = store.list().expect("list after first failure")[0].clone();
    assert!(first.enabled);
    assert!(first.next_run_at.is_some());
    assert_eq!(after_first.retry_attempts, 1);
    assert_eq!(after_first.history.len(), 1);
    assert_eq!(after_first.history[0].attempt, 1);

    let second = store
        .record_run(&job.id, "Failed", "temporary failure again")
        .expect("record second failure")
        .expect("second update");
    let after_second = store.list().expect("list after second failure")[0].clone();
    assert!(second.enabled);
    assert!(second.next_run_at.is_some());
    assert_eq!(after_second.retry_attempts, 2);

    let third = store
        .record_run(&job.id, "Failed", "permanent failure")
        .expect("record third failure")
        .expect("third update");
    let after_third = store.list().expect("list after third failure")[0].clone();
    assert!(!third.enabled);
    assert!(third.next_run_at.is_none());
    assert_eq!(after_third.retry_attempts, 3);
    assert_eq!(after_third.history.len(), 3);
}

#[test]
fn schedule_store_does_not_run_jobs_past_grace_period() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalScheduleStore::new(temp.path());
    store
        .add_with_options(
            "missed long ago",
            "2000-01-01T00:00:00Z",
            ScheduleJobOptions {
                grace_period_seconds: Some(60),
                ..ScheduleJobOptions::default()
            },
        )
        .expect("add stale job");
    store
        .add(
            "still due without grace",
            "2000-01-01T00:00:00Z",
            None,
            None,
        )
        .expect("add ordinary job");

    let due = store.due_now().expect("due");

    assert_eq!(due.len(), 1);
    assert_eq!(due[0].title, "still due without grace");
}

#[test]
fn schedule_store_rejects_intervals_that_exceed_duration_range() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalScheduleStore::new(temp.path());
    let too_large = i64::MAX as u64 + 1;

    let error = store
        .add("too large", "2000-01-01T00:00:00Z", Some(too_large), None)
        .expect_err("interval should be rejected");

    assert!(error.to_string().contains("schedule interval"));
    assert!(!store.path().exists());
}

#[cfg(unix)]
#[test]
fn schedule_store_preserves_existing_file_when_atomic_temp_cannot_be_created() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalScheduleStore::new(temp.path());
    let job = store
        .add("keep original file", "2000-01-01T00:00:00Z", None, None)
        .expect("add");
    let original = fs::read_to_string(store.path()).expect("schedule file");

    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o500))
        .expect("make schedule dir readonly");
    let result = store.set_enabled(&job.id, false);
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700))
        .expect("restore schedule dir permissions");

    result.expect_err("schedule write should fail before replacing the old file");
    let after = fs::read_to_string(store.path()).expect("schedule file after failed write");
    assert_eq!(after, original);
}

#[test]
fn schedule_store_deletes_jobs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalScheduleStore::new(temp.path());
    let job = store
        .add("delete me", "2000-01-01T00:00:00Z", None, None)
        .expect("add");

    assert!(store.delete(&job.id).expect("delete"));
    assert!(store.list().expect("list").is_empty());
    assert!(!store.delete(&job.id).expect("delete missing"));
}

#[test]
fn schedule_file_redacts_tasks_before_storage() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalScheduleStore::new(temp.path());
    store
        .add(
            "remember api_key=abc123",
            "2000-01-01T00:00:00Z",
            None,
            None,
        )
        .expect("add");

    let raw = fs::read_to_string(store.path()).expect("schedule file");
    assert!(!raw.contains("abc123"));
    assert!(raw.contains("[REDACTED_SECRET]"));
}

#[test]
fn schedule_store_writes_redacted_local_delivery() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalScheduleStore::new(temp.path());
    let path = store
        .write_local_delivery("job/unsafe", "run:unsafe", "summary api_key=abc123")
        .expect("delivery");

    assert!(path.starts_with(store.deliveries_dir()));
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("run-unsafe.md")
    );
    let raw = fs::read_to_string(path).expect("delivery file");
    assert!(!raw.contains("abc123"));
    assert!(raw.contains("[REDACTED_SECRET]"));
}
