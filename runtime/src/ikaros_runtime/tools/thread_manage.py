"""Model-facing management of other conversation Threads.

The four Tools are thin: they validate model arguments, delegate to the
Runtime-owned backend (``services.thread_messaging``), and project the result.
Storage, security, idempotency, and the cross-session hop chain stay in the
Service so the model cannot retarget them.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol

from ..cancellation import CancellationToken
from ..domain import JsonObject
from ..json_codec import dumps as json_dumps
from .core import (
    ToolCall,
    ToolDefinition,
    ToolExecutionContext,
    ToolResult,
    require_exact_arguments,
)

THREAD_CREATE_TOOL_NAME = "thread_create"
THREAD_RENAME_TOOL_NAME = "thread_rename"
THREAD_ARCHIVE_TOOL_NAME = "thread_archive"
THREAD_SEND_TOOL_NAME = "thread_send"

MAX_THREAD_ID_CHARS = 200
MAX_THREAD_TITLE_CHARS = 200
MAX_THREAD_WORKSPACE_CHARS = 4_096
MAX_THREAD_MESSAGE_CHARS = 32_768


class ThreadManagementBackend(Protocol):
    """Runtime-owned Thread operations that model arguments cannot retarget."""

    async def create(
        self,
        *,
        title: str | None,
        workspace: str | None,
        request_id: str,
        parent_run_id: str,
    ) -> JsonObject: ...

    async def rename(self, *, thread_id: str, title: str) -> JsonObject: ...

    async def set_archived(self, *, thread_id: str, archived: bool) -> JsonObject: ...

    async def send(
        self,
        *,
        thread_id: str,
        content: str,
        request_id: str,
        parent_run_id: str,
        wait: bool,
        cancellation: CancellationToken,
    ) -> JsonObject: ...


def _rejected(call: ToolCall, error: Exception) -> ToolResult:
    code = getattr(error, "code", None)
    if not isinstance(code, str) or not code:
        code = "invalid_arguments"
    return ToolResult.rejected(call, code=code, message=str(error))


def _title_problem(title: object) -> str | None:
    if not isinstance(title, str) or not title.strip():
        return "title must be a non-empty string"
    if len(title.strip()) > MAX_THREAD_TITLE_CHARS:
        return f"title must be at most {MAX_THREAD_TITLE_CHARS} characters"
    return None


def _thread_id_argument(call: ToolCall) -> str | ToolResult:
    thread_id = call.arguments.get("threadId")
    if (
        not isinstance(thread_id, str)
        or not thread_id
        or len(thread_id) > MAX_THREAD_ID_CHARS
    ):
        return ToolResult.rejected(
            call,
            code="invalid_arguments",
            message="threadId must be a non-empty Thread identifier",
        )
    return thread_id


@dataclass(frozen=True, slots=True)
class ThreadCreateTool:
    """Create a conversation Thread the model can then message."""

    backend: ThreadManagementBackend

    definition = ToolDefinition(
        name=THREAD_CREATE_TOOL_NAME,
        description=(
            "Create a new conversation Thread, optionally with a title and an existing "
            "project directory as its workspace. Without a workspace the new Thread "
            "inherits the calling Thread's project. Returns the new threadId; use "
            "thread_send to start work in it. Replaying the same Tool Call returns the "
            "same Thread."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_THREAD_TITLE_CHARS,
                },
                "workspace": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_THREAD_WORKSPACE_CHARS,
                },
            },
            "additionalProperties": False,
        },
        mutating=True,
    )

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
        context: ToolExecutionContext,
    ) -> ToolResult:
        cancellation.raise_if_cancelled()
        mismatch = require_exact_arguments(
            call.arguments,
            required=set(),
            optional={"title", "workspace"},
        )
        if mismatch is not None:
            return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)
        title = call.arguments.get("title")
        workspace = call.arguments.get("workspace")
        if title is not None:
            problem = _title_problem(title)
            if problem is not None:
                return ToolResult.rejected(
                    call,
                    code="invalid_arguments",
                    message=problem,
                )
        if workspace is not None and (
            not isinstance(workspace, str)
            or not workspace.strip()
            or len(workspace) > MAX_THREAD_WORKSPACE_CHARS
        ):
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message=(
                    "workspace must be an existing absolute directory path of at most "
                    f"{MAX_THREAD_WORKSPACE_CHARS} characters"
                ),
            )
        cancellation.raise_if_cancelled()
        try:
            payload = await self.backend.create(
                title=title.strip() if isinstance(title, str) else None,
                workspace=workspace.strip() if isinstance(workspace, str) else None,
                request_id=context.item_id,
                parent_run_id=context.run_id,
            )
        except ValueError as error:
            return _rejected(call, error)
        cancellation.raise_if_cancelled()
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output=json_dumps(payload, ensure_ascii=False, separators=(",", ":")),
            details={
                "threadId": str(payload.get("threadId", "")),
                "created": bool(payload.get("created", False)),
            },
        )


@dataclass(frozen=True, slots=True)
class ThreadRenameTool:
    """Rename another Thread."""

    backend: ThreadManagementBackend

    definition = ToolDefinition(
        name=THREAD_RENAME_TOOL_NAME,
        description=(
            "Rename another conversation Thread. Renaming to the current title is "
            "harmless and reports changed=false."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "threadId": {"type": "string", "minLength": 1},
                "title": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_THREAD_TITLE_CHARS,
                },
            },
            "required": ["threadId", "title"],
            "additionalProperties": False,
        },
        mutating=True,
    )

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
        context: ToolExecutionContext,
    ) -> ToolResult:
        del context
        cancellation.raise_if_cancelled()
        mismatch = require_exact_arguments(
            call.arguments,
            required={"threadId", "title"},
            optional=set(),
        )
        if mismatch is not None:
            return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)
        thread_id = _thread_id_argument(call)
        if isinstance(thread_id, ToolResult):
            return thread_id
        title = call.arguments["title"]
        problem = _title_problem(title)
        if problem is not None:
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message=problem,
            )
        cancellation.raise_if_cancelled()
        try:
            payload = await self.backend.rename(thread_id=thread_id, title=title.strip())
        except ValueError as error:
            return _rejected(call, error)
        cancellation.raise_if_cancelled()
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output=json_dumps(payload, ensure_ascii=False, separators=(",", ":")),
            details={
                "threadId": thread_id,
                "changed": bool(payload.get("changed", False)),
            },
        )


@dataclass(frozen=True, slots=True)
class ThreadArchiveTool:
    """Archive or unarchive another Thread."""

    backend: ThreadManagementBackend

    definition = ToolDefinition(
        name=THREAD_ARCHIVE_TOOL_NAME,
        description=(
            "Archive another conversation Thread, or unarchive it with archived=false. "
            "Archived Threads refuse new messages until they are unarchived. Repeating "
            "the same state reports changed=false."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "threadId": {"type": "string", "minLength": 1},
                "archived": {"type": "boolean"},
            },
            "required": ["threadId"],
            "additionalProperties": False,
        },
        mutating=True,
    )

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
        context: ToolExecutionContext,
    ) -> ToolResult:
        del context
        cancellation.raise_if_cancelled()
        mismatch = require_exact_arguments(
            call.arguments,
            required={"threadId"},
            optional={"archived"},
        )
        if mismatch is not None:
            return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)
        thread_id = _thread_id_argument(call)
        if isinstance(thread_id, ToolResult):
            return thread_id
        archived = call.arguments.get("archived", True)
        if not isinstance(archived, bool):
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message="archived must be a boolean",
            )
        cancellation.raise_if_cancelled()
        try:
            payload = await self.backend.set_archived(
                thread_id=thread_id,
                archived=archived,
            )
        except ValueError as error:
            return _rejected(call, error)
        cancellation.raise_if_cancelled()
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output=json_dumps(payload, ensure_ascii=False, separators=(",", ":")),
            details={
                "threadId": thread_id,
                "archived": bool(payload.get("archived", archived)),
                "changed": bool(payload.get("changed", False)),
            },
        )


@dataclass(frozen=True, slots=True)
class ThreadSendTool:
    """Deliver one message to another Thread and start a Turn there."""

    backend: ThreadManagementBackend

    definition = ToolDefinition(
        name=THREAD_SEND_TOOL_NAME,
        description=(
            "Deliver one message to another conversation Thread and start a Turn/Run "
            "there. Returns immediately with threadId/runId by default; wait=true "
            "blocks until that Run settles and returns its final answer. The target "
            "Thread must be idle and unarchived, and cross-session message chains are "
            "capped, so reply to a message instead of looping."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "threadId": {"type": "string", "minLength": 1},
                "content": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_THREAD_MESSAGE_CHARS,
                },
                "wait": {"type": "boolean"},
            },
            "required": ["threadId", "content"],
            "additionalProperties": False,
        },
        mutating=True,
    )

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
        context: ToolExecutionContext,
    ) -> ToolResult:
        cancellation.raise_if_cancelled()
        mismatch = require_exact_arguments(
            call.arguments,
            required={"threadId", "content"},
            optional={"wait"},
        )
        if mismatch is not None:
            return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)
        thread_id = _thread_id_argument(call)
        if isinstance(thread_id, ToolResult):
            return thread_id
        content = call.arguments["content"]
        if (
            not isinstance(content, str)
            or not content.strip()
            or len(content) > MAX_THREAD_MESSAGE_CHARS
        ):
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message=(
                    "content must be a non-empty string of at most "
                    f"{MAX_THREAD_MESSAGE_CHARS} characters"
                ),
            )
        wait = call.arguments.get("wait", False)
        if not isinstance(wait, bool):
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message="wait must be a boolean",
            )
        cancellation.raise_if_cancelled()
        try:
            payload = await self.backend.send(
                thread_id=thread_id,
                content=content,
                request_id=context.item_id,
                parent_run_id=context.run_id,
                wait=wait,
                cancellation=cancellation,
            )
        except ValueError as error:
            return _rejected(call, error)
        cancellation.raise_if_cancelled()
        status = str(payload.get("status", "failed"))
        details: JsonObject = {
            "threadId": str(payload.get("threadId", thread_id)),
            "runId": str(payload.get("runId", "")),
            "status": status,
            "waited": wait,
        }
        if not wait:
            return ToolResult(
                tool_call_id=call.id,
                tool_name=call.name,
                ok=True,
                output=json_dumps(payload, ensure_ascii=False, separators=(",", ":")),
                details=details,
            )
        summary = str(payload.get("summary", ""))
        if status == "completed":
            return ToolResult(
                tool_call_id=call.id,
                tool_name=call.name,
                ok=True,
                output=summary or "Thread Run completed without a final message.",
                details=details,
            )
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=False,
            output=f"Thread Run {status}.",
            details=details,
        )


__all__ = [
    "MAX_THREAD_ID_CHARS",
    "MAX_THREAD_MESSAGE_CHARS",
    "MAX_THREAD_TITLE_CHARS",
    "MAX_THREAD_WORKSPACE_CHARS",
    "THREAD_ARCHIVE_TOOL_NAME",
    "THREAD_CREATE_TOOL_NAME",
    "THREAD_RENAME_TOOL_NAME",
    "THREAD_SEND_TOOL_NAME",
    "ThreadArchiveTool",
    "ThreadCreateTool",
    "ThreadManagementBackend",
    "ThreadRenameTool",
    "ThreadSendTool",
]
