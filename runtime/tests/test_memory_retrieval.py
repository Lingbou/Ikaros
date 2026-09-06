from __future__ import annotations

import hashlib
from dataclasses import fields, replace
from pathlib import Path

import pytest

from ikaros_runtime.errors import MemoryRetrievalError
from ikaros_runtime.json_codec import dumps as json_dumps
from ikaros_runtime.memory import (
    MEMORY_RETRIEVAL_MAX_CANDIDATES,
    MaterializedMemoryV1,
    MemoryRetrieverV1,
    MemoryScope,
    MemorySnapshotReferenceV1,
    SqliteMemoryStore,
)


def _create(
    store: SqliteMemoryStore,
    request_id: str,
    content: str,
    *,
    scope: MemoryScope | None = None,
) -> str:
    return store.create_memory_once(
        kind="fact",
        scope=scope or MemoryScope("global", None),
        content=content,
        client_request_id=request_id,
    ).memory_id


def _set_updated(store: SqliteMemoryStore, memory_id: str, timestamp: str) -> None:
    store._connection.execute(
        "UPDATE memory_records SET updated_at = ? WHERE id = ?",
        (timestamp, memory_id),
    )
    store._connection.commit()


def _content_of_length(length: int) -> str:
    assert length >= 4
    return "key " + "x" * (length - 4)


def _assert_retrieval_error(
    error: pytest.ExceptionInfo[MemoryRetrievalError],
    reason: str,
) -> None:
    assert error.value.reason_code == reason


def _stored_content_digest(content: str) -> str:
    canonical = json_dumps(
        content,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    )
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def _bulk_insert(store: SqliteMemoryStore, count: int) -> None:
    timestamp = "2026-08-17T12:00:00.000Z"
    with store._connection:
        for index in range(count):
            memory_id = f"memory_{index + 1:032x}"
            content = f"overflow keyword {index}"
            store._connection.execute(
                """
                INSERT INTO memory_records(
                    id, kind, scope_type, scope_key, current_revision, state,
                    created_at, updated_at, forgotten_at
                ) VALUES (?, 'fact', 'global', NULL, 1, 'active', ?, ?, NULL)
                """,
                (memory_id, timestamp, timestamp),
            )
            store._connection.execute(
                """
                INSERT INTO memory_revisions(
                    memory_id, revision, operation, content, content_digest,
                    content_redacted_at, source_kind, source_thread_id,
                    source_turn_id, source_item_id, source_item_digest, created_at
                ) VALUES (?, 1, 'create', ?, ?, NULL, 'user_explicit',
                          NULL, NULL, NULL, NULL, ?)
                """,
                (memory_id, content, _stored_content_digest(content), timestamp),
            )


def test_retrieval_normalizes_nfkc_casefold_and_filters_scope(tmp_path: Path) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    global_id = _create(store, "global", "Straße 用户偏好")
    workspace_id = _create(
        store,
        "workspace-a",
        "DeepSeek model 42",
        scope=MemoryScope("workspace", "workspace-a"),
    )
    _create(
        store,
        "workspace-b",
        "DeepSeek model 42",
        scope=MemoryScope("workspace", "workspace-b"),
    )
    retriever = MemoryRetrieverV1(store)
    try:
        result = retriever.retrieve(
            query="ＳＴＲＡＳＳＥ 用户偏好 deepseek ４２",
            workspace_id="workspace-a",
        )
        assert {memory.memory_id for memory in result.selected} == {
            global_id,
            workspace_id,
        }
        assert result.candidate_count == 2
        assert {memory.scope for memory in result.selected} == {"global", "workspace"}

        global_only = retriever.retrieve(query="strasse", workspace_id=None)
        assert [memory.memory_id for memory in global_only.selected] == [global_id]
        assert global_only.candidate_count == 1
    finally:
        store.close()


def test_retrieval_uses_weighted_unique_tokens_and_stable_ties(tmp_path: Path) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    alpha_id = _create(store, "alpha", "alpha alpha alpha")
    words_id = _create(store, "words", "alpha beta")
    han_id = _create(store, "han", "用户偏好")
    for memory_id in (alpha_id, words_id, han_id):
        _set_updated(store, memory_id, "2026-08-17T12:00:00.000Z")
    retriever = MemoryRetrieverV1(store)
    try:
        result = retriever.retrieve(query="alpha beta 用户偏好", workspace_id=None)
        assert [memory.memory_id for memory in result.selected] == [
            han_id,
            words_id,
            alpha_id,
        ]

        duplicate_a = _create(store, "duplicate-a", "needle")
        duplicate_b = _create(store, "duplicate-b", "needle needle needle")
        for memory_id in (duplicate_a, duplicate_b):
            _set_updated(store, memory_id, "2026-08-17T13:00:00.000Z")
        tied = retriever.retrieve(query="needle", workspace_id=None)
        assert [memory.memory_id for memory in tied.selected[:2]] == sorted(
            (duplicate_a, duplicate_b)
        )
    finally:
        store.close()


