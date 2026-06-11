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

#[test]
fn schedule_store_rejects_stored_intervals_that_exceed_duration_range() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalScheduleStore::new(temp.path());
    let job = ScheduledJob::new(
        "bad legacy recurring",
        "2000-01-01T00:00:00Z",
        Some(i64::MAX as u64 + 1),
        None,
        ScheduleDeliveryTarget::default_targets(),
    )
    .expect("legacy job");
    fs::write(
        store.path(),
        format!("{}\n", serde_json::to_string(&job).expect("job json")),
    )
    .expect("write job");

    let error = store
        .record_run(&job.id, "Completed", "ok")
        .expect_err("bad stored interval should be rejected");

    assert!(error.to_string().contains("schedule interval"));
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
