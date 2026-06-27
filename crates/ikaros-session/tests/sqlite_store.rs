use ikaros_protocol::{ModelRequestDiagnostic, ModelStreamEvent};
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, ApprovalRecord, ApprovalStatus,
    ContinuationId, PersistingAgentEventSink, PersistingAgentTurnSink, SessionBranchSummaryInput,
    SessionCompactionInput, SessionContinuationClaim, SessionContinuationInput,
    SessionContinuationKind, SessionContinuationStatus, SessionContinuationStatusReason,
    SessionEntry, SessionEntryKind, SessionId, SessionInputAdmission, SessionInputStatus,
    SessionRecord, SessionRetryInput, SessionSearchIndex, SessionSearchQuery, SessionSource,
    SessionStore, SessionTimelineQuery, SessionTimelineRecord, SessionTurnRecord,
    SessionTurnStatus, SqliteSessionStore, TurnId,
};
use serde_json::json;
use std::sync::Arc;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

fn parse_time(value: &str) -> OffsetDateTime {
    OffsetDateTime::parse(value, &Rfc3339).expect("timestamp")
}

fn sample_session(session_id: SessionId) -> SessionRecord {
    let mut session = SessionRecord::new(session_id, SessionSource::Cli);
    session.agent_id = Some("build".into());
    session
}

fn sample_entry(
    session_id: SessionId,
    turn_id: TurnId,
    kind: SessionEntryKind,
    text: &str,
) -> SessionEntry {
    let mut entry = SessionEntry::new(session_id, kind);
    entry.turn_id = Some(turn_id);
    entry.visible_text = Some(text.into());
    entry.payload = json!({ "text": text });
    entry
}

fn sample_events(session_id: SessionId, turn_id: TurnId) -> Vec<AgentEvent> {
    let start = AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        None,
        AgentEventSource::Runtime,
        AgentEventKind::TurnStart,
        json!({"step": 1}),
    );
    let model = AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        Some(start.event_id.clone()),
        AgentEventSource::Model,
        AgentEventKind::ModelStream(ModelStreamEvent::TextDelta("hello".into())),
        json!({"step": 2}),
    );
    let end = AgentEvent::new(
        session_id,
        turn_id,
        Some(model.event_id.clone()),
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        json!({"step": 3}),
    );
    vec![start, model, end]
}

fn sample_approval(session_id: SessionId, turn_id: TurnId) -> ApprovalRecord {
    ApprovalRecord {
        approval_id: "approval-turn".into(),
        session_id,
        turn_id: Some(turn_id),
        at: OffsetDateTime::now_utc(),
        status: ApprovalStatus::Requested,
        request: json!({"tool": "write_file"}),
        decision: None,
    }
}

#[test]
fn sqlite_store_admits_promotes_and_cancels_session_inputs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-inputs");
    store
        .upsert_session(&sample_session(session_id.clone()))
        .expect("session");

    let mut input = SessionInputAdmission::new(session_id.clone(), json!({"text": "hello"}));
    input.idempotency_key_digest = Some("digest-a".into());
    let admitted = store.admit_input(&input).expect("admit input");

    assert_eq!(admitted.session_id, session_id);
    assert_eq!(admitted.status, SessionInputStatus::Admitted);
    assert_eq!(admitted.payload, json!({"text": "hello"}));
    assert_eq!(admitted.idempotency_key_digest.as_deref(), Some("digest-a"));

    let turn_id = TurnId::from("turn-inputs");
    let promoted = store
        .promote_input(&admitted.input_id, &turn_id)
        .expect("promote input")
        .expect("input exists");
    assert_eq!(promoted.status, SessionInputStatus::Promoted);
    assert_eq!(promoted.promoted_turn_id, Some(turn_id));
    assert!(promoted.promoted_at.is_some());

    let cancelled = store
        .cancel_input(&admitted.input_id, "too late")
        .expect("cancel promoted input");
    assert!(cancelled.is_none());

    let inputs = store.session_inputs(&session_id).expect("session inputs");
    assert_eq!(inputs, vec![promoted]);
}

#[test]
fn sqlite_store_persists_session_turn_status_and_terminal_reason() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-turns");
    let turn_id = TurnId::from("turn-turns");
    store
        .upsert_session(&sample_session(session_id.clone()))
        .expect("session");

    let mut turn = SessionTurnRecord::new(session_id.clone(), turn_id.clone());
    turn.status = SessionTurnStatus::Running;
    turn.lease_owner = Some("worker-a".into());
    turn.lease_expires_at = Some(parse_time("2026-06-20T00:01:00Z"));
    store.upsert_turn(&turn).expect("upsert running turn");

    let mut failed = turn.clone();
    failed.status = SessionTurnStatus::Failed;
    failed.completed_at = Some(parse_time("2026-06-20T00:02:00Z"));
    failed.error = Some("provider failed".into());
    store.upsert_turn(&failed).expect("upsert failed turn");

    let stored = store
        .session_turn(&session_id, &turn_id)
        .expect("read turn")
        .expect("turn exists");
    assert_eq!(stored.status, SessionTurnStatus::Failed);
    assert_eq!(stored.lease_owner.as_deref(), Some("worker-a"));
    assert_eq!(stored.error.as_deref(), Some("provider failed"));

    let turns = store.session_turns(&session_id).expect("turns");
    assert_eq!(turns, vec![stored]);
}

#[test]
fn sqlite_store_replays_session_timeline() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-a");
    let mut session = SessionRecord::new(session_id.clone(), SessionSource::Cli);
    session.agent_id = Some("build".into());
    store.upsert_session(&session).expect("session");

    let mut user = SessionEntry::new(session_id.clone(), SessionEntryKind::UserMessage);
    user.visible_text = Some("hello".into());
    let parent_id = user.entry_id.clone();
    store.append_entry(&user).expect("user entry");

    let mut assistant = SessionEntry::new(session_id.clone(), SessionEntryKind::AssistantMessage);
    assistant.parent_entry_id = Some(parent_id);
    assistant.visible_text = Some("world".into());
    store.append_entry(&assistant).expect("assistant entry");

    let turn_id = TurnId::from("turn-a");
    let start = AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        None,
        AgentEventSource::Runtime,
        AgentEventKind::TurnStart,
        json!({"iteration": 1}),
    );
    store.append_agent_event(&start).expect("start event");
    let model = AgentEvent::new(
        session_id.clone(),
        turn_id,
        Some(start.event_id.clone()),
        AgentEventSource::Model,
        AgentEventKind::ModelStream(ModelStreamEvent::TextDelta("world".into())),
        serde_json::Value::Null,
    );
    store.append_agent_event(&model).expect("model event");

    store
        .append_approval(&ApprovalRecord {
            approval_id: "approval-a".into(),
            session_id: session_id.clone(),
            turn_id: None,
            at: OffsetDateTime::now_utc(),
            status: ApprovalStatus::Requested,
            request: json!({"tool": "write_file"}),
            decision: None,
        })
        .expect("approval");

    let replay = store
        .replay_session(&session_id)
        .expect("replay")
        .expect("session exists");
    assert_eq!(replay.session.agent_id.as_deref(), Some("build"));
    assert_eq!(replay.entries.len(), 2);
    assert_eq!(
        replay.entries[1].parent_entry_id,
        Some(replay.entries[0].entry_id.clone())
    );
    assert_eq!(replay.agent_events.len(), 2);
    assert_eq!(
        replay.agent_events[1].parent_event_id,
        Some(replay.agent_events[0].event_id.clone())
    );
    assert_eq!(replay.approvals.len(), 1);
}

