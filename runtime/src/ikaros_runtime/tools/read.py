"""Built-in text-file reader, modeled after OpenCode's bounded read Tool."""

from __future__ import annotations

import asyncio
import codecs
from dataclasses import dataclass
from pathlib import Path

from ..cancellation import CancellationToken
from ..domain import JsonObject
from .core import (
    ToolCall,
    ToolDefinition,
    ToolResult,
    is_json_integer,
    require_exact_arguments,
)
from .file_common import (
    FileToolError,
    is_known_binary_path,
    looks_binary,
    path_lock,
    regular_file_size,
    validate_file_path,
)

_DEFAULT_LIMIT = 2_000
_MAX_LINE_LENGTH = 2_000
_MAX_BYTES = 50 * 1024
_READ_CHUNK_BYTES = 64 * 1024
_LINE_TRUNCATION_SUFFIX = f"... (line truncated to {_MAX_LINE_LENGTH} chars)"


@dataclass(frozen=True, slots=True)
class _ReadDisplay:
    output: str
    details: JsonObject


@dataclass(slots=True)
class _ReadState:
    offset: int
    limit: int
    lines: list[str]
    retained_bytes: int = 0
    line_number: int = 1
    line_truncations: int = 0
    byte_capped: bool = False
    next_line: int | None = None

    def append(self, line: str, *, was_truncated: bool = False) -> bool:
        if self.line_number < self.offset:
            self.line_number += 1
            return True
        if len(self.lines) >= self.limit:
            self.next_line = self.line_number
            return False
        if was_truncated or len(line) > _MAX_LINE_LENGTH:
            line = line[:_MAX_LINE_LENGTH] + _LINE_TRUNCATION_SUFFIX
            self.line_truncations += 1
        rendered = f"{self.line_number}: {line}"
        size = len(rendered.encode("utf-8")) + (1 if self.lines else 0)
        if self.retained_bytes + size > _MAX_BYTES:
            self.byte_capped = True
            self.next_line = self.line_number
            return False
        self.lines.append(rendered)
        self.retained_bytes += size
        self.line_number += 1
        return True


class ReadTool:
    definition = ToolDefinition(
        name="read",
        description=(
            "Read a UTF-8 text file with line numbers. Paths may be absolute or relative "
            "to the current Thread workspace. Binary files are not supported."
        ),
        input_schema={
            "type": "object",
            "properties": {
                "filePath": {"type": "string", "minLength": 1},
                "offset": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "First line to read (1-based).",
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": _DEFAULT_LIMIT,
                    "description": "Maximum lines to return (defaults to 2000).",
                },
            },
            "required": ["filePath"],
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
        path, offset, limit = validated
        cancellation.raise_if_cancelled()
        try:
            async with path_lock(path):
                cancellation.raise_if_cancelled()
                display = await asyncio.to_thread(
                    _read_display,
                    path,
                    offset,
                    limit,
                    cancellation,
                )
                cancellation.raise_if_cancelled()
        except FileToolError as error:
            return _error_result(call, path, error)
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
) -> tuple[Path, int, int] | ToolResult:
    mismatch = require_exact_arguments(
        call.arguments,
        required={"filePath"},
        optional={"offset", "limit"},
    )
    if mismatch is not None:
        return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)
    offset = call.arguments.get("offset", 1)
    limit = call.arguments.get("limit", _DEFAULT_LIMIT)
    if not is_json_integer(offset) or offset < 1:
        return ToolResult.rejected(
            call,
            code="invalid_arguments",
            message="offset must be a positive 1-based integer",
        )
    if not is_json_integer(limit) or not 1 <= limit <= _DEFAULT_LIMIT:
        return ToolResult.rejected(
            call,
            code="invalid_arguments",
            message=f"limit must be an integer between 1 and {_DEFAULT_LIMIT}",
        )
    try:
        path = validate_file_path(call.arguments["filePath"], default_cwd=default_cwd)
    except FileToolError as error:
        return _error_result(call, None, error)
    return path, offset, limit


