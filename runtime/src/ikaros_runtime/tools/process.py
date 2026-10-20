"""Model-facing managed-command tools. Starting and finishing are separate facts."""

from __future__ import annotations

import asyncio
from pathlib import Path

from ..cancellation import CancellationToken, RunCancelled
from ..domain import JsonObject
from .core import (
    ToolCall,
    ToolDefinition,
    ToolExecutionCancelled,
    ToolExecutionContext,
    ToolResult,
    ToolTaskCancelled,
    is_json_integer,
    require_exact_arguments,
)
from .process_manager import ProcessError, ProcessManager
from .process_platform import (
    PTY_DEFAULT_COLUMNS,
    PTY_DEFAULT_ROWS,
    PTY_MAX_CELLS,
    PTY_MIN_CELLS,
)

_MAX_WAIT_MS = 60_000
_COMMAND_LIMIT = 32_768
_CWD_LIMIT = 4_096
_STDIN_LIMIT = 32_768


class ProcessStartTool:
    definition = ToolDefinition(
        name="process_start",
        description=(
            "Start a local shell command and return its processId immediately. A running "
            "state does not mean success: use process_wait/read to inspect actual completion. "
            "Commands belong to this thread: they keep running after the Run ends (including "
            "cancellation) and stay readable from later Runs. Set background true to detach "
            "the command from this Run's completion check; foreground commands must be waited "
            "for or stopped before the Run can finish normally. Set pty true to attach an "
            "interactive terminal: process_write then sends input and process_resize changes "
            "its window size."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "command": {"type": "string", "minLength": 1, "maxLength": _COMMAND_LIMIT},
                "cwd": {"type": "string", "minLength": 1, "maxLength": _CWD_LIMIT},
                "background": {"type": "boolean"},
                "pty": {"type": "boolean"},
                "columns": {
                    "type": "integer",
                    "minimum": PTY_MIN_CELLS,
                    "maximum": PTY_MAX_CELLS,
                },
                "rows": {
                    "type": "integer",
                    "minimum": PTY_MIN_CELLS,
                    "maximum": PTY_MAX_CELLS,
                },
            },
            "required": ["command"],
            "additionalProperties": False,
        },
    )

    def __init__(self, manager: ProcessManager) -> None:
        self._manager = manager

    async def execute(
        self, call: ToolCall, *, cancellation: CancellationToken, context: ToolExecutionContext
    ) -> ToolResult:
        mismatch = require_exact_arguments(
            call.arguments,
            required={"command"},
            optional={"cwd", "background", "pty", "columns", "rows"},
        )
        if mismatch is not None:
            return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)
        terminal = _validate_terminal_arguments(call)
        if isinstance(terminal, ToolResult):
            return terminal
        pty, columns, rows = terminal
        command = call.arguments["command"]
        background = call.arguments.get("background", False)
        if not isinstance(command, str) or not command.strip() or len(command) > _COMMAND_LIMIT:
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message="command must be a non-empty bounded string.",
            )
        if not isinstance(background, bool):
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message="background must be a boolean.",
            )
        cwd_value = call.arguments.get("cwd", context.default_cwd)
        if cwd_value is not None and (
            not isinstance(cwd_value, str) or not cwd_value.strip() or len(cwd_value) > _CWD_LIMIT
        ):
            return ToolResult.rejected(
                call, code="invalid_arguments", message="cwd must be a non-empty bounded string."
            )
        try:
            resolved = _resolve_cwd(cwd_value, context.default_cwd)
        except (OSError, ValueError) as error:
            return ToolResult.rejected(call, code="invalid_cwd", message=str(error))
        try:
            fact = await self._manager.start(
                command,
                str(resolved),
                context=context,
                cancellation=cancellation,
                background=background,
                pty=pty,
                columns=columns,
                rows=rows,
            )
        except ProcessError as error:
            return ToolResult.rejected(call, code=error.code, message=str(error))
        return _result(call, fact)


def _definition(name: str, description: str, *, wait: bool = False) -> ToolDefinition:
    properties: JsonObject = {
        "processId": {"type": "string", "minLength": 1},
        "cursor": {"type": "integer", "minimum": 0},
    }
    if wait:
        properties["timeoutMs"] = {"type": "integer", "minimum": 1, "maximum": _MAX_WAIT_MS}
    return ToolDefinition(
        name=name,
        description=description,
        input_schema={
            "type": "object",
            "properties": properties,
            "required": ["processId"],
            "additionalProperties": False,
        },
    )


class ProcessReadTool:
    definition = _definition(
        "process_read",
        "Read current command status and up to 16 KiB of output. Pass nextCursor to read "
        "subsequent output without repeating it. This never starts or reruns a command.",
    )

    def __init__(self, manager: ProcessManager) -> None:
        self._manager = manager

    async def execute(
        self, call: ToolCall, *, cancellation: CancellationToken, context: ToolExecutionContext
    ) -> ToolResult:
        validated = _validate_reference(call)
        if isinstance(validated, ToolResult):
            return validated
        process_id, cursor, _ = validated
        cancellation.raise_if_cancelled()
        try:
            return _result(call, self._manager.read(process_id, context=context, cursor=cursor))
        except ProcessError as error:
            return ToolResult.rejected(call, code=error.code, message=str(error))


