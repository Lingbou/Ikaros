"""Model-facing save/search over durable explicit Memory.

The Memory vocabulary and limits live in ``memory.domain``, which reaches the
Run-input budget constants, which import ``tools.core``.  Importing them at
module scope would close an import cycle between this package and
``run_input``, so the Tool definition and the accepted vocabulary are resolved
once, on first use.
"""

from __future__ import annotations

from dataclasses import dataclass
from functools import lru_cache
from typing import Protocol

from ..cancellation import CancellationToken
from ..domain import JsonObject
from ..errors import InvalidParamsError, MemoryOperationError, MemoryRetrievalError
from ..json_codec import dumps as json_dumps
from .core import (
    ToolCall,
    ToolDefinition,
    ToolExecutionContext,
    ToolResult,
    is_json_integer,
    require_exact_arguments,
)

MEMORY_TOOL_ACTIONS = ("save", "search")
MEMORY_TOOL_SEARCH_DEFAULT_LIMIT = 5
MEMORY_TOOL_SEARCH_MAX_LIMIT = 8


class MemoryToolBackend(Protocol):
    """Runtime-owned Memory operations that model arguments cannot retarget."""

    def save_memory(
        self,
        *,
        run_id: str,
        item_id: str,
        kind: str,
        scope: str,
        content: str,
    ) -> JsonObject: ...

    def search_memories(
        self,
        *,
        run_id: str,
        query: str,
        limit: int,
    ) -> JsonObject: ...


@dataclass(frozen=True, slots=True)
class _SaveArguments:
    content: str
    kind: str
    scope: str


@dataclass(frozen=True, slots=True)
class _SearchArguments:
    query: str
    limit: int


@dataclass(frozen=True, slots=True)
class MemoryTool:
    """Save one durable Memory, or search saved Memory, inside the Run scope."""

    backend: MemoryToolBackend

    @property
    def definition(self) -> ToolDefinition:
        return memory_tool_definition()

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
        context: ToolExecutionContext,
    ) -> ToolResult:
        cancellation.raise_if_cancelled()
        action = call.arguments.get("action")
        if not isinstance(action, str) or action not in MEMORY_TOOL_ACTIONS:
            return _rejected(call, "action must be save or search")
        arguments: _SaveArguments | _SearchArguments | ToolResult = (
            _validate_save(call) if action == "save" else _validate_search(call)
        )
        if isinstance(arguments, ToolResult):
            return arguments
        cancellation.raise_if_cancelled()
        try:
            payload = self._dispatch(arguments, context)
        except InvalidParamsError as error:
            return _rejected(call, str(error))
        except MemoryOperationError as error:
            return ToolResult.rejected(
                call,
                code="memory_operation_failed",
                message=error.reason_code,
            )
        except MemoryRetrievalError as error:
            return ToolResult.rejected(
                call,
                code="memory_unavailable",
                message=error.reason_code,
            )
        except LookupError as error:
            return ToolResult.rejected(
                call,
                code="memory_unavailable",
                message=str(error),
            )
        cancellation.raise_if_cancelled()
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output=json_dumps(payload, ensure_ascii=False, separators=(",", ":")),
            details=_result_details(payload),
        )

    def _dispatch(
        self,
        arguments: _SaveArguments | _SearchArguments,
        context: ToolExecutionContext,
    ) -> JsonObject:
        if isinstance(arguments, _SaveArguments):
            return self.backend.save_memory(
                run_id=context.run_id,
                item_id=context.item_id,
                content=arguments.content,
                kind=arguments.kind,
                scope=arguments.scope,
            )
        return self.backend.search_memories(
            run_id=context.run_id,
            query=arguments.query,
            limit=arguments.limit,
        )


@dataclass(frozen=True, slots=True)
class _MemoryVocabulary:
    kinds: frozenset[str]
    scopes: frozenset[str]
    content_max_characters: int


