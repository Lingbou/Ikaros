from __future__ import annotations

from collections.abc import Callable, Mapping, Sequence

from .errors import ConfigError, InvalidParamsError
from .errors import ProtectedValueError as ProtectedValueError
from .protocol.spec import JOURNAL_EVENT_TYPE_SET, PROVIDER_TOOL_ID_SET, SERVER_NAME
from .run_input import (
    INPUT_BUDGET_MEASUREMENT_VERSION,
)

type ProtectedValuesSource = Callable[[], Sequence[str]]
type JournalSecretProbe = Callable[[Sequence[str]], bool]


class RuntimeSecurity:
    """Centralize the Runtime's credential non-disclosure invariants."""

    def __init__(
        self,
        protected_values: ProtectedValuesSource,
        journal_contains: JournalSecretProbe,
    ) -> None:
        self._protected_values = protected_values
        self._journal_contains = journal_contains

    def assert_configuration_safe(self) -> None:
        if self._journal_contains(self.protected_values()):
            raise ConfigError("configured credentials conflict with persisted Runtime data")

    def assert_request_safe(self, value: object) -> None:
        protected_values = self.protected_values()
        if protected_values and json_contains_protected_value(value, protected_values):
            raise InvalidParamsError("request contains protected configuration data")

    def assert_credentials_safe(self, values: Sequence[object]) -> None:
        protected_values = tuple(value for value in values if isinstance(value, str) and value)
        if self._journal_contains(protected_values):
            raise InvalidParamsError("credentials conflict with persisted Runtime data")

    def protected_values(self) -> tuple[str, ...]:
        return tuple(dict.fromkeys(value for value in self._protected_values() if value))

    def rpc_request_values_contain_protected_value(
        self,
        value: object,
        additional_values: Sequence[str] = (),
    ) -> bool:
        protected_values = (*self.protected_values(), *additional_values)
        return bool(protected_values) and rpc_request_values_contain_protected_value(
            value,
            protected_values,
        )

    def response_contains_protected_value(
        self,
        value: object,
        additional_values: Sequence[str] = (),
    ) -> bool:
        protected_values = (*self.protected_values(), *additional_values)
        return bool(protected_values) and response_values_contain_protected_value(
            value,
            protected_values,
        )


class ProtectedStreamGuard:
    def __init__(self, protected_values: Sequence[str]) -> None:
        values = tuple(dict.fromkeys(value for value in protected_values if value))
        self._patterns = tuple((value, _kmp_failure_table(value)) for value in values)
        self._states = [0] * len(self._patterns)
        self._pending: list[str] = []

    def feed(self, value: str) -> str:
        emitted: list[str] = []
        for character in value:
            self._pending.append(character)
            for index, (pattern, failure) in enumerate(self._patterns):
                state = self._states[index]
                while state > 0 and pattern[state] != character:
                    state = failure[state - 1]
                if pattern[state] == character:
                    state += 1
                if state == len(pattern):
                    raise ProtectedValueError("protected configuration data detected")
                self._states[index] = state
            held_characters = max(self._states, default=0)
            safe_characters = len(self._pending) - held_characters
            if safe_characters > 0:
                emitted.extend(self._pending[:safe_characters])
                del self._pending[:safe_characters]
        return "".join(emitted)

    def finish(self) -> str:
        value = "".join(self._pending)
        self._pending.clear()
        self._states = [0] * len(self._patterns)
        return value


def contains_protected_value(value: str, protected_values: Sequence[str]) -> bool:
    return any(protected in value for protected in protected_values)


def json_contains_protected_value(value: object, protected_values: Sequence[str]) -> bool:
    pending = [value]
    while pending:
        current = pending.pop()
        if isinstance(current, str):
            if contains_protected_value(current, protected_values):
                return True
        elif isinstance(current, Mapping):
            pending.extend(current.keys())
            pending.extend(current.values())
        elif isinstance(current, (list, tuple)):
            pending.extend(current)
    return False


