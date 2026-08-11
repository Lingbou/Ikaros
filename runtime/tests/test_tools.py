from __future__ import annotations

import asyncio
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Any, cast

import pytest

from ikaros_runtime import process_tool
from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.policy import FullAccessPolicy
from ikaros_runtime.process_tool import ProcessRunTool
from ikaros_runtime.tools import (
    ToolCall,
    ToolExecutionCancelled,
    ToolExecutor,
    ToolRegistry,
)


def _command(*, windows: str, posix: str) -> str:
    return windows if os.name == "nt" else posix


def _executor() -> ToolExecutor:
    return ToolExecutor(ToolRegistry([ProcessRunTool()]), FullAccessPolicy())


@pytest.mark.asyncio
@pytest.mark.skipif(os.name != "nt", reason="Windows process creation flags")
async def test_process_run_hides_the_root_shell_window(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
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
    if os.name == "nt":
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
        f"'{pid_file}'{redirect} & printf 'root-exited\\n'"
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
