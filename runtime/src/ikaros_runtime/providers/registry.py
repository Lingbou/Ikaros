from __future__ import annotations

import asyncio
import os
import re
import tempfile
from collections.abc import Callable, Mapping, Sequence
from contextlib import suppress
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol
from urllib.parse import urlsplit, urlunsplit

import yaml

from ..errors import ConfigError
from ..security import contains_protected_value
from .base import (
    ModelConfig,
    ModelInput,
    ProviderAdapter,
    ProviderConfig,
)
from .scripted import ScriptedProvider

CONFIG_VERSION = 1
DEEPSEEK_PROVIDER_ID = "deepseek"
DEEPSEEK_DISPLAY_NAME = "DeepSeek"
DEEPSEEK_BASE_URL = "https://api.deepseek.com/v1"
_MAX_CONFIG_BYTES = 256 * 1024
_MAX_SECRET_LENGTH = 8192
_PROVIDER_ID = re.compile(r"^[a-z0-9][a-z0-9._-]{0,63}$")
_HEADER_NAME = re.compile(r"^[!#$%&'*+.^_`|~0-9A-Za-z-]+$")
_RESERVED_PROVIDER_IDS = frozenset({DEEPSEEK_PROVIDER_ID, "scripted"})
_TRANSPORT_HEADERS = frozenset(
    {"accept", "connection", "content-length", "content-type", "host", "transfer-encoding"}
)


class _UniqueKeyLoader(yaml.SafeLoader):
    """Safe YAML loader that rejects mappings whose keys would be overwritten."""


def _construct_unique_mapping(
    loader: _UniqueKeyLoader,
    node: yaml.nodes.MappingNode,
    deep: bool = False,
) -> dict[object, object]:
    loader.flatten_mapping(node)
    mapping: dict[object, object] = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        try:
            duplicate = key in mapping
        except TypeError as error:
            raise yaml.constructor.ConstructorError(
                "while constructing a mapping",
                node.start_mark,
                "found an unhashable key",
                key_node.start_mark,
            ) from error
        if duplicate:
            raise yaml.constructor.ConstructorError(
                "while constructing a mapping",
                node.start_mark,
                "found a duplicate key",
                key_node.start_mark,
            )
        mapping[key] = loader.construct_object(value_node, deep=deep)
    return mapping


_UniqueKeyLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG,
    _construct_unique_mapping,
)


@dataclass(frozen=True, slots=True)
class ProviderSummary:
    id: str
    display_name: str
    origin: str
    configured: bool
    credential_configured: bool
    health: str = "unknown"

    def to_wire(self) -> dict[str, object]:
        return {
            "id": self.id,
            "displayName": self.display_name,
            "origin": self.origin,
            "configured": self.configured,
            "credentialConfigured": self.credential_configured,
            "health": self.health,
        }


def deepseek_discovery_provider(api_key: object) -> ProviderConfig:
    """Build a validated, non-persistent DeepSeek transport configuration."""

    normalized_key = _secret(api_key, required=True)
    assert normalized_key is not None
    return ProviderConfig(
        id=DEEPSEEK_PROVIDER_ID,
        display_name=DEEPSEEK_DISPLAY_NAME,
        origin="builtin",
        base_url=DEEPSEEK_BASE_URL,
        api_key=normalized_key,
        headers=(),
        models=(),
    )


@dataclass(frozen=True, slots=True)
class ModelSummary:
    provider_id: str
    id: str
    display_name: str
    enabled: bool

    def to_wire(self) -> dict[str, object]:
        return {
            "providerId": self.provider_id,
            "id": self.id,
            "displayName": self.display_name,
            "enabled": self.enabled,
        }


