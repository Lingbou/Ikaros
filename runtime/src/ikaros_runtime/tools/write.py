"""Built-in atomic text-file writer."""

from __future__ import annotations

import asyncio
from dataclasses import replace
from pathlib import Path
from typing import cast

from ..cancellation import CancellationToken
from ..domain import JsonObject
from ..file_changes import FileChangeCapture, perform_captured_write
from .core import (
    ToolCall,
    ToolDefinition,
    ToolExecutionCancelled,
    ToolResult,
    ToolTaskCancelled,
    require_exact_arguments,
)
from .file_common import (
    FileToolError,
    SettledMutationCancelled,
    detect_line_ending,
    encode_text,
    inspect_existing_text_format,
    path_lock,
    require_utf8_text,
    run_mutation_thread,
    validate_file_path,
)
from .file_common import atomic_write_bytes as atomic_write_bytes


class WriteTool:
    definition = ToolDefinition(
        name="write",
        description=(
            "Create or completely overwrite a UTF-8 text file. Parent directories are "
            "created automatically, and replacement is atomic."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "filePath": {"type": "string", "minLength": 1},
                "content": {"type": "string"},
            },
            "required": ["filePath", "content"],
            "additionalProperties": False,
        },
    )

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
        default_cwd: str | None = None,
    ) -> ToolResult:
        validated = _validate_arguments(call, default_cwd=default_cwd)
        if isinstance(validated, ToolResult):
            return validated
        path, content = validated
        task_cancelled = False
        cancellation.raise_if_cancelled()
        try:
            async with path_lock(path):
                cancellation.raise_if_cancelled()
                existing = await asyncio.to_thread(inspect_existing_text_format, path)
                payload, has_bom = encode_text(
                    content,
                    preserve_bom=existing.has_bom,
                    newline=existing.newline,
                )
                cancellation.raise_if_cancelled()
                try:
                    verified, capture = await run_mutation_thread(
                        perform_captured_write,
                        atomic_write_bytes,
                        path,
                        payload,
                        operation="write",
                    )
                except SettledMutationCancelled as error:
                    verified, capture = cast(tuple[bool, FileChangeCapture], error.result)
                    task_cancelled = True
        except FileToolError as error:
            return _error_result(call, path, error)
        except OSError as error:
            return _error_result(
                call,
                path,
                FileToolError("write_failed", f"Could not write file: {error}"),
            )

        newline = existing.newline or detect_line_ending(content.removeprefix("\ufeff"))
        details: JsonObject = {
            "path": str(path),
            "created": not existing.exists,
            "bytesWritten": len(payload),
            "verified": verified,
            "bom": has_bom,
            "newline": "crlf" if newline == "\r\n" else "lf" if newline == "\n" else None,
            "truncated": False,
        }
        action = "Created" if not existing.exists else "Wrote"
        result = ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output=f"{action} file successfully: {path}",
            details=details,
            file_change=capture,
        )
        if task_cancelled:
            raise ToolTaskCancelled(replace(result, cancelled=True))
        if cancellation.is_cancelled:
            raise ToolExecutionCancelled(replace(result, cancelled=True))
        return result


def _validate_arguments(
    call: ToolCall,
    *,
    default_cwd: str | None,
) -> tuple[Path, str] | ToolResult:
    mismatch = require_exact_arguments(
        call.arguments,
        required={"filePath", "content"},
        optional=set(),
    )
    if mismatch is not None:
        return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)
    content = call.arguments["content"]
    if not isinstance(content, str):
        return ToolResult.rejected(
            call,
            code="invalid_arguments",
            message="content must be a string",
        )
    if encoding_error := require_utf8_text(content, argument_name="content"):
        return _error_result(call, None, encoding_error)
    try:
        path = validate_file_path(call.arguments["filePath"], default_cwd=default_cwd)
    except FileToolError as error:
        return _error_result(call, None, error)
    return path, content


def _error_result(call: ToolCall, path: Path | None, error: FileToolError) -> ToolResult:
    details: JsonObject = {
        "path": str(path) if path is not None else None,
        "truncated": False,
        "errorCode": error.code,
    }
    return ToolResult(
        tool_call_id=call.id,
        tool_name=call.name,
        ok=False,
        output=str(error),
        details=details,
    )
