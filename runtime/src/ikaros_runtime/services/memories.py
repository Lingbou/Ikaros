"""Application service for explicit, user-managed durable Memory."""

from __future__ import annotations

import sqlite3
from collections.abc import Callable
from dataclasses import replace
from typing import Any, Protocol, cast

from ..errors import InvalidParamsError, MemoryOperationError
from ..memory import MEMORY_KINDS, MEMORY_STATES, MemoryScope, SqliteMemoryStore
from ..memory.domain import (
    MemoryKind,
    MemoryScopeType,
    MemorySourceSnapshot,
    MemoryState,
    validate_memory_content,
)
from ..memory.store import MEMORY_LIST_DEFAULT_LIMIT, MEMORY_LIST_MAX_LIMIT
from ..storage.session_provenance import SessionItemProvenanceSnapshotV1
from .threads import client_request_id_from

RequestSafetyCheck = Callable[[object], None]
_SQLITE_MAX_INTEGER = (1 << 63) - 1


class SessionProvenanceReader(Protocol):
    def capture_session_item_provenance(
        self,
        item_id: str,
    ) -> SessionItemProvenanceSnapshotV1: ...

    def session_item_provenance_is_available(
        self,
        expected: SessionItemProvenanceSnapshotV1,
    ) -> bool: ...


class MemoryService:
    def __init__(
        self,
        store: SqliteMemoryStore,
        assert_request_safe: RequestSafetyCheck,
        session_provenance: SessionProvenanceReader,
    ) -> None:
        self._store = store
        self._assert_request_safe = assert_request_safe
        self._session_provenance = session_provenance

    def create(self, params: dict[str, Any]) -> dict[str, object]:
        required = {"kind", "scope", "content", "clientRequestId"}
        keys = set(params)
        if keys not in (required, required | {"source"}):
            raise InvalidParamsError(
                "memory.create requires exactly kind, scope, content, clientRequestId, "
                "and optionally source"
            )
        kind = _memory_kind_from(params["kind"], method="memory.create")
        scope = _memory_scope_from(params["scope"], method="memory.create")
        try:
            content = validate_memory_content(params["content"])
        except ValueError as error:
            raise InvalidParamsError(str(error)) from error
        client_request_id = client_request_id_from(params)
        if client_request_id is None:  # pragma: no cover - exact keys require a value
            raise InvalidParamsError("memory.create clientRequestId is required")
        source_item_id = (
            _memory_source_item_id_from(params["source"])
            if "source" in params
            else None
        )
        self._assert_request_safe((scope.key, content, client_request_id))
        existing = self._store.replay_create_receipt(
            kind=kind,
            scope=scope,
            content=content,
            client_request_id=client_request_id,
            source_item_id=source_item_id,
        )
        if existing is not None:
            return existing.to_wire()
        source: MemorySourceSnapshot | None = None
        if source_item_id is not None:
            try:
                captured = self._session_provenance.capture_session_item_provenance(
                    source_item_id
                )
                source = MemorySourceSnapshot(
                    thread_id=captured.thread_id,
                    turn_id=captured.turn_id,
                    item_id=captured.item_id,
                    item_digest=captured.item_digest,
                )
            except (
                LookupError,
                RuntimeError,
                TypeError,
                ValueError,
                sqlite3.Error,
            ) as error:
                raise MemoryOperationError("memory_source_unavailable") from error
        receipt = self._store.create_memory_once(
            kind=kind,
            scope=scope,
            content=content,
            client_request_id=client_request_id,
            source=source,
        )
        return receipt.to_wire()

    def correct(self, params: dict[str, Any]) -> dict[str, object]:
        if set(params) != {
            "memoryId",
            "expectedRevision",
            "content",
            "clientRequestId",
        }:
            raise InvalidParamsError(
                "memory.correct requires exactly memoryId, expectedRevision, content, "
                "and clientRequestId"
            )
        memory_id = _memory_id_from(params["memoryId"])
        expected_revision = _expected_revision_from(params["expectedRevision"])
        try:
            content = validate_memory_content(params["content"])
        except ValueError as error:
            raise InvalidParamsError(str(error)) from error
        client_request_id = client_request_id_from(params)
        if client_request_id is None:  # pragma: no cover - exact keys require a value
            raise InvalidParamsError("memory.correct clientRequestId is required")
        self._assert_request_safe((content, client_request_id))
        return self._store.correct_memory_once(
            memory_id=memory_id,
            expected_revision=expected_revision,
            content=content,
            client_request_id=client_request_id,
        ).to_wire()

    def forget(self, params: dict[str, Any]) -> dict[str, object]:
        if set(params) != {"memoryId", "expectedRevision", "clientRequestId"}:
            raise InvalidParamsError(
                "memory.forget requires exactly memoryId, expectedRevision, and clientRequestId"
            )
        memory_id = _memory_id_from(params["memoryId"])
        expected_revision = _expected_revision_from(params["expectedRevision"])
        client_request_id = client_request_id_from(params)
        if client_request_id is None:  # pragma: no cover - exact keys require a value
            raise InvalidParamsError("memory.forget clientRequestId is required")
        self._assert_request_safe(client_request_id)
        return self._store.forget_memory_once(
            memory_id=memory_id,
            expected_revision=expected_revision,
            client_request_id=client_request_id,
        ).to_wire()

    def list(self, params: dict[str, Any]) -> dict[str, object]:
        unknown = set(params) - {"cursor", "limit", "scope", "kind", "state"}
        if unknown:
            raise InvalidParamsError("memory.list contains unsupported parameters")
        cursor: str | None = None
        if "cursor" in params:
            raw_cursor = params["cursor"]
            if not isinstance(raw_cursor, str) or not raw_cursor:
                raise InvalidParamsError("memory.list cursor must be a non-empty string")
            cursor = raw_cursor
        limit = params.get("limit", MEMORY_LIST_DEFAULT_LIMIT)
        if isinstance(limit, bool) or not isinstance(limit, int):
            raise InvalidParamsError("memory.list limit must be an integer")
        if limit < 1 or limit > MEMORY_LIST_MAX_LIMIT:
            raise InvalidParamsError(
                f"memory.list limit must be between 1 and {MEMORY_LIST_MAX_LIMIT}"
            )
        scope = (
            _memory_scope_from(params["scope"], method="memory.list")
            if "scope" in params
            else None
        )
        kind = (
            _memory_kind_from(params["kind"], method="memory.list")
            if "kind" in params
            else None
        )
        raw_state = params.get("state", "active")
        if not isinstance(raw_state, str) or raw_state not in MEMORY_STATES:
            raise InvalidParamsError("memory.list state is invalid")
        state = cast(MemoryState, raw_state)
        try:
            page = self._store.list_memories(
                cursor=cursor,
                limit=limit,
                scope=scope,
                kind=kind,
                state=state,
            )
        except ValueError as error:
            raise InvalidParamsError(str(error)) from error
        return page.to_wire()

    def get(self, params: dict[str, Any]) -> dict[str, object]:
        if set(params) != {"memoryId"}:
            raise InvalidParamsError("memory.get requires exactly one memoryId")
        memory_id = _memory_id_from(params["memoryId"])
        memory = self._store.get_memory(memory_id)
        source = self._store.current_source_snapshot(memory_id)
        if source is not None:
            expected = SessionItemProvenanceSnapshotV1(
                thread_id=source.thread_id,
                turn_id=source.turn_id,
                item_id=source.item_id,
                item_digest=source.item_digest,
            )
            try:
                available = self._session_provenance.session_item_provenance_is_available(
                    expected
                )
            except Exception:
                available = False
            memory = replace(
                memory,
                provenance=replace(
                    memory.provenance,
                    status="available" if available else "unavailable",
                ),
            )
        return {"memory": memory.to_wire()}


