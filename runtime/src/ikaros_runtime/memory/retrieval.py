"""Deterministic, bounded retrieval over explicit long-term Memory.

Recall fuses bounded ranked pools through Reciprocal Rank Fusion (k=60): an
FTS5 BM25 pool maintained inside ``memory.db`` and the keyword overlap pool
that scores the active candidate set in Python.  The vector pool is a reserved
interface (``MemorySemanticPool``) that this generation does not implement; no
LLM reranking and no external embedding service ever participate.
"""

from __future__ import annotations

import unicodedata
from collections.abc import Callable, Mapping, Sequence
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
MEMORY_RETRIEVAL_RRF_K = 60
MEMORY_RETRIEVAL_POOL_DEPTH = 200
MEMORY_RETRIEVAL_FTS_MAX_TERMS = 64
MEMORY_RETRIEVAL_FTS_MAX_PHRASE_BIGRAMS = 16
MEMORY_RETRIEVAL_VERSION: Literal["deterministic-hybrid-rrf-v2"] = (
    "deterministic-hybrid-rrf-v2"
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
    version: Literal["deterministic-hybrid-rrf-v2"]
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

    def rank_active_memory_ids_by_bm25(
        self,
        *,
        query: str,
        workspace_id: str | None,
        limit: int,
    ) -> tuple[str, ...]: ...

    def materialize_memory_revisions(
        self,
        references: Sequence[MemorySnapshotReferenceV1],
        *,
        workspace_id: str | None,
    ) -> tuple[MaterializedMemoryV1, ...]: ...


class MemorySemanticPool(Protocol):
    """Reserved ranking signal for a future deterministic vector pool.

    This generation ships no implementation.  A pool may only reorder the
    already-bounded active candidate set, and its order enters the fusion
    exactly like the two lexical pools.
    """

    def rank(
        self,
        *,
        query: str,
        candidates: Sequence[MemoryRetrievalCandidateV1],
    ) -> Sequence[str]: ...


class MemoryRetrieverV1:
    def __init__(
        self,
        reader: MemoryRetrievalReader,
        *,
        semantic_pool: MemorySemanticPool | None = None,
    ) -> None:
        self._reader = reader
        self._semantic_pool = semantic_pool

    def retrieve(
        self,
        *,
        query: str,
        workspace_id: str | None,
        accept_selection: Callable[[Sequence[MaterializedMemoryV1]], bool] | None = None,
    ) -> MemoryRetrievalV1:
        """Fuse bounded pools, then select whole ranked records within capacity."""
        candidates = self._reader.read_active_candidates(workspace_id=workspace_id)
        by_id = {candidate.memory_id: candidate for candidate in candidates}
        keyword_scores = _keyword_scores(query, candidates)
        pools = [
            _bounded_pool(
                self._reader.rank_active_memory_ids_by_bm25(
                    query=query,
                    workspace_id=workspace_id,
                    limit=MEMORY_RETRIEVAL_POOL_DEPTH,
                ),
                by_id=by_id,
            ),
            _keyword_pool(keyword_scores, by_id=by_id),
        ]
        if self._semantic_pool is not None:
            pools.append(
                _bounded_pool(
                    self._semantic_pool.rank(query=query, candidates=candidates),
                    by_id=by_id,
                )
            )
        ranked = _fuse_pools(pools, by_id=by_id, keyword_scores=keyword_scores)

        selected: list[MaterializedMemoryV1] = []
        omissions: list[MemoryRetrievalOmissionV1] = []
        memory_characters = 0
        for candidate in ranked:
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


def _keyword_scores(
    query: str,
    candidates: Sequence[MemoryRetrievalCandidateV1],
) -> dict[str, int]:
    """Score every candidate by weighted unique-token overlap (the fuzzy pool)."""
    query_tokens = _tokenize(query)
    scores: dict[str, int] = {}
    for candidate in candidates:
        score = _relevance(query_tokens, _tokenize(candidate.content))
        if score > 0:
            scores[candidate.memory_id] = score
    return scores


def _keyword_pool(
    keyword_scores: Mapping[str, int],
    *,
    by_id: Mapping[str, MemoryRetrievalCandidateV1],
) -> tuple[str, ...]:
    """Rank the fuzzy pool: weighted score, then recency, then Memory ID."""
    ranked = [by_id[memory_id] for memory_id in keyword_scores]
    ranked.sort(key=lambda candidate: candidate.memory_id)
    ranked.sort(key=lambda candidate: candidate.updated_at, reverse=True)
    ranked.sort(key=lambda candidate: keyword_scores[candidate.memory_id], reverse=True)
    return tuple(candidate.memory_id for candidate in ranked[:MEMORY_RETRIEVAL_POOL_DEPTH])


def _bounded_pool(
    ranking: Sequence[str],
    *,
    by_id: Mapping[str, MemoryRetrievalCandidateV1],
) -> tuple[str, ...]:
    """Keep the leading candidates of one pool, deduplicated and depth-bounded."""
    pool: list[str] = []
    seen: set[str] = set()
    for memory_id in ranking:
        if memory_id not in by_id or memory_id in seen:
            continue
        seen.add(memory_id)
        pool.append(memory_id)
        if len(pool) == MEMORY_RETRIEVAL_POOL_DEPTH:
            break
    return tuple(pool)


def _fuse_pools(
    pools: Sequence[Sequence[str]],
    *,
    by_id: Mapping[str, MemoryRetrievalCandidateV1],
    keyword_scores: Mapping[str, int],
) -> list[MemoryRetrievalCandidateV1]:
    """Reciprocal Rank Fusion over bounded pools, best first.

    A candidate absent from a pool contributes nothing for that pool.  Exact
    score ties fall back to the deterministic keyword score, then to recency
    and the Memory ID, so the fused order never depends on pool accidents.
    """
    scores: dict[str, float] = {}
    for pool in pools:
        for rank, memory_id in enumerate(pool, start=1):
            scores[memory_id] = scores.get(memory_id, 0.0) + 1.0 / (
                MEMORY_RETRIEVAL_RRF_K + rank
            )
    ranked = [by_id[memory_id] for memory_id in scores if memory_id in by_id]
    ranked.sort(key=lambda candidate: candidate.memory_id)
    ranked.sort(key=lambda candidate: candidate.updated_at, reverse=True)
    ranked.sort(
        key=lambda candidate: keyword_scores.get(candidate.memory_id, 0),
        reverse=True,
    )
    ranked.sort(key=lambda candidate: scores[candidate.memory_id], reverse=True)
    return ranked


def fts_token_stream(value: str) -> str:
    """Index text for ``memory_fts``: words, Han unigrams, then Han bigrams.

    FTS5 has no CJK tokenizer without a loadable extension, so tokenization
    lives here and serves both sides: the index stores this stream and every
    query reuses it.  Indexing unigrams keeps one and two character Han
    queries on the index instead of degrading into a full-table scan.
    """
    tokens: list[str] = []
    for segment in _segments(value):
        if isinstance(segment, str):
            tokens.append(segment)
            continue
        tokens.extend(segment)
        tokens.extend(_bigrams(segment))
    return " ".join(tokens)


def fts_match_expression(query: str) -> str | None:
    """Build the FTS5 MATCH expression, or None when nothing is indexable.

    A Han run of two or more characters becomes a phrase of its consecutive
    bigrams, so it matches only that exact substring; a single Han character
    matches the indexed unigram.  Terms are quoted (an ordinary word such as
    ``or`` must not be read as an FTS5 operator), deduplicated and bounded.
    """
    terms: list[str] = []
    seen: set[str] = set()
    for segment in _segments(query):
        if isinstance(segment, str):
            term = _quoted(segment)
        elif len(segment) == 1:
            term = _quoted(segment[0])
        else:
            phrase = " ".join(
                _bigrams(segment)[:MEMORY_RETRIEVAL_FTS_MAX_PHRASE_BIGRAMS]
            )
            term = _quoted(phrase)
        if term in seen:
            continue
        seen.add(term)
        terms.append(term)
        if len(terms) == MEMORY_RETRIEVAL_FTS_MAX_TERMS:
            break
    return " OR ".join(terms) if terms else None


def _quoted(term: str) -> str:
    return '"' + term.replace('"', '""') + '"'


def _segments(value: str) -> tuple[str | tuple[str, ...], ...]:
    """Ordered NFKC/casefold segments: Latin words and maximal Han runs."""
    segments: list[str | tuple[str, ...]] = []
    word: list[str] = []
    han: list[str] = []

    def flush_word() -> None:
        if word:
            segments.append("".join(word))
            word.clear()

    def flush_han() -> None:
        if han:
            segments.append(tuple(han))
            han.clear()

    for character in _normalize(value):
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
    return tuple(segments)


def _tokenize(value: str) -> _Tokens:
    words: set[str] = set()
    han_unigrams: set[str] = set()
    han_bigrams: set[str] = set()
    for segment in _segments(value):
        if isinstance(segment, str):
            words.add(segment)
            continue
        han_unigrams.update(segment)
        han_bigrams.update(_bigrams(segment))
    return _Tokens(frozenset(words), frozenset(han_unigrams), frozenset(han_bigrams))


def _normalize(value: str) -> str:
    return unicodedata.normalize(
        "NFKC",
        unicodedata.normalize("NFKC", value).casefold(),
    )


def _bigrams(han: Sequence[str]) -> list[str]:
    return [left + right for left, right in zip(han, han[1:], strict=False)]


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
    "MEMORY_RETRIEVAL_POOL_DEPTH",
    "MEMORY_RETRIEVAL_RRF_K",
    "MEMORY_RETRIEVAL_TOP_K",
    "MEMORY_RETRIEVAL_VERSION",
    "FrozenMemoryScope",
    "MaterializedMemoryV1",
    "MemoryRetrievalCandidateV1",
    "MemoryRetrievalOmissionV1",
    "MemoryRetrievalV1",
    "MemoryRetrieverV1",
    "MemorySemanticPool",
    "MemorySnapshotReferenceV1",
    "fts_match_expression",
    "fts_token_stream",
]
