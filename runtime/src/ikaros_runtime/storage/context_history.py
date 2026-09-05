"""Paged SQLite readers for provider-visible conversation history."""

from __future__ import annotations

import sqlite3
from dataclasses import dataclass, replace

from ..errors import ModelInputUnavailableError
from ..history_status import (
    EMPTY_HISTORY_STATUS_V1,
    FrozenHistoryStatusV1,
    HistoryRunStatusV1,
    historical_run_status,
)
from ..json_codec import loads as json_loads
from ..run_input import (
    EMPTY_FROZEN_MEMORY_CONTEXT_V1,
    LEGACY_CONTEXT_SELECTION_VERSION,
    ContextItemRecordV1,
    ContextSnapshotV1,
    FrozenMemoryContextV1,
    HistoryItemReferenceV1,
    RunManifestV1,
    SubmissionFrameV1,
    build_context_snapshot,
    canonical_json,
)

_FROZEN_ITEM_QUERY_CHUNK = 256


@dataclass(frozen=True, slots=True)
class ContextTurnRecordV1:
    turn_id: str
    ordinal: int
    records: tuple[ContextItemRecordV1, ...]
    run_statuses: tuple[HistoryRunStatusV1, ...] = ()


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
    selection_version: str = LEGACY_CONTEXT_SELECTION_VERSION,
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
    records_by_turn = _records_for_turn_ids(
        connection, turn_ids, selection_version=selection_version
    )
    statuses_by_turn = (
        _statuses_for_turn_ids(connection, turn_ids)
        if selection_version != LEGACY_CONTEXT_SELECTION_VERSION
        else {}
    )
    turns = tuple(
        ContextTurnRecordV1(
            turn_id=str(row["id"]),
            ordinal=int(row["ordinal"]),
            records=tuple(records_by_turn.get(str(row["id"]), ())),
            run_statuses=statuses_by_turn.get(str(row["id"]), ()),
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
            record = _historical_record_from_row(row, snapshot.selection_version)
            if _row_is_context_eligible(row, run_id, snapshot.selection_version):
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
    if snapshot.history_status.runs:
        _validate_frozen_status(connection, run_id=run_id, status=snapshot.history_status)
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

    is_v2 = manifest.context_selection_version != LEGACY_CONTEXT_SELECTION_VERSION
    history_status = EMPTY_HISTORY_STATUS_V1
    nearest_turn_id: str | None = None
    omission_notice = EMPTY_HISTORY_STATUS_V1
    omission_notice_characters = 0
    first_page: ContextTurnPageV1 | None = None
    try:
        ordinal = context_turn_ordinal(connection, branch_id=branch_id, turn_id=turn_id)
        if is_v2:
            first_page = list_context_turn_page(
                connection,
                branch_id=branch_id,
                before_ordinal=ordinal,
                limit=HISTORY_TURN_PAGE_SIZE_V1,
                selection_version=manifest.context_selection_version,
            )
            if first_page.turns:
                nearest_turn_id = first_page.turns[0].turn_id
                omission_notice = FrozenHistoryStatusV1(
                    tuple(
                        replace(run, details="omitted_by_budget")
                        for run in first_page.turns[0].run_statuses
                    )
                )
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
        page = first_page or list_context_turn_page(
            connection,
            branch_id=branch_id,
            before_ordinal=before_ordinal,
            limit=HISTORY_TURN_PAGE_SIZE_V1,
            selection_version=manifest.context_selection_version,
        )
        first_page = None
        for turn in page.turns:
            # Pages arrive newest-first; the frozen block is chronological. Charge
            # its actual included size before falling back to an omission notice.
            previous = tuple(run for run in history_status.runs if run.turn_id != turn.turn_id)
            candidate_status = FrozenHistoryStatusV1((*turn.run_statuses, *previous))
            try:
                accepted = selector.consider_turn(
                    turn_id=turn.turn_id,
                    ordinal=turn.ordinal,
                    records=turn.records,
                    additional_characters=candidate_status.characters - history_status.characters,
                )
            except ValueError:
                raise ModelInputUnavailableError("model_input_unavailable") from None
            if not accepted:
                if turn.turn_id == nearest_turn_id:
                    history_status = omission_notice
                    omission_notice_characters = omission_notice.characters
                break
            history_status = candidate_status
        if selector.stopped or page.next_before_ordinal is None:
            break
        before_ordinal = page.next_before_ordinal

    selection = selector.finish(additional_characters=omission_notice_characters)
    snapshot = build_context_snapshot(
        selection.records,
        current_run_id=run_id,
        frame=frame,
        selection_version=manifest.context_selection_version,
        maximum_characters=selection.maximum_characters,
        reserved_current_run_characters=selection.reserved_current_run_characters,
        omissions=selection.omissions,
        memory_context=memory_context,
        history_status=history_status,
    )
    return snapshot, selection.records


def _records_for_turn_ids(
    connection: sqlite3.Connection,
    turn_ids: tuple[str, ...],
    *,
    selection_version: str = LEGACY_CONTEXT_SELECTION_VERSION,
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
        if not _row_is_context_eligible(row, None, selection_version):
            continue
        record = _historical_record_from_row(row, selection_version)
        grouped.setdefault(record.turn_id, []).append(record)
    return grouped


def _row_is_context_eligible(
    row: sqlite3.Row,
    current_run_id: str | None,
    selection_version: str = LEGACY_CONTEXT_SELECTION_VERSION,
) -> bool:
    kind = str(row["kind"])
    role = str(row["role"]) if row["role"] is not None else None
    status = str(row["status"])
    if kind == "message":
        return status == "completed" and role in {"user", "assistant"}
    if selection_version != LEGACY_CONTEXT_SELECTION_VERSION:
        return (
            kind in {"tool_call", "tool_result"}
            and status in {"completed", "failed", "cancelled"}
            and (
                str(row["run_id"]) == current_run_id
                or str(row["run_status"]) in {"completed", "failed", "cancelled"}
            )
        )
    return (
        kind in {"tool_call", "tool_result"}
        and status in {"completed", "failed"}
        and (str(row["run_id"]) == current_run_id or str(row["run_status"]) == "completed")
    )


def _statuses_for_turn_ids(
    connection: sqlite3.Connection, turn_ids: tuple[str, ...]
) -> dict[str, tuple[HistoryRunStatusV1, ...]]:
    placeholders = ",".join("?" for _ in turn_ids)
    rows = connection.execute(
        f"""SELECT id, turn_id, status, reason_code FROM runs
        WHERE turn_id IN ({placeholders}) AND status IN ('failed', 'cancelled')
        ORDER BY turn_id, created_at, id""",
        turn_ids,
    ).fetchall()
    grouped: dict[str, list[HistoryRunStatusV1]] = {}
    for row in rows:
        turn_id = str(row["turn_id"])
        grouped.setdefault(turn_id, []).append(
            historical_run_status(
                turn_id=turn_id,
                run_id=str(row["id"]),
                status=str(row["status"]),
                reason_code=str(row["reason_code"]) if row["reason_code"] is not None else None,
            )
        )
    return {turn_id: tuple(runs) for turn_id, runs in grouped.items()}


def _validate_frozen_status(
    connection: sqlite3.Connection, *, run_id: str, status: FrozenHistoryStatusV1
) -> None:
    for offset in range(0, len(status.runs), _FROZEN_ITEM_QUERY_CHUNK):
        requested = status.runs[offset : offset + _FROZEN_ITEM_QUERY_CHUNK]
        placeholders = ",".join("?" for _ in requested)
        rows = connection.execute(
            f"""SELECT r.id, r.turn_id, r.status, r.reason_code
            FROM runs r JOIN turns t ON t.id = r.turn_id
            JOIN runs current_run ON current_run.id = ?
            JOIN turns current_turn ON current_turn.id = current_run.turn_id
            WHERE r.id IN ({placeholders}) AND t.branch_id = current_turn.branch_id
              AND t.ordinal < current_turn.ordinal""",
            (run_id, *(run.run_id for run in requested)),
        ).fetchall()
        by_id = {str(row["id"]): row for row in rows}
        for run in requested:
            row = by_id.get(run.run_id)
            if row is None or (row["turn_id"], row["status"], row["reason_code"]) != (
                run.turn_id,
                run.status,
                run.reason_code,
            ):
                raise ModelInputUnavailableError("model_input_unavailable")


def _historical_record_from_row(
    row: sqlite3.Row,
    selection_version: str,
) -> ContextItemRecordV1:
    record = _record_from_row(row)
    if selection_version == LEGACY_CONTEXT_SELECTION_VERSION or record.kind != "tool_result":
        return record
    result = record.data.get("result")
    if not isinstance(result, dict):
        raise ModelInputUnavailableError("model_input_unavailable")
    interrupted = (
        ((row["status"] == "cancelled" or result.get("cancelled") is True)
         and result.get("ok") is not True)
        or result.get("errorCode") == "runtime_interrupted"
        or (
            row["status"] == "failed"
            and result.get("output") == ""
            and result.get("exitCode") is None
        )
    )
    if not interrupted:
        return record
    # Legacy terminalization can say "not started" even when cancellation was
    # observed after a side effect. Keep recorded partial output, but make that
    # ambiguous execution outcome explicit only in the V2 model projection.
    normalized = dict(result)
    if normalized.get("output") in (
        "",
        "Tool call was not started because the Run was cancelled.",
    ):
        normalized["output"] = "Tool execution was interrupted; its execution outcome is unknown."
    normalized["executionOutcome"] = "unknown"
    normalized["executionNotice"] = (
        "The tool may already have changed files or external state. "
        "Inspect the current state before continuing."
    )
    return replace(
        record,
        content=canonical_json(normalized),
        data={**record.data, "result": normalized},
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