class ConfigStore:
    def __init__(self, runtime_home: Path) -> None:
        self._runtime_home = runtime_home
        self._path = runtime_home / "config.yaml"
        self._providers = self._load()

    @property
    def path(self) -> Path:
        return self._path

    def provider_summaries(self) -> tuple[ProviderSummary, ...]:
        deepseek = self._providers.get(DEEPSEEK_PROVIDER_ID)
        summaries = [
            ProviderSummary(
                id=DEEPSEEK_PROVIDER_ID,
                display_name=DEEPSEEK_DISPLAY_NAME,
                origin="builtin",
                configured=deepseek.configured if deepseek is not None else False,
                credential_configured=(
                    deepseek.api_key is not None if deepseek is not None else False
                ),
            )
        ]
        summaries.extend(
            ProviderSummary(
                id=provider.id,
                display_name=provider.display_name,
                origin="custom",
                configured=True,
                credential_configured=provider.api_key is not None or bool(provider.headers),
            )
            for provider in sorted(self._providers.values(), key=lambda item: item.id)
            if provider.origin == "custom"
        )
        return tuple(summaries)

    def model_summaries(self) -> tuple[ModelSummary, ...]:
        return tuple(
            ModelSummary(provider.id, model.id, model.display_name, model.enabled)
            for provider in sorted(self._providers.values(), key=lambda item: item.id)
            for model in provider.models
        )

    def get_provider(self, provider_id: str) -> ProviderConfig | None:
        return self._providers.get(provider_id)

    def protected_values(self) -> tuple[str, ...]:
        return _protected_values(self._providers)

    def resolve_model(self, provider_id: str, model_id: str) -> tuple[ProviderConfig, ModelConfig]:
        provider = self._providers.get(provider_id)
        if provider is None or not provider.configured:
            raise ConfigError("the selected provider is not configured")
        model = next((candidate for candidate in provider.models if candidate.id == model_id), None)
        if model is None:
            raise ConfigError("the selected model does not exist")
        if not model.enabled:
            raise ConfigError("the selected model is disabled")
        return provider, model

    def configure_deepseek(
        self,
        *,
        api_key: str,
        models: Sequence[ModelInput],
    ) -> ProviderSummary:
        normalized_key = _secret(api_key, required=True)
        assert normalized_key is not None
        existing = self._providers.get(DEEPSEEK_PROVIDER_ID)
        provider = ProviderConfig(
            id=DEEPSEEK_PROVIDER_ID,
            display_name=DEEPSEEK_DISPLAY_NAME,
            origin="builtin",
            base_url=DEEPSEEK_BASE_URL,
            api_key=normalized_key,
            headers=(),
            models=_models(models, existing.models if existing is not None else ()),
        )
        self._replace(provider)
        return self._summary(provider)

    def configure_custom(
        self,
        *,
        provider_id: str,
        display_name: str,
        base_url: str,
        api_key: str | None,
        headers: Mapping[str, str] | None,
        models: Sequence[ModelInput],
    ) -> ProviderSummary:
        normalized_id = _custom_provider_id(provider_id)
        normalized_key = _secret(api_key, required=False)
        existing = self._providers.get(normalized_id)
        provider = ProviderConfig(
            id=normalized_id,
            display_name=_display_name(display_name, "provider display name"),
            origin="custom",
            base_url=_base_url(base_url),
            api_key=normalized_key,
            headers=_headers(headers, has_api_key=normalized_key is not None),
            models=_models(models, existing.models if existing is not None else ()),
        )
        self._replace(provider)
        return self._summary(provider)

    def disconnect_deepseek(self) -> ProviderSummary:
        provider = self._providers.get(DEEPSEEK_PROVIDER_ID)
        if provider is None:
            return self.provider_summaries()[0]
        updated = dict(self._providers)
        del updated[DEEPSEEK_PROVIDER_ID]
        self._persist(updated)
        self._providers = updated
        return self.provider_summaries()[0]

    def remove_custom(self, provider_id: str) -> None:
        normalized_id = _custom_provider_id(provider_id)
        provider = self._providers.get(normalized_id)
        if provider is None or provider.origin != "custom":
            raise ConfigError("custom provider does not exist")
        updated = dict(self._providers)
        del updated[normalized_id]
        self._persist(updated)
        self._providers = updated

    def set_model_enabled(self, provider_id: str, model_id: str, enabled: bool) -> ModelSummary:
        provider = self._providers.get(provider_id)
        if provider is None:
            raise ConfigError("provider does not exist")
        found = False
        models: list[ModelConfig] = []
        for model in provider.models:
            if model.id == model_id:
                found = True
                models.append(
                    ModelConfig(
                        id=model.id,
                        display_name=model.display_name,
                        enabled=enabled,
                        supports_tools=model.supports_tools,
                    )
                )
            else:
                models.append(model)
        if not found:
            raise ConfigError("model does not exist")
        updated = ProviderConfig(
            id=provider.id,
            display_name=provider.display_name,
            origin=provider.origin,
            base_url=provider.base_url,
            api_key=provider.api_key,
            headers=provider.headers,
            models=tuple(models),
        )
        self._replace(updated)
        model = next(candidate for candidate in updated.models if candidate.id == model_id)
        return ModelSummary(updated.id, model.id, model.display_name, model.enabled)

    def _replace(self, provider: ProviderConfig) -> None:
        updated = dict(self._providers)
        updated[provider.id] = provider
        self._persist(updated)
        self._providers = updated

    @staticmethod
    def _summary(provider: ProviderConfig) -> ProviderSummary:
        return ProviderSummary(
            id=provider.id,
            display_name=provider.display_name,
            origin=provider.origin,
            configured=provider.configured,
            credential_configured=provider.api_key is not None or bool(provider.headers),
        )

    def _load(self) -> dict[str, ProviderConfig]:
        if not self._path.exists():
            return {}
        load_failed = False
        try:
            if self._path.stat().st_size > _MAX_CONFIG_BYTES:
                raise ConfigError("config.yaml exceeds the supported size")
            source = self._path.read_text(encoding="utf-8")
            document = yaml.load(source, Loader=_UniqueKeyLoader)
        except ConfigError:
            raise
        except (OSError, UnicodeError, yaml.YAMLError):
            load_failed = True
        if load_failed:
            raise ConfigError("config.yaml could not be read or parsed")
        try:
            return _parse_document(document)
        except ConfigError:
            raise
        except Exception:
            raise ConfigError("config.yaml has an invalid structure") from None

    def _persist(self, providers: Mapping[str, ProviderConfig]) -> None:
        _validate_public_fields(
            providers,
            additional_protected_values=self.protected_values(),
        )
        if not providers:
            try:
                self._path.unlink(missing_ok=True)
            except OSError:
                raise ConfigError("config.yaml could not be updated") from None
            return
        document = _document(providers)
        try:
            serialized = yaml.safe_dump(
                document,
                allow_unicode=True,
                default_flow_style=False,
                sort_keys=False,
            )
        except yaml.YAMLError:
            raise ConfigError("config.yaml could not be serialized") from None
        try:
            serialized_size = len(serialized.encode("utf-8"))
        except UnicodeError:
            raise ConfigError("config.yaml could not be serialized") from None
        if serialized_size > _MAX_CONFIG_BYTES:
            raise ConfigError("config.yaml exceeds the supported size")
        temporary_path: Path | None = None
        try:
            self._runtime_home.mkdir(parents=True, exist_ok=True, mode=0o700)
            if os.name != "nt":
                os.chmod(self._runtime_home, 0o700)
            descriptor, temporary_name = tempfile.mkstemp(
                prefix=".config-",
                suffix=".tmp",
                dir=self._runtime_home,
            )
            temporary_path = Path(temporary_name)
            try:
                os.chmod(temporary_path, 0o600)
                with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as handle:
                    descriptor = -1
                    handle.write(serialized)
                    handle.flush()
                    os.fsync(handle.fileno())
            finally:
                if descriptor >= 0:
                    os.close(descriptor)
            os.replace(temporary_path, self._path)
            temporary_path = None
            _sync_directory_best_effort(self._runtime_home)
        except OSError:
            raise ConfigError("config.yaml could not be updated") from None
        finally:
            if temporary_path is not None:
                with suppress(OSError):
                    temporary_path.unlink(missing_ok=True)