@lru_cache(maxsize=1)
def _vocabulary() -> _MemoryVocabulary:
    """Resolve the Memory vocabulary once the import cycle has cleared."""

    from ..memory.domain import (
        MEMORY_CONTENT_MAX_CHARACTERS,
        MEMORY_KINDS,
        MEMORY_SCOPE_TYPES,
    )

    return _MemoryVocabulary(
        kinds=MEMORY_KINDS,
        scopes=MEMORY_SCOPE_TYPES,
        content_max_characters=MEMORY_CONTENT_MAX_CHARACTERS,
    )


def memory_tool_definition() -> ToolDefinition:
    """Return the frozen Tool definition of the model-facing ``memory`` tool."""

    vocabulary = _vocabulary()
    return ToolDefinition(
        name="memory",
        description=(
            "Save or search durable user Memory. save requires content, kind, and scope; "
            "search requires query and accepts limit 1-8 (default 5). Save only stable facts, "
            "preferences, relationships, or project context the user expects to persist; "
            "never save credentials or transient Run details."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": list(MEMORY_TOOL_ACTIONS)},
                "content": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": vocabulary.content_max_characters,
                },
                "kind": {"type": "string", "enum": sorted(vocabulary.kinds)},
                "scope": {"type": "string", "enum": sorted(vocabulary.scopes)},
                "query": {"type": "string", "minLength": 1},
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MEMORY_TOOL_SEARCH_MAX_LIMIT,
                },
            },
            "required": ["action"],
            "additionalProperties": False,
        },
        mutating=True,
    )


def _validate_save(call: ToolCall) -> _SaveArguments | ToolResult:
    vocabulary = _vocabulary()
    mismatch = require_exact_arguments(
        call.arguments,
        required={"action", "content", "kind", "scope"},
        optional=set(),
    )
    if mismatch is not None:
        return _rejected(call, mismatch)
    content = call.arguments["content"]
    kind = call.arguments["kind"]
    scope = call.arguments["scope"]
    if not isinstance(content, str):
        return _rejected(call, "content must be a string")
    if not isinstance(kind, str) or kind not in vocabulary.kinds:
        return _rejected(call, "kind is invalid")
    if not isinstance(scope, str) or scope not in vocabulary.scopes:
        return _rejected(call, "scope is invalid")
    return _SaveArguments(content=content, kind=kind, scope=scope)


def _validate_search(call: ToolCall) -> _SearchArguments | ToolResult:
    mismatch = require_exact_arguments(
        call.arguments,
        required={"action", "query"},
        optional={"limit"},
    )
    if mismatch is not None:
        return _rejected(call, mismatch)
    query = call.arguments["query"]
    if not isinstance(query, str) or not query.strip():
        return _rejected(call, "query must be a non-empty string")
    limit = call.arguments.get("limit", MEMORY_TOOL_SEARCH_DEFAULT_LIMIT)
    if not is_json_integer(limit):
        return _rejected(call, "limit must be an integer")
    if not 1 <= limit <= MEMORY_TOOL_SEARCH_MAX_LIMIT:
        return _rejected(
            call,
            f"limit must be between 1 and {MEMORY_TOOL_SEARCH_MAX_LIMIT}",
        )
    return _SearchArguments(query=query, limit=limit)


def _rejected(call: ToolCall, message: str) -> ToolResult:
    return ToolResult.rejected(call, code="invalid_arguments", message=message)


def _result_details(payload: JsonObject) -> JsonObject:
    details: JsonObject = {
        "action": payload["action"],
        "truncated": bool(payload.get("truncated")),
    }
    if payload["action"] == "save":
        details["memoryId"] = payload["memoryId"]
        details["created"] = payload["created"]
    else:
        details["count"] = payload["count"]
    return details


__all__ = [
    "MEMORY_TOOL_ACTIONS",
    "MEMORY_TOOL_SEARCH_DEFAULT_LIMIT",
    "MEMORY_TOOL_SEARCH_MAX_LIMIT",
    "MemoryTool",
    "MemoryToolBackend",
    "memory_tool_definition",
]
