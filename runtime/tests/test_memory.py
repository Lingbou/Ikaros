from __future__ import annotations

import hashlib
import sqlite3
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from threading import Barrier
from typing import Any, cast

import pytest

import ikaros_runtime.memory.store as memory_store_module
from ikaros_runtime.errors import (
    InvalidParamsError,
    MemoryOperationError,
    MemorySchemaIncompatibleError,
)
from ikaros_runtime.memory import MemoryScope, MemorySourceSnapshot, SqliteMemoryStore
from ikaros_runtime.memory.domain import validate_memory_content
from ikaros_runtime.memory.schema import MEMORY_SCHEMA_VERSION
from ikaros_runtime.security import response_values_contain_protected_value
from ikaros_runtime.services.memories import MemoryService
from ikaros_runtime.storage import SqliteRuntimeStore

from .helpers import prepare_turn


def _create(
    store: SqliteMemoryStore,
    request_id: str,
    *,
    content: str = "The user prefers concise technical explanations.",
    kind: str = "preference",
    scope: MemoryScope | None = None,
) -> str:
    receipt = store.create_memory_once(
        kind=cast(Any, kind),
        scope=scope or MemoryScope("global", None),
        content=content,
        client_request_id=request_id,
    )
    assert receipt.created is True
    assert receipt.resulting_revision == 1
    return receipt.memory_id


def _create_session_item(
    store: SqliteRuntimeStore,
    *,
    content: str = "Remember that the user prefers concise answers.",
) -> tuple[str, str, str]:
    thread, _event = store.create_thread("Memory provenance")
    prepared = prepare_turn(
        store,
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content=content,
        provider_id="scripted",
        model_id="scripted-v1",
    )
    item_id = prepared.initial_events[0].item_id
    assert item_id is not None
    return thread.id, prepared.turn_id, item_id


def _assert_memory_error(error: pytest.ExceptionInfo[MemoryOperationError], reason: str) -> None:
    assert error.value.reason_code == reason


def test_memory_schema_is_independent_complete_and_wal_backed(tmp_path: Path) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        assert store._connection.execute("PRAGMA user_version").fetchone()[0] == (
            MEMORY_SCHEMA_VERSION
        )
        assert store._connection.execute("PRAGMA journal_mode").fetchone()[0] == "wal"
        assert store._connection.execute("PRAGMA foreign_keys").fetchone()[0] == 1
        objects = {
            row[0]
            for row in store._connection.execute(
                "SELECT name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'"
            )
        }
        assert objects == {
            "memory_records",
            "memory_revisions",
            "memory_operations",
            "memory_records_state_order_idx",
            "memory_records_scope_order_idx",
            "memory_records_scope_kind_order_idx",
        }
        assert [row[1] for row in store._connection.execute("PRAGMA database_list")] == [
            "main"
        ]
    finally:
        store.close()


