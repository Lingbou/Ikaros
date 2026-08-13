"""Ordered fan-out of persisted Runtime events to connected clients."""

from __future__ import annotations

import asyncio
from collections.abc import Callable

from ..domain import JournalEvent

EventSink = Callable[[JournalEvent], None]


class EventHub:
    def __init__(self, *, next_seq: int) -> None:
        if next_seq < 1:
            raise ValueError("next event sequence must be positive")
        self._sinks: set[EventSink] = set()
        self._pending: dict[int, JournalEvent] = {}
        self._next_seq = next_seq
        self._lock = asyncio.Lock()

    def subscribe(self, sink: EventSink) -> Callable[[], None]:
        self._sinks.add(sink)

        def unsubscribe() -> None:
            self._sinks.discard(sink)

        return unsubscribe

    async def publish(self, event: JournalEvent) -> None:
        async with self._lock:
            if event.seq < self._next_seq or event.seq in self._pending:
                return
            self._pending[event.seq] = event
            while ready := self._pending.pop(self._next_seq, None):
                self._next_seq += 1
                for sink in tuple(self._sinks):
                    try:
                        sink(ready)
                    except Exception:
                        self._sinks.discard(sink)


__all__ = ["EventHub"]