def _memory_kind_from(value: object, *, method: str) -> MemoryKind:
    if not isinstance(value, str) or value not in MEMORY_KINDS:
        raise InvalidParamsError(f"{method} kind is invalid")
    return cast(MemoryKind, value)


def _memory_scope_from(value: object, *, method: str) -> MemoryScope:
    if not isinstance(value, dict) or set(value) != {"type", "key"}:
        raise InvalidParamsError(f"{method} scope requires exactly type and key")
    scope_type = value["type"]
    scope_key = value["key"]
    if not isinstance(scope_type, str) or (
        scope_key is not None and not isinstance(scope_key, str)
    ):
        raise InvalidParamsError(f"{method} scope is invalid")
    try:
        return MemoryScope(type=cast(MemoryScopeType, scope_type), key=scope_key)
    except ValueError as error:
        raise InvalidParamsError(str(error)) from error


def _memory_id_from(value: object) -> str:
    if not isinstance(value, str) or not value.startswith("memory_"):
        raise InvalidParamsError("memoryId is invalid")
    suffix = value.removeprefix("memory_")
    if len(suffix) != 32 or any(
        character not in "0123456789abcdef" for character in suffix
    ):
        raise InvalidParamsError("memoryId is invalid")
    return value


def _memory_source_item_id_from(value: object) -> str:
    if not isinstance(value, dict) or set(value) != {"type", "itemId"}:
        raise InvalidParamsError(
            "memory.create source requires exactly type and itemId"
        )
    if value["type"] != "session_item":
        raise InvalidParamsError("memory.create source type is invalid")
    item_id = value["itemId"]
    if not isinstance(item_id, str) or not item_id.startswith("item_"):
        raise InvalidParamsError("memory.create source itemId is invalid")
    suffix = item_id.removeprefix("item_")
    if len(suffix) != 32 or any(
        character not in "0123456789abcdef" for character in suffix
    ):
        raise InvalidParamsError("memory.create source itemId is invalid")
    return item_id


def _expected_revision_from(value: object) -> int:
    if (
        isinstance(value, bool)
        or not isinstance(value, int)
        or value < 1
        or value >= _SQLITE_MAX_INTEGER
    ):
        raise InvalidParamsError("expectedRevision is invalid")
    return value


__all__ = ["MemoryService"]
