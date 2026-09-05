"""Stable Runtime errors shared across package boundaries."""

from __future__ import annotations

from typing import Final, Literal

type ProviderFailureCategory = Literal[
    "authentication",
    "rate_limit",
    "context_overflow",
    "invalid_request",
    "timeout",
    "network",
    "server",
    "cancelled",
    "protocol",
    "unknown",
]

type MemoryOperationReasonCode = Literal[
    "memory_not_found",
    "memory_revision_conflict",
    "memory_forgotten",
    "memory_idempotency_conflict",
    "memory_source_unavailable",
]
MEMORY_OPERATION_REASON_CODES: Final[tuple[MemoryOperationReasonCode, ...]] = (
    "memory_not_found",
    "memory_revision_conflict",
    "memory_forgotten",
    "memory_idempotency_conflict",
    "memory_source_unavailable",
)

type MemoryRetrievalReasonCode = Literal[
    "memory_retrieval_overflow",
    "memory_snapshot_unavailable",
]
MEMORY_RETRIEVAL_REASON_CODES: Final[tuple[MemoryRetrievalReasonCode, ...]] = (
    "memory_retrieval_overflow",
    "memory_snapshot_unavailable",
)


class RunCancelled(Exception):
    """Cooperative cancellation requested for an Agent Run."""


class ConfigError(ValueError):
    """A safe configuration error whose message never contains source values."""


class InvalidParamsError(ValueError):
    """The client supplied invalid JSON-RPC method parameters."""


class ProviderFailure(RuntimeError):
    def __init__(
        self,
        category: ProviderFailureCategory,
        safe_message: str,
        *,
        status_code: int | None = None,
        request_id: str | None = None,
        retryable: bool = False,
        retry_after: float | None = None,
    ) -> None:
        super().__init__(safe_message)
        self.category = category
        self.status_code = status_code
        self.request_id = request_id
        self.retryable = retryable
        self.retry_after = retry_after

    def __repr__(self) -> str:
        return (
            "ProviderFailure("
            f"category={self.category!r}, status_code={self.status_code!r}, "
            f"retryable={self.retryable!r})"
        )


class RuntimeHomeLockError(RuntimeError):
    """The Runtime home could not be exclusively owned by this process."""


class ProtectedValueError(RuntimeError):
    """Protected configuration data was detected without retaining its value."""


class UnsupportedJournalEventVersionError(RuntimeError):
    """A persisted Event cannot be interpreted by this Runtime version."""


class RunInputDriftError(RuntimeError):
    """The current execution environment no longer matches a queued Run Frame."""

    def __init__(self, reason_code: str) -> None:
        super().__init__(reason_code)
        self.reason_code = reason_code


class ContextBudgetExceededError(RuntimeError):
    """The current Run cannot fit the frozen provider-neutral input budget."""


class AgentStepLimitError(RuntimeError):
    """The Run used all of its frozen model Steps without completing the task."""


class ModelInputUnavailableError(RuntimeError):
    """A provider-neutral input source is missing, corrupt, or no longer canonical."""


class MemorySchemaIncompatibleError(RuntimeError):
    """The durable Memory database cannot be opened by this Runtime release."""


class MemoryOperationError(RuntimeError):
    """A stable, non-secret Memory mutation or provenance failure."""

    def __init__(self, reason_code: MemoryOperationReasonCode) -> None:
        if reason_code not in MEMORY_OPERATION_REASON_CODES:
            raise ValueError("unsupported Memory operation reason code")
        super().__init__(reason_code)
        self.reason_code = reason_code


class MemoryRetrievalError(RuntimeError):
    """A stable, non-secret Memory selection or materialization failure."""

    def __init__(self, reason_code: MemoryRetrievalReasonCode) -> None:
        if reason_code not in MEMORY_RETRIEVAL_REASON_CODES:
            raise ValueError("unsupported Memory retrieval reason code")
        super().__init__(reason_code)
        self.reason_code = reason_code
