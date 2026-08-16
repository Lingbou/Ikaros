"""Provider-independent domain records for explicit long-term Memory."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from ..domain import JsonObject

type MemoryKind = Literal["fact", "preference", "relationship", "project"]
type MemoryState = Literal["active", "forgotten"]
type MemoryScopeType = Literal["global", "workspace"]
type MemorySourceKind = Literal["user_explicit", "session_item"]
type ProvenanceStatus = Literal["not_applicable", "available", "unavailable"]

MEMORY_KINDS = frozenset({"fact", "preference", "relationship", "project"})
MEMORY_STATES = frozenset({"active", "forgotten"})
MEMORY_SCOPE_TYPES = frozenset({"global", "workspace"})
MEMORY_CONTENT_MAX_CHARACTERS = 2_048
MEMORY_PREVIEW_MAX_CHARACTERS = 160


@dataclass(frozen=True, slots=True)
class MemoryScope:
    type: MemoryScopeType
    key: str | None

    def __post_init__(self) -> None:
        if self.type not in MEMORY_SCOPE_TYPES:
            raise ValueError("Memory scope type is invalid")
        if self.type == "global":
            if self.key is not None:
                raise ValueError("Global Memory scope must not have a key")
            return
        if (
            self.key is None
            or not self.key
            or self.key != self.key.strip()
            or len(self.key) > 200
            or _contains_invalid_control(self.key)
            or not _is_valid_utf8(self.key)
        ):
            raise ValueError("Workspace Memory scope key is invalid")

    def to_wire(self) -> JsonObject:
        return {"type": self.type, "key": self.key}


@dataclass(frozen=True, slots=True)
class MemoryProvenance:
    source_kind: MemorySourceKind
    thread_id: str | None
    turn_id: str | None
    item_id: str | None
    status: ProvenanceStatus

    def __post_init__(self) -> None:
        references = (self.thread_id, self.turn_id, self.item_id)
        if self.source_kind == "user_explicit":
            if any(value is not None for value in references):
                raise ValueError("Explicit-user Memory provenance must not reference a Session")
            if self.status != "not_applicable":
                raise ValueError("Explicit-user Memory provenance status is invalid")
            return
        if self.source_kind != "session_item":
            raise ValueError("Memory provenance source kind is invalid")
        if any(value is None for value in references):
            raise ValueError("Session Memory provenance must be complete")
        if self.status not in {"available", "unavailable"}:
            raise ValueError("Session Memory provenance status is invalid")

    def to_wire(self) -> JsonObject:
        return {
            "sourceKind": self.source_kind,
            "threadId": self.thread_id,
            "turnId": self.turn_id,
            "itemId": self.item_id,
            "status": self.status,
        }


@dataclass(frozen=True, slots=True)
class MemoryRecord:
    id: str
    kind: MemoryKind
    scope: MemoryScope
    revision: int
    state: MemoryState
    content: str | None
    provenance: MemoryProvenance
    created_at: str
    updated_at: str
    forgotten_at: str | None

    def __post_init__(self) -> None:
        _validate_memory_id(self.id)
        if self.kind not in MEMORY_KINDS:
            raise ValueError("Memory kind is invalid")
        if isinstance(self.revision, bool) or self.revision < 1:
            raise ValueError("Memory revision is invalid")
        if self.state not in MEMORY_STATES:
            raise ValueError("Memory state is invalid")
        if self.state == "active":
            validate_memory_content(self.content)
            if self.forgotten_at is not None:
                raise ValueError("Active Memory must not have a forgotten timestamp")
        elif self.content is not None or self.forgotten_at is None:
            raise ValueError("Forgotten Memory must contain only tombstone metadata")

    def to_wire(self) -> JsonObject:
        return {
            "id": self.id,
            "kind": self.kind,
            "scope": self.scope.to_wire(),
            "revision": self.revision,
            "state": self.state,
            "content": self.content,
            "provenance": self.provenance.to_wire(),
            "createdAt": self.created_at,
            "updatedAt": self.updated_at,
            "forgottenAt": self.forgotten_at,
        }

    def to_summary(self) -> MemorySummary:
        return MemorySummary(
            id=self.id,
            kind=self.kind,
            scope=self.scope,
            revision=self.revision,
            state=self.state,
            preview=(
                self.content[:MEMORY_PREVIEW_MAX_CHARACTERS]
                if self.content is not None
                else None
            ),
            created_at=self.created_at,
            updated_at=self.updated_at,
            forgotten_at=self.forgotten_at,
        )


@dataclass(frozen=True, slots=True)
class MemorySummary:
    id: str
    kind: MemoryKind
    scope: MemoryScope
    revision: int
    state: MemoryState
    preview: str | None
    created_at: str
    updated_at: str
    forgotten_at: str | None

    def to_wire(self) -> JsonObject:
        return {
            "id": self.id,
            "kind": self.kind,
            "scope": self.scope.to_wire(),
            "revision": self.revision,
            "state": self.state,
            "preview": self.preview,
            "createdAt": self.created_at,
            "updatedAt": self.updated_at,
            "forgottenAt": self.forgotten_at,
        }


@dataclass(frozen=True, slots=True)
class MemoryCreateReceipt:
    memory_id: str
    resulting_revision: int
    created: bool

    def to_wire(self) -> JsonObject:
        return {
            "memoryId": self.memory_id,
            "resultingRevision": self.resulting_revision,
            "created": self.created,
        }


@dataclass(frozen=True, slots=True)
class MemoryListPage:
    memories: tuple[MemorySummary, ...]
    next_cursor: str | None
    has_more: bool

    def to_wire(self) -> JsonObject:
        return {
            "memories": [memory.to_wire() for memory in self.memories],
            "nextCursor": self.next_cursor,
            "hasMore": self.has_more,
        }


def validate_memory_content(value: object) -> str:
    if not isinstance(value, str):
        raise ValueError("Memory content must be a string")
    if not value.strip():
        raise ValueError("Memory content must not be blank")
    if len(value) > MEMORY_CONTENT_MAX_CHARACTERS:
        raise ValueError(
            f"Memory content must not exceed {MEMORY_CONTENT_MAX_CHARACTERS} characters"
        )
    if _contains_invalid_control(value):
        raise ValueError("Memory content contains unsupported control characters")
    try:
        value.encode("utf-8", errors="strict")
    except UnicodeEncodeError:
        raise ValueError("Memory content must be valid Unicode") from None
    return value


def _validate_memory_id(value: str) -> None:
    prefix = "memory_"
    suffix = value.removeprefix(prefix)
    if len(suffix) != 32 or any(character not in "0123456789abcdef" for character in suffix):
        raise ValueError("Memory ID is invalid")


def _contains_invalid_control(value: str) -> bool:
    return any(
        (ord(character) < 0x20 and character not in "\t\n\r")
        or ord(character) == 0x7F
        for character in value
    )


def _is_valid_utf8(value: str) -> bool:
    try:
        value.encode("utf-8", errors="strict")
    except UnicodeEncodeError:
        return False
    return True


__all__ = [
    "MEMORY_CONTENT_MAX_CHARACTERS",
    "MEMORY_KINDS",
    "MEMORY_PREVIEW_MAX_CHARACTERS",
    "MEMORY_SCOPE_TYPES",
    "MEMORY_STATES",
    "MemoryCreateReceipt",
    "MemoryKind",
    "MemoryListPage",
    "MemoryProvenance",
    "MemoryRecord",
    "MemoryScope",
    "MemoryScopeType",
    "MemorySourceKind",
    "MemoryState",
    "MemorySummary",
    "ProvenanceStatus",
    "validate_memory_content",
]
