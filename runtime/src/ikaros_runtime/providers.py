from __future__ import annotations

import uuid
from collections.abc import AsyncIterator, Sequence
from dataclasses import dataclass
from typing import Protocol

from .cancellation import CancellationToken
from .json_codec import loads as json_loads
from .tools import ToolCall, ToolDefinition


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
class ResponseCompleted:
    """The provider finished one response without further stream events."""


type ProviderEvent = TextDelta | ReasoningDelta | ToolCallCompleted | ResponseCompleted


class ProviderAdapter(Protocol):
    def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]: ...


class ProviderResolver(Protocol):
    def resolve(self, provider_id: str) -> ProviderAdapter | None: ...


class ScriptedProvider:
    id = "scripted"
    model_id = "scripted-v1"

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        if request.model_id != self.model_id:
            raise ValueError("unknown scripted model")
        users = [
            (index, message.content)
            for index, message in enumerate(request.messages)
            if message.role == "user"
        ]
        assistants = [
            message.content
            for message in request.messages
            if message.role == "assistant" and not message.tool_calls
        ]
        if not users:
            raise ValueError("scripted provider requires a user message")

        last_user_index, last_user = users[-1]
        command_prefix = "/process.run"
        tool_results = [
            message for message in request.messages[last_user_index + 1 :] if message.role == "tool"
        ]
        if last_user.startswith(command_prefix) and not tool_results:
            command = last_user.removeprefix(command_prefix).strip()
            available = {tool.name for tool in request.tools}
            if command and "process_run" in available:
                yield ToolCallCompleted(
                    ToolCall(
                        id=f"call_{uuid.uuid4().hex}",
                        name="process_run",
                        arguments={"command": command},
                    )
                )
                yield ResponseCompleted()
                return
            response = "Usage: /process.run <command>"
        elif tool_results:
            response = _scripted_tool_summary(tool_results[-1].content)
        elif len(users) == 1:
            response = f"Scripted response to: {last_user}"
        else:
            previous_assistant = assistants[-1] if assistants else "(none)"
            response = (
                f"Previous user: {users[-2][1]}\n"
                f"Previous assistant: {previous_assistant}\n"
                f"Current user: {last_user}"
            )

        for start in range(0, len(response), 12):
            await cancellation.sleep(0.005)
            yield TextDelta(response[start : start + 12])
        yield ResponseCompleted()


def _scripted_tool_summary(content: str) -> str:
    try:
        result = json_loads(content)
    except ValueError:
        return f"Command result: {content}"
    if not isinstance(result, dict):
        return f"Command result: {content}"
    output = result.get("output")
    exit_code = result.get("exitCode")
    if not isinstance(output, str):
        output = ""
    if result.get("cancelled") is True:
        return f"Command was cancelled.\n{output}".rstrip()
    if result.get("timedOut") is True:
        return f"Command timed out.\n{output}".rstrip()
    return f"Command exited with code {exit_code}.\n{output}".rstrip()
