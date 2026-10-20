"""Built-in ``list`` Tool: one bounded directory level."""

from __future__ import annotations

import asyncio
import os
import stat
from dataclasses import dataclass
from pathlib import Path

from ..cancellation import CancellationToken
from ..domain import JsonObject
from .core import (
    ToolCall,
    ToolDefinition,
    ToolExecutionContext,
    ToolResult,
    require_exact_arguments,
)
from .file_common import (
    FileToolError,
    resolve_workspace_directory,
    validate_directory_path,
)
from .traversal import format_timestamp

_MAX_ENTRIES = 5_000


@dataclass(frozen=True, slots=True)
class _ListDisplay:
    output: str
    details: JsonObject


class ListTool:
    definition = ToolDefinition(
        name="list",
        description=(
            "List one directory level. Each line is '<name>\\t<type>\\t<size bytes>\\t<modified "
            "UTC>' sorted by name, where type is dir, file, symlink, or other. Subdirectories "
            "are not expanded; use glob or grep to search a tree."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Directory to list (defaults to the Thread workspace).",
                }
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
        cancellation.raise_if_cancelled()
        try:
            validated = _validate_arguments(call, default_cwd=context.default_cwd)
            if isinstance(validated, ToolResult):
                return validated
            root = validated
            cancellation.raise_if_cancelled()
            display = await asyncio.to_thread(_collect, root, cancellation)
        except FileToolError as error:
            return _error_result(call, error)
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output=display.output,
            details=display.details,
        )


def _validate_arguments(call: ToolCall, *, default_cwd: str | None) -> Path | ToolResult:
    mismatch = require_exact_arguments(call.arguments, required=set(), optional={"path"})
    if mismatch is not None:
        return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)
    path_argument = call.arguments.get("path")
    if path_argument is None:
        return resolve_workspace_directory(default_cwd=default_cwd)
    return validate_directory_path(path_argument, default_cwd=default_cwd)


def _collect(root: Path, cancellation: CancellationToken) -> _ListDisplay:
    entries: list[os.DirEntry[str]] = []
    truncated = False
    try:
        with os.scandir(root) as iterator:
            for entry in iterator:
                if len(entries) >= _MAX_ENTRIES:
                    truncated = True
                    break
                entries.append(entry)
    except OSError as error:
        raise FileToolError("read_failed", f"Could not list directory: {error}") from None
    cancellation.raise_if_cancelled()
    entries.sort(key=lambda entry: entry.name)
    lines = [_render_entry(entry) for entry in entries]
    if truncated:
        status = f"({len(lines)} entries shown; directory listing was truncated at {_MAX_ENTRIES})"
    else:
        status = f"({len(lines)} entries under {root})"
    output = "\n".join([*lines, "", status]) if lines else status
    return _ListDisplay(
        output=output,
        details={"path": str(root), "entryCount": len(lines), "truncated": truncated},
    )


def _render_entry(entry: os.DirEntry[str]) -> str:
    try:
        if entry.is_symlink():
            kind = "symlink"
        elif entry.is_dir(follow_symlinks=False):
            kind = "dir"
        elif entry.is_file(follow_symlinks=False):
            kind = "file"
        else:
            kind = "other"
    except OSError:
        return f"{entry.name}\tunknown\t-\t-"
    size = "-"
    modified = "-"
    try:
        metadata = entry.stat(follow_symlinks=True)
    except OSError:
        metadata = None
    if metadata is not None:
        if stat.S_ISREG(metadata.st_mode):
            size = str(metadata.st_size)
        modified = format_timestamp(metadata.st_mtime)
    return f"{entry.name}\t{kind}\t{size}\t{modified}"


def _error_result(call: ToolCall, error: FileToolError) -> ToolResult:
    return ToolResult(
        tool_call_id=call.id,
        tool_name=call.name,
        ok=False,
        output=str(error),
        details={"truncated": False, "errorCode": error.code},
    )
