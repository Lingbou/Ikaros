"""Deterministic, bounded retrieval over explicit long-term Memory."""

from __future__ import annotations

import unicodedata
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from typing import Literal, Protocol

from ..errors import MemoryRetrievalError
from .domain import (
    MEMORY_CONTENT_MAX_CHARACTERS,
    MEMORY_KINDS,
    MemoryKind,
    MemoryScope,
    validate_memory_content,
)

type FrozenMemoryScope = Literal["global", "workspace"]
type MemoryOmissionReason = Literal["omitted_by_limit", "omitted_by_budget"]

MEMORY_RETRIEVAL_MAX_CANDIDATES = 2_000
MEMORY_RETRIEVAL_FETCH_LIMIT = MEMORY_RETRIEVAL_MAX_CANDIDATES + 1
MEMORY_RETRIEVAL_TOP_K = 8
MEMORY_RETRIEVAL_CHARACTER_BUDGET = 6_000
MEMORY_RETRIEVAL_VERSION: Literal["deterministic-keywords-v1"] = (
    "deterministic-keywords-v1"
)


@dataclass(frozen=True, slots=True)
class MemoryRetrievalCandidateV1:
    memory_id: str
    kind: MemoryKind
    scope: MemoryScope
    revision: int
    content: str
    updated_at: str

    def __post_init__(self) -> None:
        _memory_id(self.memory_id)
        if self.kind not in MEMORY_KINDS:
            raise ValueError("Memory retrieval candidate kind is invalid")
        _positive_revision(self.revision)
        validate_memory_content(self.content)
        if not self.updated_at:
            raise ValueError("Memory retrieval candidate timestamp is invalid")

    @property
    def frozen_scope(self) -> FrozenMemoryScope:
        return self.scope.type

    @property
    def characters(self) -> int:
        return len(self.content)

    def to_reference(self) -> MemorySnapshotReferenceV1:
        scope = self.frozen_scope
        return MemorySnapshotReferenceV1(
            memory_id=self.memory_id,
            revision=self.revision,
            scope=scope,
            characters=self.characters,
        )


@dataclass(frozen=True, slots=True)
class MemorySnapshotReferenceV1:
    memory_id: str
    revision: int
    scope: FrozenMemoryScope
    characters: int

    def __post_init__(self) -> None:
        _memory_id(self.memory_id)
        _positive_revision(self.revision)
        if self.scope not in {"global", "workspace"}:
            raise ValueError("frozen Memory scope is invalid")
        if (
            not isinstance(self.characters, int)
            or isinstance(self.characters, bool)
            or self.characters < 1
            or self.characters > MEMORY_CONTENT_MAX_CHARACTERS
        ):
            raise ValueError("frozen Memory character count is invalid")


@dataclass(frozen=True, slots=True)
class MaterializedMemoryV1:
    reference: MemorySnapshotReferenceV1
    content: str

    def __post_init__(self) -> None:
        validate_memory_content(self.content)
        if len(self.content) != self.reference.characters:
            raise ValueError("materialized Memory character count does not match")

    @property
    def memory_id(self) -> str:
        return self.reference.memory_id

    @property
    def revision(self) -> int:
        return self.reference.revision

    @property
    def scope(self) -> FrozenMemoryScope:
        return self.reference.scope

    @property
    def characters(self) -> int:
        return self.reference.characters


@dataclass(frozen=True, slots=True)
class MemoryRetrievalOmissionV1:
    memory_id: str
    revision: int
    characters: int
    reason: MemoryOmissionReason

    def __post_init__(self) -> None:
        _memory_id(self.memory_id)
        _positive_revision(self.revision)
        if (
            not isinstance(self.characters, int)
            or isinstance(self.characters, bool)
            or self.characters < 1
            or self.characters > MEMORY_CONTENT_MAX_CHARACTERS
        ):
            raise ValueError("omitted Memory character count is invalid")
        if self.reason not in {"omitted_by_limit", "omitted_by_budget"}:
            raise ValueError("Memory omission reason is invalid")


