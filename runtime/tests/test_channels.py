from __future__ import annotations

from pathlib import Path
from typing import Any, cast

import pytest

from ikaros_runtime.bootstrap import RuntimeApplication
from ikaros_runtime.channels.config import TelegramConfig, TelegramConfigStore
from ikaros_runtime.channels.store import ChannelStore
from ikaros_runtime.channels.telegram.adapter import TelegramChannel
from ikaros_runtime.channels.telegram.client import (
    TelegramBotIdentity,
    TelegramChat,
    TelegramMessage,
    TelegramUpdate,
    TelegramUser,
)
from ikaros_runtime.domain import CommandOutcome, JournalEvent
from ikaros_runtime.memory import SqliteMemoryStore
from ikaros_runtime.storage import SqliteRuntimeStore


class _FakeApi:
    def __init__(self) -> None:
        self.sent: list[tuple[str, str]] = []
        self.actions: list[tuple[str, str]] = []
        self.closed = False

    async def get_me(self) -> TelegramBotIdentity:
        return TelegramBotIdentity(id="bot-1", username="ikaros_bot", first_name="Ikaros")

    async def get_updates(
        self,
        *,
        offset: int,
        poll_timeout: int = 50,
    ) -> tuple[TelegramUpdate, ...]:
        del offset, poll_timeout
        return ()

    async def send_message(self, *, chat_id: str, text: str) -> int:
        self.sent.append((chat_id, text))
        return len(self.sent)

    async def send_chat_action(self, *, chat_id: str, action: str = "typing") -> None:
        self.actions.append((chat_id, action))

    async def close(self) -> None:
        self.closed = True


class _FakeThreads:
    def __init__(self) -> None:
        self.created: list[dict[str, Any]] = []
        self.archived: list[str] = []

    def create(self, params: dict[str, Any]) -> CommandOutcome:
        self.created.append(dict(params))
        index = len(self.created)
        return CommandOutcome(
            result={
                "thread": {
                    "id": f"thread_{index}",
                    "defaultBranchId": f"branch_{index}",
                },
                "event": {},
            }
        )

    def archive(self, params: dict[str, Any]) -> CommandOutcome:
        self.archived.append(str(params["threadId"]))
        return CommandOutcome(result={"thread": {}, "changed": True})


class _FakeTurns:
    def __init__(self) -> None:
        self.started: list[dict[str, Any]] = []
        self.cancelled: list[str] = []

    def start_turn(self, params: dict[str, Any]) -> CommandOutcome:
        self.started.append(dict(params))
        index = len(self.started)
        return CommandOutcome(
            result={
                "turnId": f"turn_{index}",
                "runId": f"run_{index}",
                "threadId": params["threadId"],
                "branchId": params["branchId"],
            }
        )

    def cancel_run(self, params: dict[str, Any]) -> CommandOutcome:
        self.cancelled.append(str(params["runId"]))
        return CommandOutcome(result={"accepted": True, "runId": params["runId"]})


class _FakeProviders:
    def resolve_model(self, provider_id: str, model_id: str) -> object:
        if (provider_id, model_id) != ("provider", "model"):
            raise ValueError("the selected model does not exist")
        return object()

    def model_summaries(self) -> tuple[object, ...]:
        return (_ModelSummary(),)


class _ModelSummary:
    provider_id = "provider"
    id = "model"
    enabled = True


class _FakeRuntimeStore:
    archived = False

    def get_thread(self, thread_id: str) -> object:
        del thread_id
        return type(
            "Metadata",
            (),
            {"thread": type("Thread", (), {"archived_at": None if not self.archived else "now"})()},
        )()

    def run_status(self, run_id: str) -> str:
        del run_id
        return "running"

    def final_assistant_message_for_run(self, run_id: str) -> str | None:
        del run_id
        return "final answer"


class _Finish:
    def __init__(self) -> None:
        self.outcomes: list[CommandOutcome] = []

    async def __call__(self, outcome: CommandOutcome) -> None:
        self.outcomes.append(outcome)


