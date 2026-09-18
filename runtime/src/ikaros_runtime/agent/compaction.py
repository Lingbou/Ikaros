"""Deterministic context trimming used before semantic compaction.

The trimmer never mutates persisted Items.  It treats assistant tool exchanges as
atomic units so a trimmed input cannot leave an orphaned ``tool_result``.  The
caller can persist the omitted IDs and, when available, replace them with a
semantic summary in a subsequent ContextRevision.
"""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass

from ..json_codec import dumps as json_dumps
from ..json_codec import loads as json_loads
from ..run_input import ContextItemRecordV1

_COMPACTION_SOURCE_MAX_BYTES = 24_000
_COMPACTION_RECORD_MAX_BYTES = 700
_COMPACTION_ARGUMENTS_MAX_BYTES = 700
_RETAINED_USER_MESSAGE_TOKEN_BUDGET = 20_000


def build_compaction_source(
    records: Sequence[ContextItemRecordV1],
    *,
    max_bytes: int = _COMPACTION_SOURCE_MAX_BYTES,
) -> str:
    """Render bounded, explicitly untrusted source data for a summary request."""

    if isinstance(max_bytes, bool) or not isinstance(max_bytes, int) or max_bytes < 2:
        raise ValueError("max_bytes must be a positive integer")
    payload: list[dict[str, object]] = []
    for record in records:
        content = _truncate_text(record.content.replace("\x00", " ").strip())
        value: dict[str, object] = {
            "itemId": record.item_id,
            "turnId": record.turn_id,
            "kind": record.kind,
            "role": record.role,
            "content": content,
        }
        if record.kind == "tool_call":
            value["toolName"] = record.data.get("toolName")
            arguments = record.data.get("arguments")
            arguments_json = json_dumps(arguments, ensure_ascii=False, separators=(",", ":"))
            value["arguments"] = (
                arguments
                if len(arguments_json.encode("utf-8")) <= _COMPACTION_ARGUMENTS_MAX_BYTES
                else _truncate_text(arguments_json, max_bytes=_COMPACTION_ARGUMENTS_MAX_BYTES)
            )
        candidate = json_dumps([*payload, value], ensure_ascii=False, separators=(",", ":"))
        if len(candidate.encode("utf-8")) > max_bytes:
            break
        payload.append(value)
    return json_dumps(payload, ensure_ascii=False, separators=(",", ":"))


def compaction_source_item_ids(source: str) -> tuple[str, ...]:
    """Return the source records actually rendered in a bounded payload.

    The source renderer intentionally stops at the byte budget.  Callers that
    persist a summary must therefore verify that every omitted record was
    represented; otherwise the resulting revision could silently discard facts
    the summarizer never received.
    """

    decoded = json_loads(source)
    if not isinstance(decoded, list):
        raise ValueError("compaction source must be a JSON array")
    item_ids: list[str] = []
    for value in decoded:
        if not isinstance(value, dict) or not isinstance(value.get("itemId"), str):
            raise ValueError("compaction source record is invalid")
        item_ids.append(value["itemId"])
    return tuple(item_ids)


def _truncate_text(value: str, *, max_bytes: int = _COMPACTION_RECORD_MAX_BYTES) -> str:
    encoded = value.encode("utf-8")
    if len(encoded) <= max_bytes:
        return value
    return encoded[:max_bytes].decode("utf-8", errors="ignore") + " [truncated]"


@dataclass(frozen=True, slots=True)
class ContextTrimResult:
    """Result of deterministic trimming.

    ``records`` remains in original order.  ``omitted_item_ids`` is ordered as
    in the original input, which makes the result suitable for an audit event.
    """

    records: tuple[ContextItemRecordV1, ...]
    omitted_item_ids: tuple[str, ...]
    retained_tokens: int
    truncated: bool


def trim_context_records(
    records: Sequence[ContextItemRecordV1],
    *,
    maximum_tokens: int,
    preserve_prefix_units: int = 1,
    preserve_suffix_units: int = 4,
    preserve_recent_user_tokens: int = 0,
    retain_latest_oversized: bool = True,
) -> ContextTrimResult:
    """Keep a deterministic head/tail subset within ``maximum_tokens``.

    Units are ordinary messages or complete assistant tool exchanges.  The
    first ``preserve_prefix_units`` and last ``preserve_suffix_units`` units are
    preferred; when they do not fit, newest units win after the prefix.  No
    unit is split.  A unit larger than the budget is retained by itself so the
    caller can fall back to semantic compaction rather than silently dropping
    the only user request.
    """

    if (
        isinstance(maximum_tokens, bool)
        or not isinstance(maximum_tokens, int)
        or maximum_tokens < 1
    ):
        raise ValueError("maximum_tokens must be a positive integer")
    if preserve_prefix_units < 0 or preserve_suffix_units < 0:
        raise ValueError("preserve unit counts must be non-negative")
    if preserve_recent_user_tokens < 0:
        raise ValueError("preserve recent user token budget must be non-negative")
    frozen = tuple(records)
    units = _units(frozen)
    costs = tuple(sum(item.estimated_tokens for item in unit) for unit in units)
    total = sum(costs)
    if total <= maximum_tokens:
        return ContextTrimResult(frozen, (), total, False)
    if not units:
        return ContextTrimResult((), (), 0, False)

    selected: set[int] = set()
    used = 0

    # Protect the beginning (typically the original user request).  If it is
    # over budget, retaining it is still safer than producing an empty prompt.
    for index in range(min(preserve_prefix_units, len(units))):
        selected.add(index)
        used += costs[index]

    if preserve_recent_user_tokens > 0:
        retained_user_tokens = sum(
            costs[index]
            for index in selected
            if any(item.kind == "message" and item.role == "user" for item in units[index])
        )
        user_budget = min(
            preserve_recent_user_tokens,
            _RETAINED_USER_MESSAGE_TOKEN_BUDGET,
        )
        for index in range(len(units) - 1, -1, -1):
            if index in selected:
                continue
            unit = units[index]
            if not any(item.kind == "message" and item.role == "user" for item in unit):
                continue
            if (
                retained_user_tokens + costs[index] <= user_budget
                and used + costs[index] <= maximum_tokens
            ):
                selected.add(index)
                used += costs[index]
                retained_user_tokens += costs[index]

    # Fill from the newest end.  This naturally retains the latest tool result
    # and any steering message while keeping output deterministic. The newest
    # unit is mandatory: if it cannot fit, returning it makes the caller fail
    # explicitly instead of silently hiding the latest execution fact.
    for index in range(len(units) - 1, -1, -1):
        if index in selected:
            continue
        in_suffix = index >= len(units) - preserve_suffix_units
        fits = used + costs[index] <= maximum_tokens
        if (index == len(units) - 1 and not fits and retain_latest_oversized) or (
            (in_suffix or fits) and (fits or not selected)
        ):
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
                if (
                    candidate.kind not in {"tool_call", "tool_result"}
                    or candidate.data.get("stepId") != step_id
                ):
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
                if (
                    candidate.kind not in {"tool_call", "tool_result"}
                    or candidate.data.get("stepId") != step_id
                ):
                    break
                end += 1
            units.append(records[index:end])
            index = end
            continue
        units.append((item,))
        index += 1
    return tuple(units)


__all__ = [
    "ContextTrimResult",
    "build_compaction_source",
    "compaction_source_item_ids",
    "trim_context_records",
]
