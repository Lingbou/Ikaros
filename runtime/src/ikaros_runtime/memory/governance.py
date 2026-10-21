"""Deterministic normalization for Memory deduplication and revision merges."""

from __future__ import annotations

import unicodedata

_CJK_RANGES = (
    (0x3400, 0x4DBF),
    (0x4E00, 0x9FFF),
    (0xF900, 0xFAFF),
    (0x20000, 0x2FA1F),
    (0x30000, 0x323AF),
)


def canonical_memory_text(value: str) -> str:
    """Return the stable comparison form of one Memory body.

    NFKC folds compatibility characters, casefold removes case, and punctuation
    and symbol characters become separators.  Whitespace is collapsed so line
    wrapping or repeated spaces cannot create a duplicate Memory.
    """

    normalized = unicodedata.normalize("NFKC", value).casefold()
    characters = [
        character if character.isalnum() or character.isspace() else " "
        for character in normalized
    ]
    return " ".join("".join(characters).split())


def memory_content_tokens(value: str) -> frozenset[str]:
    """Return stable Latin word and CJK n-gram tokens for similarity scoring."""

    normalized = unicodedata.normalize("NFKC", value).casefold()
    tokens: set[str] = set()
    word: list[str] = []
    cjk: list[str] = []

    def flush_word() -> None:
        if word:
            tokens.add("".join(word))
        word.clear()

    def flush_cjk() -> None:
        if len(cjk) == 1:
            tokens.add(cjk[0])
        elif len(cjk) >= 2:
            tokens.update(left + right for left, right in zip(cjk, cjk[1:], strict=False))
        cjk.clear()

    for character in normalized:
        if _is_cjk(character):
            flush_word()
            cjk.append(character)
        elif character.isalnum():
            flush_cjk()
            word.append(character)
        else:
            flush_word()
            flush_cjk()
    flush_word()
    flush_cjk()
    return frozenset(tokens)


def memory_content_similarity(left: str, right: str) -> float:
    """Return a deterministic lexical similarity in the closed interval [0, 1]."""

    left_tokens = memory_content_tokens(left)
    right_tokens = memory_content_tokens(right)
    if not left_tokens or not right_tokens:
        return 0.0
    shared = len(left_tokens & right_tokens)
    if shared == 0:
        return 0.0
    union = len(left_tokens | right_tokens)
    containment = shared / min(len(left_tokens), len(right_tokens))
    return max(shared / union, containment)


def _is_cjk(character: str) -> bool:
    codepoint = ord(character)
    return any(start <= codepoint <= end for start, end in _CJK_RANGES)


__all__ = [
    "canonical_memory_text",
    "memory_content_similarity",
    "memory_content_tokens",
]