def test_schema_supports_forget_redaction_without_deleting_revision_metadata(
    tmp_path: Path,
) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    memory_id = _create(store, "redact-revision", content="sensitive Memory body")
    timestamp = "2026-08-16T12:34:56.789Z"
    source_ids = (
        "thread_" + "1" * 32,
        "turn_" + "2" * 32,
        "item_" + "3" * 32,
    )
    try:
        store._connection.execute(
            """
            UPDATE memory_revisions
            SET source_kind = 'session_item',
                source_thread_id = ?,
                source_turn_id = ?,
                source_item_id = ?,
                source_item_digest = ?
            WHERE memory_id = ? AND revision = 1
            """,
            (*source_ids, "a" * 64, memory_id),
        )
        store._connection.commit()

        store._connection.execute("BEGIN IMMEDIATE")
        store._connection.execute(
            """
            INSERT INTO memory_revisions(
                memory_id, revision, operation, content, content_digest,
                content_redacted_at, source_kind, source_thread_id,
                source_turn_id, source_item_id, source_item_digest, created_at
            ) VALUES (?, 2, 'forget', NULL, NULL, NULL,
                      'user_explicit', NULL, NULL, NULL, NULL, ?)
            """,
            (memory_id, timestamp),
        )
        store._connection.execute(
            """
            UPDATE memory_revisions
            SET content = NULL,
                content_digest = NULL,
                source_item_digest = NULL,
                content_redacted_at = ?
            WHERE memory_id = ? AND revision < 2
            """,
            (timestamp, memory_id),
        )
        store._connection.execute(
            """
            UPDATE memory_records
            SET current_revision = 2,
                state = 'forgotten',
                updated_at = ?,
                forgotten_at = ?
            WHERE id = ?
            """,
            (timestamp, timestamp, memory_id),
        )
        store._connection.commit()

        old_revision = store._connection.execute(
            """
            SELECT operation, content, content_digest, content_redacted_at,
                   source_kind, source_thread_id, source_turn_id, source_item_id,
                   source_item_digest, created_at
            FROM memory_revisions
            WHERE memory_id = ? AND revision = 1
            """,
            (memory_id,),
        ).fetchone()
        assert old_revision is not None
        assert tuple(old_revision) == (
            "create",
            None,
            None,
            timestamp,
            "session_item",
            *source_ids,
            None,
            old_revision["created_at"],
        )
        assert store._connection.execute("PRAGMA foreign_key_check").fetchall() == []
        assert store.get_memory(memory_id).to_wire() == {
            "id": memory_id,
            "kind": "preference",
            "scope": {"type": "global", "key": None},
            "revision": 2,
            "state": "forgotten",
            "content": None,
            "provenance": {
                "sourceKind": "user_explicit",
                "threadId": None,
                "turnId": None,
                "itemId": None,
                "status": "not_applicable",
            },
            "createdAt": store._connection.execute(
                "SELECT created_at FROM memory_records WHERE id = ?", (memory_id,)
            ).fetchone()[0],
            "updatedAt": timestamp,
            "forgottenAt": timestamp,
        }
    finally:
        store.close()


def test_create_get_and_list_preserve_exact_content_and_scope(tmp_path: Path) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    content = "  第一行\nsecond line with emoji: 🌸  "
    workspace = MemoryScope("workspace", "workspace-stable-id")
    try:
        memory_id = _create(
            store,
            "create-workspace-memory",
            content=content,
            kind="project",
            scope=workspace,
        )

        record = store.get_memory(memory_id)
        assert record.content == content
        assert record.kind == "project"
        assert record.scope == workspace
        assert record.revision == 1
        assert record.state == "active"
        assert record.provenance.to_wire() == {
            "sourceKind": "user_explicit",
            "threadId": None,
            "turnId": None,
            "itemId": None,
            "status": "not_applicable",
        }
        page = store.list_memories(
            cursor=None,
            limit=50,
            scope=workspace,
            kind="project",
        )
        assert page.has_more is False
        assert page.next_cursor is None
        assert [memory.id for memory in page.memories] == [memory_id]
        assert page.memories[0].preview == content
    finally:
        store.close()


def test_list_returns_only_bounded_preview_and_get_returns_full_content(
    tmp_path: Path,
) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    content = "界" * 200
    try:
        memory_id = _create(store, "preview", content=content, kind="fact")
        summary = store.list_memories(
            cursor=None,
            limit=50,
            scope=None,
            kind=None,
        ).memories[0]
        assert summary.preview == "界" * 160
        assert store.get_memory(memory_id).content == content
    finally:
        store.close()


def test_create_is_idempotent_across_restart_and_rejects_changed_input(
    tmp_path: Path,
) -> None:
    path = tmp_path / "memory.db"
    store = SqliteMemoryStore(path)
    memory_id = _create(store, "stable-request")
    store.close()

    reopened = SqliteMemoryStore(path)
    try:
        repeated = reopened.create_memory_once(
            kind="preference",
            scope=MemoryScope("global", None),
            content="The user prefers concise technical explanations.",
            client_request_id="stable-request",
        )
        assert repeated.memory_id == memory_id
        assert repeated.resulting_revision == 1
        assert repeated.created is False
        assert reopened._connection.execute(
            "SELECT COUNT(*) FROM memory_records"
        ).fetchone()[0] == 1
        assert reopened._connection.execute(
            "SELECT COUNT(*) FROM memory_revisions"
        ).fetchone()[0] == 1
        assert reopened._connection.execute(
            "SELECT COUNT(*) FROM memory_operations"
        ).fetchone()[0] == 1

        with pytest.raises(MemoryOperationError, match="memory_idempotency_conflict"):
            reopened.create_memory_once(
                kind="fact",
                scope=MemoryScope("global", None),
                content="The user prefers concise technical explanations.",
                client_request_id="stable-request",
            )
        with pytest.raises(MemoryOperationError, match="memory_idempotency_conflict"):
            reopened.create_memory_once(
                kind="preference",
                scope=MemoryScope("global", None),
                content="different",
                client_request_id="stable-request",
            )
    finally:
        reopened.close()