#[test]
fn sqlite_store_replays_unified_session_timeline_in_write_order() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-unified-timeline");
    let turn_id = TurnId::from("turn-unified-timeline");
    store
        .upsert_session(&sample_session(session_id.clone()))
        .expect("session");

    let start = AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        None,
        AgentEventSource::Runtime,
        AgentEventKind::TurnStart,
        json!({"step": "start"}),
    );
    store.append_agent_event(&start).expect("start event");

    let user = sample_entry(
        session_id.clone(),
        turn_id.clone(),
        SessionEntryKind::UserMessage,
        "hello timeline",
    );
    store.append_entry(&user).expect("user entry");

    let approval = sample_approval(session_id.clone(), turn_id.clone());
    store.append_approval(&approval).expect("approval");

    let end = AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        Some(start.event_id.clone()),
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        json!({"step": "end"}),
    );
    store.append_agent_event(&end).expect("end event");

    let timeline = store.session_timeline(&session_id).expect("timeline");
    assert_eq!(timeline.len(), 4);
    assert!(
        timeline
            .windows(2)
            .all(|items| items[0].sequence < items[1].sequence)
    );
    assert_eq!(timeline[0].turn_id.as_ref(), Some(&turn_id));
    assert_eq!(timeline[1].turn_id.as_ref(), Some(&turn_id));
    assert_eq!(timeline[2].turn_id.as_ref(), Some(&turn_id));
    assert_eq!(timeline[3].turn_id.as_ref(), Some(&turn_id));

    match &timeline[0].record {
        SessionTimelineRecord::AgentEvent(event) => assert_eq!(event.event_id, start.event_id),
        other => panic!("expected start event, got {other:?}"),
    }
    match &timeline[1].record {
        SessionTimelineRecord::Entry(entry) => assert_eq!(entry.entry_id, user.entry_id),
        other => panic!("expected user entry, got {other:?}"),
    }
    match &timeline[2].record {
        SessionTimelineRecord::Approval(record) => {
            assert_eq!(record.approval_id, approval.approval_id)
        }
        other => panic!("expected approval, got {other:?}"),
    }
    match &timeline[3].record {
        SessionTimelineRecord::AgentEvent(event) => assert_eq!(event.event_id, end.event_id),
        other => panic!("expected end event, got {other:?}"),
    }
}

#[test]
fn sqlite_store_pages_and_filters_unified_session_timeline() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-unified-timeline-page");
    let first_turn = TurnId::from("turn-page-a");
    let second_turn = TurnId::from("turn-page-b");
    store
        .upsert_session(&sample_session(session_id.clone()))
        .expect("session");

    let first_start = AgentEvent::new(
        session_id.clone(),
        first_turn.clone(),
        None,
        AgentEventSource::Runtime,
        AgentEventKind::TurnStart,
        json!({"turn": "a"}),
    );
    store.append_agent_event(&first_start).expect("first start");
    let first_entry = sample_entry(
        session_id.clone(),
        first_turn.clone(),
        SessionEntryKind::UserMessage,
        "first page entry",
    );
    store.append_entry(&first_entry).expect("first entry");
    let approval = sample_approval(session_id.clone(), first_turn.clone());
    store.append_approval(&approval).expect("approval");

    let second_start = AgentEvent::new(
        session_id.clone(),
        second_turn.clone(),
        None,
        AgentEventSource::Runtime,
        AgentEventKind::TurnStart,
        json!({"turn": "b"}),
    );
    store
        .append_agent_event(&second_start)
        .expect("second start");
    let second_entry = sample_entry(
        session_id.clone(),
        second_turn.clone(),
        SessionEntryKind::AssistantMessage,
        "second page entry",
    );
    store.append_entry(&second_entry).expect("second entry");

    let page = store
        .session_timeline_page(&SessionTimelineQuery::new(session_id.clone()).with_page(2, 2))
        .expect("timeline page");
    assert_eq!(page.page, 2);
    assert_eq!(page.page_size, 2);
    assert_eq!(page.total_items, 5);
    assert_eq!(page.items.len(), 2);
    match &page.items[0].record {
        SessionTimelineRecord::Approval(record) => {
            assert_eq!(record.approval_id, approval.approval_id)
        }
        other => panic!("expected approval on page 2, got {other:?}"),
    }
    match &page.items[1].record {
        SessionTimelineRecord::AgentEvent(event) => {
            assert_eq!(event.event_id, second_start.event_id)
        }
        other => panic!("expected second turn event on page 2, got {other:?}"),
    }

    let filtered = store
        .session_timeline_page(
            &SessionTimelineQuery::new(session_id)
                .for_turn(second_turn.clone())
                .with_page(1, 10),
        )
        .expect("filtered timeline page");
    assert_eq!(filtered.turn_id.as_ref(), Some(&second_turn));
    assert_eq!(filtered.total_items, 2);
    assert_eq!(filtered.items.len(), 2);
    assert!(
        filtered
            .items
            .iter()
            .all(|item| item.turn_id.as_ref() == Some(&second_turn))
    );
    match &filtered.items[0].record {
        SessionTimelineRecord::AgentEvent(event) => {
            assert_eq!(event.event_id, second_start.event_id)
        }
        other => panic!("expected filtered event, got {other:?}"),
    }
    match &filtered.items[1].record {
        SessionTimelineRecord::Entry(entry) => assert_eq!(entry.entry_id, second_entry.entry_id),
        other => panic!("expected filtered entry, got {other:?}"),
    }
}

