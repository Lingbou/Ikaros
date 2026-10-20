from __future__ import annotations

import sqlite3
from dataclasses import dataclass
from typing import Any

from ..domain import JsonObject, ThreadSummary
from ..json_codec import loads as json_loads
from .cursor import decode_cursor, encode_cursor
from .journal import latest_sequence
from .projections import workspace_from_json

TURN_HISTORY_DEFAULT_LIMIT = 50
TURN_HISTORY_MAX_LIMIT = 100

_CURSOR_VERSION = 1
_MAX_CURSOR_LENGTH = 2048
_MAX_RECORD_ID_LENGTH = 200
_MAX_SQL_PARAMETERS = 500
_INVALID_CURSOR_MESSAGE = "turn.list cursor is invalid"
_TURN_HISTORY_CURSOR_KEYS = {"v", "threadId", "branchId", "ordinal"}


@dataclass(frozen=True, slots=True)
class ThreadMetadata:
    thread: ThreadSummary
    snapshot_seq: int

    def to_wire(self) -> JsonObject:
        return {
            "thread": self.thread.to_wire(),
            "snapshotSeq": self.snapshot_seq,
        }


@dataclass(frozen=True, slots=True)
class TurnHistoryCursor:
    thread_id: str
    branch_id: str
    ordinal: int


@dataclass(frozen=True, slots=True)
class RunHistoryRecord:
    id: str
    turn_id: str
    provider_id: str
    model_id: str
    execution_policy: str
    status: str
    created_at: str
    started_at: str | None
    model_calls: int
    compactions: int
    settled_at: str | None
    reason_code: str | None
    items: tuple[ItemHistoryRecord, ...]

    def to_wire(self) -> JsonObject:
        return {
            "id": self.id,
            "turnId": self.turn_id,
            "providerId": self.provider_id,
            "modelId": self.model_id,
            "executionPolicy": self.execution_policy,
            "status": self.status,
            "createdAt": self.created_at,
            "startedAt": self.started_at,
            "modelCalls": self.model_calls,
            "compactions": self.compactions,
            "settledAt": self.settled_at,
            "reasonCode": self.reason_code,
            "items": [item.to_wire() for item in self.items],
        }


@dataclass(frozen=True, slots=True)
class ItemHistoryRecord:
    id: str
    turn_id: str
    run_id: str
    ordinal: int
    kind: str
    role: str | None
    status: str
    content: str
    data: JsonObject
    created_at: str
    updated_at: str

    def to_wire(self) -> JsonObject:
        return {
            "id": self.id,
            "turnId": self.turn_id,
            "runId": self.run_id,
            "ordinal": self.ordinal,
            "kind": self.kind,
            "role": self.role,
            "status": self.status,
            "content": self.content,
            "data": self.data,
            "createdAt": self.created_at,
            "updatedAt": self.updated_at,
        }


@dataclass(frozen=True, slots=True)
class TurnHistoryRecord:
    id: str
    thread_id: str
    branch_id: str
    ordinal: int
    status: str
    created_at: str
    updated_at: str
    runs: tuple[RunHistoryRecord, ...]

    def to_wire(self) -> JsonObject:
        return {
            "id": self.id,
            "threadId": self.thread_id,
            "branchId": self.branch_id,
            "ordinal": self.ordinal,
            "status": self.status,
            "createdAt": self.created_at,
            "updatedAt": self.updated_at,
            "runs": [run.to_wire() for run in self.runs],
        }


@dataclass(frozen=True, slots=True)
class TurnHistoryPage:
    turns: tuple[TurnHistoryRecord, ...]
    next_cursor: str | None
    has_more: bool
    snapshot_seq: int

    def to_wire(self) -> JsonObject:
        return {
            "turns": [turn.to_wire() for turn in self.turns],
            "nextCursor": self.next_cursor,
            "hasMore": self.has_more,
            "snapshotSeq": self.snapshot_seq,
        }


