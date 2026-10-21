"""The sole production composition root for Ikaros Runtime."""

from __future__ import annotations

import asyncio
import logging
import sys
from collections.abc import Mapping, Sequence

from websockets.asyncio.server import ServerConnection

from .agent.loop import AgentLoop, EventPublisher, ProviderSnapshotResolver
from .agent.scheduler import AgentScheduler
from .config import ConfigDocumentStore
from .domain import CommandOutcome, JournalEvent
from .identity import load_identity_core
from .memory import MemoryRetriever, SqliteMemoryStore
from .paths import RuntimePaths
from .protocol.router import RuntimeRouter
from .providers.base import ProviderAdapter
from .providers.openai_compatible.adapter import OpenAICompatibleAdapter
from .providers.openai_compatible.discovery import discover_openai_compatible_models
from .providers.registry import ConfigStore, RuntimeProviderRegistry
from .run_input import InstructionBlock, ProviderExecutionSnapshot
from .security import RuntimeSecurity, contains_protected_value
from .server.connection import handle_connection
from .server.event_hub import EventHub
from .server.host import RuntimeHomeLock, ServerSettings, run_host
from .services.files import FileService
from .services.memories import MemoryService
from .services.processes import ProcessService
from .services.providers import ModelDiscovery, ProviderService
from .services.skills import SkillService
from .services.threads import ThreadService
from .services.turns import TurnService
from .services.usage import UsageService
from .skills import SkillCatalog
from .storage import SqliteRuntimeStore
from .tools import (
    EditTool,
    GlobTool,
    GrepTool,
    HistoryReadTool,
    ListTool,
    ProcessManager,
    ProcessReadTool,
    ProcessResizeTool,
    ProcessStartTool,
    ProcessStopTool,
    ProcessWaitTool,
    ProcessWriteTool,
    ReadTool,
    ThreadListTool,
    ThreadReadTool,
    WriteTool,
)
from .tools.artifacts import ToolArtifactStore
from .tools.core import ToolExecutor, ToolRegistry
from .tools.policy import FullAccessPolicy
from .workspaces import provision_managed_workspace


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
        identity_core: InstructionBlock | None = None,
        extra_providers: Mapping[str, ProviderAdapter] | None = None,
        provider_snapshot_resolver: ProviderSnapshotResolver | None = None,
    ) -> None:
        self._store = store
        self._memory_store = memory_store
        paths = RuntimePaths.from_home(store.database_path.parent)
        self._config = config_store or ConfigStore(ConfigDocumentStore(paths.home))
        self.security = RuntimeSecurity(
            self._config.protected_values,
            lambda values: (
                self._store.journal_contains_protected_values(values)
                or self._memory_store.contains_protected_values(tuple(values))
            ),
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
            extra_providers=extra_providers,
        )
        self._process_events: set[asyncio.Task[None]] = set()

        async def publish_process(event: JournalEvent) -> None:
            await publish(event)

        def record_process(record: dict[str, object]) -> None:
            event = self._store.record_process(record)
            task = asyncio.create_task(publish_process(event))
            self._process_events.add(task)
            task.add_done_callback(self._process_events.discard)

        self._process_manager = ProcessManager(
            record=record_process,
            protected_values=self.security.protected_values,
        )
        self._process_manager.restore(
            tuple(
                record
                for record in self._store.process_records()
                if record["state"] == "running" or record["errorCode"] == "start_pending"
            )
        )
        tool_executor = ToolExecutor(
            ToolRegistry(
                [
                    ProcessStartTool(self._process_manager),
                    ProcessReadTool(self._process_manager),
                    ProcessWaitTool(self._process_manager),
                    ProcessStopTool(self._process_manager),
                    ProcessWriteTool(self._process_manager),
                    ProcessResizeTool(self._process_manager),
                    ReadTool(),
                    WriteTool(),
                    EditTool(),
                    HistoryReadTool(store.read_history_slice),
                    ThreadListTool(store.list_threads_for_model),
                    ThreadReadTool(store.read_thread_slice),
                    GlobTool(),
                    GrepTool(),
                    ListTool(),
                ]
            ),
            FullAccessPolicy(),
            artifacts=ToolArtifactStore(
                paths.artifacts,
                is_safe=lambda text: not contains_protected_value(
                    text,
                    self.security.protected_values(),
                ),
            ),
        )

        def provider_snapshot(
            provider_id: str,
            model_id: str,
        ) -> ProviderExecutionSnapshot:
            if provider_snapshot_resolver is not None:
                try:
                    return provider_snapshot_resolver(provider_id, model_id)
                except LookupError:
                    pass
            return self._config.execution_snapshot(provider_id, model_id)

        loop = AgentLoop(
            store,
            self._provider_registry,
            publish,
            tool_executor,
            process_manager=self._process_manager,
            protected_values=self.security.protected_values,
            provider_snapshot_resolver=provider_snapshot,
            identity_core=self.identity_core,
            memory_retriever=MemoryRetriever(memory_store),
        )
        self._scheduler = AgentScheduler(loop)
        self._publish = publish

        self.threads = ThreadService(
            store,
            self.security.assert_request_safe,
            managed_workspace_factory=provision_managed_workspace,
        )
        self.turns = TurnService(
            store,
            self._scheduler,
            self._config,
            self.security.assert_request_safe,
            self.identity_core,
            self.skills.enabled_descriptors,
            lambda: tool_executor.definitions,
            tool_executor.policy_name,
            provider_snapshot=provider_snapshot,
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
            ProcessService(self._process_manager, store),
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
            await self._process_manager.close()
            if self._process_events:
                await asyncio.gather(*self._process_events)
            await self._provider_registry.close()
        finally:
            self._memory_store.close()


async def run_runtime_server(
    settings: ServerSettings,
    *,
    extra_providers: Mapping[str, ProviderAdapter] | None = None,
    provider_snapshot_resolver: ProviderSnapshotResolver | None = None,
) -> None:
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
            lambda values: (
                store.journal_contains_protected_values(values)
                or opened_memory_store.contains_protected_values(tuple(values))
            ),
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
            extra_providers=extra_providers,
            provider_snapshot_resolver=provider_snapshot_resolver,
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