def json_values_contain_protected_value(
    value: object,
    protected_values: Sequence[str],
) -> bool:
    """Strict substring scan of data values while ignoring fixed mapping keys."""
    pending = [value]
    while pending:
        current = pending.pop()
        if isinstance(current, str):
            if contains_protected_value(current, protected_values):
                return True
        elif isinstance(current, Mapping):
            pending.extend(current.values())
        elif isinstance(current, (list, tuple)):
            pending.extend(current)
    return False


def rpc_request_values_contain_protected_value(
    value: object,
    protected_values: Sequence[str],
) -> bool:
    """Inspect client-controlled RPC scalars without treating schema as secret data.

    Short values use equality because substring matching a one-character credential
    against fixed protocol vocabulary would reject every response. Values of eight
    characters or more retain defense-in-depth substring detection.
    """
    pending = [value]
    while pending:
        current = pending.pop()
        if isinstance(current, str):
            if any(
                current == protected or (len(protected) >= 8 and protected in current)
                for protected in protected_values
            ):
                return True
        elif isinstance(current, Mapping):
            pending.extend(current.values())
        elif isinstance(current, (list, tuple)):
            pending.extend(current)
    return False


_FIXED_STATUSES = frozenset({"queued", "running", "streaming", "completed", "failed", "cancelled"})
_FIXED_ROLES = frozenset({"user", "assistant", "tool"})
_FIXED_KINDS = frozenset({"message", "tool_call", "tool_result"})
_FIXED_MEMORY_KINDS = frozenset({"fact", "preference", "relationship", "project"})
_FIXED_MEMORY_STATES = frozenset({"active", "forgotten"})
_FIXED_MEMORY_SCOPE_TYPES = frozenset({"global", "workspace"})
_FIXED_MEMORY_SOURCE_KINDS = frozenset({"user_explicit", "session_item"})
_FIXED_MEMORY_PROVENANCE_STATUSES = frozenset(
    {"not_applicable", "available", "unavailable"}
)
_GENERATED_ID_PREFIXES = {
    "branchId": "branch_",
    "defaultBranchId": "branch_",
    "itemId": "item_",
    "memoryId": "memory_",
    "runId": "run_",
    "stepId": "step_",
    "threadId": "thread_",
    "toolCallItemId": "item_",
    "turnId": "turn_",
    "userItemId": "item_",
}
_TIMESTAMP_KEYS = frozenset(
    {
        "activityDate",
        "archivedAt",
        "completedAt",
        "createdAt",
        "finishedAt",
        "forgottenAt",
        "preparedAt",
        "recordedAt",
        "settledAt",
        "startDate",
        "timestamp",
        "updatedAt",
    }
)
_FIXED_RESPONSE_KEYS = frozenset(
    {
        "accepted",
        "activityDate",
        "archivedAt",
        "arguments",
        "branchId",
        "branch",
        "callId",
        "cachedInputTokens",
        "cancelled",
        "changed",
        "bom",
        "bytesRead",
        "bytesWritten",
        "capabilities",
        "code",
        "configured",
        "content",
        "clientRequestId",
        "createdAt",
        "created",
        "completedAt",
        "credentialConfigured",
        "currentStreakDays",
        "cwd",
        "data",
        "dailyUsageBuckets",
        "description",
        "diagnostics",
        "defaultBranchId",
        "delta",
        "displayName",
        "durationMs",
        "entry",
        "enabled",
        "error",
        "errorCode",
        "event",
        "eventReplay",
        "events",
        "executionPolicy",
        "exitCode",
        "hasMore",
        "health",
        "id",
        "item",
        "items",
        "itemId",
        "inputTokens",
        "isDefault",
        "jsonrpc",
        "key",
        "kind",
        "latestSeq",
        "lineEnd",
        "lineStart",
        "lineTruncations",
        "location",
        "lifetimeTokens",
        "longestRunningTurnSec",
        "longestStreakDays",
        "message",
        "method",
        "model",
        "modelId",
        "models",
        "name",
        "newline",
        "nextOffset",
        "nextAfterSeq",
        "nextCursor",
        "ok",
        "ordinal",
        "origin",
        "outcome",
        "output",
        "outputTokens",
        "path",
        "peakDailyTokens",
        "params",
        "payload",
        "protocolVersion",
        "provider",
        "providerId",
        "providers",
        "reasonCode",
        "reasoningOutputTokens",
        "reasoningContent",
        "replacements",
        "removed",
        "result",
        "role",
        "run",
        "runs",
        "runCancellation",
        "runId",
        "scriptedProvider",
        "schemaVersion",
        "seq",
        "server",
        "settledAt",
        "status",
        "skill",
        "skills",
        "stderr",
        "stdout",
        "stepId",
        "streaming",
        "snapshotSeq",
        "startDate",
        "stepOrdinal",
        "summary",
        "thread",
        "threadId",
        "threads",
        "timedOut",
        "totalTokens",
        "totalLines",
        "timestamp",
        "title",
        "toolCallId",
        "toolCallItemId",
        "toolName",
        "tools",
        "truncated",
        "turnId",
        "turns",
        "type",
        "usage",
        "updatedAt",
        "version",
        "verified",
        "workspace",
        "rootUri",
        # Provider-neutral model-input snapshot vocabulary.
        "authority",
        "baseUrl",
        "budget",
        "characters",
        "contentSha256",
        "contextData",
        "contextDataCharacters",
        "contextRevision",
        "compactionSummary",
        "currentRunOmittedThroughItemId",
        "contextWindow",
        "maxOutputTokens",
        "maximumTokens",
        "reservedCurrentRunTokens",
        "instructionTokens",
        "contextDataTokens",
        "toolTokens",
        "historyTokens",
        "currentRunTokens",
        "memoryTokens",
        "definitionSha256",
        "descriptorSha256",
        "finishedAt",
        "forgottenAt",
        "historyGroups",
        "historyItems",
        "identityCore",
        "inputSchema",
        "instructions",
        "itemIds",
        "lifetime",
        "measurementVersion",
        "memory",
        "memoryCharacters",
        "memoryContextVersion",
        "memoryId",
        "memories",
        "mode",
        "omissions",
        "omittedItemIds",
        "outputStyle",
        "preparedAt",
        "preview",
        "provenance",
        "publicProviderConfigFingerprint",
        "requestId",
        "targetTokens",
        "trigger",
        "reason",
        "resultingRevision",
        "responseModelId",
        "revision",
        "skillCatalog",
        "source",
        "sourceId",
        "sourceKind",
        "sourceType",
        "stepInput",
        "scope",
        "state",
        "runConfig",
        "supportsTools",
        "userItemId",
        # Runtime-owned file inspector metadata; body/path/diff remain guarded.
        "before",
        "after",
        "operation",
        "diff",
        "additions",
        "deletions",
        "recordedAt",
        "exists",
        "byteCount",
        "encoding",
        "lineCount",
        "truncationReason",
    }
)


