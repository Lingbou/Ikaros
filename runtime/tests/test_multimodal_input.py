from __future__ import annotations

import base64
import hashlib
from collections.abc import Callable, Sequence
from pathlib import Path
from typing import cast

import pytest

from ikaros_runtime.agent import ContextBuilder, ModelInputPlanner
from ikaros_runtime.agent.scheduler import AgentScheduler
from ikaros_runtime.attachments import (
    AttachmentRef,
    attachment_refs_to_wire,
    message_attachment_refs,
    resolve_attachment_inputs,
)
from ikaros_runtime.domain import ContextItem
from ikaros_runtime.errors import InvalidParamsError
from ikaros_runtime.identity import load_identity_core
from ikaros_runtime.providers.base import ImageUrlPart, ProviderMessage
from ikaros_runtime.providers.registry import ConfigStore
from ikaros_runtime.security import RuntimeSecurity
from ikaros_runtime.services.turns import TurnService
from ikaros_runtime.storage import SqliteRuntimeStore

from .fakes.scripted_provider import scripted_provider_snapshot
from .helpers import bounded_budget, run_config

PNG_BYTES = b"\x89PNG\r\n\x1a\n" + b"ikaros-image-body"
PNG_DATA_URL = "data:image/png;base64," + base64.b64encode(PNG_BYTES).decode("ascii")


def write_image(root: Path, name: str = "pixel.png") -> Path:
    path = root / name
    path.write_bytes(PNG_BYTES)
    return path


def image_input(path: Path) -> dict[str, object]:
    return {"kind": "image", "path": str(path), "mimeType": "image/png"}


def user_item_with_attachments(*inputs: dict[str, object]) -> ContextItem:
    refs = resolve_attachment_inputs(list(inputs))
    return ContextItem(
        kind="message",
        role="user",
        content="describe this",
        data={"attachments": attachment_refs_to_wire(refs)},
    )


def build_user_message(item: ContextItem, model_id: str) -> ProviderMessage:
    plan = ModelInputPlanner().build_plan(
        config=run_config("provider", model_id),
        items=(item,),
        budget_snapshot=bounded_budget(),
    )
    return next(
        message
        for message in ContextBuilder().build_request(plan).messages
        if message.role == "user"
    )


def test_vision_model_receives_inline_image_parts(tmp_path: Path) -> None:
    image = write_image(tmp_path)
    message = build_user_message(
        user_item_with_attachments(image_input(image)),
        "deepseek-v4-flash-vision-exp",
    )

    assert message.parts == (ImageUrlPart(url=PNG_DATA_URL),)
    assert "describe this" in message.content
    assert str(image) in message.content
    assert "Attached files (local paths on the Runtime host):" in message.content


def test_text_only_model_degrades_images_to_paths(tmp_path: Path) -> None:
    image = write_image(tmp_path)
    message = build_user_message(
        user_item_with_attachments(image_input(image)), "deepseek-v4-flash"
    )

    assert message.parts == ()
    assert "cannot view images" in message.content
    assert str(image) in message.content
    assert hashlib.sha256(PNG_BYTES).hexdigest() in message.content


def test_missing_image_file_degrades_instead_of_silently_dropping(tmp_path: Path) -> None:
    image = write_image(tmp_path)
    item = user_item_with_attachments(image_input(image))
    image.unlink()

    message = build_user_message(item, "deepseek-v4-flash-vision-exp")

    assert message.parts == ()
    assert "image was not sent" in message.content
    assert "missing" in message.content
    assert str(image) in message.content


def test_text_attachment_is_a_readable_path_reference(tmp_path: Path) -> None:
    notes = tmp_path / "notes.md"
    notes.write_text("# Notes\n", encoding="utf-8")
    item = user_item_with_attachments(
        {"kind": "text", "path": str(notes), "mimeType": "text/markdown"}
    )

    message = build_user_message(item, "deepseek-v4-flash-vision-exp")

    assert message.parts == ()
    assert "[text]" in message.content
    assert str(notes) in message.content
    assert "read the file with the read tool" in message.content


class _RecordingScheduler:
    """Records scheduling calls and mirrors the Agent Loop steer persistence."""

    def __init__(self, store: SqliteRuntimeStore | None = None) -> None:
        self.reserved: list[str] = []
        self.steers: list[tuple[str, str, str, tuple[AttachmentRef, ...]]] = []
        self.created: list[bool] = []
        self._store = store

    def reserve(self, run_id: str) -> None:
        self.reserved.append(run_id)

    async def steer(
        self,
        run_id: str,
        content: str,
        request_id: str,
        attachments: Sequence[AttachmentRef] = (),
    ) -> bool:
        self.steers.append((run_id, content, request_id, tuple(attachments)))
        if self._store is not None:
            _event, created = self._store.append_steer_item_idempotent(
                run_id, content, request_id, tuple(attachments)
            )
            self.created.append(created)
        return True


def turn_service(
    store: SqliteRuntimeStore,
    tmp_path: Path,
    scheduler: _RecordingScheduler,
    assert_request_safe: Callable[[object], None] | None = None,
) -> TurnService:
    return TurnService(
        store,
        cast(AgentScheduler, scheduler),
        ConfigStore(tmp_path),
        assert_request_safe or (lambda _value: None),
        load_identity_core(),
        provider_snapshot=scripted_provider_snapshot,
    )


