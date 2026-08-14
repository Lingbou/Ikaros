from __future__ import annotations

from typing import Any

from ..errors import InvalidParamsError
from ..storage import SqliteRuntimeStore


class UsageService:
    def __init__(self, store: SqliteRuntimeStore) -> None:
        self._store = store

    def read(self, params: dict[str, Any]) -> dict[str, Any]:
        if params:
            raise InvalidParamsError("usage.read does not accept parameters")
        return self._store.read_usage().to_wire()


__all__ = ["UsageService"]
