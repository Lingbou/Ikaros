"""Deterministic, provider-neutral history selection for one Agent Run."""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass

from ..errors import ContextBudgetExceededError
from ..run_input import (
    EMPTY_FROZEN_MEMORY_CONTEXT_V1,
    ContextItemRecordV1,
    FrozenMemoryContextV1,
    OmissionRecordV1,
    RunConfig,
    config_input_token_counts,
)

HISTORY_TURN_PAGE_SIZE_V1 = 32


@dataclass(frozen=True, slots=True)
class HistorySelection:
    records: tuple[ContextItemRecordV1, ...]
    maximum_tokens: int
    reserved_current_run_tokens: int
    omissions: tuple[OmissionRecordV1, ...]


class HistorySelector:
    """Select a contiguous recent suffix of complete past Turns.

    Candidate Turns are offered newest-first.  The first Turn that does not fit
    becomes the durable omission boundary; no older Turn may then be considered.
    """

    def __init__(
        self,
        *,
        config: RunConfig,
        current_run_id: str,
        current_turn_id: str,
        current_turn_ordinal: int,
        current_records: Sequence[ContextItemRecordV1],
        memory_context: FrozenMemoryContextV1 = EMPTY_FROZEN_MEMORY_CONTEXT_V1,
        maximum_tokens: int | None = None,
        reserved_current_run_tokens: int | None = None,
    ) -> None:
        maximum_tokens = (
            maximum_tokens if maximum_tokens is not None else config.maximum_input_tokens
        )
        reserved_current_run_tokens = (
            reserved_current_run_tokens
            if reserved_current_run_tokens is not None
            else config.reserved_current_run_tokens
        )
        if (
            not isinstance(maximum_tokens, int)
            or isinstance(maximum_tokens, bool)
            or maximum_tokens < 1
        ):
            raise ValueError("history maximum token estimate must be positive")
        if (
            not isinstance(reserved_current_run_tokens, int)
            or isinstance(reserved_current_run_tokens, bool)
            or reserved_current_run_tokens < 1
            or reserved_current_run_tokens >= maximum_tokens
        ):
            raise ValueError("history current Run reserve is invalid")
        if (
            not isinstance(current_turn_ordinal, int)
            or isinstance(current_turn_ordinal, bool)
            or current_turn_ordinal < 1
        ):
            raise ValueError("current Turn ordinal is invalid")
        if config.run_id != current_run_id:
            raise ValueError("history selector Run scope is invalid")

        current = tuple(current_records)
        _validate_turn_group(
            current_turn_id,
            current,
            required_run_id=current_run_id,
            required_user_item_id=config.user_item_id,
        )
        instruction_tokens, tool_tokens = config_input_token_counts(config)
        current_tokens = sum(record.estimated_tokens for record in current)
        fixed_with_reserve = (
            instruction_tokens
            + tool_tokens
            + memory_context.context_data_characters * 4
            + memory_context.memory_characters * 4
            + current_tokens
            + reserved_current_run_tokens
        )
        if fixed_with_reserve > maximum_tokens:
            raise ContextBudgetExceededError("context_budget_exceeded")

        self._current = current
        self._maximum_tokens = maximum_tokens
        self._reserved_current_run_tokens = reserved_current_run_tokens
        self._remaining_history_tokens = maximum_tokens - fixed_with_reserve
        self._selected_newest_first: list[tuple[ContextItemRecordV1, ...]] = []
        self._omissions: tuple[OmissionRecordV1, ...] = ()
        self._last_ordinal = current_turn_ordinal
        self._stopped = False

    @property
    def stopped(self) -> bool:
        return self._stopped

    def consider_turn(
        self,
        *,
        turn_id: str,
        ordinal: int,
        records: Sequence[ContextItemRecordV1],
        additional_tokens: int = 0,
    ) -> bool:
        if self._stopped:
            raise RuntimeError("history selection already reached its omission boundary")
        if (
            not isinstance(ordinal, int)
            or isinstance(ordinal, bool)
            or ordinal < 1
            or ordinal >= self._last_ordinal
        ):
            raise ValueError("history Turn order is invalid")
        group = tuple(records)
        _validate_turn_group(turn_id, group)
        tokens = sum(record.estimated_tokens for record in group) + additional_tokens
        if tokens > self._remaining_history_tokens:
            self._omissions = (
                OmissionRecordV1(
                    source_type="history",
                    source_id=turn_id,
                    reason="omitted_by_budget",
                ),
            )
            self._stopped = True
            return False
        self._selected_newest_first.append(group)
        self._remaining_history_tokens -= tokens
        self._last_ordinal = ordinal
        return True

    def finish(self, *, additional_tokens: int = 0) -> HistorySelection:
        if additional_tokens > self._remaining_history_tokens:
            raise ContextBudgetExceededError("context_budget_exceeded")
        chronological = tuple(
            record for group in reversed(self._selected_newest_first) for record in group
        )
        return HistorySelection(
            records=(*chronological, *self._current),
            maximum_tokens=self._maximum_tokens,
            reserved_current_run_tokens=self._reserved_current_run_tokens,
            omissions=self._omissions,
        )


