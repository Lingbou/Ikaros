from __future__ import annotations

import asyncio
import codecs
import json
import os
import re
import stat
import subprocess
import sys
import threading
from pathlib import Path
from typing import Any, cast

import pytest

from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.errors import RunCancelled
from ikaros_runtime.tools import edit as edit_tool
from ikaros_runtime.tools import file_common
from ikaros_runtime.tools import process as process_tool
from ikaros_runtime.tools import write as write_tool
from ikaros_runtime.tools.core import (
    ToolCall,
    ToolExecutionCancelled,
    ToolExecutor,
    ToolRegistry,
)
from ikaros_runtime.tools.edit import EditTool
from ikaros_runtime.tools.policy import FullAccessPolicy
from ikaros_runtime.tools.process import ProcessRunTool
from ikaros_runtime.tools.read import ReadTool
from ikaros_runtime.tools.write import WriteTool


def _command(*, windows: str, posix: str) -> str:
    return windows if os.name == "nt" else posix


def _executor() -> ToolExecutor:
    return ToolExecutor(ToolRegistry([ProcessRunTool()]), FullAccessPolicy())


def _file_executor() -> ToolExecutor:
    return ToolExecutor(
        ToolRegistry([ReadTool(), WriteTool(), EditTool()]),
        FullAccessPolicy(),
    )