def response_values_contain_protected_value(
    value: object,
    protected_values: Sequence[str],
) -> bool:
    """Inspect only data-bearing response values, not immutable wire vocabulary.

    The Runtime permits credentials that happen to equal protocol constants such
    as ``2.0`` or ``full_access``.  Treating every serialized byte as secret data
    would therefore make valid responses impossible.  Mapping keys and the fixed
    values below have protocol provenance; all other string values remain guarded.
    """

    values = tuple(dict.fromkeys(protected for protected in protected_values if protected))
    if not values:
        return False
    pending: list[
        tuple[tuple[str, ...], object, Mapping[object, object] | None]
    ] = [((), value, None)]
    while pending:
        path, current, container = pending.pop()
        if isinstance(current, str):
            if _is_fixed_response_value(path, current, container):
                continue
            if _matches_protected_value(current, values):
                return True
        elif isinstance(current, Mapping):
            for key, child in current.items():
                child_path = path + (str(key),)
                if (
                    isinstance(key, str)
                    and key not in _FIXED_RESPONSE_KEYS
                    and not _is_runtime_owned_model_input_path(child_path)
                    and _matches_protected_value(key, values)
                ):
                    return True
                pending.append((child_path, child, current))
        elif isinstance(current, (list, tuple)):
            pending.extend((path, child, None) for child in current)
    return False


