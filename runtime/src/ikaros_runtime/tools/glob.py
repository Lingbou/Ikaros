"""Built-in ``glob`` Tool: pattern-based file discovery without a shell."""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from pathlib import Path

from ..cancellation import CancellationToken
from ..domain import JsonObject
from .core import (
    ToolCall,
    ToolDefinition,
    ToolExecutionContext,
    ToolResult,
    is_json_integer,
    require_exact_arguments,
)
from .file_common import (
    FileToolError,
    is_known_binary_path,
    resolve_workspace_directory,
    validate_directory_path,
)
from .traversal import GlobPattern, WalkedFile, compile_glob_pattern, walk_files

_DEFAULT_MAX_RESULTS = 1_000
_MAX_RESULTS = 5_000


@dataclass(frozen=True, slots=True)
class _GlobDisplay:
    output: str
    details: JsonObject


class GlobTool:
    definition = ToolDefinition(
        name="glob",
        description=(
            "Find files by glob pattern such as '**/*.py'. Each result line is "
            "'<absolute path>\\t<size bytes>\\t<modified UTC>', sorted by path. Directories "
            "named .git or node_modules, known binary files, and directories deeper than the "
            "scan limit are skipped."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Glob pattern relative to the search root, e.g. '**/*.py'.",
                },
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Directory to search (defaults to the Thread workspace).",
                },
                "maxResults": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": _MAX_RESULTS,
                    "description": (
                        f"Maximum matches to return (defaults to {_DEFAULT_MAX_RESULTS})."
                    ),
                },
            },
            "required": ["pattern"],
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
            pattern, root, max_results = validated
            cancellation.raise_if_cancelled()
            display = await asyncio.to_thread(
                _collect,
                pattern,
                root,
                max_results,
                cancellation,
            )
        except FileToolError as error:
            return _error_result(call, error)
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output=display.output,
            details=display.details,
        )


def _validate_arguments(
    call: ToolCall,
    *,
    default_cwd: str | None,
) -> tuple[GlobPattern, Path, int] | ToolResult:
    mismatch = require_exact_arguments(
        call.arguments,
        required={"pattern"},
        optional={"path", "maxResults"},
    )
    if mismatch is not None:
        return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)
    max_results = call.arguments.get("maxResults", _DEFAULT_MAX_RESULTS)
    if not is_json_integer(max_results) or not 1 <= max_results <= _MAX_RESULTS:
        return ToolResult.rejected(
            call,
            code="invalid_arguments",
            message=f"maxResults must be an integer between 1 and {_MAX_RESULTS}",
        )
    pattern = compile_glob_pattern(call.arguments["pattern"], argument_name="pattern")
    root_argument = call.arguments.get("path")
    root = (
        resolve_workspace_directory(default_cwd=default_cwd)
        if root_argument is None
        else validate_directory_path(root_argument, default_cwd=default_cwd)
    )
    return pattern, root, max_results


def _collect(
    pattern: GlobPattern,
    root: Path,
    max_results: int,
    cancellation: CancellationToken,
) -> _GlobDisplay:
    matches: list[WalkedFile] = []
    truncated = False
    for entry in walk_files(root, cancellation=cancellation):
        if is_known_binary_path(entry.path) or not pattern.matches(entry.relative_path):
            continue
        if len(matches) >= max_results:
            truncated = True
            break
        matches.append(entry)
    matches.sort(key=lambda entry: entry.relative_path)

    lines = [f"{entry.path}\t{entry.size}\t{entry.modified_at}" for entry in matches]
    if truncated:
        status = (
            f"({len(matches)} files shown; more matches exist under {root}; "
            "narrow the pattern or raise maxResults)"
        )
    elif matches:
        status = f"({len(matches)} files under {root})"
    else:
        status = f"(no files matched pattern {pattern.source} under {root})"
    output = "\n".join([*lines, "", status]) if lines else status
    return _GlobDisplay(
        output=output,
        details={
            "path": str(root),
            "pattern": pattern.source,
            "entryCount": len(matches),
            "maxResults": max_results,
            "truncated": truncated,
        },
    )


def _error_result(call: ToolCall, error: FileToolError) -> ToolResult:
    return ToolResult(
        tool_call_id=call.id,
        tool_name=call.name,
        ok=False,
        output=str(error),
        details={"truncated": False, "errorCode": error.code},
    )
