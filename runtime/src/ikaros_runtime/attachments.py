"""Validated local-file attachments carried by one User Item.

The records in this module contain no file bodies: only the path plus the
integrity facts (``size`` and ``sha256``) that let a later model input rebuild
the exact bytes or degrade explicitly when they are gone.  Image bytes are read
lazily and encoded as ``data:`` URLs; nothing is ever fetched from the network.
"""

from __future__ import annotations

import base64
import hashlib
import re
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Literal, cast

from .protocol.spec import (
    ATTACHMENT_IMAGE_MIME_TYPES,
    ATTACHMENT_KINDS,
    ATTACHMENT_MAX_BYTES,
    ATTACHMENT_MAX_COUNT,
)

type AttachmentKind = Literal["image", "text"]

# Conservative per-image budget estimate: vision Providers tile images, so a
# fixed cost keeps the UTF-8 character measurement honest without decoding the
# image itself.  Text attachments are path references; only the rendered note
# enters the input.
ATTACHMENT_IMAGE_TOKEN_ESTIMATE = 2_048
ATTACHMENT_TEXT_TOKEN_OVERHEAD = 64

_MIME_TYPE = re.compile(r"^[a-z0-9][a-z0-9!#$&^_.+-]{0,63}/[a-z0-9][a-z0-9!#$&^_.+-]{0,63}$")
_IMAGE_SIGNATURES: tuple[tuple[str, bytes], ...] = (
    ("image/png", b"\x89PNG\r\n\x1a\n"),
    ("image/jpeg", b"\xff\xd8\xff"),
    ("image/gif", b"GIF87a"),
    ("image/gif", b"GIF89a"),
)
_WEBP_RIFF = b"RIFF"
_WEBP_FORMAT = b"WEBP"
_HEAD_BYTES = 16
_READ_CHUNK_BYTES = 1 << 20
_INPUT_KEYS = {"kind", "path", "mimeType"}
_REF_KEYS = {"kind", "path", "mimeType", "size", "sha256"}


class AttachmentUnavailableError(Exception):
    """A recorded attachment can no longer be sent to a Provider."""


@dataclass(frozen=True, slots=True)
class AttachmentInput:
    """One ``turn.start`` / ``run.steer`` attachment request."""

    kind: AttachmentKind
    path: str
    mime_type: str


@dataclass(frozen=True, slots=True)
class AttachmentRef:
    """The durable attachment reference stored on a User Item."""

    kind: AttachmentKind
    path: str
    mime_type: str
    size: int
    sha256: str

    def __post_init__(self) -> None:
        if self.kind not in ATTACHMENT_KINDS:
            raise ValueError("attachment kind is invalid")
        if not isinstance(self.path, str) or not self.path:
            raise ValueError("attachment path is invalid")
        _validate_mime_type(self.kind, self.mime_type)
        if not isinstance(self.size, int) or isinstance(self.size, bool) or self.size < 1:
            raise ValueError("attachment size is invalid")
        if (
            not isinstance(self.sha256, str)
            or len(self.sha256) != 64
            or any(character not in "0123456789abcdef" for character in self.sha256)
        ):
            raise ValueError("attachment sha256 is invalid")

    def to_wire(self) -> dict[str, object]:
        return {
            "kind": self.kind,
            "path": self.path,
            "mimeType": self.mime_type,
            "size": self.size,
            "sha256": self.sha256,
        }

    @classmethod
    def from_wire(cls, value: object) -> AttachmentRef:
        if not isinstance(value, dict) or set(value) != _REF_KEYS:
            raise ValueError("attachment reference has an invalid structure")
        row = cast(dict[str, Any], value)
        size = row["size"]
        if not isinstance(size, int) or isinstance(size, bool):
            raise ValueError("attachment size is invalid")
        return cls(
            kind=cast(AttachmentKind, row["kind"]),
            path=cast(str, row["path"]),
            mime_type=cast(str, row["mimeType"]),
            size=size,
            sha256=cast(str, row["sha256"]),
        )


def attachment_inputs_from_params(value: object) -> tuple[AttachmentInput, ...]:
    """Validate the RPC-level ``attachments`` argument of one User message."""

    if value is None:
        return ()
    if not isinstance(value, list):
        raise ValueError("attachments must be an array")
    if len(value) > ATTACHMENT_MAX_COUNT:
        raise ValueError(f"at most {ATTACHMENT_MAX_COUNT} attachments are allowed")
    inputs: list[AttachmentInput] = []
    seen: set[str] = set()
    for item in value:
        if not isinstance(item, dict) or set(item) != _INPUT_KEYS:
            raise ValueError("each attachment requires kind, path, and mimeType")
        row = cast(dict[str, Any], item)
        kind = row["kind"]
        path = row["path"]
        mime_type = row["mimeType"]
        if kind not in ATTACHMENT_KINDS:
            raise ValueError("attachment kind must be image or text")
        if not isinstance(path, str) or not path:
            raise ValueError("attachment path must be a non-empty string")
        if not isinstance(mime_type, str):
            raise ValueError("attachment mimeType must be a string")
        _validate_mime_type(kind, mime_type)
        if path in seen:
            raise ValueError("attachment paths must be unique")
        seen.add(path)
        inputs.append(AttachmentInput(kind=kind, path=path, mime_type=mime_type))
    return tuple(inputs)


def resolve_attachment_inputs(value: object) -> tuple[AttachmentRef, ...]:
    """Resolve request attachments into durable references backed by local files."""

    refs = tuple(_resolve_input(item) for item in attachment_inputs_from_params(value))
    return refs


