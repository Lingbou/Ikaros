"""Bounded, deterministic directory traversal shared by the search Tools."""

from __future__ import annotations

import os
import re
import stat
from collections import deque
from collections.abc import Iterator
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path, PurePosixPath

from ..cancellation import CancellationToken
from .file_common import FileToolError

DEFAULT_IGNORED_DIRECTORIES: frozenset[str] = frozenset({".git", "node_modules"})
MAX_SCAN_DEPTH = 12
MAX_PATTERN_LENGTH = 1_024
_CANCELLATION_CHECK_INTERVAL = 64


@dataclass(frozen=True, slots=True)
class WalkedFile:
    """One regular file discovered below a search root."""

    path: Path
    relative_path: str
    size: int
    modified_at: str


@dataclass(frozen=True, slots=True)
class GlobPattern:
    """One validated glob pattern matched against posix-style relative paths."""

    source: str
    regex: re.Pattern[str]
    basename_only: bool

    def matches(self, relative_path: str) -> bool:
        candidate = PurePosixPath(relative_path).name if self.basename_only else relative_path
        return self.regex.fullmatch(candidate) is not None


def compile_glob_pattern(value: object, *, argument_name: str) -> GlobPattern:
    """Validate one glob pattern argument and compile it for relative-path matching."""

    if not isinstance(value, str) or not value.strip():
        raise FileToolError(
            "invalid_arguments",
            f"{argument_name} must be a non-empty glob pattern",
        )
    pattern = value.strip().replace("\\", "/")
    if len(pattern) > MAX_PATTERN_LENGTH or has_control_characters(pattern):
        raise FileToolError(
            "invalid_arguments",
            f"{argument_name} must be at most {MAX_PATTERN_LENGTH} printable characters",
        )
    if pattern.startswith("/"):
        raise FileToolError(
            "invalid_pattern",
            f"{argument_name} must be relative to the search root",
        )
    try:
        regex = re.compile(_glob_to_regex(pattern))
    except re.error as error:
        raise FileToolError(
            "invalid_pattern",
            f"{argument_name} is not a valid glob pattern: {error}",
        ) from None
    return GlobPattern(source=pattern, regex=regex, basename_only="/" not in pattern)


def compile_search_regex(
    value: object,
    *,
    argument_name: str,
    ignore_case: bool,
) -> re.Pattern[str]:
    """Validate one regular-expression argument used by ``grep``."""

    if not isinstance(value, str) or not value.strip():
        raise FileToolError(
            "invalid_arguments",
            f"{argument_name} must be a non-empty regular expression",
        )
    if len(value) > MAX_PATTERN_LENGTH or has_control_characters(value):
        raise FileToolError(
            "invalid_arguments",
            f"{argument_name} must be at most {MAX_PATTERN_LENGTH} printable characters",
        )
    flags = re.IGNORECASE if ignore_case else 0
    try:
        return re.compile(value, flags)
    except re.error as error:
        raise FileToolError(
            "invalid_pattern",
            f"{argument_name} is not a valid regular expression: {error}",
        ) from None


def has_control_characters(value: str) -> bool:
    return any(ord(character) < 32 or ord(character) == 127 for character in value)


def format_timestamp(seconds: float) -> str:
    """Render one file mtime as a stable UTC ISO-8601 string."""

    try:
        moment = datetime.fromtimestamp(seconds, tz=UTC)
    except (OSError, OverflowError, ValueError):
        return "-"
    return moment.isoformat(timespec="seconds").replace("+00:00", "Z")


def walk_files(
    root: Path,
    *,
    cancellation: CancellationToken,
    max_depth: int = MAX_SCAN_DEPTH,
    ignored_directories: frozenset[str] = DEFAULT_IGNORED_DIRECTORIES,
) -> Iterator[WalkedFile]:
    """Yield regular files below ``root`` breadth-first without following directory links.

    Directory links are never followed, unreadable entries are skipped, and descent stops
    at ``max_depth`` levels below the root so that a deep tree cannot stall one Tool call.
    """

    pending: deque[tuple[Path, int]] = deque([(root, 1)])
    scanned = 0
    while pending:
        directory, depth = pending.popleft()
        try:
            with os.scandir(directory) as iterator:
                entries = sorted(iterator, key=lambda entry: entry.name)
        except OSError:
            continue
        for entry in entries:
            scanned += 1
            if scanned % _CANCELLATION_CHECK_INTERVAL == 0:
                cancellation.raise_if_cancelled()
            try:
                if entry.is_symlink():
                    if entry.is_dir(follow_symlinks=True):
                        continue
                elif entry.is_dir(follow_symlinks=False):
                    if depth < max_depth and entry.name not in ignored_directories:
                        pending.append((Path(entry.path), depth + 1))
                    continue
                metadata = entry.stat(follow_symlinks=True)
            except OSError:
                continue
            if not stat.S_ISREG(metadata.st_mode):
                continue
            path = Path(entry.path)
            yield WalkedFile(
                path=path,
                relative_path=relative_path_of(root, path),
                size=metadata.st_size,
                modified_at=format_timestamp(metadata.st_mtime),
            )


def relative_path_of(root: Path, path: Path) -> str:
    try:
        return path.relative_to(root).as_posix()
    except ValueError:
        return path.name


def _glob_to_regex(pattern: str) -> str:
    parts: list[str] = []
    index = 0
    while index < len(pattern):
        character = pattern[index]
        if character == "*":
            if pattern.startswith("**", index):
                index += 2
                if pattern.startswith("/", index):
                    index += 1
                    parts.append("(?:[^/]*/)*")
                else:
                    parts.append(".*")
            else:
                index += 1
                parts.append("[^/]*")
        elif character == "?":
            index += 1
            parts.append("[^/]")
        elif character == "[":
            end = _character_class_end(pattern, index)
            if end is None:
                index += 1
                parts.append(re.escape(character))
            else:
                body = pattern[index + 1 : end]
                if body.startswith("!"):
                    body = "^" + body[1:]
                parts.append("[" + body.replace("\\", "\\\\") + "]")
                index = end + 1
        else:
            parts.append(re.escape(character))
            index += 1
    return "".join(parts)


def _character_class_end(pattern: str, start: int) -> int | None:
    index = start + 1
    if pattern.startswith("!", index) or pattern.startswith("^", index):
        index += 1
    if pattern.startswith("]", index):
        index += 1
    while index < len(pattern):
        if pattern[index] == "]":
            return index
        index += 1
    return None
