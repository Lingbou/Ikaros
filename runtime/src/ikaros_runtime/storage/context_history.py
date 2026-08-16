"""Paged SQLite readers for provider-visible conversation history."""

from __future__ import annotations

import sqlite3
from dataclasses import dataclass

from ..errors import ModelInputUnavailableError
from ..json_codec import loads as json_loads
from ..run_input import (
    EMPTY_FROZEN_MEMORY_CONTEXT_V1,
    ContextItemRecordV1,
    ContextSnapshotV1,
    FrozenMemoryContextV1,
    HistoryItemReferenceV1,
    RunManifestV1,
    SubmissionFrameV1,
    build_context_snapshot,
)

_FROZEN_ITEM_QUERY_CHUNK = 256


@dataclass(frozen=True, slots=True)
class ContextTurnRecordV1:
    turn_id: str
    ordinal: int
    records: tuple[ContextItemRecordV1, ...]


@dataclass(frozen=True, slots=True)
class ContextTurnPageV1:
    turns: tuple[ContextTurnRecordV1, ...]
    next_before_ordinal: int | None


def context_turn_ordinal(
    connection: sqlite3.Connection,
    *,
    branch_id: str,
    turn_id: str,
) -> int:
    row = connection.execute(
        "SELECT ordinal FROM turns WHERE id = ? AND branch_id = ?",
        (turn_id, branch_id),
    ).fetchone()
    if row is None:
        raise LookupError("context boundary turn was not found")
    return int(row["ordinal"])


def list_context_turn_page(
    connection: sqlite3.Connection,
    *,
    branch_id: str,
    before_ordinal: int,
    limit: int,
) -> ContextTurnPageV1:
    if not isinstance(limit, int) or isinstance(limit, bool) or limit < 1:
        raise ValueError("context Turn page limit must be positive")
    rows = connection.execute(
        """
        SELECT id, ordinal
        FROM turns
        WHERE branch_id = ? AND ordinal < ?
        ORDER BY ordinal DESC
        LIMIT ?
        """,
        (branch_id, before_ordinal, limit + 1),
    ).fetchall()
    has_more = len(rows) > limit
    selected_rows = rows[:limit]
    if not selected_rows:
        return ContextTurnPageV1((), None)

    turn_ids = tuple(str(row["id"]) for row in selected_rows)
    records_by_turn = _records_for_turn_ids(connection, turn_ids)
    turns = tuple(
        ContextTurnRecordV1(
            turn_id=str(row["id"]),
            ordinal=int(row["ordinal"]),
            records=tuple(records_by_turn.get(str(row["id"]), ())),
        )
        for row in selected_rows
    )
    return ContextTurnPageV1(
        turns=turns,
        next_before_ordinal=(turns[-1].ordinal if has_more else None),
    )


def load_current_run_context(
    connection: sqlite3.Connection,
    *,
    run_id: str,
) -> tuple[ContextItemRecordV1, ...]:
    rows = connection.execute(
        """
        SELECT i.id, i.turn_id, i.run_id, i.kind, i.role, i.status, i.content,
               i.data_json, r.status AS run_status
        FROM items i
        JOIN runs r ON r.id = i.run_id
        WHERE i.run_id = ?
        ORDER BY i.ordinal ASC, i.id ASC
        """,
        (run_id,),
    ).fetchall()
    return tuple(_record_from_row(row) for row in rows if _row_is_context_eligible(row, run_id))


def load_context_for_snapshot(
    connection: sqlite3.Connection,
    *,
    run_id: str,
    snapshot: ContextSnapshotV1,
) -> tuple[ContextItemRecordV1, ...]:
    frozen_ids = tuple(reference.item_id for reference in snapshot.history_items)
    frozen_by_id: dict[str, ContextItemRecordV1] = {}
    for offset in range(0, len(frozen_ids), _FROZEN_ITEM_QUERY_CHUNK):
        chunk = frozen_ids[offset : offset + _FROZEN_ITEM_QUERY_CHUNK]
        placeholders = ",".join("?" for _ in chunk)
        rows = connection.execute(
            f"""
            SELECT i.id, i.turn_id, i.run_id, i.kind, i.role, i.status, i.content,
                   i.data_json, r.status AS run_status
            FROM items i
            JOIN runs r ON r.id = i.run_id
            WHERE i.id IN ({placeholders})
            """,
            chunk,
        ).fetchall()
        for row in rows:
            record = _record_from_row(row)
            if _row_is_context_eligible(row, run_id):
                frozen_by_id[record.item_id] = record

    try:
        frozen = tuple(frozen_by_id[item_id] for item_id in frozen_ids)
    except KeyError:
        raise ModelInputUnavailableError("model_input_unavailable") from None
    if tuple(HistoryItemReferenceV1.from_record(record) for record in frozen) != (
        snapshot.history_items
    ):
        raise ModelInputUnavailableError("model_input_unavailable")

    current = load_current_run_context(connection, run_id=run_id)
    frozen_id_set = set(frozen_ids)
    appended = tuple(record for record in current if record.item_id not in frozen_id_set)
    return (*frozen, *appended)


