"""Minimal Telegram Bot API client built on the Runtime's existing httpx dependency."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Protocol

import httpx

TELEGRAM_API_BASE_URL = "https://api.telegram.org"
DEFAULT_POLL_TIMEOUT_SECONDS = 50
DEFAULT_REQUEST_TIMEOUT = httpx.Timeout(connect=10.0, read=60.0, write=30.0, pool=10.0)


@dataclass(frozen=True, slots=True)
class TelegramBotIdentity:
    id: str
    username: str
    first_name: str


@dataclass(frozen=True, slots=True)
class TelegramChat:
    id: str
    type: str
    title: str | None
    username: str | None
    first_name: str | None


@dataclass(frozen=True, slots=True)
class TelegramUser:
    id: str
    is_bot: bool
    username: str | None
    first_name: str | None


@dataclass(frozen=True, slots=True)
class TelegramMessage:
    message_id: int
    chat: TelegramChat
    sender: TelegramUser | None
    text: str
    date: int


@dataclass(frozen=True, slots=True)
class TelegramUpdate:
    update_id: int
    message: TelegramMessage | None


class TelegramApiError(RuntimeError):
    """A safe, typed Telegram failure that never carries the bot token."""

    def __init__(
        self,
        code: str,
        *,
        status_code: int | None = None,
        description: str | None = None,
        retry_after: float | None = None,
    ) -> None:
        super().__init__(code)
        self.code = code
        self.status_code = status_code
        self.description = description
        self.retry_after = retry_after


class TelegramBotApi(Protocol):
    async def get_me(self) -> TelegramBotIdentity: ...

    async def get_updates(
        self,
        *,
        offset: int,
        poll_timeout: int = DEFAULT_POLL_TIMEOUT_SECONDS,
    ) -> tuple[TelegramUpdate, ...]: ...

    async def send_message(self, *, chat_id: str, text: str) -> int: ...

    async def send_chat_action(self, *, chat_id: str, action: str = "typing") -> None: ...

    async def close(self) -> None: ...


class HttpTelegramBotApi:
    """Direct Bot API transport; no third-party Telegram SDK is required."""

    def __init__(
        self,
        *,
        token: str,
        proxy: str | None = None,
        base_url: str = TELEGRAM_API_BASE_URL,
        transport: httpx.AsyncBaseTransport | None = None,
    ) -> None:
        if not token:
            raise ValueError("telegram bot token must not be empty")
        self._token = token
        self._base_url = base_url.rstrip("/")
        self._client = httpx.AsyncClient(
            base_url=self._base_url,
            proxy=proxy,
            timeout=DEFAULT_REQUEST_TIMEOUT,
            transport=transport,
            limits=httpx.Limits(max_connections=8, max_keepalive_connections=4),
            headers={"User-Agent": "Ikaros-Runtime/0.1"},
        )

    async def close(self) -> None:
        await self._client.aclose()

    async def get_me(self) -> TelegramBotIdentity:
        result = await self._call("getMe")
        if not isinstance(result, dict):
            raise TelegramApiError("telegram_protocol")
        return _identity_from_payload(result)

    async def get_updates(
        self,
        *,
        offset: int,
        poll_timeout: int = DEFAULT_POLL_TIMEOUT_SECONDS,
    ) -> tuple[TelegramUpdate, ...]:
        result = await self._call(
            "getUpdates",
            payload={
                "offset": offset,
                "timeout": poll_timeout,
                "allowed_updates": '["message"]',
            },
            request_timeout=poll_timeout + 15,
        )
        if not isinstance(result, list):
            raise TelegramApiError("telegram_protocol")
        return tuple(
            update
            for raw in result
            if (update := _update_from_payload(raw)) is not None
        )

    async def send_message(self, *, chat_id: str, text: str) -> int:
        result = await self._call(
            "sendMessage",
            payload={
                "chat_id": chat_id,
                "text": text,
                "disable_web_page_preview": True,
            },
        )
        if not isinstance(result, dict) or not isinstance(result.get("message_id"), int):
            raise TelegramApiError("telegram_protocol")
        return int(result["message_id"])

    async def send_chat_action(self, *, chat_id: str, action: str = "typing") -> None:
        await self._call(
            "sendChatAction",
            payload={"chat_id": chat_id, "action": action},
        )

    async def _call(
        self,
        method: str,
        *,
        payload: dict[str, Any] | None = None,
        request_timeout: float | None = None,
    ) -> Any:
        try:
            response = await self._client.post(
                f"/bot{self._token}/{method}",
                json=payload or {},
                timeout=request_timeout,
            )
        except httpx.HTTPError as error:
            raise TelegramApiError(
                "telegram_network",
                description=type(error).__name__,
            ) from None

        try:
            body = response.json()
        except ValueError:
            body = None
        if response.status_code == 401:
            raise TelegramApiError("telegram_unauthorized", status_code=401)
        if response.status_code == 409:
            raise TelegramApiError("telegram_polling_conflict", status_code=409)
        if response.status_code == 429:
            retry_after = _retry_after(body)
            raise TelegramApiError(
                "telegram_rate_limited",
                status_code=429,
                retry_after=retry_after,
            )
        if not response.is_success:
            raise TelegramApiError(
                "telegram_http_error",
                status_code=response.status_code,
                description=_safe_description(body),
            )
        if not isinstance(body, dict) or body.get("ok") is not True:
            raise TelegramApiError(
                "telegram_protocol",
                description=_safe_description(body),
            )
        return body.get("result")


def _identity_from_payload(value: dict[str, Any]) -> TelegramBotIdentity:
    bot_id = value.get("id")
    username = value.get("username")
    first_name = value.get("first_name")
    if not isinstance(bot_id, int) or not isinstance(username, str):
        raise TelegramApiError("telegram_protocol")
    return TelegramBotIdentity(
        id=str(bot_id),
        username=username,
        first_name=first_name if isinstance(first_name, str) else username,
    )


def _update_from_payload(value: object) -> TelegramUpdate | None:
    if not isinstance(value, dict):
        return None
    update_id = value.get("update_id")
    if not isinstance(update_id, int):
        return None
    raw_message = value.get("message")
    if not isinstance(raw_message, dict):
        return TelegramUpdate(update_id=update_id, message=None)
    message = _message_from_payload(raw_message)
    return TelegramUpdate(update_id=update_id, message=message)


def _message_from_payload(value: dict[str, Any]) -> TelegramMessage | None:
    message_id = value.get("message_id")
    chat = value.get("chat")
    if not isinstance(message_id, int) or not isinstance(chat, dict):
        return None
    chat_id = chat.get("id")
    chat_type = chat.get("type")
    if not isinstance(chat_id, int) or not isinstance(chat_type, str):
        return None
    sender_value = value.get("from")
    sender = _user_from_payload(sender_value)
    text = value.get("text")
    date = value.get("date")
    return TelegramMessage(
        message_id=message_id,
        chat=TelegramChat(
            id=str(chat_id),
            type=chat_type,
            title=chat.get("title") if isinstance(chat.get("title"), str) else None,
            username=chat.get("username") if isinstance(chat.get("username"), str) else None,
            first_name=(
                chat.get("first_name") if isinstance(chat.get("first_name"), str) else None
            ),
        ),
        sender=sender,
        text=text if isinstance(text, str) else "",
        date=date if isinstance(date, int) else 0,
    )


def _user_from_payload(value: object) -> TelegramUser | None:
    if not isinstance(value, dict):
        return None
    user_id = value.get("id")
    if not isinstance(user_id, int):
        return None
    return TelegramUser(
        id=str(user_id),
        is_bot=value.get("is_bot") is True,
        username=value.get("username") if isinstance(value.get("username"), str) else None,
        first_name=(
            value.get("first_name") if isinstance(value.get("first_name"), str) else None
        ),
    )


def _retry_after(body: object) -> float:
    if isinstance(body, dict):
        parameters = body.get("parameters")
        if isinstance(parameters, dict):
            value = parameters.get("retry_after")
            if isinstance(value, (int, float)) and not isinstance(value, bool):
                return max(0.0, float(value))
    return 1.0


def _safe_description(body: object) -> str | None:
    if not isinstance(body, dict):
        return None
    description = body.get("description")
    if not isinstance(description, str):
        return None
    return " ".join(description.split())[:200]


__all__ = [
    "DEFAULT_POLL_TIMEOUT_SECONDS",
    "HttpTelegramBotApi",
    "TELEGRAM_API_BASE_URL",
    "TelegramApiError",
    "TelegramBotApi",
    "TelegramBotIdentity",
    "TelegramChat",
    "TelegramMessage",
    "TelegramUpdate",
    "TelegramUser",
]
