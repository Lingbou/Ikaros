"""Background Thread titles: trigger, provenance, isolation, and lifecycle."""

from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator, Awaitable, Callable, Sequence
from pathlib import Path

import pytest

from ikaros_runtime.agent.loop import AgentLoop
from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.domain import JournalEvent
from ikaros_runtime.errors import ProviderFailure
from ikaros_runtime.providers.base import (
    ProviderAdapter,
    ProviderEvent,
    ProviderRequest,
    ResponseCompleted,
    TextDelta,
)
from ikaros_runtime.run_input import ProviderExecutionSnapshot
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.thread_titles import (
    THREAD_TITLE_MAX_CHARACTERS,
    THREAD_TITLE_MAX_OUTPUT_TOKENS,
    ThreadTitleService,
)

from .helpers import prepare_turn

PublishEvent = Callable[[JournalEvent], Awaitable[None]]

_PROVIDER_ID = "scripted"
_MODEL_ID = "scripted-v1"
_TITLE_PROMPT_MARKER = "You name conversations."


def _snapshot(provider_id: str, model_id: str) -> ProviderExecutionSnapshot:
    return ProviderExecutionSnapshot(
        provider_id=provider_id,
        origin="test",
        base_url=None,
        model_id=model_id,
        supports_tools=True,
    )


def _is_title_request(request: ProviderRequest) -> bool:
    return any(
        message.role == "system" and _TITLE_PROMPT_MARKER in message.content
        for message in request.messages
    )


class _TitleProvider:
    """Answer an Agent Run, then reply to the background title call."""

    def __init__(self, title: str | Exception) -> None:
        self.title = title
        self.run_requests: list[ProviderRequest] = []
        self.title_requests: list[ProviderRequest] = []

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        cancellation.raise_if_cancelled()
        if _is_title_request(request):
            self.title_requests.append(request)
            if isinstance(self.title, Exception):
                raise self.title
            yield TextDelta(self.title)
            yield ResponseCompleted()
            return
        self.run_requests.append(request)
        yield TextDelta("Done.")
        yield ResponseCompleted()


class _BlockingTitleProvider:
    """Answer Runs immediately and hold the title call until released."""

    def __init__(self, title: str) -> None:
        self.title = title
        self.started = asyncio.Event()
        self.release = asyncio.Event()
        self.title_requests: list[ProviderRequest] = []

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        cancellation.raise_if_cancelled()
        if not _is_title_request(request):
            yield TextDelta("Done.")
            yield ResponseCompleted()
            return
        self.title_requests.append(request)
        self.started.set()
        await self.release.wait()
        yield TextDelta(self.title)
        yield ResponseCompleted()


async def _noop_publish(_event: JournalEvent) -> None:
    return None


def _publisher(
    published: list[JournalEvent],
) -> PublishEvent:
    async def publish(event: JournalEvent) -> None:
        published.append(event)

    return publish


def _service(
    *,
    store: SqliteRuntimeStore,
    provider: ProviderAdapter,
    protected_values: Sequence[str] = (),
    enabled: bool = True,
    publish: PublishEvent = _noop_publish,
) -> ThreadTitleService:
    return ThreadTitleService(
        runtime_store=store,
        resolve_provider=lambda provider_id: (
            provider if provider_id == _PROVIDER_ID else None
        ),
        provider_snapshot=_snapshot,
        protected_values=lambda: tuple(protected_values),
        publish=publish,
        enabled=enabled,
    )


def _thread_with_first_turn(
    store: SqliteRuntimeStore,
    *,
    content: str,
    title: str | None = "Placeholder title",
) -> tuple[str, str]:
    thread, _event = store.create_thread(title)
    prepared = prepare_turn(
        store,
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content=content,
        provider_id=_PROVIDER_ID,
        model_id=_MODEL_ID,
    )
    return thread.id, prepared.run_id


async def _drain(service: ThreadTitleService) -> None:
    tasks = tuple(service._tasks.values())
    if tasks:
        await asyncio.gather(*tasks)


def _loop(
    store: SqliteRuntimeStore,
    provider: ProviderAdapter,
    service: ThreadTitleService,
    publish: PublishEvent = _noop_publish,
) -> AgentLoop:
    return AgentLoop(
        store,
        {_PROVIDER_ID: provider},
        publish,
        on_run_terminal=service.notify_run_terminal,
    )