def test_correct_appends_revision_and_is_idempotent_across_restart(
    tmp_path: Path,
) -> None:
    path = tmp_path / "memory.db"
    store = SqliteMemoryStore(path)
    memory_id = _create(store, "create-before-correct", content="original")
    corrected = store.correct_memory_once(
        memory_id=memory_id,
        expected_revision=1,
        content="corrected",
        client_request_id="correct-once",
    )
    assert corrected.to_wire() == {
        "memoryId": memory_id,
        "resultingRevision": 2,
        "created": True,
    }
    assert store.get_memory(memory_id).content == "corrected"
    assert store.get_memory(memory_id).revision == 2
    assert store._connection.execute(
        "SELECT content FROM memory_revisions WHERE memory_id = ? AND revision = 1",
        (memory_id,),
    ).fetchone()[0] == "original"
    store.close()

    reopened = SqliteMemoryStore(path)
    try:
        repeated = reopened.correct_memory_once(
            memory_id=memory_id,
            expected_revision=1,
            content="corrected",
            client_request_id="correct-once",
        )
        assert repeated.resulting_revision == 2
        assert repeated.created is False
        with pytest.raises(MemoryOperationError) as stale:
            reopened.correct_memory_once(
                memory_id=memory_id,
                expected_revision=1,
                content="stale",
                client_request_id="correct-stale",
            )
        _assert_memory_error(stale, "memory_revision_conflict")
        with pytest.raises(MemoryOperationError) as changed_retry:
            reopened.correct_memory_once(
                memory_id=memory_id,
                expected_revision=1,
                content="changed retry",
                client_request_id="correct-once",
            )
        _assert_memory_error(changed_retry, "memory_idempotency_conflict")
    finally:
        reopened.close()


