from __future__ import annotations

import httpx
import pytest

from ikaros_runtime.channels.telegram.client import HttpTelegramBotApi, TelegramApiError


@pytest.mark.asyncio
async def test_http_telegram_client_parses_bot_updates_and_sends_messages() -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        if request.url.path.endswith("/getMe"):
            return httpx.Response(
                200,
                json={
                    "ok": True,
                    "result": {"id": 7, "username": "ikaros_bot", "first_name": "Ikaros"},
                },
            )
        if request.url.path.endswith("/getUpdates"):
            return httpx.Response(
                200,
                json={
                    "ok": True,
                    "result": [
                        {
                            "update_id": 5,
                            "message": {
                                "message_id": 9,
                                "chat": {"id": 42, "type": "private", "first_name": "Alice"},
                                "from": {
                                    "id": 42,
                                    "is_bot": False,
                                    "username": "alice",
                                    "first_name": "Alice",
                                },
                                "text": "hello",
                                "date": 1,
                            },
                        }
                    ],
                },
            )
        if request.url.path.endswith("/sendMessage"):
            return httpx.Response(200, json={"ok": True, "result": {"message_id": 11}})
        if request.url.path.endswith("/sendChatAction"):
            return httpx.Response(200, json={"ok": True, "result": True})
        return httpx.Response(404, json={"ok": False, "description": "not found"})

    api = HttpTelegramBotApi(
        token="123456:test-token",
        transport=httpx.MockTransport(handler),
    )
    try:
        identity = await api.get_me()
        updates = await api.get_updates(offset=0, poll_timeout=1)
        message_id = await api.send_message(chat_id="42", text="hello")
        await api.send_chat_action(chat_id="42")
    finally:
        await api.close()

    assert identity.id == "7"
    assert identity.username == "ikaros_bot"
    assert len(updates) == 1
    assert updates[0].message is not None
    assert updates[0].message.text == "hello"
    assert updates[0].message.sender is not None
    assert updates[0].message.sender.username == "alice"
    assert message_id == 11


@pytest.mark.asyncio
async def test_http_telegram_client_errors_do_not_expose_the_token() -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        del request
        return httpx.Response(
            429,
            json={
                "ok": False,
                "description": "Too Many Requests",
                "parameters": {"retry_after": 3},
            },
        )

    api = HttpTelegramBotApi(
        token="123456:secret-token",
        transport=httpx.MockTransport(handler),
    )
    try:
        with pytest.raises(TelegramApiError) as captured:
            await api.get_me()
    finally:
        await api.close()

    assert captured.value.code == "telegram_rate_limited"
    assert captured.value.retry_after == 3.0
    assert "secret-token" not in str(captured.value)
