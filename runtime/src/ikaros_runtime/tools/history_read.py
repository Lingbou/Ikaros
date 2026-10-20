"""Model-facing bounded recovery of persisted conversation history."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

from ..cancellation import CancellationToken
from ..domain import JsonObject
from ..json_codec import dumps as json_dumps
from ..storage.history_read import (
    MAX_HISTORY_READ_CHARS,
    MAX_HISTORY_READ_ITEMS,
    MIN_HISTORY_READ_CHARS,
    MIN_HISTORY_READ_ITEMS,
)
from .core import (
    ToolCall,
    ToolDefinition,
    ToolExecutionContext,
    ToolResult,
    is_json_integer,
    require_exact_arguments,
)

HistoryReader = Callable[..., JsonObject]


@dataclass(frozen=True, slots=True)
class HistoryReadTool:
    """Read one bounded slice through a Runtime-owned reader.

    Injecting the reader keeps storage ownership in the composition root and
    makes the tool impossible to retarget through model-supplied thread IDs.
    """

    reader: HistoryReader

    definition = ToolDefinition(
        name="history_read",
        description="Read a bounded slice of settled conversation Items around an Item ID.",
        input_schema={
            "type": "object",
            "properties": {
                "itemId": {"type": "string", "minLength": 1},
                "before": {
                    "type": "integer",
                    "minimum": MIN_HISTORY_READ_ITEMS,
                    "maximum": MAX_HISTORY_READ_ITEMS,
                },
                "after": {
                    "type": "integer",
                    "minimum": MIN_HISTORY_READ_ITEMS,
                    "maximum": MAX_HISTORY_READ_ITEMS,
                },
                "maxChars": {
                    "type": "integer",
                    "minimum": MIN_HISTORY_READ_CHARS,
                    "maximum": MAX_HISTORY_READ_CHARS,
                },
            },
            "required": ["itemId"],
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
            required={"itemId"},
            optional={"before", "after", "maxChars"},
        )
        if mismatch is not None:
            return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)
        item_id = call.arguments["itemId"]
        if not isinstance(item_id, str) or not item_id:
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message="itemId must be a non-empty string",
            )
        limits = {
            "before": call.arguments.get("before", 10),
            "after": call.arguments.get("after", 10),
            "max_chars": call.arguments.get("maxChars", MAX_HISTORY_READ_CHARS),
        }
        if not all(is_json_integer(value) for value in limits.values()):
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message="history limits must be integers",
            )
        if (
            not MIN_HISTORY_READ_ITEMS <= limits["before"] <= MAX_HISTORY_READ_ITEMS
            or not MIN_HISTORY_READ_ITEMS <= limits["after"] <= MAX_HISTORY_READ_ITEMS
            or not MIN_HISTORY_READ_CHARS <= limits["max_chars"] <= MAX_HISTORY_READ_CHARS
        ):
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message="history limits are out of range",
            )
        cancellation.raise_if_cancelled()
        try:
            payload = self.reader(
                thread_id=context.thread_id,
                item_id=item_id,
                before=limits["before"],
                after=limits["after"],
                max_chars=limits["max_chars"],
            )
        except LookupError as error:
            return ToolResult.rejected(call, code="history_unavailable", message=str(error))
        except ValueError as error:
            return ToolResult.rejected(call, code="invalid_arguments", message=str(error))
        cancellation.raise_if_cancelled()
        output = json_dumps(payload, ensure_ascii=False, separators=(",", ":"))
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output=output,
            details={"itemId": item_id, "truncated": bool(payload["truncated"])},
        )


__all__ = ["HistoryReadTool", "HistoryReader"]
