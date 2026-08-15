from __future__ import annotations

import json
import math
from collections.abc import Mapping
from typing import Any, NoReturn

MAX_SAFE_INTEGER = (1 << 53) - 1


def _reject_nonstandard_constant(_value: str) -> NoReturn:
    raise ValueError("non-standard JSON constants are not supported")


def _parse_safe_integer(value: str) -> int:
    parsed = int(value)
    if parsed < -MAX_SAFE_INTEGER or parsed > MAX_SAFE_INTEGER:
        raise ValueError("JSON integers must be safe for JavaScript clients")
    return parsed


def _parse_finite_float(value: str) -> float:
    parsed = float(value)
    if not math.isfinite(parsed):
        raise ValueError("JSON numbers must be finite")
    if parsed.is_integer() and abs(parsed) > MAX_SAFE_INTEGER:
        raise ValueError("integer-valued JSON numbers must be safe for JavaScript clients")
    return parsed


def loads(source: str | bytes | bytearray) -> Any:
    """Decode strict JSON that can cross the Python/JavaScript boundary safely."""
    try:
        return json.loads(
            source,
            parse_constant=_reject_nonstandard_constant,
            parse_float=_parse_finite_float,
            parse_int=_parse_safe_integer,
        )
    except RecursionError:
        raise ValueError("JSON nesting is too deep") from None


def dumps(
    value: object,
    *,
    ensure_ascii: bool = True,
    separators: tuple[str, str] | None = None,
    sort_keys: bool = False,
) -> str:
    """Encode JSON only when every number remains valid in JavaScript."""
    _validate_numbers(value)
    try:
        return json.dumps(
            value,
            ensure_ascii=ensure_ascii,
            separators=separators,
            sort_keys=sort_keys,
            allow_nan=False,
        )
    except RecursionError:
        raise ValueError("JSON nesting is too deep") from None


def _validate_numbers(value: object) -> None:
    pending: list[tuple[object, bool]] = [(value, False)]
    active_containers: set[int] = set()
    while pending:
        current, leaving = pending.pop()
        if leaving:
            active_containers.remove(id(current))
            continue
        if current is None or isinstance(current, (bool, str)):
            continue
        if isinstance(current, int):
            if current < -MAX_SAFE_INTEGER or current > MAX_SAFE_INTEGER:
                raise ValueError("JSON integers must be safe for JavaScript clients")
            continue
        if isinstance(current, float):
            if not math.isfinite(current):
                raise ValueError("JSON numbers must be finite")
            if current.is_integer() and abs(current) > MAX_SAFE_INTEGER:
                raise ValueError("integer-valued JSON numbers must be safe for JavaScript clients")
            continue

        children: tuple[object, ...] | None = None
        if isinstance(current, Mapping):
            children = tuple(current.keys()) + tuple(current.values())
        elif isinstance(current, (list, tuple)):
            children = tuple(current)
        if children is None:
            continue

        container_id = id(current)
        if container_id in active_containers:
            raise ValueError("circular JSON structures are not supported")
        active_containers.add(container_id)
        pending.append((current, True))
        pending.extend((child, False) for child in children)