class ProcessWaitTool:
    definition = _definition(
        "process_wait",
        "Wait up to timeoutMs (default 1000, maximum 60000) for a command to finish, "
        "then read status and output. A wait timeout returns running and leaves the command alive. "
        "Use nextCursor on subsequent calls. Check state and exitCode before claiming success.",
        wait=True,
    )

    def __init__(self, manager: ProcessManager) -> None:
        self._manager = manager

    async def execute(
        self, call: ToolCall, *, cancellation: CancellationToken, context: ToolExecutionContext
    ) -> ToolResult:
        validated = _validate_reference(call, wait=True)
        if isinstance(validated, ToolResult):
            return validated
        process_id, cursor, timeout_ms = validated
        try:
            fact = await self._manager.wait(
                process_id,
                context=context,
                cancellation=cancellation,
                timeout_ms=timeout_ms,
                cursor=cursor,
            )
            return _result(call, fact)
        except (RunCancelled, asyncio.CancelledError) as interrupted:
            fact = self._manager.read(process_id, context=context, cursor=cursor)
            result = _result(call, fact)
            cancelled_result = ToolResult(
                tool_call_id=call.id,
                tool_name=call.name,
                ok=False,
                output=result.output,
                details={**result.details, "errorCode": "cancelled"},
                cancelled=True,
            )
            if isinstance(interrupted, asyncio.CancelledError):
                raise ToolTaskCancelled(cancelled_result) from None
            raise ToolExecutionCancelled(cancelled_result) from None
        except ProcessError as error:
            return ToolResult.rejected(call, code=error.code, message=str(error))


class ProcessStopTool:
    definition = _definition(
        "process_stop",
        "Stop a command and its descendant processes, and return the final status and output. "
        "Stopping an already finished command is harmless and never starts a new process.",
    )

    def __init__(self, manager: ProcessManager) -> None:
        self._manager = manager

    async def execute(
        self, call: ToolCall, *, cancellation: CancellationToken, context: ToolExecutionContext
    ) -> ToolResult:
        validated = _validate_reference(call)
        if isinstance(validated, ToolResult):
            return validated
        process_id, cursor, _ = validated
        cancellation.raise_if_cancelled()
        try:
            return _result(
                call, await self._manager.stop(process_id, context=context, cursor=cursor)
            )
        except ProcessError as error:
            return ToolResult.rejected(call, code=error.code, message=str(error))


def _validate_terminal_arguments(call: ToolCall) -> tuple[bool, int, int] | ToolResult:
    pty = call.arguments.get("pty", False)
    columns = call.arguments.get("columns", PTY_DEFAULT_COLUMNS)
    rows = call.arguments.get("rows", PTY_DEFAULT_ROWS)
    if not isinstance(pty, bool):
        return ToolResult.rejected(
            call, code="invalid_arguments", message="pty must be a boolean."
        )
    if not pty:
        if "columns" in call.arguments or "rows" in call.arguments:
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message="columns and rows require pty true.",
            )
        return False, columns, rows
    if (
        not is_json_integer(columns)
        or not PTY_MIN_CELLS <= columns <= PTY_MAX_CELLS
        or not is_json_integer(rows)
        or not PTY_MIN_CELLS <= rows <= PTY_MAX_CELLS
    ):
        return ToolResult.rejected(
            call,
            code="invalid_arguments",
            message=(
                f"columns and rows must be integers between {PTY_MIN_CELLS} "
                f"and {PTY_MAX_CELLS}."
            ),
        )
    return True, columns, rows


class ProcessWriteTool:
    definition = ToolDefinition(
        name="process_write",
        description=(
            "Write UTF-8 text to the standard input of a command started with pty true. "
            "The returned page shows terminal output produced so far; pass nextCursor to "
            "keep reading. This never starts or reruns a command."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "processId": {"type": "string", "minLength": 1},
                "data": {"type": "string", "minLength": 1, "maxLength": _STDIN_LIMIT},
                "cursor": {"type": "integer", "minimum": 0},
            },
            "required": ["processId", "data"],
            "additionalProperties": False,
        },
    )

    def __init__(self, manager: ProcessManager) -> None:
        self._manager = manager

    async def execute(
        self, call: ToolCall, *, cancellation: CancellationToken, context: ToolExecutionContext
    ) -> ToolResult:
        validated = _validate_terminal_command(call, required={"data"})
        if isinstance(validated, ToolResult):
            return validated
        process_id, cursor, _ = validated
        data = call.arguments["data"]
        if not isinstance(data, str) or not data or len(data) > _STDIN_LIMIT:
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message="data must be a non-empty bounded string.",
            )
        cancellation.raise_if_cancelled()
        try:
            return _result(
                call,
                await self._manager.write(
                    process_id, context=context, data=data, cursor=cursor
                ),
            )
        except ProcessError as error:
            return ToolResult.rejected(call, code=error.code, message=str(error))