def start_params(thread_id: str, branch_id: str, **extra: object) -> dict[str, object]:
    return {
        "threadId": thread_id,
        "branchId": branch_id,
        "content": "describe this",
        "providerId": "scripted",
        "modelId": "scripted-v1",
        **extra,
    }


def test_turn_start_records_attachment_references(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    scheduler = _RecordingScheduler()
    service = turn_service(store, tmp_path, scheduler)
    try:
        thread, _branch = store.create_thread("Multimodal")
        image = write_image(tmp_path)
        params = start_params(
            thread.id,
            thread.default_branch_id,
            clientRequestId="request-attachments",
            attachments=[image_input(image)],
        )

        started = service.start_turn(params)
        assert service.start_turn(params).result == started.result
        assert len(scheduler.reserved) == 1

        items = store.context_items(
            thread.default_branch_id,
            through_turn_id=str(started.result["turnId"]),
        )
        (reference,) = message_attachment_refs(items[0].data)
        assert reference.path == str(image)
        assert reference.size == len(PNG_BYTES)
        assert reference.sha256 == hashlib.sha256(PNG_BYTES).hexdigest()
        assert items[0].content == "describe this"
    finally:
        store.close()


def test_turn_start_rejects_missing_and_duplicate_attachments(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    service = turn_service(store, tmp_path, _RecordingScheduler())
    try:
        thread, _branch = store.create_thread("Multimodal")
        params = start_params(thread.id, thread.default_branch_id)
        image = write_image(tmp_path)

        with pytest.raises(InvalidParamsError, match="was not found"):
            service.start_turn({**params, "attachments": [image_input(tmp_path / "missing.png")]})
        with pytest.raises(InvalidParamsError, match="paths must be unique"):
            service.start_turn({**params, "attachments": [image_input(image), image_input(image)]})

        service.start_turn(
            {**params, "clientRequestId": "request-1", "attachments": [image_input(image)]}
        )
        with pytest.raises(InvalidParamsError, match="different turn.start parameters"):
            service.start_turn(
                {
                    **params,
                    "clientRequestId": "request-1",
                    "attachments": [image_input(write_image(tmp_path, "other.png"))],
                }
            )
    finally:
        store.close()


def test_turn_start_scans_attachment_paths_for_protected_values(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    security = RuntimeSecurity(lambda: ("sk-protected-value",), lambda _values: False)
    service = turn_service(
        store,
        tmp_path,
        _RecordingScheduler(),
        security.assert_request_safe,
    )
    try:
        thread, _branch = store.create_thread("Multimodal")
        protected = tmp_path / "sk-protected-value"
        protected.mkdir()
        image = write_image(protected)

        with pytest.raises(InvalidParamsError, match="protected configuration data"):
            service.start_turn(
                start_params(
                    thread.id,
                    thread.default_branch_id,
                    attachments=[image_input(image)],
                )
            )
    finally:
        store.close()


@pytest.mark.asyncio
async def test_steer_records_attachment_references_idempotently(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    scheduler = _RecordingScheduler(store)
    service = turn_service(store, tmp_path, scheduler)
    try:
        thread, _branch = store.create_thread("Multimodal")
        started = service.start_turn(start_params(thread.id, thread.default_branch_id))
        run_id = str(started.result["runId"])
        store.mark_run_running(run_id)
        image = write_image(tmp_path)
        params = {
            "runId": run_id,
            "expectedRunId": run_id,
            "content": "and this one",
            "clientRequestId": "steer-attachments",
            "attachments": [image_input(image)],
        }

        accepted = await service.steer_run(params)
        assert accepted.result["status"] == "received"
        assert len(scheduler.steers) == 1
        item = store.find_steer_item(run_id, "steer-attachments")
        assert item is not None
        (reference,) = message_attachment_refs(item["data"])
        assert reference.sha256 == hashlib.sha256(PNG_BYTES).hexdigest()

        repeated = await service.steer_run(params)
        assert repeated.result == accepted.result
        assert scheduler.created == [True, False]
        assert store.find_steer_item(run_id, "steer-attachments") is not None

        with pytest.raises(InvalidParamsError, match="different steer content"):
            await service.steer_run(
                {
                    **params,
                    "attachments": [image_input(write_image(tmp_path, "other.png"))],
                }
            )
    finally:
        store.close()


def test_history_replay_rebuilds_image_parts_from_stored_items(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    service = turn_service(store, tmp_path, _RecordingScheduler())
    try:
        thread, _branch = store.create_thread("Multimodal")
        image = write_image(tmp_path)
        started = service.start_turn(
            start_params(
                thread.id,
                thread.default_branch_id,
                attachments=[image_input(image)],
            )
        )
        run_id = str(started.result["runId"])
        turn_id = str(started.result["turnId"])

        items = store.context_items(thread.default_branch_id, through_turn_id=turn_id)
        plan = ModelInputPlanner().build_plan(
            config=store.get_run_config(run_id),
            items=items,
            budget_snapshot=bounded_budget(),
        )
        messages = (
            ContextBuilder(supports_vision=lambda model_id: model_id == "scripted-v1")
            .build_request(plan)
            .messages
        )
        message = next(message for message in messages if message.role == "user")

        assert message.parts == (ImageUrlPart(url=PNG_DATA_URL),)
        assert str(image) in message.content
    finally:
        store.close()
