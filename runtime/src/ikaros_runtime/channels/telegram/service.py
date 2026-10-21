"""RPC-facing facade for the Telegram channel."""

from __future__ import annotations

from collections.abc import Callable
from typing import Any

from ...errors import ConfigError, InvalidParamsError
from ...providers.registry import ConfigStore
from ..config import TelegramConfig, TelegramConfigStore
from ..store import ChannelStore
from .adapter import CHANNEL_ID, TelegramChannel
from .client import (
    HttpTelegramBotApi,
    TelegramApiError,
    TelegramBotApi,
    TelegramBotIdentity,
)

type ApiFactory = Callable[[TelegramConfig], TelegramBotApi]


class ChannelService:
    """Expose channel status and configuration without leaking credentials."""

    def __init__(
        self,
        *,
        channel: TelegramChannel,
        config_store: TelegramConfigStore,
        channel_store: ChannelStore,
        provider_store: ConfigStore,
        api_factory: ApiFactory | None = None,
    ) -> None:
        self._channel = channel
        self._config_store = config_store
        self._channel_store = channel_store
        self._provider_store = provider_store
        self._api_factory = api_factory or _http_api_factory

    async def start(self) -> None:
        await self._channel.start()

    async def close(self) -> None:
        await self._channel.close()

    def list_channels(self, params: dict[str, Any]) -> dict[str, object]:
        if params:
            raise InvalidParamsError("channel.list does not accept parameters")
        return {"channels": [self._channel.public_wire()]}

    async def configure(self, params: dict[str, Any]) -> dict[str, object]:
        allowed = {
            "channelId",
            "enabled",
            "token",
            "proxy",
            "defaultProviderId",
            "defaultModelId",
        }
        if set(params) - allowed or params.get("channelId") != CHANNEL_ID:
            raise InvalidParamsError("channel.configure requires channelId telegram")
        current = self._config_store.config
        enabled = params.get("enabled", current.enabled)
        if not isinstance(enabled, bool):
            raise InvalidParamsError("enabled must be a boolean")
        token = _optional_patch(params, "token", current.token)
        proxy = _optional_patch(params, "proxy", current.proxy)
        provider_id = _optional_patch(
            params,
            "defaultProviderId",
            current.default_provider_id,
        )
        model_id = _optional_patch(
            params,
            "defaultModelId",
            current.default_model_id,
        )
        if provider_id is not None and model_id is not None:
            try:
                self._provider_store.resolve_model(provider_id, model_id)
            except (ConfigError, ValueError) as error:
                raise InvalidParamsError(str(error)) from None
        try:
            self._config_store.save(
                TelegramConfig(
                    enabled=enabled,
                    token=token,
                    proxy=proxy,
                    default_provider_id=provider_id,
                    default_model_id=model_id,
                )
            )
        except ConfigError as error:
            raise InvalidParamsError(str(error)) from None
        await self._channel.reload()
        return {"channel": self._channel.public_wire()}

    async def test(self, params: dict[str, Any]) -> dict[str, object]:
        allowed = {"channelId", "token", "proxy"}
        if set(params) - allowed or params.get("channelId") != CHANNEL_ID:
            raise InvalidParamsError("channel.test requires channelId telegram")
        current = self._config_store.config
        token = _optional_patch(params, "token", current.token)
        proxy = _optional_patch(params, "proxy", current.proxy)
        if token is None:
            return {"ok": False, "reasonCode": "telegram_token_missing", "bot": None}
        api: TelegramBotApi | None = None
        identity: TelegramBotIdentity | None = None
        try:
            api = self._api_factory(
                TelegramConfig(
                    enabled=False,
                    token=token,
                    proxy=proxy,
                    default_provider_id=current.default_provider_id,
                    default_model_id=current.default_model_id,
                )
            )
            identity = await api.get_me()
        except TelegramApiError as error:
            return {"ok": False, "reasonCode": error.code, "bot": None}
        except Exception:
            return {"ok": False, "reasonCode": "telegram_unexpected", "bot": None}
        finally:
            if api is not None:
                await api.close()
        if identity is None:
            return {"ok": False, "reasonCode": "telegram_unexpected", "bot": None}
        return {
            "ok": True,
            "reasonCode": None,
            "bot": {
                "id": identity.id,
                "username": identity.username,
                "name": identity.first_name,
            },
        }

    async def reset_binding(self, params: dict[str, Any]) -> dict[str, object]:
        if set(params) - {"channelId"} or params.get("channelId") != CHANNEL_ID:
            raise InvalidParamsError("channel.resetBinding requires channelId telegram")
        self._channel_store.reset_channel(CHANNEL_ID)
        await self._channel.reload()
        return {"channel": self._channel.public_wire()}


def _http_api_factory(config: TelegramConfig) -> TelegramBotApi:
    if config.token is None:
        raise ValueError("telegram bot token is missing")
    return HttpTelegramBotApi(token=config.token, proxy=config.proxy)


def _optional_patch(
    params: dict[str, Any],
    key: str,
    current: str | None,
) -> str | None:
    if key not in params:
        return current
    value = params[key]
    if value is None:
        return None
    if not isinstance(value, str):
        raise InvalidParamsError(f"{key} must be a string or null")
    normalized = value.strip()
    return normalized or None


__all__ = ["ChannelService"]
