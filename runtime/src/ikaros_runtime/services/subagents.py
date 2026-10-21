"""Spawn a child Thread + Run for a self-contained sub-task."""

from __future__ import annotations

import asyncio
import os
from collections.abc import Awaitable, Callable, Mapping
from contextlib import suppress
from typing import Any

from ..cancellation import CancellationToken
from ..domain import JsonObject
from ..run_input import RunConfig
from ..storage.store import SqliteRuntimeStore

RunChild = Callable[[str, CancellationToken], Awaitable[None]]
EnqueueRun = Callable[[str], Awaitable[None]]
CancelRun = Callable[[str], Awaitable[object]]
PublishEvent = Callable[[Any], Awaitable[None]]

SUBAGENT_MAX_DEPTH_ENV = "IKAROS_SUBAGENT_MAX_DEPTH"
DEFAULT_SUBAGENT_MAX_DEPTH = 2
MIN_SUBAGENT_MAX_DEPTH = 1
MAX_SUBAGENT_MAX_DEPTH = 8
SUBAGENT_MAX_ACTIVE_PER_RUN = 4


def subagent_depth_limit(environment: Mapping[str, str] | None = None) -> int:
    """Read and validate the configured nesting limit for child Runs."""

    source = os.environ if environment is None else environment
    raw = source.get(SUBAGENT_MAX_DEPTH_ENV)
    if raw is None:
        return DEFAULT_SUBAGENT_MAX_DEPTH
    try:
        value = int(raw.strip())
    except ValueError:
        raise ValueError(
            f"{SUBAGENT_MAX_DEPTH_ENV} must be an integer between "
            f"{MIN_SUBAGENT_MAX_DEPTH} and {MAX_SUBAGENT_MAX_DEPTH}"
        ) from None
    if not MIN_SUBAGENT_MAX_DEPTH <= value <= MAX_SUBAGENT_MAX_DEPTH:
        raise ValueError(
            f"{SUBAGENT_MAX_DEPTH_ENV} must be an integer between "
            f"{MIN_SUBAGENT_MAX_DEPTH} and {MAX_SUBAGENT_MAX_DEPTH}"
        )
    return value


class SubagentError(ValueError):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