def attachment_refs_to_wire(refs: Sequence[AttachmentRef]) -> list[dict[str, object]]:
    return [ref.to_wire() for ref in refs]


def attachment_refs_from_wire(value: object) -> tuple[AttachmentRef, ...]:
    """Read attachment references from stored Item data."""

    if value is None:
        return ()
    if not isinstance(value, list):
        raise ValueError("attachment references must be an array")
    if len(value) > ATTACHMENT_MAX_COUNT:
        raise ValueError("stored attachments exceed the supported count")
    return tuple(AttachmentRef.from_wire(item) for item in value)


def message_attachment_refs(data: Mapping[str, Any]) -> tuple[AttachmentRef, ...]:
    """Return the attachment references recorded on one Item, if any."""

    return attachment_refs_from_wire(data.get("attachments"))


def estimate_attachment_tokens(refs: Sequence[AttachmentRef]) -> int:
    tokens = 0
    for ref in refs:
        if ref.kind == "image":
            tokens += ATTACHMENT_IMAGE_TOKEN_ESTIMATE
        else:
            tokens += ATTACHMENT_TEXT_TOKEN_OVERHEAD + len(ref.path) // 4
    return tokens


def attachment_reference_line(ref: AttachmentRef, *, notice: str | None = None) -> str:
    line = f"- [{ref.kind}] {ref.path} ({ref.mime_type}, {ref.size} bytes, sha256 {ref.sha256})"
    if notice is not None:
        line = f"{line} - {notice}"
    return line


def attachments_prompt_note(lines: Sequence[str]) -> str:
    return "Attached files (local paths on the Runtime host):\n" + "\n".join(lines)


def image_data_url(ref: AttachmentRef) -> str:
    """Encode one recorded image as an inline ``data:`` URL."""

    if ref.kind != "image":
        raise ValueError("only image attachments can be encoded")
    path = Path(ref.path)
    try:
        size = path.stat().st_size
    except OSError:
        raise AttachmentUnavailableError("attachment file is missing") from None
    if size != ref.size or size > ATTACHMENT_MAX_BYTES:
        raise AttachmentUnavailableError("attachment file changed after it was attached")
    try:
        data = path.read_bytes()
    except OSError:
        raise AttachmentUnavailableError("attachment file is missing") from None
    if len(data) != ref.size or hashlib.sha256(data).hexdigest() != ref.sha256:
        raise AttachmentUnavailableError("attachment file changed after it was attached")
    return f"data:{ref.mime_type};base64,{base64.b64encode(data).decode('ascii')}"


def _resolve_input(item: AttachmentInput) -> AttachmentRef:
    path = Path(item.path)
    if not path.is_absolute():
        raise ValueError("attachment path must be absolute")
    try:
        stat = path.stat()
    except OSError:
        raise ValueError(f"attachment file was not found: {item.path}") from None
    if not path.is_file():
        raise ValueError(f"attachment is not a file: {item.path}")
    size = stat.st_size
    if size < 1:
        raise ValueError(f"attachment file is empty: {item.path}")
    if size > ATTACHMENT_MAX_BYTES:
        raise ValueError(
            f"attachment exceeds the {ATTACHMENT_MAX_BYTES} byte limit: {item.path}"
        )
    digest, head = _digest_and_head(path)
    if item.kind == "image" and not _image_signature_matches(item.mime_type, head):
        raise ValueError(f"attachment is not a valid {item.mime_type} image: {item.path}")
    return AttachmentRef(
        kind=item.kind,
        path=item.path,
        mime_type=item.mime_type,
        size=size,
        sha256=digest,
    )


def _digest_and_head(path: Path) -> tuple[str, bytes]:
    digest = hashlib.sha256()
    head = b""
    try:
        with path.open("rb") as stream:
            while chunk := stream.read(_READ_CHUNK_BYTES):
                if not head:
                    head = chunk[:_HEAD_BYTES]
                digest.update(chunk)
    except OSError:
        raise ValueError(f"attachment file could not be read: {path}") from None
    return digest.hexdigest(), head


def _image_signature_matches(mime_type: str, head: bytes) -> bool:
    if mime_type == "image/webp":
        return head.startswith(_WEBP_RIFF) and head[8:12] == _WEBP_FORMAT
    return any(
        candidate == mime_type and head.startswith(signature)
        for candidate, signature in _IMAGE_SIGNATURES
    )


def _validate_mime_type(kind: AttachmentKind, mime_type: object) -> None:
    if not isinstance(mime_type, str) or not _MIME_TYPE.match(mime_type):
        raise ValueError("attachment mimeType is invalid")
    if kind == "image" and mime_type not in ATTACHMENT_IMAGE_MIME_TYPES:
        raise ValueError(
            "image attachments must be one of " + ", ".join(ATTACHMENT_IMAGE_MIME_TYPES)
        )


__all__ = [
    "ATTACHMENT_IMAGE_TOKEN_ESTIMATE",
    "ATTACHMENT_TEXT_TOKEN_OVERHEAD",
    "AttachmentInput",
    "AttachmentKind",
    "AttachmentRef",
    "AttachmentUnavailableError",
    "attachment_inputs_from_params",
    "attachment_reference_line",
    "attachment_refs_from_wire",
    "attachment_refs_to_wire",
    "attachments_prompt_note",
    "estimate_attachment_tokens",
    "image_data_url",
    "message_attachment_refs",
    "resolve_attachment_inputs",
]
