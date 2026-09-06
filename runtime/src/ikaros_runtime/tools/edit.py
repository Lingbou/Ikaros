"""Built-in exact text replacement Tool."""

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
    ToolExecutionContext,
    ToolResult,
    ToolTaskCancelled,
    require_exact_arguments,
)
from .file_common import (
    FileToolError,
    SettledMutationCancelled,
    encode_text,
    normalize_line_endings,
    path_lock,
    require_utf8_text,
    run_mutation_thread,
    validate_file_path,
)
from .file_common import atomic_write_bytes as atomic_write_bytes
from .file_common import read_text_document as read_text_document


class EditTool:
    definition = ToolDefinition(
        name="edit",
        description=(
            "Replace an exact text span in an existing UTF-8 file. The match must be "
            "unique unless replaceAll is true."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "filePath": {"type": "string", "minLength": 1},
                "oldString": {"type": "string", "minLength": 1},
                "newString": {"type": "string"},
                "replaceAll": {"type": "boolean", "default": False},
            },
            "required": ["filePath", "oldString", "newString"],
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
        validated = _validate_arguments(call, default_cwd=context.default_cwd)
        if isinstance(validated, ToolResult):
            return validated
        path, old_string, new_string, replace_all = validated
        task_cancelled = False
        cancellation.raise_if_cancelled()
        try:
            async with path_lock(path):
                cancellation.raise_if_cancelled()
                document = await asyncio.to_thread(read_text_document, path)
                newline = document.newline or "\n"
                normalized_content = normalize_line_endings(document.text)
                old = normalize_line_endings(old_string)
                replacement = normalize_line_endings(new_string)
                if old == replacement:
                    return _error_result(
                        call,
                        path,
                        FileToolError(
                            "no_change",
                            "No changes to apply after normalizing line endings",
                        ),
                    )

                matches = normalized_content.count(old)
                if matches == 0:
                    return _error_result(
                        call,
                        path,
                        FileToolError(
                            "no_match",
                            "Could not find oldString in the file. It must match exactly, "
                            "including whitespace and indentation.",
                        ),
                    )
                if matches > 1 and not replace_all:
                    return _error_result(
                        call,
                        path,
                        FileToolError(
                            "multiple_matches",
                            "Found multiple matches for oldString. Provide more surrounding "
                            "context or set replaceAll to true.",
                        ),
                    )

                replacements = matches if replace_all else 1
                updated = normalized_content.replace(old, replacement, -1 if replace_all else 1)
                payload, has_bom = encode_text(
                    updated,
                    preserve_bom=document.has_bom,
                    newline=newline,
                )
                cancellation.raise_if_cancelled()
                try:
                    verified, capture = await run_mutation_thread(
                        perform_captured_write,
                        atomic_write_bytes,
                        path,
                        payload,
                        operation="edit",
                        expected=document.raw_bytes,
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
                FileToolError("edit_failed", f"Could not edit file: {error}"),
            )

        details: JsonObject = {
            "path": str(path),
            "replacements": replacements,
            "bytesWritten": len(payload),
            "verified": verified,
            "bom": has_bom,
            "newline": "crlf" if document.newline == "\r\n" else "lf",
            "truncated": False,
        }
        noun = "occurrence" if replacements == 1 else "occurrences"
        result = ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output=f"Replaced {replacements} {noun} in {path}",
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
) -> tuple[Path, str, str, bool] | ToolResult:
    mismatch = require_exact_arguments(
        call.arguments,
        required={"filePath", "oldString", "newString"},
        optional={"replaceAll"},
    )
    if mismatch is not None:
        return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)
    old_string = call.arguments["oldString"]
    new_string = call.arguments["newString"]
    replace_all = call.arguments.get("replaceAll", False)
    if not isinstance(old_string, str) or not old_string:
        return ToolResult.rejected(
            call,
            code="invalid_arguments",
            message="oldString must be a non-empty string",
        )
    if not isinstance(new_string, str):
        return ToolResult.rejected(
            call,
            code="invalid_arguments",
            message="newString must be a string",
        )
    for argument_name, value in (("oldString", old_string), ("newString", new_string)):
        if encoding_error := require_utf8_text(value, argument_name=argument_name):
            return _error_result(call, None, encoding_error)
    if not isinstance(replace_all, bool):
        return ToolResult.rejected(
            call,
            code="invalid_arguments",
            message="replaceAll must be a boolean",
        )
    if old_string == new_string:
        return ToolResult.rejected(
            call,
            code="no_change",
            message="No changes to apply: oldString and newString are identical",
        )
    try:
        path = validate_file_path(call.arguments["filePath"], default_cwd=default_cwd)
    except FileToolError as error:
        return _error_result(call, None, error)
    return path, old_string, new_string, replace_all


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