def get_thread_metadata(
    connection: sqlite3.Connection,
    *,
    thread_id: str,
) -> ThreadMetadata:
    _require_idle_connection(connection)
    connection.execute("BEGIN")
    try:
        snapshot_seq = latest_sequence(connection)
        row = connection.execute(
            """
            SELECT id, title, default_branch_id, workspace_json, created_at, updated_at,
                   archived_at
            FROM threads
            WHERE id = ?
            """,
            (thread_id,),
        ).fetchone()
        connection.commit()
    except BaseException:
        connection.rollback()
        raise
    if row is None:
        raise LookupError("thread was not found")
    return ThreadMetadata(thread=_thread_from_row(row), snapshot_seq=snapshot_seq)


def list_turn_history_page(
    connection: sqlite3.Connection,
    *,
    thread_id: str,
    branch_id: str,
    cursor: str | None,
    limit: int,
) -> TurnHistoryPage:
    if isinstance(limit, bool) or not isinstance(limit, int):
        raise ValueError("turn.list limit must be an integer")
    if limit < 1 or limit > TURN_HISTORY_MAX_LIMIT:
        raise ValueError(f"turn.list limit must be between 1 and {TURN_HISTORY_MAX_LIMIT}")
    boundary = (
        decode_turn_history_cursor(cursor, thread_id=thread_id, branch_id=branch_id)
        if cursor is not None
        else None
    )

    _require_idle_connection(connection)
    connection.execute("BEGIN")
    try:
        owner = connection.execute(
            "SELECT 1 FROM branches WHERE id = ? AND thread_id = ?",
            (branch_id, thread_id),
        ).fetchone()
        if owner is None:
            raise LookupError("thread or branch was not found")
        snapshot_seq = latest_sequence(connection)
        rows = _turn_rows(
            connection,
            thread_id=thread_id,
            branch_id=branch_id,
            boundary=boundary,
            limit=limit,
        )
        returned_rows = rows[:limit]
        turn_ids = tuple(str(row["id"]) for row in returned_rows)
        run_rows = _run_rows_for_turns(connection, turn_ids)
        run_ids = tuple(str(row["id"]) for row in run_rows)
        items_by_run = _items_for_runs(connection, run_ids)
        runs_by_turn = _runs_by_turn(run_rows, items_by_run=items_by_run)
        connection.commit()
    except BaseException:
        connection.rollback()
        raise

    has_more = len(rows) > limit
    turns = tuple(
        _turn_from_row(
            row,
            runs=runs_by_turn.get(str(row["id"]), ()),
        )
        for row in reversed(returned_rows)
    )
    next_cursor = None
    if has_more:
        last_row = returned_rows[-1]
        next_cursor = encode_turn_history_cursor(
            TurnHistoryCursor(
                thread_id=thread_id,
                branch_id=branch_id,
                ordinal=int(last_row["ordinal"]),
            )
        )
    return TurnHistoryPage(
        turns=turns,
        next_cursor=next_cursor,
        has_more=has_more,
        snapshot_seq=snapshot_seq,
    )


def encode_turn_history_cursor(cursor: TurnHistoryCursor) -> str:
    _validate_cursor(cursor)
    return encode_cursor(
        {
            "v": _CURSOR_VERSION,
            "threadId": cursor.thread_id,
            "branchId": cursor.branch_id,
            "ordinal": cursor.ordinal,
        }
    )