#[test]
fn sqlite_store_replays_session_timeline_page_without_full_replay() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-page");
    let turn_id = TurnId::from("turn-page");
    store
        .upsert_session(&sample_session(session_id.clone()))
        .expect("session");

    for index in 0..5 {
        store
            .append_entry(&sample_entry(
                session_id.clone(),
                turn_id.clone(),
                SessionEntryKind::UserMessage,
                &format!("entry {index}"),
            ))
            .expect("entry");
    }
    for event in sample_events(session_id.clone(), turn_id.clone()) {
        store.append_agent_event(&event).expect("event");
    }
    for index in 0..3 {
        let mut approval = sample_approval(session_id.clone(), turn_id.clone());
        approval.approval_id = format!("approval-{index}");
        store.append_approval(&approval).expect("approval");
    }

    let page = store
        .replay_session_page(&session_id, 2, 2)
        .expect("paged replay")
        .expect("session exists");

    assert_eq!(page.page, 2);
    assert_eq!(page.page_size, 2);
    assert_eq!(page.total_entries, 5);
    assert_eq!(page.total_agent_events, 3);
    assert_eq!(page.total_approvals, 3);
    assert_eq!(page.entries.len(), 2);
    assert_eq!(page.entries[0].visible_text.as_deref(), Some("entry 2"));
    assert_eq!(page.entries[1].visible_text.as_deref(), Some("entry 3"));
    assert_eq!(page.agent_events.len(), 1);
    assert_eq!(page.agent_events[0].kind, AgentEventKind::TurnEnd);
    assert_eq!(page.approvals.len(), 1);
    assert_eq!(page.approvals[0].approval_id, "approval-2");
}

#[test]
fn persisting_event_sink_creates_session_and_appends_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store: Arc<dyn SessionStore> = Arc::new(SqliteSessionStore::new(temp.path()));
    let sink = PersistingAgentEventSink::new(store.clone())
        .with_source(SessionSource::Cli)
        .with_agent_id("build")
        .with_workspace(temp.path().join("workspace"));
    let event = AgentEvent::new(
        "session-b",
        "turn-b",
        None,
        AgentEventSource::Runtime,
        AgentEventKind::SessionStart,
        serde_json::Value::Null,
    );

    sink.emit(&event).expect("emit");

    let replay = store
        .replay_session(&SessionId::from("session-b"))
        .expect("replay")
        .expect("session exists");
    assert_eq!(replay.session.source, SessionSource::Cli);
    assert_eq!(replay.session.agent_id.as_deref(), Some("build"));
    assert_eq!(replay.agent_events, vec![event]);
}

#[test]
fn event_sink_does_not_clear_existing_session_tree_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(SqliteSessionStore::new(temp.path()));
    let session_id = SessionId::from("session-tree");
    let mut parent = SessionRecord::new("parent-session", SessionSource::Cli);
    parent.agent_id = Some("parent-agent".into());
    store.upsert_session(&parent).expect("parent session");
    let mut session = SessionRecord::new(session_id.clone(), SessionSource::Cli);
    session.parent_session_id = Some(parent.session_id.clone());
    store.upsert_session(&session).expect("session");
    let leaf = SessionEntry::new(session_id.clone(), SessionEntryKind::Leaf);
    let leaf_id = leaf.entry_id.clone();
    store.append_entry(&leaf).expect("leaf");

    let sink = PersistingAgentEventSink::new(store.clone());
    let event = AgentEvent::new(
        session_id.clone(),
        "turn-tree",
        None,
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        serde_json::Value::Null,
    );
    sink.emit(&event).expect("emit");

    let replay = store
        .replay_session(&session_id)
        .expect("replay")
        .expect("session exists");
    assert_eq!(replay.session.parent_session_id, Some(parent.session_id));
    assert_eq!(replay.session.active_leaf_entry_id, Some(leaf_id));
}

#[test]
fn session_writer_preserves_event_order_for_one_turn() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-writer-order");
    let turn_id = TurnId::from("turn-writer-order");
    let session = sample_session(session_id.clone());
    let mut events = sample_events(session_id.clone(), turn_id.clone());
    events[0].at = parse_time("2026-06-20T00:00:00Z");
    events[1].at = parse_time("2026-06-20T00:00:01Z");
    events[2].at = parse_time("2026-06-20T00:00:00.5Z");

    let mut writer = store.begin_turn(&session, &turn_id).expect("writer");
    for event in &events {
        writer.append_agent_event(event).expect("event");
    }
    writer.commit().expect("commit");

    let replay = store
        .replay_session(&session_id)
        .expect("replay")
        .expect("session exists");
    let stored_ids = replay
        .agent_events
        .iter()
        .map(|event| event.event_id.clone())
        .collect::<Vec<_>>();
    let expected_ids = events
        .iter()
        .map(|event| event.event_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(stored_ids, expected_ids);
}

#[test]
fn session_writer_records_committed_turn_status() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-writer-turn");
    let turn_id = TurnId::from("turn-writer-turn");
    let session = sample_session(session_id.clone());
    let event = AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        None,
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        serde_json::Value::Null,
    );

    let mut writer = store.begin_turn(&session, &turn_id).expect("writer");
    writer.append_agent_event(&event).expect("event");
    writer.commit().expect("commit");

    let turn = store
        .session_turn(&session_id, &turn_id)
        .expect("turn")
        .expect("turn exists");
    assert_eq!(turn.status, SessionTurnStatus::Completed);
    assert!(turn.completed_at.is_some());
}

#[test]
fn session_writer_rollback_removes_uncommitted_turn_status() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-writer-turn-rollback");
    let turn_id = TurnId::from("turn-writer-turn-rollback");
    let session = sample_session(session_id.clone());

    let writer = store.begin_turn(&session, &turn_id).expect("writer");
    writer.rollback().expect("rollback");

    let turn = store
        .session_turn(&session_id, &turn_id)
        .expect("turn after rollback");
    assert!(turn.is_none());
}

#[test]
fn session_writer_event_write_does_not_clear_active_leaf() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-writer-leaf");
    let turn_id = TurnId::from("turn-writer-leaf");
    let session = sample_session(session_id.clone());
    store.upsert_session(&session).expect("session");
    let leaf = sample_entry(
        session_id.clone(),
        turn_id.clone(),
        SessionEntryKind::Leaf,
        "leaf",
    );
    let leaf_id = leaf.entry_id.clone();
    store.append_entry(&leaf).expect("leaf");
    let event = sample_events(session_id.clone(), turn_id.clone())
        .into_iter()
        .next()
        .expect("event");

    let mut writer = store.begin_turn(&session, &turn_id).expect("writer");
    writer.append_agent_event(&event).expect("event");
    writer.commit().expect("commit");

    let replay = store
        .replay_session(&session_id)
        .expect("replay")
        .expect("session exists");
    assert_eq!(replay.session.active_leaf_entry_id, Some(leaf_id));
}

