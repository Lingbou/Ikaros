"""Read/write gate that keeps one Run's Tool calls ordered where it matters.

Tool calls produced by a single model step run concurrently, but two properties
must survive: a mutating Tool (one that changes durable state) never interleaves
with any other Tool call, and read-only Tools may run together. This mirrors the
``parallel_execution: Arc<RwLock<()>>`` gate codex puts around Tool execution.
"""

from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager


class ToolExecutionGate:
    """Reader/writer gate with writer preference.

    Readers (read-only Tool calls) share the gate; a writer (mutating Tool call)
    needs it alone. A reader never overtakes a waiting writer, so a long stream of
    read-only calls cannot starve a write.
    """

    def __init__(self) -> None:
        self._condition = asyncio.Condition()
        self._readers = 0
        self._writer = False
        self._waiting_writers = 0

    @asynccontextmanager
    async def shared(self) -> AsyncIterator[None]:
        async with self._condition:
            while self._writer or self._waiting_writers:
                await self._condition.wait()
            self._readers += 1
        try:
            yield
        finally:
            async with self._condition:
                self._readers -= 1
                if self._readers == 0:
                    self._condition.notify_all()

    @asynccontextmanager
    async def exclusive(self) -> AsyncIterator[None]:
        async with self._condition:
            self._waiting_writers += 1
            try:
                while self._writer or self._readers:
                    await self._condition.wait()
            except BaseException:
                self._waiting_writers -= 1
                self._condition.notify_all()
                raise
            self._waiting_writers -= 1
            self._writer = True
        try:
            yield
        finally:
            async with self._condition:
                self._writer = False
                self._condition.notify_all()


__all__ = ["ToolExecutionGate"]