from __future__ import annotations

import logging
from dataclasses import dataclass
from typing import Any

from ..domain import CommandOutcome
from ..errors import InvalidParamsError, ProviderFailure
from ..services.providers import ProviderService
from ..services.threads import ThreadService
from ..services.turns import TurnService
from .jsonrpc import JSONRPC_VERSION, jsonrpc_error

RPC_METHODS = frozenset(
    {
        "runtime.shutdown",
        "thread.create",
        "thread.list",
        "provider.list",
        "provider.configure",
        "provider.discover_models",
        "provider.disconnect",
        "provider.remove",
        "model.list",
        "model.set_enabled",
        "turn.start",
        "run.cancel",
        "event.replay",
    }
)

_LOGGER = logging.getLogger("ikaros_runtime")


@dataclass(frozen=True, slots=True)
class RouteResult:
    """A routed response plus work that must happen only after its ACK is sent."""

    response: dict[str, object]
    outcome: CommandOutcome | None = None
    shutdown_accepted: bool = False


class RuntimeRouter:
    """Dispatch initialized, object-parameter Runtime RPCs to application services.

    Connection-level validation, initialization, protected-value checks, response
    delivery, and post-ACK outcome completion remain owned by the server layer.
    """

    def __init__(
        self,
        threads: ThreadService,
        turns: TurnService,
        providers: ProviderService,
    ) -> None:
        self._threads = threads
        self._turns = turns
        self._providers = providers

    async def dispatch(
        self,
        request_id: object,
        method: str,
        params: dict[str, Any],
    ) -> RouteResult:
        outcome: CommandOutcome | None = None
        try:
            if method == "runtime.shutdown":
                result = {"accepted": True}
            elif method == "thread.create":
                outcome = self._threads.create(params)
                result = outcome.result
            elif method == "thread.list":
                result = self._threads.list(params)
            elif method == "provider.list":
                result = self._providers.list_providers(params)
            elif method == "provider.configure":
                result = self._providers.configure_provider(params)
            elif method == "provider.discover_models":
                result = await self._providers.discover_provider_models(params)
            elif method == "provider.disconnect":
                result = self._providers.disconnect_provider(params)
            elif method == "provider.remove":
                result = self._providers.remove_provider(params)
            elif method == "model.list":
                result = self._providers.list_models(params)
            elif method == "model.set_enabled":
                result = self._providers.set_model_enabled(params)
            elif method == "turn.start":
                outcome = self._turns.start_turn(params)
                result = outcome.result
            elif method == "run.cancel":
                outcome = self._turns.cancel_run(params)
                result = outcome.result
            elif method == "event.replay":
                result = self._turns.replay_events(params)
            else:
                return RouteResult(jsonrpc_error(request_id, -32601, "method not found"))
        except InvalidParamsError as error:
            return RouteResult(jsonrpc_error(request_id, -32602, str(error)))
        except ProviderFailure as error:
            return RouteResult(jsonrpc_error(request_id, -32010, str(error)))
        except Exception:
            _LOGGER.error("Unhandled Runtime method failure")
            return RouteResult(jsonrpc_error(request_id, -32603, "internal error"))

        response: dict[str, object] = {
            "jsonrpc": JSONRPC_VERSION,
            "id": request_id,
            "result": result,
        }
        return RouteResult(
            response=response,
            outcome=outcome,
            shutdown_accepted=method == "runtime.shutdown",
        )


__all__ = ["RPC_METHODS", "RouteResult", "RuntimeRouter"]
