"""Stable Runtime errors shared across package boundaries."""

from __future__ import annotations

from typing import Literal

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
