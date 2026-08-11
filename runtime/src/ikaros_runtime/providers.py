from __future__ import annotations

from collections.abc import AsyncIterator, Sequence
from dataclasses import dataclass
from typing import Protocol

from .cancellation import CancellationToken


@dataclass(frozen=True, slots=True)
class ProviderMessage:
    role: str
    content: str


@dataclass(frozen=True, slots=True)
class ProviderRequest:
    model_id: str
    messages: Sequence[ProviderMessage]


class ProviderAdapter(Protocol):
    def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[str]: ...


class ScriptedProvider:
    id = "scripted"
    model_id = "scripted-v1"

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[str]:
        if request.model_id != self.model_id:
            raise ValueError("unknown scripted model")
        users = [message.content for message in request.messages if message.role == "user"]
        assistants = [
            message.content for message in request.messages if message.role == "assistant"
        ]
        if not users:
            raise ValueError("scripted provider requires a user message")

        if len(users) == 1:
            response = f"Scripted response to: {users[-1]}"
        else:
            previous_assistant = assistants[-1] if assistants else "(none)"
            response = (
                f"Previous user: {users[-2]}\n"
                f"Previous assistant: {previous_assistant}\n"
                f"Current user: {users[-1]}"
            )

        for start in range(0, len(response), 12):
            await cancellation.sleep(0.005)
            yield response[start : start + 12]
