"""Narrow, read-only Session Item provenance capture for durable Memory."""

from __future__ import annotations

import sqlite3
from dataclasses import dataclass

from ..json_codec import loads as json_loads
from ..run_input import canonical_sha256
from .projections import item_row

SESSION_ITEM_PROVENANCE_SCHEMA_VERSION = 1


@dataclass(frozen=True, slots=True)
class SessionItemProvenanceSnapshotV1:
    thread_id: str
    turn_id: str
    item_id: str
    item_digest: str


def capture_session_item_provenance(
    connection: sqlite3.Connection,
    item_id: str,
) -> SessionItemProvenanceSnapshotV1:
    row = item_row(connection, item_id)
    if (
        row["kind"] != "message"
        or row["role"] not in {"user", "assistant"}
        or row["status"] != "completed"
    ):
        raise LookupError("Session Memory source is unavailable")
    data = json_loads(row["data_json"])
    if not isinstance(data, dict):
        raise LookupError("Session Memory source is unavailable")
    thread_id = str(row["thread_id"])
    turn_id = str(row["turn_id"])
    snapshot = {
        "schemaVersion": SESSION_ITEM_PROVENANCE_SCHEMA_VERSION,
        "threadId": thread_id,
        "branchId": str(row["branch_id"]),
        "item": {
            "id": str(row["id"]),
            "turnId": turn_id,
            "runId": str(row["run_id"]),
            "ordinal": int(row["ordinal"]),
            "kind": str(row["kind"]),
            "role": str(row["role"]),
            "status": str(row["status"]),
            "content": str(row["content"]),
            "data": data,
            "createdAt": str(row["created_at"]),
            "updatedAt": str(row["updated_at"]),
        },
    }
    return SessionItemProvenanceSnapshotV1(
        thread_id=thread_id,
        turn_id=turn_id,
        item_id=str(row["id"]),
        item_digest=canonical_sha256(snapshot),
    )


def session_item_provenance_is_available(
    connection: sqlite3.Connection,
    expected: SessionItemProvenanceSnapshotV1,
) -> bool:
    try:
        return capture_session_item_provenance(connection, expected.item_id) == expected
    except (LookupError, RuntimeError, TypeError, ValueError, sqlite3.Error):
        return False


__all__ = [
    "SESSION_ITEM_PROVENANCE_SCHEMA_VERSION",
    "SessionItemProvenanceSnapshotV1",
    "capture_session_item_provenance",
    "session_item_provenance_is_available",
]