def decode_turn_history_cursor(
    value: str,
    *,
    thread_id: str,
    branch_id: str,
) -> TurnHistoryCursor:
    def _parse(payload: dict[str, Any]) -> TurnHistoryCursor:
        return TurnHistoryCursor(
            thread_id=payload["threadId"],
            branch_id=payload["branchId"],
            ordinal=payload["ordinal"],
        )

    def _validate(cursor: TurnHistoryCursor) -> None:
        _validate_cursor(cursor)
        if cursor.thread_id != thread_id or cursor.branch_id != branch_id:
            raise ValueError(_INVALID_CURSOR_MESSAGE)

    return decode_cursor(
        value,
        expected_version=_CURSOR_VERSION,
        expected_keys=_TURN_HISTORY_CURSOR_KEYS,
        max_length=_MAX_CURSOR_LENGTH,
        invalid_message=_INVALID_CURSOR_MESSAGE,
        parse_payload=_parse,
        validate_cursor=_validate,
        reencode=encode_turn_history_cursor,
    )


def _turn_rows(
    connection: sqlite3.Connection,
    *,
    thread_id: str,
    branch_id: str,
    boundary: TurnHistoryCursor | None,
    limit: int,
) -> list[sqlite3.Row]:
    if boundary is None:
        return connection.execute(
            """
            SELECT id, thread_id, branch_id, ordinal, status, created_at, updated_at
            FROM turns
            WHERE thread_id = ? AND branch_id = ?
            ORDER BY ordinal DESC
            LIMIT ?
            """,
            (thread_id, branch_id, limit + 1),
        ).fetchall()
    return connection.execute(
        """
        SELECT id, thread_id, branch_id, ordinal, status, created_at, updated_at
        FROM turns
        WHERE thread_id = ? AND branch_id = ?
          AND ordinal < ?
        ORDER BY ordinal DESC
        LIMIT ?
        """,
        (
            thread_id,
            branch_id,
            boundary.ordinal,
            limit + 1,
        ),
    ).fetchall()


def _run_rows_for_turns(
    connection: sqlite3.Connection,
    turn_ids: tuple[str, ...],
) -> list[sqlite3.Row]:
    if not turn_ids:
        return []
    placeholders = ",".join("?" for _turn_id in turn_ids)
    return connection.execute(
        f"""
        SELECT r.id, r.turn_id, r.provider_id, r.model_id, r.execution_policy, r.status,
               r.created_at, r.started_at, r.settled_at, r.reason_code,
               (SELECT count(*) FROM model_calls mc
                WHERE mc.run_id = r.id AND mc.purpose = 'execution') AS model_calls,
               (SELECT count(*) FROM events e
                WHERE e.run_id = r.id AND e.event_type = 'context.compacted') AS compactions
        FROM runs r JOIN run_configs rc ON rc.run_id = r.id
        WHERE turn_id IN ({placeholders})
        ORDER BY turn_id ASC, created_at ASC, id ASC
        """,
        turn_ids,
    ).fetchall()


def _runs_by_turn(
    rows: list[sqlite3.Row],
    *,
    items_by_run: dict[str, tuple[ItemHistoryRecord, ...]],
) -> dict[str, tuple[RunHistoryRecord, ...]]:
    grouped: dict[str, list[RunHistoryRecord]] = {}
    for row in rows:
        turn_id = str(row["turn_id"])
        records = grouped.setdefault(turn_id, [])
        run_id = str(row["id"])
        records.append(
            RunHistoryRecord(
                id=run_id,
                turn_id=turn_id,
                provider_id=str(row["provider_id"]),
                model_id=str(row["model_id"]),
                execution_policy=str(row["execution_policy"]),
                status=str(row["status"]),
                created_at=str(row["created_at"]),
                started_at=(str(row["started_at"]) if row["started_at"] is not None else None),
                model_calls=int(row["model_calls"]),
                compactions=int(row["compactions"]),
                settled_at=(str(row["settled_at"]) if row["settled_at"] is not None else None),
                reason_code=(str(row["reason_code"]) if row["reason_code"] is not None else None),
                items=items_by_run.get(run_id, ()),
            )
        )
    return {turn_id: tuple(records) for turn_id, records in grouped.items()}


