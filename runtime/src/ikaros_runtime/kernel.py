from __future__ import annotations

from typing import Any

from .storage import SqliteRuntimeStore


class InvalidParamsError(ValueError):
    """The client supplied invalid JSON-RPC method parameters."""


class RuntimeKernel:
    def __init__(self, store: SqliteRuntimeStore) -> None:
        self._store = store

    def create_thread(self, params: dict[str, Any]) -> dict[str, Any]:
        unknown = set(params) - {"title"}
        if unknown:
            raise InvalidParamsError(f"unknown thread.create parameters: {sorted(unknown)}")
        title = params.get("title")
        if title is not None:
            if not isinstance(title, str):
                raise InvalidParamsError("title must be a string or null")
            title = title.strip()
            if not title:
                title = None
            elif len(title) > 200:
                raise InvalidParamsError("title must not exceed 200 characters")
        thread, event = self._store.create_thread(title)
        return {"thread": thread.to_wire(), "event": event.to_wire()}

    def list_threads(self, params: dict[str, Any]) -> dict[str, Any]:
        if params:
            raise InvalidParamsError("thread.list does not accept parameters")
        return {"threads": [thread.to_wire() for thread in self._store.list_threads()]}

    def replay_events(self, params: dict[str, Any]) -> dict[str, Any]:
        unknown = set(params) - {"afterSeq", "limit"}
        if unknown:
            raise InvalidParamsError(f"unknown event.replay parameters: {sorted(unknown)}")
        after_seq = params.get("afterSeq", 0)
        limit = params.get("limit", 500)
        if not isinstance(after_seq, int) or isinstance(after_seq, bool) or after_seq < 0:
            raise InvalidParamsError("afterSeq must be a non-negative integer")
        if not isinstance(limit, int) or isinstance(limit, bool) or not 1 <= limit <= 1000:
            raise InvalidParamsError("limit must be an integer between 1 and 1000")
        events, latest_seq = self._store.replay_events(after_seq, limit)
        next_after_seq = events[-1].seq if events else after_seq
        return {
            "events": [event.to_wire() for event in events],
            "latestSeq": latest_seq,
            "nextAfterSeq": next_after_seq,
            "hasMore": next_after_seq < latest_seq,
        }
