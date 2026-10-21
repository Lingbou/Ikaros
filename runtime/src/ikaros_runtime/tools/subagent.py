"""Spawn a child conversation from inside a Run."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol

from ..cancellation import CancellationToken
from ..domain import JsonObject
from .core import (
    ToolCall,
    ToolDefinition,
    ToolExecutionContext,
    ToolResult,
    require_exact_arguments,
)

SUBAGENT_TOOL_NAME = "subagent"
SUBAGENT_MAX_TITLE_CHARS = 200


class SubagentBackend(Protocol):
    async def spawn(
        self,
        *,
        parent_run_id: str,
        request_id: str,
        prompt: str,
        title: str | None,
        cancellation: CancellationToken,
    ) -> JsonObject: ...


@dataclass(frozen=True, slots=True)
class SubagentTool:
    """Run a self-contained sub-task in a child conversation."""

    backend: SubagentBackend

    @property
    def definition(self) -> ToolDefinition:
        return ToolDefinition(
            name=SUBAGENT_TOOL_NAME,
            description=(
                "Spawn a child conversation (its own thread and run) for a "
                "self-contained sub-task. Blocks until the child settles and returns "
                "its final answer; use thread_read with the returned threadId for "
                "details. Children cannot spawn further subagents."
            ),
            input_schema={
                "type": "object",
                "properties": {
                    "prompt": {"type": "string", "minLength": 1},
                    "title": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": SUBAGENT_MAX_TITLE_CHARS,
                    },
                },
                "required": ["prompt"],
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
        cancellation.raise_if_cancelled()
        problem = require_exact_arguments(
            call.arguments,
            required={"prompt"},
            optional={"title"},
        )
        if problem is not None:
            return ToolResult.rejected(call, code="invalid_arguments", message=problem)
        prompt = call.arguments["prompt"]
        title = call.arguments.get("title")
        if not isinstance(prompt, str) or not prompt.strip():
            return ToolResult.rejected(
                call, code="invalid_arguments", message="prompt must be a non-empty string"
            )
        if title is not None and (
            not isinstance(title, str)
            or not title.strip()
            or len(title) > SUBAGENT_MAX_TITLE_CHARS
        ):
            return ToolResult.rejected(
                call, code="invalid_arguments", message="title is invalid"
            )
        cancellation.raise_if_cancelled()
        try:
            payload = await self.backend.spawn(
                parent_run_id=context.run_id,
                request_id=context.item_id,
                prompt=prompt,
                title=title,
                cancellation=cancellation,
            )
        except ValueError as error:
            return ToolResult.rejected(
                call,
                code=str(getattr(error, "code", "subagent_failed")),
                message=str(error),
            )
        except LookupError as error:
            return ToolResult.rejected(
                call, code="subagent_not_found", message=str(error)
            )
        cancellation.raise_if_cancelled()
        status = str(payload.get("status", "failed"))
        summary = str(payload.get("summary", ""))
        if status == "completed":
            output = summary or "Subagent completed without a final message."
        else:
            output = f"Subagent {status}."
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=status == "completed",
            output=output,
            details={
                "threadId": str(payload.get("threadId", "")),
                "runId": str(payload.get("runId", "")),
                "status": status,
            },
        )


__all__ = ["SUBAGENT_TOOL_NAME", "SubagentTool"]