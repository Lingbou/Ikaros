from __future__ import annotations

from dataclasses import replace

import pytest

from ikaros_runtime.agent import ContextBuilder, ContextDataBlockV1, ModelInputPlanner
from ikaros_runtime.domain import ContextItem
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

    plan = ModelInputPlanner().build_plan(
        model_id="model-1",
        items=items,
        tools=(tool,),
    )
    request = ContextBuilder().build_request(plan)

    assert request.model_id == "model-1"
    assert request.tools == (tool,)
    assert [message.role for message in request.messages] == [
        "system",
        "user",
        "assistant",
        "tool",
        "tool",
    ]
    assert "Markdown hyphen bullets (`- item`)" in request.messages[0].content
    assistant = request.messages[2]
    assert assistant.content == "I will run them."
    assert assistant.reasoning_content == "choose commands"
    assert assistant.tool_calls == (
        ToolCall("call-1", "process_run", {"command": "first"}),
        ToolCall("call-2", "process_run", {"command": "second"}),
    )
    assert request.messages[3].tool_call_id == "call-1"
    assert request.messages[4].tool_call_id == "call-2"


def test_context_builder_preserves_narration_when_a_step_has_no_replayable_calls() -> None:
    plan = ModelInputPlanner().build_plan(
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
    request = ContextBuilder().build_request(plan)

    assert [(message.role, message.content) for message in request.messages[1:]] == [
        ("assistant", "The interrupted narration remains visible."),
        ("user", "continue"),
    ]
    assert request.messages[1].tool_calls == ()


def test_context_builder_rejects_context_data_until_safe_lowering_is_defined() -> None:
    plan = ModelInputPlanner().build_plan(model_id="model-1", items=())
    plan = replace(
        plan,
        context_data=(
            ContextDataBlockV1(
                id="future-memory",
                version=1,
                source="memory:example@1",
                authority="contextual_data",
                scope="global",
                lifetime="run",
                content="Untrusted context data",
            ),
        ),
    )

    with pytest.raises(RuntimeError, match="context data lowering is not implemented"):
        ContextBuilder().build_request(plan)


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
        plan = ModelInputPlanner().build_plan(model_id="model-1", items=items)
        ContextBuilder().build_request(plan)
