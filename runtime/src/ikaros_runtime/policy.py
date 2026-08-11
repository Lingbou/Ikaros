from __future__ import annotations

from collections.abc import Mapping
from typing import Any, Protocol


class ExecutionPolicy(Protocol):
    name: str

    def authorize(self, tool_name: str, arguments: Mapping[str, Any]) -> None: ...


class FullAccessPolicy:
    name = "full_access"

    def authorize(self, tool_name: str, arguments: Mapping[str, Any]) -> None:
        del tool_name, arguments
