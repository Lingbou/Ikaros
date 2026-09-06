from __future__ import annotations

import sqlite3
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest

import ikaros_runtime.storage.store as store_module
from ikaros_runtime.domain import (
    JOURNAL_EVENT_SCHEMA_VERSION,
    JournalEvent,
    ModelUsage,
    SkillDescriptor,
    WorkspaceSummary,
)
from ikaros_runtime.errors import UnsupportedJournalEventVersionError
from ikaros_runtime.json_codec import dumps as json_dumps
from ikaros_runtime.json_codec import loads as json_loads
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools.core import ToolDefinition

from .helpers import prepare_turn


def _utc_timestamp(value: datetime) -> str:
    return value.astimezone(UTC).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def _projection_snapshot(
    store: SqliteRuntimeStore,
) -> dict[str, tuple[tuple[object, ...], ...]]:
    projection_tables = (
        ("threads", "id"),
        ("branches", "id"),
        ("turns", "id"),
        ("runs", "id"),
        ("run_configs", "run_id"),
        ("items", "id"),
        ("model_calls", "run_id, step_ordinal"),
        ("model_usages", "run_id, step_ordinal"),
    )
    return {
        table: tuple(
            tuple(row)
            for row in store._connection.execute(f"SELECT * FROM {table} ORDER BY {order}")
        )
        for table, order in projection_tables
    }


def _journal_snapshot(
    store: SqliteRuntimeStore,
) -> tuple[tuple[JournalEvent, ...], int]:
    events, latest_seq = store.replay_events(0, 10_000)
    return tuple(events), latest_seq


def _finish_model_step_with_usage(
    store: SqliteRuntimeStore,
    run_id: str,
    *,
    step_ordinal: int,
    usage: ModelUsage,
) -> JournalEvent:
    store.prepare_model_step(run_id, step_ordinal=step_ordinal)
    completed = store.complete_provider_step(
        run_id,
        step_ordinal=step_ordinal,
        assistant_item_id=None,
        tool_calls=(),
        reasoning_content=None,
        usage=usage,
        response_model_id=None,
        request_id=None,
    )
    return completed.events[-1]


