"""Built-in ``grep`` Tool: bounded regular-expression content search."""

from __future__ import annotations

import asyncio
import codecs
import re
from dataclasses import dataclass, field
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
    looks_binary,
    resolve_workspace_directory,
    validate_directory_path,
)
from .traversal import (
    GlobPattern,
    compile_glob_pattern,
    compile_search_regex,
    walk_files,
)

_DEFAULT_MAX_MATCHES = 100
_MAX_MATCHES = 1_000
_MAX_CONTEXT_LINES = 200
_MAX_LINE_LENGTH = 2_000
_MAX_FILE_BYTES = 32 * 1024 * 1024
_SAMPLE_BYTES = 4_096
_LINE_TRUNCATION_SUFFIX = f"... (line truncated to {_MAX_LINE_LENGTH} chars)"


@dataclass(frozen=True, slots=True)
class _GrepDisplay:
    output: str
    details: JsonObject


@dataclass(slots=True)
class _GrepState:
    lines: list[str] = field(default_factory=list)
    matches: int = 0
    line_truncations: int = 0
    emitted_line: int = 0


class GrepTool:
    definition = ToolDefinition(
        name="grep",
        description=(
            "Search file contents with a regular expression. Match lines are rendered as "
            "'<path>:<line>:<text>' and context lines as '<path>-<line>-<text>'. Each rendered "
            "line is truncated to 2000 characters. Binary files and files above the size limit "
            "are skipped."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Regular expression to search for; literal text works as-is.",
                },
                "path": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Directory to search (defaults to the Thread workspace).",
                },
                "include": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Glob filter for file names or relative paths, e.g. '*.py'.",
                },
                "ignoreCase": {
                    "type": "boolean",
                    "description": "Match case-insensitively (defaults to false).",
                },
                "contextLines": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": _MAX_CONTEXT_LINES,
                    "description": "Context lines before and after each match (defaults to 0).",
                },
                "maxMatches": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": _MAX_MATCHES,
                    "description": (
                        f"Maximum matches to return (defaults to {_DEFAULT_MAX_MATCHES})."
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
            regex, root, include, context_lines, max_matches = validated
            cancellation.raise_if_cancelled()
            display = await asyncio.to_thread(
                _collect,
                regex,
                root,
                include,
                context_lines,
                max_matches,
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
) -> tuple[re.Pattern[str], Path, GlobPattern | None, int, int] | ToolResult:
    mismatch = require_exact_arguments(
        call.arguments,
        required={"pattern"},
        optional={"path", "include", "ignoreCase", "contextLines", "maxMatches"},
    )
    if mismatch is not None:
        return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)

    ignore_case = call.arguments.get("ignoreCase", False)
    if not isinstance(ignore_case, bool):
        return ToolResult.rejected(
            call,
            code="invalid_arguments",
            message="ignoreCase must be a boolean",
        )
    context_lines = call.arguments.get("contextLines", 0)
    if not is_json_integer(context_lines) or not 0 <= context_lines <= _MAX_CONTEXT_LINES:
        return ToolResult.rejected(
            call,
            code="invalid_arguments",
            message=f"contextLines must be an integer between 0 and {_MAX_CONTEXT_LINES}",
        )
    max_matches = call.arguments.get("maxMatches", _DEFAULT_MAX_MATCHES)
    if not is_json_integer(max_matches) or not 1 <= max_matches <= _MAX_MATCHES:
        return ToolResult.rejected(
            call,
            code="invalid_arguments",
            message=f"maxMatches must be an integer between 1 and {_MAX_MATCHES}",
        )

    regex = compile_search_regex(
        call.arguments["pattern"],
        argument_name="pattern",
        ignore_case=ignore_case,
    )
    include_argument = call.arguments.get("include")
    include = (
        None
        if include_argument is None
        else compile_glob_pattern(include_argument, argument_name="include")
    )
    root_argument = call.arguments.get("path")
    root = (
        resolve_workspace_directory(default_cwd=default_cwd)
        if root_argument is None
        else validate_directory_path(root_argument, default_cwd=default_cwd)
    )
    return regex, root, include, context_lines, max_matches