#[test]
fn session_writer_rolls_back_after_mid_turn_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-writer-rollback");
    let turn_id = TurnId::from("turn-writer-rollback");
    let session = sample_session(session_id.clone());
    let event = sample_events(session_id.clone(), turn_id.clone())
        .into_iter()
        .next()
        .expect("event");
    let wrong_session_event = AgentEvent::new(
        "other-session",
        turn_id.clone(),
        None,
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        serde_json::Value::Null,
    );

    let mut writer = store.begin_turn(&session, &turn_id).expect("writer");
    writer.append_agent_event(&event).expect("event");
    assert!(writer.append_agent_event(&wrong_session_event).is_err());
    assert!(writer.commit().is_err());

    assert!(store.replay_session(&session_id).expect("replay").is_none());
}

#[test]
fn session_writer_replay_matches_one_shot_writes() {
    let one_shot_temp = tempfile::tempdir().expect("one shot tempdir");
    let writer_temp = tempfile::tempdir().expect("writer tempdir");
    let one_shot = SqliteSessionStore::new(one_shot_temp.path());
    let writer_store = SqliteSessionStore::new(writer_temp.path());
    let session_id = SessionId::from("session-writer-parity");
    let turn_id = TurnId::from("turn-writer-parity");
    let session = sample_session(session_id.clone());
    let user = sample_entry(
        session_id.clone(),
        turn_id.clone(),
        SessionEntryKind::UserMessage,
        "hello",
    );
    let mut assistant = sample_entry(
        session_id.clone(),
        turn_id.clone(),
        SessionEntryKind::AssistantMessage,
        "world",
    );
    assistant.parent_entry_id = Some(user.entry_id.clone());
    let events = sample_events(session_id.clone(), turn_id.clone());
    let approval = sample_approval(session_id.clone(), turn_id.clone());

    one_shot.upsert_session(&session).expect("session");
    one_shot.append_entry(&user).expect("user");
    one_shot.append_entry(&assistant).expect("assistant");
    for event in &events {
        one_shot.append_agent_event(event).expect("event");
    }
    one_shot.append_approval(&approval).expect("approval");

    let mut writer = writer_store.begin_turn(&session, &turn_id).expect("writer");
    writer.append_entry(&user).expect("user");
    writer.append_entry(&assistant).expect("assistant");
    for event in &events {
        writer.append_agent_event(event).expect("event");
    }
    writer.append_approval(&approval).expect("approval");
    writer.commit().expect("commit");

    let one_shot_replay = one_shot
        .replay_session(&session_id)
        .expect("one shot replay")
        .expect("session exists");
    let writer_replay = writer_store
        .replay_session(&session_id)
        .expect("writer replay")
        .expect("session exists");
    assert_eq!(writer_replay, one_shot_replay);
}

#[test]
fn persisting_turn_sink_commits_events_in_one_transaction() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store: Arc<dyn SessionStore> = Arc::new(SqliteSessionStore::new(temp.path()));
    let sink = PersistingAgentTurnSink::new(store.clone())
        .with_source(SessionSource::Cli)
        .with_agent_id("build");
    let session_id = SessionId::from("session-turn-sink");
    let turn_id = TurnId::from("turn-turn-sink");
    let events = sample_events(session_id.clone(), turn_id);

    for event in &events {
        sink.emit(event).expect("emit");
    }
    sink.commit().expect("commit");

    let replay = store
        .replay_session(&session_id)
        .expect("replay")
        .expect("session exists");
    assert_eq!(replay.session.agent_id.as_deref(), Some("build"));
    assert_eq!(replay.agent_events, events);
}

#[test]
fn session_tree_reads_and_switches_active_branch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-branch");
    let turn_id = TurnId::from("turn-branch");
    store
        .upsert_session(&sample_session(session_id.clone()))
        .expect("session");
    let root = sample_entry(
        session_id.clone(),
        turn_id.clone(),
        SessionEntryKind::UserMessage,
        "root",
    );
    store.append_entry(&root).expect("root");
    let mut first_child = sample_entry(
        session_id.clone(),
        turn_id.clone(),
        SessionEntryKind::AssistantMessage,
        "first child",
    );
    first_child.parent_entry_id = Some(root.entry_id.clone());
    store.append_entry(&first_child).expect("first child");
    let mut second_child = sample_entry(
        session_id.clone(),
        turn_id,
        SessionEntryKind::AssistantMessage,
        "second child",
    );
    second_child.parent_entry_id = Some(root.entry_id.clone());
    store.append_entry(&second_child).expect("second child");

    let branch = store
        .active_branch(&session_id)
        .expect("active branch")
        .expect("session exists");
    assert_eq!(branch.entries.len(), 2);
    assert_eq!(branch.entries[0].entry_id, root.entry_id);
    assert_eq!(branch.entries[1].entry_id, second_child.entry_id);

    store
        .set_active_leaf(&session_id, &first_child.entry_id)
        .expect("switch leaf");
    let branch = store
        .active_branch(&session_id)
        .expect("active branch")
        .expect("session exists");
    assert_eq!(
        branch
            .entries
            .iter()
            .map(|entry| entry.visible_text.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("root"), Some("first child")]
    );
}

#[test]
fn session_tree_rejects_cross_session_active_leaf() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-active");
    let other_session_id = SessionId::from("session-other-active");
    let turn_id = TurnId::from("turn-active");
    store
        .upsert_session(&sample_session(session_id.clone()))
        .expect("session");
    store
        .upsert_session(&sample_session(other_session_id.clone()))
        .expect("other session");
    let other = sample_entry(
        other_session_id,
        turn_id,
        SessionEntryKind::UserMessage,
        "other",
    );
    store.append_entry(&other).expect("other");

    let error = store
        .set_active_leaf(&session_id, &other.entry_id)
        .expect_err("cross-session active leaf");
    assert!(error.to_string().contains("belongs to session"));
}

