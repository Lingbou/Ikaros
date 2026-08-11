from __future__ import annotations

import json
from collections.abc import Sequence
from dataclasses import dataclass
from typing import Any, Protocol

from .cancellation import CancellationToken, RunCancelled
from .domain import JsonObject
from .policy import ExecutionPolicy


@dataclass(frozen=True, slots=True)
class ToolDefinition:
    name: str
    description: str
    input_schema: JsonObject


@dataclass(frozen=True, slots=True)
class ToolCall:
    id: str
    name: str
    arguments: JsonObject


@dataclass(frozen=True, slots=True)
class ToolResult:
    tool_call_id: str
    tool_name: str
    ok: bool
    output: str
    details: JsonObject
    cancelled: bool = False

    def to_wire(self) -> JsonObject:
        return {
            "toolCallId": self.tool_call_id,
            "toolName": self.tool_name,
            "ok": self.ok,
            "output": self.output,
            "cancelled": self.cancelled,
            **self.details,
        }

    def to_model_content(self) -> str:
        return json.dumps(self.to_wire(), ensure_ascii=False, separators=(",", ":"))

    @classmethod
    def rejected(cls, call: ToolCall, *, code: str, message: str) -> ToolResult:
        return cls(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=False,
            output=message,
            details={
                "stdout": "",
                "stderr": "",
                "exitCode": None,
                "durationMs": 0,
                "timedOut": False,
                "truncated": False,
                "errorCode": code,
            },
        )


class ToolExecutionCancelled(RunCancelled):
    def __init__(self, result: ToolResult) -> None:
        super().__init__("tool execution was cancelled")
        self.result = result


class Tool(Protocol):
    definition: ToolDefinition

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
    ) -> ToolResult: ...


class ToolRegistry:
    def __init__(self, tools: Sequence[Tool]) -> None:
        self._tools: dict[str, Tool] = {}
        for tool in tools:
            name = tool.definition.name
            if not name or name in self._tools:
                raise ValueError(f"duplicate or empty tool name: {name!r}")
            self._tools[name] = tool

    @property
    def definitions(self) -> tuple[ToolDefinition, ...]:
        return tuple(tool.definition for tool in self._tools.values())

    def resolve(self, name: str) -> Tool | None:
        return self._tools.get(name)


class ToolExecutor:
    def __init__(self, registry: ToolRegistry, policy: ExecutionPolicy) -> None:
        self._registry = registry
        self._policy = policy

    @property
    def definitions(self) -> tuple[ToolDefinition, ...]:
        return self._registry.definitions

    @property
    def policy_name(self) -> str:
        return self._policy.name

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
    ) -> ToolResult:
        cancellation.raise_if_cancelled()
        tool = self._registry.resolve(call.name)
        if tool is None:
            return ToolResult.rejected(
                call,
                code="unknown_tool",
                message=f"Tool is not registered: {call.name}",
            )
        self._policy.authorize(call.name, call.arguments)
        cancellation.raise_if_cancelled()
        return await tool.execute(call, cancellation=cancellation)


def require_exact_arguments(
    arguments: JsonObject,
    *,
    required: set[str],
    optional: set[str],
) -> str | None:
    keys = set(arguments)
    missing = sorted(required - keys)
    unknown = sorted(keys - required - optional)
    if missing or unknown:
        return f"tool arguments mismatch; missing={missing}, unknown={unknown}"
    return None


def is_json_integer(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)