@pytest.mark.asyncio
async def test_first_run_terminal_generates_and_publishes_a_title(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread_id, run_id = _thread_with_first_turn(
            store,
            content="Please fix the parser bug in the loader.",
        )
        provider = _TitleProvider('\n  "Fix the parser bug"  \n')
        published: list[JournalEvent] = []
        publish = _publisher(published)
        service = _service(store=store, provider=provider, publish=publish)

        await _loop(store, provider, service, publish).run(
            run_id,
            CancellationToken(),
        )
        await _drain(service)

        assert store.run_status(run_id) == "completed"
        thread = store.get_thread(thread_id).thread
        assert thread.title == "Fix the parser bug"
        assert store.thread_title_source(thread_id) == "auto"

        assert len(provider.title_requests) == 1
        request = provider.title_requests[0]
        assert request.model_id == _MODEL_ID
        assert request.tools == ()
        assert request.max_output_tokens == THREAD_TITLE_MAX_OUTPUT_TOKENS
        assert "Please fix the parser bug in the loader." in request.messages[-1].content

        renamed = [event for event in published if event.type == "thread.renamed"]
        assert len(renamed) == 1
        assert renamed[0].payload["titleSource"] == "auto"
        assert renamed[0].payload["thread"]["title"] == "Fix the parser bug"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_without_a_title_gets_one(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread_id, run_id = _thread_with_first_turn(
            store,
            content="Explain the scheduler.",
            title=None,
        )
        provider = _TitleProvider("Scheduler overview")
        service = _service(store=store, provider=provider)

        await _loop(store, provider, service).run(run_id, CancellationToken())
        await _drain(service)

        assert store.get_thread(thread_id).thread.title == "Scheduler overview"
        assert store.thread_title_source(thread_id) == "auto"
        assert len(provider.title_requests) == 1
    finally:
        store.close()


@pytest.mark.asyncio
async def test_generated_titles_are_bounded_to_sixty_characters(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread_id, run_id = _thread_with_first_turn(store, content="A long request.")
        provider = _TitleProvider("A" * 200)
        service = _service(store=store, provider=provider)

        await _loop(store, provider, service).run(run_id, CancellationToken())
        await _drain(service)

        title = store.get_thread(thread_id).thread.title
        assert title == "A" * THREAD_TITLE_MAX_CHARACTERS
    finally:
        store.close()


@pytest.mark.asyncio
async def test_user_rename_is_never_overwritten(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread_id, run_id = _thread_with_first_turn(store, content="Name this chat.")
        _renamed, event = store.rename_thread(
            thread_id,
            "My own title",
            title_source="user",
        )
        assert event is not None
        provider = _TitleProvider("Generated title")
        service = _service(store=store, provider=provider)

        await _loop(store, provider, service).run(run_id, CancellationToken())
        await _drain(service)

        assert store.get_thread(thread_id).thread.title == "My own title"
        assert store.thread_title_source(thread_id) == "user"
        assert provider.title_requests == []
    finally:
        store.close()


@pytest.mark.asyncio
async def test_user_rename_during_generation_wins(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread_id, run_id = _thread_with_first_turn(store, content="Race the title.")
        provider = _BlockingTitleProvider("Generated title")
        service = _service(store=store, provider=provider)

        await _loop(store, provider, service).run(run_id, CancellationToken())
        await asyncio.wait_for(provider.started.wait(), timeout=5)
        store.rename_thread(thread_id, "User title", title_source="user")
        provider.release.set()
        await _drain(service)

        assert store.get_thread(thread_id).thread.title == "User title"
        assert store.thread_title_source(thread_id) == "user"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_failed_generation_keeps_the_previous_title(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread_id, run_id = _thread_with_first_turn(store, content="Fail the title.")
        provider = _TitleProvider(
            ProviderFailure("server", "provider is unavailable", retryable=True)
        )
        published: list[JournalEvent] = []
        publish = _publisher(published)
        service = _service(store=store, provider=provider, publish=publish)

        await _loop(store, provider, service, publish).run(
            run_id,
            CancellationToken(),
        )
        await _drain(service)

        assert store.run_status(run_id) == "completed"
        assert store.get_thread(thread_id).thread.title == "Placeholder title"
        assert store.thread_title_source(thread_id) == "provisional"
        assert [event for event in published if event.type == "thread.renamed"] == []
        assert len(provider.title_requests) == 1
    finally:
        store.close()


@pytest.mark.asyncio
async def test_blank_generation_keeps_the_previous_title(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread_id, run_id = _thread_with_first_turn(store, content="Blank title.")
        provider = _TitleProvider('\n  ""  \n')
        service = _service(store=store, provider=provider)

        await _loop(store, provider, service).run(run_id, CancellationToken())
        await _drain(service)

        assert store.get_thread(thread_id).thread.title == "Placeholder title"
        assert store.thread_title_source(thread_id) == "provisional"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_same_run_generates_at_most_one_title(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread_id, run_id = _thread_with_first_turn(store, content="Only once.")
        provider = _TitleProvider(
            ProviderFailure("timeout", "provider timed out", retryable=True)
        )
        service = _service(store=store, provider=provider)

        await _loop(store, provider, service).run(run_id, CancellationToken())
        await _drain(service)
        assert len(provider.title_requests) == 1

        config = store.get_run_config(run_id)
        service.notify_run_terminal(config, "completed")
        service.notify_run_terminal(config, "failed")
        await _drain(service)

        assert len(provider.title_requests) == 1
        assert store.get_thread(thread_id).thread.title == "Placeholder title"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_only_the_first_run_triggers_title_generation(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread_id, run_id = _thread_with_first_turn(store, content="First turn.")
        provider = _TitleProvider(
            ProviderFailure("server", "provider is unavailable", retryable=True)
        )
        service = _service(store=store, provider=provider)
        loop = _loop(store, provider, service)

        await loop.run(run_id, CancellationToken())
        await _drain(service)
        assert len(provider.title_requests) == 1

        thread = store.get_thread(thread_id).thread
        second = prepare_turn(
            store,
            thread_id=thread_id,
            branch_id=thread.default_branch_id,
            content="Second turn.",
            provider_id=_PROVIDER_ID,
            model_id=_MODEL_ID,
        )
        await loop.run(second.run_id, CancellationToken())
        await _drain(service)

        assert len(provider.title_requests) == 1
        assert store.get_thread(thread_id).thread.title == "Placeholder title"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_title_generation_never_blocks_run_settlement(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread_id, run_id = _thread_with_first_turn(store, content="Do not block.")
        provider = _BlockingTitleProvider("Generated title")
        published: list[JournalEvent] = []
        publish = _publisher(published)
        service = _service(store=store, provider=provider, publish=publish)

        await _loop(store, provider, service, publish).run(
            run_id,
            CancellationToken(),
        )

        assert store.run_status(run_id) == "completed"
        assert any(event.type == "run.settled" for event in published)
        await asyncio.wait_for(provider.started.wait(), timeout=5)
        assert store.get_thread(thread_id).thread.title == "Placeholder title"

        provider.release.set()
        await _drain(service)
        assert store.get_thread(thread_id).thread.title == "Generated title"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_protected_submission_never_reaches_the_provider(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        secret = "sk-verysecretvalue"
        thread_id, run_id = _thread_with_first_turn(
            store,
            content=f"My key is {secret}.",
        )
        provider = _TitleProvider("Leaked key")
        service = _service(
            store=store,
            provider=provider,
            protected_values=(secret,),
        )

        await _loop(store, provider, service).run(run_id, CancellationToken())
        await _drain(service)

        assert provider.title_requests == []
        assert store.get_thread(thread_id).thread.title == "Placeholder title"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_close_cancels_in_flight_title_generation(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread_id, run_id = _thread_with_first_turn(store, content="Shut down.")
        provider = _BlockingTitleProvider("Generated title")
        service = _service(store=store, provider=provider)

        await _loop(store, provider, service).run(run_id, CancellationToken())
        await asyncio.wait_for(provider.started.wait(), timeout=5)
        await service.close()

        assert service._tasks == {}
        assert store.get_thread(thread_id).thread.title == "Placeholder title"
    finally:
        store.close()


def test_projection_rebuild_preserves_title_source(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _event = store.create_thread("Placeholder")
        assert store.thread_title_source(thread.id) == "provisional"

        store.rename_thread(thread.id, "Generated", title_source="auto")
        store.rebuild_projections()
        assert store.thread_title_source(thread.id) == "auto"

        store.rename_thread(thread.id, "Mine", title_source="user")
        store.rebuild_projections()
        assert store.thread_title_source(thread.id) == "user"
        assert store.get_thread(thread.id).thread.title == "Mine"
    finally:
        store.close()


def test_auto_rename_never_replaces_a_user_title(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _event = store.create_thread("Placeholder")
        store.rename_thread(thread.id, "Mine", title_source="user")
        _thread, event = store.rename_thread(
            thread.id,
            "Generated",
            title_source="auto",
        )

        assert event is None
        assert store.get_thread(thread.id).thread.title == "Mine"
        assert store.thread_title_source(thread.id) == "user"
    finally:
        store.close()