def _read_display(
    path: Path,
    offset: int,
    limit: int,
    cancellation: CancellationToken,
) -> _ReadDisplay:
    regular_file_size(path)
    if is_known_binary_path(path):
        raise FileToolError("binary_file", f"Cannot read binary file: {path}")

    state = _ReadState(offset=offset, limit=limit, lines=[])
    decoder = codecs.getincrementaldecoder("utf-8")(errors="strict")
    pending = ""
    discard = False
    has_bom = False
    bytes_read = 0
    first_chunk = True
    stopped_early = False

    try:
        with path.open("rb") as stream:
            while True:
                cancellation.raise_if_cancelled()
                chunk = stream.read(_READ_CHUNK_BYTES)
                if not chunk:
                    break
                bytes_read += len(chunk)
                if first_chunk:
                    first_chunk = False
                    has_bom = chunk.startswith(codecs.BOM_UTF8)
                    if has_bom:
                        chunk = chunk[len(codecs.BOM_UTF8) :]
                pending, discard, keep_reading = _consume_chunk(
                    state,
                    decoder,
                    path,
                    pending,
                    discard,
                    chunk,
                )
                if not keep_reading:
                    stopped_early = True
                    break
            if not stopped_early:
                try:
                    tail = decoder.decode(b"", final=True)
                except UnicodeDecodeError:
                    raise FileToolError(
                        "unsupported_encoding",
                        f"File is not valid UTF-8 text: {path}",
                    ) from None
                pending, discard, keep_reading = _consume_text(
                    state,
                    pending,
                    discard,
                    tail,
                )
                if keep_reading and (pending or discard):
                    state.append(pending, was_truncated=discard)
    except FileNotFoundError:
        raise FileToolError("file_not_found", f"File not found: {path}") from None
    except FileToolError:
        raise
    except OSError as error:
        raise FileToolError("read_failed", f"Could not read file: {error}") from None

    lines_seen = state.line_number - 1
    if not state.lines and offset != 1:
        raise FileToolError(
            "offset_out_of_range",
            f"Offset {offset} is out of range for this file ({lines_seen} lines)",
        )

    last_line = offset + len(state.lines) - 1
    has_more = state.next_line is not None
    content = "\n".join(state.lines)
    if state.byte_capped:
        status = (
            f"Output capped at 50 KB. Showing lines {offset}-{last_line}. "
            f"Use offset={state.next_line} to continue."
        )
    elif has_more:
        status = (
            f"Showing lines {offset}-{last_line}. "
            f"Use offset={state.next_line} to continue."
        )
    else:
        status = f"End of file - total {lines_seen} lines"

    output = (
        f"<path>{path}</path>\n"
        "<type>file</type>\n"
        "<content>\n"
        f"{content}\n\n"
        f"({status})\n"
        "</content>"
    )
    details: JsonObject = {
        "path": str(path),
        "lineStart": offset,
        "lineEnd": last_line,
        "bytesRead": bytes_read,
        "bom": has_bom,
        "truncated": has_more,
        "lineTruncations": state.line_truncations,
    }
    if not has_more:
        details["totalLines"] = lines_seen
    else:
        details["nextOffset"] = state.next_line
    return _ReadDisplay(output=output, details=details)


def _consume_text(
    state: _ReadState,
    pending: str,
    discard: bool,
    text: str,
) -> tuple[str, bool, bool]:
    while True:
        index = text.find("\n")
        if index == -1:
            if not discard:
                pending += text
                if len(pending) > _MAX_LINE_LENGTH:
                    pending = pending[:_MAX_LINE_LENGTH]
                    discard = True
            return pending, discard, True
        was_truncated = discard
        current = pending + ("" if was_truncated else text[:index])
        pending = ""
        discard = False
        text = text[index + 1 :]
        if current.endswith("\r"):
            current = current[:-1]
        if not state.append(current, was_truncated=was_truncated):
            return pending, discard, False


def _consume_chunk(
    state: _ReadState,
    decoder: codecs.IncrementalDecoder,
    path: Path,
    pending: str,
    discard: bool,
    chunk: bytes,
) -> tuple[str, bool, bool]:
    start = 0
    while start < len(chunk):
        if len(state.lines) >= state.limit:
            state.next_line = state.line_number
            return pending, discard, False
        newline = chunk.find(b"\n", start)
        end = len(chunk) if newline == -1 else newline + 1
        segment = chunk[start:end]
        if looks_binary(segment):
            raise FileToolError("binary_file", f"Cannot read binary file: {path}")
        try:
            text = decoder.decode(segment, final=False)
        except UnicodeDecodeError:
            raise FileToolError(
                "unsupported_encoding",
                f"File is not valid UTF-8 text: {path}",
            ) from None
        pending, discard, keep_reading = _consume_text(
            state,
            pending,
            discard,
            text,
        )
        if not keep_reading:
            return pending, discard, False
        start = end
    return pending, discard, True


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