def test_projections_can_be_rebuilt_from_the_event_journal(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        expected, event = store.create_thread("Rebuild me")
        assert event.seq == 1

        store.rebuild_projections()

        assert store.list_thread_page(cursor=None, limit=50).threads == (expected,)
        replayed, latest_seq = store.replay_events(0, 100)
        assert replayed == [event]
        assert replayed[0].schema_version == JOURNAL_EVENT_SCHEMA_VERSION
        assert replayed[0].to_wire()["schemaVersion"] == JOURNAL_EVENT_SCHEMA_VERSION
        assert latest_seq == 1
    finally:
        store.close()


def test_model_response_finished_usage_projection_and_rebuild_are_lossless(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Usage projection")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="measure this step",
            provider_id="deepseek",
            model_id="deepseek-chat",
        )
        store.mark_run_running(prepared.run_id)
        event = _finish_model_step_with_usage(
            store,
            prepared.run_id,
            step_ordinal=1,
            usage=ModelUsage(
                input_tokens=13,
                cached_input_tokens=8,
                output_tokens=5,
                reasoning_output_tokens=2,
                total_tokens=18,
            ),
        )
        assert event.type == "model.response_finished"
        assert event.payload["providerId"] == "deepseek"
        assert event.payload["modelId"] == "deepseek-chat"
        assert event.payload["outcome"] == "completed"
        assert event.payload["usage"] == {
            "inputTokens": 13,
            "cachedInputTokens": 8,
            "outputTokens": 5,
            "reasoningOutputTokens": 2,
            "totalTokens": 18,
        }
        before = tuple(
            store._connection.execute(
                "SELECT * FROM model_usages WHERE run_id = ?",
                (prepared.run_id,),
            ).fetchone()
        )
        journal_before, latest_seq = store.replay_events(0, 100)

        store.rebuild_projections()

        after = tuple(
            store._connection.execute(
                "SELECT * FROM model_usages WHERE run_id = ?",
                (prepared.run_id,),
            ).fetchone()
        )
        journal_after, rebuilt_latest_seq = store.replay_events(0, 100)
        assert after == before
        assert journal_after == journal_before
        assert rebuilt_latest_seq == latest_seq
    finally:
        store.close()


def test_projection_rebuild_rejects_model_response_after_run_settlement(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Late usage")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="settle first",
            provider_id="deepseek",
            model_id="deepseek-chat",
        )
        store.mark_run_running(prepared.run_id)
        store.prepare_model_step(prepared.run_id, step_ordinal=1)
        store.complete_provider_step(
            prepared.run_id,
            step_ordinal=1,
            assistant_item_id=None,
            tool_calls=(),
            reasoning_content=None,
            usage=None,
            response_model_id=None,
            request_id=None,
        )
        settled_at = store.terminalize_run(prepared.run_id, "completed")[-1].timestamp
        payload = {
            "stepOrdinal": 1,
            "providerId": "deepseek",
            "modelId": "deepseek-chat",
            "outcome": "completed",
            "reasonCode": None,
            "responseModelId": None,
            "requestId": None,
            "activityDate": datetime.now().astimezone().date().isoformat(),
            "usage": {
                "inputTokens": 1,
                "cachedInputTokens": None,
                "outputTokens": 1,
                "reasoningOutputTokens": None,
                "totalTokens": 2,
            },
            "finishedAt": settled_at,
            "turnId": prepared.turn_id,
            "runId": prepared.run_id,
        }
        with store._connection:
            store._connection.execute(
                """
                INSERT INTO events(
                    schema_version, event_type, thread_id, branch_id, turn_id, run_id,
                    created_at, payload_json
                ) VALUES (?, 'model.response_finished', ?, ?, ?, ?, ?, ?)
                """,
                (
                    JOURNAL_EVENT_SCHEMA_VERSION,
                    thread.id,
                    thread.default_branch_id,
                    prepared.turn_id,
                    prepared.run_id,
                    settled_at,
                    json_dumps(payload, separators=(",", ":"), ensure_ascii=False),
                ),
            )

        with pytest.raises(RuntimeError, match="model response completion requires a running Run"):
            store.rebuild_projections()
    finally:
        store.close()


def test_model_step_state_machine_rejects_invalid_calls_without_appending_events(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Usage lifecycle")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="measure once",
            provider_id="deepseek",
            model_id="deepseek-chat",
        )
        usage = ModelUsage(input_tokens=2, output_tokens=1, total_tokens=3)

        latest_seq = store.latest_sequence()
        with pytest.raises(RuntimeError, match="input preparation requires a running Run"):
            store.prepare_model_step(prepared.run_id, step_ordinal=1)
        assert store.latest_sequence() == latest_seq

        store.mark_run_running(prepared.run_id)
        latest_seq = store.latest_sequence()
        with pytest.raises(RuntimeError, match="no prepared Step"):
            store.complete_provider_step(
                prepared.run_id,
                step_ordinal=1,
                assistant_item_id=None,
                tool_calls=(),
                reasoning_content=None,
                usage=usage,
                response_model_id=None,
                request_id=None,
            )
        assert store.latest_sequence() == latest_seq

        with pytest.raises(RuntimeError, match="ordinal is not contiguous"):
            store.prepare_model_step(prepared.run_id, step_ordinal=2)
        assert store.latest_sequence() == latest_seq

        store.prepare_model_step(prepared.run_id, step_ordinal=1)
        latest_seq = store.latest_sequence()
        with pytest.raises(RuntimeError, match="unfinished model Step"):
            store.prepare_model_step(prepared.run_id, step_ordinal=1)
        assert store.latest_sequence() == latest_seq

        store.complete_provider_step(
            prepared.run_id,
            step_ordinal=1,
            assistant_item_id=None,
            tool_calls=(),
            reasoning_content=None,
            usage=usage,
            response_model_id=None,
            request_id=None,
        )
        latest_seq = store.latest_sequence()
        with pytest.raises(RuntimeError, match="already finished"):
            store.complete_provider_step(
                prepared.run_id,
                step_ordinal=1,
                assistant_item_id=None,
                tool_calls=(),
                reasoning_content=None,
                usage=usage,
                response_model_id=None,
                request_id=None,
            )
        assert store.latest_sequence() == latest_seq

        store.terminalize_run(prepared.run_id, "completed")
        latest_seq = store.latest_sequence()
        with pytest.raises(RuntimeError, match="input preparation requires a running Run"):
            store.prepare_model_step(prepared.run_id, step_ordinal=2)
        assert store.latest_sequence() == latest_seq
    finally:
        store.close()


def test_projection_rebuild_accepts_queued_notification_then_single_running_transition(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Valid Run lifecycle")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="run once",
            provider_id="scripted",
            model_id="scripted-v1",
        )

        queued_projection = _projection_snapshot(store)
        queued_journal = _journal_snapshot(store)
        store.rebuild_projections()
        assert _projection_snapshot(store) == queued_projection
        assert _journal_snapshot(store) == queued_journal

        store.mark_run_running(prepared.run_id)
        running_projection = _projection_snapshot(store)
        running_journal = _journal_snapshot(store)
        store.rebuild_projections()
        assert _projection_snapshot(store) == running_projection
        assert _journal_snapshot(store) == running_journal
        assert store.run_status(prepared.run_id) == "running"
    finally:
        store.close()


@pytest.mark.parametrize(
    ("prior_state", "invalid_status", "message"),
    (
        ("queued", "queued", "queued Run notification does not match initial state"),
        ("running", "queued", "queued Run notification does not match initial state"),
        ("running", "running", "only transition once from queued to running"),
        ("settled", "running", "only transition once from queued to running"),
    ),
)
def test_projection_rebuild_rejects_invalid_run_state_transitions_atomically(
    tmp_path: Path,
    prior_state: str,
    invalid_status: str,
    message: str,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread(f"Invalid lifecycle {prior_state}")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="reject lifecycle corruption",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        if prior_state in {"running", "settled"}:
            store.mark_run_running(prepared.run_id)
        if prior_state == "settled":
            store.terminalize_run(prepared.run_id, "completed")

        timestamp = _utc_timestamp(datetime.now(UTC))
        with store._connection:
            store._append_event(
                event_type="run.state_changed",
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                turn_id=prepared.turn_id,
                run_id=prepared.run_id,
                timestamp=timestamp,
                payload={"status": invalid_status},
            )
        projection_before = _projection_snapshot(store)
        journal_before = _journal_snapshot(store)

        with pytest.raises(RuntimeError, match=message):
            store.rebuild_projections()

        assert _projection_snapshot(store) == projection_before
        assert _journal_snapshot(store) == journal_before
    finally:
        store.close()


@pytest.mark.parametrize("active_item_kind", ("message", "tool_call"))
def test_projection_rebuild_rejects_settlement_before_active_item_completion(
    tmp_path: Path,
    active_item_kind: str,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread(f"Active {active_item_kind}")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="do not synthesize completion",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(prepared.run_id)
        if active_item_kind == "message":
            item_id, _ = store.create_assistant_item(prepared.run_id)
        else:
            item_id, _ = store.create_tool_call_item(
                prepared.run_id,
                step_id="step-active",
                call_id="call-active",
                tool_name="process_run",
                arguments={"command": "echo active"},
            )

        timestamp = _utc_timestamp(datetime.now(UTC))
        with store._connection:
            store._append_event(
                event_type="run.settled",
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                turn_id=prepared.turn_id,
                run_id=prepared.run_id,
                timestamp=timestamp,
                payload={"status": "cancelled", "settledAt": timestamp},
            )
        projection_before = _projection_snapshot(store)
        journal_before = _journal_snapshot(store)

        with pytest.raises(RuntimeError, match="cannot settle with an active Item"):
            store.rebuild_projections()

        assert _projection_snapshot(store) == projection_before
        assert _journal_snapshot(store) == journal_before
        item_events = tuple(event.type for event in journal_before[0] if event.item_id == item_id)
        assert item_events == ("item.started",)
    finally:
        store.close()


def test_usage_aggregation_uses_local_activity_dates_and_actual_run_start(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    local_now = datetime.now().astimezone().replace(microsecond=0)
    active_days = [local_now.date() - timedelta(days=offset) for offset in (2, 1, 0)]
    timestamp = _utc_timestamp(local_now)
    monkeypatch.setattr(store_module, "utc_now", lambda: timestamp)
    try:
        for index, (activity_day, token_counts) in enumerate(
            zip(active_days, ((4,), (6,), (2, 8)), strict=True),
        ):
            thread, _ = store.create_thread(f"Usage day {index}")
            prepared = prepare_turn(
                store,
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                content="measure",
                provider_id="deepseek",
                model_id="deepseek-chat",
            )
            started_local = datetime.combine(
                activity_day,
                datetime.min.time(),
                tzinfo=local_now.tzinfo,
            ).replace(hour=10)
            timestamp = _utc_timestamp(started_local)
            store.mark_run_running(prepared.run_id)
            for step_ordinal, tokens in enumerate(token_counts, start=1):
                timestamp = _utc_timestamp(started_local + timedelta(seconds=step_ordinal))
                _finish_model_step_with_usage(
                    store,
                    prepared.run_id,
                    step_ordinal=step_ordinal,
                    usage=ModelUsage(
                        input_tokens=tokens - 1,
                        output_tokens=1,
                        total_tokens=tokens,
                    ),
                )
            duration = 125 if index == 0 else 30 + index
            timestamp = _utc_timestamp(started_local + timedelta(seconds=duration))
            store.terminalize_run(prepared.run_id, "completed")

        interrupted_thread, _ = store.create_thread("Interrupted startup recovery")
        interrupted = prepare_turn(
            store,
            thread_id=interrupted_thread.id,
            branch_id=interrupted_thread.default_branch_id,
            content="do not count synthetic recovery time",
            provider_id="deepseek",
            model_id="deepseek-chat",
        )
        interrupted_started = local_now - timedelta(seconds=1_000)
        timestamp = _utc_timestamp(interrupted_started)
        store.mark_run_running(interrupted.run_id)
        timestamp = _utc_timestamp(local_now)
        store.terminalize_run(
            interrupted.run_id,
            "failed",
            reason_code="runtime_interrupted",
        )

        snapshot = store.read_usage()

        assert snapshot.summary.lifetime_tokens == 20
        assert snapshot.summary.peak_daily_tokens == 10
        assert snapshot.summary.longest_running_turn_sec == 125
        assert snapshot.summary.current_streak_days == 3
        assert snapshot.summary.longest_streak_days == 3
        assert [(bucket.start_date, bucket.tokens) for bucket in snapshot.daily_usage_buckets] == [
            (active_days[0].isoformat(), 4),
            (active_days[1].isoformat(), 6),
            (active_days[2].isoformat(), 10),
        ]
    finally:
        store.close()


def test_empty_usage_snapshot_does_not_invent_token_totals(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        snapshot = store.read_usage()
        assert snapshot.to_wire() == {
            "summary": {
                "lifetimeTokens": None,
                "peakDailyTokens": None,
                "longestRunningTurnSec": None,
                "currentStreakDays": 0,
                "longestStreakDays": 0,
            },
            "dailyUsageBuckets": [],
        }
    finally:
        store.close()


def test_projection_rebuild_rejects_noncanonical_thread_payload_without_data_loss(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        expected, event = store.create_thread("Canonical payload")
        payload = dict(event.payload)
        thread_payload = dict(payload["thread"])
        del thread_payload["archivedAt"]
        payload["thread"] = thread_payload
        with store._connection:
            store._connection.execute(
                "UPDATE events SET payload_json = ? WHERE seq = ?",
                (json_dumps(payload, ensure_ascii=False, separators=(",", ":")), event.seq),
            )

        with pytest.raises(RuntimeError, match="payload shape is invalid"):
            store.rebuild_projections()

        assert store.list_thread_page(cursor=None, limit=50).threads == (expected,)
    finally:
        store.close()


def test_future_journal_event_versions_are_rejected_by_replay_and_rebuild(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        _thread, event = store.create_thread("Future event")
        with store._connection:
            store._connection.execute(
                "UPDATE events SET schema_version = ? WHERE seq = ?",
                (JOURNAL_EVENT_SCHEMA_VERSION + 1, event.seq),
            )

        message = "does not match"
        with pytest.raises(UnsupportedJournalEventVersionError, match=message):
            store.replay_events(0, 100)
        with pytest.raises(UnsupportedJournalEventVersionError, match=message):
            store.rebuild_projections()
    finally:
        store.close()


def test_projection_rebuild_rejects_lifecycle_event_semantics_that_disagree_with_payload(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _created = store.create_thread("Before rename")
        renamed, rename_event = store.rename_thread(thread.id, "After rename")
        assert rename_event is not None
        with store._connection:
            store._connection.execute(
                "UPDATE events SET event_type = 'thread.archived' WHERE seq = ?",
                (rename_event.seq,),
            )

        with pytest.raises(RuntimeError, match="thread.archived payload state is invalid"):
            store.rebuild_projections()

        assert store.list_thread_page(cursor=None, limit=50).threads == (renamed,)
    finally:
        store.close()


def test_projection_rebuild_rejects_completed_item_immutable_field_changes(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _created = store.create_thread("Item integrity")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="hello",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(prepared.run_id)
        item_id, _started = store.create_assistant_item(prepared.run_id)
        store.append_text_delta(item_id, "world")
        terminal_events = store.terminalize_run(prepared.run_id, "completed")
        item_event = terminal_events[0]
        payload = dict(item_event.payload)
        item_payload = dict(payload["item"])
        item_payload["kind"] = "tool_call"
        payload["item"] = item_payload
        projection_before = [
            tuple(row) for row in store._connection.execute("SELECT * FROM items ORDER BY id")
        ]
        with store._connection:
            store._connection.execute(
                "UPDATE events SET payload_json = ? WHERE seq = ?",
                (
                    json_dumps(payload, ensure_ascii=False, separators=(",", ":")),
                    item_event.seq,
                ),
            )

        with pytest.raises(RuntimeError, match="immutable Item fields"):
            store.rebuild_projections()

        assert [
            tuple(row) for row in store._connection.execute("SELECT * FROM items ORDER BY id")
        ] == projection_before
    finally:
        store.close()


def test_thread_workspace_survives_list_reload_and_projection_rebuild(tmp_path: Path) -> None:
    database_path = tmp_path / "state.db"
    workspace_root = tmp_path / "research"
    workspace_root.mkdir()
    workspace = WorkspaceSummary("workspace-research", "Research", str(workspace_root.resolve()))
    store = SqliteRuntimeStore(database_path)
    try:
        expected, event, created = store.create_thread_once(
            "Workspace thread",
            "workspace-create",
            workspace=workspace,
        )

        assert created is True
        assert expected.workspace == workspace
        assert event.payload["thread"]["workspace"] == {
            "id": "workspace-research",
            "name": "Research",
            "rootUri": str(workspace_root.resolve()),
        }
        assert store.list_thread_page(cursor=None, limit=50).threads == (expected,)

        store.rebuild_projections()
        assert store.list_thread_page(cursor=None, limit=50).threads == (expected,)
    finally:
        store.close()

    reloaded = SqliteRuntimeStore(database_path)
    try:
        assert reloaded.list_thread_page(cursor=None, limit=50).threads == (expected,)
        repeated, repeated_event, repeated_created = reloaded.create_thread_once(
            "Workspace thread",
            "workspace-create",
            workspace=workspace,
        )
        assert repeated == expected
        assert repeated_event == event
        assert repeated_created is False
        with pytest.raises(LookupError, match="different thread.create parameters"):
            reloaded.create_thread_once(
                "Workspace thread",
                "workspace-create",
                workspace=WorkspaceSummary("workspace-other", "Other", None),
            )
    finally:
        reloaded.close()


def test_thread_rename_archive_and_unarchive_survive_projection_rebuild(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        created, _created_event = store.create_thread("Original")

        renamed, renamed_event = store.rename_thread(created.id, "Renamed")
        assert renamed.title == "Renamed"
        assert renamed.archived_at is None
        assert renamed_event is not None
        assert renamed_event.type == "thread.renamed"
        assert renamed_event.payload["thread"] == renamed.to_wire()

        archived, archived_event = store.set_thread_archived(created.id, archived=True)
        assert archived.archived_at is not None
        assert archived_event is not None
        assert archived_event.type == "thread.archived"
        assert store.list_thread_page(cursor=None, limit=50).threads == ()
        assert store.list_thread_page(cursor=None, limit=50, archived=True).threads == (archived,)

        sequence_before_noop = store.latest_sequence()
        repeated, repeated_event = store.set_thread_archived(created.id, archived=True)
        assert repeated == archived
        assert repeated_event is None
        assert store.latest_sequence() == sequence_before_noop

        store.rebuild_projections()
        assert store.get_thread(created.id).thread == archived
        assert store.list_thread_page(cursor=None, limit=50).threads == ()
        assert store.list_thread_page(cursor=None, limit=50, archived=True).threads == (archived,)

        restored, restored_event = store.set_thread_archived(created.id, archived=False)
        assert restored.archived_at is None
        assert restored_event is not None
        assert restored_event.type == "thread.unarchived"
        assert store.list_thread_page(cursor=None, limit=50).threads == (restored,)
        assert store.list_thread_page(cursor=None, limit=50, archived=True).threads == ()
    finally:
        store.close()


def test_archived_threads_cannot_start_turns_and_active_threads_cannot_be_archived(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _created_event = store.create_thread("Lifecycle")
        store.set_thread_archived(thread.id, archived=True)
        with pytest.raises(LookupError, match="thread is archived"):
            prepare_turn(
                store,
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                content="must not run",
                provider_id="scripted",
                model_id="scripted-v1",
            )

        restored, _restored_event = store.set_thread_archived(thread.id, archived=False)
        prepare_turn(
            store,
            thread_id=restored.id,
            branch_id=restored.default_branch_id,
            content="now run",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        with pytest.raises(LookupError, match="active run cannot be archived"):
            store.set_thread_archived(thread.id, archived=True)
    finally:
        store.close()


def test_run_descriptor_inherits_its_thread_workspace(tmp_path: Path) -> None:
    workspace = WorkspaceSummary("workspace-run", "Run Workspace", None)
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Workspace run", workspace=workspace)
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="use workspace",
            provider_id="scripted",
            model_id="scripted-v1",
        )

        assert store.get_run(prepared.run_id).workspace == workspace
    finally:
        store.close()


def test_credential_conflict_scan_uses_dynamic_projections_not_fixed_schema(
    tmp_path: Path,
) -> None:
    protected = "split-projection-credential-sentinel"
    split_at = len(protected) // 2
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Safe thread")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="safe user content",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(prepared.run_id)
        assistant_item_id, _ = store.create_assistant_item(prepared.run_id)
        store.append_text_delta(assistant_item_id, protected[:split_at])
        store.append_text_delta(assistant_item_id, protected[split_at:])
        tool_item_id, _ = store.create_tool_call_item(
            prepared.run_id,
            step_id="step-safe",
            call_id="call-safe",
            tool_name="process_run",
            arguments={"command": "echo safe"},
        )
        store.complete_tool_call(
            tool_item_id,
            status="completed",
            result={
                "toolCallId": "call-safe",
                "toolName": "process_run",
                "ok": True,
                "output": "safe",
                "cancelled": False,
                "stdout": "safe",
                "stderr": "",
                "exitCode": 0,
                "durationMs": 1,
                "timedOut": False,
                "truncated": False,
            },
            result_content='{"output":"safe"}',
        )

        assert store.journal_contains_protected_values((protected,)) is True
        assert store.journal_contains_protected_values(("full_access",)) is False
        assert store.journal_contains_protected_values(("stdout",)) is False
        assert store.journal_contains_protected_values(("item.completed",)) is False
    finally:
        store.close()


def test_credential_conflict_scan_includes_thread_workspace(tmp_path: Path) -> None:
    protected = "workspace-credential-sentinel"
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        store.create_thread(
            "Workspace credential",
            workspace=WorkspaceSummary("workspace-safe", protected, None),
        )

        assert store.journal_contains_protected_values((protected,)) is True
    finally:
        store.close()


def test_credential_conflict_scan_ignores_runtime_owned_model_input_provenance(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Safe model input")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="safe user content",
            provider_id="scripted",
            model_id="scripted-v1",
            tools=(
                ToolDefinition(
                    name="runtime-owned-tool",
                    description="Runtime owned Tool description",
                    input_schema={
                        "type": "object",
                        "properties": {"command": {"type": "string"}},
                    },
                ),
            ),
        )
        store.mark_run_running(prepared.run_id)
        store.prepare_model_step(prepared.run_id, step_ordinal=1)

        for protected in (
            "full_access",
            "runtime_instruction",
            "release",
            "output-style",
            "bounded",
            "runtime-owned-tool",
            "Runtime owned Tool description",
            "object",
            "command",
            "string",
        ):
            assert store.journal_contains_protected_values((protected,)) is False
    finally:
        store.close()


@pytest.mark.parametrize(
    ("provider_id", "model_id", "protected"),
    (
        ("provider-credential-sentinel", "safe-model", "provider-credential-sentinel"),
        ("safe-provider", "model-credential-sentinel", "model-credential-sentinel"),
    ),
)
def test_credential_conflict_scan_includes_dynamic_provider_and_model_identifiers(
    tmp_path: Path,
    provider_id: str,
    model_id: str,
    protected: str,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Dynamic Provider")
        prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="safe user content",
            provider_id=provider_id,
            model_id=model_id,
        )

        assert store.journal_contains_protected_values((protected,)) is True
    finally:
        store.close()


@pytest.mark.parametrize(
    "skill",
    (
        SkillDescriptor("skill-credential-sentinel", "Safe description", "C:/safe/SKILL.md"),
        SkillDescriptor(
            "safe-skill",
            "skill description credential sentinel",
            "C:/safe/SKILL.md",
        ),
        SkillDescriptor(
            "safe-skill",
            "Safe description",
            "C:/skill-location-credential-sentinel/SKILL.md",
        ),
    ),
)
def test_credential_conflict_scan_includes_dynamic_skill_catalog_values(
    tmp_path: Path,
    skill: SkillDescriptor,
) -> None:
    protected = next(
        value for value in (skill.name, skill.description, skill.location) if "credential" in value
    )
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Dynamic Skill")
        prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="safe user content",
            provider_id="scripted",
            model_id="scripted-v1",
            skills=(skill,),
        )

        assert store.journal_contains_protected_values((protected,)) is True
    finally:
        store.close()


def test_credential_conflict_scan_includes_provider_response_identifiers(
    tmp_path: Path,
) -> None:
    response_model_id = "response-model-credential-sentinel"
    request_id = "request-id-credential-sentinel"
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Provider response")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="safe user content",
            provider_id="safe-provider",
            model_id="safe-model",
        )
        store.mark_run_running(prepared.run_id)
        store.prepare_model_step(prepared.run_id, step_ordinal=1)
        store.complete_provider_step(
            prepared.run_id,
            step_ordinal=1,
            assistant_item_id=None,
            tool_calls=(),
            reasoning_content=None,
            usage=None,
            response_model_id=response_model_id,
            request_id=request_id,
        )

        assert store.journal_contains_protected_values((response_model_id,)) is True
        assert store.journal_contains_protected_values((request_id,)) is True
    finally:
        store.close()


def test_client_request_ids_make_thread_and_turn_creation_idempotent(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, created_event, thread_created = store.create_thread_once(
            "Exactly once",
            "create-request",
        )
        repeated_thread, repeated_event, repeated_created = store.create_thread_once(
            "Exactly once",
            "create-request",
        )

        assert thread_created is True
        assert repeated_created is False
        assert repeated_thread == thread
        assert repeated_event == created_event

        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="only one turn",
            provider_id="scripted",
            model_id="scripted-v1",
            client_request_id="turn-request",
        )
        repeated = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="only one turn",
            provider_id="scripted",
            model_id="scripted-v1",
            client_request_id="turn-request",
        )

        assert prepared.newly_created is True
        assert repeated.newly_created is False
        assert repeated.run_id == prepared.run_id
        assert repeated.turn_id == prepared.turn_id
        assert repeated.initial_events == ()
        assert store._connection.execute("SELECT COUNT(*) FROM threads").fetchone()[0] == 1
        assert store._connection.execute("SELECT COUNT(*) FROM turns").fetchone()[0] == 1
        assert store._connection.execute("SELECT COUNT(*) FROM runs").fetchone()[0] == 1

        store.rebuild_projections()
        rebuilt_thread, _, rebuilt_created = store.create_thread_once(
            "Exactly once",
            "create-request",
        )
        rebuilt_turn = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="only one turn",
            provider_id="scripted",
            model_id="scripted-v1",
            client_request_id="turn-request",
        )
        assert rebuilt_thread.id == thread.id
        assert rebuilt_thread.title == thread.title
        assert rebuilt_thread.default_branch_id == thread.default_branch_id
        assert rebuilt_created is False
        assert rebuilt_turn.run_id == prepared.run_id
        assert rebuilt_turn.newly_created is False

        with pytest.raises(LookupError, match="different thread.create parameters"):
            store.create_thread_once("Different title", "create-request")
        with pytest.raises(LookupError, match="different turn.start parameters"):
            prepare_turn(
                store,
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                content="different content",
                provider_id="scripted",
                model_id="scripted-v1",
                client_request_id="turn-request",
            )
    finally:
        store.close()


def test_agent_projections_rebuild_without_rewriting_the_journal(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Agent history")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="hello",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(prepared.run_id)
        item_id, _ = store.create_assistant_item(prepared.run_id)
        store.append_text_delta(item_id, "world")
        terminal_events = store.terminalize_run(prepared.run_id, "completed")
        assert [event.type for event in terminal_events] == ["item.completed", "run.settled"]
        before, latest_seq = store.replay_events(0, 100)

        store.rebuild_projections()

        after, rebuilt_latest_seq = store.replay_events(0, 100)
        assert after == before
        assert rebuilt_latest_seq == latest_seq
        assert store.context_messages(
            thread.default_branch_id,
            through_turn_id=prepared.turn_id,
        ) == [
            ("user", "hello"),
            ("assistant", "world"),
        ]
    finally:
        store.close()


def test_projection_rebuild_rejects_corrupted_submission_instruction_slot(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Corrupt Submission Frame")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="preserve the canonical output slot",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        row = store._connection.execute(
            """
            SELECT seq, payload_json FROM events
            WHERE run_id = ? AND event_type = 'item.completed'
            ORDER BY seq ASC LIMIT 1
            """,
            (prepared.run_id,),
        ).fetchone()
        payload = json_loads(str(row["payload_json"]))
        payload["runConfig"]["instructions"]["outputStyle"]["scope"] = "run"
        with store._connection:
            store._connection.execute(
                "UPDATE events SET payload_json = ? WHERE seq = ?",
                (
                    json_dumps(payload, separators=(",", ":"), ensure_ascii=False),
                    row["seq"],
                ),
            )
        journal_before = _journal_snapshot(store)

        with pytest.raises(RuntimeError, match="initial Turn input snapshots are invalid"):
            store.rebuild_projections()

        assert _journal_snapshot(store) == journal_before
        assert store.run_status(prepared.run_id) == "queued"
    finally:
        store.close()


@pytest.mark.parametrize(
    "corruption",
    (
        "context-group-ghost",
        "context-budget-count",
        "step-ordinal",
        "step-budget-count",
    ),
)
def test_projection_rebuild_rejects_corrupted_model_input_dto(
    tmp_path: Path,
    corruption: str,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Corrupt model input DTO")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="preserve model input invariants",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(prepared.run_id)
        store.prepare_model_step(prepared.run_id, step_ordinal=1)
        row = store._connection.execute(
            """
            SELECT seq, payload_json FROM events
            WHERE run_id = ? AND event_type = 'model.input_prepared'
            ORDER BY seq ASC LIMIT 1
            """,
            (prepared.run_id,),
        ).fetchone()
        payload = json_loads(str(row["payload_json"]))
        if corruption == "context-group-ghost":
            payload["contextRevision"]["historyGroups"][0]["itemIds"][0] = "item_ghost"
        elif corruption == "context-budget-count":
            budget = payload["contextRevision"]["budget"]
            budget["currentRunTokens"] += 1
            budget["totalTokens"] += 1
        elif corruption == "step-ordinal":
            payload["stepInput"]["stepOrdinal"] = 0
        else:
            budget = payload["stepInput"]["budget"]
            budget["currentRunTokens"] += 1
            budget["totalTokens"] += 1
        with store._connection:
            store._connection.execute(
                "UPDATE events SET payload_json = ? WHERE seq = ?",
                (
                    json_dumps(payload, separators=(",", ":"), ensure_ascii=False),
                    row["seq"],
                ),
            )
        journal_before = _journal_snapshot(store)

        with pytest.raises(
            RuntimeError, match="(Context Revision|Step input|StepInput|budget|ordinal)"
        ):
            store.rebuild_projections()

        assert _journal_snapshot(store) == journal_before
        assert store.run_status(prepared.run_id) == "running"
    finally:
        store.close()


def test_terminalize_run_is_atomic_idempotent_and_uniquely_settled(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Atomic terminal state")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="keep partial output",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(prepared.run_id)
        item_id, _ = store.create_assistant_item(prepared.run_id)
        store.append_text_delta(item_id, "partial")
        store._connection.execute(
            """
            CREATE TRIGGER abort_run_settled
            BEFORE INSERT ON events
            WHEN NEW.event_type = 'run.settled'
            BEGIN
                SELECT RAISE(ABORT, 'settled insert failed');
            END;
            """
        )

        with pytest.raises(sqlite3.IntegrityError, match="settled insert failed"):
            store.terminalize_run(prepared.run_id, "cancelled")

        item = store._connection.execute(
            "SELECT status, content FROM items WHERE id = ?", (item_id,)
        ).fetchone()
        assert store.run_status(prepared.run_id) == "running"
        assert dict(item) == {"status": "streaming", "content": "partial"}
        replayed, _ = store.replay_events(0, 100)
        assert not any(
            event.run_id == prepared.run_id
            and (
                event.type == "run.settled"
                or (
                    event.type == "item.completed"
                    and event.payload.get("item", {}).get("role") == "assistant"
                )
            )
            for event in replayed
        )

        store._connection.execute("DROP TRIGGER abort_run_settled")
        events = store.terminalize_run(prepared.run_id, "cancelled")
        assert [event.type for event in events] == ["item.completed", "run.settled"]
        assert events[0].payload["item"]["status"] == "cancelled"
        assert events[0].payload["item"]["content"] == "partial"
        assert events[1].payload["status"] == "cancelled"
        assert store.terminalize_run(prepared.run_id, "cancelled") == ()

        with pytest.raises(sqlite3.IntegrityError):
            store._connection.execute(
                """
                INSERT INTO events(event_type, run_id, created_at, payload_json)
                VALUES ('run.settled', ?, '2026-08-11T00:00:00.000Z', '{}')
                """,
                (prepared.run_id,),
            )
        store._connection.rollback()
    finally:
        store.close()


def test_terminalize_run_cancels_a_running_tool_item_before_settling(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Cancel tool")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="run a command",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(prepared.run_id)
        tool_item_id, _ = store.create_tool_call_item(
            prepared.run_id,
            step_id="step-cancel",
            call_id="call-cancel",
            tool_name="process_run",
            arguments={"command": "long-running"},
        )

        events = store.terminalize_run(prepared.run_id, "cancelled")

        assert [event.type for event in events] == [
            "item.completed",
            "item.completed",
            "run.settled",
        ]
        assert events[0].item_id == tool_item_id
        assert events[0].payload["item"]["kind"] == "tool_call"
        assert events[0].payload["item"]["status"] == "cancelled"
        assert events[1].payload["item"]["kind"] == "tool_result"
        assert events[1].payload["item"]["status"] == "cancelled"
        assert events[1].payload["item"]["data"]["result"]["cancelled"] is True
        assert events[2].payload["status"] == "cancelled"
        assert store.terminalize_run(prepared.run_id, "cancelled") == ()
        row = store._connection.execute(
            "SELECT status, data_json FROM items WHERE id = ?",
            (tool_item_id,),
        ).fetchone()
        assert row["status"] == "cancelled"
        assert '"callId":"call-cancel"' in row["data_json"]

        before, latest_seq = store.replay_events(0, 100)
        store.rebuild_projections()
        after, rebuilt_latest_seq = store.replay_events(0, 100)
        assert after == before
        assert rebuilt_latest_seq == latest_seq
    finally:
        store.close()


def test_tool_call_and_result_completion_is_atomic(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Atomic tool result")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="run a command",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(prepared.run_id)
        tool_item_id, _ = store.create_tool_call_item(
            prepared.run_id,
            step_id="step-atomic",
            call_id="call-atomic",
            tool_name="process_run",
            arguments={"command": "echo atomic"},
        )
        store._connection.execute(
            f"""
            CREATE TRIGGER abort_tool_result_event
            BEFORE INSERT ON events
            WHEN NEW.event_type = 'item.completed'
             AND NEW.item_id != '{tool_item_id}'
            BEGIN
                SELECT RAISE(ABORT, 'tool result insert failed');
            END;
            """
        )
        result = {
            "toolCallId": "call-atomic",
            "toolName": "process_run",
            "ok": True,
            "output": "atomic",
            "cancelled": False,
            "stdout": "atomic\n",
            "stderr": "",
            "exitCode": 0,
            "durationMs": 1,
            "timedOut": False,
            "truncated": False,
        }

        with pytest.raises(sqlite3.IntegrityError, match="tool result insert failed"):
            store.complete_tool_call(
                tool_item_id,
                status="completed",
                result=result,
                result_content="atomic",
            )

        rows = store._connection.execute(
            "SELECT kind, status FROM items WHERE run_id = ? ORDER BY ordinal",
            (prepared.run_id,),
        ).fetchall()
        assert [dict(row) for row in rows] == [
            {"kind": "message", "status": "completed"},
            {"kind": "tool_call", "status": "running"},
        ]
        replayed, _ = store.replay_events(0, 100)
        assert not any(
            event.type == "item.completed" and event.item_id == tool_item_id for event in replayed
        )
    finally:
        store.close()


def test_recovery_fails_running_runs_and_orders_queued_runs_by_event_seq(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Recovery")
        running = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="running",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(running.run_id)
        item_id, _ = store.create_assistant_item(running.run_id)
        store.append_text_delta(item_id, "preserved partial")
        first_queued = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="first queued",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        second_queued = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="second queued",
            provider_id="scripted",
            model_id="scripted-v1",
        )

        recovery = store.recover_incomplete_runs()

        assert recovery.queued_run_ids == (first_queued.run_id, second_queued.run_id)
        assert store.run_status(running.run_id) == "failed"
        assert store.run_status(first_queued.run_id) == "queued"
        replayed, _ = store.replay_events(0, 100)
        running_terminal = [
            event
            for event in replayed
            if event.run_id == running.run_id
            and (
                event.type == "run.settled"
                or (
                    event.type == "item.completed"
                    and event.payload.get("item", {}).get("role") == "assistant"
                )
            )
        ]
        assert [event.type for event in running_terminal] == [
            "item.completed",
            "run.settled",
        ]
        assert running_terminal[0].payload["item"]["content"] == "preserved partial"
        assert running_terminal[0].payload["item"]["status"] == "failed"
        assert running_terminal[1].payload["reasonCode"] == "runtime_interrupted"
    finally:
        store.close()


def test_recovery_completes_an_interrupted_tool_with_a_matching_result(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Recover tool")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="run before crash",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(prepared.run_id)
        call_item_id, _ = store.create_tool_call_item(
            prepared.run_id,
            step_id="step-recovery",
            call_id="call-recovery",
            tool_name="process_run",
            arguments={"command": "long-running"},
        )

        store.recover_incomplete_runs()

        replayed, _ = store.replay_events(0, 100)
        terminal = [
            event
            for event in replayed
            if event.run_id == prepared.run_id
            and event.type in {"item.completed", "run.settled"}
            and (
                event.type == "run.settled"
                or event.payload.get("item", {}).get("kind") in {"tool_call", "tool_result"}
            )
        ]
        assert [event.type for event in terminal] == [
            "item.completed",
            "item.completed",
            "run.settled",
        ]
        assert terminal[0].item_id == call_item_id
        assert terminal[0].payload["item"]["status"] == "failed"
        result_item = terminal[1].payload["item"]
        assert result_item["status"] == "failed"
        assert result_item["data"]["callId"] == "call-recovery"
        assert result_item["data"]["toolCallItemId"] == call_item_id
        assert result_item["data"]["result"]["errorCode"] == "runtime_interrupted"
        assert terminal[2].payload["status"] == "failed"
        assert terminal[2].payload["reasonCode"] == "runtime_interrupted"
    finally:
        store.close()


def test_context_does_not_include_later_queued_turns(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Queued context")
        first = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="alpha",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        second = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="beta",
            provider_id="scripted",
            model_id="scripted-v1",
        )

        assert store.context_messages(
            thread.default_branch_id,
            through_turn_id=first.turn_id,
        ) == [("user", "alpha")]
        assert store.context_messages(
            thread.default_branch_id,
            through_turn_id=second.turn_id,
        ) == [("user", "alpha"), ("user", "beta")]
    finally:
        store.close()


@pytest.mark.parametrize("tamper", ["missing-limits", "changed-limits", "excess-call"])
def test_rebuild_enforces_frozen_execution_budget(tmp_path: Path, tamper: str) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Budget integrity")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="two requests",
            provider_id="test",
            model_id="test",
            max_model_calls=2,
        )
        store.mark_run_running(prepared.run_id)
        for ordinal in (1, 2):
            store.prepare_model_step(prepared.run_id, step_ordinal=ordinal)
            store.complete_provider_step(
                prepared.run_id,
                step_ordinal=ordinal,
                assistant_item_id=None,
                tool_calls=(),
                reasoning_content=None,
                usage=None,
                response_model_id=None,
                request_id=None,
            )
        row = store._connection.execute(
            "SELECT seq, payload_json FROM events WHERE run_id = ? "
            "AND event_type = 'item.completed' ORDER BY seq LIMIT 1",
            (prepared.run_id,),
        ).fetchone()
        payload = json_loads(row["payload_json"])
        if tamper == "missing-limits":
            del payload["run"]["executionLimits"]
        else:
            payload["run"]["executionLimits"]["maxModelCalls"] = 1
            if tamper == "excess-call":
                payload["runConfig"]["maxModelCalls"] = 1
        with store._connection:
            store._connection.execute(
                "UPDATE events SET payload_json = ? WHERE seq = ?",
                (json_dumps(payload), row["seq"]),
            )
        before = _projection_snapshot(store)
        with pytest.raises(RuntimeError, match="(shape|execution limits|budget_exceeded)"):
            store.rebuild_projections()
        assert _projection_snapshot(store) == before
    finally:
        store.close()