def _validate_turn_group(
    turn_id: str,
    records: Sequence[ContextItemRecordV1],
    *,
    required_run_id: str | None = None,
    required_user_item_id: str | None = None,
) -> None:
    if not turn_id or not records:
        raise ValueError("history Turn group is empty")
    if any(record.turn_id != turn_id for record in records):
        raise ValueError("history Turn group crosses Turn boundaries")
    item_ids = tuple(record.item_id for record in records)
    if len(item_ids) != len(set(item_ids)):
        raise ValueError("history Turn group contains duplicate Items")
    if required_run_id is not None and any(record.run_id != required_run_id for record in records):
        raise ValueError("current Turn group crosses Run boundaries")

    user_items = tuple(
        record
        for record in records
        if record.kind == "message"
        and record.role == "user"
        and record.data.get("steer") is not True
    )
    if len(user_items) != 1:
        raise ValueError("history Turn group must contain exactly one User Item")
    if required_user_item_id is not None and user_items[0].item_id != required_user_item_id:
        raise ValueError("current Turn group does not contain the frozen User Item")

    calls: dict[tuple[str, str, str], ContextItemRecordV1] = {}
    results: dict[tuple[str, str, str], ContextItemRecordV1] = {}
    for record in records:
        _ = record.estimated_tokens
        if record.kind == "tool_call":
            call_id = record.data.get("callId")
            step_id = record.data.get("stepId")
            if (
                not isinstance(call_id, str)
                or not call_id
                or not isinstance(step_id, str)
                or not step_id
            ):
                raise ValueError("history Tool Call identity is invalid")
            identity = (record.run_id, step_id, call_id)
            if identity in calls:
                raise ValueError("history Tool Call ID is duplicated")
            calls[identity] = record
        elif record.kind == "tool_result":
            call_id = record.data.get("callId")
            step_id = record.data.get("stepId")
            tool_call_item_id = record.data.get("toolCallItemId")
            if (
                not isinstance(call_id, str)
                or not call_id
                or not isinstance(step_id, str)
                or not step_id
                or not isinstance(tool_call_item_id, str)
                or not tool_call_item_id
            ):
                raise ValueError("history Tool Result identity is invalid")
            identity = (record.run_id, step_id, call_id)
            if identity in results:
                raise ValueError("history Tool Result ID is duplicated")
            results[identity] = record

    if calls.keys() != results.keys():
        raise ValueError("history Tool Calls and Results are not atomic")
    for identity, call in calls.items():
        result = results[identity]
        if (
            result.data["toolCallItemId"] != call.item_id
            or result.data["stepId"] != call.data["stepId"]
        ):
            raise ValueError("history Tool Result does not match its Tool Call")


__all__ = [
    "HISTORY_TURN_PAGE_SIZE_V1",
    "HistorySelection",
    "HistorySelector",
]
