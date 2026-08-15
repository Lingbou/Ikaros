"""One authenticated Runtime WebSocket connection."""

from __future__ import annotations

import asyncio
import logging
from collections.abc import Awaitable, Callable, Sequence
from typing import Any, Protocol

from websockets.asyncio.server import ServerConnection
from websockets.exceptions import ConnectionClosed

from ..domain import CommandOutcome, JournalEvent
from ..json_codec import dumps as json_dumps
from ..json_codec import loads as json_loads
from ..protocol.jsonrpc import (
    configuration_write_values,
    initialize_result,
    jsonrpc_error,
    valid_request_id,
)
from ..protocol.spec import JSONRPC_VERSION, PROTOCOL_VERSION

_LOGGER = logging.getLogger("ikaros_runtime")
_EVENT_QUEUE_LIMIT = 1024
_SEND_TIMEOUT_SECONDS = 5.0


class RoutedResponse(Protocol):
    @property
    def response(self) -> dict[str, object]: ...

    @property
    def outcome(self) -> CommandOutcome | None: ...

    @property
    def shutdown_accepted(self) -> bool: ...


class ConnectionRouter(Protocol):
    async def dispatch(
        self,
        request_id: object,
        method: str,
        params: dict[str, Any],
    ) -> RoutedResponse: ...


class ConnectionSecurity(Protocol):
    def protected_values(self) -> tuple[str, ...]: ...

    def rpc_request_values_contain_protected_value(
        self,
        value: object,
        additional_values: Sequence[str] = (),
    ) -> bool: ...

    def response_contains_protected_value(
        self,
        value: object,
        additional_values: Sequence[str] = (),
    ) -> bool: ...


class ConnectionEventHub(Protocol):
    def subscribe(self, sink: Callable[[JournalEvent], None]) -> Callable[[], None]: ...


async def handle_connection(
    connection: ServerConnection,
    stop_event: asyncio.Event,
    router: ConnectionRouter,
    event_hub: ConnectionEventHub,
    security: ConnectionSecurity,
    finish_command: Callable[[CommandOutcome], Awaitable[None]],
) -> None:
    """Serve one WebSocket whose HTTP upgrade has already been authenticated."""

    initialized = False
    send_lock = asyncio.Lock()
    unsubscribe: Callable[[], None] | None = None
    event_queue: asyncio.Queue[JournalEvent] = asyncio.Queue(maxsize=_EVENT_QUEUE_LIMIT)
    event_sender: asyncio.Task[None] | None = None
    overflow_close: asyncio.Task[None] | None = None

    async def send_json(
        value: dict[str, object],
        *,
        additional_protected_values: tuple[str, ...] = (),
    ) -> None:
        if security.response_contains_protected_value(value, additional_protected_values):
            value = jsonrpc_error(
                None,
                -32603,
                "response contained protected configuration data",
            )
        try:
            payload = json_dumps(value, separators=(",", ":"))
        except (TypeError, ValueError):
            payload = json_dumps(
                jsonrpc_error(None, -32603, "internal error"),
                separators=(",", ":"),
            )
        async with send_lock:
            await connection.send(payload)

    async def send_event(event: JournalEvent) -> None:
        await send_json(
            {
                "jsonrpc": JSONRPC_VERSION,
                "method": "event",
                "params": event.to_wire(),
            }
        )

    def enqueue_event(event: JournalEvent) -> None:
        nonlocal overflow_close
        try:
            event_queue.put_nowait(event)
        except asyncio.QueueFull:
            if overflow_close is None:
                overflow_close = asyncio.create_task(
                    connection.close(code=1013, reason="event consumer too slow")
                )
            raise

    async def send_events() -> None:
        try:
            while True:
                event = await event_queue.get()
                try:
                    async with asyncio.timeout(_SEND_TIMEOUT_SECONDS):
                        await send_event(event)
                finally:
                    event_queue.task_done()
        except asyncio.CancelledError:
            raise
        except Exception:
            await connection.close(code=1011, reason="event stream failed")

    try:
        async for raw_message in connection:
            if not isinstance(raw_message, str):
                await send_json(jsonrpc_error(None, -32600, "text messages only"))
                continue

            try:
                request: Any = json_loads(raw_message)
            except ValueError:
                await send_json(jsonrpc_error(None, -32700, "parse error"))
                continue

            if not isinstance(request, dict) or request.get("jsonrpc") != JSONRPC_VERSION:
                await send_json(jsonrpc_error(None, -32600, "invalid request"))
                continue

            request_id = request.get("id")
            method = request.get("method")
            params = request.get("params", {})

            if not valid_request_id(request_id):
                await send_json(jsonrpc_error(None, -32600, "invalid request id"))
                continue

            configuration_values = (
                *security.protected_values(),
                *configuration_write_values(method, params),
            )
            if security.rpc_request_values_contain_protected_value(
                request_id,
                configuration_values,
            ):
                await send_json(
                    jsonrpc_error(
                        None,
                        -32600,
                        "request contains protected configuration data",
                    )
                )
                continue

            if not isinstance(method, str):
                await send_json(jsonrpc_error(None, -32600, "invalid request method"))
                continue

            route: RoutedResponse | None = None
            if method == "initialize":
                if (
                    not isinstance(params, dict)
                    or params.get("protocolVersion") != PROTOCOL_VERSION
                ):
                    response = jsonrpc_error(
                        request_id,
                        -32001,
                        "unsupported protocol version",
                    )
                elif initialized:
                    response = jsonrpc_error(
                        request_id,
                        -32002,
                        "connection already initialized",
                    )
                else:
                    initialized = True
                    response = {
                        "jsonrpc": JSONRPC_VERSION,
                        "id": request_id,
                        "result": initialize_result(),
                    }
            elif not initialized:
                response = jsonrpc_error(
                    request_id,
                    -32000,
                    "initialize must be called first",
                )
            elif not isinstance(params, dict):
                response = jsonrpc_error(request_id, -32602, "params must be an object")
            else:
                route = await router.dispatch(request_id, method, params)
                response = route.response

            initialized_now = (
                method == "initialize" and "result" in response and unsubscribe is None
            )
            if initialized_now:
                unsubscribe = event_hub.subscribe(enqueue_event)
            accepted_outcome = (
                route.outcome
                if route is not None and route.outcome is not None and "result" in response
                else None
            )
            shutdown_accepted = (
                route is not None and route.shutdown_accepted and "result" in response
            )
            try:
                async with asyncio.timeout(_SEND_TIMEOUT_SECONDS):
                    await send_json(
                        response,
                        additional_protected_values=configuration_write_values(method, params),
                    )
            finally:
                if accepted_outcome is not None:
                    await finish_command(accepted_outcome)
                if shutdown_accepted:
                    stop_event.set()
            if initialized_now:
                event_sender = asyncio.create_task(
                    send_events(),
                    name="ikaros-runtime-event-sender",
                )
            if shutdown_accepted:
                return
    except ConnectionClosed:
        return
    finally:
        if unsubscribe is not None:
            unsubscribe()
        if event_sender is not None:
            event_sender.cancel()
            await asyncio.gather(event_sender, return_exceptions=True)
        if overflow_close is not None:
            await asyncio.gather(overflow_close, return_exceptions=True)


__all__ = [
    "ConnectionEventHub",
    "ConnectionRouter",
    "ConnectionSecurity",
    "RoutedResponse",
    "handle_connection",
]
