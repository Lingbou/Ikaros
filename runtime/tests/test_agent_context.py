from __future__ import annotations

import pytest

from ikaros_runtime.agent.context import ContextBuilder
from ikaros_runtime.domain import ContextItem
from ikaros_runtime.providers.base import ProviderMessage
from ikaros_runtime.tools.core import ToolCall, ToolDefinition


def test_context_builder_preserves_system_context_and_tool_step_order() -> None:
    tool = ToolDefinition(
        name="process_run",
        description="Run a process.",
        input_schema={"type": "object"},
    )
    items = (
        ContextItem(kind="message", role="user", content="run both", data={}),
        ContextItem(
            kind="message",
            role="assistant",
            content="I will run them.",
            data={"stepId": "step-1"},
        ),
        ContextItem(
            kind="tool_call",
            role=None,
            content="",
            data={
                "stepId": "step-1",
                "callId": "call-1",
                "toolName": "process_run",
                "arguments": {"command": "first"},
                "reasoningContent": "choose commands",
            },
        ),
        ContextItem(
            kind="tool_call",
            role=None,
            content="",
            data={
                "stepId": "step-1",
                "callId": "call-2",
                "toolName": "process_run",
                "arguments": {"command": "second"},
            },
        ),
        ContextItem(
            kind="tool_result",
            role=None,
            content="first result",
            data={"callId": "call-1"},
        ),
        ContextItem(
            kind="tool_result",
            role=None,
            content="second result",
            data={"callId": "call-2"},
        ),
    )

    request = ContextBuilder().build_request(
        model_id="model-1",
        items=items,
        tools=(tool,),
        extra_system=(
            "first frozen fragment",
            ProviderMessage(role="system", content="second frozen fragment"),
        ),
    )

    assert request.model_id == "model-1"
    assert request.tools == (tool,)
    assert [message.role for message in request.messages] == [
        "system",
        "system",
        "system",
        "user",
        "assistant",
        "tool",
        "tool",
    ]
    assert "Markdown hyphen bullets (`- item`)" in request.messages[0].content
    assert [message.content for message in request.messages[1:3]] == [
        "first frozen fragment",
        "second frozen fragment",
    ]
    assistant = request.messages[4]
    assert assistant.content == "I will run them."
    assert assistant.reasoning_content == "choose commands"
    assert assistant.tool_calls == (
        ToolCall("call-1", "process_run", {"command": "first"}),
        ToolCall("call-2", "process_run", {"command": "second"}),
    )
    assert request.messages[5].tool_call_id == "call-1"
    assert request.messages[6].tool_call_id == "call-2"


def test_context_builder_preserves_narration_when_a_step_has_no_replayable_calls() -> None:
    request = ContextBuilder().build_request(
        model_id="model-1",
        items=(
            ContextItem(
                kind="message",
                role="assistant",
                content="The interrupted narration remains visible.",
                data={"stepId": "interrupted-step"},
            ),
            ContextItem(kind="message", role="user", content="continue", data={}),
        ),
    )

    assert [(message.role, message.content) for message in request.messages[1:]] == [
        ("assistant", "The interrupted narration remains visible."),
        ("user", "continue"),
    ]
    assert request.messages[1].tool_calls == ()


def test_context_builder_rejects_non_system_extra_message() -> None:
    with pytest.raises(ValueError, match="plain system messages"):
        ContextBuilder().build_request(
            model_id="model-1",
            items=(),
            extra_system=(ProviderMessage(role="user", content="not a system message"),),
        )


def test_context_builder_rejects_duplicate_reasoning_for_one_tool_step() -> None:
    items = (
        ContextItem(
            kind="tool_call",
            role=None,
            content="",
            data={
                "stepId": "step-1",
                "callId": "call-1",
                "toolName": "process_run",
                "arguments": {"command": "first"},
                "reasoningContent": "first reasoning",
            },
        ),
        ContextItem(
            kind="tool_call",
            role=None,
            content="",
            data={
                "stepId": "step-1",
                "callId": "call-2",
                "toolName": "process_run",
                "arguments": {"command": "second"},
                "reasoningContent": "duplicate reasoning",
            },
        ),
    )

    with pytest.raises(RuntimeError, match="duplicate reasoning context"):
        ContextBuilder().build_request(model_id="model-1", items=items)
