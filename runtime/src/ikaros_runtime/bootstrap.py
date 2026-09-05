"""The sole production composition root for Ikaros Runtime."""

from __future__ import annotations

import asyncio
import logging
import sys
from collections.abc import Sequence

from websockets.asyncio.server import ServerConnection

from .agent.loop import AgentLoop, EventPublisher
from .agent.scheduler import AgentScheduler
from .config import ConfigDocumentStore
from .domain import CommandOutcome
from .identity import load_identity_core
from .memory import MemoryRetrieverV1, SqliteMemoryStore
from .paths import RuntimePaths
from .protocol.router import RuntimeRouter
from .providers.openai_compatible.adapter import OpenAICompatibleAdapter
from .providers.openai_compatible.discovery import discover_openai_compatible_models
from .providers.registry import ConfigStore, RuntimeProviderRegistry
from .providers.scripted import ScriptedProvider
from .run_input import InstructionBlockV1, ProviderExecutionSnapshotV1
from .security import RuntimeSecurity
from .server.connection import handle_connection
from .server.event_hub import EventHub
from .server.host import RuntimeHomeLock, ServerSettings, run_host
from .services.files import FileService
from .services.memories import MemoryService
from .services.providers import ModelDiscovery, ProviderService
from .services.skills import SkillService
from .services.threads import ThreadService
from .services.turns import TurnService
from .services.usage import UsageService
from .skills import SkillCatalog
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
        memory_store: SqliteMemoryStore,
        config_store: ConfigStore | None = None,
        model_discovery: ModelDiscovery = discover_openai_compatible_models,
        identity_core: InstructionBlockV1 | None = None,
    ) -> None:
        self._store = store
        self._memory_store = memory_store
        paths = RuntimePaths.from_home(store.database_path.parent)
        self._config = config_store or ConfigStore(
            ConfigDocumentStore(paths.home)
        )
        self.security = RuntimeSecurity(
            self._config.protected_values,
            lambda values: self._store.journal_contains_protected_values(values)
            or self._memory_store.contains_protected_values(tuple(values)),
        )
        self.security.assert_configuration_safe()
        self.identity_core = identity_core or load_identity_core()

        self.skills = SkillService(
            SkillCatalog(
                paths.skills,
                self.security.protected_values,
            ),
            self._config.document_store,
        )

        self._provider_registry = RuntimeProviderRegistry(
            self._config,
            adapter_factory=OpenAICompatibleAdapter,
        )
        tool_executor = ToolExecutor(
            ToolRegistry([ProcessRunTool(), ReadTool(), WriteTool(), EditTool()]),
            FullAccessPolicy(),
        )

        def provider_execution_snapshot(
            provider_id: str,
            model_id: str,
        ) -> ProviderExecutionSnapshotV1:
            if provider_id == ScriptedProvider.id:
                if model_id != ScriptedProvider.model_id:
                    raise ValueError("scripted model is unavailable")
                return ProviderExecutionSnapshotV1(
                    provider_id=ScriptedProvider.id,
                    origin="scripted",
                    base_url=None,
                    model_id=ScriptedProvider.model_id,
                    supports_tools=True,
                )
            return self._config.execution_snapshot(provider_id, model_id)

        loop = AgentLoop(
            store,
            self._provider_registry,
            publish,
            tool_executor,
            protected_values=self.security.protected_values,
            provider_snapshot_resolver=provider_execution_snapshot,
            identity_core=self.identity_core,
            memory_retriever=MemoryRetrieverV1(memory_store),
        )
        self._scheduler = AgentScheduler(loop)
        self._publish = publish

        self.threads = ThreadService(store, self.security.assert_request_safe)
        self.turns = TurnService(
            store,
            self._scheduler,
            self._config,
            self.security.assert_request_safe,
            self.identity_core,
            self.skills.enabled_descriptors,
            lambda: tool_executor.definitions,
            tool_executor.policy_name,
            loop.max_steps,
        )
        self.providers = ProviderService(
            self._config,
            self._provider_registry,
            store,
            model_discovery,
            self.security.assert_credentials_safe,
        )
        self.usage = UsageService(store)
        self.memories = MemoryService(
            memory_store,
            self.security.assert_request_safe,
            store,
        )
        self.files = FileService(store, self.security, paths)
        self.router = RuntimeRouter(
            self.threads,
            self.turns,
            self.providers,
            self.usage,
            self.skills,
            self.memories,
            self.files,
        )

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
        try:
            await self._scheduler.close()
            await self._provider_registry.close()
        finally:
            self._memory_store.close()


async def run_runtime_server(settings: ServerSettings) -> None:
    """Acquire the Runtime home, assemble the application, and serve it."""

    settings.validate()
    runtime_lock = await RuntimeHomeLock.acquire(settings.runtime_home)
    store: SqliteRuntimeStore | None = None
    memory_store: SqliteMemoryStore | None = None
    application: RuntimeApplication | None = None
    try:
        logging.basicConfig(level=logging.INFO, stream=sys.stderr)
        paths = RuntimePaths.from_home(settings.runtime_home)
        identity_core = load_identity_core()
        config_document_store = ConfigDocumentStore(paths.home)
        config_store = ConfigStore(config_document_store)
        store = SqliteRuntimeStore(paths.state_db)
        opened_memory_store = SqliteMemoryStore(paths.memory_db)
        memory_store = opened_memory_store

        pre_recovery_security = RuntimeSecurity(
            config_store.protected_values,
            lambda values: store.journal_contains_protected_values(values)
            or opened_memory_store.contains_protected_values(tuple(values)),
        )
        pre_recovery_security.assert_configuration_safe()
        recovery = store.recover_incomplete_runs()

        event_hub = EventHub(next_seq=store.latest_sequence() + 1)
        application = RuntimeApplication(
            store,
            event_hub.publish,
            memory_store=opened_memory_store,
            config_store=config_store,
            identity_core=identity_core,
        )
        memory_store = None
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
        if memory_store is not None:
            memory_store.close()
        if store is not None:
            store.close()
        runtime_lock.release()


__all__ = ["RuntimeApplication", "ServerSettings", "run_runtime_server"]
