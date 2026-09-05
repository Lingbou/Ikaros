from __future__ import annotations

import codecs
import os
from collections.abc import Iterator
from pathlib import Path
from typing import BinaryIO
from unittest.mock import patch

import pytest

from ikaros_runtime.domain import WorkspaceSummary
from ikaros_runtime.errors import InvalidParamsError
from ikaros_runtime.file_preview import preview_text_file
from ikaros_runtime.paths import RuntimePaths
from ikaros_runtime.security import RuntimeSecurity
from ikaros_runtime.services.files import FileService
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools.read import ReadTool

from .helpers import prepare_turn


@pytest.mark.parametrize("body,bom", [(b"", False), (codecs.BOM_UTF8, True)])
def test_empty_file_is_a_successful_empty_page(tmp_path: Path, body: bytes, bom: bool) -> None:
    target = tmp_path / "empty.txt"
    target.write_bytes(body)
    page = preview_text_file(thread_id="thread", path=target)
    assert page["status"] == "text"
    assert (page["content"], page["lineStart"], page["lineEnd"]) == ("", 1, 0)
    assert page["bom"] is bom
    assert page["nextOffset"] is None and page["truncated"] is False


@pytest.mark.parametrize("newline", ["\n", "\r\n", "\r"])
def test_unicode_and_original_line_endings_survive_pagination(tmp_path: Path, newline: str) -> None:
    target = tmp_path / "中文.txt"
    content = newline.join(f"中文第 {number} 行" for number in range(2003)) + newline
    target.write_bytes(codecs.BOM_UTF8 + content.encode("utf-8"))
    first = preview_text_file(thread_id="thread", path=target)
    assert first["status"] == "text"
    assert first["lineEnd"] == 2000 and first["nextOffset"] == 2001
    assert first["truncationReason"] == "line_limit"
    second = preview_text_file(
        thread_id="thread",
        path=target,
        offset=first["nextOffset"],
        expected_revision=first["revision"],
    )
    assert second["status"] == "text"
    assert second["lineStart"] == 2001 and second["lineEnd"] == 2003
    assert second["nextOffset"] is None
    assert first["content"] + second["content"] == content
    assert first["bom"] and second["bom"]


def test_page_byte_limit_keeps_complete_lines_and_exact_fit(tmp_path: Path) -> None:
    target = tmp_path / "large.txt"
    line = "界" * 100 + "\r\n"
    target.write_bytes((line * 1000).encode())
    first = preview_text_file(thread_id="thread", path=target)
    assert first["truncationReason"] == "byte_limit"
    assert len(first["content"].encode("utf-8")) <= 50 * 1024
    second = preview_text_file(
        thread_id="thread",
        path=target,
        offset=first["nextOffset"],
        expected_revision=first["revision"],
    )
    assert second["lineStart"] == first["lineEnd"] + 1
    assert second["content"].startswith(line)
    target.write_bytes(b"a" * (50 * 1024 - 1) + b"\n")
    exact = preview_text_file(thread_id="thread", path=target)
    assert exact["status"] == "text" and exact["truncated"] is False
    target.write_bytes(b"a" * (50 * 1024) + b"\n")
    assert preview_text_file(thread_id="thread", path=target)["reason"] == "too_large"


@pytest.mark.parametrize(
    "body,reason",
    [
        (b"abc\x00def", "binary_file"),
        (b"hello\xff\n", "unsupported_encoding"),
        (b"\xff\xfea\x00", "binary_file"),
    ],
)
def test_invalid_text_has_no_body(tmp_path: Path, body: bytes, reason: str) -> None:
    target = tmp_path / "bad.txt"
    target.write_bytes(body)
    result = preview_text_file(thread_id="thread", path=target)
    assert result["reason"] == reason
    assert "content" not in result


def test_deleted_directory_and_changed_file_require_clear_action(tmp_path: Path) -> None:
    target = tmp_path / "data.txt"
    target.write_text("old\n" * 2001)
    first = preview_text_file(thread_id="thread", path=target)
    target.write_text("new\n" * 2001)
    result = preview_text_file(
        thread_id="thread",
        path=target,
        offset=2001,
        expected_revision=first["revision"],
    )
    assert result["reason"] == "revision_changed"
    assert preview_text_file(thread_id="thread", path=target)["content"].startswith("new")
    target.unlink()
    assert preview_text_file(thread_id="thread", path=target)["reason"] == "file_not_found"
    assert preview_text_file(thread_id="thread", path=tmp_path)["reason"] == "not_a_file"


def test_scanning_is_bounded_even_for_far_offsets(tmp_path: Path) -> None:
    target = tmp_path / "many-lines.txt"
    target.write_bytes(b"a\n" * 5000)
    with patch("ikaros_runtime.file_preview.PREVIEW_SCAN_BYTES", 4096):
        result = preview_text_file(thread_id="thread", path=target, offset=3000)
    assert result["reason"] == "scan_limit"


def test_credential_crossing_page_boundary_is_not_exposed(tmp_path: Path) -> None:
    target = tmp_path / "protected.txt"
    target.write_text("safe\n" * 1999 + "secret-prefix\nsecret-suffix\n")
    protected = ("secret-prefix\nsecret-suffix",)
    for offset in (1, 2001):
        result = preview_text_file(
            thread_id="thread",
            path=target,
            offset=offset,
            protected_values=protected,
        )
        assert result["reason"] == "protected_content"
        assert "content" not in result


