// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use ikaros_core::IkarosPaths;
use ikaros_gateway::{GatewayMessageKind, GatewayMessageStatus, GatewayRoute, LocalGatewayStore};

#[tokio::test]
async fn drain_gateway_chat_message_records_delivery_and_redacts_inbox() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    let queued = store
        .enqueue(GatewayRoute::new(
            "test",
            GatewayMessageKind::Chat,
            "hello token=abc123",
            Some("build".into()),
        ))
        .expect("enqueue");

    let reports = drain_gateway_messages(
        store.pending(10).expect("pending"),
        &store,
        &paths,
        &workspace,
        None,
    )
    .await
    .expect("drain");

    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].message_id, queued.id);
    assert_eq!(reports[0].kind, "chat");
    assert_eq!(reports[0].status, GatewayMessageStatus::Processed);
    assert_eq!(reports[0].provider.as_deref(), Some("mock"));
    assert_eq!(store.deliveries().expect("deliveries").len(), 1);
    let inbox = std::fs::read_to_string(store.inbox_path()).expect("inbox");
    assert!(!inbox.contains("abc123"));
    assert!(inbox.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn drain_gateway_task_message_records_task_report_delivery() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    let queued = store
        .enqueue(GatewayRoute::new(
            "test",
            GatewayMessageKind::Task,
            "summarize runtime gateway",
            Some("build".into()),
        ))
        .expect("enqueue");

    let reports = drain_gateway_messages(
        store.pending(10).expect("pending"),
        &store,
        &paths,
        &workspace,
        None,
    )
    .await
    .expect("drain");

    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].message_id, queued.id);
    assert_eq!(reports[0].kind, "task");
    assert_eq!(reports[0].status, GatewayMessageStatus::Processed);
    assert!(reports[0].task_report.is_some());
    let deliveries = store.deliveries().expect("deliveries");
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].kind, "task_report");
    assert!(deliveries[0].content.contains("\"task_id\""));
}

#[tokio::test]
async fn gateway_worker_tick_drains_limited_pending_messages() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    let queued = store
        .enqueue(GatewayRoute::new(
            "test",
            GatewayMessageKind::Task,
            "summarize gateway worker",
            Some("build".into()),
        ))
        .expect("enqueue");

    let tick = run_gateway_worker_tick(&store, 1, &paths, &workspace, None)
        .await
        .expect("worker tick");

    assert_eq!(tick.kind, "gateway_worker_tick");
    assert_eq!(tick.pending, 1);
    assert_eq!(tick.drained, 1);
    assert_eq!(tick.reports[0].message_id, queued.id);
    assert_eq!(store.pending(10).expect("pending").len(), 0);
    assert_eq!(store.deliveries().expect("deliveries").len(), 1);
}

fn write_offline_mock_config(paths: &IkarosPaths) {
    std::fs::create_dir_all(&paths.home).expect("home");
    std::fs::write(
        &paths.config,
        r#"[model.default]
provider = "mock"
runtime = "harness-agent-loop"
transport = "mock"
model = "mock-ikaros"

[rag]
embedding_provider = "hash"
embedding_model = "text-embedding-3-small"
"#,
    )
    .expect("mock config");
}
