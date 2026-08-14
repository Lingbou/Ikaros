from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Any

JsonObject = dict[str, Any]
JOURNAL_EVENT_SCHEMA_VERSION = 1


def utc_now() -> str:
    return datetime.now(UTC).isoformat(timespec="milliseconds").replace("+00:00", "Z")


@dataclass(frozen=True, slots=True)
class WorkspaceSummary:
    id: str
    name: str
    root_uri: str | None

    def to_wire(self) -> JsonObject:
        return {
            "id": self.id,
            "name": self.name,
            "rootUri": self.root_uri,
        }


@dataclass(frozen=True, slots=True)
class ThreadSummary:
    id: str
    title: str | None
    default_branch_id: str
    workspace: WorkspaceSummary | None
    created_at: str
    updated_at: str
    archived_at: str | None

    def to_wire(self) -> JsonObject:
        return {
            "id": self.id,
            "title": self.title,
            "defaultBranchId": self.default_branch_id,
            "workspace": self.workspace.to_wire() if self.workspace is not None else None,
            "createdAt": self.created_at,
            "updatedAt": self.updated_at,
            "archivedAt": self.archived_at,
        }


@dataclass(frozen=True, slots=True)
class JournalEvent:
    seq: int
    schema_version: int
    type: str
    thread_id: str | None
    branch_id: str | None
    turn_id: str | None
    run_id: str | None
    item_id: str | None
    timestamp: str
    payload: JsonObject

    def to_wire(self) -> JsonObject:
        return {
            "seq": self.seq,
            "schemaVersion": self.schema_version,
            "type": self.type,
            "threadId": self.thread_id,
            "branchId": self.branch_id,
            "turnId": self.turn_id,
            "runId": self.run_id,
            "itemId": self.item_id,
            "timestamp": self.timestamp,
            "payload": self.payload,
        }


@dataclass(frozen=True, slots=True)
class RunDescriptor:
    id: str
    turn_id: str
    thread_id: str
    branch_id: str
    provider_id: str
    model_id: str
    execution_policy: str
    workspace: WorkspaceSummary | None


@dataclass(frozen=True, slots=True)
class ContextItem:
    kind: str
    role: str | None
    content: str
    data: JsonObject


@dataclass(frozen=True, slots=True)
class PreparedTurn:
    turn_id: str
    run_id: str
    thread_id: str
    branch_id: str
    initial_events: tuple[JournalEvent, ...]
    newly_created: bool = True


@dataclass(frozen=True, slots=True)
class RecoveryPlan:
    queued_run_ids: tuple[str, ...]


@dataclass(frozen=True, slots=True)
class ModelUsage:
    input_tokens: int
    output_tokens: int
    total_tokens: int
    cached_input_tokens: int | None = None
    reasoning_output_tokens: int | None = None


@dataclass(frozen=True, slots=True)
class DailyUsageBucket:
    start_date: str
    tokens: int

    def to_wire(self) -> JsonObject:
        return {"startDate": self.start_date, "tokens": self.tokens}


@dataclass(frozen=True, slots=True)
class UsageSummary:
    lifetime_tokens: int | None
    peak_daily_tokens: int | None
    longest_running_turn_sec: int | None
    current_streak_days: int
    longest_streak_days: int

    def to_wire(self) -> JsonObject:
        return {
            "lifetimeTokens": self.lifetime_tokens,
            "peakDailyTokens": self.peak_daily_tokens,
            "longestRunningTurnSec": self.longest_running_turn_sec,
            "currentStreakDays": self.current_streak_days,
            "longestStreakDays": self.longest_streak_days,
        }


@dataclass(frozen=True, slots=True)
class UsageSnapshot:
    summary: UsageSummary
    daily_usage_buckets: tuple[DailyUsageBucket, ...]

    def to_wire(self) -> JsonObject:
        return {
            "summary": self.summary.to_wire(),
            "dailyUsageBuckets": [bucket.to_wire() for bucket in self.daily_usage_buckets],
        }


@dataclass(frozen=True, slots=True)
class CommandOutcome:
    result: JsonObject
    events_after_ack: tuple[JournalEvent, ...] = ()
    run_after_ack: str | None = None
    cancel_after_ack: str | None = None