@pytest.mark.asyncio
async def test_service_uses_saved_workspace_and_has_no_journal_side_effect(tmp_path: Path) -> None:
    paths = RuntimePaths.from_home(tmp_path / "runtime")
    store = SqliteRuntimeStore(paths.state_db)
    try:
        workspace = tmp_path / "workspace"
        workspace.mkdir()
        target = workspace / "generated-by-script.txt"
        target.write_text("one\ntwo\n")
        thread, _ = store.create_thread(
            "Inspect",
            workspace=WorkspaceSummary("project", "Project", str(workspace)),
        )
        security = RuntimeSecurity(lambda: (), store.journal_contains_protected_values)
        service = FileService(store, security, paths)
        before = store.latest_sequence()
        page = await service.preview({"threadId": thread.id, "path": target.name})
        assert page["content"] == "one\ntwo\n" and page["path"] == str(target)
        assert store.latest_sequence() == before
        with pytest.raises(InvalidParamsError):
            await service.preview({"threadId": thread.id, "path": "../outside.txt"})
        with pytest.raises(InvalidParamsError):
            await service.preview({"threadId": thread.id, "path": target.name, "offset": 2})
        with pytest.raises(InvalidParamsError):
            await service.preview(
                {"threadId": thread.id, "path": target.name, "workspace": str(tmp_path)}
            )
        with pytest.raises(InvalidParamsError):
            await service.preview({"threadId": "foreign", "path": str(target)})
    finally:
        store.close()


@pytest.mark.asyncio
async def test_runtime_internal_files_and_symlink_escape_are_not_previewed(tmp_path: Path) -> None:
    paths = RuntimePaths.from_home(tmp_path / "runtime")
    store = SqliteRuntimeStore(paths.state_db)
    try:
        paths.config.write_text("sensitive configuration")
        thread, _ = store.create_thread(
            "Inspect",
            workspace=WorkspaceSummary("all", "All", str(tmp_path)),
        )
        service = FileService(
            store,
            RuntimeSecurity(lambda: (), store.journal_contains_protected_values),
            paths,
        )
        for owned in (paths.config, paths.state_db):
            assert (await service.preview({"threadId": thread.id, "path": str(owned)}))[
                "reason"
            ] == "protected_content"
        hardlink = tmp_path / "config-alias.txt"
        os.link(paths.config, hardlink)
        assert (await service.preview({"threadId": thread.id, "path": str(hardlink)}))[
            "reason"
        ] == "protected_content"
        if os.name == "nt":
            # NTFS hardlink protection is exercised above. POSIX also exercises
            # symlink traversal, which needs extra privileges on Windows.
            return
        outside = tmp_path.parent / (tmp_path.name + "-outside.txt")
        outside.write_text("outside")
        alias = tmp_path / "escape.txt"
        try:
            alias.symlink_to(outside)
            with pytest.raises(InvalidParamsError):
                await service.preview({"threadId": thread.id, "path": str(alias)})
        finally:
            outside.unlink()
    finally:
        store.close()


@pytest.mark.asyncio
async def test_outside_preview_requires_matching_same_thread_file_tool(tmp_path: Path) -> None:
    paths = RuntimePaths.from_home(tmp_path / "runtime")
    store = SqliteRuntimeStore(paths.state_db)
    try:
        target = tmp_path / "outside.txt"
        target.write_text("already inspected")
        thread, _ = store.create_thread("No workspace")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="Read",
            provider_id="scripted",
            model_id="scripted-v1",
            tools=(ReadTool.definition,),
        )
        store.mark_run_running(prepared.run_id)
        item, _ = store.create_tool_call_item(
            prepared.run_id,
            step_id="step",
            call_id="call",
            tool_name="read",
            arguments={"filePath": str(target)},
        )
        store.complete_tool_call(
            item, status="completed", result={"path": str(target)}, result_content="{}"
        )
        service = FileService(
            store,
            RuntimeSecurity(lambda: (), store.journal_contains_protected_values),
            paths,
        )
        params = {"threadId": thread.id, "path": str(target), "sourceToolCallItemId": item}
        assert (await service.preview(params))["content"] == "already inspected"
        other, _ = store.create_thread("Other")
        with pytest.raises(InvalidParamsError):
            await service.preview({**params, "threadId": other.id})
        with pytest.raises(InvalidParamsError):
            await service.preview({**params, "path": str(tmp_path / "different.txt")})
        with pytest.raises(InvalidParamsError):
            await service.preview({"threadId": thread.id, "path": str(target)})
        with pytest.raises(InvalidParamsError):
            await service.preview({**params, "sourceToolCallItemId": None})
        if os.name != "nt":
            alternate = tmp_path / "alternate.txt"
            alternate.write_text("unbound")
            target.unlink()
            target.symlink_to(alternate)
            with pytest.raises(InvalidParamsError):
                await service.preview(params)
    finally:
        store.close()


def test_file_changed_during_read_never_returns_mixed_text(tmp_path: Path) -> None:
    from ikaros_runtime import file_preview

    target = tmp_path / "changing.txt"
    target.write_text("before\n" * 2001)
    original = file_preview._lines

    def changing_lines(stream: BinaryIO, size: int) -> Iterator[bytes]:
        for line in original(stream, size):
            target.write_text("after\n" * 2001)
            yield line
            return

    with patch.object(file_preview, "_lines", changing_lines):
        result = preview_text_file(thread_id="thread", path=target)
    assert result["reason"] == "revision_changed" and "content" not in result
