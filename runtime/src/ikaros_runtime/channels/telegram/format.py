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
        if character in {"\n", "\t"} or ord(character) >= 0x20
    )
    return normalized.strip()


def split_message(value: str, *, limit: int = TELEGRAM_SAFE_MESSAGE_LENGTH) -> tuple[str, ...]:
    """Split a message into Telegram-safe chunks without cutting mid-code-fence."""

    if limit < 1:
        raise ValueError("telegram split limit must be positive")
    remaining = normalize_outgoing_text(value)
    if not remaining:
        return ()
    chunks: list[str] = []
    while len(remaining) > limit:
        cut = remaining.rfind("\n", 0, limit + 1)
        if cut <= 0:
            cut = remaining.rfind(" ", 0, limit + 1)
        if cut <= 0:
            cut = limit
        chunks.append(remaining[:cut].rstrip())
        remaining = remaining[cut:].lstrip()
    if remaining:
        chunks.append(remaining)
    return tuple(chunk for chunk in chunks if chunk)


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
]