def _message(
    *,
    message_id: int = 1,
    text: str = "hello",
    sender_id: str = "user-1",
    username: str | None = "alice",
) -> TelegramMessage:
    return TelegramMessage(
        message_id=message_id,
        chat=TelegramChat(
            id="chat-1",
            type="private",
            title=None,
            username=username,
            first_name="Alice",
        ),
        sender=TelegramUser(
            id=sender_id,
            is_bot=False,
            username=username,
            first_name="Alice",
        ),
        text=text,
        date=0,
    )


def _channel(
    tmp_path: Path,
) -> tuple[
    TelegramChannel,
    _FakeApi,
    _FakeThreads,
    _FakeTurns,
    ChannelStore,
    TelegramConfigStore,
]:
    config_store = TelegramConfigStore(tmp_path)
    config_store.save(
        TelegramConfig(
            enabled=True,
            token="123456:test-token",
            default_provider_id="provider",
            default_model_id="model",
        )
    )
    channel_store = ChannelStore(tmp_path / "channels.db")
    api = _FakeApi()
    threads = _FakeThreads()
    turns = _FakeTurns()
    finish = _Finish()
    channel = TelegramChannel(
        config_store=config_store,
        channel_store=channel_store,
        runtime_store=cast(Any, _FakeRuntimeStore()),
        provider_store=cast(Any, _FakeProviders()),
        threads=cast(Any, threads),
        turns=cast(Any, turns),
        finish_command=finish,
        api_factory=lambda _config: cast(Any, api),
    )
    channel._identity = TelegramBotIdentity(  # noqa: SLF001 - deterministic adapter setup
        id="bot-1",
        username="ikaros_bot",
        first_name="Ikaros",
    )
    channel._api = api  # noqa: SLF001 - delivery is exercised without polling
    return channel, api, threads, turns, channel_store, config_store


@pytest.mark.asyncio
async def test_first_private_message_creates_thread_and_starts_turn(tmp_path: Path) -> None:
    channel, _api, threads, turns, channel_store, _config = _channel(tmp_path)

    await channel._handle_update(TelegramUpdate(update_id=1, message=_message()))  # noqa: SLF001

    assert threads.created[0]["title"] == "Telegram"
    assert turns.started[0]["content"] == "hello"
    binding = channel_store.get_binding("telegram", "bot-1", "chat-1")
    assert binding is not None
    assert binding.thread_id == "thread_1"
    assert binding.owner_user_id == "user-1"
    assert channel._telegram_runs == {"run_1": "chat-1"}  # noqa: SLF001


@pytest.mark.asyncio
async def test_non_owner_private_message_is_ignored(tmp_path: Path) -> None:
    channel, _api, threads, turns, channel_store, _config = _channel(tmp_path)
    channel_store.upsert_binding(
        channel_id="telegram",
        account_id="bot-1",
        chat_id="chat-1",
        thread_id="thread-existing",
        branch_id="branch-existing",
        owner_user_id="user-1",
        owner_username="alice",
    )

    await channel._handle_update(  # noqa: SLF001
        TelegramUpdate(update_id=2, message=_message(sender_id="user-2", username="bob"))
    )

    assert threads.created == []
    assert turns.started == []


@pytest.mark.asyncio
async def test_new_command_archives_and_rebinds_fixed_title(tmp_path: Path) -> None:
    channel, _api, threads, _turns, channel_store, _config = _channel(tmp_path)
    await channel._handle_update(TelegramUpdate(update_id=1, message=_message()))  # noqa: SLF001
    await channel._handle_update(  # noqa: SLF001
        TelegramUpdate(update_id=2, message=_message(message_id=2, text="/new"))
    )

    assert threads.archived == ["thread_1"]
    assert [item["title"] for item in threads.created] == ["Telegram", "Telegram"]
    binding = channel_store.get_binding("telegram", "bot-1", "chat-1")
    assert binding is not None
    assert binding.thread_id == "thread_2"


