"""Telegram formatting: the Bot API limit is UTF-16 code units."""

from __future__ import annotations

from ikaros_runtime.channels.telegram.format import (
    TELEGRAM_MAX_MESSAGE_LENGTH,
    split_message,
    utf16_length,
)


def test_astral_characters_count_as_two_utf16_units() -> None:
    assert utf16_length("😀") == 2
    assert utf16_length("汉") == 1
    assert utf16_length("a😀b") == 4


def test_emoji_heavy_message_splits_within_the_telegram_limit() -> None:
    chunks = split_message("😀" * 5_000)

    assert len(chunks) == 3
    assert "".join(chunks) == "😀" * 5_000
    assert all(
        utf16_length(chunk) <= TELEGRAM_MAX_MESSAGE_LENGTH for chunk in chunks
    )


def test_chinese_text_keeps_its_previous_split() -> None:
    chunks = split_message("汉" * 5_000)

    assert len(chunks) == 2
    assert [utf16_length(chunk) for chunk in chunks] == [4_000, 1_000]


def test_splitting_prefers_a_newline_boundary() -> None:
    first = "a" * 3_000
    second = "b" * 3_000

    chunks = split_message(first + "\n" + second)

    assert chunks == (first, second)


def test_tiny_limit_still_makes_progress() -> None:
    chunks = split_message("😀😀", limit=1)

    assert chunks == ("😀", "😀")