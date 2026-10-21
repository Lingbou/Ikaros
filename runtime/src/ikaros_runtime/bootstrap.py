"""The sole production composition root for Ikaros Runtime."""

from __future__ import annotations

import asyncio
import logging
import os
import sys
from collections.abc import Callable, Mapping, Sequence

from websockets.asyncio.server import ServerConnection

from .agent.loop import AgentLoop, EventPublisher, ProviderSnapshotResolver
from .agent.scheduler import AgentScheduler
from .cancellation import CancellationToken
from .channels.config import TelegramConfig, TelegramConfigStore
from .channels.store import ChannelStore
from .channels.telegram.adapter import TelegramChannel
from .channels.telegram.client import TelegramBotApi
from .channels.telegram.service import ChannelService
from .config import ConfigDocumentStore
from .domain import CommandOutcome, JournalEvent
from .identity import load_identity_core
from .memory import MemoryRefinementService, MemoryRetriever, SqliteMemoryStore
from .paths import RuntimePaths
from .protocol.router import RuntimeRouter
from .providers.base import ProviderAdapter
from .providers.openai_compatible.adapter import OpenAICompatibleAdapter
from .providers.openai_compatible.discovery import discover_openai_compatible_models
from .providers.registry import ConfigStore, RuntimeProviderRegistry
from .run_input import InstructionBlock, ProviderExecutionSnapshot
from .security import (
    RuntimeSecurity,
    contains_protected_value,
    install_protected_value_redaction,
)
from .server.connection import handle_connection
from .server.event_hub import EventHub, EventSink
from .server.host import RuntimeHomeLock, ServerSettings, run_host
from .services.files import FileService
from .services.memories import MemoryService, MemoryToolService
from .services.processes import ProcessService
from .services.providers import ModelDiscovery, ProviderService
from .services.skills import SkillService
from .services.subagents import SubagentService
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
    MemoryTool,
    ProcessManager,
    ProcessReadTool,
    ProcessResizeTool,
    ProcessStartTool,
    ProcessStopTool,
    ProcessWaitTool,
    ProcessWriteTool,
    ReadTool,
    SubagentTool,
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
        agent_workers: int | None = None,
        channel_config_store: TelegramConfigStore | None = None,
        channel_store: ChannelStore | None = None,
        event_subscribe: Callable[[EventSink], Callable[[], None]] | None = None,
        channel_api_factory: Callable[[TelegramConfig], TelegramBotApi] | None = None,
    ) -> None:
        self._store = store
        self._memory_store = memory_store
        paths = RuntimePaths.from_home(store.database_path.parent)
        self._config = config_store or ConfigStore(ConfigDocumentStore(paths.home))
        channel_protected_values = (
            channel_config_store.protected_values
            if channel_config_store is not None
            else lambda: ()
        )
        self.security = RuntimeSecurity(
            lambda: (*self._config.protected_values(), *channel_protected_values()),
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
        self.memories = MemoryService(
            memory_store,
            self.security.assert_request_safe,
            store,
        )
        memory_retriever = MemoryRetriever(memory_store)
        self._subagents = SubagentService(
            store=store,
            run_child=self._run_subagent,
            publish=publish,
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
                    MemoryTool(
                        MemoryToolService(
                            self.memories,
                            memory_retriever,
                            store.memory_tool_run_scope,
                        )
                    ),
                    SubagentTool(self._subagents),
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

        self._refinement = MemoryRefinementService(
            memory_store=memory_store,
            runtime_store=store,
            resolve_provider=self._provider_registry.resolve,
            provider_snapshot=provider_snapshot,
            protected_values=self.security.protected_values,
            enabled=_memory_refinement_enabled(),
        )
        loop = AgentLoop(
            store,
            self._provider_registry,
            publish,
            tool_executor,
            process_manager=self._process_manager,
            protected_values=self.security.protected_values,
            provider_snapshot_resolver=provider_snapshot,
            identity_core=self.identity_core,
            memory_retriever=memory_retriever,
            on_run_terminal=self._refinement.notify_run_terminal,
        )
        self._scheduler = AgentScheduler(loop, max_workers=agent_workers)
        self._agent_loop = loop
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
        self.files = FileService(store, self.security, paths)
        self._channel_service: ChannelService | None = None
        self._channel_subscription: Callable[[], None] | None = None
        if channel_config_store is not None and channel_store is not None:
            channel = TelegramChannel(
                config_store=channel_config_store,
                channel_store=channel_store,
                runtime_store=store,
                provider_store=self._config,
                threads=self.threads,
                turns=self.turns,
                finish_command=self.finish_command,
                api_factory=channel_api_factory,
            )
            self._channel_service = ChannelService(
                channel=channel,
                config_store=channel_config_store,
                channel_store=channel_store,
                provider_store=self._config,
                api_factory=channel_api_factory,
            )
            if event_subscribe is not None:
                self._channel_subscription = event_subscribe(channel.handle_event)
        self.router = RuntimeRouter(
            self.threads,
            self.turns,
            self.providers,
            self.usage,
            self.skills,
            self.memories,
            self.files,
            ProcessService(self._process_manager, store),
            self._channel_service,
        )

    def start(self, recovered_run_ids: Sequence[str] = ()) -> None:
        self._scheduler.start(recovered_run_ids)

    async def start_channels(self) -> None:
        if self._channel_service is not None:
            await self._channel_service.start()

    async def finish_command(self, outcome: CommandOutcome) -> None:
        for event in outcome.events_after_ack:
            await self._publish(event)
        if outcome.run_after_ack is not None:
            await self._scheduler.activate(outcome.run_after_ack)
        if outcome.cancel_after_ack is not None:
            await self._scheduler.cancel(outcome.cancel_after_ack)

    async def _run_subagent(
        self,
        run_id: str,
        cancellation: CancellationToken,
    ) -> None:
        await self._agent_loop.run(run_id, cancellation)

    async def close(self) -> None:
        try:
            if self._channel_subscription is not None:
                self._channel_subscription()
                self._channel_subscription = None
            if self._channel_service is not None:
                try:
                    await self._channel_service.close()
                except Exception:
                    logging.getLogger("ikaros_runtime").exception(
                        "telegram channel shutdown failed"
                    )
            await self._scheduler.close()
            await self._refinement.close()
            await self._process_manager.close()
            if self._process_events:
                await asyncio.gather(*self._process_events)
            await self._provider_registry.close()
        finally:
            self._memory_store.close()


def _memory_refinement_enabled() -> bool:
    """Honor the ``IKAROS_MEMORY_REFINEMENT=0`` kill switch."""

    return os.environ.get("IKAROS_MEMORY_REFINEMENT", "1").strip() != "0"


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
    channel_store: ChannelStore | None = None
    application: RuntimeApplication | None = None
    try:
        logging.basicConfig(level=logging.INFO, stream=sys.stderr)
        paths = RuntimePaths.from_home(settings.runtime_home)
        identity_core = load_identity_core()
        config_document_store = ConfigDocumentStore(paths.home)
        config_store = ConfigStore(config_document_store)
        channel_config_store = TelegramConfigStore(paths.home)
        channel_store = ChannelStore(paths.channels_db)
        store = SqliteRuntimeStore(paths.state_db)
        opened_memory_store = SqliteMemoryStore(paths.memory_db)
        memory_store = opened_memory_store

        pre_recovery_security = RuntimeSecurity(
            lambda: (
                *config_store.protected_values(),
                *channel_config_store.protected_values(),
            ),
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
            channel_config_store=channel_config_store,
            channel_store=channel_store,
            event_subscribe=event_hub.subscribe,
        )
        memory_store = None
        install_protected_value_redaction(
            application.security.protected_values,
            handlers=tuple(logging.getLogger().handlers),
            loggers=(logging.getLogger("httpx"),),
        )
        application.start(recovery.queued_run_ids)
        await application.start_channels()

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
        if channel_store is not None:
            channel_store.close()
        runtime_lock.release()


__all__ = ["RuntimeApplication", "ServerSettings", "run_runtime_server"]
