"""Body-free, frozen descriptions of incomplete historical Runs."""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Literal, cast

from .domain import JsonObject
from .json_codec import dumps as json_dumps

HISTORY_STATUS_PREAMBLE_V1 = (
    "Runtime history status (contextual data): failed or cancelled Runs did not complete. "
    "Earlier tools may already have changed files or external state; failed, cancelled, "
    "or missing results do not prove that an action was not executed. "
    "Do not replay old Tool Calls. If details are omitted, inspect the current state "
    "or ask for missing information before continuing."
)


@dataclass(frozen=True, slots=True)
class HistoryRunStatusV1:
    turn_id: str
    run_id: str
    status: Literal["failed", "cancelled"]
    reason_code: str | None
    details: Literal["included", "omitted_by_budget"] = "included"

    def __post_init__(self) -> None:
        for value in (self.turn_id, self.run_id):
            if not isinstance(value, str) or not value or len(value) > 200:
                raise ValueError("history status reference is invalid")
        if self.status not in {"failed", "cancelled"}:
            raise ValueError("history status outcome is invalid")
        if self.reason_code is not None and (
            not isinstance(self.reason_code, str)
            or not self.reason_code
            or len(self.reason_code) > 200
            or re.fullmatch(r"[A-Za-z0-9_.-]+", self.reason_code) is None
        ):
            raise ValueError("history status reason code is invalid")
        if self.details not in {"included", "omitted_by_budget"}:
            raise ValueError("history status detail selection is invalid")

    def to_wire(self) -> JsonObject:
        return {
            "turnId": self.turn_id,
            "runId": self.run_id,
            "status": self.status,
            "reasonCode": self.reason_code,
            "details": self.details,
        }

    @classmethod
    def from_wire(cls, value: object) -> HistoryRunStatusV1:
        if not isinstance(value, dict) or set(value) != {
            "turnId",
            "runId",
            "status",
            "reasonCode",
            "details",
        }:
            raise ValueError("history status record is invalid")
        return cls(
            turn_id=value["turnId"],
            run_id=value["runId"],
            status=value["status"],
            reason_code=value["reasonCode"],
            details=value["details"],
        )


@dataclass(frozen=True, slots=True)
class FrozenHistoryStatusV1:
    runs: tuple[HistoryRunStatusV1, ...] = ()

    def __post_init__(self) -> None:
        object.__setattr__(self, "runs", tuple(self.runs))
        if any(not isinstance(run, HistoryRunStatusV1) for run in self.runs):
            raise ValueError("history status records are invalid")
        if len({run.run_id for run in self.runs}) != len(self.runs):
            raise ValueError("history status contains duplicate Runs")

    @property
    def content(self) -> str:
        if not self.runs:
            return ""
        payload = {"version": 1, "runs": [run.to_wire() for run in self.runs]}
        return (
            HISTORY_STATUS_PREAMBLE_V1
            + "\n"
            + json_dumps(payload, ensure_ascii=False, separators=(",", ":"), sort_keys=True)
        )

    @property
    def characters(self) -> int:
        return len(self.content)

    def to_wire(self) -> JsonObject:
        return {
            "version": 1,
            "characters": self.characters,
            "runs": [run.to_wire() for run in self.runs],
        }

    @classmethod
    def from_wire(cls, value: object) -> FrozenHistoryStatusV1:
        if not isinstance(value, dict) or set(value) != {"version", "characters", "runs"}:
            raise ValueError("frozen history status is invalid")
        if type(value["version"]) is not int or value["version"] != 1:
            raise ValueError("history status version is unsupported")
        if not isinstance(value["runs"], list) or type(value["characters"]) is not int:
            raise ValueError("history status records or budget are invalid")
        result = cls(tuple(HistoryRunStatusV1.from_wire(run) for run in value["runs"]))
        if result.characters != value["characters"]:
            raise ValueError("history status character count does not match")
        return result


def historical_run_status(
    *, turn_id: str, run_id: str, status: str, reason_code: str | None
) -> HistoryRunStatusV1:
    return HistoryRunStatusV1(
        turn_id, run_id, cast(Literal["failed", "cancelled"], status), reason_code
    )


EMPTY_HISTORY_STATUS_V1 = FrozenHistoryStatusV1()