class ProcessResizeTool:
    definition = ToolDefinition(
        name="process_resize",
        description=(
            "Resize the terminal of a command started with pty true. The returned page "
            "shows terminal output produced so far; pass nextCursor to keep reading."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "processId": {"type": "string", "minLength": 1},
                "columns": {
                    "type": "integer",
                    "minimum": PTY_MIN_CELLS,
                    "maximum": PTY_MAX_CELLS,
                },
                "rows": {
                    "type": "integer",
                    "minimum": PTY_MIN_CELLS,
                    "maximum": PTY_MAX_CELLS,
                },
                "cursor": {"type": "integer", "minimum": 0},
            },
            "required": ["processId", "columns", "rows"],
            "additionalProperties": False,
        },
    )

    def __init__(self, manager: ProcessManager) -> None:
        self._manager = manager

    async def execute(
        self, call: ToolCall, *, cancellation: CancellationToken, context: ToolExecutionContext
    ) -> ToolResult:
        validated = _validate_terminal_command(call, required={"columns", "rows"})
        if isinstance(validated, ToolResult):
            return validated
        process_id, cursor, size = validated
        cancellation.raise_if_cancelled()
        try:
            return _result(
                call,
                await self._manager.resize(
                    process_id,
                    context=context,
                    columns=size[0],
                    rows=size[1],
                    cursor=cursor,
                ),
            )
        except ProcessError as error:
            return ToolResult.rejected(call, code=error.code, message=str(error))


def _validate_terminal_command(
    call: ToolCall, *, required: set[str]
) -> tuple[str, int, tuple[int, int]] | ToolResult:
    mismatch = require_exact_arguments(
        call.arguments,
        required={"processId", *required},
        optional={"cursor"},
    )
    if mismatch is not None:
        return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)
    process_id = call.arguments["processId"]
    cursor = call.arguments.get("cursor", 0)
    if (
        not isinstance(process_id, str)
        or not process_id
        or len(process_id) > 128
        or not is_json_integer(cursor)
        or cursor < 0
    ):
        return ToolResult.rejected(
            call, code="invalid_arguments", message="Invalid processId or cursor."
        )
    columns = call.arguments.get("columns", PTY_DEFAULT_COLUMNS)
    rows = call.arguments.get("rows", PTY_DEFAULT_ROWS)
    if ("columns" in call.arguments or "rows" in call.arguments) and (
        not is_json_integer(columns)
        or not PTY_MIN_CELLS <= columns <= PTY_MAX_CELLS
        or not is_json_integer(rows)
        or not PTY_MIN_CELLS <= rows <= PTY_MAX_CELLS
    ):
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message=(
                    f"columns and rows must be integers between {PTY_MIN_CELLS} "
                    f"and {PTY_MAX_CELLS}."
                ),
            )
    return process_id, cursor, (columns, rows)


def _validate_reference(call: ToolCall, *, wait: bool = False) -> tuple[str, int, int] | ToolResult:
    optional = {"cursor", "timeoutMs"} if wait else {"cursor"}
    mismatch = require_exact_arguments(call.arguments, required={"processId"}, optional=optional)
    if mismatch is not None:
        return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)
    process_id = call.arguments["processId"]
    cursor = call.arguments.get("cursor", 0)
    timeout_ms = call.arguments.get("timeoutMs", 1000)
    if (
        not isinstance(process_id, str)
        or not process_id
        or len(process_id) > 128
        or not is_json_integer(cursor)
        or cursor < 0
        or not is_json_integer(timeout_ms)
        or not 1 <= timeout_ms <= _MAX_WAIT_MS
    ):
        return ToolResult.rejected(
            call, code="invalid_arguments", message="Invalid processId, cursor, or timeoutMs."
        )
    return process_id, cursor, timeout_ms


def _result(call: ToolCall, fact: JsonObject) -> ToolResult:
    details = dict(fact)
    output = str(details.pop("output"))
    # The model-facing page is the only command-output body. Full stream copies
    # remain Runtime facts for projection and Desktop rendering.
    details.pop("stdout", None)
    details.pop("stderr", None)
    state = details["state"]
    ok = state == "running" or (state == "exited" and details["exitCode"] == 0)
    if call.name == "process_stop" and state == "terminated":
        ok = True
    if details.get("errorCode") == "protected_output":
        ok = False
    return ToolResult(
        tool_call_id=call.id,
        tool_name=call.name,
        ok=ok,
        output=output,
        details=details,
    )


def _resolve_cwd(value: str | None, default_cwd: str | None) -> Path:
    path = Path(value).expanduser() if value is not None else Path.cwd()
    if not path.is_absolute() and default_cwd is not None:
        path = Path(default_cwd) / path
    resolved = path.resolve(strict=True)
    if not resolved.is_dir():
        raise ValueError("cwd is not a directory")
    return resolved