def _items_for_runs(
    connection: sqlite3.Connection,
    run_ids: tuple[str, ...],
) -> dict[str, tuple[ItemHistoryRecord, ...]]:
    if not run_ids:
        return {}
    grouped: dict[str, list[ItemHistoryRecord]] = {}
    for offset in range(0, len(run_ids), _MAX_SQL_PARAMETERS):
        batch = run_ids[offset : offset + _MAX_SQL_PARAMETERS]
        placeholders = ",".join("?" for _run_id in batch)
        rows = connection.execute(
            f"""
            SELECT id, turn_id, run_id, ordinal, kind, role, status, content,
                   data_json, created_at, updated_at
            FROM items
            WHERE run_id IN ({placeholders})
            ORDER BY run_id ASC, ordinal ASC, id ASC
            """,
            batch,
        ).fetchall()
        for row in rows:
            turn_id = str(row["turn_id"])
            run_id = str(row["run_id"])
            grouped.setdefault(run_id, []).append(
                ItemHistoryRecord(
                    id=str(row["id"]),
                    turn_id=turn_id,
                    run_id=run_id,
                    ordinal=int(row["ordinal"]),
                    kind=str(row["kind"]),
                    role=str(row["role"]) if row["role"] is not None else None,
                    status=str(row["status"]),
                    content=str(row["content"]),
                    data=_json_object(row["data_json"]),
                    created_at=str(row["created_at"]),
                    updated_at=str(row["updated_at"]),
                )
            )
    return {run_id: tuple(records) for run_id, records in grouped.items()}


def _turn_from_row(
    row: sqlite3.Row,
    *,
    runs: tuple[RunHistoryRecord, ...],
) -> TurnHistoryRecord:
    return TurnHistoryRecord(
        id=str(row["id"]),
        thread_id=str(row["thread_id"]),
        branch_id=str(row["branch_id"]),
        ordinal=int(row["ordinal"]),
        status=str(row["status"]),
        created_at=str(row["created_at"]),
        updated_at=str(row["updated_at"]),
        runs=runs,
    )


def _thread_from_row(row: sqlite3.Row) -> ThreadSummary:
    return ThreadSummary(
        id=str(row["id"]),
        title=row["title"],
        default_branch_id=str(row["default_branch_id"]),
        workspace=workspace_from_json(row["workspace_json"]),
        created_at=str(row["created_at"]),
        updated_at=str(row["updated_at"]),
        archived_at=row["archived_at"],
    )


def _json_object(value: str | bytes | bytearray) -> JsonObject:
    decoded = json_loads(value)
    if not isinstance(decoded, dict):
        raise RuntimeError("history item data is not an object")
    return decoded


def _validate_cursor(cursor: TurnHistoryCursor) -> None:
    _validate_record_id(cursor.thread_id)
    _validate_record_id(cursor.branch_id)
    if isinstance(cursor.ordinal, bool) or not isinstance(cursor.ordinal, int):
        raise ValueError(_INVALID_CURSOR_MESSAGE)
    if cursor.ordinal < 1 or cursor.ordinal > 2**63 - 1:
        raise ValueError(_INVALID_CURSOR_MESSAGE)


def _validate_record_id(value: object) -> None:
    if (
        not isinstance(value, str)
        or not value
        or len(value) > _MAX_RECORD_ID_LENGTH
        or value != value.strip()
        or any(ord(character) < 0x20 or ord(character) == 0x7F for character in value)
    ):
        raise ValueError(_INVALID_CURSOR_MESSAGE)


def _require_idle_connection(connection: sqlite3.Connection) -> None:
    if connection.in_transaction:
        raise RuntimeError("history reads require an idle SQLite connection")


__all__ = [
    "TURN_HISTORY_DEFAULT_LIMIT",
    "TURN_HISTORY_MAX_LIMIT",
    "ThreadMetadata",
    "TurnHistoryCursor",
    "TurnHistoryPage",
    "decode_turn_history_cursor",
    "encode_turn_history_cursor",
    "get_thread_metadata",
    "list_turn_history_page",
]
