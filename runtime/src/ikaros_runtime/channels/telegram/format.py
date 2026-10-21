"""Formatting helpers for Telegram plain-text delivery."""

from __future__ import annotations

TELEGRAM_MAX_MESSAGE_LENGTH = 4096
TELEGRAM_SAFE_MESSAGE_LENGTH = 4000


def normalize_outgoing_text(value: str) -> str:
    """Drop control characters Telegram rejects while keeping normal whitespace."""

    normalized = value.replace("\x00", "").replace("\r\n", "\n").replace("\r", "\n")
    normalized = "".join(
        character
        for character in normalized
        if character in {"\n", "\t"}
        or (ord(character) >= 0x20 and not 0xD800 <= ord(character) <= 0xDFFF)
    )
    return normalized.strip()


def utf16_length(value: str) -> int:
    """Telegram measures message length in UTF-16 code units, not characters."""

    return sum(2 if ord(character) > 0xFFFF else 1 for character in value)


def split_message(
    value: str,
    *,
    limit: int = TELEGRAM_SAFE_MESSAGE_LENGTH,
) -> tuple[str, ...]:
    """Split a message into chunks whose UTF-16 length fits the Telegram limit.

    The Bot API counts 4096 *UTF-16 code units*, so astral characters (most emoji)
    cost two units each. Splitting by Python ``len`` would send chunks Telegram
    rejects as too long and drop the reply.
    """

    if limit < 1:
        raise ValueError("telegram split limit must be positive")
    remaining = normalize_outgoing_text(value)
    if not remaining:
        return ()
    chunks: list[str] = []
    while utf16_length(remaining) > limit:
        max_characters = _max_prefix_characters(remaining, limit)
        cut = remaining.rfind("\n", 0, max_characters + 1)
        if cut <= 0:
            cut = remaining.rfind(" ", 0, max_characters + 1)
        if cut <= 0:
            cut = max_characters
        if cut <= 0:
            # One character is wider than the whole limit; still make progress.
            cut = 1
        chunks.append(remaining[:cut].rstrip())
        remaining = remaining[cut:].lstrip()
    if remaining:
        chunks.append(remaining)
    return tuple(chunk for chunk in chunks if chunk)


def _max_prefix_characters(value: str, limit: int) -> int:
    """Largest prefix of ``value`` whose UTF-16 length does not exceed ``limit``."""

    units = 0
    for index, character in enumerate(value):
        width = 2 if ord(character) > 0xFFFF else 1
        if units + width > limit:
            return index
        units += width
    return len(value)


def format_failure_reason(reason_code: str | None) -> str:
    if not reason_code:
        return "运行失败。"
    return f"运行失败：{reason_code}"


__all__ = [
    "TELEGRAM_MAX_MESSAGE_LENGTH",
    "TELEGRAM_SAFE_MESSAGE_LENGTH",
    "format_failure_reason",
    "normalize_outgoing_text",
    "split_message",
    "utf16_length",
]