def test_punctuation_and_emoji_only_query_retrieves_nothing(tmp_path: Path) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    _create(store, "candidate", "a relevant-looking Memory")
    try:
        result = MemoryRetrieverV1(store).retrieve(query="🌸……！？", workspace_id=None)
        assert result.candidate_count == 1
        assert result.relevant_count == 0
        assert result.selected == ()
        assert result.omissions == ()
    finally:
        store.close()


@pytest.mark.parametrize(
    "candidate_count, overflows",
    (
        (MEMORY_RETRIEVAL_MAX_CANDIDATES, False),
        (MEMORY_RETRIEVAL_MAX_CANDIDATES + 1, True),
    ),
)
def test_candidate_limit_is_complete_or_explicit_overflow(
    tmp_path: Path,
    candidate_count: int,
    overflows: bool,
) -> None:
    store = SqliteMemoryStore(tmp_path / f"memory-{candidate_count}.db")
    _bulk_insert(store, candidate_count)
    try:
        if overflows:
            with pytest.raises(MemoryRetrievalError) as error:
                store.read_active_candidates(workspace_id=None)
            _assert_retrieval_error(error, "memory_retrieval_overflow")
        else:
            assert len(store.read_active_candidates(workspace_id=None)) == candidate_count
    finally:
        store.close()


def test_top_k_records_every_relevant_item_omitted_by_limit(tmp_path: Path) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    ids = [_create(store, f"top-{index}", f"needle {index}") for index in range(9)]
    for memory_id in ids:
        _set_updated(store, memory_id, "2026-08-17T12:00:00.000Z")
    try:
        result = MemoryRetrieverV1(store).retrieve(query="needle", workspace_id=None)
        expected = sorted(ids)
        assert [memory.memory_id for memory in result.selected] == expected[:8]
        assert [(item.memory_id, item.reason) for item in result.omissions] == [
            (expected[8], "omitted_by_limit")
        ]
        assert result.relevant_count == 9
    finally:
        store.close()


def test_character_budget_skips_whole_item_then_accepts_shorter_item(
    tmp_path: Path,
) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    lengths = (2048, 2048, 1900, 100, 4)
    ids = [
        _create(store, f"budget-{index}", _content_of_length(length))
        for index, length in enumerate(lengths)
    ]
    for index, memory_id in enumerate(ids):
        _set_updated(
            store,
            memory_id,
            f"2026-08-17T12:00:{50 - index:02d}.000Z",
        )
    try:
        result = MemoryRetrieverV1(store).retrieve(query="key", workspace_id=None)
        assert [memory.characters for memory in result.selected] == [2048, 2048, 1900, 4]
        assert result.memory_characters == 6000
        assert [(item.memory_id, item.characters, item.reason) for item in result.omissions] == [
            (ids[3], 100, "omitted_by_budget")
        ]
        assert all(
            memory.content == _content_of_length(memory.characters)
            for memory in result.selected
        )
    finally:
        store.close()


def test_capacity_budget_skips_large_candidates_before_filling_top_k(
    tmp_path: Path,
) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    lengths = (2048,) * 5 + (10,) * 9
    ids = [
        _create(store, f"capacity-{index}", _content_of_length(length))
        for index, length in enumerate(lengths)
    ]
    for index, memory_id in enumerate(ids):
        _set_updated(store, memory_id, f"2026-08-17T12:00:{50 - index:02d}.000Z")
    try:
        result = MemoryRetrieverV1(store).retrieve(
            query="key",
            workspace_id=None,
            accept_selection=lambda records: sum(record.characters for record in records)
            <= 100,
        )
        assert [memory.memory_id for memory in result.selected] == ids[5:13]
        assert [(item.memory_id, item.reason) for item in result.omissions] == [
            *((memory_id, "omitted_by_budget") for memory_id in ids[:5]),
            (ids[13], "omitted_by_limit"),
        ]
        assert result.memory_characters == 80
        assert result.relevant_count == len(ids)
    finally:
        store.close()