def _is_fixed_response_value(
    path: tuple[str, ...],
    value: str,
    container: Mapping[object, object] | None,
) -> bool:
    if _is_runtime_owned_model_input_path(path):
        return True
    if path == ("jsonrpc",) and value == "2.0":
        return True
    if path == ("method",) and value == "event":
        return True
    if path[-2:] == ("server", "name") and value == SERVER_NAME:
        return True
    if path[-2:] == ("server", "version"):
        return True
    if path[-2:] == ("capabilities", "tools") and value in PROVIDER_TOOL_ID_SET:
        return True
    if path and path[-1] == "newline" and value in {"lf", "crlf", "mixed"}:
        return True
    if path and container is not None:
        file_record = "toolCallItemId" in container and "before" in container
        file_preview = "threadId" in container and "path" in container
        file_metadata = "exists" in container and "byteCount" in container
        if (file_record or file_preview or file_metadata) and (
            (path[-1] == "status" and value in {"text", "unavailable", "recorded"})
            or (path[-1] == "operation" and value in {"write", "edit"})
            or (path[-1] == "encoding" and value == "utf-8")
            or (path[-1] == "revision" and _is_sha256(value))
            or (path[-1] == "truncationReason" and value in {
                "byte_limit", "line_limit", "scan_limit",
            })
            or (path[-1] == "reason" and value in {
                "file_not_found", "not_a_file", "binary_file", "unsupported_encoding",
                "too_large", "scan_limit", "revision_changed", "read_failed",
                "protected_content", "not_recorded", "result_unknown",
            })
        ):
            return True
    if path and path[-1] == "executionPolicy" and value == "full_access":
        return True
    if (
        path
        and path[-1] == "measurementVersion"
        and value == INPUT_BUDGET_MEASUREMENT_VERSION
    ):
        return True
    if path and path[-1] in {
        "contentSha256",
        "definitionSha256",
        "descriptorSha256",
        "publicProviderConfigFingerprint",
    }:
        return _is_sha256(value)
    if (
        path
        and path[-1] == "reason"
        and value in {"omitted_by_budget", "omitted_by_limit"}
        and container is not None
        and container.get("sourceType") in {"history", "memory"}
    ):
        return True
    if path and path[-1] == "type" and value in JOURNAL_EVENT_TYPE_SET:
        return True
    if path and path[-1] in {"status", "outcome"} and value in _FIXED_STATUSES:
        return True
    if path and path[-1] == "role" and value in _FIXED_ROLES:
        return True
    if path and path[-1] == "kind" and value in _FIXED_KINDS:
        return True
    if (
        path
        and path[-1] == "kind"
        and value in _FIXED_MEMORY_KINDS
        and _is_memory_record_container(container)
    ):
        return True
    if (
        path
        and path[-1] == "state"
        and value in _FIXED_MEMORY_STATES
        and _is_memory_record_container(container)
    ):
        return True
    if (
        path[-2:] == ("scope", "type")
        and value in _FIXED_MEMORY_SCOPE_TYPES
        and container is not None
        and set(container) == {"type", "key"}
    ):
        return True
    if (
        path[-2:] == ("provenance", "sourceKind")
        and value in _FIXED_MEMORY_SOURCE_KINDS
        and _is_memory_provenance_container(container)
    ):
        return True
    if (
        path[-2:] == ("provenance", "status")
        and value in _FIXED_MEMORY_PROVENANCE_STATUSES
        and _is_memory_provenance_container(container)
    ):
        return True
    if path and path[-1] == "providerId" and value == "scripted":
        return True
    if (
        path
        and path[-1] == "modelId"
        and value == "scripted-v1"
        and container is not None
        and container.get("providerId") == "scripted"
    ):
        return True
    if path and path[-1] == "reasonCode":
        return True
    if path and path[-1] in _TIMESTAMP_KEYS:
        return True
    if path and path[-1] == "id":
        prefix = _generated_record_id_prefix(path)
        return prefix is not None and _is_generated_identifier(value, prefix)
    if path and (prefix := _GENERATED_ID_PREFIXES.get(path[-1])) is not None:
        return _is_generated_identifier(value, prefix)
    return False