@pytest.mark.asyncio
async def test_completed_run_enqueues_final_answer(tmp_path: Path) -> None:
    channel, api, _threads, _turns, channel_store, _config = _channel(tmp_path)
    await channel._handle_update(TelegramUpdate(update_id=1, message=_message()))  # noqa: SLF001

    channel.handle_event(
        JournalEvent(
            seq=1,
            schema_version=10,
            type="item.completed",
            thread_id="thread_1",
            branch_id="branch_1",
            turn_id="turn_1",
            run_id="run_1",
            item_id="item-1",
            timestamp="2026-01-01T00:00:00.000Z",
            payload={
                "item": {
                    "kind": "message",
                    "role": "assistant",
                    "status": "completed",
                    "content": "final answer",
                }
            },
        )
    )
    channel.handle_event(
        JournalEvent(
            seq=2,
            schema_version=10,
            type="run.settled",
            thread_id="thread_1",
            branch_id="branch_1",
            turn_id="turn_1",
            run_id="run_1",
            item_id=None,
            timestamp="2026-01-01T00:00:00.000Z",
            payload={"status": "completed", "settledAt": "2026-01-01T00:00:00.000Z"},
        )
    )
    item = channel_store.claim_outbox()
    assert item is not None
    await channel._deliver(item)  # noqa: SLF001
    assert api.sent == [("chat-1", "final answer")]


@pytest.mark.asyncio
async def test_start_command_binds_owner_and_replies(tmp_path: Path) -> None:
    channel, api, threads, turns, channel_store, _config = _channel(tmp_path)

    await channel._handle_update(  # noqa: SLF001
        TelegramUpdate(update_id=3, message=_message(text="/start"))
    )

    assert threads.created
    assert turns.started == []
    assert channel_store.get_binding("telegram", "bot-1", "chat-1") is not None
    item = channel_store.claim_outbox()
    assert item is not None
    await channel._deliver(item)  # noqa: SLF001
    assert api.sent and "Ikaros Telegram" in api.sent[0][1]


def test_channel_config_and_store_are_isolated(tmp_path: Path) -> None:
    config_store = TelegramConfigStore(tmp_path)
    assert config_store.config.enabled is False
    saved = config_store.save(
        TelegramConfig(
            enabled=True,
            token="123456:secret-token",
            proxy="http://127.0.0.1:7890",
            default_provider_id="provider",
            default_model_id="model",
        )
    )
    assert saved.enabled is True
    assert config_store.protected_values() == (
        "123456:secret-token",
        "http://127.0.0.1:7890",
    )
    assert "secret-token" not in repr(config_store.config)
    assert (tmp_path / "channels.yaml").exists()
    assert not (tmp_path / "state.db").exists()

    channel_store = ChannelStore(tmp_path / "channels.db")
    channel_store.set_cursor("telegram", "bot-1", 7)
    assert channel_store.get_cursor("telegram", "bot-1") == 7
    first = channel_store.enqueue_outbox(
        channel_id="telegram",
        account_id="bot-1",
        chat_id="chat-1",
        run_id="run-1",
        text="hello",
    )
    second = channel_store.enqueue_outbox(
        channel_id="telegram",
        account_id="bot-1",
        chat_id="chat-1",
        run_id="run-1",
        text="hello again",
    )
    assert first == second
    item = channel_store.claim_outbox()
    assert item is not None
    channel_store.mark_outbox_sent(item.id, 42)
    assert channel_store.pending_outbox_count() == 0
    channel_store.close()


@pytest.mark.asyncio
async def test_runtime_application_exposes_channel_list_rpc(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    channel_store = ChannelStore(tmp_path / "channels.db")
    config_store = TelegramConfigStore(tmp_path)

    async def discard_event(event: JournalEvent) -> None:
        del event

    kernel = RuntimeApplication(
        store,
        discard_event,
        memory_store=memory_store,
        channel_config_store=config_store,
        channel_store=channel_store,
    )
    try:
        routed = await kernel.router.dispatch(1, "channel.list", {})
        assert routed.response["result"] == {
            "channels": [
                {
                    "id": "telegram",
                    "status": "disabled",
                    "enabled": False,
                    "configured": False,
                    "proxyConfigured": False,
                    "defaultProviderId": None,
                    "defaultModelId": None,
                    "bot": None,
                    "owner": None,
                    "lastErrorCode": None,
                }
            ]
        }
    finally:
        await kernel.close()
        channel_store.close()
