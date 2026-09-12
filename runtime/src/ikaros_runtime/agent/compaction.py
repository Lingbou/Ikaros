"""Deterministic context trimming used before semantic compaction.

The trimmer never mutates persisted Items.  It treats assistant tool exchanges as
atomic units so a trimmed input cannot leave an orphaned ``tool_result``.  The
caller can persist the omitted IDs and, when available, replace them with a
semantic summary in a subsequent ContextRevision.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Sequence

from ..run_input import ContextItemRecordV1


@dataclass(frozen=True, slots=True)
class ContextTrimResult:
    """Result of deterministic trimming.

    ``records`` remains in original order.  ``omitted_item_ids`` is ordered as
    in the original input, which makes the result suitable for an audit event.
    """

    records: tuple[ContextItemRecordV1, ...]
    omitted_item_ids: tuple[str, ...]
    retained_tokens: int
    omitted_tokens: int
    truncated: bool


def trim_context_records(
    records: Sequence[ContextItemRecordV1],
    *,
    maximum_tokens: int,
    preserve_prefix_units: int = 1,
    preserve_suffix_units: int = 4,
) -> ContextTrimResult:
    """Keep a deterministic head/tail subset within ``maximum_tokens``.

    Units are ordinary messages or complete assistant tool exchanges.  The
    first ``preserve_prefix_units`` and last ``preserve_suffix_units`` units are
    preferred; when they do not fit, newest units win after the prefix.  No
    unit is split.  A unit larger than the budget is retained by itself so the
    caller can fall back to semantic compaction rather than silently dropping
    the only user request.
    """

    if isinstance(maximum_tokens, bool) or not isinstance(maximum_tokens, int) or maximum_tokens < 1:
        raise ValueError("maximum_tokens must be a positive integer")
    if preserve_prefix_units < 0 or preserve_suffix_units < 0:
        raise ValueError("preserve unit counts must be non-negative")
    frozen = tuple(records)
    units = _units(frozen)
    costs = tuple(sum(item.estimated_tokens for item in unit) for unit in units)
    total = sum(costs)
    if total <= maximum_tokens:
        return ContextTrimResult(frozen, (), total, 0, False)
    if not units:
        return ContextTrimResult((), (), 0, 0, False)

    selected: set[int] = set()
    used = 0

    # Protect the beginning (typically the original user request).  If it is
    # over budget, retaining it is still safer than producing an empty prompt.
    for index in range(min(preserve_prefix_units, len(units))):
        selected.add(index)
        used += costs[index]

    # Fill from the newest end.  This naturally retains the latest tool result
    # and any steering message while keeping output deterministic.
    for index in range(len(units) - 1, -1, -1):
        if index in selected:
            continue
        if index >= len(units) - preserve_suffix_units or used + costs[index] <= maximum_tokens:
            if used + costs[index] <= maximum_tokens or not selected:
                selected.add(index)
                used += costs[index]

    # If the protected prefix itself exceeds the budget, keep only that prefix;
    # semantic compaction can then summarize it in a separate model call.
    retained_indices = tuple(sorted(selected))
    retained = tuple(item for index in retained_indices for item in units[index])
    retained_ids = {item.item_id for item in retained}
    omitted = tuple(item for item in frozen if item.item_id not in retained_ids)
    return ContextTrimResult(
        records=retained,
        omitted_item_ids=tuple(item.item_id for item in omitted),
        retained_tokens=sum(item.estimated_tokens for item in retained),
        omitted_tokens=sum(item.estimated_tokens for item in omitted),
        truncated=bool(omitted),
    )


def _units(records: tuple[ContextItemRecordV1, ...]) -> tuple[tuple[ContextItemRecordV1, ...], ...]:
    """Partition records into provider-valid atomic message/tool units."""

    units: list[tuple[ContextItemRecordV1, ...]] = []
    index = 0
    while index < len(records):
        item = records[index]
        step_id = item.data.get("stepId") if item.kind == "message" else None
        if item.kind == "message" and item.role == "assistant" and isinstance(step_id, str):
            end = index + 1
            while end < len(records):
                candidate = records[end]
                if candidate.kind not in {"tool_call", "tool_result"} or candidate.data.get("stepId") != step_id:
                    break
                end += 1
            units.append(records[index:end])
            index = end
            continue
        if item.kind == "tool_call":
            step_id = item.data.get("stepId")
            end = index + 1
            while end < len(records):
                candidate = records[end]
                if candidate.kind not in {"tool_call", "tool_result"} or candidate.data.get("stepId") != step_id:
                    break
                end += 1
            units.append(records[index:end])
            index = end
            continue
        units.append((item,))
        index += 1
    return tuple(units)


__all__ = ["ContextTrimResult", "trim_context_records"]
