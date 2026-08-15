from __future__ import annotations

from collections.abc import Sequence

from ..domain import ContextItem
from ..providers.base import ProviderMessage, ProviderRequest
from ..tools.core import ToolCall, ToolDefinition

type SystemMessageInput = str | ProviderMessage

_OUTPUT_STYLE_SYSTEM_MESSAGE = ProviderMessage(
    role="system",
    content=(
        "Use a restrained, professional response style. Do not use emoji or decorative "
        "Unicode symbols unless the user explicitly asks for them. Never use them for "
        "decoration, headings, or list markers. Use Markdown hyphen bullets (`- item`) "
        "for ordinary unordered lists; the client will render them as simple round bullets."
    ),
)


class ContextBuilder:
    """Build one provider request from persisted conversation context."""

    def build_request(
        self,
        *,
        model_id: str,
        items: Sequence[ContextItem],
        tools: Sequence[ToolDefinition] = (),
        extra_system: Sequence[SystemMessageInput] = (),
    ) -> ProviderRequest:
        """Build a request with stable system-message and conversation ordering.

        ``extra_system`` is deliberately supplied per request rather than discovered here.
        A caller can therefore freeze future Run-specific context, such as a Skill catalog,
        once and reuse the same ordered fragments for every model step in that Run.
        """

        system_messages = tuple(_system_message(value) for value in extra_system)
        return ProviderRequest(
            model_id=model_id,
            messages=(
                _OUTPUT_STYLE_SYSTEM_MESSAGE,
                *system_messages,
                *_provider_messages(items),
            ),
            tools=tuple(tools),
        )


def _system_message(value: SystemMessageInput) -> ProviderMessage:
    if isinstance(value, str):
        return ProviderMessage(role="system", content=value)
    if (
        value.role != "system"
        or value.tool_calls
        or value.tool_call_id is not None
        or value.reasoning_content is not None
    ):
        raise ValueError("extra system messages must be plain system messages")
    return value


def _provider_messages(items: Sequence[ContextItem]) -> tuple[ProviderMessage, ...]:
    messages: list[ProviderMessage] = []
    index = 0
    while index < len(items):
        item = items[index]
        if item.kind == "message":
            if item.role not in {"user", "assistant"}:
                raise RuntimeError("message context item has an invalid role")
            step_id = item.data.get("stepId")
            if item.role == "assistant" and isinstance(step_id, str):
                index += 1
                calls, index, reasoning_content = _tool_calls_for_step(items, index, step_id)
                if not calls:
                    # Calls from a failed/cancelled Run are omitted from later context.
                    # Preserve its narration as plain text instead of breaking every
                    # future context rebuild.
                    messages.append(ProviderMessage(role="assistant", content=item.content))
                    continue
                messages.append(
                    ProviderMessage(
                        role="assistant",
                        content=item.content,
                        tool_calls=tuple(calls),
                        reasoning_content=reasoning_content,
                    )
                )
                continue
            messages.append(ProviderMessage(role=item.role, content=item.content))
            index += 1
            continue
        if item.kind == "tool_call":
            step_id = item.data.get("stepId")
            if not isinstance(step_id, str):
                raise RuntimeError("tool call context item has no step ID")
            calls, index, reasoning_content = _tool_calls_for_step(items, index, step_id)
            messages.append(
                ProviderMessage(
                    role="assistant",
                    content="",
                    tool_calls=tuple(calls),
                    reasoning_content=reasoning_content,
                )
            )
            continue
        if item.kind == "tool_result":
            call_id = item.data.get("callId")
            if not isinstance(call_id, str):
                raise RuntimeError("tool result context item is invalid")
            messages.append(
                ProviderMessage(
                    role="tool",
                    content=item.content,
                    tool_call_id=call_id,
                )
            )
            index += 1
            continue
        raise RuntimeError(f"unknown context item kind: {item.kind}")
    return tuple(messages)


def _reasoning_content(item: ContextItem) -> str | None:
    value = item.data.get("reasoningContent")
    if value is None:
        return None
    if not isinstance(value, str):
        raise RuntimeError("tool call reasoning context is invalid")
    return value


def _tool_calls_for_step(
    items: Sequence[ContextItem],
    index: int,
    step_id: str,
) -> tuple[list[ToolCall], int, str | None]:
    calls: list[ToolCall] = []
    reasoning_content: str | None = None
    while index < len(items):
        candidate = items[index]
        if candidate.kind != "tool_call" or candidate.data.get("stepId") != step_id:
            break
        call_id = candidate.data.get("callId")
        tool_name = candidate.data.get("toolName")
        arguments = candidate.data.get("arguments")
        if (
            not isinstance(call_id, str)
            or not isinstance(tool_name, str)
            or not isinstance(arguments, dict)
        ):
            raise RuntimeError("tool call context item is invalid")
        candidate_reasoning = _reasoning_content(candidate)
        if candidate_reasoning is not None:
            if reasoning_content is not None:
                raise RuntimeError("tool call step contains duplicate reasoning context")
            reasoning_content = candidate_reasoning
        calls.append(ToolCall(call_id, tool_name, arguments))
        index += 1
    return calls, index, reasoning_content
