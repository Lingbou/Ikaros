from __future__ import annotations

import asyncio

from .errors import RunCancelled as RunCancelled


class CancellationToken:
    def __init__(self) -> None:
        self._cancelled = asyncio.Event()

    @property
    def is_cancelled(self) -> bool:
        return self._cancelled.is_set()

    def cancel(self) -> None:
        self._cancelled.set()

    def raise_if_cancelled(self) -> None:
        if self.is_cancelled:
            raise RunCancelled

    async def wait(self) -> None:
        await self._cancelled.wait()

    async def sleep(self, delay: float) -> None:
        self.raise_if_cancelled()
        if delay <= 0:
            await asyncio.sleep(0)
            self.raise_if_cancelled()
            return
        try:
            await asyncio.wait_for(self._cancelled.wait(), timeout=delay)
        except TimeoutError:
            return
        raise RunCancelled
