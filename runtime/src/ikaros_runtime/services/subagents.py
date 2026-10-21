"""Spawn a child Thread + Run for a self-contained sub-task."""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from contextlib import suppress
from typing import Any

from ..cancellation import CancellationToken
from ..domain import JsonObject
from ..storage.store import SqliteRuntimeStore

RunChild = Callable[[str, CancellationToken], Awaitable[None]]
PublishEvent = Callable[[Any], Awaitable[None]]

SUBAGENT_MAX_DEPTH = 1


class SubagentError(ValueError):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


class SubagentService:
    """Create and drive a child conversation from inside a tool call.

    The child Run executes inline on the calling worker. It never waits for a
    scheduler slot, so a parent that waits for its child cannot deadlock behind
    the worker pool. There is deliberately no wall-clock timeout: long child
    work is stopped by cancellation only (the same policy hermes delegation
    documents for heavy work).
    """

    def __init__(
        self,
        *,
        store: SqliteRuntimeStore,
        run_child: RunChild,
        publish: PublishEvent,
    ) -> None:
        self._store = store
        self._run_child = run_child
        self._publish = publish
        self._depth_by_run: dict[str, int] = {}

    def depth(self, run_id: str) -> int:
        return self._depth_by_run.get(run_id, 0)

    async def spawn(
        self,
        *,
        parent_run_id: str,
        request_id: str,
        prompt: str,
        title: str | None,
        cancellation: CancellationToken,
    ) -> JsonObject:
        if self.depth(parent_run_id) >= SUBAGENT_MAX_DEPTH:
            raise SubagentError(
                "subagent_depth_limit",
                "Subagents cannot spawn further subagents.",
            )
        parent_config = self._store.get_run_config(parent_run_id)
        child_title = (title or _default_title(prompt)).strip() or None
        thread, _event, _created = self._store.create_thread_once(
            child_title,
            f"subagent-thread:{request_id}",
            workspace=parent_config.workspace,
        )
        prepared = self._store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content=prompt,
            run_config_template=parent_config.template,
            client_request_id=f"subagent-turn:{request_id}",
        )
        if prepared.newly_created:
            for event in prepared.initial_events:
                await self._publish(event)
        if self._store.run_status(prepared.run_id) in {"queued", "running"}:
            await self._execute(
                prepared.run_id,
                self.depth(parent_run_id) + 1,
                cancellation,
            )
        return {
            "threadId": thread.id,
            "runId": prepared.run_id,
            "status": self._store.run_status(prepared.run_id),
            "summary": self._store.final_assistant_message(thread.id) or "",
        }

    async def _execute(
        self,
        run_id: str,
        depth: int,
        cancellation: CancellationToken,
    ) -> None:
        self._depth_by_run[run_id] = depth
        child_token = CancellationToken()

        async def propagate() -> None:
            await cancellation.wait()
            child_token.cancel()

        watcher = asyncio.create_task(propagate())
        child_task: asyncio.Future[None] = asyncio.ensure_future(
            self._run_child(run_id, child_token)
        )
        try:
            await child_task
        except asyncio.CancelledError:
            child_token.cancel()
            with suppress(BaseException):
                await child_task
            raise
        finally:
            watcher.cancel()
            with suppress(asyncio.CancelledError):
                await watcher
            self._depth_by_run.pop(run_id, None)


def _default_title(prompt: str) -> str:
    first = prompt.strip().splitlines()[0].strip() if prompt.strip() else ""
    return f"Subagent: {(first[:48].rstrip() or 'task')}"


__all__ = ["SUBAGENT_MAX_DEPTH", "SubagentError", "SubagentService"]