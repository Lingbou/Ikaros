"""Bounded file snapshots and historical text patches, separate from model input."""

from __future__ import annotations

import codecs
import difflib
import hashlib
import os
import re
import stat
from collections.abc import Callable, Sequence
from dataclasses import dataclass, replace
from pathlib import Path

from .domain import JsonObject
from .json_codec import dumps as json_dumps
from .json_codec import loads as json_loads

MAX_CAPTURE_BYTES = 256 * 1024
MAX_CAPTURE_LINES = 5_000
MAX_CHANGE_EVENT_BYTES = 256 * 1024
_REASONS = frozenset(
    {
        "not_recorded",
        "too_large",
        "binary_file",
        "unsupported_encoding",
        "read_failed",
        "protected_content",
        "result_unknown",
    }
)
_METADATA_KEYS = {"exists", "byteCount", "revision", "encoding", "bom", "newline", "lineCount"}
_COMMON_KEYS = {
    "threadId",
    "toolCallItemId",
    "path",
    "operation",
    "recordedAt",
    "before",
    "after",
    "status",
}


@dataclass(frozen=True, slots=True)
class FileRevisionCapture:
    metadata: JsonObject
    raw: bytes | None = None
    text: str | None = None
    reason: str | None = None


@dataclass(frozen=True, slots=True)
class FileChangeCapture:
    path: str | None
    operation: str
    before: FileRevisionCapture | None
    after: FileRevisionCapture | None
    diff: str = ""
    additions: int = 0
    deletions: int = 0
    reason: str | None = None

    def protected(self, values: Sequence[str]) -> FileChangeCapture:
        dynamic = (
            self.path,
            self.diff,
            self.before.text if self.before else None,
            self.after.text if self.after else None,
        )
        if not any(
            value and any(value in text for text in dynamic if text is not None) for value in values
        ):
            return self
        path = self.path
        if path is not None and any(value and value in path for value in values):
            path = None
        return FileChangeCapture(path, self.operation, None, None, reason="protected_content")

    def to_record(self, *, thread_id: str, tool_call_item_id: str, recorded_at: str) -> JsonObject:
        record: JsonObject = {
            "threadId": thread_id,
            "toolCallItemId": tool_call_item_id,
            "path": self.path,
            "operation": self.operation,
            "recordedAt": recorded_at,
            "before": self.before.metadata if self.before else None,
            "after": self.after.metadata if self.after else None,
            "status": "unavailable" if self.reason is not None else "recorded",
        }
        if self.reason is not None:
            record["reason"] = self.reason
        else:
            record.update(diff=self.diff, additions=self.additions, deletions=self.deletions)
        return validate_file_change_record(record)


def _unknown_metadata(*, exists: bool, size: int | None) -> JsonObject:
    return {
        "exists": exists,
        "byteCount": size,
        "revision": None,
        "encoding": None,
        "bom": None,
        "newline": None,
        "lineCount": None if exists else 0,
    }


def capture_bytes(raw: bytes, *, binary_path: bool = False) -> FileRevisionCapture:
    from .tools.file_common import looks_binary

    metadata = _unknown_metadata(exists=True, size=len(raw))
    if len(raw) > MAX_CAPTURE_BYTES:
        return FileRevisionCapture(metadata, reason="too_large")
    has_bom = raw.startswith(codecs.BOM_UTF8)
    body = raw[len(codecs.BOM_UTF8) :] if has_bom else raw
    if binary_path or looks_binary(body):
        return FileRevisionCapture(metadata, reason="binary_file")
    try:
        text = body.decode("utf-8")
    except UnicodeDecodeError:
        return FileRevisionCapture(metadata, reason="unsupported_encoding")
    lines = _text_lines(text)
    if len(lines) > MAX_CAPTURE_LINES:
        return FileRevisionCapture(metadata, reason="too_large")
    crlf = "\r\n" in text
    remaining = text.replace("\r\n", "")
    newline = (
        "mixed"
        if "\r" in remaining or (crlf and "\n" in remaining)
        else "crlf"
        if crlf
        else "lf"
        if "\n" in text
        else None
    )
    metadata.update(
        revision=hashlib.sha256(raw).hexdigest(),
        encoding="utf-8",
        bom=has_bom,
        newline=newline,
        lineCount=len(lines),
    )
    return FileRevisionCapture(metadata, raw, text)


def capture_before(path: Path) -> FileRevisionCapture:
    from .tools.file_common import is_known_binary_path

    try:
        with path.open("rb") as stream:
            before = os.fstat(stream.fileno())
            if not stat.S_ISREG(before.st_mode):
                return FileRevisionCapture(
                    _unknown_metadata(exists=True, size=None), reason="read_failed"
                )
            if before.st_size > MAX_CAPTURE_BYTES:
                return FileRevisionCapture(
                    _unknown_metadata(exists=True, size=before.st_size), reason="too_large"
                )
            raw = stream.read(MAX_CAPTURE_BYTES + 1)
            after = os.fstat(stream.fileno())
        if (
            before.st_dev,
            before.st_ino,
            before.st_size,
            before.st_mtime_ns,
            before.st_ctime_ns,
        ) != (
            after.st_dev,
            after.st_ino,
            after.st_size,
            after.st_mtime_ns,
            after.st_ctime_ns,
        ):
            return FileRevisionCapture(
                _unknown_metadata(exists=True, size=after.st_size), reason="result_unknown"
            )
        return capture_bytes(raw, binary_path=is_known_binary_path(path))
    except FileNotFoundError:
        return FileRevisionCapture(_unknown_metadata(exists=False, size=0), text="")
    except OSError:
        return FileRevisionCapture(_unknown_metadata(exists=True, size=None), reason="read_failed")


