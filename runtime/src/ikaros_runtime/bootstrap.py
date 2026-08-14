"""The sole production composition root for Ikaros Runtime."""

from __future__ import annotations

import asyncio
import logging
import sys
from collections.abc import Sequence

from websockets.asyncio.server import ServerConnection

from .agent.loop import AgentLoop, EventPublisher
from .agent.scheduler import AgentScheduler
from .domain import CommandOutcome
from .protocol.router import RuntimeRouter
from .providers.openai_compatible.adapter import OpenAICompatibleAdapter
from .providers.openai_compatible.discovery import discover_openai_compatible_models
from .providers.registry import ConfigStore, RuntimeProviderRegistry
from .security import RuntimeSecurity
from .server.connection import handle_connection
from .server.event_hub import EventHub
from .server.host import RuntimeHomeLock, ServerSettings, run_host
from .services.providers import ModelDiscovery, ProviderService
from .services.threads import ThreadService
from .services.turns import TurnService
from .services.usage import UsageService
from .storage import SqliteRuntimeStore
from .tools import EditTool, ProcessRunTool, ReadTool, WriteTool
from .tools.core import ToolExecutor, ToolRegistry
from .tools.policy import FullAccessPolicy


class RuntimeApplication:
    """Own the assembled application services and their execution lifecycle."""

    def __init__(
        self,
        store: SqliteRuntimeStore,
        publish: EventPublisher,
        *,
        config_store: ConfigStore | None = None,
        model_discovery: ModelDiscovery = discover_openai_compatible_models,
    ) -> None:
        self._store = store
        self._config = config_store or ConfigStore(store.database_path.parent)
        self.security = RuntimeSecurity(
            self._config.protected_values,
            self._store.journal_contains_protected_values,
        )
        self.security.assert_configuration_safe()

        self._provider_registry = RuntimeProviderRegistry(
            self._config,
            adapter_factory=OpenAICompatibleAdapter,
        )
        tool_executor = ToolExecutor(
            ToolRegistry([ProcessRunTool(), ReadTool(), WriteTool(), EditTool()]),
            FullAccessPolicy(),
        )
        loop = AgentLoop(
            store,
            self._provider_registry,
            publish,
            tool_executor,
            protected_values=self.security.protected_values,
        )
        self._scheduler = AgentScheduler(loop)
        self._publish = publish

        self.threads = ThreadService(store, self.security.assert_request_safe)
        self.turns = TurnService(
            store,
            self._scheduler,
            self._config,
            self.security.assert_request_safe,
        )
        self.providers = ProviderService(
            self._config,
            self._provider_registry,
            store,
            model_discovery,
            self.security.assert_credentials_safe,
        )
        self.usage = UsageService(store)
        self.router = RuntimeRouter(self.threads, self.turns, self.providers, self.usage)

    def start(self, recovered_run_ids: Sequence[str] = ()) -> None:
        self._scheduler.start(recovered_run_ids)

    async def finish_command(self, outcome: CommandOutcome) -> None:
        for event in outcome.events_after_ack:
            await self._publish(event)
        if outcome.run_after_ack is not None:
            await self._scheduler.activate(outcome.run_after_ack)
        if outcome.cancel_after_ack is not None:
            await self._scheduler.cancel(outcome.cancel_after_ack)

    async def close(self) -> None:
        await self._scheduler.close()
        await self._provider_registry.close()


async def run_runtime_server(settings: ServerSettings) -> None:
    """Acquire the Runtime home, assemble the application, and serve it."""

    settings.validate()
    runtime_lock = await RuntimeHomeLock.acquire(settings.runtime_home)
    store: SqliteRuntimeStore | None = None
    application: RuntimeApplication | None = None
    try:
        logging.basicConfig(level=logging.INFO, stream=sys.stderr)
        config_store = ConfigStore(settings.runtime_home)
        store = SqliteRuntimeStore(settings.runtime_home / "state.db")

        pre_recovery_security = RuntimeSecurity(
            config_store.protected_values,
            store.journal_contains_protected_values,
        )
        pre_recovery_security.assert_configuration_safe()
        recovery = store.recover_incomplete_runs()

        event_hub = EventHub(next_seq=store.latest_sequence() + 1)
        application = RuntimeApplication(
            store,
            event_hub.publish,
            config_store=config_store,
        )
        application.start(recovery.queued_run_ids)

        async def connection_handler(
            connection: ServerConnection,
            stop_event: asyncio.Event,
        ) -> None:
            await handle_connection(
                connection,
                stop_event,
                application.router,
                event_hub,
                application.security,
                application.finish_command,
            )

        await run_host(settings, connection_handler)
    finally:
        if application is not None:
            await application.close()
        if store is not None:
            store.close()
        runtime_lock.release()


__all__ = ["RuntimeApplication", "ServerSettings", "run_runtime_server"]