#[test]
fn session_tree_appends_branch_compaction_and_retry_entries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-tree-ops");
    let turn_id = TurnId::from("turn-tree-ops");
    store
        .upsert_session(&sample_session(session_id.clone()))
        .expect("session");
    let root = sample_entry(
        session_id.clone(),
        turn_id,
        SessionEntryKind::UserMessage,
        "original user message",
    );
    store.append_entry(&root).expect("root");

    let branch = store
        .branch_from_entry(&SessionBranchSummaryInput {
            session_id: session_id.clone(),
            parent_entry_id: root.entry_id.clone(),
            summary: "try a shorter answer".into(),
            payload: json!({"reason": "user retry"}),
        })
        .expect("branch summary");
    assert_eq!(branch.kind, SessionEntryKind::BranchSummary);
    assert_eq!(branch.parent_entry_id, Some(root.entry_id.clone()));

    let compaction = store
        .append_compaction(&SessionCompactionInput {
            session_id: session_id.clone(),
            parent_entry_id: branch.entry_id.clone(),
            summary: "compressed prior context".into(),
            compacted_entry_ids: vec![root.entry_id.clone(), branch.entry_id.clone()],
            payload: json!({"tokens_saved": 128}),
        })
        .expect("compaction");
    assert_eq!(compaction.kind, SessionEntryKind::Compaction);
    assert_eq!(
        compaction.payload["compacted_entry_ids"],
        json!([root.entry_id.as_str(), branch.entry_id.as_str()])
    );

    let retry = store
        .retry_from_entry(&SessionRetryInput {
            session_id: session_id.clone(),
            parent_entry_id: compaction.entry_id.clone(),
            reason: Some("retry after compaction".into()),
            payload: json!({"attempt": 2}),
        })
        .expect("retry");
    assert_eq!(retry.kind, SessionEntryKind::Leaf);
    assert_eq!(retry.parent_entry_id, Some(compaction.entry_id.clone()));

    let replay = store
        .replay_session(&session_id)
        .expect("replay")
        .expect("session exists");
    assert_eq!(replay.session.active_leaf_entry_id, Some(retry.entry_id));
    let active_branch = store
        .active_branch(&session_id)
        .expect("active branch")
        .expect("session exists");
    assert_eq!(
        active_branch
            .entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            SessionEntryKind::UserMessage,
            SessionEntryKind::BranchSummary,
            SessionEntryKind::Compaction,
            SessionEntryKind::Leaf,
        ]
    );
}

#[test]
fn continuation_queue_claims_by_priority_and_tracks_terminal_status() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-continuation-priority");
    store
        .upsert_session(&sample_session(session_id.clone()))
        .expect("session");

    let mut next =
        SessionContinuationInput::new(session_id.clone(), SessionContinuationKind::NextTurn);
    next.payload = json!({"content": "next"});
    let next = store.enqueue_continuation(&next).expect("next");

    let mut follow =
        SessionContinuationInput::new(session_id.clone(), SessionContinuationKind::FollowUp);
    follow.payload = json!({"content": "follow"});
    let follow = store.enqueue_continuation(&follow).expect("follow");

    let mut steer =
        SessionContinuationInput::new(session_id.clone(), SessionContinuationKind::Steer);
    steer.payload = json!({"content": "steer"});
    let steer = store.enqueue_continuation(&steer).expect("steer");

    let claim =
        SessionContinuationClaim::for_session(session_id.clone()).with_lease_owner("worker-a");
    let claimed = store
        .claim_next_continuation(&claim)
        .expect("claim")
        .expect("claimed");
    assert_eq!(claimed.continuation_id, steer.continuation_id);
    assert_eq!(claimed.status, SessionContinuationStatus::Running);
    assert_eq!(
        claimed.status_reason,
        Some(SessionContinuationStatusReason::Claimed)
    );
    assert_eq!(claimed.lease_owner.as_deref(), Some("worker-a"));
    assert!(claimed.claimed_at.is_some());

    let completed = store
        .complete_continuation(&claimed.continuation_id, json!({"turn_id": "turn-a"}))
        .expect("complete")
        .expect("completed");
    assert_eq!(completed.status, SessionContinuationStatus::Completed);
    assert_eq!(
        completed.status_reason,
        Some(SessionContinuationStatusReason::Completed)
    );
    assert_eq!(completed.payload["turn_id"], json!("turn-a"));
    assert!(completed.completed_at.is_some());

    let failed = store
        .claim_next_continuation(&claim)
        .expect("claim follow")
        .expect("follow claimed");
    assert_eq!(failed.continuation_id, follow.continuation_id);
    let failed = store
        .fail_continuation(&failed.continuation_id, "provider unavailable")
        .expect("fail")
        .expect("failed");
    assert_eq!(failed.status, SessionContinuationStatus::Failed);
    assert_eq!(
        failed.status_reason,
        Some(SessionContinuationStatusReason::Failed)
    );
    assert_eq!(failed.error.as_deref(), Some("provider unavailable"));

    let cancelled = store
        .cancel_continuation(&next.continuation_id, "user cancelled")
        .expect("cancel")
        .expect("cancelled");
    assert_eq!(cancelled.status, SessionContinuationStatus::Cancelled);
    assert_eq!(
        cancelled.status_reason,
        Some(SessionContinuationStatusReason::Cancelled)
    );
    assert_eq!(cancelled.error.as_deref(), Some("user cancelled"));

    let continuations = store.continuations(&session_id).expect("continuations");
    let status_for = |id: &ContinuationId| {
        continuations
            .iter()
            .find(|continuation| &continuation.continuation_id == id)
            .map(|continuation| continuation.status)
    };
    assert_eq!(
        status_for(&steer.continuation_id),
        Some(SessionContinuationStatus::Completed)
    );
    assert_eq!(
        status_for(&follow.continuation_id),
        Some(SessionContinuationStatus::Failed)
    );
    assert_eq!(
        status_for(&next.continuation_id),
        Some(SessionContinuationStatus::Cancelled)
    );
    assert!(
        store
            .claim_next_continuation(&claim)
            .expect("claim none")
            .is_none()
    );
}

#[test]
fn continuation_queue_survives_store_reopen_and_filters_by_turn_and_kind() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session_id = SessionId::from("session-continuation-reopen");
    let turn_id = TurnId::from("turn-continuation-reopen");
    let other_turn = TurnId::from("turn-other-continuation-reopen");
    let queued_id = {
        let store = SqliteSessionStore::new(temp.path());
        store
            .upsert_session(&sample_session(session_id.clone()))
            .expect("session");

        let mut retry =
            SessionContinuationInput::new(session_id.clone(), SessionContinuationKind::Retry);
        retry.turn_id = Some(turn_id.clone());
        retry.payload = json!({"entry_id": "leaf-a", "reason": "try again"});
        let retry = store.enqueue_continuation(&retry).expect("retry");

        let mut compact =
            SessionContinuationInput::new(session_id.clone(), SessionContinuationKind::Compact);
        compact.turn_id = Some(other_turn.clone());
        compact.payload = json!({"entry_id": "leaf-b"});
        store.enqueue_continuation(&compact).expect("compact");
        retry.continuation_id
    };

    let reopened = SqliteSessionStore::new(temp.path());
    let claim = SessionContinuationClaim::for_session(session_id.clone())
        .with_turn(turn_id)
        .with_kinds([SessionContinuationKind::Retry])
        .with_lease_owner("worker-b");
    let claimed = reopened
        .claim_next_continuation(&claim)
        .expect("claim")
        .expect("claimed");
    assert_eq!(claimed.continuation_id, queued_id);
    assert_eq!(claimed.kind, SessionContinuationKind::Retry);
    assert_eq!(claimed.payload["reason"], json!("try again"));

    let remaining = reopened.continuations(&session_id).expect("continuations");
    assert_eq!(remaining.len(), 2);
    assert_eq!(remaining[0].status, SessionContinuationStatus::Running);
    assert_eq!(remaining[1].status, SessionContinuationStatus::Queued);
}

