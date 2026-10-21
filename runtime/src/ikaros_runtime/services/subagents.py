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
SUBAGENT_MAX_SUMMARY_CHARS = 4000
SUBAGENT_SUMMARY_READ_CHARS = 24000
SUBAGENT_TIMEOUT_SECONDS = 900.0


class SubagentError(ValueError):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


class SubagentService:
    """Create and drive a child conversation from inside a tool call.

    The child Run executes inline on the calling worker. It therefore never
    waits for a scheduler slot, so a parent that waits for its child cannot
    deadlock behind the worker pool limit.
    """

    def __init__(
        self,
        *,
        store: SqliteRuntimeStore,
        run_child: RunChild,
        publish: PublishEvent,
        timeout_seconds: float = SUBAGENT_TIMEOUT_SECONDS,
    ) -> None:
        self._store = store
        self._run_child = run_child
        self._publish = publish
        self._timeout_seconds = timeout_seconds
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
        template = parent_config.template
        child_title = (title or _default_title(prompt)).strip()[:120] or None
        thread, _event, _created = self._store.create_thread_once(
            child_title,
            f"subagent-thread:{request_id}",
            workspace=parent_config.workspace,
        )
        prepared = self._store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content=prompt,
            run_config_template=template,
            client_request_id=f"subagent-turn:{request_id}",
        )
        if prepared.newly_created:
            for event in prepared.initial_events:
                await self._publish(event)
        timed_out = False
        if self._store.run_status(prepared.run_id) in {"queued", "running"}:
            timed_out = await self._execute(
                prepared.run_id,
                self.depth(parent_run_id) + 1,
                cancellation,
            )
        status = self._store.run_status(prepared.run_id)
        summary, summary_truncated = self._final_summary(thread.id)
        return {
            "threadId": thread.id,
            "runId": prepared.run_id,
            "status": status,
            "summary": summary,
            "truncated": summary_truncated,
            "timedOut": timed_out,
        }

    async def _execute(
        self,
        run_id: str,
        depth: int,
        cancellation: CancellationToken,
    ) -> bool:
        self._depth_by_run[run_id] = depth
        child_token = CancellationToken()

        async def propagate() -> None:
            await cancellation.wait()
            child_token.cancel()

        watcher = asyncio.create_task(propagate())
        child_task: asyncio.Future[None] = asyncio.ensure_future(
            self._run_child(run_id, child_token)
        )
        timed_out = False
        try:
            done, _pending = await asyncio.wait({child_task}, timeout=self._timeout_seconds)
            if not done:
                timed_out = True
                child_token.cancel()
                with suppress(BaseException):
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
        return timed_out

    def _final_summary(self, thread_id: str) -> tuple[str, bool]:
        try:
            page = self._store.read_thread_slice(
                thread_id=thread_id,
                max_chars=SUBAGENT_SUMMARY_READ_CHARS,
            )
        except (LookupError, ValueError):
            return "", False
        items = page.get("items")
        if not isinstance(items, list):
            return "", False
        for item in reversed(items):
            if not isinstance(item, dict):
                continue
            if (
                item.get("role") != "assistant"
                or item.get("kind") != "message"
                or item.get("status") != "completed"
            ):
                continue
            content = str(item.get("content", ""))
            if len(content) > SUBAGENT_MAX_SUMMARY_CHARS:
                return content[:SUBAGENT_MAX_SUMMARY_CHARS].rstrip() + "\n…", True
            return content, bool(page.get("truncated"))
        return "", False


def _default_title(prompt: str) -> str:
    first = prompt.strip().splitlines()[0].strip() if prompt.strip() else ""
    return f"Subagent: {(first[:48].rstrip() or 'task')}"



__all__ = [
    "SUBAGENT_MAX_DEPTH",
    "SUBAGENT_MAX_SUMMARY_CHARS",
    "SubagentError",
    "SubagentService",
]