def _collect(
    regex: re.Pattern[str],
    root: Path,
    include: GlobPattern | None,
    context_lines: int,
    max_matches: int,
    cancellation: CancellationToken,
) -> _GrepDisplay:
    state = _GrepState()
    scanned_files = 0
    truncated = False
    for entry in walk_files(root, cancellation=cancellation):
        if include is not None and not include.matches(entry.relative_path):
            continue
        if entry.size > _MAX_FILE_BYTES or is_known_binary_path(entry.path):
            continue
        text = _read_text(entry.path)
        if text is None:
            continue
        scanned_files += 1
        remaining = max_matches - state.matches
        matched, truncated = _search_text(
            entry.path,
            text,
            regex=regex,
            context_lines=context_lines,
            remaining=remaining,
            state=state,
        )
        state.matches += matched
        if truncated:
            break

    if truncated:
        status = (
            f"({state.matches} matches shown; match limit {max_matches} reached under {root})"
        )
    elif state.matches:
        status = f"({state.matches} matches in {scanned_files} files under {root})"
    else:
        status = f"(no matches for pattern {regex.pattern} under {root})"
    output = "\n".join([*state.lines, "", status]) if state.lines else status
    return _GrepDisplay(
        output=output,
        details={
            "path": str(root),
            "pattern": regex.pattern,
            "matchCount": state.matches,
            "scannedFiles": scanned_files,
            "maxMatches": max_matches,
            "lineTruncations": state.line_truncations,
            "truncated": truncated,
        },
    )


def _search_text(
    path: Path,
    text: str,
    *,
    regex: re.Pattern[str],
    context_lines: int,
    remaining: int,
    state: _GrepState,
) -> tuple[int, bool]:
    """Append rendered matches for one file and report (matches, limit reached)."""

    source_lines = text.splitlines()
    matches = 0
    for number, line in enumerate(source_lines, start=1):
        if regex.search(line) is None:
            continue
        if matches >= remaining:
            return matches, True
        matches += 1
        first_context = max(1, number - context_lines)
        for context_number in range(max(first_context, state.emitted_line + 1), number):
            state.lines.append(
                _render_line(
                    path,
                    context_number,
                    source_lines[context_number - 1],
                    is_match=False,
                    state=state,
                )
            )
        state.lines.append(_render_line(path, number, line, is_match=True, state=state))
        state.emitted_line = number
        last_context = min(len(source_lines), number + context_lines)
        for context_number in range(number + 1, last_context + 1):
            state.lines.append(
                _render_line(
                    path,
                    context_number,
                    source_lines[context_number - 1],
                    is_match=False,
                    state=state,
                )
            )
        state.emitted_line = last_context
    return matches, False


def _render_line(
    path: Path,
    number: int,
    line: str,
    *,
    is_match: bool,
    state: _GrepState,
) -> str:
    if len(line) > _MAX_LINE_LENGTH:
        line = line[:_MAX_LINE_LENGTH] + _LINE_TRUNCATION_SUFFIX
        state.line_truncations += 1
    separator = ":" if is_match else "-"
    return f"{path}{separator}{number}{separator}{line}"


def _read_text(path: Path) -> str | None:
    """Return decodable UTF-8 text, or ``None`` for binary/unreadable/oversized files."""

    try:
        with path.open("rb") as stream:
            data = stream.read(_MAX_FILE_BYTES + 1)
    except OSError:
        return None
    if len(data) > _MAX_FILE_BYTES:
        return None
    body = data[len(codecs.BOM_UTF8) :] if data.startswith(codecs.BOM_UTF8) else data
    if looks_binary(body[:_SAMPLE_BYTES]):
        return None
    try:
        return body.decode("utf-8")
    except UnicodeDecodeError:
        return None


def _error_result(call: ToolCall, error: FileToolError) -> ToolResult:
    return ToolResult(
        tool_call_id=call.id,
        tool_name=call.name,
        ok=False,
        output=str(error),
        details={"truncated": False, "errorCode": error.code},
    )