#[test]
fn continuation_claim_reclaims_expired_running_lease() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-continuation-lease");
    store
        .upsert_session(&sample_session(session_id.clone()))
        .expect("session");

    let mut input =
        SessionContinuationInput::new(session_id.clone(), SessionContinuationKind::FollowUp);
    input.payload = json!({"content": "resume after crash"});
    let queued = store.enqueue_continuation(&input).expect("queued");

    let expired_claim = SessionContinuationClaim::for_session(session_id.clone())
        .with_lease_owner("worker-old")
        .with_lease_duration_seconds(0);
    let first = store
        .claim_next_continuation(&expired_claim)
        .expect("first claim")
        .expect("claimed");
    assert_eq!(first.continuation_id, queued.continuation_id);
    assert_eq!(first.status, SessionContinuationStatus::Running);
    assert_eq!(first.lease_owner.as_deref(), Some("worker-old"));
    assert_eq!(first.attempt_count, 1);
    assert!(first.lease_expires_at.is_some());

    let reopened = SqliteSessionStore::new(temp.path());
    let reclaim = SessionContinuationClaim::for_session(session_id.clone())
        .with_lease_owner("worker-new")
        .with_lease_duration_seconds(60);
    let reclaimed = reopened
        .claim_next_continuation(&reclaim)
        .expect("reclaim")
        .expect("reclaimed");
    assert_eq!(reclaimed.continuation_id, queued.continuation_id);
    assert_eq!(reclaimed.status, SessionContinuationStatus::Running);
    assert_eq!(
        reclaimed.status_reason,
        Some(SessionContinuationStatusReason::LeaseExpired)
    );
    assert_eq!(reclaimed.lease_owner.as_deref(), Some("worker-new"));
    assert_eq!(reclaimed.attempt_count, 2);
    assert_eq!(reclaimed.error.as_deref(), Some("lease expired"));
}

#[test]
fn running_continuation_can_be_cancelled_from_reopened_store() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-continuation-cancel-running");
    store
        .upsert_session(&sample_session(session_id.clone()))
        .expect("session");
    let queued = store
        .enqueue_continuation(&SessionContinuationInput::new(
            session_id.clone(),
            SessionContinuationKind::FollowUp,
        ))
        .expect("queued");
    let claimed = store
        .claim_next_continuation(
            &SessionContinuationClaim::for_session(session_id.clone())
                .with_lease_owner("worker-a")
                .with_lease_duration_seconds(60),
        )
        .expect("claim")
        .expect("claimed");
    assert_eq!(claimed.continuation_id, queued.continuation_id);
    assert_eq!(claimed.status, SessionContinuationStatus::Running);

    let reopened = SqliteSessionStore::new(temp.path());
    let cancelled = reopened
        .cancel_continuation(&queued.continuation_id, "external abort")
        .expect("cancel")
        .expect("cancelled");
    assert_eq!(cancelled.status, SessionContinuationStatus::Cancelled);
    assert_eq!(
        cancelled.status_reason,
        Some(SessionContinuationStatusReason::Cancelled)
    );
    assert_eq!(cancelled.error.as_deref(), Some("external abort"));
    assert!(cancelled.completed_at.is_some());
    assert!(cancelled.lease_expires_at.is_none());

    let observed = store
        .continuations(&session_id)
        .expect("continuations")
        .into_iter()
        .find(|continuation| continuation.continuation_id == queued.continuation_id)
        .expect("observed continuation");
    assert_eq!(observed.status, SessionContinuationStatus::Cancelled);
    assert_eq!(
        observed.status_reason,
        Some(SessionContinuationStatusReason::Cancelled)
    );
}

#[test]
fn failed_or_cancelled_continuation_can_be_requeued_for_retry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-continuation-requeue");
    store
        .upsert_session(&sample_session(session_id.clone()))
        .expect("session");

    let failed = store
        .enqueue_continuation(&SessionContinuationInput::new(
            session_id.clone(),
            SessionContinuationKind::Retry,
        ))
        .expect("queued");
    let failed_claim = store
        .claim_next_continuation(
            &SessionContinuationClaim::for_session(session_id.clone()).with_lease_owner("worker-a"),
        )
        .expect("claim")
        .expect("claimed");
    store
        .fail_continuation(&failed_claim.continuation_id, "provider unavailable")
        .expect("fail");

    let requeued = store
        .requeue_continuation(
            &failed.continuation_id,
            "retry after provider cooldown",
            json!({"retry_after_seconds": 30}),
        )
        .expect("requeue")
        .expect("requeued");
    assert_eq!(requeued.status, SessionContinuationStatus::Queued);
    assert_eq!(requeued.attempt_count, 1);
    assert_eq!(
        requeued.error.as_deref(),
        Some("retry after provider cooldown")
    );
    assert_eq!(requeued.payload["retry_after_seconds"], json!(30));

    let cancelled = store
        .enqueue_continuation(&SessionContinuationInput::new(
            session_id.clone(),
            SessionContinuationKind::Compact,
        ))
        .expect("cancel queued");
    store
        .cancel_continuation(&cancelled.continuation_id, "operator cancelled")
        .expect("cancel");
    let requeued_cancel = store
        .requeue_continuation(
            &cancelled.continuation_id,
            "operator resumed",
            serde_json::Value::Null,
        )
        .expect("requeue cancel")
        .expect("requeued");
    assert_eq!(requeued_cancel.status, SessionContinuationStatus::Queued);
    assert_eq!(requeued_cancel.error.as_deref(), Some("operator resumed"));
}

#[test]
fn requeue_continuation_returns_none_for_non_requeueable_states() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-continuation-requeue-noop");
    store
        .upsert_session(&sample_session(session_id.clone()))
        .expect("session");

    let queued = store
        .enqueue_continuation(&SessionContinuationInput::new(
            session_id.clone(),
            SessionContinuationKind::FollowUp,
        ))
        .expect("queued");
    assert!(
        store
            .requeue_continuation(&queued.continuation_id, "already queued", json!({}))
            .expect("requeue queued")
            .is_none()
    );

    let completed = store
        .enqueue_continuation(&SessionContinuationInput::new(
            session_id.clone(),
            SessionContinuationKind::Retry,
        ))
        .expect("completed queued");
    let claimed = store
        .claim_next_continuation(
            &SessionContinuationClaim::for_session(session_id)
                .with_kinds([SessionContinuationKind::Retry])
                .with_lease_owner("worker-a"),
        )
        .expect("claim")
        .expect("claimed");
    assert_eq!(claimed.continuation_id, completed.continuation_id);
    store
        .complete_continuation(&completed.continuation_id, json!({"entry_id": "leaf-a"}))
        .expect("complete");

    assert!(
        store
            .requeue_continuation(
                &completed.continuation_id,
                "completed should not requeue",
                json!({})
            )
            .expect("requeue completed")
            .is_none()
    );
    let unchanged = store
        .continuations(&SessionId::from("session-continuation-requeue-noop"))
        .expect("continuations");
    assert_eq!(unchanged[0].status, SessionContinuationStatus::Queued);
    assert_eq!(unchanged[1].status, SessionContinuationStatus::Completed);
}

