from __future__ import annotations

from pathlib import Path

import httpx
import pytest
import yaml

from ikaros_runtime.providers.base import ModelInput, ProviderConfig
from ikaros_runtime.providers.openai_compatible.discovery import discover_openai_compatible_models
from ikaros_runtime.providers.registry import (
    ConfigStore,
    ManagedProviderAdapter,
    RuntimeProviderRegistry,
    deepseek_discovery_provider,
)
from ikaros_runtime.services.providers import ProviderService
from ikaros_runtime.storage import SqliteRuntimeStore


@pytest.mark.parametrize(
    "model_id,explicit_limits,expected",
    [
        ("deepseek-v4-flash", {}, (1_000_000, 64_000)),
        ("deepseek-v4-pro", {}, (1_000_000, 64_000)),
        ("deepseek-v4-flash-vision-exp", {}, (1_000_000, 64_000)),
        ("unknown-local-model", {}, (32_768, 4_096)),
        (
            "deepseek-v4-pro",
            {"context_window": 131_072, "max_output_tokens": 16_384},
            (131_072, 16_384),
        ),
    ],
)
def test_loading_model_capacity_defaults_keeps_explicit_values_and_file(
    tmp_path: Path,
    model_id: str,
    explicit_limits: dict[str, int],
    expected: tuple[int, int],
) -> None:
    path = tmp_path / "config.yaml"
    path.write_text(
        yaml.safe_dump(
            {
                "version": 1,
                "providers": {
                    "local": {
                        "type": "openai_compatible",
                        "display_name": "Local",
                        "base_url": "http://localhost:8080/v1",
                        "models": {
                            model_id: {
                                "display_name": model_id,
                                "enabled": True,
                                "supports_tools": True,
                                **explicit_limits,
                            }
                        },
                    }
                },
            }
        ),
        encoding="utf-8",
    )
    original = path.read_bytes()

    model = ConfigStore(tmp_path).model_summaries()[0]

    assert (model.context_window, model.max_output_tokens) == expected
    assert path.read_bytes() == original


@pytest.mark.asyncio
async def test_discovery_assigns_known_capacity_and_unknown_fallback() -> None:
    async def respond(_request: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            json={
                "data": [
                    {"id": "deepseek-v4-flash"},
                    {"id": "deepseek-v4-pro"},
                    {"id": "deepseek-v4-flash-vision-exp"},
                    {"id": "unknown-local-model"},
                ]
            },
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(respond)) as client:
        discovered = await discover_openai_compatible_models(
            deepseek_discovery_provider("sk-discovery-fixture"), client=client
        )

    assert {item.id: (item.context_window, item.max_output_tokens) for item in discovered} == {
        "deepseek-v4-flash": (1_000_000, 64_000),
        "deepseek-v4-pro": (1_000_000, 64_000),
        "deepseek-v4-flash-vision-exp": (1_000_000, 64_000),
        "unknown-local-model": (32_768, 4_096),
    }


@pytest.mark.asyncio
async def test_rediscovery_preserves_saved_capacity_but_provider_edit_can_change_it(
    tmp_path: Path,
) -> None:
    config = ConfigStore(tmp_path)
    config.configure_deepseek(
        api_key="sk-existing-fixture",
        models=[ModelInput("deepseek-v4-flash", "Flash", 131_072, 16_384)],
    )
    original = config.path.read_bytes()

    async def discover(_provider: ProviderConfig) -> tuple[ModelInput, ...]:
        return (
            ModelInput("deepseek-v4-flash", "Flash", 1_000_000, 64_000),
            ModelInput("deepseek-v4-pro", "Pro", 1_000_000, 64_000),
        )

    def forbid_adapter(_provider: ProviderConfig) -> ManagedProviderAdapter:
        raise AssertionError("discovery must not start a model request")

    registry = RuntimeProviderRegistry(config, adapter_factory=forbid_adapter)
    store = SqliteRuntimeStore(tmp_path / "state.db")
    service = ProviderService(config, registry, store, discover, lambda _values: None)
    try:
        result = await service.discover_provider_models(
            {"kind": "deepseek", "apiKey": "sk-replacement-fixture"}
        )
        assert result == {
            "models": [
                {
                    "id": "deepseek-v4-flash",
                    "displayName": "Flash",
                    "contextWindow": 131_072,
                    "maxOutputTokens": 16_384,
                },
                {
                    "id": "deepseek-v4-pro",
                    "displayName": "Pro",
                    "contextWindow": 1_000_000,
                    "maxOutputTokens": 64_000,
                },
            ]
        }
        assert config.path.read_bytes() == original

        service.configure_provider(
            {
                "kind": "deepseek",
                "apiKey": "sk-replacement-fixture",
                "models": [
                    {
                        "id": "deepseek-v4-flash",
                        "displayName": "Flash",
                        "contextWindow": 262_144,
                        "maxOutputTokens": 32_768,
                    }
                ],
            }
        )
        updated = ConfigStore(tmp_path).model_summaries()[0]
        assert (updated.context_window, updated.max_output_tokens) == (262_144, 32_768)
    finally:
        await registry.close()
        store.close()
