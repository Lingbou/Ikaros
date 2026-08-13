"""Built-in atomic text-file writer."""

from __future__ import annotations

import asyncio
from pathlib import Path

from ..cancellation import CancellationToken
from ..domain import JsonObject
from .core import ToolCall, ToolDefinition, ToolResult, require_exact_arguments
from .file_common import (
    FileToolError,
    atomic_write_bytes,
    detect_line_ending,
    encode_text,
    inspect_existing_text_format,
    path_lock,
    require_utf8_text,
    run_mutation_thread,
    validate_file_path,
)


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
                verified = await run_mutation_thread(atomic_write_bytes, path, payload)
                cancellation.raise_if_cancelled()
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
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output=f"{action} file successfully: {path}",
            details=details,
        )


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
