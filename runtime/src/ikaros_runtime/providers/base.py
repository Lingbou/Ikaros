from __future__ import annotations

from collections.abc import AsyncIterator, Sequence
from dataclasses import dataclass
from typing import Protocol

from ..cancellation import CancellationToken
from ..domain import ModelUsage
from ..tools.core import ToolCall, ToolDefinition
from .model_defaults import UNKNOWN_MODEL_CAPACITY


@dataclass(frozen=True, slots=True)
class ProviderMessage:
    role: str
    content: str
    tool_calls: tuple[ToolCall, ...] = ()
    tool_call_id: str | None = None
    reasoning_content: str | None = None


@dataclass(frozen=True, slots=True)
class ProviderRequest:
    model_id: str
    messages: Sequence[ProviderMessage]
    tools: Sequence[ToolDefinition] = ()
    max_output_tokens: int | None = None


@dataclass(frozen=True, slots=True)
class ModelInput:
    id: str
    display_name: str
    context_window: int = UNKNOWN_MODEL_CAPACITY.context_window
    max_output_tokens: int = UNKNOWN_MODEL_CAPACITY.max_output_tokens


@dataclass(frozen=True, slots=True)
class ModelConfig:
    id: str
    display_name: str
    enabled: bool
    supports_tools: bool
    context_window: int = UNKNOWN_MODEL_CAPACITY.context_window
    max_output_tokens: int = UNKNOWN_MODEL_CAPACITY.max_output_tokens


@dataclass(frozen=True, slots=True, repr=False)
class ProviderConfig:
    id: str
    display_name: str
    origin: str
    base_url: str
    api_key: str | None
    headers: tuple[tuple[str, str], ...]
    models: tuple[ModelConfig, ...]

    @property
    def configured(self) -> bool:
        return self.origin == "custom" or self.api_key is not None

    def header_map(self) -> dict[str, str]:
        return dict(self.headers)


@dataclass(frozen=True, slots=True)
class TextDelta:
    delta: str


@dataclass(frozen=True, slots=True)
class ReasoningDelta:
    delta: str


@dataclass(frozen=True, slots=True)
class ToolCallCompleted:
    call: ToolCall


@dataclass(frozen=True, slots=True)
class ResponseMetadata:
    """Provider response identity known before the response has completed."""

    model_id: str | None = None
    request_id: str | None = None


@dataclass(frozen=True, slots=True)
class ResponseCompleted:
    """The provider finished one response without further stream events."""

    usage: ModelUsage | None = None
    model_id: str | None = None
    request_id: str | None = None


type ProviderEvent = (
    TextDelta | ReasoningDelta | ToolCallCompleted | ResponseMetadata | ResponseCompleted
)


class ProviderAdapter(Protocol):
    def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]: ...


class ProviderResolver(Protocol):
    def resolve(self, provider_id: str) -> ProviderAdapter | None: ...
