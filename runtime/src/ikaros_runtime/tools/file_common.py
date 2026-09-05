"""Shared path, text, locking, and atomic-write behavior for file Tools."""

from __future__ import annotations

import asyncio
import codecs
import hashlib
import os as os
import secrets
import stat
import tempfile
from collections.abc import AsyncIterator, Callable
from contextlib import asynccontextmanager, suppress
from dataclasses import dataclass
from pathlib import Path

_PATH_LIMIT = 4_096
_SAMPLE_BYTES = 4_096
_UTF8_BOM = codecs.BOM_UTF8
_PATH_LOCKS: dict[str, _PathLockEntry] = {}
_BINARY_EXTENSIONS = {
    ".7z",
    ".a",
    ".bin",
    ".class",
    ".dat",
    ".dll",
    ".doc",
    ".docx",
    ".exe",
    ".gz",
    ".jar",
    ".lib",
    ".o",
    ".obj",
    ".odp",
    ".ods",
    ".odt",
    ".ppt",
    ".pptx",
    ".pyc",
    ".pyo",
    ".so",
    ".tar",
    ".war",
    ".wasm",
    ".xls",
    ".xlsx",
    ".zip",
}


class FileToolError(Exception):
    """A stable file Tool failure that is safe to return to the model."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


class SettledMutationCancelled(asyncio.CancelledError):
    """Task cancellation observed after its mutation produced a known result."""

    def __init__(self, result: object) -> None:
        super().__init__("file mutation settled while the task was cancelled")
        self.result = result


@dataclass(slots=True)
class _PathLockEntry:
    lock: asyncio.Lock
    users: int = 0


@dataclass(frozen=True, slots=True)
class TextDocument:
    text: str
    has_bom: bool
    newline: str | None
    byte_count: int
    raw_bytes: bytes


@dataclass(frozen=True, slots=True)
class ExistingTextFormat:
    exists: bool
    has_bom: bool
    newline: str | None


def validate_file_path(value: object, *, default_cwd: str | None) -> Path:
    if (
        not isinstance(value, str)
        or not value.strip()
        or len(value) > _PATH_LIMIT
        or "\x00" in value
    ):
        raise FileToolError(
            "invalid_arguments",
            f"filePath must be a non-empty string of at most {_PATH_LIMIT} characters",
        )

    try:
        path = Path(value).expanduser()
        if not path.is_absolute():
            base = Path(default_cwd).expanduser() if default_cwd is not None else Path.cwd()
            try:
                base = base.resolve(strict=True)
            except OSError as error:
                raise FileToolError(
                    "invalid_workspace",
                    f"workspace is not accessible: {error}",
                ) from None
            if not base.is_dir():
                raise FileToolError("invalid_workspace", "workspace is not a directory")
            path = base / path
        return path.resolve(strict=False)
    except FileToolError:
        raise
    except (OSError, RuntimeError, ValueError) as error:
        raise FileToolError("invalid_path", f"filePath is not accessible: {error}") from None


@asynccontextmanager
async def path_lock(path: Path) -> AsyncIterator[None]:
    """Serialize one resolved path and discard the lock after its last user."""

    key = os.path.normcase(str(path))
    entry = _PATH_LOCKS.get(key)
    if entry is None:
        entry = _PathLockEntry(asyncio.Lock())
        _PATH_LOCKS[key] = entry
    entry.users += 1
    acquired = False
    try:
        await entry.lock.acquire()
        acquired = True
        yield
    finally:
        if acquired:
            entry.lock.release()
        entry.users -= 1
        if entry.users == 0 and _PATH_LOCKS.get(key) is entry:
            del _PATH_LOCKS[key]


def read_text_document(path: Path) -> TextDocument:
    data = _read_regular_file(path)
    has_bom = data.startswith(_UTF8_BOM)
    body = data[len(_UTF8_BOM) :] if has_bom else data
    if is_known_binary_path(path) or looks_binary(body):
        raise FileToolError("binary_file", f"Cannot read binary file: {path}")
    try:
        text = body.decode("utf-8")
    except UnicodeDecodeError:
        raise FileToolError(
            "unsupported_encoding",
            f"File is not valid UTF-8 text: {path}",
        ) from None
    return TextDocument(
        text=text,
        has_bom=has_bom,
        newline=detect_line_ending(text),
        byte_count=len(data),
        raw_bytes=data,
    )


def inspect_existing_text_format(path: Path) -> ExistingTextFormat:
    try:
        metadata = path.stat()
    except FileNotFoundError:
        return ExistingTextFormat(exists=False, has_bom=False, newline=None)
    except OSError as error:
        raise FileToolError("write_failed", f"Could not inspect file: {error}") from None

    if stat.S_ISDIR(metadata.st_mode):
        raise FileToolError("path_is_directory", f"Path is a directory, not a file: {path}")
    if not stat.S_ISREG(metadata.st_mode):
        raise FileToolError("not_regular_file", f"Path is not a regular file: {path}")

    try:
        with path.open("rb") as stream:
            sample = stream.read(64 * 1024)
    except OSError as error:
        raise FileToolError("write_failed", f"Could not inspect file: {error}") from None

    has_bom = sample.startswith(_UTF8_BOM)
    body = sample[len(_UTF8_BOM) :] if has_bom else sample
    if looks_binary(body[:_SAMPLE_BYTES]):
        return ExistingTextFormat(exists=True, has_bom=has_bom, newline=None)
    text = body.decode("utf-8", errors="ignore")
    return ExistingTextFormat(
        exists=True,
        has_bom=has_bom,
        newline=detect_line_ending(text),
    )


def encode_text(
    content: str,
    *,
    preserve_bom: bool,
    newline: str | None,
) -> tuple[bytes, bool]:
    explicit_bom = content.startswith("\ufeff")
    if explicit_bom:
        content = content[1:]
    if newline is not None:
        content = convert_line_endings(content, newline)
    has_bom = preserve_bom or explicit_bom
    try:
        payload = content.encode("utf-8")
    except UnicodeEncodeError:
        raise FileToolError(
            "invalid_arguments",
            "text arguments must be valid Unicode encodable as UTF-8",
        ) from None
    return ((_UTF8_BOM + payload) if has_bom else payload), has_bom


def require_utf8_text(content: str, *, argument_name: str) -> FileToolError | None:
    try:
        content.encode("utf-8")
    except UnicodeEncodeError:
        return FileToolError(
            "invalid_arguments",
            f"{argument_name} must be valid Unicode encodable as UTF-8",
        )
    return None


def detect_line_ending(text: str) -> str | None:
    first_lf = text.find("\n")
    if first_lf == -1:
        return None
    return "\r\n" if first_lf > 0 and text[first_lf - 1] == "\r" else "\n"


def convert_line_endings(text: str, newline: str) -> str:
    normalized = normalize_line_endings(text)
    return normalized if newline == "\n" else normalized.replace("\n", "\r\n")


def normalize_line_endings(text: str) -> str:
    """Normalize CRLF and lone CR to LF before matching or conversion."""

    return text.replace("\r\n", "\n").replace("\r", "\n")


async def run_mutation_thread[**P, T](
    function: Callable[P, T],
    /,
    *args: P.args,
    **kwargs: P.kwargs,
) -> T:
    """Run one mutation thread to settlement before propagating task cancellation."""

    mutation = asyncio.create_task(asyncio.to_thread(function, *args, **kwargs))
    try:
        return await asyncio.shield(mutation)
    except asyncio.CancelledError:
        while not mutation.done():
            try:
                await asyncio.shield(mutation)
            except asyncio.CancelledError:
                continue
        try:
            result = mutation.result()
        except (asyncio.CancelledError, Exception):
            raise asyncio.CancelledError from None
        raise SettledMutationCancelled(result) from None


def atomic_write_bytes(
    path: Path,
    payload: bytes,
    *,
    expected: bytes | None = None,
    expected_missing: bool = False,
) -> bool:
    """Replace a file atomically using a temporary file in the same directory."""

    parent = path.parent
    try:
        parent.mkdir(parents=True, exist_ok=True)
        if not parent.is_dir():
            raise NotADirectoryError(str(parent))
        previous_mode: int | None = None
        try:
            metadata = path.stat()
        except FileNotFoundError:
            pass
        else:
            if stat.S_ISDIR(metadata.st_mode):
                raise IsADirectoryError(str(path))
            if not stat.S_ISREG(metadata.st_mode):
                raise OSError(f"path is not a regular file: {path}")
            previous_mode = stat.S_IMODE(metadata.st_mode)

        descriptor, temporary_name = _create_temporary_file(parent, path.name)
    except OSError:
        raise

    temporary = Path(temporary_name)
    descriptor_open = True
    try:
        if previous_mode is not None and hasattr(os, "fchmod"):
            os.fchmod(descriptor, previous_mode)
        with os.fdopen(descriptor, "wb") as stream:
            descriptor_open = False
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        if (expected is not None and not _file_matches_bytes(path, expected)) or (
            expected_missing and path.exists()
        ):
            raise FileToolError(
                "stale_content",
                f"File changed since it was read; read it again before editing: {path}",
            )
        try:
            temporary_hash = _sha256_file(temporary)
        except OSError as error:
            raise OSError(f"could not verify temporary file: {error}") from None
        expected_hash = hashlib.sha256(payload).hexdigest()
        if temporary_hash != expected_hash:
            raise OSError("temporary file content did not match the intended write")
        os.replace(temporary, path)
        return True
    finally:
        if descriptor_open:
            os.close(descriptor)
        with suppress(FileNotFoundError):
            temporary.unlink()


def _create_temporary_file(parent: Path, target_name: str) -> tuple[int, str]:
    if os.name != "posix":
        return tempfile.mkstemp(
            dir=parent,
            prefix=f".{target_name}.",
            suffix=".tmp",
        )

    flags = os.O_RDWR | os.O_CREAT | os.O_EXCL
    for _attempt in range(100):
        temporary = parent / f".{target_name}.{secrets.token_hex(8)}.tmp"
        try:
            descriptor = os.open(temporary, flags, 0o666)
        except FileExistsError:
            continue
        return descriptor, str(temporary)
    raise FileExistsError(f"could not allocate a temporary file in {parent}")


def _file_matches_bytes(path: Path, expected: bytes) -> bool:
    try:
        with path.open("rb") as stream:
            offset = 0
            while True:
                chunk = stream.read(64 * 1024)
                if not chunk:
                    return offset == len(expected)
                end = offset + len(chunk)
                if end > len(expected) or chunk != expected[offset:end]:
                    return False
                offset = end
    except OSError:
        return False


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(64 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _read_regular_file(path: Path) -> bytes:
    regular_file_size(path)
    try:
        return path.read_bytes()
    except FileNotFoundError:
        raise FileToolError("file_not_found", f"File not found: {path}") from None
    except OSError as error:
        raise FileToolError("read_failed", f"Could not read file: {error}") from None


def regular_file_size(path: Path) -> int:
    try:
        metadata = path.stat()
    except FileNotFoundError:
        raise FileToolError("file_not_found", f"File not found: {path}") from None
    except OSError as error:
        raise FileToolError("read_failed", f"Could not inspect file: {error}") from None
    if stat.S_ISDIR(metadata.st_mode):
        raise FileToolError("path_is_directory", f"Path is a directory, not a file: {path}")
    if not stat.S_ISREG(metadata.st_mode):
        raise FileToolError("not_regular_file", f"Path is not a regular file: {path}")
    return metadata.st_size


def is_known_binary_path(path: Path) -> bool:
    return path.suffix.lower() in _BINARY_EXTENSIONS


def looks_binary(sample: bytes) -> bool:
    if not sample:
        return False
    if b"\x00" in sample:
        return True
    controls = sum(byte < 9 or 13 < byte < 32 for byte in sample)
    return controls / len(sample) > 0.3