def test_forget_creates_tombstone_redacts_all_derived_content_and_replays(
    tmp_path: Path,
) -> None:
    path = tmp_path / "memory.db"
    store = SqliteMemoryStore(path)
    source = MemorySourceSnapshot(
        thread_id="thread_" + "1" * 32,
        turn_id="turn_" + "2" * 32,
        item_id="item_" + "3" * 32,
        item_digest="4" * 64,
    )
    created = store.create_memory_once(
        kind="fact",
        scope=MemoryScope("global", None),
        content="sensitive original",
        client_request_id="forget-create",
        source=source,
    )
    memory_id = created.memory_id
    store.correct_memory_once(
        memory_id=memory_id,
        expected_revision=1,
        content="sensitive correction",
        client_request_id="forget-correct",
    )
    forgotten = store.forget_memory_once(
        memory_id=memory_id,
        expected_revision=2,
        client_request_id="forget-once",
    )
    assert forgotten.to_wire() == {
        "memoryId": memory_id,
        "resultingRevision": 3,
        "created": True,
    }
    record = store.get_memory(memory_id)
    assert record.state == "forgotten"
    assert record.content is None
    assert record.revision == 3
    assert store.list_memories(
        cursor=None,
        limit=50,
        scope=None,
        kind=None,
    ).memories == ()
    forgotten_page = store.list_memories(
        cursor=None,
        limit=50,
        scope=None,
        kind=None,
        state="forgotten",
    )
    assert len(forgotten_page.memories) == 1
    assert forgotten_page.memories[0].preview is None
    revisions = store._connection.execute(
        """
        SELECT revision, operation, content, content_digest, content_redacted_at,
               source_thread_id, source_turn_id, source_item_id, source_item_digest
        FROM memory_revisions WHERE memory_id = ? ORDER BY revision
        """,
        (memory_id,),
    ).fetchall()
    assert [(row["revision"], row["operation"]) for row in revisions] == [
        (1, "create"),
        (2, "correct"),
        (3, "forget"),
    ]
    assert all(row["content"] is None for row in revisions)
    assert all(row["content_digest"] is None for row in revisions)
    assert all(row["source_item_digest"] is None for row in revisions)
    assert all(row["content_redacted_at"] is not None for row in revisions[:2])
    assert revisions[2]["content_redacted_at"] is None
    assert tuple(revisions[0][key] for key in (
        "source_thread_id",
        "source_turn_id",
        "source_item_id",
    )) == (source.thread_id, source.turn_id, source.item_id)
    store.close()

    reopened = SqliteMemoryStore(path)
    try:
        repeated = reopened.forget_memory_once(
            memory_id=memory_id,
            expected_revision=2,
            client_request_id="forget-once",
        )
        assert repeated.resulting_revision == 3
        assert repeated.created is False
        with pytest.raises(MemoryOperationError) as changed_forget_retry:
            reopened.forget_memory_once(
                memory_id=memory_id,
                expected_revision=3,
                client_request_id="forget-once",
            )
        _assert_memory_error(changed_forget_retry, "memory_idempotency_conflict")
        with pytest.raises(MemoryOperationError) as cross_method_retry:
            reopened.correct_memory_once(
                memory_id=memory_id,
                expected_revision=2,
                content="must not cross methods",
                client_request_id="forget-once",
            )
        _assert_memory_error(cross_method_retry, "memory_idempotency_conflict")
        for request_id, operation in (
            ("forget-create", "create"),
            ("forget-correct", "correct"),
        ):
            with pytest.raises(MemoryOperationError) as old_retry:
                if operation == "create":
                    reopened.create_memory_once(
                        kind="fact",
                        scope=MemoryScope("global", None),
                        content="sensitive original",
                        client_request_id=request_id,
                        source=source,
                    )
                else:
                    reopened.correct_memory_once(
                        memory_id=memory_id,
                        expected_revision=1,
                        content="sensitive correction",
                        client_request_id=request_id,
                    )
            _assert_memory_error(old_retry, "memory_forgotten")
        with pytest.raises(MemoryOperationError) as second_forget:
            reopened.forget_memory_once(
                memory_id=memory_id,
                expected_revision=3,
                client_request_id="another-forget",
            )
        _assert_memory_error(second_forget, "memory_forgotten")
    finally:
        reopened.close()


def test_correct_and_forget_transactions_are_all_or_nothing(tmp_path: Path) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    memory_id = _create(store, "atomic-mutations", content="before")
    try:
        store._connection.execute(
            """
            CREATE TEMP TRIGGER reject_correct_receipt
            BEFORE INSERT ON memory_operations
            WHEN NEW.method = 'memory.correct'
            BEGIN
                SELECT RAISE(ABORT, 'reject correct');
            END
            """
        )
        with pytest.raises(sqlite3.IntegrityError, match="reject correct"):
            store.correct_memory_once(
                memory_id=memory_id,
                expected_revision=1,
                content="should rollback",
                client_request_id="atomic-correct",
            )
        assert store.get_memory(memory_id).revision == 1
        assert store.get_memory(memory_id).content == "before"
        store._connection.execute("DROP TRIGGER reject_correct_receipt")
        store._connection.execute(
            """
            CREATE TEMP TRIGGER reject_forget_record
            BEFORE UPDATE ON memory_records
            WHEN NEW.state = 'forgotten'
            BEGIN
                SELECT RAISE(ABORT, 'reject forget');
            END
            """
        )
        with pytest.raises(sqlite3.IntegrityError, match="reject forget"):
            store.forget_memory_once(
                memory_id=memory_id,
                expected_revision=1,
                client_request_id="atomic-forget",
            )
        assert store.get_memory(memory_id).revision == 1
        assert store.get_memory(memory_id).content == "before"
        assert store._connection.execute(
            "SELECT content_digest FROM memory_revisions WHERE memory_id = ?",
            (memory_id,),
        ).fetchone()[0] is not None
        assert store._connection.execute(
            """
            SELECT COUNT(*) FROM memory_operations
            WHERE client_request_id IN ('atomic-correct', 'atomic-forget')
            """
        ).fetchone()[0] == 0
    finally:
        store.close()