#[test]
fn session_search_uses_fts_trigram_and_substring_indexes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-search");
    let other_session_id = SessionId::from("session-other");
    let turn_id = TurnId::from("turn-search");
    store
        .upsert_session(&sample_session(session_id.clone()))
        .expect("session");
    store
        .upsert_session(&sample_session(other_session_id.clone()))
        .expect("other session");

    let english = sample_entry(
        session_id.clone(),
        turn_id.clone(),
        SessionEntryKind::UserMessage,
        "Prefer concise project search notes.",
    );
    let chinese = sample_entry(
        session_id.clone(),
        turn_id.clone(),
        SessionEntryKind::AssistantMessage,
        "中文搜索体验需要支持子串匹配。",
    );
    let other = sample_entry(
        other_session_id.clone(),
        turn_id.clone(),
        SessionEntryKind::UserMessage,
        "Prefer concise notes in another session.",
    );
    store.append_entry(&english).expect("english entry");
    store.append_entry(&chinese).expect("chinese entry");
    store.append_entry(&other).expect("other entry");

    let english_hits = store
        .search_entries(
            &SessionSearchQuery::new("concise")
                .for_session(session_id.clone())
                .with_limit(10),
        )
        .expect("english search");
    assert_eq!(english_hits.len(), 1);
    assert_eq!(english_hits[0].entry.entry_id, english.entry_id);
    assert_eq!(english_hits[0].index, SessionSearchIndex::Fts);
    assert!(english_hits[0].snippet.contains("concise"));

    let trigram_hits = store
        .search_entries(
            &SessionSearchQuery::new("搜索体验")
                .for_session(session_id.clone())
                .with_limit(10),
        )
        .expect("trigram search");
    assert_eq!(trigram_hits.len(), 1);
    assert_eq!(trigram_hits[0].entry.entry_id, chinese.entry_id);
    assert_eq!(trigram_hits[0].index, SessionSearchIndex::Trigram);
    assert!(trigram_hits[0].snippet.contains("搜索体验"));

    let short_cjk_hits = store
        .search_entries(
            &SessionSearchQuery::new("中文")
                .for_session(session_id)
                .with_limit(10),
        )
        .expect("short cjk search");
    assert_eq!(short_cjk_hits.len(), 1);
    assert_eq!(short_cjk_hits[0].entry.entry_id, chinese.entry_id);
    assert_eq!(short_cjk_hits[0].index, SessionSearchIndex::Substring);
}

#[test]
fn session_search_sanitizes_fts_query_special_characters() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-search-sanitize");
    let turn_id = TurnId::from("turn-search-sanitize");
    let text = r#"literal query has quotes " plus colon:path and NEAR/token*"#;
    let entry = sample_entry(
        session_id.clone(),
        turn_id,
        SessionEntryKind::AssistantMessage,
        text,
    );
    store.append_entry(&entry).expect("entry");

    let hits = store
        .search_entries(
            &SessionSearchQuery::new(r#"quotes " plus colon:path"#)
                .for_session(session_id)
                .with_limit(5),
        )
        .expect("special character query should not fail");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entry.entry_id, entry.entry_id);
}

#[test]
fn session_search_indexes_turn_writer_entries_on_commit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-writer-search");
    let turn_id = TurnId::from("turn-writer-search");
    let session = sample_session(session_id.clone());
    let entry = sample_entry(
        session_id.clone(),
        turn_id.clone(),
        SessionEntryKind::AssistantMessage,
        "writer committed searchable content",
    );

    let mut writer = store.begin_turn(&session, &turn_id).expect("writer");
    writer.append_entry(&entry).expect("entry");
    writer.commit().expect("commit");

    let hits = store
        .search_entries(
            &SessionSearchQuery::new("searchable")
                .for_session(session_id)
                .with_limit(5),
        )
        .expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entry.entry_id, entry.entry_id);
}

#[test]
fn session_search_does_not_index_rolled_back_writer_entries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-writer-search-rollback");
    let turn_id = TurnId::from("turn-writer-search-rollback");
    let session = sample_session(session_id.clone());
    let entry = sample_entry(
        session_id.clone(),
        turn_id.clone(),
        SessionEntryKind::AssistantMessage,
        "rolled back searchable content",
    );

    let mut writer = store.begin_turn(&session, &turn_id).expect("writer");
    writer.append_entry(&entry).expect("entry");
    writer.rollback().expect("rollback");

    let hits = store
        .search_entries(
            &SessionSearchQuery::new("searchable")
                .for_session(session_id)
                .with_limit(5),
        )
        .expect("search");
    assert!(hits.is_empty());
}

