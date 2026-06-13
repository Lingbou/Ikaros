// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use ikaros_automation::{LocalScheduleStore, ScheduleDeliveryTarget};
use ikaros_core::{IkarosPaths, TaskState};

#[tokio::test]
async fn run_schedule_worker_tick_executes_due_job_and_records_update() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);
    let store = LocalScheduleStore::new(&paths.automation_dir);
    let job = store
        .add(
            "summarize runtime schedule",
            "now",
            None,
            Some("build".into()),
        )
        .expect("add job");

    let tick = run_schedule_worker_tick(&store, 10, &paths, &workspace, None)
        .await
        .expect("tick");

    assert_eq!(tick.kind, "schedule_worker_tick");
    assert_eq!(tick.due, 1);
    assert_eq!(tick.ran, 1);
    assert_eq!(tick.reports[0].job_id, job.id);
    assert_eq!(tick.reports[0].task_state, TaskState::Completed);
    assert!(tick.reports[0].summary.contains("scheduled step"));
    assert_eq!(tick.reports[0].deliveries.len(), 1);
    assert_eq!(
        tick.reports[0].deliveries[0].target,
        ScheduleDeliveryTarget::LocalFile
    );
    assert_eq!(tick.reports[0].deliveries[0].status, "Delivered");
    let delivery_path = tick.reports[0].deliveries[0]
        .path
        .as_ref()
        .expect("delivery path");
    let delivery = std::fs::read_to_string(delivery_path).expect("delivery content");
    assert!(delivery.contains("# Ikaros Scheduled Job Result"));
    assert!(delivery.contains(&job.id));
    assert!(
        tick.reports[0]
            .update
            .as_ref()
            .is_some_and(|update| !update.enabled)
    );

    let stored = store
        .list()
        .expect("list")
        .into_iter()
        .find(|candidate| candidate.id == job.id)
        .expect("stored job");
    assert!(!stored.enabled);
    assert_eq!(stored.last_status.as_deref(), Some("Completed"));
}

#[tokio::test]
async fn run_schedule_worker_tick_can_deliver_to_gateway_outbox() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);
    let store = LocalScheduleStore::new(&paths.automation_dir);
    let job = store
        .add_with_deliveries(
            "summarize gateway schedule",
            "now",
            None,
            None,
            vec![ScheduleDeliveryTarget::GatewayOutbox],
        )
        .expect("add job");

    let tick = run_schedule_worker_tick(&store, 10, &paths, &workspace, None)
        .await
        .expect("tick");

    assert_eq!(tick.reports[0].job_id, job.id);
    assert_eq!(
        tick.reports[0].deliveries[0].target,
        ScheduleDeliveryTarget::GatewayOutbox
    );
    assert_eq!(tick.reports[0].deliveries[0].status, "Delivered");
    let outbox = std::fs::read_to_string(paths.gateway_dir.join("outbox.jsonl")).expect("outbox");
    assert!(outbox.contains("\"kind\":\"schedule_report\""));
    assert!(outbox.contains("Ikaros Scheduled Job Result"));
    assert!(outbox.contains(&job.id));
}

fn write_offline_mock_config(paths: &IkarosPaths) {
    std::fs::create_dir_all(&paths.home).expect("home");
    std::fs::write(
        &paths.config,
        r#"model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag:
  embedding_provider: hash
  embedding_model: text-embedding-3-small
"#,
    )
    .expect("mock config");
}
