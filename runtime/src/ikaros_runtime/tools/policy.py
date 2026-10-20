"""Execution policies for built-in Runtime Tools."""

from __future__ import annotations

from collections.abc import Mapping
from typing import Any


class FullAccessPolicy:
    name = "full_access"

    def authorize(self, tool_name: str, arguments: Mapping[str, Any]) -> None:
        del tool_name, arguments


__all__ = ["FullAccessPolicy"]