def _sync_directory_best_effort(directory: Path) -> None:
    if os.name == "nt":
        return
    try:
        descriptor = os.open(directory, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    except OSError:
        pass


def _document(providers: Mapping[str, ProviderConfig]) -> dict[str, object]:
    rows: dict[str, object] = {}
    for provider in sorted(providers.values(), key=lambda item: item.id):
        models = {
            model.id: {
                "display_name": model.display_name,
                "enabled": model.enabled,
                "supports_tools": model.supports_tools,
            }
            for model in provider.models
        }
        if provider.origin == "builtin":
            row: dict[str, object] = {
                "type": "openai_compatible",
                "preset": "deepseek",
                "base_url": DEEPSEEK_BASE_URL,
            }
        else:
            row = {
                "type": "openai_compatible",
                "display_name": provider.display_name,
                "base_url": provider.base_url,
            }
        if provider.api_key is not None:
            row["api_key"] = provider.api_key
        row["models"] = models
        if provider.headers:
            row["headers"] = dict(provider.headers)
        rows[provider.id] = row
    return {"version": CONFIG_VERSION, "providers": rows}


def _parse_document(value: Any) -> dict[str, ProviderConfig]:
    if not isinstance(value, dict) or set(value) != {"version", "providers"}:
        raise ConfigError("config.yaml has an invalid top-level structure")
    if value["version"] != CONFIG_VERSION or isinstance(value["version"], bool):
        raise ConfigError("config.yaml uses an unsupported version")
    rows = value["providers"]
    if not isinstance(rows, dict):
        raise ConfigError("config.yaml providers must be a mapping")
    providers: dict[str, ProviderConfig] = {}
    for raw_id, raw_provider in rows.items():
        if not isinstance(raw_id, str) or not isinstance(raw_provider, dict):
            raise ConfigError("config.yaml contains an invalid provider record")
        provider = _parse_provider(raw_id, raw_provider)
        if provider.id in providers:
            raise ConfigError("provider IDs must be unique after normalization")
        providers[provider.id] = provider
    _validate_public_fields(providers)
    return providers


def _protected_values(providers: Mapping[str, ProviderConfig]) -> tuple[str, ...]:
    values: list[str] = []
    seen: set[str] = set()
    for provider in providers.values():
        candidates = (
            provider.api_key,
            *(header_value for _header_name, header_value in provider.headers),
        )
        for candidate in candidates:
            if candidate and candidate not in seen:
                seen.add(candidate)
                values.append(candidate)
    return tuple(values)


def _validate_public_fields(
    providers: Mapping[str, ProviderConfig],
    additional_protected_values: Sequence[str] = (),
) -> None:
    protected_values = tuple(
        dict.fromkeys((*_protected_values(providers), *additional_protected_values))
    )
    if not protected_values:
        return
    public_values = [
        DEEPSEEK_PROVIDER_ID,
        DEEPSEEK_DISPLAY_NAME,
        "builtin",
        "unknown",
    ]
    for provider in providers.values():
        public_values.extend(
            (provider.id, provider.display_name, provider.origin, provider.base_url)
        )
        for model in provider.models:
            public_values.extend((model.id, model.display_name))
    if any(contains_protected_value(value, protected_values) for value in public_values):
        raise ConfigError("provider public fields must not contain credential values")


def _parse_provider(provider_id: str, value: dict[Any, Any]) -> ProviderConfig:
    if provider_id == DEEPSEEK_PROVIDER_ID:
        allowed = {"type", "preset", "base_url", "api_key", "models"}
        if set(value) - allowed:
            raise ConfigError("DeepSeek configuration contains unsupported fields")
        if (
            value.get("type") != "openai_compatible"
            or value.get("preset") != "deepseek"
            or value.get("base_url") != DEEPSEEK_BASE_URL
        ):
            raise ConfigError("DeepSeek configuration does not match the built-in preset")
        api_key = _secret(value.get("api_key"), required=False)
        return ProviderConfig(
            id=DEEPSEEK_PROVIDER_ID,
            display_name=DEEPSEEK_DISPLAY_NAME,
            origin="builtin",
            base_url=DEEPSEEK_BASE_URL,
            api_key=api_key,
            headers=(),
            models=_parse_models(value.get("models")),
        )
    normalized_id = _custom_provider_id(provider_id)
    allowed = {"type", "display_name", "base_url", "api_key", "models", "headers"}
    if set(value) - allowed or value.get("type") != "openai_compatible":
        raise ConfigError("custom Provider configuration contains unsupported fields")
    api_key = _secret(value.get("api_key"), required=False)
    return ProviderConfig(
        id=normalized_id,
        display_name=_display_name(value.get("display_name"), "provider display name"),
        origin="custom",
        base_url=_base_url(value.get("base_url")),
        api_key=api_key,
        headers=_headers(value.get("headers"), has_api_key=api_key is not None),
        models=_parse_models(value.get("models")),
    )


def _parse_models(value: Any) -> tuple[ModelConfig, ...]:
    if not isinstance(value, dict) or not value:
        raise ConfigError("provider models must be a non-empty mapping")
    models: list[ModelConfig] = []
    seen: set[str] = set()
    for raw_id, raw_model in value.items():
        if not isinstance(raw_id, str) or not isinstance(raw_model, dict):
            raise ConfigError("provider contains an invalid model record")
        if set(raw_model) != {"display_name", "enabled", "supports_tools"}:
            raise ConfigError("model configuration contains unsupported fields")
        model_id = _model_id(raw_id)
        if model_id in seen:
            raise ConfigError("model IDs must be unique after normalization")
        seen.add(model_id)
        enabled = raw_model["enabled"]
        supports_tools = raw_model["supports_tools"]
        if not isinstance(enabled, bool) or not isinstance(supports_tools, bool):
            raise ConfigError("model flags must be booleans")
        models.append(
            ModelConfig(
                id=model_id,
                display_name=_display_name(raw_model["display_name"], "model display name"),
                enabled=enabled,
                supports_tools=supports_tools,
            )
        )
    return tuple(models)


def _models(
    inputs: Sequence[ModelInput],
    existing: Sequence[ModelConfig] = (),
) -> tuple[ModelConfig, ...]:
    if not inputs:
        raise ConfigError("at least one model is required")
    models: list[ModelConfig] = []
    seen: set[str] = set()
    existing_by_id = {model.id: model for model in existing}
    for item in inputs:
        model_id = _model_id(item.id)
        if model_id in seen:
            raise ConfigError("model IDs must be unique")
        seen.add(model_id)
        previous = existing_by_id.get(model_id)
        models.append(
            ModelConfig(
                id=model_id,
                display_name=_display_name(item.display_name, "model display name"),
                enabled=previous.enabled if previous is not None else True,
                supports_tools=previous.supports_tools if previous is not None else True,
            )
        )
    return tuple(models)


def _custom_provider_id(value: Any) -> str:
    if not isinstance(value, str):
        raise ConfigError("provider ID must be a string")
    normalized = value.strip().lower()
    if normalized in _RESERVED_PROVIDER_IDS or _PROVIDER_ID.fullmatch(normalized) is None:
        raise ConfigError("provider ID must be a valid, non-reserved identifier")
    return normalized


def _model_id(value: Any) -> str:
    if not isinstance(value, str):
        raise ConfigError("model ID must be a string")
    normalized = value.strip()
    if (
        not normalized
        or len(normalized) > 200
        or any(ord(character) < 32 or ord(character) == 127 for character in normalized)
    ):
        raise ConfigError("model ID must be a non-empty printable string of at most 200 characters")
    return normalized


def _display_name(value: Any, label: str) -> str:
    if not isinstance(value, str):
        raise ConfigError(f"{label} must be a string")
    normalized = value.strip()
    if (
        not normalized
        or len(normalized) > 200
        or any(ord(character) < 32 or ord(character) == 127 for character in normalized)
    ):
        raise ConfigError(f"{label} must be a non-empty printable string of at most 200 characters")
    return normalized


def _secret(value: Any, *, required: bool) -> str | None:
    if value is None or value == "":
        if required:
            raise ConfigError("API key is required")
        return None
    if not isinstance(value, str):
        raise ConfigError("API key must be a string")
    if (
        value != value.strip()
        or len(value) > _MAX_SECRET_LENGTH
        or _contains_control_character(value)
    ):
        raise ConfigError("API key has an invalid format")
    return value


def _base_url(value: Any) -> str:
    if not isinstance(value, str):
        raise ConfigError("Base URL must be a string")
    normalized = value.strip().rstrip("/")
    if len(normalized) > 2048:
        raise ConfigError("Base URL is too long")
    if _contains_control_character(normalized):
        raise ConfigError("Base URL has an invalid format")
    parse_failed = False
    parsed: (
        tuple[
            str,
            str | None,
            str | None,
            str | None,
            str,
            str,
            str,
        ]
        | None
    ) = None
    try:
        parts = urlsplit(normalized)
        parsed = (
            parts.scheme,
            parts.hostname,
            parts.username,
            parts.password,
            parts.query,
            parts.fragment,
            urlunsplit((parts.scheme.lower(), parts.netloc, parts.path.rstrip("/"), "", "")),
        )
        _ = parts.port
    except (UnicodeError, ValueError):
        parse_failed = True
    if parse_failed or parsed is None:
        raise ConfigError("Base URL must be an HTTP(S) origin or path without credentials or query")
    scheme, hostname, username, password, query, fragment, rebuilt = parsed
    if (
        scheme not in {"http", "https"}
        or not hostname
        or username is not None
        or password is not None
        or query
        or fragment
    ):
        raise ConfigError("Base URL must be an HTTP(S) origin or path without credentials or query")
    return rebuilt


def _headers(value: Any, *, has_api_key: bool) -> tuple[tuple[str, str], ...]:
    if value is None:
        return ()
    if not isinstance(value, Mapping):
        raise ConfigError("custom headers must be a string mapping")
    normalized: list[tuple[str, str]] = []
    names: set[str] = set()
    for raw_name, raw_value in value.items():
        if not isinstance(raw_name, str) or not isinstance(raw_value, str):
            raise ConfigError("custom headers must contain only string names and values")
        lowered = raw_name.lower()
        if (
            _HEADER_NAME.fullmatch(raw_name) is None
            or lowered in names
            or lowered in _TRANSPORT_HEADERS
            or (has_api_key and lowered == "authorization")
            or _contains_control_character(raw_value)
            or len(raw_name) > 200
            or len(raw_value) > _MAX_SECRET_LENGTH
        ):
            raise ConfigError("custom headers contain a duplicate, reserved, or invalid entry")
        names.add(lowered)
        normalized.append((raw_name, raw_value))
    return tuple(normalized)


def _contains_control_character(value: str) -> bool:
    return any(ord(character) < 32 or ord(character) == 127 for character in value)


class ManagedProviderAdapter(ProviderAdapter, Protocol):
    async def aclose(self) -> None: ...


AdapterFactory = Callable[[ProviderConfig], ManagedProviderAdapter]


class RuntimeProviderRegistry:
    def __init__(
        self,
        config_store: ConfigStore,
        *,
        adapter_factory: AdapterFactory,
    ) -> None:
        self._config_store = config_store
        self._scripted = ScriptedProvider()
        self._adapter_factory = adapter_factory
        self._adapters: dict[str, tuple[ProviderConfig, ManagedProviderAdapter]] = {}
        self._retired: list[ManagedProviderAdapter] = []
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

    def _retire(self, adapter: ManagedProviderAdapter) -> None:
        try:
            loop = asyncio.get_running_loop()
        except RuntimeError:
            self._retired.append(adapter)
            return
        task = loop.create_task(adapter.aclose(), name="ikaros-provider-client-close")
        self._close_tasks.add(task)
        task.add_done_callback(self._close_tasks.discard)


__all__ = [
    "DEEPSEEK_BASE_URL",
    "DEEPSEEK_DISPLAY_NAME",
    "DEEPSEEK_PROVIDER_ID",
    "ConfigStore",
    "ModelConfig",
    "ModelInput",
    "ModelSummary",
    "ProviderConfig",
    "ProviderSummary",
    "RuntimeProviderRegistry",
    "deepseek_discovery_provider",
]
