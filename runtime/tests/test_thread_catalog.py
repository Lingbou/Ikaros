from __future__ import annotations

import base64
import sqlite3
from pathlib import Path

import pytest

from ikaros_runtime.json_codec import dumps as json_dumps
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.storage.journal import latest_sequence as journal_latest_sequence
from ikaros_runtime.storage.thread_catalog import (
    ThreadCatalogCursor,
    decode_thread_catalog_cursor,
    encode_thread_catalog_cursor,
)


def _encoded_cursor(payload: object) -> str:
    encoded = json_dumps(payload, separators=(",", ":")).encode()
    return base64.urlsafe_b64encode(encoded).rstrip(b"=").decode()


def test_thread_catalog_keyset_pages_ties_without_duplicates_or_omissions(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        threads = [store.create_thread(f"Thread {index}")[0] for index in range(7)]
        timestamps = (
            "2026-08-14T12:00:03.000Z",
            "2026-08-14T12:00:03.000Z",
            "2026-08-14T12:00:02.000Z",
            "2026-08-14T12:00:02.000Z",
            "2026-08-14T12:00:02.000Z",
            "2026-08-14T12:00:01.000Z",
            "2026-08-14T12:00:00.000Z",
        )
        with store._connection:
            for thread, timestamp in zip(threads, timestamps, strict=True):
                store._connection.execute(
                    "UPDATE threads SET updated_at = ? WHERE id = ?",
                    (timestamp, thread.id),
                )

        expected = sorted(
            sorted(zip(threads, timestamps, strict=True), key=lambda pair: pair[0].id),
            key=lambda pair: pair[1],
            reverse=True,
        )
        expected_ids = [thread.id for thread, _timestamp in expected]
        observed_ids: list[str] = []
        cursor: str | None = None
        snapshots: list[int] = []
        while True:
            page = store.list_thread_page(cursor=cursor, limit=2)
            observed_ids.extend(thread.id for thread in page.threads)
            snapshots.append(page.snapshot_seq)
            if not page.has_more:
                assert page.next_cursor is None
                break
            assert page.next_cursor is not None
            assert page.next_cursor != cursor
            cursor = page.next_cursor

        assert observed_ids == expected_ids
        assert len(observed_ids) == len(set(observed_ids))
        assert snapshots == [7, 7, 7, 7]
    finally:
        store.close()


def test_thread_catalog_exact_limit_and_empty_page_have_no_cursor(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        store.create_thread("First")
        store.create_thread("Second")
        page = store.list_thread_page(cursor=None, limit=2)

        assert len(page.threads) == 2
        assert page.has_more is False
        assert page.next_cursor is None
        assert page.snapshot_seq == 2

        oldest = page.threads[-1]
        after_oldest = encode_thread_catalog_cursor(
            ThreadCatalogCursor(updated_at=oldest.updated_at, thread_id=oldest.id)
        )
        empty = store.list_thread_page(cursor=after_oldest, limit=2)
        assert empty.threads == ()
        assert empty.has_more is False
        assert empty.next_cursor is None
        assert empty.snapshot_seq == 2
    finally:
        store.close()


def test_thread_catalog_separates_active_and_archived_pages_and_cursors(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        active, _event = store.create_thread("Active")
        first_archived, _event = store.create_thread("Archived one")
        second_archived, _event = store.create_thread("Archived two")
        store.set_thread_archived(first_archived.id, archived=True)
        store.set_thread_archived(second_archived.id, archived=True)

        active_page = store.list_thread_page(cursor=None, limit=50)
        assert [thread.id for thread in active_page.threads] == [active.id]
        assert active_page.threads[0].archived_at is None

        archived_first_page = store.list_thread_page(
            cursor=None,
            limit=1,
            archived=True,
        )
        assert len(archived_first_page.threads) == 1
        assert archived_first_page.threads[0].archived_at is not None
        assert archived_first_page.next_cursor is not None
        archived_second_page = store.list_thread_page(
            cursor=archived_first_page.next_cursor,
            limit=1,
            archived=True,
        )
        assert len(archived_second_page.threads) == 1
        assert archived_second_page.threads[0].id != archived_first_page.threads[0].id
        with pytest.raises(ValueError, match="thread.list cursor is invalid"):
            store.list_thread_page(
                cursor=archived_first_page.next_cursor,
                limit=1,
                archived=False,
            )
    finally:
        store.close()


def test_thread_catalog_snapshot_seq_is_each_page_query_watermark(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        store.create_thread("First")
        store.create_thread("Second")
        first_page = store.list_thread_page(cursor=None, limit=1)
        assert first_page.snapshot_seq == 2
        assert first_page.next_cursor is not None

        store.create_thread("Created between page requests")
        second_page = store.list_thread_page(cursor=first_page.next_cursor, limit=1)
        assert second_page.snapshot_seq == 3
    finally:
        store.close()


def test_thread_catalog_page_and_watermark_share_one_sqlite_snapshot(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    database_path = tmp_path / "state.db"
    store = SqliteRuntimeStore(database_path)
    writer = sqlite3.connect(database_path)
    try:
        original = store.create_thread("Original")[0]
        real_latest_sequence = journal_latest_sequence
        inserted = False

        def interleaved_latest_sequence(connection: sqlite3.Connection) -> int:
            nonlocal inserted
            snapshot_seq = real_latest_sequence(connection)
            if not inserted:
                inserted = True
                with writer:
                    writer.execute(
                        """
                        INSERT INTO threads(
                            id, title, default_branch_id, created_at, updated_at
                        ) VALUES (?, ?, ?, ?, ?)
                        """,
                        (
                            "thread_interleaved",
                            "Interleaved",
                            "branch_interleaved",
                            "2026-08-14T12:00:00.000Z",
                            "2026-08-14T12:00:00.000Z",
                        ),
                    )
                    writer.execute(
                        """
                        INSERT INTO events(
                            schema_version, event_type, thread_id, created_at, payload_json
                        ) VALUES (1, 'thread.created', ?, ?, '{}')
                        """,
                        ("thread_interleaved", "2026-08-14T12:00:00.000Z"),
                    )
            return snapshot_seq

        monkeypatch.setattr(
            "ikaros_runtime.storage.thread_catalog.latest_sequence",
            interleaved_latest_sequence,
        )

        page = store.list_thread_page(cursor=None, limit=50)

        assert page.snapshot_seq == 1
        assert [thread.id for thread in page.threads] == [original.id]
        monkeypatch.setattr(
            "ikaros_runtime.storage.thread_catalog.latest_sequence",
            real_latest_sequence,
        )
        refreshed = store.list_thread_page(cursor=None, limit=50)
        assert refreshed.snapshot_seq == 2
        assert {thread.id for thread in refreshed.threads} == {
            original.id,
            "thread_interleaved",
        }
    finally:
        writer.close()
        store.close()


@pytest.mark.parametrize(
    ("archived_filter", "index_name"),
    [
        ("IS NULL", "threads_active_catalog_order_idx"),
        ("IS NOT NULL", "threads_archived_catalog_order_idx"),
    ],
)
def test_thread_catalog_query_uses_scope_order_index(
    tmp_path: Path,
    archived_filter: str,
    index_name: str,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        plan = store._connection.execute(
            f"""
            EXPLAIN QUERY PLAN
            SELECT id, title, default_branch_id, workspace_json, created_at, updated_at,
                   archived_at
            FROM threads
            WHERE archived_at {archived_filter}
            ORDER BY updated_at DESC, id ASC
            LIMIT 51
            """
        ).fetchall()

        details = [str(row["detail"]) for row in plan]
        assert any(index_name in detail for detail in details)
        assert all("USE TEMP B-TREE" not in detail for detail in details)
        keyset_plan = store._connection.execute(
            f"""
            EXPLAIN QUERY PLAN
            SELECT id, title, default_branch_id, workspace_json, created_at, updated_at,
                   archived_at
            FROM threads
            WHERE archived_at {archived_filter}
              AND updated_at <= ? AND (updated_at < ? OR id > ?)
            ORDER BY updated_at DESC, id ASC
            LIMIT 51
            """,
            (
                "2026-08-14T12:00:00.000Z",
                "2026-08-14T12:00:00.000Z",
                "thread-boundary",
            ),
        ).fetchall()
        keyset_details = [str(row["detail"]) for row in keyset_plan]
        assert any(
            "SEARCH" in detail and index_name in detail
            for detail in keyset_details
        )
        assert all("USE TEMP B-TREE" not in detail for detail in keyset_details)
    finally:
        store.close()


def test_thread_catalog_cursor_is_canonical_versioned_base64url() -> None:
    cursor = ThreadCatalogCursor(
        updated_at="2026-08-14T12:34:56.789Z",
        thread_id="thread_0123456789abcdef0123456789abcdef",
    )

    encoded = encode_thread_catalog_cursor(cursor)

    assert set(encoded) <= set(
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
    )
    assert decode_thread_catalog_cursor(encoded) == cursor


@pytest.mark.parametrize(
    "cursor",
    [
        "not*base64url",
        "A" * 1025,
        _encoded_cursor(
            {
                "v": 2,
                "updatedAt": "2026-08-14T12:34:56.789Z",
                "id": "thread-id",
            }
        ),
        _encoded_cursor(
            {
                "v": True,
                "updatedAt": "2026-08-14T12:34:56.789Z",
                "id": "thread-id",
            }
        ),
        _encoded_cursor(
            {
                "v": 1,
                "updatedAt": "2026-08-14T12:34:56.789Z",
                "id": "thread-id",
                "extra": True,
            }
        ),
        _encoded_cursor(
            {"v": 1, "updatedAt": "2026-02-30T12:34:56.789Z", "id": "thread-id"}
        ),
        _encoded_cursor(
            {"v": 1, "updatedAt": "2026-08-14T12:34:56Z", "id": "thread-id"}
        ),
        _encoded_cursor(
            {"v": 1, "updatedAt": "2026-08-14T12:34:56.789Z", "id": ""}
        ),
    ],
)
def test_thread_catalog_rejects_invalid_cursors_without_echoing_them(cursor: str) -> None:
    with pytest.raises(ValueError, match="thread.list cursor is invalid") as raised:
        decode_thread_catalog_cursor(cursor)

    assert cursor not in str(raised.value)
