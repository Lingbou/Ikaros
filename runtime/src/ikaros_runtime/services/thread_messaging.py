"""Create, rename, archive, and message other conversation Threads.

The RPC surface owns Thread lifecycle for the Desktop; this Service owns the
same lifecycle for the model. It deliberately routes every mutation through
``ThreadService`` so the model and the Desktop share one validation,
idempotency, and Journal-event path, and it keeps the cross-session hop chain
in the target Run's User Item data so a message loop cannot run forever.
"""

from __future__ import annotations

import asyncio
import hashlib
import os
from collections.abc import Awaitable, Callable
from contextlib import suppress
from pathlib import Path
from typing import Any

from ..cancellation import CancellationToken
from ..domain import (
    CommandOutcome,
    JournalEvent,
    JsonObject,
    PreparedTurn,
    ThreadSummary,
)
from ..storage.store import SqliteRuntimeStore
from ..storage.thread_messaging import THREAD_SEND_DATA_KEY
from .threads import ThreadService

THREAD_SEND_MAX_HOPS = 3
THREAD_CREATE_REQUEST_PREFIX = "thread-tool-create"
THREAD_SEND_REQUEST_PREFIX = "thread-tool-send"

RequestSafetyCheck = Callable[[object], None]
PublishEvent = Callable[[JournalEvent], Awaitable[None]]
EnqueueRun = Callable[[str], Awaitable[None]]
RunInline = Callable[[str, CancellationToken], Awaitable[None]]


