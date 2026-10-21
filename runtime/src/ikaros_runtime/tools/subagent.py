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
        wait: bool = True,
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
                "self-contained sub-task. By default blocks until the child settles "
                "and returns its final answer; set wait=false to enqueue the child and "
                "return its threadId/runId immediately. A non-blocking child keeps "
                "running after this Run completes normally, but is cancelled if this "
                "Run is cancelled. Collect settled results with thread_read using the "
                "returned threadId; while the child is unsettled, thread_read includes "
                "activeRun and a notice. Children can spawn further subagents up to the "
                "configured depth."
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
                    "wait": {
                        "type": "boolean",
                        "description": (
                            "Wait for the child Run to settle before returning. "
                            "Default true."
                        ),
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
            optional={"title", "wait"},
        )
        if problem is not None:
            return ToolResult.rejected(call, code="invalid_arguments", message=problem)
        prompt = call.arguments["prompt"]
        title = call.arguments.get("title")
        wait = call.arguments.get("wait", True)
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
        if not isinstance(wait, bool):
            return ToolResult.rejected(
                call, code="invalid_arguments", message="wait must be a boolean"
            )
        cancellation.raise_if_cancelled()
        try:
            payload = await self.backend.spawn(
                parent_run_id=context.run_id,
                request_id=context.item_id,
                prompt=prompt,
                title=title,
                wait=wait,
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
        thread_id = str(payload.get("threadId", ""))
        run_id = str(payload.get("runId", ""))
        if not wait and status in {"queued", "running"}:
            output = (
                f"Subagent dispatched as thread {thread_id} / run {run_id} "
                f"(status: {status}). It keeps running after this Run ends unless "
                "this Run is cancelled; collect settled results with thread_read."
            )
            ok = True
        elif status == "completed":
            output = summary or "Subagent completed without a final message."
            ok = True
        else:
            output = f"Subagent {status}."
            ok = False
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=ok,
            output=output,
            details={
                "threadId": thread_id,
                "runId": run_id,
                "status": status,
            },
        )


__all__ = ["SUBAGENT_TOOL_NAME", "SubagentTool"]