def test_create_transaction_is_all_or_nothing(tmp_path: Path) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    store._connection.execute(
        """
        CREATE TEMP TRIGGER reject_memory_revision
        BEFORE INSERT ON memory_revisions
        BEGIN
            SELECT RAISE(ABORT, 'injected failure');
        END
        """
    )
    try:
        with pytest.raises(sqlite3.IntegrityError, match="injected failure"):
            _create(store, "atomic-failure")
        assert store._connection.in_transaction is False
        for table in ("memory_records", "memory_revisions", "memory_operations"):
            assert store._connection.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0] == 0
    finally:
        store.close()


def test_keyset_pagination_is_stable_and_cursor_is_filter_bound(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(memory_store_module, "utc_now", lambda: "2026-08-16T12:34:56.789Z")
    store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        ids = {
            _create(store, f"global-{index}", content=f"global {index}", kind="fact")
            for index in range(5)
        }
        workspace_id = _create(
            store,
            "workspace-only",
            content="workspace memory",
            kind="project",
            scope=MemoryScope("workspace", "workspace-1"),
        )

        first = store.list_memories(
            cursor=None,
            limit=2,
            scope=MemoryScope("global", None),
            kind="fact",
        )
        second = store.list_memories(
            cursor=first.next_cursor,
            limit=2,
            scope=MemoryScope("global", None),
            kind="fact",
        )
        third = store.list_memories(
            cursor=second.next_cursor,
            limit=2,
            scope=MemoryScope("global", None),
            kind="fact",
        )
        observed = [
            memory.id
            for page in (first, second, third)
            for memory in page.memories
        ]
        assert observed == sorted(ids)
        assert len(observed) == len(set(observed)) == 5
        assert first.has_more and second.has_more
        assert third.has_more is False and third.next_cursor is None
        assert workspace_id not in observed

        assert first.next_cursor is not None
        with pytest.raises(ValueError, match="cursor is invalid"):
            store.list_memories(
                cursor=first.next_cursor,
                limit=2,
                scope=None,
                kind="fact",
            )
        with pytest.raises(ValueError, match="cursor is invalid"):
            store.list_memories(
                cursor=first.next_cursor + "=",
                limit=2,
                scope=MemoryScope("global", None),
                kind="fact",
            )
    finally:
        store.close()


@pytest.mark.parametrize("kind", ("fact", "preference", "relationship", "project"))
def test_all_v0_memory_kinds_are_supported(tmp_path: Path, kind: str) -> None:
    store = SqliteMemoryStore(tmp_path / f"{kind}.db")
    try:
        memory_id = _create(store, kind, kind=kind, content=f"kind {kind}")
        assert store.get_memory(memory_id).kind == kind
    finally:
        store.close()


@pytest.mark.parametrize(
    "content, message",
    (
        ("", "must not be blank"),
        (" \t\r\n", "must not be blank"),
        ("x" * 2049, "must not exceed"),
        ("nul\x00byte", "control characters"),
        ("lone-surrogate-\ud800", "valid Unicode"),
    ),
)
def test_memory_content_validation_rejects_invalid_input(
    content: str,
    message: str,
) -> None:
    with pytest.raises(ValueError, match=message):
        validate_memory_content(content)


def test_memory_content_accepts_exact_2048_unicode_characters() -> None:
    assert validate_memory_content("🌸" * 2048) == "🌸" * 2048


def test_incompatible_memory_schema_fails_without_rewriting_file(tmp_path: Path) -> None:
    path = tmp_path / "memory.db"
    connection = sqlite3.connect(path)
    connection.execute("CREATE TABLE foreign_data(value TEXT)")
    connection.execute("PRAGMA user_version = 99")
    connection.commit()
    connection.close()
    before = path.read_bytes()

    with pytest.raises(MemorySchemaIncompatibleError, match="incompatible"):
        SqliteMemoryStore(path)

    assert path.read_bytes() == before
    assert not Path(f"{path}-wal").exists()
    assert not Path(f"{path}-shm").exists()


def test_same_version_memory_schema_drift_is_rejected(tmp_path: Path) -> None:
    path = tmp_path / "memory.db"
    store = SqliteMemoryStore(path)
    store.close()

    connection = sqlite3.connect(path)
    connection.execute("ALTER TABLE memory_records ADD COLUMN unexpected TEXT")
    connection.commit()
    connection.close()
    before = path.read_bytes()

    with pytest.raises(MemorySchemaIncompatibleError, match="incompatible"):
        SqliteMemoryStore(path)

    assert path.read_bytes() == before


def test_state_reset_does_not_touch_closed_memory_database(tmp_path: Path) -> None:
    memory_path = tmp_path / "memory.db"
    store = SqliteMemoryStore(memory_path)
    memory_id = _create(store, "survive-state-reset", content="durable memory")
    store.close()
    before = hashlib.sha256(memory_path.read_bytes()).hexdigest()

    for name in ("state.db", "state.db-wal", "state.db-shm"):
        candidate = tmp_path / name
        candidate.write_bytes(b"disposable session state")
        candidate.unlink()

    assert hashlib.sha256(memory_path.read_bytes()).hexdigest() == before
    reopened = SqliteMemoryStore(memory_path)
    try:
        assert reopened.get_memory(memory_id).content == "durable memory"
    finally:
        reopened.close()


def test_memory_protected_value_probe_scans_only_persisted_user_data(
    tmp_path: Path,
) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    protected = "sk-memory-persisted-sentinel"
    source = MemorySourceSnapshot(
        thread_id="thread_" + "1" * 32,
        turn_id="turn_" + "2" * 32,
        item_id="item_" + "3" * 32,
        item_digest="4" * 64,
    )
    try:
        _create(store, "safe-id", content=f"prefix {protected} suffix", kind="fact")
        store.create_memory_once(
            kind="fact",
            scope=MemoryScope("global", None),
            content="safe provenance body",
            client_request_id="source-provenance-probe",
            source=source,
        )
        assert store.contains_protected_values((protected,)) is True
        assert store.contains_protected_values((source.thread_id,)) is False
        assert store.contains_protected_values((source.turn_id[5:13],)) is False
        assert store.contains_protected_values((source.item_id,)) is False
        assert store.contains_protected_values(("fact",)) is False
        assert store.contains_protected_values(("memory.create",)) is False
        assert store.contains_protected_values(()) is False
    finally:
        store.close()


def test_memory_service_validates_params_scope_security_and_unknown_ids(
    tmp_path: Path,
) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    session_store = SqliteRuntimeStore(tmp_path / "state.db")
    observed: list[object] = []
    service = MemoryService(store, observed.append, session_store)
    params: dict[str, object] = {
        "kind": "fact",
        "scope": {"type": "global", "key": None},
        "content": "Ikaros is the product identity.",
        "clientRequestId": "service-create",
    }
    try:
        created = service.create(params)
        assert created["created"] is True
        memory_id = cast(str, created["memoryId"])
        assert service.get({"memoryId": memory_id})["memory"] == store.get_memory(
            memory_id
        ).to_wire()
        assert service.list({})["memories"]
        assert observed

        with pytest.raises(InvalidParamsError, match="requires exactly"):
            service.create({**params, "extra": True})
        with pytest.raises(InvalidParamsError, match="scope"):
            service.create({**params, "scope": {"type": "global", "key": "bad"}})
        with pytest.raises(InvalidParamsError, match="kind"):
            service.create({**params, "kind": "instruction"})
        with pytest.raises(MemoryOperationError, match="memory_not_found"):
            service.get({"memoryId": "memory_" + "0" * 32})
        with pytest.raises(InvalidParamsError, match="state"):
            service.list({"state": "all"})
        with pytest.raises(InvalidParamsError, match="memoryId"):
            service.get({"memoryId": "\ud800"})
        with pytest.raises(InvalidParamsError, match="memoryId"):
            service.get({"memoryId": "0" * 32})
        with pytest.raises(InvalidParamsError, match="scope"):
            service.create(
                {
                    **params,
                    "scope": {"type": "workspace", "key": "\ud800"},
                }
            )
        with pytest.raises(InvalidParamsError, match="clientRequestId"):
            service.create({**params, "clientRequestId": "\ud800"})
    finally:
        session_store.close()
        store.close()


def test_session_item_provenance_is_derived_available_and_detects_drift(
    tmp_path: Path,
) -> None:
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    session_store = SqliteRuntimeStore(tmp_path / "state.db")
    thread_id, turn_id, item_id = _create_session_item(session_store)
    service = MemoryService(memory_store, lambda _value: None, session_store)
    params = {
        "kind": "preference",
        "scope": {"type": "global", "key": None},
        "content": "The user prefers concise answers.",
        "clientRequestId": "source-create",
        "source": {"type": "session_item", "itemId": item_id},
    }
    try:
        created = service.create(params)
        memory_id = cast(str, created["memoryId"])
        provenance = cast(dict[str, object], service.get({"memoryId": memory_id})["memory"])[
            "provenance"
        ]
        assert provenance == {
            "sourceKind": "session_item",
            "threadId": thread_id,
            "turnId": turn_id,
            "itemId": item_id,
            "status": "available",
        }
        run_id = session_store._connection.execute(
            "SELECT id FROM runs WHERE turn_id = ?",
            (turn_id,),
        ).fetchone()[0]
        session_store.terminalize_run(run_id, "cancelled", reason_code="test")
        session_store.set_thread_archived(thread_id, archived=True)
        archived = cast(
            dict[str, object], service.get({"memoryId": memory_id})["memory"]
        )
        assert cast(dict[str, object], archived["provenance"])["status"] == "available"
        session_store._connection.execute(
            "UPDATE items SET content = ? WHERE id = ?",
            ("source changed", item_id),
        )
        session_store._connection.commit()
        drifted = cast(
            dict[str, object], service.get({"memoryId": memory_id})["memory"]
        )
        assert cast(dict[str, object], drifted["provenance"])["status"] == "unavailable"
    finally:
        session_store.close()
        memory_store.close()


def test_session_source_create_replays_after_state_reset_without_reading_source(
    tmp_path: Path,
) -> None:
    memory_path = tmp_path / "memory.db"
    state_path = tmp_path / "state.db"
    memory_store = SqliteMemoryStore(memory_path)
    session_store = SqliteRuntimeStore(state_path)
    _thread_id, _turn_id, item_id = _create_session_item(session_store)
    service = MemoryService(memory_store, lambda _value: None, session_store)
    params = {
        "kind": "fact",
        "scope": {"type": "global", "key": None},
        "content": "A durable fact from a Session Item.",
        "clientRequestId": "source-reset-replay",
        "source": {"type": "session_item", "itemId": item_id},
    }
    first = service.create(params)
    memory_id = cast(str, first["memoryId"])
    session_store.close()
    memory_store.close()
    before = hashlib.sha256(memory_path.read_bytes()).hexdigest()
    for path in (state_path, Path(f"{state_path}-wal"), Path(f"{state_path}-shm")):
        if path.exists():
            path.unlink()

    reset_session_store = SqliteRuntimeStore(state_path)
    reopened_memory_store = SqliteMemoryStore(memory_path)
    reset_service = MemoryService(
        reopened_memory_store,
        lambda _value: None,
        reset_session_store,
    )
    try:
        replayed = reset_service.create(params)
        assert replayed == {
            "memoryId": memory_id,
            "resultingRevision": 1,
            "created": False,
        }
        memory = cast(
            dict[str, object], reset_service.get({"memoryId": memory_id})["memory"]
        )
        assert cast(dict[str, object], memory["provenance"])["status"] == "unavailable"
    finally:
        reset_session_store.close()
        reopened_memory_store.close()
    assert hashlib.sha256(memory_path.read_bytes()).hexdigest() == before


@pytest.mark.parametrize(
    "column, value",
    (
        ("kind", "tool_call"),
        ("role", "tool"),
        ("status", "running"),
    ),
)
def test_session_source_rejects_ineligible_items(
    tmp_path: Path,
    column: str,
    value: str,
) -> None:
    memory_store = SqliteMemoryStore(tmp_path / f"memory-{column}.db")
    session_store = SqliteRuntimeStore(tmp_path / f"state-{column}.db")
    _thread_id, _turn_id, item_id = _create_session_item(session_store)
    session_store._connection.execute(
        f"UPDATE items SET {column} = ? WHERE id = ?",
        (value, item_id),
    )
    session_store._connection.commit()
    service = MemoryService(memory_store, lambda _value: None, session_store)
    try:
        with pytest.raises(MemoryOperationError) as unavailable:
            service.create(
                {
                    "kind": "fact",
                    "scope": {"type": "global", "key": None},
                    "content": "must not persist",
                    "clientRequestId": f"invalid-source-{column}",
                    "source": {"type": "session_item", "itemId": item_id},
                }
            )
        _assert_memory_error(unavailable, "memory_source_unavailable")
        assert memory_store._connection.execute(
            "SELECT COUNT(*) FROM memory_records"
        ).fetchone()[0] == 0
        assert memory_store._connection.in_transaction is False
    finally:
        session_store.close()
        memory_store.close()


def test_session_source_request_cannot_forge_thread_or_turn(tmp_path: Path) -> None:
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    session_store = SqliteRuntimeStore(tmp_path / "state.db")
    _thread_id, _turn_id, item_id = _create_session_item(session_store)
    service = MemoryService(memory_store, lambda _value: None, session_store)
    try:
        with pytest.raises(InvalidParamsError, match="source requires exactly"):
            service.create(
                {
                    "kind": "fact",
                    "scope": {"type": "global", "key": None},
                    "content": "forged",
                    "clientRequestId": "forged-source",
                    "source": {
                        "type": "session_item",
                        "itemId": item_id,
                        "threadId": "thread_" + "0" * 32,
                    },
                }
            )
        with pytest.raises(InvalidParamsError, match="source itemId"):
            service.create(
                {
                    "kind": "fact",
                    "scope": {"type": "global", "key": None},
                    "content": "missing prefix",
                    "clientRequestId": "missing-source-prefix",
                    "source": {"type": "session_item", "itemId": "0" * 32},
                }
            )
    finally:
        session_store.close()
        memory_store.close()


def test_memory_mutations_serialize_across_independent_connections(
    tmp_path: Path,
) -> None:
    path = tmp_path / "memory.db"
    initial = SqliteMemoryStore(path)
    memory_id = _create(initial, "concurrent-create", content="before")
    initial.close()

    barrier = Barrier(2)

    def correct(request_id: str, content: str) -> tuple[str, object]:
        store = SqliteMemoryStore(path)
        try:
            barrier.wait()
            receipt = store.correct_memory_once(
                memory_id=memory_id,
                expected_revision=1,
                content=content,
                client_request_id=request_id,
            )
            return "ok", receipt
        except MemoryOperationError as error:
            return "error", error.reason_code
        finally:
            store.close()

    with ThreadPoolExecutor(max_workers=2) as executor:
        results = list(
            executor.map(
                lambda args: correct(*args),
                (("concurrent-a", "alpha"), ("concurrent-b", "beta")),
            )
        )
    assert sum(status == "ok" for status, _value in results) == 1
    assert ("error", "memory_revision_conflict") in results

    final = SqliteMemoryStore(path)
    try:
        assert final.get_memory(memory_id).revision == 2
        assert final._connection.execute(
            "SELECT COUNT(*) FROM memory_revisions WHERE memory_id = ?",
            (memory_id,),
        ).fetchone()[0] == 2
    finally:
        final.close()


def test_memory_wire_vocabulary_has_fixed_security_provenance() -> None:
    record = {
        "id": "memory_" + "1" * 32,
        "kind": "preference",
        "scope": {"type": "workspace", "key": "project-1"},
        "revision": 1,
        "state": "active",
        "content": "dynamic Memory content",
        "provenance": {
            "sourceKind": "user_explicit",
            "threadId": None,
            "turnId": None,
            "itemId": None,
            "status": "not_applicable",
        },
        "createdAt": "2026-08-16T12:00:00.000Z",
        "updatedAt": "2026-08-16T12:00:00.000Z",
        "forgottenAt": None,
    }
    response = {
        "memory": record,
        "memories": [{**record, "preview": "dynamic preview", "content": None}],
        "memoryId": record["id"],
        "resultingRevision": 1,
    }

    for protected in (
        "memory",
        "memories",
        "memoryId",
        "resultingRevision",
        "preference",
        "workspace",
        "active",
        "user_explicit",
        "not_applicable",
        str(record["id"]),
    ):
        assert response_values_contain_protected_value(response, [protected]) is False

    for protected in ("project-1", "dynamic Memory content", "dynamic preview"):
        assert response_values_contain_protected_value(response, [protected]) is True
