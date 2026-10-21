"""Runtime-owned Telegram channel configuration.

Channel settings are intentionally isolated from ``config.yaml`` so adding a
messaging channel never invalidates an existing provider configuration.
"""

from __future__ import annotations

import os
import tempfile
from contextlib import suppress
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import urlsplit

import yaml

from ..errors import ConfigError
from ..paths import RuntimePaths

CHANNEL_CONFIG_VERSION = 1
MAX_CHANNEL_CONFIG_BYTES = 64 * 1024
_TELEGRAM_KEY = "telegram"
_TELEGRAM_KEYS = frozenset(
    {"enabled", "token", "proxy", "defaultProviderId", "defaultModelId"}
)


@dataclass(frozen=True, slots=True)
class TelegramConfig:
    enabled: bool = False
    token: str | None = field(default=None, repr=False)
    proxy: str | None = field(default=None, repr=False)
    default_provider_id: str | None = None
    default_model_id: str | None = None

    def public_wire(self) -> dict[str, object]:
        return {
            "enabled": self.enabled,
            "configured": self.token is not None,
            "proxyConfigured": self.proxy is not None,
            "defaultProviderId": self.default_provider_id,
            "defaultModelId": self.default_model_id,
        }


class TelegramConfigStore:
    """Load and atomically replace the versioned Telegram channel document."""

    def __init__(self, runtime_home: Path) -> None:
        self._path = RuntimePaths.from_home(runtime_home).channels_config
        self._config = self._load()

    @property
    def path(self) -> Path:
        return self._path

    @property
    def config(self) -> TelegramConfig:
        return self._config

    def reload(self) -> TelegramConfig:
        self._config = self._load()
        return self._config

    def save(self, config: TelegramConfig) -> TelegramConfig:
        normalized = _normalize(config)
        if normalized == TelegramConfig():
            with suppress(FileNotFoundError):
                self._path.unlink()
            self._config = normalized
            return normalized

        telegram: dict[str, object] = {
            "enabled": normalized.enabled,
            "defaultProviderId": normalized.default_provider_id,
            "defaultModelId": normalized.default_model_id,
        }
        if normalized.token is not None:
            telegram["token"] = normalized.token
        if normalized.proxy is not None:
            telegram["proxy"] = normalized.proxy
        document = {"version": CHANNEL_CONFIG_VERSION, _TELEGRAM_KEY: telegram}
        self._atomic_write(_serialize(document))
        self._config = normalized
        return normalized

    def protected_values(self) -> tuple[str, ...]:
        config = self._config
        return tuple(
            value
            for value in (config.token, config.proxy)
            if isinstance(value, str) and value
        )

    def _load(self) -> TelegramConfig:
        if not self._path.exists():
            return TelegramConfig()

        try:
            with self._path.open("rb") as handle:
                source = handle.read(MAX_CHANNEL_CONFIG_BYTES + 1)
        except OSError:
            raise ConfigError("channels.yaml could not be read") from None
        if len(source) > MAX_CHANNEL_CONFIG_BYTES:
            raise ConfigError("channels.yaml exceeds the supported size")

        try:
            document = yaml.safe_load(source.decode("utf-8"))
        except (UnicodeError, yaml.YAMLError):
            raise ConfigError("channels.yaml could not be parsed") from None

        if document is None:
            return TelegramConfig()
        if not isinstance(document, dict):
            raise ConfigError("channels.yaml has an invalid top-level structure")
        if set(document) - {"version", _TELEGRAM_KEY}:
            raise ConfigError("channels.yaml has an invalid top-level structure")
        version = document.get("version")
        if version != CHANNEL_CONFIG_VERSION or isinstance(version, bool):
            raise ConfigError("channels.yaml uses an unsupported version")

        raw = document.get(_TELEGRAM_KEY)
        if raw is None:
            return TelegramConfig()
        if not isinstance(raw, dict) or set(raw) - _TELEGRAM_KEYS:
            raise ConfigError("channels.yaml telegram section is invalid")
        return _normalize(
            TelegramConfig(
                enabled=raw.get("enabled", False),
                token=raw.get("token"),
                proxy=raw.get("proxy"),
                default_provider_id=raw.get("defaultProviderId"),
                default_model_id=raw.get("defaultModelId"),
            )
        )

    def _atomic_write(self, serialized: bytes) -> None:
        temporary_path: Path | None = None
        descriptor = -1
        try:
            self._path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            if os.name != "nt":
                os.chmod(self._path.parent, 0o700)
            descriptor, temporary_name = tempfile.mkstemp(
                prefix=".channels-",
                suffix=".tmp",
                dir=self._path.parent,
            )
            temporary_path = Path(temporary_name)
            if os.name != "nt":
                os.chmod(temporary_path, 0o600)
            with os.fdopen(descriptor, "wb") as handle:
                descriptor = -1
                handle.write(serialized)
                handle.flush()
                os.fsync(handle.fileno())
            os.replace(temporary_path, self._path)
            temporary_path = None
            _sync_directory_best_effort(self._path.parent)
        except OSError:
            raise ConfigError("channels.yaml could not be updated") from None
        finally:
            if descriptor >= 0:
                with suppress(OSError):
                    os.close(descriptor)
            if temporary_path is not None:
                with suppress(OSError):
                    temporary_path.unlink(missing_ok=True)


def _normalize(config: TelegramConfig) -> TelegramConfig:
    enabled = config.enabled
    if not isinstance(enabled, bool):
        raise ConfigError("telegram enabled must be a boolean")
    token = _optional_text(config.token, "telegram bot token", 4096)
    if token is not None and any(character.isspace() for character in token):
        raise ConfigError("telegram bot token must not contain whitespace")
    proxy = _optional_text(config.proxy, "telegram proxy", 4096)
    if proxy is not None:
        parsed = urlsplit(proxy)
        if parsed.scheme not in {"http", "https"} or not parsed.hostname:
            raise ConfigError("telegram proxy must be an HTTP or HTTPS URL")
    provider_id = _optional_text(
        config.default_provider_id,
        "telegram default provider",
        200,
    )
    model_id = _optional_text(config.default_model_id, "telegram default model", 200)
    if (provider_id is None) != (model_id is None):
        raise ConfigError("telegram default provider and model must be set together")
    if enabled and token is None:
        raise ConfigError("telegram cannot be enabled without a bot token")
    if enabled and provider_id is None:
        raise ConfigError("telegram cannot be enabled without a default model")
    return TelegramConfig(
        enabled=enabled,
        token=token,
        proxy=proxy,
        default_provider_id=provider_id,
        default_model_id=model_id,
    )


def _optional_text(value: object, name: str, maximum: int) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise ConfigError(f"{name} must be a string")
    normalized = value.strip()
    if not normalized:
        return None
    if len(normalized) > maximum or any(ord(character) < 0x20 for character in normalized):
        raise ConfigError(f"{name} is invalid")
    return normalized


def _serialize(document: dict[str, object]) -> bytes:
    try:
        serialized = yaml.safe_dump(
            document,
            allow_unicode=True,
            default_flow_style=False,
            sort_keys=False,
        ).encode("utf-8")
    except (UnicodeError, yaml.YAMLError):
        raise ConfigError("channels.yaml could not be serialized") from None
    if len(serialized) > MAX_CHANNEL_CONFIG_BYTES:
        raise ConfigError("channels.yaml exceeds the supported size")
    return serialized


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


__all__ = [
    "CHANNEL_CONFIG_VERSION",
    "MAX_CHANNEL_CONFIG_BYTES",
    "TelegramConfig",
    "TelegramConfigStore",
]