def test_snapshot_reference_omits_content_fingerprints(tmp_path: Path) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    memory_id = _create(store, "digest", "alpha\n\"quoted\"")
    try:
        selected = MemoryRetrieverV1(store).retrieve(query="alpha", workspace_id=None).selected[0]
        assert {field.name for field in fields(selected.reference)} == {
            "memory_id",
            "revision",
            "scope",
            "characters",
        }
        assert selected.memory_id == memory_id
        assert selected.content == "alpha\n\"quoted\""
    finally:
        store.close()


def test_exact_materialization_keeps_old_revision_after_correction_then_fails_forget(
    tmp_path: Path,
) -> None:
    memory_path = tmp_path / "memory.db"
    store = SqliteMemoryStore(memory_path)
    memory_id = _create(store, "lifecycle", "remember alpha")
    retriever = MemoryRetrieverV1(store)
    try:
        reference = retriever.retrieve(query="alpha", workspace_id=None).references[0]
        store.correct_memory_once(
            memory_id=memory_id,
            expected_revision=1,
            content="corrected alpha",
            client_request_id="correct-lifecycle",
        )

        store.close()
        store = SqliteMemoryStore(memory_path)
        retriever = MemoryRetrieverV1(store)
        materialized = retriever.materialize_exact((reference,), workspace_id=None)
        assert materialized == (MaterializedMemoryV1(reference, "remember alpha"),)

        store.forget_memory_once(
            memory_id=memory_id,
            expected_revision=2,
            client_request_id="forget-lifecycle",
        )
        with pytest.raises(MemoryRetrievalError) as unavailable:
            retriever.materialize_exact((reference,), workspace_id=None)
        _assert_retrieval_error(unavailable, "memory_snapshot_unavailable")
    finally:
        store.close()


def test_exact_materialization_preserves_order_and_validates_workspace(
    tmp_path: Path,
) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    first = _create(
        store,
        "workspace-first",
        "workspace alpha first",
        scope=MemoryScope("workspace", "workspace-a"),
    )
    second = _create(
        store,
        "workspace-second",
        "workspace alpha second",
        scope=MemoryScope("workspace", "workspace-a"),
    )
    retriever = MemoryRetrieverV1(store)
    try:
        result = retriever.retrieve(query="alpha", workspace_id="workspace-a")
        references = {reference.memory_id: reference for reference in result.references}
        reversed_references = (references[second], references[first])
        materialized = retriever.materialize_exact(
            reversed_references,
            workspace_id="workspace-a",
        )
        assert [memory.memory_id for memory in materialized] == [second, first]

        with pytest.raises(MemoryRetrievalError) as wrong_workspace:
            retriever.materialize_exact(reversed_references, workspace_id="workspace-b")
        _assert_retrieval_error(wrong_workspace, "memory_snapshot_unavailable")
    finally:
        store.close()


@pytest.mark.parametrize("mutation", ("missing_revision", "scope"))
def test_exact_materialization_rejects_reference_drift(
    tmp_path: Path,
    mutation: str,
) -> None:
    store = SqliteMemoryStore(tmp_path / f"memory-{mutation}.db")
    _create(store, "drift", "alpha snapshot")
    retriever = MemoryRetrieverV1(store)
    try:
        reference = retriever.retrieve(query="alpha", workspace_id=None).references[0]
        changed: MemorySnapshotReferenceV1
        if mutation == "missing_revision":
            changed = replace(reference, revision=99)
        else:
            changed = replace(reference, scope="workspace")
        with pytest.raises(MemoryRetrievalError) as unavailable:
            retriever.materialize_exact((changed,), workspace_id=None)
        _assert_retrieval_error(unavailable, "memory_snapshot_unavailable")
    finally:
        store.close()


def test_exact_materialization_rejects_stored_digest_drift_and_duplicate_refs(
    tmp_path: Path,
) -> None:
    store = SqliteMemoryStore(tmp_path / "memory.db")
    memory_id = _create(store, "stored-drift", "alpha snapshot")
    retriever = MemoryRetrieverV1(store)
    try:
        reference = retriever.retrieve(query="alpha", workspace_id=None).references[0]
        with pytest.raises(MemoryRetrievalError) as duplicate:
            retriever.materialize_exact((reference, reference), workspace_id=None)
        _assert_retrieval_error(duplicate, "memory_snapshot_unavailable")

        store._connection.execute(
            """
            UPDATE memory_revisions SET content_digest = ?
            WHERE memory_id = ? AND revision = 1
            """,
            ("0" * 64, memory_id),
        )
        store._connection.commit()
        with pytest.raises(MemoryRetrievalError) as drifted:
            retriever.materialize_exact((reference,), workspace_id=None)
        _assert_retrieval_error(drifted, "memory_snapshot_unavailable")
    finally:
        store.close()