class ThreadMessagingError(ValueError):
    """Expected, model-visible Thread management failure."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


class ThreadMessagingService:
    """Own the model-facing Thread lifecycle and cross-Thread delivery."""

    def __init__(
        self,
        *,
        store: SqliteRuntimeStore,
        threads: ThreadService,
        assert_request_safe: RequestSafetyCheck,
        publish: PublishEvent,
        enqueue_run: EnqueueRun,
        run_inline: RunInline,
    ) -> None:
        self._store = store
        self._threads = threads
        self._assert_request_safe = assert_request_safe
        self._publish = publish
        self._enqueue_run = enqueue_run
        self._run_inline = run_inline

    async def create(
        self,
        *,
        title: str | None,
        workspace: str | None,
        request_id: str,
        parent_run_id: str,
    ) -> JsonObject:
        params: dict[str, Any] = {
            "clientRequestId": f"{THREAD_CREATE_REQUEST_PREFIX}:{request_id}",
        }
        if title is not None:
            params["title"] = title
        if workspace is not None:
            params["workspace"] = project_workspace_from_directory(workspace)
        else:
            # New sessions belong to the caller's project unless the caller has no
            # workspace of its own; only then does the Thread fall back to a managed
            # workspace (Recents).
            try:
                caller = self._store.get_run_config(parent_run_id)
            except (LookupError, RuntimeError):
                caller = None
            if caller is not None and caller.workspace is not None:
                params["workspace"] = caller.workspace.to_wire()
        outcome = self._threads.create(params)
        await self._publish_outcome(outcome)
        thread = _object(outcome.result["thread"])
        return {
            "threadId": str(thread["id"]),
            "title": thread["title"],
            "workspace": thread["workspace"],
            "created": bool(outcome.events_after_ack),
        }

    async def rename(
        self,
        *,
        thread_id: str,
        title: str,
    ) -> JsonObject:
        self._require_thread(thread_id)
        outcome = self._threads.rename({"threadId": thread_id, "title": title})
        await self._publish_outcome(outcome)
        thread = _object(outcome.result["thread"])
        return {
            "threadId": str(thread["id"]),
            "title": thread["title"],
            "changed": bool(outcome.result["changed"]),
        }

    async def set_archived(
        self,
        *,
        thread_id: str,
        archived: bool,
    ) -> JsonObject:
        self._require_thread(thread_id)
        if archived and self._store.thread_has_active_run(thread_id):
            raise ThreadMessagingError(
                "thread_busy",
                "Thread cannot be archived while it has an active Run.",
            )
        params = {"threadId": thread_id}
        outcome = (
            self._threads.archive(params) if archived else self._threads.unarchive(params)
        )
        await self._publish_outcome(outcome)
        thread = _object(outcome.result["thread"])
        return {
            "threadId": str(thread["id"]),
            "archived": thread["archivedAt"] is not None,
            "changed": bool(outcome.result["changed"]),
        }

    async def send(
        self,
        *,
        thread_id: str,
        content: str,
        request_id: str,
        parent_run_id: str,
        wait: bool,
        cancellation: CancellationToken,
    ) -> JsonObject:
        self._assert_request_safe((thread_id, content, request_id))
        thread = self._target_thread(thread_id)
        try:
            parent_config = self._store.get_run_config(parent_run_id)
        except (LookupError, RuntimeError) as error:
            raise ThreadMessagingError("thread_send_unavailable", str(error)) from error
        message = f"{_message_header(parent_config.thread_id)}{content}"
        client_request_id = f"{THREAD_SEND_REQUEST_PREFIX}:{request_id}:{thread_id}"
        try:
            existing = self._store.find_turn_by_client_request_id(
                thread_id=thread_id,
                branch_id=thread.default_branch_id,
                content=message,
                provider_id=parent_config.provider_id,
                model_id=parent_config.model_id,
                client_request_id=client_request_id,
            )
        except LookupError as error:
            raise ThreadMessagingError("thread_send_unavailable", str(error)) from error
        if existing is not None:
            # A replay must return the Run it already delivered, even while that
            # Run is still active, instead of reporting the target as busy.
            return self._delivery_payload(existing, wait=wait)
        hop = self._parent_hop(parent_run_id)
        if hop >= THREAD_SEND_MAX_HOPS:
            raise ThreadMessagingError(
                "thread_hop_limit",
                "Cross-session messages reached the hop limit "
                f"({THREAD_SEND_MAX_HOPS}); ask the user instead of looping.",
            )
        if self._store.thread_has_active_run(thread_id):
            raise ThreadMessagingError(
                "thread_busy",
                "Target Thread already has an active Run; wait for it to settle "
                "before sending another message.",
            )
        try:
            prepared = self._store.prepare_turn(
                thread_id=thread_id,
                branch_id=thread.default_branch_id,
                content=message,
                run_config_template=parent_config.template,
                client_request_id=client_request_id,
                user_item_data={
                    THREAD_SEND_DATA_KEY: {
                        "hop": hop + 1,
                        "sourceThreadId": parent_config.thread_id,
                        "sourceRunId": parent_run_id,
                    }
                },
            )
        except LookupError as error:
            raise ThreadMessagingError("thread_send_unavailable", str(error)) from error
        if prepared.newly_created:
            for event in prepared.initial_events:
                await self._publish(event)
            if wait:
                await self._execute(prepared.run_id, cancellation)
            else:
                try:
                    await self._enqueue_run(prepared.run_id)
                except RuntimeError as error:
                    raise ThreadMessagingError(
                        "thread_send_unavailable", str(error)
                    ) from error
        return self._delivery_payload(prepared, wait=wait)

    def _delivery_payload(self, prepared: PreparedTurn, *, wait: bool) -> JsonObject:
        payload: JsonObject = {
            "threadId": prepared.thread_id,
            "turnId": prepared.turn_id,
            "runId": prepared.run_id,
            "status": self._store.run_status(prepared.run_id),
            "waited": wait,
        }
        if wait:
            payload["summary"] = self._store.final_assistant_message_for_run(
                prepared.run_id
            ) or ""
        return payload

    def _require_thread(self, thread_id: str) -> ThreadSummary:
        try:
            return self._store.get_thread(thread_id).thread
        except LookupError as error:
            raise ThreadMessagingError("thread_not_found", str(error)) from error

    def _target_thread(self, thread_id: str) -> ThreadSummary:
        thread = self._require_thread(thread_id)
        if thread.archived_at is not None:
            raise ThreadMessagingError(
                "thread_archived",
                "Target Thread is archived; unarchive it before sending a message.",
            )
        return thread

    def _parent_hop(self, parent_run_id: str) -> int:
        try:
            return self._store.thread_send_hop(parent_run_id)
        except LookupError as error:
            raise ThreadMessagingError("thread_send_unavailable", str(error)) from error

    async def _publish_outcome(self, outcome: CommandOutcome) -> None:
        for event in outcome.events_after_ack:
            await self._publish(event)

    async def _execute(self, run_id: str, cancellation: CancellationToken) -> None:
        """Run the delivered Turn on the calling worker.

        ``wait=true`` must never queue behind the worker pool it is occupying,
        so the Run executes inline exactly like a subagent child Run.
        """

        child_token = CancellationToken()

        async def propagate() -> None:
            await cancellation.wait()
            child_token.cancel()

        watcher = asyncio.create_task(propagate())
        child_task: asyncio.Future[None] = asyncio.ensure_future(
            self._run_inline(run_id, child_token)
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


def _message_header(source_thread_id: str) -> str:
    return f"[Cross-session message from thread {source_thread_id}]\n\n"


def _object(value: object) -> JsonObject:
    if not isinstance(value, dict):
        raise RuntimeError("Thread mutation result is invalid")
    return value


def project_workspace_from_directory(directory: str) -> JsonObject:
    """Derive the project workspace identity the Desktop uses for a directory."""

    root = Path(directory).expanduser()
    if not root.is_absolute():
        raise ThreadMessagingError(
            "invalid_arguments", "workspace must be an absolute directory path"
        )
    try:
        resolved = root.resolve(strict=True)
    except OSError:
        raise ThreadMessagingError(
            "invalid_arguments", "workspace must be an existing directory"
        ) from None
    if not resolved.is_dir():
        raise ThreadMessagingError(
            "invalid_arguments", "workspace must be an existing directory"
        )
    identity = str(resolved).lower() if os.name == "nt" else str(resolved)
    digest = hashlib.sha256(identity.encode("utf-8")).hexdigest()[:24]
    return {
        "id": f"workspace-{digest}",
        "name": resolved.name or str(resolved),
        "kind": "project",
        "rootUri": str(resolved),
    }


__all__ = [
    "THREAD_CREATE_REQUEST_PREFIX",
    "THREAD_SEND_MAX_HOPS",
    "THREAD_SEND_REQUEST_PREFIX",
    "ThreadMessagingError",
    "ThreadMessagingService",
    "project_workspace_from_directory",
]