#[test]
fn sqlite_store_round_trips_model_diagnostic_event() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("session-diag");
    let mut session = SessionRecord::new(session_id.clone(), SessionSource::Cli);
    session.agent_id = Some("build".into());
    store.upsert_session(&session).expect("session");

    let turn_id = TurnId::from("turn-diag");
    let diagnostic = ModelRequestDiagnostic {
        kind: "fallback_provider_failed".into(),
        message: "provider moonshot-kimi/kimi-k2.6 fallback attempt 1 failed with rate_limit error"
            .into(),
        parameter: None,
    };
    let event = AgentEvent::new(
        session_id.clone(),
        turn_id,
        None,
        AgentEventSource::Model,
        AgentEventKind::ModelDiagnostic(diagnostic.clone()),
        json!({"iteration": 1}),
    );
    store
        .append_agent_event(&event)
        .expect("append diagnostic event");

    let replay = store
        .replay_session(&session_id)
        .expect("replay")
        .expect("session exists");
    assert_eq!(replay.agent_events.len(), 1);
    match &replay.agent_events[0].kind {
        AgentEventKind::ModelDiagnostic(stored) => assert_eq!(stored, &diagnostic),
        other => panic!("expected ModelDiagnostic, got {other:?}"),
    }
}
#[test]
fn sqlite_store_round_trips_tool_result_continuation() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::new();
    let turn_id = TurnId::new();

    // enqueue a ToolResult continuation
    let mut input =
        SessionContinuationInput::new(session_id.clone(), SessionContinuationKind::ToolResult);
    input.turn_id = Some(turn_id.clone());
    input.payload = json!({"tool_name": "fs_read", "tool_input": {"path": "/tmp/test.txt"}});
    let enqueued = store
        .enqueue_continuation(&input)
        .expect("enqueue ToolResult");
    assert_eq!(enqueued.kind, SessionContinuationKind::ToolResult);
    assert_eq!(enqueued.status, SessionContinuationStatus::Queued);

    // claim it — must succeed
    let claim = SessionContinuationClaim::for_session(session_id.clone())
        .with_kinds([SessionContinuationKind::ToolResult])
        .with_lease_owner("test_worker");
    let claimed = store
        .claim_next_continuation(&claim)
        .expect("claim")
        .expect("continuation exists");
    assert_eq!(claimed.continuation_id, enqueued.continuation_id);
    assert_eq!(claimed.kind, SessionContinuationKind::ToolResult);
    assert_eq!(claimed.status, SessionContinuationStatus::Running);

    // complete it
    let completed = store
        .complete_continuation(
            &claimed.continuation_id,
            json!({"tool_name": "fs_read", "ok": true}),
        )
        .expect("complete")
        .expect("continuation still exists");
    assert_eq!(completed.status, SessionContinuationStatus::Completed);

    // query continuations for this session
    let list = store.continuations(&session_id).expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].kind, SessionContinuationKind::ToolResult);
}

#[test]
fn sqlite_operational_report_exposes_wal_search_and_write_policy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("sqlite-operational-session");
    let turn_id = TurnId::from("sqlite-operational-turn");
    store
        .append_entry(&sample_entry(
            session_id,
            turn_id,
            SessionEntryKind::UserMessage,
            "operational report searchable text",
        ))
        .expect("seed entry");

    let report = store.operational_report().expect("operational report");

    assert!(report.schema_version > 0);
    assert_eq!(report.journal_mode, "wal");
    assert!(report.integrity_check.ok);
    assert_eq!(report.integrity_check.messages, vec!["ok"]);
    assert_eq!(report.write_policy.transaction_begin, "BEGIN IMMEDIATE");
    assert!(report.write_policy.busy_timeout_ms >= 1000);
    assert!(report.write_policy.busy_retry_attempts > 0);
    assert!(report.write_policy.retry_jitter_ms > 0);
    assert!(
        report
            .search_indexes
            .iter()
            .any(|index| index.name == "session_entries_fts"
                && index.index == SessionSearchIndex::Fts
                && index.available)
    );
    assert!(
        report
            .search_indexes
            .iter()
            .any(|index| index.name == "session_entries_trigram"
                && index.index == SessionSearchIndex::Trigram
                && index.available)
    );
    assert!(report.wal_checkpoint.log_frames >= report.wal_checkpoint.checkpointed_frames);
}

#[test]
fn sqlite_repair_to_writes_integrity_checked_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path().join("state"));
    let session_id = SessionId::from("sqlite-repair-session");
    let turn_id = TurnId::from("sqlite-repair-turn");
    store
        .append_entry(&sample_entry(
            session_id,
            turn_id,
            SessionEntryKind::UserMessage,
            "repair artifact searchable text",
        ))
        .expect("seed entry");
    let destination = temp.path().join("repair/state-repair.db");

    let repair = store.repair_to(&destination).expect("repair artifact");

    assert_eq!(repair.path, destination);
    assert!(repair.created);
    assert!(repair.integrity_check.ok);
    assert_eq!(repair.integrity_check.messages, vec!["ok"]);
    assert!(destination.is_file());

    let repaired_report = SqliteSessionStore::from_file(destination)
        .operational_report()
        .expect("open repaired artifact");
    assert!(repaired_report.integrity_check.ok);
}

#[test]
fn sqlite_prune_ended_sessions_removes_timeline_artifacts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let old_session_id = SessionId::from("sqlite-prune-old-session");
    let active_session_id = SessionId::from("sqlite-prune-active-session");
    let old_turn_id = TurnId::from("sqlite-prune-old-turn");
    let active_turn_id = TurnId::from("sqlite-prune-active-turn");
    let old_ended_at = OffsetDateTime::now_utc() - time::Duration::days(10);
    let cutoff = OffsetDateTime::now_utc() - time::Duration::days(2);

    store
        .upsert_session(&sample_session(old_session_id.clone()))
        .expect("old session");
    store
        .append_entry(&sample_entry(
            old_session_id.clone(),
            old_turn_id.clone(),
            SessionEntryKind::UserMessage,
            "old prune searchable text",
        ))
        .expect("old entry");
    for event in sample_events(old_session_id.clone(), old_turn_id.clone()) {
        store.append_agent_event(&event).expect("old event");
    }
    store
        .append_approval(&sample_approval(
            old_session_id.clone(),
            old_turn_id.clone(),
        ))
        .expect("old approval");
    let mut continuation =
        SessionContinuationInput::new(old_session_id.clone(), SessionContinuationKind::NextTurn);
    continuation.turn_id = Some(old_turn_id.clone());
    store
        .enqueue_continuation(&continuation)
        .expect("old continuation");
    store
        .finish_session(&old_session_id, old_ended_at)
        .expect("finish old session");

    store
        .upsert_session(&sample_session(active_session_id.clone()))
        .expect("active session");
    store
        .append_entry(&sample_entry(
            active_session_id.clone(),
            active_turn_id,
            SessionEntryKind::UserMessage,
            "active prune searchable text",
        ))
        .expect("active entry");

    let report = store
        .prune_ended_sessions_before(cutoff)
        .expect("prune old sessions");

    assert_eq!(report.sessions_pruned, 1);
    assert_eq!(report.entries_pruned, 1);
    assert_eq!(report.agent_events_pruned, 3);
    assert_eq!(report.approvals_pruned, 1);
    assert_eq!(report.timeline_items_pruned, 5);
    assert_eq!(report.continuations_pruned, 1);
    assert_eq!(
        store.get_session(&old_session_id).expect("old lookup"),
        None
    );
    assert!(
        store
            .get_session(&active_session_id)
            .expect("active lookup")
            .is_some()
    );
    assert!(
        store
            .search_entries(
                &SessionSearchQuery::new("old prune searchable text")
                    .for_session(old_session_id)
                    .with_limit(5),
            )
            .expect("old search")
            .is_empty()
    );
    assert_eq!(
        store
            .search_entries(
                &SessionSearchQuery::new("active prune searchable text")
                    .for_session(active_session_id)
                    .with_limit(5),
            )
            .expect("active search")
            .len(),
        1
    );
}