@dataclass(frozen=True, slots=True)
class MemoryRetrievalV1:
    version: Literal["deterministic-keywords-v1"]
    selected: tuple[MaterializedMemoryV1, ...]
    omissions: tuple[MemoryRetrievalOmissionV1, ...]
    candidate_count: int
    relevant_count: int
    memory_characters: int

    def __post_init__(self) -> None:
        if self.version != MEMORY_RETRIEVAL_VERSION:
            raise ValueError("Memory retrieval version is unsupported")
        object.__setattr__(self, "selected", tuple(self.selected))
        object.__setattr__(self, "omissions", tuple(self.omissions))
        counts = (self.candidate_count, self.relevant_count, self.memory_characters)
        if any(
            not isinstance(value, int) or isinstance(value, bool) or value < 0
            for value in counts
        ):
            raise ValueError("Memory retrieval counts are invalid")
        if (
            self.candidate_count > MEMORY_RETRIEVAL_MAX_CANDIDATES
            or self.relevant_count > self.candidate_count
            or len(self.selected) > MEMORY_RETRIEVAL_TOP_K
            or self.memory_characters > MEMORY_RETRIEVAL_CHARACTER_BUDGET
            or self.memory_characters
            != sum(memory.characters for memory in self.selected)
            or self.relevant_count != len(self.selected) + len(self.omissions)
        ):
            raise ValueError("Memory retrieval result is inconsistent")
        ids = tuple(memory.memory_id for memory in self.selected)
        omitted_ids = tuple(omission.memory_id for omission in self.omissions)
        if len((*ids, *omitted_ids)) != len(set((*ids, *omitted_ids))):
            raise ValueError("Memory retrieval result contains duplicate IDs")

    @property
    def references(self) -> tuple[MemorySnapshotReferenceV1, ...]:
        return tuple(memory.reference for memory in self.selected)


class MemoryRetrievalReader(Protocol):
    def read_active_candidates(
        self,
        *,
        workspace_id: str | None,
    ) -> tuple[MemoryRetrievalCandidateV1, ...]: ...

    def materialize_memory_revisions(
        self,
        references: Sequence[MemorySnapshotReferenceV1],
        *,
        workspace_id: str | None,
    ) -> tuple[MaterializedMemoryV1, ...]: ...


class MemoryRetrieverV1:
    def __init__(self, reader: MemoryRetrievalReader) -> None:
        self._reader = reader

    def retrieve(
        self,
        *,
        query: str,
        workspace_id: str | None,
        accept_selection: Callable[[Sequence[MaterializedMemoryV1]], bool] | None = None,
    ) -> MemoryRetrievalV1:
        """Select whole ranked records within both retrieval and caller capacity."""
        candidates = self._reader.read_active_candidates(workspace_id=workspace_id)
        query_tokens = _tokenize(query)
        ranked = [
            (_relevance(query_tokens, _tokenize(candidate.content)), candidate)
            for candidate in candidates
        ]
        ranked = [(score, candidate) for score, candidate in ranked if score > 0]
        ranked.sort(key=lambda item: item[1].memory_id)
        ranked.sort(key=lambda item: item[1].updated_at, reverse=True)
        ranked.sort(key=lambda item: item[0], reverse=True)

        selected: list[MaterializedMemoryV1] = []
        omissions: list[MemoryRetrievalOmissionV1] = []
        memory_characters = 0
        for _score, candidate in ranked:
            if len(selected) >= MEMORY_RETRIEVAL_TOP_K:
                omissions.append(_omission(candidate, "omitted_by_limit"))
                continue
            if (
                memory_characters + candidate.characters
                > MEMORY_RETRIEVAL_CHARACTER_BUDGET
            ):
                omissions.append(_omission(candidate, "omitted_by_budget"))
                continue
            materialized = MaterializedMemoryV1(
                reference=candidate.to_reference(),
                content=candidate.content,
            )
            if accept_selection is not None and not accept_selection(
                (*selected, materialized)
            ):
                omissions.append(_omission(candidate, "omitted_by_budget"))
                continue
            selected.append(materialized)
            memory_characters += materialized.characters

        return MemoryRetrievalV1(
            version=MEMORY_RETRIEVAL_VERSION,
            selected=tuple(selected),
            omissions=tuple(omissions),
            candidate_count=len(candidates),
            relevant_count=len(ranked),
            memory_characters=memory_characters,
        )

    def materialize_exact(
        self,
        references: Sequence[MemorySnapshotReferenceV1],
        *,
        workspace_id: str | None,
    ) -> tuple[MaterializedMemoryV1, ...]:
        frozen = tuple(references)
        if len(frozen) > MEMORY_RETRIEVAL_TOP_K or len(
            {reference.memory_id for reference in frozen}
        ) != len(frozen):
            raise MemoryRetrievalError("memory_snapshot_unavailable")
        return self._reader.materialize_memory_revisions(
            frozen,
            workspace_id=workspace_id,
        )


