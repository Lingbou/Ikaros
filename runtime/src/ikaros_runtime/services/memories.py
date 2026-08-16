"""Application service for explicit, user-managed durable Memory."""

from __future__ import annotations

from collections.abc import Callable
from typing import Any, cast

from ..errors import InvalidParamsError
from ..memory import MEMORY_KINDS, MEMORY_STATES, MemoryScope, SqliteMemoryStore
from ..memory.domain import (
    MemoryKind,
    MemoryScopeType,
    MemoryState,
    validate_memory_content,
)
from ..memory.store import MEMORY_LIST_DEFAULT_LIMIT, MEMORY_LIST_MAX_LIMIT
from .threads import client_request_id_from

RequestSafetyCheck = Callable[[object], None]


class MemoryService:
    def __init__(
        self,
        store: SqliteMemoryStore,
        assert_request_safe: RequestSafetyCheck,
    ) -> None:
        self._store = store
        self._assert_request_safe = assert_request_safe

    def create(self, params: dict[str, Any]) -> dict[str, object]:
        if set(params) != {"kind", "scope", "content", "clientRequestId"}:
            raise InvalidParamsError(
                "memory.create requires exactly kind, scope, content, and clientRequestId"
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
        self._assert_request_safe((scope.key, content, client_request_id))
        try:
            receipt = self._store.create_memory_once(
                kind=kind,
                scope=scope,
                content=content,
                client_request_id=client_request_id,
            )
        except ValueError as error:
            raise InvalidParamsError(str(error)) from error
        return receipt.to_wire()

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
        try:
            memory = self._store.get_memory(memory_id)
        except LookupError as error:
            raise InvalidParamsError(str(error)) from error
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
    if not isinstance(value, str):
        raise InvalidParamsError("memoryId is invalid")
    suffix = value.removeprefix("memory_")
    if len(suffix) != 32 or any(
        character not in "0123456789abcdef" for character in suffix
    ):
        raise InvalidParamsError("memoryId is invalid")
    return value


__all__ = ["MemoryService"]
