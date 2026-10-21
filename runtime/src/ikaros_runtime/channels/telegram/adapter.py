"""Telegram private-chat adapter for the Ikaros Runtime."""

from __future__ import annotations

import asyncio
import logging
from collections.abc import Awaitable, Callable
from contextlib import suppress
from dataclasses import replace
from uuid import uuid4

from ...domain import CommandOutcome, JournalEvent
from ...errors import ConfigError, InvalidParamsError
from ...providers.registry import ConfigStore
from ...services.threads import ThreadService
from ...services.turns import TurnService
from ...storage.store import SqliteRuntimeStore
from ..config import TelegramConfig, TelegramConfigStore
from ..store import ChannelBinding, ChannelOutboxItem, ChannelStore
from .client import (
    HttpTelegramBotApi,
    TelegramApiError,
    TelegramBotApi,
    TelegramBotIdentity,
    TelegramMessage,
    TelegramUpdate,
)
from .format import format_failure_reason, split_message

CHANNEL_ID = "telegram"
_LOGGER = logging.getLogger("ikaros_runtime.channels.telegram")
_FINISH_COMMAND = Callable[[CommandOutcome], Awaitable[None]]
_API_FACTORY = Callable[[TelegramConfig], TelegramBotApi]
_HELP_TEXT = """Ikaros Telegram

直接发送文本即可开始对话。

命令：
/start  显示帮助
/new    归档当前会话并新建
/status 查看当前任务
/cancel 取消当前任务
/model [provider/model] 查看或切换默认模型
/help   显示帮助"""
_MAX_CATCH_UP = 100
_TYPING_INTERVAL_SECONDS = 4.0