def select_context_snapshot(
    connection: sqlite3.Connection,
    *,
    branch_id: str,
    turn_id: str,
    run_id: str,
    frame: SubmissionFrameV1,
    manifest: RunManifestV1,
    memory_context: FrozenMemoryContextV1 = EMPTY_FROZEN_MEMORY_CONTEXT_V1,
) -> tuple[ContextSnapshotV1, tuple[ContextItemRecordV1, ...]]:
    # Local import avoids the package-level agent -> loop -> storage dependency cycle.
    from ..agent.history import HISTORY_TURN_PAGE_SIZE_V1, HistorySelectorV1

    try:
        ordinal = context_turn_ordinal(connection, branch_id=branch_id, turn_id=turn_id)
        selector = HistorySelectorV1(
            frame=frame,
            current_run_id=run_id,
            current_turn_id=turn_id,
            current_turn_ordinal=ordinal,
            current_records=load_current_run_context(connection, run_id=run_id),
            memory_context=memory_context,
        )
    except (LookupError, TypeError, ValueError):
        raise ModelInputUnavailableError("model_input_unavailable") from None
    before_ordinal = ordinal
    while not selector.stopped:
        page = list_context_turn_page(
            connection,
            branch_id=branch_id,
            before_ordinal=before_ordinal,
            limit=HISTORY_TURN_PAGE_SIZE_V1,
        )
        for turn in page.turns:
            try:
                accepted = selector.consider_turn(
                    turn_id=turn.turn_id,
                    ordinal=turn.ordinal,
                    records=turn.records,
                )
            except ValueError:
                raise ModelInputUnavailableError("model_input_unavailable") from None
            if not accepted:
                break
        if selector.stopped or page.next_before_ordinal is None:
            break
        before_ordinal = page.next_before_ordinal

    selection = selector.finish()
    snapshot = build_context_snapshot(
        selection.records,
        current_run_id=run_id,
        frame=frame,
        selection_version=manifest.context_selection_version,
        maximum_characters=selection.maximum_characters,
        reserved_current_run_characters=selection.reserved_current_run_characters,
        omissions=selection.omissions,
        memory_context=memory_context,
    )
    return snapshot, selection.records


def _records_for_turn_ids(
    connection: sqlite3.Connection,
    turn_ids: tuple[str, ...],
) -> dict[str, list[ContextItemRecordV1]]:
    placeholders = ",".join("?" for _ in turn_ids)
    rows = connection.execute(
        f"""
        SELECT i.id, i.turn_id, i.run_id, i.kind, i.role, i.status, i.content,
               i.data_json, r.status AS run_status, r.created_at AS run_created_at
        FROM items i
        JOIN runs r ON r.id = i.run_id
        WHERE i.turn_id IN ({placeholders})
        ORDER BY i.turn_id ASC, run_created_at ASC, i.run_id ASC, i.ordinal ASC, i.id ASC
        """,
        turn_ids,
    ).fetchall()
    grouped: dict[str, list[ContextItemRecordV1]] = {}
    for row in rows:
        if not _row_is_context_eligible(row, None):
            continue
        record = _record_from_row(row)
        grouped.setdefault(record.turn_id, []).append(record)
    return grouped


def _row_is_context_eligible(row: sqlite3.Row, current_run_id: str | None) -> bool:
    kind = str(row["kind"])
    role = str(row["role"]) if row["role"] is not None else None
    status = str(row["status"])
    if kind == "message":
        return status == "completed" and role in {"user", "assistant"}
    return (
        kind in {"tool_call", "tool_result"}
        and status in {"completed", "failed"}
        and (str(row["run_id"]) == current_run_id or str(row["run_status"]) == "completed")
    )


def _record_from_row(row: sqlite3.Row) -> ContextItemRecordV1:
    try:
        record = ContextItemRecordV1(
            item_id=str(row["id"]),
            turn_id=str(row["turn_id"]),
            run_id=str(row["run_id"]),
            kind=str(row["kind"]),
            role=str(row["role"]) if row["role"] is not None else None,
            content=str(row["content"]),
            data=json_loads(row["data_json"]),
        )
        _ = record.characters
        return record
    except (TypeError, ValueError):
        raise ModelInputUnavailableError("model_input_unavailable") from None


__all__ = [
    "ContextTurnPageV1",
    "ContextTurnRecordV1",
    "context_turn_ordinal",
    "list_context_turn_page",
    "load_context_for_snapshot",
    "load_current_run_context",
    "select_context_snapshot",
]
