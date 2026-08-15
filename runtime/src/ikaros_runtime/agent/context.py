from __future__ import annotations

from collections.abc import Sequence

from ..domain import ContextItem
from ..providers.base import ProviderMessage, ProviderRequest
from ..tools.core import ToolCall
from .model_input import ModelInputPlanV1


class ContextBuilder:
    """Render one provider-neutral model input plan into a Provider request."""

    def build_request(self, plan: ModelInputPlanV1) -> ProviderRequest:
        """Lower ordered plan blocks without changing their content."""

        if plan.context_data:
            raise RuntimeError(
                "context data lowering is not implemented for model input plan version 1"
            )
        return ProviderRequest(
            model_id=plan.model_id,
            messages=(
                *(
                    ProviderMessage(role="system", content=block.content)
                    for block in plan.instructions
                ),
                *_provider_messages(plan.messages),
            ),
            tools=plan.tools,
        )


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