@dataclass(frozen=True, slots=True)
class _Tokens:
    words: frozenset[str]
    han_unigrams: frozenset[str]
    han_bigrams: frozenset[str]


def _tokenize(value: str) -> _Tokens:
    normalized = unicodedata.normalize(
        "NFKC",
        unicodedata.normalize("NFKC", value).casefold(),
    )
    words: set[str] = set()
    han_unigrams: set[str] = set()
    han_bigrams: set[str] = set()
    word: list[str] = []
    han: list[str] = []

    def flush_word() -> None:
        if word:
            words.add("".join(word))
            word.clear()

    def flush_han() -> None:
        if not han:
            return
        han_unigrams.update(han)
        han_bigrams.update(left + right for left, right in zip(han, han[1:], strict=False))
        han.clear()

    for character in normalized:
        if _is_han(character):
            flush_word()
            han.append(character)
            continue
        flush_han()
        category = unicodedata.category(character)
        if character.isalnum() or (category.startswith("M") and word):
            word.append(character)
        else:
            flush_word()
    flush_word()
    flush_han()
    return _Tokens(frozenset(words), frozenset(han_unigrams), frozenset(han_bigrams))


def _is_han(character: str) -> bool:
    codepoint = ord(character)
    return (
        0x3400 <= codepoint <= 0x4DBF
        or 0x4E00 <= codepoint <= 0x9FFF
        or 0xF900 <= codepoint <= 0xFAFF
        or 0x20000 <= codepoint <= 0x2FA1F
        or 0x30000 <= codepoint <= 0x323AF
    )


def _relevance(query: _Tokens, candidate: _Tokens) -> int:
    return (
        4 * len(query.words & candidate.words)
        + 4 * len(query.han_bigrams & candidate.han_bigrams)
        + len(query.han_unigrams & candidate.han_unigrams)
    )


def _omission(
    candidate: MemoryRetrievalCandidateV1,
    reason: MemoryOmissionReason,
) -> MemoryRetrievalOmissionV1:
    return MemoryRetrievalOmissionV1(
        memory_id=candidate.memory_id,
        revision=candidate.revision,
        characters=candidate.characters,
        reason=reason,
    )


def _memory_id(value: str) -> None:
    if not value.startswith("memory_"):
        raise ValueError("Memory ID is invalid")
    suffix = value.removeprefix("memory_")
    if len(suffix) != 32 or any(character not in "0123456789abcdef" for character in suffix):
        raise ValueError("Memory ID is invalid")


def _positive_revision(value: int) -> None:
    if not isinstance(value, int) or isinstance(value, bool) or value < 1:
        raise ValueError("Memory revision is invalid")


__all__ = [
    "MEMORY_RETRIEVAL_CHARACTER_BUDGET",
    "MEMORY_RETRIEVAL_FETCH_LIMIT",
    "MEMORY_RETRIEVAL_MAX_CANDIDATES",
    "MEMORY_RETRIEVAL_TOP_K",
    "MEMORY_RETRIEVAL_VERSION",
    "FrozenMemoryScope",
    "MaterializedMemoryV1",
    "MemoryRetrievalCandidateV1",
    "MemoryRetrievalOmissionV1",
    "MemoryRetrievalV1",
    "MemoryRetrieverV1",
    "MemorySnapshotReferenceV1",
]