class SubagentService:
    """Create and drive child conversations from inside a tool call.

    ``wait=true`` keeps the original inline execution path: the child Run runs
    on the calling worker, so a parent that waits for its child cannot deadlock
    behind the worker pool. ``wait=false`` hands the child Run to the Runtime
    scheduler and returns immediately; a normal parent completion deliberately
    leaves the child running, while parent cancellation still propagates to the
    child. There is no wall-clock timeout: long child work is stopped by
    cancellation only (the same policy hermes delegation documents for heavy
    work).
    """

    def __init__(
        self,
        *,
        store: SqliteRuntimeStore,
        run_child: RunChild,
        publish: PublishEvent,
        enqueue: EnqueueRun | None = None,
        cancel: CancelRun | None = None,
        max_depth: int | None = None,
        max_active_per_run: int = SUBAGENT_MAX_ACTIVE_PER_RUN,
    ) -> None:
        resolved_depth = subagent_depth_limit() if max_depth is None else max_depth
        if (
            isinstance(resolved_depth, bool)
            or not isinstance(resolved_depth, int)
            or not MIN_SUBAGENT_MAX_DEPTH <= resolved_depth <= MAX_SUBAGENT_MAX_DEPTH
        ):
            raise ValueError(
                f"max_depth must be an integer between "
                f"{MIN_SUBAGENT_MAX_DEPTH} and {MAX_SUBAGENT_MAX_DEPTH}"
            )
        if (
            isinstance(max_active_per_run, bool)
            or not isinstance(max_active_per_run, int)
            or max_active_per_run < 1
        ):
            raise ValueError("max_active_per_run must be a positive integer")
        self._store = store
        self._run_child = run_child
        self._publish = publish
        self._enqueue = enqueue
        self._cancel = cancel
        self._max_depth = resolved_depth
        self._max_active_per_run = max_active_per_run
        self._depth_by_run: dict[str, int] = {}
        self._active_by_parent: dict[str, set[str]] = {}
        self._parent_by_run: dict[str, str] = {}
        self._run_by_thread: dict[str, str] = {}
        self._watchers: dict[str, asyncio.Task[None]] = {}

    @property
    def max_depth(self) -> int:
        return self._max_depth

    def depth(self, run_id: str) -> int:
        return self._depth_by_run.get(run_id, 0)

    def read_thread(self, **kwargs: Any) -> JsonObject:
        """Read a Thread and annotate a live non-blocking child Run.

        The parent collects child results with the existing ``thread_read``
        Tool. This wrapper keeps that Tool ID and its storage reader untouched
        while making an unsettled child explicit in the returned metadata.
        """

        payload = self._store.read_thread_slice(**kwargs)
        thread = payload.get("thread")
        if not isinstance(thread, dict):
            return payload
        thread_id = thread.get("id")
        if not isinstance(thread_id, str):
            return payload
        run_id = self._run_by_thread.get(thread_id)
        if run_id is None:
            return payload
        try:
            status = self._store.run_status(run_id)
        except LookupError:
            return payload
        if status not in {"queued", "running"}:
            return payload
        annotated: JsonObject = dict(payload)
        annotated_thread: JsonObject = dict(thread)
        annotated_thread["activeRun"] = {"id": run_id, "status": status}
        annotated_thread["notice"] = (
            f"Thread has an active Run {run_id} (status: {status}); only settled "
            "Items are visible. Read again after the Run settles."
        )
        annotated["thread"] = annotated_thread
        return annotated

    def notify_run_terminal(self, run_id: str) -> None:
        """Release bookkeeping after a child Run settles."""

        self._release_run(run_id)

    def close(self) -> None:
        """Drop cancellation watchers during Runtime shutdown."""

        for watcher in tuple(self._watchers.values()):
            watcher.cancel()
        self._watchers.clear()
        self._depth_by_run.clear()
        self._active_by_parent.clear()
        self._parent_by_run.clear()
        self._run_by_thread.clear()

    async def spawn(
        self,
        *,
        parent_run_id: str,
        request_id: str,
        prompt: str,
        title: str | None,
        wait: bool = True,
        cancellation: CancellationToken,
    ) -> JsonObject:
        parent_depth = self.depth(parent_run_id)
        if parent_depth >= self._max_depth:
            raise SubagentError(
                "subagent_depth_limit",
                (
                    f"Subagent depth limit reached (max {self._max_depth}); "
                    f"this Run is already at depth {parent_depth}."
                ),
            )
        parent_config = self._store.get_run_config(parent_run_id)
        child_title = (title or _default_title(prompt)).strip() or None
        child_depth = parent_depth + 1
        if wait:
            return await self._spawn_inline(
                request_id=request_id,
                prompt=prompt,
                child_title=child_title,
                child_depth=child_depth,
                parent_config=parent_config,
                cancellation=cancellation,
            )
        return await self._spawn_non_blocking(
            parent_run_id=parent_run_id,
            request_id=request_id,
            prompt=prompt,
            child_title=child_title,
            child_depth=child_depth,
            parent_config=parent_config,
            cancellation=cancellation,
        )

    async def _spawn_inline(
        self,
        *,
        request_id: str,
        prompt: str,
        child_title: str | None,
        child_depth: int,
        parent_config: RunConfig,
        cancellation: CancellationToken,
    ) -> JsonObject:
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
            self._run_by_thread[thread.id] = prepared.run_id
            await self._execute(prepared.run_id, child_depth, cancellation)
        return self._payload(thread.id, prepared.run_id)

    async def _spawn_non_blocking(
        self,
        *,
        parent_run_id: str,
        request_id: str,
        prompt: str,
        child_title: str | None,
        child_depth: int,
        parent_config: RunConfig,
        cancellation: CancellationToken,
    ) -> JsonObject:
        self._prune_active(parent_run_id)
        active = self._active_by_parent.get(parent_run_id, set())
        if len(active) >= self._max_active_per_run:
            raise SubagentError(
                "subagent_concurrency_limit",
                (
                    f"Run already has {len(active)} live non-blocking subagents "
                    f"(max {self._max_active_per_run}); wait for one to settle or "
                    "use wait=true."
                ),
            )
        if self._enqueue is None or self._cancel is None:
            raise SubagentError(
                "subagent_unavailable",
                "Non-blocking subagents require a running Runtime scheduler.",
            )
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
            self._register_child(parent_run_id, thread.id, prepared.run_id, child_depth)
            self._watch_parent_cancellation(
                run_id=prepared.run_id,
                cancellation=cancellation,
            )
            try:
                await self._enqueue(prepared.run_id)
            except Exception as error:
                self._release_run(prepared.run_id)
                raise SubagentError(
                    "subagent_unavailable",
                    f"Failed to enqueue subagent Run: {error}",
                ) from error
        return self._payload(thread.id, prepared.run_id)

    def _payload(self, thread_id: str, run_id: str) -> JsonObject:
        return {
            "threadId": thread_id,
            "runId": run_id,
            "status": self._store.run_status(run_id),
            "summary": self._store.final_assistant_message(thread_id) or "",
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
            self._release_run(run_id)

    def _register_child(
        self,
        parent_run_id: str,
        thread_id: str,
        run_id: str,
        depth: int,
    ) -> None:
        self._depth_by_run[run_id] = depth
        self._parent_by_run[run_id] = parent_run_id
        self._run_by_thread[thread_id] = run_id
        self._active_by_parent.setdefault(parent_run_id, set()).add(run_id)

    def _watch_parent_cancellation(
        self,
        *,
        run_id: str,
        cancellation: CancellationToken,
    ) -> None:
        cancel_run = self._cancel
        if cancel_run is None:
            return

        async def propagate() -> None:
            try:
                await cancellation.wait()
                await cancel_run(run_id)
            finally:
                self._release_run(run_id)

        watcher = asyncio.create_task(
            propagate(),
            name=f"ikaros-subagent-cancel-{run_id}",
        )
        self._watchers[run_id] = watcher

    def _prune_active(self, parent_run_id: str) -> None:
        active = self._active_by_parent.get(parent_run_id)
        if not active:
            return
        for run_id in tuple(active):
            try:
                status = self._store.run_status(run_id)
            except LookupError:
                status = "failed"
            if status not in {"queued", "running"}:
                self._release_run(run_id)

    def _release_run(self, run_id: str) -> None:
        self._depth_by_run.pop(run_id, None)
        for thread_id, mapped_run_id in tuple(self._run_by_thread.items()):
            if mapped_run_id == run_id:
                self._run_by_thread.pop(thread_id, None)
        parent_run_id = self._parent_by_run.pop(run_id, None)
        if parent_run_id is not None:
            active = self._active_by_parent.get(parent_run_id)
            if active is not None:
                active.discard(run_id)
                if not active:
                    self._active_by_parent.pop(parent_run_id, None)
        watcher = self._watchers.pop(run_id, None)
        if watcher is not None and watcher is not asyncio.current_task():
            watcher.cancel()


def _default_title(prompt: str) -> str:
    first = prompt.strip().splitlines()[0].strip() if prompt.strip() else ""
    return f"Subagent: {(first[:48].rstrip() or 'task')}"


__all__ = [
    "DEFAULT_SUBAGENT_MAX_DEPTH",
    "MAX_SUBAGENT_MAX_DEPTH",
    "MIN_SUBAGENT_MAX_DEPTH",
    "SUBAGENT_MAX_ACTIVE_PER_RUN",
    "SUBAGENT_MAX_DEPTH_ENV",
    "SubagentError",
    "SubagentService",
    "subagent_depth_limit",
]