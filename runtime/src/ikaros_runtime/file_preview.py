"""Bounded read-only text pages, separate from model-visible file tools."""

from __future__ import annotations

import codecs
import hashlib
import os
import stat
from collections.abc import Iterator, Sequence
from pathlib import Path
from typing import BinaryIO

from .domain import JsonObject
from .errors import ProtectedValueError
from .security import ProtectedStreamGuard
from .tools.file_common import is_known_binary_path, looks_binary

PREVIEW_MAX_BYTES = 50 * 1024
PREVIEW_MAX_LINES = 2_000
PREVIEW_SCAN_BYTES = 8 * 1024 * 1024
_CHUNK_BYTES = 4096


class _Unavailable(Exception):
    def __init__(self, reason: str) -> None:
        self.reason = reason


def unavailable_preview(thread_id: str, path: str | None, reason: str) -> JsonObject:
    return {"threadId": thread_id, "path": path, "status": "unavailable", "reason": reason}


def _revision(metadata: os.stat_result) -> str:
    # This is a version fingerprint, not a digest of the complete file body.
    values = (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )
    return hashlib.sha256(repr(values).encode("ascii")).hexdigest()


def _lines(stream: BinaryIO, size: int) -> Iterator[bytes]:
    """Yield original LF/CRLF/CR lines with a strict total scan and line bound."""
    pending = bytearray()
    scanned = 0
    while True:
        remaining = PREVIEW_SCAN_BYTES - scanned
        if remaining <= 0 and scanned < size:
            raise _Unavailable("scan_limit")
        chunk = stream.read(min(_CHUNK_BYTES, remaining))
        scanned += len(chunk)
        eof = not chunk
        pending.extend(chunk)
        start = 0
        index = 0
        while index < len(pending):
            byte = pending[index]
            if byte not in (10, 13):
                index += 1
                continue
            if byte == 13 and index + 1 == len(pending) and not eof:
                break
            end = index + 1
            if byte == 13 and end < len(pending) and pending[end] == 10:
                end += 1
            line = bytes(pending[start:end])
            if len(line) > PREVIEW_MAX_BYTES + len(codecs.BOM_UTF8):
                raise _Unavailable("too_large")
            yield line
            start = end
            index = end
        del pending[:start]
        if len(pending) > PREVIEW_MAX_BYTES + len(codecs.BOM_UTF8):
            raise _Unavailable("too_large")
        if eof:
            if pending:
                yield bytes(pending)
            return


def preview_text_file(
    *,
    thread_id: str,
    path: Path,
    offset: int = 1,
    expected_revision: str | None = None,
    protected_values: Sequence[str] = (),
) -> JsonObject:
    def unavailable(reason: str) -> JsonObject:
        return unavailable_preview(thread_id, str(path), reason)

    try:
        metadata = path.stat()
        if not stat.S_ISREG(metadata.st_mode):
            return unavailable("not_a_file")
        flags = os.O_RDONLY | getattr(os, "O_BINARY", 0)
        flags |= getattr(os, "O_NONBLOCK", 0) | getattr(os, "O_NOFOLLOW", 0)
        descriptor = os.open(path, flags)
        with os.fdopen(descriptor, "rb") as stream:
            opened = os.fstat(stream.fileno())
            if not stat.S_ISREG(opened.st_mode):
                return unavailable("not_a_file")
            revision = _revision(opened)
            if revision != _revision(metadata) or path.resolve() != path:
                return unavailable("revision_changed")
            if expected_revision is not None and expected_revision != revision:
                return unavailable("revision_changed")
            if is_known_binary_path(path):
                return unavailable("binary_file")
            lines: list[str] = []
            byte_count = 0
            bom = False
            line_number = 0
            truncation: str | None = None
            guard = ProtectedStreamGuard(protected_values)
            try:
                for line_number, raw in enumerate(_lines(stream, opened.st_size), start=1):
                    if line_number == 1 and raw.startswith(codecs.BOM_UTF8):
                        bom = True
                        raw = raw[len(codecs.BOM_UTF8) :]
                    if looks_binary(raw):
                        raise _Unavailable("binary_file")
                    text = raw.decode("utf-8", errors="strict")
                    guard.feed(text)
                    if line_number < offset:
                        continue
                    if len(raw) > PREVIEW_MAX_BYTES:
                        raise _Unavailable("too_large")
                    if len(lines) >= PREVIEW_MAX_LINES:
                        truncation = "line_limit"
                        break
                    if byte_count + len(raw) > PREVIEW_MAX_BYTES:
                        truncation = "byte_limit"
                        break
                    lines.append(text)
                    byte_count += len(raw)
            except _Unavailable as error:
                if error.reason != "scan_limit" or not lines:
                    raise
                truncation = "scan_limit"
            # A possible credential prefix at an unfinished scan boundary cannot
            # be emitted across independently fetched pages.
            pending_secret = guard.finish()
            if truncation is not None and pending_secret:
                raise _Unavailable("protected_content")
            if revision != _revision(os.fstat(stream.fileno())) or (
                revision != _revision(path.stat()) or path.resolve() != path
            ):
                return unavailable("revision_changed")
            content = "".join(lines)
            # A BOM-only file is an empty file, with formatting metadata intact.
            if lines == [""]:
                lines = []
            return {
                "threadId": thread_id,
                "path": str(path),
                "status": "text",
                "revision": revision,
                "encoding": "utf-8",
                "bom": bom,
                "content": content,
                "lineStart": offset,
                "lineEnd": offset + len(lines) - 1,
                "nextOffset": offset + len(lines) if truncation else None,
                "truncated": truncation is not None,
                "truncationReason": truncation,
            }
    except FileNotFoundError:
        return unavailable("file_not_found")
    except UnicodeDecodeError:
        return unavailable("unsupported_encoding")
    except ProtectedValueError:
        return unavailable("protected_content")
    except _Unavailable as error:
        return unavailable(error.reason)
    except (OSError, ValueError, RuntimeError):
        return unavailable("read_failed")
