from __future__ import annotations

import asyncio
from collections.abc import Callable

from .config import ConfigStore, ProviderConfig
from .openai_compatible import OpenAICompatibleAdapter
from .providers import ProviderAdapter, ScriptedProvider

AdapterFactory = Callable[[ProviderConfig], OpenAICompatibleAdapter]


class RuntimeProviderRegistry:
    def __init__(
        self,
        config_store: ConfigStore,
        *,
        adapter_factory: AdapterFactory = OpenAICompatibleAdapter,
    ) -> None:
        self._config_store = config_store
        self._scripted = ScriptedProvider()
        self._adapter_factory = adapter_factory
        self._adapters: dict[str, tuple[ProviderConfig, OpenAICompatibleAdapter]] = {}
        self._retired: list[OpenAICompatibleAdapter] = []
        self._close_tasks: set[asyncio.Task[None]] = set()

    def resolve(self, provider_id: str) -> ProviderAdapter | None:
        if provider_id == self._scripted.id:
            return self._scripted
        provider = self._config_store.get_provider(provider_id)
        if provider is None or not provider.configured:
            return None
        cached = self._adapters.get(provider_id)
        if cached is not None and cached[0] == provider:
            return cached[1]
        if cached is not None:
            self._retire(cached[1])
        adapter = self._adapter_factory(provider)
        self._adapters[provider_id] = (provider, adapter)
        return adapter

    def configuration_changed(self, provider_id: str) -> None:
        cached = self._adapters.pop(provider_id, None)
        if cached is not None:
            self._retire(cached[1])

    async def close(self) -> None:
        adapters = [adapter for _provider, adapter in self._adapters.values()]
        adapters.extend(self._retired)
        self._adapters.clear()
        self._retired.clear()
        if adapters:
            await asyncio.gather(*(adapter.aclose() for adapter in adapters))
        if self._close_tasks:
            await asyncio.gather(*tuple(self._close_tasks), return_exceptions=True)
            self._close_tasks.clear()

    def _retire(self, adapter: OpenAICompatibleAdapter) -> None:
        try:
            loop = asyncio.get_running_loop()
        except RuntimeError:
            self._retired.append(adapter)
            return
        task = loop.create_task(adapter.aclose(), name="ikaros-provider-client-close")
        self._close_tasks.add(task)
        task.add_done_callback(self._close_tasks.discard)
