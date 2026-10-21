"""Model-facing, read-only access to other conversation Threads."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

from ..cancellation import CancellationToken
from ..domain import JsonObject
from ..json_codec import dumps as json_dumps
from .core import (
    ToolCall,
    ToolDefinition,
    ToolExecutionContext,
    ToolResult,
    is_json_integer,
    require_exact_arguments,
)

MIN_THREAD_LIST_LIMIT = 1
MAX_THREAD_LIST_LIMIT = 50
DEFAULT_THREAD_LIST_LIMIT = 20
MIN_THREAD_READ_ITEMS = 0
MAX_THREAD_READ_ITEMS = 41
DEFAULT_THREAD_READ_ITEMS = 10
MIN_THREAD_READ_CHARS = 256
MAX_THREAD_READ_CHARS = 24_000
DEFAULT_THREAD_READ_CHARS = 4_000


class ThreadReadError(LookupError):
    """Expected, model-visible Thread read failure."""

    code = "thread_unavailable"


class ThreadNotFoundError(ThreadReadError):
    code = "thread_not_found"


class ItemNotFoundError(ThreadReadError):
    code = "item_not_found"


class ItemNotSettledError(ThreadReadError):
    code = "item_not_settled"


ThreadCatalogReader = Callable[..., JsonObject]
ThreadHistoryReader = Callable[..., JsonObject]


@dataclass(frozen=True, slots=True)
class ThreadListTool:
    """List Threads through a Runtime-owned reader.

    Injecting the reader keeps storage ownership in the composition root and
    keeps the tool read-only.
    """

    reader: ThreadCatalogReader

    definition = ToolDefinition(
        name="thread_list",
        description=(
            "List conversation Threads with stable IDs so their settled history can be "
            "read with thread_read."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "archived": {"type": "boolean"},
                "limit": {
                    "type": "integer",
                    "minimum": MIN_THREAD_LIST_LIMIT,
                    "maximum": MAX_THREAD_LIST_LIMIT,
                },
            },
            "additionalProperties": False,
        },
    )

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
        context: ToolExecutionContext,
    ) -> ToolResult:
        mismatch = require_exact_arguments(
            call.arguments,
            required=set(),
            optional={"archived", "limit"},
        )
        if mismatch is not None:
            return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)
        archived = call.arguments.get("archived", False)
        limit = call.arguments.get("limit", DEFAULT_THREAD_LIST_LIMIT)
        if not isinstance(archived, bool):
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message="archived must be a boolean",
            )
        if (
            not is_json_integer(limit)
            or not MIN_THREAD_LIST_LIMIT <= limit <= MAX_THREAD_LIST_LIMIT
        ):
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message=(
                    "limit must be an integer between "
                    f"{MIN_THREAD_LIST_LIMIT} and {MAX_THREAD_LIST_LIMIT}"
                ),
            )
        cancellation.raise_if_cancelled()
        payload = self.reader(archived=archived, limit=limit)
        cancellation.raise_if_cancelled()
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output=json_dumps(payload, ensure_ascii=False, separators=(",", ":")),
            details={
                "archived": archived,
                "threadCount": len(payload["threads"]),
                "hasMore": bool(payload.get("hasMore", False)),
            },
        )


@dataclass(frozen=True, slots=True)
class ThreadReadTool:
    """Read a bounded slice of settled Items from one Thread by ID."""

    reader: ThreadHistoryReader

    definition = ToolDefinition(
        name="thread_read",
        description=(
            "Read a bounded slice of settled conversation Items from another Thread, "
            "optionally around an Item ID. Only final user/assistant messages and "
            "completed tool Items are visible."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "threadId": {"type": "string", "minLength": 1},
                "itemId": {"type": "string", "minLength": 1},
                "before": {
                    "type": "integer",
                    "minimum": MIN_THREAD_READ_ITEMS,
                    "maximum": MAX_THREAD_READ_ITEMS,
                },
                "after": {
                    "type": "integer",
                    "minimum": MIN_THREAD_READ_ITEMS,
                    "maximum": MAX_THREAD_READ_ITEMS,
                },
                "maxChars": {
                    "type": "integer",
                    "minimum": MIN_THREAD_READ_CHARS,
                    "maximum": MAX_THREAD_READ_CHARS,
                },
            },
            "required": ["threadId"],
            "additionalProperties": False,
        },
    )

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
        context: ToolExecutionContext,
    ) -> ToolResult:
        mismatch = require_exact_arguments(
            call.arguments,
            required={"threadId"},
            optional={"itemId", "before", "after", "maxChars"},
        )
        if mismatch is not None:
            return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)
        thread_id = call.arguments["threadId"]
        if not isinstance(thread_id, str) or not thread_id:
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message="threadId must be a non-empty string",
            )
        item_id = call.arguments.get("itemId")
        if item_id is not None and (not isinstance(item_id, str) or not item_id):
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message="itemId must be a non-empty string when supplied",
            )
        limits = {
            "before": call.arguments.get("before", DEFAULT_THREAD_READ_ITEMS),
            "after": call.arguments.get("after", DEFAULT_THREAD_READ_ITEMS),
            "max_chars": call.arguments.get("maxChars", DEFAULT_THREAD_READ_CHARS),
        }
        if not all(is_json_integer(value) for value in limits.values()):
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message="thread read limits must be integers",
            )
        if (
            not MIN_THREAD_READ_ITEMS <= limits["before"] <= MAX_THREAD_READ_ITEMS
            or not MIN_THREAD_READ_ITEMS <= limits["after"] <= MAX_THREAD_READ_ITEMS
            or not MIN_THREAD_READ_CHARS <= limits["max_chars"] <= MAX_THREAD_READ_CHARS
        ):
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message="thread read limits are out of range",
            )
        cancellation.raise_if_cancelled()
        try:
            payload = self.reader(
                thread_id=thread_id,
                item_id=item_id,
                before=limits["before"],
                after=limits["after"],
                max_chars=limits["max_chars"],
            )
        except ThreadReadError as error:
            return ToolResult.rejected(call, code=error.code, message=str(error))
        except ValueError as error:
            return ToolResult.rejected(call, code="invalid_arguments", message=str(error))
        cancellation.raise_if_cancelled()
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output=json_dumps(payload, ensure_ascii=False, separators=(",", ":")),
            details={
                "threadId": thread_id,
                "itemId": payload.get("targetItemId"),
                "truncated": bool(payload["truncated"]),
            },
        )


__all__ = [
    "DEFAULT_THREAD_LIST_LIMIT",
    "DEFAULT_THREAD_READ_CHARS",
    "DEFAULT_THREAD_READ_ITEMS",
    "ItemNotFoundError",
    "ItemNotSettledError",
    "MAX_THREAD_LIST_LIMIT",
    "MAX_THREAD_READ_CHARS",
    "MAX_THREAD_READ_ITEMS",
    "MIN_THREAD_LIST_LIMIT",
    "MIN_THREAD_READ_CHARS",
    "MIN_THREAD_READ_ITEMS",
    "ThreadCatalogReader",
    "ThreadHistoryReader",
    "ThreadListTool",
    "ThreadNotFoundError",
    "ThreadReadError",
    "ThreadReadTool",
]