def _is_runtime_owned_model_input_path(path: tuple[str, ...]) -> bool:
    """Return whether a snapshot path contains only Runtime-owned provenance.

    Snapshot roots must be direct Journal-event payload fields. This prevents a
    Tool result from smuggling protected text under look-alike object names.
    Run configurations are handled field by field because their Workspace,
    Skill, and Provider/Model values are dynamic.
    """

    frame_index = _journal_payload_child_index(path, "runConfig")
    if frame_index is not None:
        relative = path[frame_index + 1 :]
        if relative[:1] == ("tools",):
            return True
        if relative[:2] in {
            ("instructions", "outputStyle"),
            ("instructions", "identityCore"),
        }:
            return True
        if relative[:2] == ("instructions", "skillCatalog"):
            return relative[-1:] != ("content",)

    return any(
        _journal_payload_child_index(path, field) is not None
        for field in ("contextRevision", "stepInput")
    )


def _journal_payload_child_index(path: tuple[str, ...], field: str) -> int | None:
    for root in (
        ("payload", field),
        ("params", "payload", field),
        ("result", "events", "payload", field),
    ):
        if path[: len(root)] == root:
            return len(root) - 1
    return None


def _generated_record_id_prefix(path: tuple[str, ...]) -> str | None:
    if len(path) < 2:
        return None
    return {
        "branch": "branch_",
        "branches": "branch_",
        "item": "item_",
        "items": "item_",
        "memory": "memory_",
        "memories": "memory_",
        "run": "run_",
        "runs": "run_",
        "thread": "thread_",
        "threads": "thread_",
        "turn": "turn_",
        "turns": "turn_",
    }.get(path[-2])


def _is_memory_record_container(
    container: Mapping[object, object] | None,
) -> bool:
    return container is not None and {
        "id",
        "kind",
        "scope",
        "revision",
        "state",
        "createdAt",
        "updatedAt",
        "forgottenAt",
    }.issubset(container)


def _is_memory_provenance_container(
    container: Mapping[object, object] | None,
) -> bool:
    return container is not None and set(container) == {
        "sourceKind",
        "threadId",
        "turnId",
        "itemId",
        "status",
    }


def _is_sha256(value: str) -> bool:
    return len(value) == 64 and all(character in "0123456789abcdef" for character in value)


def _matches_protected_value(value: str, protected_values: Sequence[str]) -> bool:
    return any(
        value == protected or (len(protected) >= 8 and protected in value)
        for protected in protected_values
    )


def _is_generated_identifier(value: str, prefix: str) -> bool:
    suffix = value.removeprefix(prefix)
    return len(suffix) == 32 and all(character in "0123456789abcdef" for character in suffix)


def _kmp_failure_table(pattern: str) -> tuple[int, ...]:
    failure = [0] * len(pattern)
    matched = 0
    for index in range(1, len(pattern)):
        while matched > 0 and pattern[matched] != pattern[index]:
            matched = failure[matched - 1]
        if pattern[matched] == pattern[index]:
            matched += 1
        failure[index] = matched
    return tuple(failure)