def build_file_change(
    path: Path,
    operation: str,
    before: FileRevisionCapture,
    payload: bytes,
) -> FileChangeCapture:
    from .tools.file_common import is_known_binary_path

    after = capture_bytes(payload, binary_path=is_known_binary_path(path))
    reason = before.reason or after.reason
    if reason is not None:
        return FileChangeCapture(str(path), operation, before, after, reason=reason)
    old = _text_lines(before.text or "")
    new = _text_lines(after.text or "")
    rows: list[str] = []
    additions = deletions = 0
    for index, line in enumerate(
        difflib.unified_diff(old, new, fromfile="before", tofile="after", n=3)
    ):
        if index > 1 and line.startswith("+"):
            additions += 1
        elif index > 1 and line.startswith("-"):
            deletions += 1
        rows.append(line if line.endswith("\n") else line + "\n\\ No newline at end of file\n")
    return FileChangeCapture(
        str(path), operation, before, after, "".join(rows), additions, deletions
    )


def _text_lines(text: str) -> list[str]:
    normalized = text.replace("\r\n", "\n").replace("\r", "\n")
    # Only CR/LF terminate logical file lines; Python splitlines also splits
    # Unicode separators and control characters that are ordinary file content.
    parts = normalized.split("\n")
    return [part + "\n" for part in parts[:-1]] + ([parts[-1]] if parts[-1] else [])


def perform_captured_write(
    writer: Callable[..., bool],
    path: Path,
    payload: bytes,
    *,
    operation: str,
    expected: bytes | None = None,
) -> tuple[bool, FileChangeCapture]:
    before = capture_before(path)
    options: dict[str, object] = {}
    if expected is not None:
        options["expected"] = expected
        if before.raw is not None and before.raw != expected:
            before = replace(before, reason="result_unknown")
    elif before.raw is not None:
        options["expected"] = before.raw
    elif before.metadata["exists"] is False:
        options["expected_missing"] = True
    verified = writer(path, payload, **options)
    return verified, build_file_change(path, operation, before, payload)


def validate_file_change_record(value: object, *, allow_not_recorded: bool = False) -> JsonObject:
    if not isinstance(value, dict):
        raise ValueError("file change record is not an object")
    recorded = value.get("status") == "recorded"
    expected = _COMMON_KEYS | ({"diff", "additions", "deletions"} if recorded else {"reason"})
    if set(value) != expected or value.get("status") not in {"recorded", "unavailable"}:
        raise ValueError("file change record shape is invalid")
    for name in ("threadId", "toolCallItemId"):
        if not isinstance(value[name], str) or not value[name] or len(value[name]) > 200:
            raise ValueError("file change identity is invalid")
    if value["operation"] not in {"write", "edit"}:
        raise ValueError("file change operation is invalid")
    if value["path"] is not None and (not isinstance(value["path"], str) or not value["path"]):
        raise ValueError("file change path is invalid")
    timestamp = value["recordedAt"]
    if timestamp is not None and (not isinstance(timestamp, str) or not timestamp):
        raise ValueError("file change timestamp is invalid")
    for name in ("before", "after"):
        if value[name] is not None:
            _validate_metadata(value[name])
    if recorded:
        if (
            value["path"] is None
            or timestamp is None
            or value["before"] is None
            or value["after"] is None
        ):
            raise ValueError("recorded file change is incomplete")
        if not isinstance(value["diff"], str) or any(
            type(value[name]) is not int or value[name] < 0 for name in ("additions", "deletions")
        ):
            raise ValueError("file change diff is invalid")
    else:
        reason = value["reason"]
        if reason not in _REASONS or (reason == "not_recorded" and not allow_not_recorded):
            raise ValueError("file change reason is invalid")
        if reason != "not_recorded" and timestamp is None:
            raise ValueError("persisted file change requires a timestamp")
    return json_loads(json_dumps(value, ensure_ascii=False))  # type: ignore[no-any-return]


def _validate_metadata(value: object) -> None:
    if not isinstance(value, dict) or set(value) != _METADATA_KEYS:
        raise ValueError("file revision metadata is invalid")
    if type(value["exists"]) is not bool:
        raise ValueError("file revision existence is invalid")
    for name in ("byteCount", "lineCount"):
        if value[name] is not None and (type(value[name]) is not int or value[name] < 0):
            raise ValueError("file revision size is invalid")
    revision = value["revision"]
    if revision is not None and (
        not isinstance(revision, str) or re.fullmatch(r"[a-f0-9]{64}", revision) is None
    ):
        raise ValueError("file revision digest is invalid")
    if value["encoding"] not in {None, "utf-8"} or value["newline"] not in {
        None,
        "lf",
        "crlf",
        "mixed",
    }:
        raise ValueError("file revision format is invalid")
    if value["bom"] is not None and type(value["bom"]) is not bool:
        raise ValueError("file revision BOM is invalid")
    if value["exists"] is False and value != _unknown_metadata(exists=False, size=0):
        raise ValueError("missing file metadata is inconsistent")
