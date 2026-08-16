from __future__ import annotations

import json
import uuid
from pathlib import Path
from typing import cast
from unittest.mock import patch

import httpx
import pytest

import ikaros_runtime.memory.store as memory_store_module
from ikaros_runtime.agent import AgentLoop
from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.domain import JournalEvent, SkillDescriptor
from ikaros_runtime.identity import load_identity_core
from ikaros_runtime.memory import MemoryRetrieverV1, MemoryScope, SqliteMemoryStore
from ikaros_runtime.providers.base import ModelConfig, ProviderConfig
from ikaros_runtime.providers.openai_compatible.adapter import OpenAICompatibleAdapter
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools.core import (
    ToolCall,
    ToolDefinition,
    ToolExecutor,
    ToolRegistry,
    ToolResult,
)
from ikaros_runtime.tools.policy import FullAccessPolicy

from .helpers import prepare_turn

_FIXTURE_PATH = Path(__file__).parent / "fixtures" / "model_input_plan_v1_openai_bodies.json"


class GoldenProcessTool:
    definition = ToolDefinition(
        name="process_run",
        description="Run one deterministic test process.",
        input_schema={
            "type": "object",
            "properties": {"command": {"type": "string"}},
            "required": ["command"],
            "additionalProperties": False,
        },
    )

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
        default_cwd: str | None = None,
    ) -> ToolResult:
        cancellation.raise_if_cancelled()
        del default_cwd
        command = call.arguments.get("command")
        if not isinstance(command, str):
            raise AssertionError("golden process command must be a string")
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output=f"{command}-output",
            details={"durationMs": 0, "truncated": False},
        )


def _sse(value: object) -> bytes:
    return f"data: {json.dumps(value, separators=(',', ':'))}\n\n".encode()


def _response_body(ordinal: int) -> bytes:
    if ordinal == 1:
        chunks = [
            _sse(
                {
                    "choices": [
                        {
                            "index": 0,
                            "delta": {
                                "reasoning_content": "choose two commands",
                                "content": "I will inspect both.",
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call-1",
                                        "type": "function",
                                        "function": {
                                            "name": "process_run",
                                            "arguments": '{"command":"first"}',
                                        },
                                    },
                                    {
                                        "index": 1,
                                        "id": "call-2",
                                        "type": "function",
                                        "function": {
                                            "name": "process_run",
                                            "arguments": '{"command":"second"}',
                                        },
                                    },
                                ],
                            },
                            "finish_reason": "tool_calls",
                        }
                    ]
                }
            ),
            b"data: [DONE]\n\n",
        ]
    else:
        chunks = [
            _sse(
                {
                    "choices": [
                        {
                            "index": 0,
                            "delta": {"content": "Both commands completed."},
                            "finish_reason": "stop",
                        }
                    ]
                }
            ),
            b"data: [DONE]\n\n",
        ]
    return b"".join(chunks)


@pytest.mark.asyncio
async def test_model_input_plan_v1_preserves_complete_openai_wire_body(tmp_path: Path) -> None:
    captured_bodies: list[dict[str, object]] = []

    async def handler(incoming: httpx.Request) -> httpx.Response:
        captured_bodies.append(json.loads(incoming.content))
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            content=_response_body(len(captured_bodies)),
        )

    configured = ProviderConfig(
        id="custom",
        display_name="Custom",
        origin="custom",
        base_url="https://provider.invalid/v1",
        api_key="test-provider-key",
        headers=(),
        models=(
            ModelConfig(
                id="model",
                display_name="Model",
                enabled=True,
                supports_tools=True,
            ),
        ),
    )
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        identity_core = load_identity_core()
        thread, _event = store.create_thread("Golden model input")
        executor = ToolExecutor(
            ToolRegistry((GoldenProcessTool(),)),
            FullAccessPolicy(),
        )
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="Run both commands.",
            provider_id="custom",
            model_id="model",
            skills=(
                SkillDescriptor(
                    name="demo",
                    description="Use the demo workflow.",
                    location=r"C:\Users\example\.ikaros\skills\demo\SKILL.md",
                ),
            ),
            tools=executor.definitions,
            identity_core=identity_core,
        )
        with patch.object(
            memory_store_module.uuid,
            "uuid4",
            return_value=uuid.UUID(int=1),
        ):
            memory_store.create_memory_once(
                kind="preference",
                scope=MemoryScope("global", None),
                content="Commands should remain concise and deterministic.",
                client_request_id="golden-model-input-memory",
            )
        async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
            adapter = OpenAICompatibleAdapter(configured, client=client, max_retries=0)
            await AgentLoop(
                store,
                {"custom": adapter},
                publish,
                tool_executor=executor,
                identity_core=identity_core,
                memory_retriever=MemoryRetrieverV1(memory_store),
            ).run(prepared.run_id, CancellationToken())

        expected = json.loads(_FIXTURE_PATH.read_text(encoding="utf-8"))
        assert captured_bodies == expected
        message_payloads = [body["messages"] for body in captured_bodies]
        serialized_messages = json.dumps(message_payloads, ensure_ascii=False)
        for body in captured_bodies:
            messages = cast(list[object], body["messages"])
            assert messages[0] == {"role": "system", "content": identity_core.content}
            assert (
                sum(
                    message == {"role": "system", "content": identity_core.content}
                    for message in messages
                )
                == 1
            )
        assert "Run one deterministic test process." not in serialized_messages
        assert "additionalProperties" not in serialized_messages
        assert [event.payload["status"] for event in events if event.type == "run.settled"] == [
            "completed"
        ]
    finally:
        memory_store.close()
        store.close()