@pytest.mark.asyncio
@pytest.mark.skipif(os.name != "nt", reason="Windows process creation flags")
async def test_process_run_hides_the_root_shell_window(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    if sys.platform != "win32":
        pytest.skip("Windows process creation flags")

    captured: dict[str, Any] = {}
    fake_process = cast(asyncio.subprocess.Process, object())

    async def fake_create_subprocess_exec(*arguments: str, **options: Any) -> Any:
        captured["arguments"] = arguments
        captured["options"] = options
        return fake_process

    monkeypatch.setattr(asyncio, "create_subprocess_exec", fake_create_subprocess_exec)
    monkeypatch.setattr(
        process_tool,
        "_create_windows_job",
        lambda process: cast(Any, object()),
    )

    spawned = await process_tool._spawn_process("Write-Output 'hidden'", None)

    assert spawned.process is fake_process
    flags = captured["options"]["creationflags"]
    assert flags & subprocess.CREATE_NO_WINDOW
    assert flags & subprocess.CREATE_NEW_PROCESS_GROUP
    assert flags & 0x00000004


def _process_is_alive(pid: int) -> bool:
    if sys.platform == "win32":
        completed = subprocess.run(
            ["tasklist.exe", "/FI", f"PID eq {pid}", "/FO", "CSV", "/NH"],
            check=False,
            capture_output=True,
            text=True,
            timeout=5,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        return re.search(rf'"{pid}"', completed.stdout) is not None
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True


async def _wait_for_file(path: Path) -> str:
    for _ in range(100):
        try:
            value = await asyncio.to_thread(path.read_text, encoding="utf-8")
        except FileNotFoundError:
            value = ""
        if value.strip():
            return value
        await asyncio.sleep(0.05)
    raise AssertionError(f"process did not write {path}")


async def _wait_for_process_exit(pid: int) -> None:
    for _ in range(100):
        if not await asyncio.to_thread(_process_is_alive, pid):
            return
        await asyncio.sleep(0.05)
    raise AssertionError(f"process {pid} is still alive")


def _background_child_command(pid_file: Path, *, inherit_output: bool) -> str:
    if os.name == "nt":
        return _windows_sleep_child_command(
            pid_file,
            inherit_output=inherit_output,
            wait_for_exit=False,
        )
    redirect = " >/dev/null 2>&1" if not inherit_output else ""
    return (
        'sh -c \'printf "%s" "$$" > "$1"; sleep 60\' sh '
        f"'{pid_file}'{redirect} & "
        f"while [ ! -s '{pid_file}' ]; do sleep 0.01; done; "
        "printf 'root-exited\\n'"
    )


def _windows_sleep_child_command(
    pid_file: Path,
    *,
    inherit_output: bool,
    wait_for_exit: bool,
) -> str:
    escaped_path = str(pid_file).replace("'", "''")
    if not inherit_output:
        python_path = sys.executable.replace("'", "''")
        helper = str(Path(__file__).parent / "fixtures" / "spawn_sleep_child.py").replace("'", "''")
        wait_mode = "wait" if wait_for_exit else "detach"
        tail = "" if wait_for_exit else "; Write-Output 'root-exited'"
        return f"& '{python_path}' '{helper}' '{escaped_path}' '{wait_mode}'{tail}"
    tail = "$child.WaitForExit()" if wait_for_exit else "Write-Output 'root-exited'"
    return (
        "$psi = [Diagnostics.ProcessStartInfo]::new(); "
        "$psi.FileName = 'powershell.exe'; "
        "$psi.Arguments = '-NoLogo -NoProfile -NonInteractive -Command "
        '"Start-Sleep -Seconds 60"\'; '
        "$psi.UseShellExecute = $false; "
        "$psi.CreateNoWindow = $true; "
        "$child = [Diagnostics.Process]::Start($psi); "
        f"[IO.File]::WriteAllText('{escaped_path}', [string]$child.Id); "
        f"{tail}"
    )


@pytest.mark.asyncio
async def test_process_run_captures_stdout_stderr_exit_and_cwd(tmp_path: Path) -> None:
    command = _command(
        windows="Write-Output 'stdout-value'; [Console]::Error.WriteLine('stderr-value')",
        posix="printf 'stdout-value\\n'; printf 'stderr-value\\n' >&2",
    )
    result = await _executor().execute(
        ToolCall(
            "call-success",
            "process_run",
            {"command": command, "cwd": str(tmp_path), "timeoutMs": 5_000},
        ),
        cancellation=CancellationToken(),
    )

    assert result.ok is True
    assert result.cancelled is False
    assert result.details["exitCode"] == 0
    assert result.details["timedOut"] is False
    assert result.details["truncated"] is False
    resolved_tmp_path = await asyncio.to_thread(tmp_path.resolve)
    assert result.details["cwd"] == str(resolved_tmp_path)
    assert "stdout-value" in result.details["stdout"]
    assert "stderr-value" in result.details["stderr"]
    assert "stdout-value" in result.output
    assert "stderr-value" in result.output


@pytest.mark.asyncio
async def test_process_run_uses_the_run_workspace_as_its_default_cwd(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    result = await _executor().execute(
        ToolCall(
            "call-workspace",
            "process_run",
            {"command": _command(windows="(Get-Location).Path", posix="pwd")},
        ),
        cancellation=CancellationToken(),
        default_cwd=str(workspace),
    )

    assert result.ok is True
    assert result.details["cwd"] == str(workspace.resolve())
    assert str(workspace.resolve()) in result.output.strip()


@pytest.mark.asyncio
async def test_process_run_resolves_relative_cwd_from_the_run_workspace(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    nested = workspace / "nested"
    nested.mkdir(parents=True)
    result = await _executor().execute(
        ToolCall(
            "call-relative-workspace",
            "process_run",
            {
                "command": _command(windows="(Get-Location).Path", posix="pwd"),
                "cwd": "nested",
            },
        ),
        cancellation=CancellationToken(),
        default_cwd=str(workspace),
    )

    assert result.ok is True
    assert result.details["cwd"] == str(nested.resolve())


@pytest.mark.asyncio
async def test_process_run_does_not_fall_back_when_the_workspace_is_missing(
    tmp_path: Path,
) -> None:
    missing_workspace = tmp_path / "removed-workspace"
    result = await _executor().execute(
        ToolCall("call-missing-workspace", "process_run", {"command": "echo unsafe"}),
        cancellation=CancellationToken(),
        default_cwd=str(missing_workspace),
    )

    assert result.ok is False
    assert result.details["errorCode"] == "invalid_cwd"


@pytest.mark.asyncio
async def test_process_run_returns_nonzero_exit_as_a_tool_error() -> None:
    command = _command(
        windows="Write-Output 'before-exit'; exit 7",
        posix="printf 'before-exit\\n'; exit 7",
    )
    result = await _executor().execute(
        ToolCall("call-nonzero", "process_run", {"command": command}),
        cancellation=CancellationToken(),
    )

    assert result.ok is False
    assert result.cancelled is False
    assert result.details["exitCode"] == 7
    assert result.details["errorCode"] == "non_zero_exit"
    assert "before-exit" in result.output


@pytest.mark.asyncio
async def test_process_run_timeout_preserves_partial_output_and_stops_the_tree() -> None:
    command = _command(
        windows="Write-Output 'partial-timeout'; Start-Sleep -Seconds 60",
        posix="printf 'partial-timeout\\n'; sleep 60",
    )
    result = await asyncio.wait_for(
        _executor().execute(
            ToolCall(
                "call-timeout",
                "process_run",
                {"command": command, "timeoutMs": 500},
            ),
            cancellation=CancellationToken(),
        ),
        timeout=10,
    )

    assert result.ok is False
    assert result.cancelled is False
    assert result.details["timedOut"] is True
    assert result.details["errorCode"] == "timeout"
    assert "partial-timeout" in result.output


@pytest.mark.asyncio
async def test_process_run_cancellation_returns_partial_output_and_stops_the_tree() -> None:
    command = _command(
        windows="Write-Output 'partial-cancel'; Start-Sleep -Seconds 60",
        posix="printf 'partial-cancel\\n'; sleep 60",
    )
    cancellation = CancellationToken()
    task = asyncio.create_task(
        _executor().execute(
            ToolCall("call-cancel", "process_run", {"command": command}),
            cancellation=cancellation,
        )
    )
    await asyncio.sleep(0.5)
    cancellation.cancel()

    with pytest.raises(ToolExecutionCancelled) as cancelled:
        await asyncio.wait_for(task, timeout=10)

    assert cancelled.value.result.cancelled is True
    assert cancelled.value.result.details["errorCode"] == "cancelled"
    assert "partial-cancel" in cancelled.value.result.output


@pytest.mark.asyncio
async def test_cancelling_the_executor_task_still_terminates_the_process_tree(
    tmp_path: Path,
) -> None:
    pid_file = tmp_path / "child.pid"
    if os.name == "nt":
        command = _windows_sleep_child_command(
            pid_file,
            inherit_output=False,
            wait_for_exit=True,
        )
    else:
        command = f"sleep 60 & child=$!; printf '%s' \"$child\" > '{pid_file}'; wait $child"
    task = asyncio.create_task(
        _executor().execute(
            ToolCall("call-task-cancel", "process_run", {"command": command}),
            cancellation=CancellationToken(),
        )
    )
    child_pid = int(await _wait_for_file(pid_file))
    assert _process_is_alive(child_pid)

    task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(task, timeout=10)

    await _wait_for_process_exit(child_pid)


@pytest.mark.asyncio
async def test_timeout_covers_pipe_drain_after_the_root_process_exits(
    tmp_path: Path,
) -> None:
    pid_file = tmp_path / "background-timeout.pid"
    result = await asyncio.wait_for(
        _executor().execute(
            ToolCall(
                "call-background-timeout",
                "process_run",
                {
                    "command": _background_child_command(pid_file, inherit_output=True),
                    "timeoutMs": 750,
                },
            ),
            cancellation=CancellationToken(),
        ),
        timeout=10,
    )

    child_pid = int(await _wait_for_file(pid_file))
    assert result.ok is False
    assert result.details["timedOut"] is True
    assert result.details["errorCode"] == "timeout"
    assert "root-exited" in result.output
    await _wait_for_process_exit(child_pid)


@pytest.mark.asyncio
async def test_task_cancel_kills_a_pipe_holder_after_the_root_process_exits(
    tmp_path: Path,
) -> None:
    pid_file = tmp_path / "background-task-cancel.pid"
    task = asyncio.create_task(
        _executor().execute(
            ToolCall(
                "call-background-task-cancel",
                "process_run",
                {"command": _background_child_command(pid_file, inherit_output=True)},
            ),
            cancellation=CancellationToken(),
        )
    )
    child_pid = int(await _wait_for_file(pid_file))
    await asyncio.sleep(0.25)

    task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(task, timeout=10)

    await _wait_for_process_exit(child_pid)


@pytest.mark.asyncio
async def test_successful_root_exit_does_not_leave_a_detached_child(
    tmp_path: Path,
) -> None:
    pid_file = tmp_path / "background-success.pid"
    result = await asyncio.wait_for(
        _executor().execute(
            ToolCall(
                "call-background-success",
                "process_run",
                {
                    "command": _background_child_command(pid_file, inherit_output=False),
                    "timeoutMs": 5_000,
                },
            ),
            cancellation=CancellationToken(),
        ),
        timeout=10,
    )

    child_pid = int(await _wait_for_file(pid_file))
    assert result.ok is True
    assert "root-exited" in result.output
    await _wait_for_process_exit(child_pid)


@pytest.mark.asyncio
@pytest.mark.skipif(os.name != "nt", reason="Windows suspended-spawn regression")
async def test_task_cancel_during_spawn_never_resumes_the_suspended_command(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    marker = tmp_path / "must-not-run.txt"
    escaped_marker = str(marker).replace("'", "''")
    command = f"[IO.File]::WriteAllText('{escaped_marker}', 'ran')"
    original_spawn = process_tool._spawn_process
    supervised = asyncio.Event()
    release = asyncio.Event()

    async def pause_after_supervision(command: str, cwd: str | None) -> Any:
        spawned = await original_spawn(command, cwd)
        supervised.set()
        await release.wait()
        return spawned

    monkeypatch.setattr(process_tool, "_spawn_process", pause_after_supervision)
    task = asyncio.create_task(
        _executor().execute(
            ToolCall("call-spawn-task-cancel", "process_run", {"command": command}),
            cancellation=CancellationToken(),
        )
    )
    await asyncio.wait_for(supervised.wait(), timeout=5)

    task.cancel()
    release.set()
    with pytest.raises(asyncio.CancelledError):
        await asyncio.wait_for(task, timeout=10)

    assert not await asyncio.to_thread(marker.exists)


@pytest.mark.asyncio
async def test_process_run_bounds_output_and_does_not_inherit_arbitrary_secrets(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("IKAROS_TEST_SECRET", "must-not-cross-tool-boundary")
    command = _command(
        windows=(
            "$secret = $env:IKAROS_TEST_SECRET; "
            "if ($secret) { [Console]::Write($secret) }; "
            "[Console]::Write(('x' * 70000))"
        ),
        posix=(
            'if [ -n "$IKAROS_TEST_SECRET" ]; then printf \'%s\' "$IKAROS_TEST_SECRET"; fi; '
            "head -c 70000 /dev/zero | tr '\\0' x"
        ),
    )
    result = await _executor().execute(
        ToolCall("call-output-limit", "process_run", {"command": command}),
        cancellation=CancellationToken(),
    )

    assert result.ok is True
    assert result.details["truncated"] is True
    assert len(result.details["stdout"].encode("utf-8")) <= 64 * 1024
    assert "must-not-cross-tool-boundary" not in result.output


@pytest.mark.asyncio
async def test_invalid_and_unknown_tools_return_structured_results() -> None:
    executor = _executor()
    invalid = await executor.execute(
        ToolCall("call-invalid", "process_run", {"timeoutMs": 5}),
        cancellation=CancellationToken(),
    )
    unknown = await executor.execute(
        ToolCall("call-unknown", "missing_tool", {}),
        cancellation=CancellationToken(),
    )

    assert invalid.ok is False
    assert invalid.details["errorCode"] == "invalid_arguments"
    assert invalid.tool_call_id == "call-invalid"
    assert unknown.ok is False
    assert unknown.details["errorCode"] == "unknown_tool"
    assert unknown.tool_call_id == "call-unknown"


@pytest.mark.asyncio
async def test_read_returns_numbered_utf8_lines_with_offset_and_limit(tmp_path: Path) -> None:
    path = tmp_path / "notes.txt"
    path.write_text("alpha\n中文\ngamma\ndelta\n", encoding="utf-8")

    result = await _file_executor().execute(
        ToolCall("read-lines", "read", {"filePath": str(path), "offset": 2, "limit": 2}),
        cancellation=CancellationToken(),
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
        default_cwd=str(workspace),
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
        default_cwd=str(missing_workspace),
    )

    assert result.ok is False
    assert result.details["errorCode"] == "invalid_workspace"


@pytest.mark.asyncio
async def test_write_creates_parent_directories_and_empty_files(tmp_path: Path) -> None:
    path = tmp_path / "new" / "nested" / "empty.txt"

    result = await _file_executor().execute(
        ToolCall("write-create", "write", {"filePath": str(path), "content": ""}),
        cancellation=CancellationToken(),
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
        )
    )
    assert await asyncio.to_thread(entered.wait, 2)
    cancellation.cancel()
    second_task = asyncio.create_task(
        _file_executor().execute(
            ToolCall(f"{tool_name}-cancel-second", tool_name, second_arguments),
            cancellation=CancellationToken(),
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
        )
    )
    assert await asyncio.to_thread(entered.wait, 2)

    first_task.cancel()
    second_task = asyncio.create_task(
        _file_executor().execute(
            ToolCall(f"{tool_name}-task-cancel-second", tool_name, second_arguments),
            cancellation=CancellationToken(),
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
        ),
        _file_executor().execute(
            ToolCall(
                "edit-alias",
                "edit",
                {"filePath": str(alias), "oldString": "beta", "newString": "BETA"},
            ),
            cancellation=CancellationToken(),
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
        )

    assert path.read_text(encoding="utf-8") == "before"
