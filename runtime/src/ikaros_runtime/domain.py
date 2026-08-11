from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Any

JsonObject = dict[str, Any]


def utc_now() -> str:
    return datetime.now(UTC).isoformat(timespec="milliseconds").replace("+00:00", "Z")


@dataclass(frozen=True, slots=True)
class ThreadSummary:
    id: str
    title: str | None
    default_branch_id: str
    created_at: str
    updated_at: str

    def to_wire(self) -> JsonObject:
        return {
            "id": self.id,
            "title": self.title,
            "defaultBranchId": self.default_branch_id,
            "createdAt": self.created_at,
            "updatedAt": self.updated_at,
        }


@dataclass(frozen=True, slots=True)
class JournalEvent:
    seq: int
    type: str
    thread_id: str | None
    branch_id: str | None
    timestamp: str
    payload: JsonObject

    def to_wire(self) -> JsonObject:
        return {
            "seq": self.seq,
            "type": self.type,
            "threadId": self.thread_id,
            "branchId": self.branch_id,
            "timestamp": self.timestamp,
            "payload": self.payload,
        }
