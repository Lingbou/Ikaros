from __future__ import annotations

import asyncio
import codecs
import json
import os
import stat
import threading
from pathlib import Path
from typing import Any

import pytest

from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.errors import RunCancelled
from ikaros_runtime.tools import edit as edit_tool
from ikaros_runtime.tools import file_common
from ikaros_runtime.tools import write as write_tool
from ikaros_runtime.tools.core import (
    ToolCall,
    ToolExecutionContext,
    ToolExecutor,
    ToolRegistry,
)
from ikaros_runtime.tools.edit import EditTool
from ikaros_runtime.tools.policy import FullAccessPolicy
from ikaros_runtime.tools.read import ReadTool
from ikaros_runtime.tools.write import WriteTool


def _context(*, default_cwd: str | None = None) -> ToolExecutionContext:
    return ToolExecutionContext("run_test", 1, "item_test", "thread_test", default_cwd)


def _file_executor() -> ToolExecutor:
    return ToolExecutor(
        ToolRegistry([ReadTool(), WriteTool(), EditTool()]),
        FullAccessPolicy(),
    )


@pytest.mark.asyncio
async def test_read_returns_numbered_utf8_lines_with_offset_and_limit(tmp_path: Path) -> None:
    path = tmp_path / "notes.txt"
    path.write_text("alpha\n中文\ngamma\ndelta\n", encoding="utf-8")

    result = await _file_executor().execute(
        ToolCall("read-lines", "read", {"filePath": str(path), "offset": 2, "limit": 2}),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is True
    assert "2: 中文\n3: gamma" in result.output
    assert "Use offset=4 to continue" in result.output
    assert result.details == {
        "path": str(path.resolve()),
        "lineStart": 2,
        "lineEnd": 3,
        "bytesRead": len(path.read_bytes()),
        "bom": False,
        "truncated": True,
        "lineTruncations": 0,
        "nextOffset": 4,
    }


@pytest.mark.asyncio
async def test_read_resolves_relative_paths_from_the_thread_workspace(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    path = workspace / "nested" / "notes.txt"
    path.parent.mkdir(parents=True)
    path.write_text("workspace text", encoding="utf-8")

    result = await _file_executor().execute(
        ToolCall("read-relative", "read", {"filePath": "nested/notes.txt"}),
        cancellation=CancellationToken(),
        context=_context(default_cwd=str(workspace)),
    )

    assert result.ok is True
    assert result.details["path"] == str(path.resolve())
    assert "1: workspace text" in result.output


@pytest.mark.asyncio
async def test_read_accepts_an_absolute_path_without_a_workspace(tmp_path: Path) -> None:
    path = tmp_path / "absolute.txt"
    path.write_text("absolute", encoding="utf-8")

    result = await _file_executor().execute(
        ToolCall("read-absolute", "read", {"filePath": str(path)}),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is True
    assert result.details["path"] == str(path.resolve())


@pytest.mark.asyncio
async def test_read_resolves_relative_paths_from_the_runtime_cwd(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    path = tmp_path / "cwd.txt"
    path.write_text("runtime cwd", encoding="utf-8")
    monkeypatch.chdir(tmp_path)

    result = await _file_executor().execute(
        ToolCall("read-cwd", "read", {"filePath": "cwd.txt"}),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is True
    assert result.details["path"] == str(path.resolve())


@pytest.mark.asyncio
async def test_read_preserves_bom_metadata_and_handles_crlf(tmp_path: Path) -> None:
    path = tmp_path / "windows.txt"
    path.write_bytes(codecs.BOM_UTF8 + b"first\r\nsecond\r\n")

    result = await _file_executor().execute(
        ToolCall("read-bom", "read", {"filePath": str(path)}),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is True
    assert result.details["bom"] is True
    assert "1: first\n2: second" in result.output
    assert "\ufeff" not in result.output


@pytest.mark.asyncio
async def test_read_caps_long_lines_and_total_output(tmp_path: Path) -> None:
    path = tmp_path / "large.txt"
    path.write_text("\n".join("x" * 2_500 for _ in range(80)), encoding="utf-8")

    result = await _file_executor().execute(
        ToolCall("read-large", "read", {"filePath": str(path)}),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is True
    assert result.details["truncated"] is True
    assert result.details["lineTruncations"] > 0
    assert "totalLines" not in result.details
    assert result.details["nextOffset"] == result.details["lineEnd"] + 1
    assert "line truncated to 2000 chars" in result.output
    assert "Output capped at 50 KB" in result.output
    assert len(result.output.encode("utf-8")) < 52 * 1024


@pytest.mark.asyncio
async def test_read_streams_large_files_and_stops_after_the_requested_page(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    path = tmp_path / "huge.txt"
    path.write_bytes(b"first\n" + b"tail\n" * 100_000)

    def forbidden_read_bytes(_path: Path) -> bytes:
        raise AssertionError("bounded read must not load the entire file")

    monkeypatch.setattr(Path, "read_bytes", forbidden_read_bytes)
    result = await _file_executor().execute(
        ToolCall("read-one-page", "read", {"filePath": str(path), "limit": 1}),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is True
    assert "1: first" in result.output
    assert result.details["truncated"] is True
    assert result.details["nextOffset"] == 2
    assert result.details["bytesRead"] < path.stat().st_size


@pytest.mark.asyncio
async def test_read_does_not_consume_invalid_content_beyond_the_requested_page(
    tmp_path: Path,
) -> None:
    path = tmp_path / "page-boundary.txt"
    path.write_bytes(b"first\n" + b"safe-prefix" + b"\x00\xff")

    result = await _file_executor().execute(
        ToolCall("read-page-boundary", "read", {"filePath": str(path), "limit": 1}),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is True
    assert "1: first" in result.output
    assert result.details["nextOffset"] == 2


@pytest.mark.asyncio
async def test_read_rejects_binary_content_after_the_initial_four_kibibytes(
    tmp_path: Path,
) -> None:
    path = tmp_path / "late-binary.txt"
    path.write_bytes(b"x" * 5_000 + b"\x00" + b"tail")

    result = await _file_executor().execute(
        ToolCall("read-late-binary", "read", {"filePath": str(path)}),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is False
    assert result.details["errorCode"] == "binary_file"


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("file_kind", "expected_code"),
    [
        ("missing", "file_not_found"),
        ("directory", "path_is_directory"),
        ("binary", "binary_file"),
        ("invalid_utf8", "unsupported_encoding"),
    ],
)
async def test_read_rejects_non_text_inputs(
    tmp_path: Path,
    file_kind: str,
    expected_code: str,
) -> None:
    path = tmp_path / file_kind
    if file_kind == "directory":
        path.mkdir()
    elif file_kind == "binary":
        path.write_bytes(b"text\x00binary")
    elif file_kind == "invalid_utf8":
        path.write_bytes(b"\xff\xfe\xfd")

    result = await _file_executor().execute(
        ToolCall(f"read-{file_kind}", "read", {"filePath": str(path)}),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is False
    assert result.details["errorCode"] == expected_code
    assert str(path.resolve()) in result.output


@pytest.mark.asyncio
async def test_read_rejects_known_binary_extensions_even_with_text_like_bytes(
    tmp_path: Path,
) -> None:
    path = tmp_path / "archive.zip"
    path.write_bytes(b"PK text-like test fixture")

    result = await _file_executor().execute(
        ToolCall("read-binary-extension", "read", {"filePath": str(path)}),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is False
    assert result.details["errorCode"] == "binary_file"


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "arguments",
    [
        {},
        {"filePath": ""},
        {"filePath": "bad\x00path"},
        {"filePath": "file", "offset": 0},
        {"filePath": "file", "offset": True},
        {"filePath": "file", "limit": 0},
        {"filePath": "file", "limit": 2_001},
        {"filePath": "file", "limit": "2"},
        {"filePath": "file", "extra": True},
    ],
)
async def test_read_rejects_invalid_arguments(arguments: dict[str, Any]) -> None:
    result = await _file_executor().execute(
        ToolCall("read-invalid", "read", arguments),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is False
    assert result.details["errorCode"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_read_rejects_an_out_of_range_offset_without_guessing(tmp_path: Path) -> None:
    path = tmp_path / "short.txt"
    path.write_text("one\ntwo", encoding="utf-8")

    result = await _file_executor().execute(
        ToolCall("read-offset", "read", {"filePath": str(path), "offset": 3}),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is False
    assert result.details["errorCode"] == "offset_out_of_range"
    assert "2 lines" in result.output


@pytest.mark.asyncio
async def test_read_accepts_an_empty_text_file_at_the_default_offset(tmp_path: Path) -> None:
    path = tmp_path / "empty.txt"
    path.write_bytes(b"")

    result = await _file_executor().execute(
        ToolCall("read-empty", "read", {"filePath": str(path)}),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is True
    assert result.details["totalLines"] == 0
    assert result.details["lineEnd"] == 0
    assert "End of file - total 0 lines" in result.output


@pytest.mark.asyncio
async def test_relative_file_tool_path_does_not_fall_back_from_a_missing_workspace(
    tmp_path: Path,
) -> None:
    missing_workspace = tmp_path / "missing-workspace"

    result = await _file_executor().execute(
        ToolCall("read-missing-workspace", "read", {"filePath": "notes.txt"}),
        cancellation=CancellationToken(),
        context=_context(default_cwd=str(missing_workspace)),
    )

    assert result.ok is False
    assert result.details["errorCode"] == "invalid_workspace"


@pytest.mark.asyncio
async def test_write_creates_parent_directories_and_empty_files(tmp_path: Path) -> None:
    path = tmp_path / "new" / "nested" / "empty.txt"

    result = await _file_executor().execute(
        ToolCall("write-create", "write", {"filePath": str(path), "content": ""}),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is True
    assert path.read_bytes() == b""
    assert result.details["path"] == str(path.resolve())
    assert result.details["created"] is True
    assert result.details["bytesWritten"] == 0
    assert result.details["verified"] is True
    assert "contentHash" not in result.to_wire()
    assert json.loads(result.to_model_content()) == result.to_wire()


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "arguments",
    [
        {},
        {"filePath": "file"},
        {"content": "text"},
        {"filePath": "file", "content": 1},
        {"filePath": "", "content": "text"},
        {"filePath": "file", "content": "text", "extra": True},
    ],
)
async def test_write_rejects_invalid_arguments(arguments: dict[str, Any]) -> None:
    result = await _file_executor().execute(
        ToolCall("write-invalid", "write", arguments),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is False
    assert result.details["errorCode"] == "invalid_arguments"


@pytest.mark.asyncio
async def test_write_overwrites_and_preserves_existing_bom_and_crlf(tmp_path: Path) -> None:
    path = tmp_path / "existing.txt"
    path.write_bytes(codecs.BOM_UTF8 + b"old\r\nvalue\r\n")

    result = await _file_executor().execute(
        ToolCall(
            "write-overwrite",
            "write",
            {"filePath": str(path), "content": "new\nvalue\n"},
        ),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is True
    assert path.read_bytes() == codecs.BOM_UTF8 + b"new\r\nvalue\r\n"
    assert result.details["created"] is False
    assert result.details["bom"] is True
    assert result.details["newline"] == "crlf"


@pytest.mark.asyncio
async def test_write_normalizes_lone_cr_and_mixed_endings_to_the_existing_style(
    tmp_path: Path,
) -> None:
    path = tmp_path / "mixed-write.txt"
    path.write_bytes(b"old\r\nvalue\r\n")

    result = await _file_executor().execute(
        ToolCall(
            "write-mixed-eol",
            "write",
            {"filePath": str(path), "content": "one\rtwo\nthree\r\nfour"},
        ),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is True
    assert path.read_bytes() == b"one\r\ntwo\r\nthree\r\nfour"


@pytest.mark.asyncio
async def test_write_failure_leaves_the_previous_file_intact(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    path = tmp_path / "atomic.txt"
    path.write_text("before", encoding="utf-8")

    def fail_replace(source: str | bytes | Path, destination: str | bytes | Path) -> None:
        del source, destination
        raise OSError("simulated replace failure")

    monkeypatch.setattr(file_common.os, "replace", fail_replace)
    result = await _file_executor().execute(
        ToolCall("write-atomic-failure", "write", {"filePath": str(path), "content": "after"}),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is False
    assert result.details["errorCode"] == "write_failed"
    assert path.read_text(encoding="utf-8") == "before"
    assert await asyncio.to_thread(lambda: list(tmp_path.glob(".atomic.txt.*.tmp"))) == []


def test_atomic_write_does_not_misreport_an_external_write_after_publish(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    path = tmp_path / "published.txt"
    path.write_bytes(b"before")
    original_replace = file_common.os.replace

    def replace_then_overwrite(
        source: str | bytes | Path,
        destination: str | bytes | Path,
    ) -> None:
        original_replace(source, destination)
        target = Path(os.fsdecode(destination))
        target.write_bytes(b"external write")

    monkeypatch.setattr(file_common.os, "replace", replace_then_overwrite)

    assert file_common.atomic_write_bytes(path, b"intended write") is True
    assert path.read_bytes() == b"external write"


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("tool_name", "arguments"),
    [
        ("write", {"content": "invalid \ud800 text"}),
        ("edit", {"oldString": "target", "newString": "invalid \ud800 text"}),
        ("edit", {"oldString": "invalid \ud800 text", "newString": "replacement"}),
    ],
)
async def test_file_mutations_reject_text_that_cannot_be_encoded_as_utf8(
    tmp_path: Path,
    tool_name: str,
    arguments: dict[str, Any],
) -> None:
    path = tmp_path / f"invalid-unicode-{tool_name}.txt"
    path.write_text("before target", encoding="utf-8")
    original = path.read_bytes()

    result = await _file_executor().execute(
        ToolCall(
            f"{tool_name}-invalid-unicode",
            tool_name,
            {"filePath": str(path), **arguments},
        ),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is False
    assert result.details["errorCode"] == "invalid_arguments"
    assert path.read_bytes() == original


@pytest.mark.asyncio
async def test_edit_replaces_one_unique_exact_match(tmp_path: Path) -> None:
    path = tmp_path / "edit.txt"
    path.write_text("before target after", encoding="utf-8")

    result = await _file_executor().execute(
        ToolCall(
            "edit-unique",
            "edit",
            {"filePath": str(path), "oldString": "target", "newString": "updated"},
        ),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is True
    assert path.read_text(encoding="utf-8") == "before updated after"
    assert result.details["replacements"] == 1
    assert result.details["verified"] is True
    assert "contentHash" not in result.to_wire()


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("old_string", "expected_code"),
    [("missing", "no_match"), ("target", "multiple_matches")],
)
async def test_edit_refuses_zero_or_ambiguous_matches_without_modifying(
    tmp_path: Path,
    old_string: str,
    expected_code: str,
) -> None:
    path = tmp_path / f"{expected_code}.txt"
    original = "target and target"
    path.write_text(original, encoding="utf-8")

    result = await _file_executor().execute(
        ToolCall(
            f"edit-{expected_code}",
            "edit",
            {"filePath": str(path), "oldString": old_string, "newString": "updated"},
        ),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is False
    assert result.details["errorCode"] == expected_code
    assert path.read_text(encoding="utf-8") == original


@pytest.mark.asyncio
async def test_edit_rejects_missing_files_and_directories(tmp_path: Path) -> None:
    directory = tmp_path / "folder"
    directory.mkdir()

    for path, code in (
        (tmp_path / "missing.txt", "file_not_found"),
        (directory, "path_is_directory"),
    ):
        result = await _file_executor().execute(
            ToolCall(
                f"edit-{code}",
                "edit",
                {"filePath": str(path), "oldString": "old", "newString": "new"},
            ),
            cancellation=CancellationToken(),
            context=_context(),
        )

        assert result.ok is False
        assert result.details["errorCode"] == code


@pytest.mark.asyncio
@pytest.mark.parametrize("target,updated", [("target", "updated"), ("原文", "修改后")])
async def test_edit_replace_all_changes_every_exact_match(
    tmp_path: Path,
    target: str,
    updated: str,
) -> None:
    path = tmp_path / "all.txt"
    path.write_text(f"{target} and {target}", encoding="utf-8")

    result = await _file_executor().execute(
        ToolCall(
            "edit-all",
            "edit",
            {
                "filePath": str(path),
                "oldString": target,
                "newString": updated,
                "replaceAll": True,
            },
        ),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is True
    assert path.read_text(encoding="utf-8") == f"{updated} and {updated}"
    assert result.details["replacements"] == 2
    assert result.file_change is not None
    assert f"-{target} and {target}\n" in result.file_change.diff
    assert f"+{updated} and {updated}\n" in result.file_change.diff


@pytest.mark.asyncio
async def test_edit_normalizes_requested_line_endings_and_preserves_crlf_bom(
    tmp_path: Path,
) -> None:
    path = tmp_path / "crlf.txt"
    path.write_bytes(codecs.BOM_UTF8 + b"one\r\ntwo\r\nthree\r\n")

    result = await _file_executor().execute(
        ToolCall(
            "edit-crlf",
            "edit",
            {
                "filePath": str(path),
                "oldString": "one\ntwo",
                "newString": "first\nsecond",
            },
        ),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is True
    assert path.read_bytes() == codecs.BOM_UTF8 + b"first\r\nsecond\r\nthree\r\n"
    assert result.details["bom"] is True
    assert result.details["newline"] == "crlf"


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("original", "expected", "newline"),
    [
        (b"one\rtwo\rthree\r", b"one\nupdated\nthree\n", "lf"),
        (b"one\ntwo\r\nthree\n", b"one\nupdated\nthree\n", "lf"),
        (b"one\r\ntwo\nthree\r", b"one\r\nupdated\r\nthree\r\n", "crlf"),
    ],
    ids=["lone-cr", "lf-first-mixed", "crlf-first-mixed"],
)
async def test_edit_normalizes_lone_cr_and_mixed_line_endings(
    tmp_path: Path,
    original: bytes,
    expected: bytes,
    newline: str,
) -> None:
    path = tmp_path / f"{newline}-mixed.txt"
    path.write_bytes(original)

    result = await _file_executor().execute(
        ToolCall(
            f"edit-{newline}-mixed",
            "edit",
            {
                "filePath": str(path),
                "oldString": "two\nthree",
                "newString": "updated\nthree",
            },
        ),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is True
    assert path.read_bytes() == expected
    assert result.details["newline"] == newline


@pytest.mark.asyncio
async def test_edit_refuses_to_overwrite_external_changes_after_its_read(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    path = tmp_path / "stale.txt"
    path.write_text("before target", encoding="utf-8")
    original_atomic_write = edit_tool.atomic_write_bytes

    def change_before_commit(
        target: Path,
        payload: bytes,
        *,
        expected: bytes | None = None,
    ) -> bool:
        target.write_text("external change", encoding="utf-8")
        return original_atomic_write(target, payload, expected=expected)

    monkeypatch.setattr(edit_tool, "atomic_write_bytes", change_before_commit)
    result = await _file_executor().execute(
        ToolCall(
            "edit-stale",
            "edit",
            {"filePath": str(path), "oldString": "target", "newString": "updated"},
        ),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is False
    assert result.details["errorCode"] == "stale_content"
    assert path.read_text(encoding="utf-8") == "external change"


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "arguments",
    [
        {"filePath": "file", "oldString": "", "newString": "new"},
        {"filePath": "file", "oldString": "same", "newString": "same"},
        {"filePath": "file", "oldString": "old", "newString": 1},
        {"filePath": "file", "oldString": "old", "newString": "new", "replaceAll": 1},
    ],
)
async def test_edit_rejects_invalid_arguments(arguments: dict[str, Any]) -> None:
    result = await _file_executor().execute(
        ToolCall("edit-invalid", "edit", arguments),
        cancellation=CancellationToken(),
        context=_context(),
    )

    assert result.ok is False
    assert result.details["errorCode"] in {"invalid_arguments", "no_change"}


@pytest.mark.asyncio
async def test_write_and_edit_share_a_per_path_lock(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    path = tmp_path / "serialized.txt"
    path.write_text("initial target", encoding="utf-8")
    original_atomic_write = write_tool.atomic_write_bytes
    original_read = edit_tool.read_text_document
    write_entered = threading.Event()
    release_write = threading.Event()
    edit_read = threading.Event()

    def paused_atomic_write(
        target: Path,
        payload: bytes,
        *,
        expected: bytes | None = None,
    ) -> bool:
        write_entered.set()
        if not release_write.wait(timeout=5):
            raise TimeoutError("test did not release the write")
        return original_atomic_write(target, payload, expected=expected)

    def observed_edit_read(target: Path) -> file_common.TextDocument:
        edit_read.set()
        return original_read(target)

    monkeypatch.setattr(write_tool, "atomic_write_bytes", paused_atomic_write)
    monkeypatch.setattr(edit_tool, "read_text_document", observed_edit_read)
    write_task = asyncio.create_task(
        _file_executor().execute(
            ToolCall(
                "write-first",
                "write",
                {"filePath": str(path), "content": "written target"},
            ),
            cancellation=CancellationToken(),
            context=_context(),
        )
    )
    assert await asyncio.to_thread(write_entered.wait, 2)
    edit_task = asyncio.create_task(
        _file_executor().execute(
            ToolCall(
                "edit-second",
                "edit",
                {"filePath": str(path), "oldString": "target", "newString": "updated"},
            ),
            cancellation=CancellationToken(),
            context=_context(),
        )
    )
    await asyncio.sleep(0.05)
    assert edit_read.is_set() is False

    release_write.set()
    write_result, edit_result = await asyncio.gather(write_task, edit_task)

    assert write_result.ok is True
    assert edit_result.ok is True
    assert edit_read.is_set() is True
    assert path.read_text(encoding="utf-8") == "written updated"
    assert file_common._PATH_LOCKS == {}


@pytest.mark.asyncio
@pytest.mark.parametrize("tool_name", ["write", "edit"])
async def test_cancelled_mutation_holds_its_lock_until_disk_write_settles(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    tool_name: str,
) -> None:
    path = tmp_path / f"cancel-{tool_name}.txt"
    path.write_text("alpha beta", encoding="utf-8")
    entered = threading.Event()
    release = threading.Event()
    second_entered = threading.Event()
    original_atomic_write = (
        write_tool.atomic_write_bytes if tool_name == "write" else edit_tool.atomic_write_bytes
    )
    invocation_count = 0

    def paused_first_write(
        target: Path,
        payload: bytes,
        *,
        expected: bytes | None = None,
    ) -> bool:
        nonlocal invocation_count
        invocation_count += 1
        if invocation_count == 1:
            entered.set()
            if not release.wait(timeout=5):
                raise TimeoutError("test did not release the first mutation")
        else:
            second_entered.set()
        if tool_name == "write":
            return original_atomic_write(target, payload)
        return original_atomic_write(target, payload, expected=expected)

    module = write_tool if tool_name == "write" else edit_tool
    monkeypatch.setattr(module, "atomic_write_bytes", paused_first_write)
    first_arguments = (
        {"filePath": str(path), "content": "first beta"}
        if tool_name == "write"
        else {"filePath": str(path), "oldString": "alpha", "newString": "first"}
    )
    second_arguments = (
        {"filePath": str(path), "content": "second beta"}
        if tool_name == "write"
        else {"filePath": str(path), "oldString": "beta", "newString": "second"}
    )
    cancellation = CancellationToken()
    first_task = asyncio.create_task(
        _file_executor().execute(
            ToolCall(f"{tool_name}-cancel-first", tool_name, first_arguments),
            cancellation=cancellation,
            context=_context(),
        )
    )
    assert await asyncio.to_thread(entered.wait, 2)
    cancellation.cancel()
    second_task = asyncio.create_task(
        _file_executor().execute(
            ToolCall(f"{tool_name}-cancel-second", tool_name, second_arguments),
            cancellation=CancellationToken(),
            context=_context(),
        )
    )

    await asyncio.sleep(0.05)
    assert second_entered.is_set() is False
    release.set()
    with pytest.raises(RunCancelled):
        await first_task
    second_result = await second_task

    assert second_result.ok is True
    assert second_entered.is_set() is True
    expected_text = "second beta" if tool_name == "write" else "first second"
    assert path.read_text(encoding="utf-8") == expected_text
    temporary_files = await asyncio.to_thread(lambda: list(tmp_path.glob(f".{path.name}.*.tmp")))
    assert temporary_files == []
    assert file_common._PATH_LOCKS == {}


@pytest.mark.asyncio
@pytest.mark.parametrize("tool_name", ["write", "edit"])
async def test_task_cancellation_waits_for_mutation_before_releasing_path_lock(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    tool_name: str,
) -> None:
    path = tmp_path / f"task-cancel-{tool_name}.txt"
    path.write_text("alpha beta", encoding="utf-8")
    entered = threading.Event()
    release = threading.Event()
    second_entered = threading.Event()
    original_atomic_write = (
        write_tool.atomic_write_bytes if tool_name == "write" else edit_tool.atomic_write_bytes
    )
    invocation_count = 0

    def paused_first_write(
        target: Path,
        payload: bytes,
        *,
        expected: bytes | None = None,
    ) -> bool:
        nonlocal invocation_count
        invocation_count += 1
        if invocation_count == 1:
            entered.set()
            if not release.wait(timeout=5):
                raise TimeoutError("test did not release the first mutation")
        else:
            second_entered.set()
        if tool_name == "write":
            return original_atomic_write(target, payload)
        return original_atomic_write(target, payload, expected=expected)

    module = write_tool if tool_name == "write" else edit_tool
    monkeypatch.setattr(module, "atomic_write_bytes", paused_first_write)
    first_arguments = (
        {"filePath": str(path), "content": "first beta"}
        if tool_name == "write"
        else {"filePath": str(path), "oldString": "alpha", "newString": "first"}
    )
    second_arguments = (
        {"filePath": str(path), "content": "second beta"}
        if tool_name == "write"
        else {"filePath": str(path), "oldString": "beta", "newString": "second"}
    )
    first_task = asyncio.create_task(
        _file_executor().execute(
            ToolCall(f"{tool_name}-task-cancel-first", tool_name, first_arguments),
            cancellation=CancellationToken(),
            context=_context(),
        )
    )
    assert await asyncio.to_thread(entered.wait, 2)

    first_task.cancel()
    second_task = asyncio.create_task(
        _file_executor().execute(
            ToolCall(f"{tool_name}-task-cancel-second", tool_name, second_arguments),
            cancellation=CancellationToken(),
            context=_context(),
        )
    )
    await asyncio.sleep(0.05)
    assert second_entered.is_set() is False

    release.set()
    with pytest.raises(asyncio.CancelledError):
        await first_task
    second_result = await second_task

    assert second_result.ok is True
    assert second_entered.is_set() is True
    expected_text = "second beta" if tool_name == "write" else "first second"
    assert path.read_text(encoding="utf-8") == expected_text
    assert file_common._PATH_LOCKS == {}


@pytest.mark.asyncio
async def test_symlink_aliases_share_a_lock_and_preserve_the_link(tmp_path: Path) -> None:
    target = tmp_path / "target.txt"
    alias = tmp_path / "alias.txt"
    target.write_text("alpha beta gamma", encoding="utf-8")
    try:
        alias.symlink_to(target.name)
    except (NotImplementedError, OSError) as error:
        pytest.skip(f"symlink creation is unavailable: {error}")

    first_result, second_result = await asyncio.gather(
        _file_executor().execute(
            ToolCall(
                "edit-target",
                "edit",
                {"filePath": str(target), "oldString": "alpha", "newString": "ALPHA"},
            ),
            cancellation=CancellationToken(),
            context=_context(),
        ),
        _file_executor().execute(
            ToolCall(
                "edit-alias",
                "edit",
                {"filePath": str(alias), "oldString": "beta", "newString": "BETA"},
            ),
            cancellation=CancellationToken(),
            context=_context(),
        ),
    )

    assert first_result.ok is True
    assert second_result.ok is True
    assert target.read_text(encoding="utf-8") == "ALPHA BETA gamma"
    assert alias.is_symlink()
    assert alias.read_text(encoding="utf-8") == "ALPHA BETA gamma"
    assert file_common._PATH_LOCKS == {}


def test_path_locks_are_reclaimed_and_do_not_cross_event_loops(tmp_path: Path) -> None:
    path = tmp_path / "loop-safe.txt"
    path.write_text("value", encoding="utf-8")

    async def contend_for_lock() -> None:
        holder_entered = asyncio.Event()
        release_holder = asyncio.Event()

        async def holder() -> None:
            async with file_common.path_lock(path):
                holder_entered.set()
                await release_holder.wait()

        async def waiter() -> None:
            await holder_entered.wait()
            async with file_common.path_lock(path):
                pass

        holder_task = asyncio.create_task(holder())
        waiter_task = asyncio.create_task(waiter())
        await holder_entered.wait()
        await asyncio.sleep(0)
        release_holder.set()
        await asyncio.gather(holder_task, waiter_task)

    asyncio.run(contend_for_lock())
    assert file_common._PATH_LOCKS == {}
    asyncio.run(contend_for_lock())
    assert file_common._PATH_LOCKS == {}


@pytest.mark.asyncio
async def test_cancelled_path_lock_waiter_does_not_drop_the_active_holder(
    tmp_path: Path,
) -> None:
    path = tmp_path / "cancelled-waiter.txt"
    holder_entered = asyncio.Event()
    release_holder = asyncio.Event()

    async def holder() -> None:
        async with file_common.path_lock(path):
            holder_entered.set()
            await release_holder.wait()

    async def waiter() -> None:
        async with file_common.path_lock(path):
            pass

    holder_task = asyncio.create_task(holder())
    await holder_entered.wait()
    waiter_task = asyncio.create_task(waiter())
    await asyncio.sleep(0)
    waiter_task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await waiter_task
    assert file_common._PATH_LOCKS

    release_holder.set()
    await holder_task
    assert file_common._PATH_LOCKS == {}


@pytest.mark.skipif(os.name != "posix", reason="POSIX umask permissions")
@pytest.mark.parametrize("test_umask", [0o022, 0o002, 0o077])
def test_atomic_write_uses_process_umask_for_new_files(
    tmp_path: Path,
    test_umask: int,
) -> None:
    path = tmp_path / f"mode-{test_umask:o}.txt"
    previous_umask = os.umask(test_umask)
    try:
        assert file_common.atomic_write_bytes(path, b"content") is True
    finally:
        os.umask(previous_umask)

    assert stat.S_IMODE(path.stat().st_mode) == 0o666 & ~test_umask


@pytest.mark.skipif(os.name != "posix", reason="POSIX file permissions")
def test_atomic_write_preserves_existing_posix_mode(tmp_path: Path) -> None:
    path = tmp_path / "executable.sh"
    path.write_bytes(b"before")
    path.chmod(0o755)

    assert file_common.atomic_write_bytes(path, b"after") is True

    assert stat.S_IMODE(path.stat().st_mode) == 0o755


@pytest.mark.asyncio
async def test_pre_cancelled_file_write_does_not_modify_the_file(tmp_path: Path) -> None:
    path = tmp_path / "cancelled.txt"
    path.write_text("before", encoding="utf-8")
    cancellation = CancellationToken()
    cancellation.cancel()

    with pytest.raises(RunCancelled):
        await _file_executor().execute(
            ToolCall("write-cancelled", "write", {"filePath": str(path), "content": "after"}),
            cancellation=cancellation,
            context=_context(),
        )

    assert path.read_text(encoding="utf-8") == "before"
