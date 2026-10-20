from __future__ import annotations

import base64
import binascii
import re
from collections.abc import Callable, Mapping
from typing import Any

from ..json_codec import dumps as json_dumps
from ..json_codec import loads as json_loads

CURSOR_CHARACTERS = re.compile(r"^[A-Za-z0-9_-]+$")


def encode_cursor(
    payload: Mapping[str, Any],
) -> str:
    encoded = json_dumps(
        dict(payload),
        ensure_ascii=False,
        separators=(",", ":"),
    ).encode("utf-8")
    return base64.urlsafe_b64encode(encoded).rstrip(b"=").decode("ascii")


def decode_cursor[T](
    value: str,
    *,
    expected_version: int,
    expected_keys: set[str],
    max_length: int,
    invalid_message: str,
    parse_payload: Callable[[dict[str, Any]], T],
    validate_cursor: Callable[[T], None] | None = None,
    reencode: Callable[[T], str],
    version_key: str = "v",
) -> T:
    if (
        not value
        or len(value) > max_length
        or CURSOR_CHARACTERS.fullmatch(value) is None
    ):
        raise ValueError(invalid_message)
    try:
        padding = "=" * (-len(value) % 4)
        decoded = base64.b64decode(
            value + padding,
            altchars=b"-_",
            validate=True,
        )
        payload = json_loads(decoded)
    except (UnicodeDecodeError, ValueError, binascii.Error):
        raise ValueError(invalid_message) from None
    if not isinstance(payload, dict) or set(payload) != expected_keys:
        raise ValueError(invalid_message)
    version = payload.get(version_key)
    if isinstance(version, bool) or not isinstance(version, int) or version != expected_version:
        raise ValueError(invalid_message)
    try:
        cursor = parse_payload(payload)
        if validate_cursor is not None:
            validate_cursor(cursor)
    except (TypeError, ValueError, KeyError):
        raise ValueError(invalid_message) from None
    if reencode(cursor) != value:
        raise ValueError(invalid_message)
    return cursor