class TelegramChannel:
    """Own the Telegram polling, inbound routing and outbound delivery loops."""

    def __init__(
        self,
        *,
        config_store: TelegramConfigStore,
        channel_store: ChannelStore,
        runtime_store: SqliteRuntimeStore,
        provider_store: ConfigStore,
        threads: ThreadService,
        turns: TurnService,
        finish_command: _FINISH_COMMAND,
        api_factory: _API_FACTORY | None = None,
        poll_timeout_seconds: int = 50,
    ) -> None:
        self._config_store = config_store
        self._channel_store = channel_store
        self._runtime_store = runtime_store
        self._provider_store = provider_store
        self._threads = threads
        self._turns = turns
        self._finish_command = finish_command
        self._api_factory = api_factory or _default_api_factory
        self._poll_timeout_seconds = poll_timeout_seconds
        self._status = "disabled"
        self._identity: TelegramBotIdentity | None = None
        self._last_error_code: str | None = None
        self._api: TelegramBotApi | None = None
        self._stop_event = asyncio.Event()
        self._outbox_wakeup = asyncio.Event()
        self._poll_task: asyncio.Task[None] | None = None
        self._outbox_task: asyncio.Task[None] | None = None
        self._typing_task: asyncio.Task[None] | None = None
        self._telegram_runs: dict[str, str] = {}
        self._active_runs: dict[str, str] = {}
        self._pending_final: dict[str, str] = {}
        self._typing_chats: set[str] = set()

    @property
    def status(self) -> str:
        return self._status

    @property
    def identity(self) -> TelegramBotIdentity | None:
        return self._identity

    def public_wire(self) -> dict[str, object]:
        config = self._config_store.config
        bindings = self._channel_store.list_bindings(CHANNEL_ID)
        owner = next(
            (binding for binding in bindings if binding.owner_user_id is not None),
            None,
        )
        bot = None
        if self._identity is not None:
            bot = {
                "id": self._identity.id,
                "username": self._identity.username,
                "name": self._identity.first_name,
            }
        return {
            "id": CHANNEL_ID,
            "status": self._status,
            "enabled": config.enabled,
            "configured": config.token is not None,
            "proxyConfigured": config.proxy is not None,
            "defaultProviderId": config.default_provider_id,
            "defaultModelId": config.default_model_id,
            "bot": bot,
            "owner": (
                {
                    "userId": owner.owner_user_id,
                    "username": owner.owner_username,
                }
                if owner is not None
                else None
            ),
            "lastErrorCode": self._last_error_code,
        }

    async def start(self) -> None:
        await self.close()
        self._stop_event = asyncio.Event()
        config = self._config_store.config
        if not config.enabled or config.token is None:
            self._status = "disabled"
            self._last_error_code = None
            return

        self._status = "connecting"
        self._last_error_code = None
        try:
            api = self._api_factory(config)
        except Exception:
            _LOGGER.exception("telegram API initialization failed")
            self._status = "error"
            self._last_error_code = "telegram_unexpected"
            return
        try:
            self._identity = await api.get_me()
        except TelegramApiError as error:
            self._status = "error"
            self._last_error_code = error.code
            await api.close()
            return
        except Exception:
            _LOGGER.exception("telegram identity check failed")
            self._status = "error"
            self._last_error_code = "telegram_unexpected"
            await api.close()
            return

        self._api = api
        self._channel_store.recover_sending()
        self._status = "connected"
        self._poll_task = asyncio.create_task(
            self._poll_loop(),
            name="ikaros-telegram-poll",
        )
        self._outbox_task = asyncio.create_task(
            self._outbox_loop(),
            name="ikaros-telegram-outbox",
        )
        self._typing_task = asyncio.create_task(
            self._typing_loop(),
            name="ikaros-telegram-typing",
        )

    async def reload(self) -> None:
        await self.close()
        await self.start()

    async def close(self) -> None:
        self._stop_event.set()
        self._outbox_wakeup.set()
        tasks = tuple(
            task
            for task in (self._poll_task, self._outbox_task, self._typing_task)
            if task is not None
        )
        for task in tasks:
            task.cancel()
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)
        self._poll_task = None
        self._outbox_task = None
        self._typing_task = None
        api = self._api
        self._api = None
        if api is not None:
            await api.close()
        self._identity = None
        self._telegram_runs.clear()
        self._active_runs.clear()
        self._pending_final.clear()
        self._typing_chats.clear()
        if self._status != "error":
            self._status = "disabled"

    def handle_event(self, event: JournalEvent) -> None:
        """Synchronous EventHub sink; never let delivery failures affect a Run."""

        try:
            if (
                event.type == "item.completed"
                and event.run_id is not None
                and event.run_id in self._telegram_runs
            ):
                payload = event.payload.get("item")
                if (
                    isinstance(payload, dict)
                    and payload.get("kind") == "message"
                    and payload.get("role") == "assistant"
                    and payload.get("status") == "completed"
                    and isinstance(payload.get("content"), str)
                ):
                    self._pending_final[event.run_id] = str(payload["content"])
            elif event.type == "run.settled" and event.run_id is not None:
                self._settle_run(event)
        except Exception:
            _LOGGER.exception("telegram event handling failed")

    def _settle_run(self, event: JournalEvent) -> None:
        run_id = event.run_id
        if run_id is None:
            return
        chat_id = self._telegram_runs.pop(run_id, None)
        if chat_id is None:
            return
        binding = (
            self._channel_store.get_binding_by_thread(CHANNEL_ID, event.thread_id)
            if event.thread_id is not None
            else None
        )
        identity = self._identity
        account_id = binding.account_id if binding is not None else (
            identity.id if identity is not None else None
        )
        if account_id is None:
            return
        self._active_runs.pop(chat_id, None)
        self._typing_chats.discard(chat_id)
        status = event.payload.get("status")
        reason_code = event.payload.get("reasonCode")
        if status == "completed":
            text = self._pending_final.pop(run_id, None)
            if text is None:
                text = self._runtime_store.final_assistant_message_for_run(run_id)
            if text is None:
                text = "运行已完成。"
        elif status == "cancelled":
            self._pending_final.pop(run_id, None)
            text = "已取消。"
        else:
            self._pending_final.pop(run_id, None)
            text = format_failure_reason(
                reason_code if isinstance(reason_code, str) else None
            )
        self._channel_store.enqueue_outbox(
            channel_id=CHANNEL_ID,
            account_id=account_id,
            chat_id=chat_id,
            run_id=run_id,
            text=text,
        )
        self._outbox_wakeup.set()

    async def _poll_loop(self) -> None:
        backoff = 2.0
        while not self._stop_event.is_set():
            api = self._api
            identity = self._identity
            if api is None or identity is None:
                return
            try:
                offset = self._channel_store.get_cursor(CHANNEL_ID, identity.id)
                updates = await api.get_updates(
                    offset=offset,
                    poll_timeout=self._poll_timeout_seconds,
                )
                backoff = 2.0
                self._status = "connected"
                for update in updates[:_MAX_CATCH_UP]:
                    if self._stop_event.is_set():
                        return
                    try:
                        await self._handle_update(update)
                    except Exception:
                        _LOGGER.exception("telegram update handling failed")
                    finally:
                        self._channel_store.set_cursor(
                            CHANNEL_ID,
                            identity.id,
                            update.update_id + 1,
                        )
            except asyncio.CancelledError:
                raise
            except TelegramApiError as error:
                if error.code == "telegram_polling_conflict":
                    self._status = "error"
                    self._last_error_code = error.code
                else:
                    self._status = "connecting"
                    self._last_error_code = error.code
                await _sleep_unless_stopped(self._stop_event, backoff)
                backoff = min(30.0, backoff * 2.0)
            except Exception:
                _LOGGER.exception("telegram polling failed")
                self._status = "connecting"
                self._last_error_code = "telegram_unexpected"
                await _sleep_unless_stopped(self._stop_event, backoff)
                backoff = min(30.0, backoff * 2.0)

    async def _handle_update(self, update: TelegramUpdate) -> None:
        message = update.message
        if message is None or message.sender is None or message.sender.is_bot:
            return
        if message.chat.type != "private":
            return
        identity = self._identity
        if identity is None:
            return
        if message.sender.id == identity.id:
            return

        binding = self._channel_store.get_binding(
            CHANNEL_ID,
            identity.id,
            message.chat.id,
        )
        if binding is not None and binding.owner_user_id not in {
            None,
            message.sender.id,
        }:
            return

        if binding is None or self._thread_is_archived(binding.thread_id):
            binding = await self._create_binding(message)

        text = message.text.strip()
        if not text:
            await self._reply(message.chat.id, "目前只支持文本消息。")
            return
        if text.startswith("/"):
            await self._handle_command(message, binding, text)
            return
        await self._start_turn(message, binding)

    def _thread_is_archived(self, thread_id: str) -> bool:
        try:
            return self._runtime_store.get_thread(thread_id).thread.archived_at is not None
        except LookupError:
            return True

    async def _create_binding(self, message: TelegramMessage) -> ChannelBinding:
        identity = self._identity
        if identity is None:
            raise RuntimeError("telegram identity is unavailable")
        title = _thread_title()
        outcome = self._threads.create(
            {
                "title": title,
                "clientRequestId": (
                    f"tg:thread:{identity.id}:{message.chat.id}:{uuid4().hex}"
                ),
            }
        )
        await self._finish_command(outcome)
        thread = outcome.result.get("thread")
        if not isinstance(thread, dict):
            raise RuntimeError("thread.create returned no Thread")
        thread_id = thread.get("id")
        branch_id = thread.get("defaultBranchId")
        if not isinstance(thread_id, str) or not isinstance(branch_id, str):
            raise RuntimeError("thread.create returned an invalid Thread")
        return self._channel_store.upsert_binding(
            channel_id=CHANNEL_ID,
            account_id=identity.id,
            chat_id=message.chat.id,
            thread_id=thread_id,
            branch_id=branch_id,
            owner_user_id=message.sender.id if message.sender is not None else None,
            owner_username=message.sender.username if message.sender is not None else None,
        )

    async def _handle_command(
        self,
        message: TelegramMessage,
        binding: ChannelBinding,
        text: str,
    ) -> None:
        command, arguments = _parse_command(text)
        identity = self._identity
        if identity is None:
            return
        if command in {"start", "help"}:
            await self._reply(message.chat.id, _HELP_TEXT)
            return
        if command == "new":
            await self._new_thread(message, binding)
            return
        if command == "status":
            active = self._active_runs.get(message.chat.id)
            if active is None:
                await self._reply(message.chat.id, "当前没有运行中的任务。")
            else:
                try:
                    status = self._runtime_store.run_status(active)
                except LookupError:
                    status = "unknown"
                config = self._config_store.config
                model = f"{config.default_provider_id}/{config.default_model_id}"
                await self._reply(
                    message.chat.id,
                    f"当前任务：{status}\n模型：{model}",
                )
            return
        if command == "cancel":
            active = self._active_runs.get(message.chat.id)
            if active is None:
                await self._reply(message.chat.id, "当前没有运行中的任务。")
                return
            outcome = self._turns.cancel_run({"runId": active})
            await self._finish_command(outcome)
            if outcome.result.get("accepted") is True:
                await self._reply(message.chat.id, "已取消。")
            else:
                await self._reply(message.chat.id, "任务已经结束。")
            return
        if command == "model":
            await self._handle_model_command(message.chat.id, arguments)
            return
        await self._reply(message.chat.id, "未知命令。\n\n" + _HELP_TEXT)

    async def _new_thread(
        self,
        message: TelegramMessage,
        binding: ChannelBinding,
    ) -> None:
        active = self._active_runs.get(message.chat.id)
        if active is not None:
            outcome = self._turns.cancel_run({"runId": active})
            await self._finish_command(outcome)
        identity = self._identity
        if identity is None:
            return
        try:
            archive_outcome = self._threads.archive({"threadId": binding.thread_id})
        except InvalidParamsError:
            archive_outcome = None
        if archive_outcome is not None:
            await self._finish_command(archive_outcome)
        title = _thread_title()
        outcome = self._threads.create(
            {
                "title": title,
                "clientRequestId": (
                    f"tg:thread:{identity.id}:{message.chat.id}:{uuid4().hex}"
                ),
            }
        )
        await self._finish_command(outcome)
        thread = outcome.result.get("thread")
        if not isinstance(thread, dict):
            await self._reply(message.chat.id, "新建会话失败。")
            return
        thread_id = thread.get("id")
        branch_id = thread.get("defaultBranchId")
        if not isinstance(thread_id, str) or not isinstance(branch_id, str):
            await self._reply(message.chat.id, "新建会话失败。")
            return
        self._channel_store.upsert_binding(
            channel_id=CHANNEL_ID,
            account_id=identity.id,
            chat_id=message.chat.id,
            thread_id=thread_id,
            branch_id=branch_id,
            owner_user_id=binding.owner_user_id,
            owner_username=binding.owner_username,
        )
        await self._reply(message.chat.id, "已创建新会话，后续消息会写入新 Thread。")

    async def _handle_model_command(self, chat_id: str, arguments: str) -> None:
        config = self._config_store.config
        if not arguments:
            lines = [
                f"{provider_id}/{model_id}"
                for provider_id, model_id in _enabled_models(self._provider_store)
            ]
            current = (
                f"{config.default_provider_id}/{config.default_model_id}"
                if config.default_provider_id and config.default_model_id
                else "未设置"
            )
            body = "可用模型：\n" + "\n".join(f"- {line}" for line in lines)
            await self._reply(chat_id, f"当前模型：{current}\n{body}")
            return
        if "/" in arguments:
            provider_id, model_id = arguments.split("/", 1)
        else:
            parts = arguments.split()
            if len(parts) != 2:
                await self._reply(chat_id, "用法：/model <providerId>/<modelId>")
                return
            provider_id, model_id = parts
        try:
            self._provider_store.resolve_model(provider_id, model_id)
        except (ConfigError, ValueError) as error:
            await self._reply(chat_id, f"模型不可用：{error}")
            return
        self._config_store.save(
            replace(
                config,
                default_provider_id=provider_id,
                default_model_id=model_id,
            )
        )
        await self._reply(chat_id, f"已切换模型：{provider_id}/{model_id}")

    async def _start_turn(
        self,
        message: TelegramMessage,
        binding: ChannelBinding,
    ) -> None:
        identity = self._identity
        if identity is None:
            return
        config = self._config_store.config
        if config.default_provider_id is None or config.default_model_id is None:
            await self._reply(
                message.chat.id,
                "还没有配置默认模型。请在 Ikaros Settings → Telegram 中选择模型。",
            )
            return
        queued_behind = message.chat.id in self._active_runs
        try:
            outcome = self._turns.start_turn(
                {
                    "threadId": binding.thread_id,
                    "branchId": binding.branch_id,
                    "content": message.text.strip(),
                    "providerId": config.default_provider_id,
                    "modelId": config.default_model_id,
                    "clientRequestId": (
                        f"tg:{identity.id}:{message.chat.id}:{message.message_id}"
                    ),
                }
            )
        except (InvalidParamsError, ConfigError) as error:
            _LOGGER.info("telegram turn rejected: %s", type(error).__name__)
            await self._reply(
                message.chat.id,
                "启动任务失败，请检查 Telegram 默认模型和 Provider 配置。",
            )
            return
        await self._finish_command(outcome)
        run_id = outcome.result.get("runId")
        if isinstance(run_id, str):
            if run_id in self._telegram_runs:
                return
            self._telegram_runs[run_id] = message.chat.id
            self._active_runs[message.chat.id] = run_id
            self._typing_chats.add(message.chat.id)
        if queued_behind:
            await self._reply(message.chat.id, "上一条仍在处理中，这条已排队。")

    async def _reply(self, chat_id: str, text: str) -> None:
        identity = self._identity
        if identity is None:
            return
        self._channel_store.enqueue_outbox(
            channel_id=CHANNEL_ID,
            account_id=identity.id,
            chat_id=chat_id,
            text=text,
        )
        self._outbox_wakeup.set()

    async def _outbox_loop(self) -> None:
        while not self._stop_event.is_set():
            item = self._channel_store.claim_outbox()
            if item is None:
                self._outbox_wakeup.clear()
                with suppress(TimeoutError):
                    await asyncio.wait_for(self._outbox_wakeup.wait(), timeout=1.0)
                continue
            try:
                await self._deliver(item)
            except asyncio.CancelledError:
                raise
            except Exception:
                _LOGGER.exception("telegram outbox item failed unexpectedly")

    async def _deliver(self, item: ChannelOutboxItem) -> None:
        api = self._api
        if api is None:
            return
        chunks = split_message(item.text)
        if not chunks:
            self._channel_store.fail_outbox(item.id, "telegram_empty_message")
            return
        last_message_id: int | None = None
        try:
            for chunk in chunks:
                last_message_id = await api.send_message(
                    chat_id=item.chat_id,
                    text=chunk,
                )
        except asyncio.CancelledError:
            raise
        except TelegramApiError as error:
            if error.code == "telegram_rate_limited":
                self._channel_store.retry_outbox(
                    item.id,
                    delay_seconds=error.retry_after or 1.0,
                    error_code=error.code,
                )
                return
            if error.code in {"telegram_network", "telegram_unexpected"}:
                self._channel_store.retry_outbox(
                    item.id,
                    delay_seconds=min(30.0, 2.0 ** min(item.attempts + 1, 5)),
                    error_code=error.code,
                )
                return
            self._channel_store.fail_outbox(item.id, error.code)
            if error.code in {"telegram_unauthorized", "telegram_polling_conflict"}:
                self._status = "error"
                self._last_error_code = error.code
            return
        except Exception:
            _LOGGER.exception("telegram outbox delivery failed")
            self._channel_store.retry_outbox(
                item.id,
                delay_seconds=min(30.0, 2.0 ** min(item.attempts + 1, 5)),
                error_code="telegram_unexpected",
            )
            return
        if last_message_id is not None:
            self._channel_store.mark_outbox_sent(item.id, last_message_id)

    async def _typing_loop(self) -> None:
        while not self._stop_event.is_set():
            api = self._api
            if api is not None:
                for chat_id in tuple(self._typing_chats):
                    try:
                        await api.send_chat_action(chat_id=chat_id)
                    except asyncio.CancelledError:
                        raise
                    except Exception:
                        pass
            with suppress(TimeoutError):
                await asyncio.wait_for(
                    self._stop_event.wait(),
                    timeout=_TYPING_INTERVAL_SECONDS,
                )


def _default_api_factory(config: TelegramConfig) -> TelegramBotApi:
    if config.token is None:
        raise ValueError("telegram bot token is missing")
    return HttpTelegramBotApi(token=config.token, proxy=config.proxy)


def _thread_title() -> str:
    """Keep one stable Recents entry for the single-user Telegram channel."""

    return "Telegram"


def _parse_command(value: str) -> tuple[str, str]:
    without_slash = value[1:].strip()
    if not without_slash:
        return "", ""
    parts = without_slash.split(maxsplit=1)
    command = parts[0].split("@", 1)[0].lower()
    arguments = parts[1].strip() if len(parts) == 2 else ""
    return command, arguments


def _enabled_models(store: ConfigStore) -> tuple[tuple[str, str], ...]:
    return tuple(
        (model.provider_id, model.id)
        for model in store.model_summaries()
        if model.enabled
    )


async def _sleep_unless_stopped(stop_event: asyncio.Event, seconds: float) -> None:
    with suppress(TimeoutError):
        await asyncio.wait_for(stop_event.wait(), timeout=seconds)


__all__ = ["CHANNEL_ID", "TelegramChannel"]
