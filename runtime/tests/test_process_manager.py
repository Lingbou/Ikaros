from __future__ import annotations

import asyncio
import json
import os
import re
import signal
import subprocess
import sys
import time
from contextlib import suppress
from dataclasses import replace
from pathlib import Path
from types import SimpleNamespace
from typing import Any, cast

import pytest

from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.domain import JsonObject
from ikaros_runtime.errors import RunCancelled
from ikaros_runtime.services.processes import ProcessService
from ikaros_runtime.tools import (
    FullAccessPolicy,
    ProcessManager,
    ProcessReadTool,
    ProcessStartTool,
    ProcessStopTool,
    ProcessWaitTool,
    ToolCall,
    ToolExecutionContext,
    ToolExecutor,
    ToolRegistry,
    process_platform,
)
from ikaros_runtime.tools import process_manager as manager_module
from ikaros_runtime.tools.process_log import ProcessLog, ProcessLogError
from ikaros_runtime.tools.process_manager import ProcessError


def _context(
    *, item: str = "item_start", run: str = "run_test", cwd: str | None = None
) -> ToolExecutionContext:
    return ToolExecutionContext(run, 1, item, "thread_test", cwd)


def _executor(manager: ProcessManager) -> ToolExecutor:
    return ToolExecutor(
        ToolRegistry(
            [
                ProcessStartTool(manager),
                ProcessReadTool(manager),
                ProcessWaitTool(manager),
                ProcessStopTool(manager),
            ]
        ),
        FullAccessPolicy(),
    )


async def _dispose(manager: ProcessManager) -> None:
    """Stop every command, then close (commands outlive Runs since B2)."""
    for entry in list(manager._entries.values()):
        with suppress(Exception):
            await manager.stop(entry.process_id, context=entry.context)
    await manager.close()


def test_restart_process_read_uses_byte_cursors(tmp_path: Path) -> None:
    class RestartStore:
        def process_records(self) -> list[JsonObject]:
            return [
                {
                    "processId": "process_restarted",
                    "threadId": "thread_test",
                    "output": "中文 output",
                    "outputBytes": 13,
                    "headBytes": 13,
                    "tailStart": 0,
                    "fileBytes": 13,
                    "truncated": False,
                }
            ]

    manager = ProcessManager(logs_root=tmp_path / "processes")
    service = ProcessService(manager, cast(Any, RestartStore()), logs_root=tmp_path / "processes")

    page = service.read(
        {"threadId": "thread_test", "processId": "process_restarted", "cursor": 3}
    )

    assert page["output"] == "文 output"
    assert page["nextCursor"] == 13
    assert page["hasMore"] is False


def _command(*, windows: str, posix: str) -> str:
    return windows if os.name == "nt" else posix


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
async def test_start_wait_proves_actual_completion_and_workspace(tmp_path: Path) -> None:
    facts: list[JsonObject] = []
    manager = ProcessManager(logs_root=tmp_path / "processes", record=facts.append)
    executor = _executor(manager)
    context = _context(cwd=str(tmp_path))
    started = await executor.execute(
        ToolCall(
            "call_start",
            "process_start",
            {
                "command": _command(
                    windows="Write-Output '中文'; [Console]::Error.WriteLine('error')",
                    posix="printf '中文\\n'; printf 'error\\n' >&2",
                ),
            },
        ),
        cancellation=CancellationToken(),
        context=context,
    )
    assert started.ok
    assert started.details["state"] == "running"
    process_id = started.details["processId"]
    result = await executor.execute(
        ToolCall(
            "call_wait",
            "process_wait",
            {
                "processId": process_id,
                "timeoutMs": 5000,
            },
        ),
        cancellation=CancellationToken(),
        context=replace(context, item_id="item_wait"),
    )
    assert result.ok
    assert result.details["state"] == "exited"
    assert result.details["exitCode"] == 0
    assert result.details["cwd"] == str(tmp_path)
    assert "中文" in result.output and "error" in result.output
    assert [facts[0]["state"], facts[1]["state"], facts[-1]["state"]] == [
        "unknown",
        "running",
        "exited",
    ]
    assert facts[0]["errorCode"] == "start_pending"
    assert "中文" in facts[-1]["output"] and "error" in facts[-1]["output"]
    canonical_timestamp = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$")
    for fact in facts:
        assert canonical_timestamp.fullmatch(str(fact["startedAt"]))
        if fact["finishedAt"] is not None:
            assert canonical_timestamp.fullmatch(str(fact["finishedAt"]))
    await manager.close()


@pytest.mark.asyncio
async def test_repeated_start_and_read_do_not_replay_command(tmp_path: Path) -> None:
    manager = ProcessManager(logs_root=tmp_path / "processes")
    context = _context()
    command = _command(windows="Write-Output 'once'", posix="printf once")
    first, second = await asyncio.gather(
        *[
            manager.start(command, str(tmp_path), context=context, cancellation=CancellationToken())
            for _ in range(2)
        ]
    )
    assert first["processId"] == second["processId"]
    final = await manager.wait(
        first["processId"], context=context, cancellation=CancellationToken(), timeout_ms=5000
    )
    assert final["output"].strip() == "once"
    assert manager.read(first["processId"], context=context) == final
    assert (
        await manager.start(
            command, str(tmp_path), context=context, cancellation=CancellationToken()
        )
    ) == final
    with pytest.raises(ProcessError, match="different command"):
        await manager.start(
            "echo different", str(tmp_path), context=context, cancellation=CancellationToken()
        )
    with pytest.raises(ProcessError, match="different command"):
        await manager.start(
            command, str(tmp_path.parent), context=context, cancellation=CancellationToken()
        )
    with pytest.raises(ProcessError, match="different command"):
        await manager.start(
            command,
            str(tmp_path),
            context=context,
            cancellation=CancellationToken(),
            background=True,
        )
    # Commands are thread-owned: another Run of the same thread can still read them.
    assert manager.read(first["processId"], context=_context(run="run_other"))["state"] == "exited"
    await manager.close()


@pytest.mark.asyncio
async def test_wait_timeout_leaves_process_running_and_stop_is_terminal(tmp_path: Path) -> None:
    manager = ProcessManager(logs_root=tmp_path / "processes")
    context = _context()
    result = await manager.start(
        _command(
            windows="Write-Output 'partial'; Start-Sleep 60", posix="printf 'partial\\n'; sleep 60"
        ),
        str(tmp_path),
        context=context,
        cancellation=CancellationToken(),
    )
    process_id = result["processId"]
    waited = await manager.wait(
        process_id, context=context, cancellation=CancellationToken(), timeout_ms=100
    )
    assert waited["state"] == "running" and waited["exitCode"] is None
    stopped = await asyncio.wait_for(manager.stop(process_id, context=context), timeout=10)
    assert stopped["state"] == "terminated"
    assert (await manager.stop(process_id, context=context)) == stopped
    await manager.close()


@pytest.mark.asyncio
async def test_wait_cancellation_does_not_spawn_or_implicitly_stop(tmp_path: Path) -> None:
    manager = ProcessManager(logs_root=tmp_path / "processes")
    context = _context()
    result = await manager.start(
        _command(windows="Start-Sleep 60", posix="sleep 60"),
        str(tmp_path),
        context=context,
        cancellation=CancellationToken(),
    )
    cancellation = CancellationToken()
    task = asyncio.create_task(
        manager.wait(
            result["processId"], context=context, cancellation=cancellation, timeout_ms=60000
        )
    )
    await asyncio.sleep(0)
    cancellation.cancel()
    with pytest.raises(RunCancelled):
        await task
    assert manager.read(result["processId"], context=context)["state"] == "running"
    await manager.close_run(context.run_id)
    # Commands are thread-owned: closing the Run keeps them readable and stoppable.
    assert manager.read(result["processId"], context=context)["state"] == "running"
    with pytest.raises(ProcessError, match="ended"):
        await manager.start(
            "echo forbidden",
            str(tmp_path),
            context=_context(item="item_late"),
            cancellation=CancellationToken(),
        )
    await _dispose(manager)


@pytest.mark.asyncio
@pytest.mark.parametrize("inherit_output", [True, False])
async def test_root_exit_stops_descendants_without_pipe_hang(
    tmp_path: Path,
    inherit_output: bool,
) -> None:
    manager = ProcessManager(logs_root=tmp_path / "processes")
    context = _context()
    pid_file = tmp_path / "child.pid"
    result = await manager.start(
        _background_child_command(pid_file, inherit_output=inherit_output),
        str(tmp_path),
        context=context,
        cancellation=CancellationToken(),
    )
    child_pid = int(await _wait_for_file(pid_file))
    final = await manager.wait(
        result["processId"], context=context, cancellation=CancellationToken(), timeout_ms=10000
    )
    assert final["state"] == "exited"
    assert "root-exited" in final["output"]
    await _wait_for_process_exit(child_pid)
    await manager.close()


@pytest.mark.asyncio
async def test_nonzero_exit_and_invalid_tool_arguments(tmp_path: Path) -> None:
    manager = ProcessManager(logs_root=tmp_path / "processes")
    executor = _executor(manager)
    context = _context(cwd=str(tmp_path))
    started = await executor.execute(
        ToolCall("start", "process_start", {"command": "exit 7"}),
        cancellation=CancellationToken(),
        context=context,
    )
    final = await executor.execute(
        ToolCall(
            "wait",
            "process_wait",
            {
                "processId": started.details["processId"],
                "timeoutMs": 5000,
            },
        ),
        cancellation=CancellationToken(),
        context=context,
    )
    assert not final.ok and final.details["exitCode"] == 7
    cases: list[tuple[str, JsonObject]] = [
        ("process_start", {"command": "echo bad", "runId": "spoof"}),
        ("process_start", {"command": "echo bad", "background": "yes"}),
        ("process_read", {"processId": "missing", "cursor": True}),
        ("process_wait", {"processId": "missing", "timeoutMs": 0}),
        ("process_stop", {"processId": "missing"}),
    ]
    for name, arguments in cases:
        rejected = await executor.execute(
            ToolCall("invalid", name, arguments), cancellation=CancellationToken(), context=context
        )
        assert not rejected.ok
    old = await executor.execute(
        ToolCall("old", "process_run", {"command": "echo old"}),
        cancellation=CancellationToken(),
        context=context,
    )
    assert old.details["errorCode"] == "unknown_tool"
    await manager.close()


@pytest.mark.asyncio
async def test_relative_and_missing_workspace(tmp_path: Path) -> None:
    manager = ProcessManager(logs_root=tmp_path / "processes")
    executor = _executor(manager)
    child = tmp_path / "nested"
    child.mkdir()
    context = _context(cwd=str(tmp_path))
    started = await executor.execute(
        ToolCall(
            "start",
            "process_start",
            {
                "command": "echo cwd",
                "cwd": "nested",
            },
        ),
        cancellation=CancellationToken(),
        context=context,
    )
    assert started.details["cwd"] == str(child)
    invalid = await executor.execute(
        ToolCall("invalid", "process_start", {"command": "echo bad"}),
        cancellation=CancellationToken(),
        context=_context(cwd=str(tmp_path / "missing")),
    )
    assert not invalid.ok and invalid.details["errorCode"] == "invalid_cwd"
    await manager.close()


@pytest.mark.asyncio
async def test_output_is_bounded_paged_and_secret_environment_not_inherited(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("IKAROS_TEST_SECRET", "do-not-inherit-me")
    manager = ProcessManager(logs_root=tmp_path / "processes")
    context = _context()
    command = _command(
        windows="[Console]::Write($env:IKAROS_TEST_SECRET); [Console]::Write(('x' * 2000000))",
        posix='printf "%s" "$IKAROS_TEST_SECRET"; head -c 2000000 /dev/zero | tr "\\0" x',
    )
    started = await manager.start(
        command, str(tmp_path), context=context, cancellation=CancellationToken()
    )
    first = await manager.wait(
        started["processId"], context=context, cancellation=CancellationToken(), timeout_ms=5000
    )
    assert first["state"] == "exited" and first["truncated"]
    pages = [first["output"]]
    page = first
    while page["hasMore"]:
        page = manager.read(started["processId"], context=context, cursor=page["nextCursor"])
        pages.append(page["output"])
    assert all(len(page.encode()) <= 16 * 1024 for page in pages)
    rendered = "".join(pages)
    assert rendered.startswith("x" * (512 * 1024))
    assert rendered.endswith("x" * (512 * 1024))
    assert f"{2_000_000 - 1024 * 1024} bytes omitted" in rendered
    await manager.close()


@pytest.mark.asyncio
async def test_secret_split_across_stream_chunks_never_reaches_read_or_record(
    tmp_path: Path,
) -> None:
    facts: list[JsonObject] = []
    secret = "credential-very-secret"
    manager = ProcessManager(
        logs_root=tmp_path / "processes",
        record=facts.append,
        protected_values=lambda: (secret,),
    )
    context = _context()
    command = _command(
        windows="[Console]::Write('credential-'); Start-Sleep -Milliseconds 300; "
        "[Console]::Write('very-secret'); Write-Output 'after'",
        posix="printf 'credential-'; sleep 0.3; printf 'very-secret'; printf after",
    )
    started = await manager.start(
        command, str(tmp_path), context=context, cancellation=CancellationToken()
    )
    first = await manager.wait(
        started["processId"], context=context, cancellation=CancellationToken(), timeout_ms=100
    )
    assert "credential-" not in first["output"]
    final = await manager.wait(
        started["processId"], context=context, cancellation=CancellationToken(), timeout_ms=15000
    )
    assert final["errorCode"] == "protected_output"
    assert secret not in json.dumps(facts)
    assert "credential-" not in final["output"]
    await manager.close()


@pytest.mark.asyncio
async def test_commands_outlive_the_run_and_serve_later_runs(tmp_path: Path) -> None:
    manager = ProcessManager(logs_root=tmp_path / "processes")
    first = _context(item="item_first", run="run_one")
    started = await manager.start(
        _command(windows="Start-Sleep 60", posix="sleep 60"),
        str(tmp_path),
        context=first,
        cancellation=CancellationToken(),
        background=True,
    )
    process_id = started["processId"]
    assert started["background"] is True
    await manager.close_run("run_one")
    later = _context(item="item_later", run="run_two")
    later_read = manager.read(process_id, context=later)
    assert later_read["state"] == "running" and later_read["background"] is True
    stopped = await manager.stop(process_id, context=later)
    assert stopped["state"] == "terminated" and stopped["background"] is True
    other_thread = ToolExecutionContext("run_two", 1, "item_later", "thread_other", None)
    with pytest.raises(ProcessError, match="belongs to this thread"):
        manager.read(process_id, context=other_thread)
    await manager.close()


@pytest.mark.asyncio
async def test_restore_readopts_a_live_command_and_stop_settles_it(tmp_path: Path) -> None:
    facts: list[JsonObject] = []
    manager = ProcessManager(logs_root=tmp_path / "processes", record=facts.append)
    context = _context()
    started = await manager.start(
        _command(windows="Start-Sleep 60", posix="sleep 60"),
        str(tmp_path),
        context=context,
        cancellation=CancellationToken(),
        background=True,
    )
    assert started["background"] is True
    running = dict(facts[-1])
    await manager.close()  # a Runtime exit keeps the command alive
    recovered: list[JsonObject] = []
    restored = ProcessManager(logs_root=tmp_path / "processes", record=recovered.append)
    restored.restore([running])
    assert recovered[-1]["state"] == "running"
    assert recovered[-1]["errorCode"] == "runtime_recovered"
    assert recovered[-1]["background"] is True
    read = restored.read(started["processId"], context=context)
    assert read["state"] == "running" and read["errorCode"] == "runtime_recovered"
    assert read["background"] is True
    with pytest.raises(ProcessError, match="live input is unavailable"):
        await restored.write(started["processId"], context=context, data="x\n")
    stopped = await restored.stop(started["processId"], context=context)
    assert stopped["state"] == "terminated"
    assert stopped["exitCode"] is None
    assert stopped["background"] is True
    await _wait_for_process_exit(int(started["pid"]))
    assert not _process_is_alive(int(started["pid"]))
    await restored.close()


@pytest.mark.asyncio
async def test_restore_settles_missing_or_reused_commands_to_unknown(tmp_path: Path) -> None:
    facts: list[JsonObject] = []
    manager = ProcessManager(logs_root=tmp_path / "processes", record=facts.append)
    context = _context()
    started = await manager.start(
        _command(windows="Start-Sleep 60", posix="sleep 60"),
        str(tmp_path),
        context=context,
        cancellation=CancellationToken(),
        background=True,
    )
    assert started["background"] is True
    running = dict(facts[-1])
    await manager.stop(started["processId"], context=context)
    await _wait_for_process_exit(int(running["pid"]))
    await manager.close()
    recovered: list[JsonObject] = []
    restored = ProcessManager(logs_root=tmp_path / "processes", record=recovered.append)
    restored.restore([running])  # the command already exited
    assert recovered[-1]["state"] == "unknown"
    assert recovered[-1]["exitCode"] is None
    assert recovered[-1]["errorCode"] == "runtime_interrupted"
    assert recovered[-1]["background"] is True
    # A live PID whose recorded start time does not match must never be adopted.
    mismatched = dict(running, pid=os.getpid(), startedAt="2020-01-01T00:00:00.000Z")
    reused: list[JsonObject] = []
    other = ProcessManager(logs_root=tmp_path / "processes", record=reused.append)
    other.restore([mismatched])
    assert reused[-1]["state"] == "unknown"
    assert reused[-1]["errorCode"] == "runtime_interrupted"
    await restored.close()
    await other.close()


@pytest.mark.asyncio
async def test_restore_keeps_the_recovered_window_inside_the_log_size(
    tmp_path: Path,
) -> None:
    facts: list[JsonObject] = []
    manager = ProcessManager(logs_root=tmp_path / "processes", record=facts.append)
    context = _context()
    started = await manager.start(
        _command(windows="Start-Sleep 30", posix="sleep 30"),
        str(tmp_path),
        context=context,
        cancellation=CancellationToken(),
        background=True,
    )
    running = dict(facts[-1])
    await manager.stop(started["processId"], context=context)
    await _wait_for_process_exit(int(running["pid"]))
    await manager.close()

    # Windows shells commonly emit UTF-16. Decoding replaces every invalid byte
    # with U+FFFD (three UTF-8 bytes), so the retained window can be larger than
    # the raw log file. Recovery must still publish a self-consistent record.
    log_path = next((tmp_path / "processes").rglob(f"{started['processId']}.log"))
    with log_path.open("ab") as handle:
        handle.write(b"\xff\xfe" * 64)

    recovered: list[JsonObject] = []
    restored = ProcessManager(logs_root=tmp_path / "processes", record=recovered.append)
    restored.restore([running])
    assert recovered[-1]["fileBytes"] >= recovered[-1]["headBytes"]
    assert recovered[-1]["fileBytes"] >= recovered[-1]["tailStart"]
    await restored.close()


@pytest.mark.asyncio
async def test_active_command_quota(tmp_path: Path) -> None:
    manager = ProcessManager(logs_root=tmp_path / "processes")
    for index in range(64):
        await manager.start(
            _command(windows="Start-Sleep 60", posix="sleep 60"),
            str(tmp_path),
            context=_context(item=f"item_{index}"),
            cancellation=CancellationToken(),
        )
    with pytest.raises(ProcessError, match="at most 64"):
        await manager.start(
            "echo sixty-fifth",
            str(tmp_path),
            context=_context(item="item_sixty_fifth"),
            cancellation=CancellationToken(),
        )
    await _dispose(manager)


@pytest.mark.asyncio
async def test_record_failure_after_spawn_cleans_tree(tmp_path: Path) -> None:
    seen: list[JsonObject] = []

    def record(fact: JsonObject) -> None:
        seen.append(fact)
        if fact["state"] == "running":
            raise RuntimeError("disk write failed")

    manager = ProcessManager(logs_root=tmp_path / "processes", record=record)
    with pytest.raises(RuntimeError, match="disk write failed"):
        await manager.start(
            _command(windows="Start-Sleep 60", posix="sleep 60"),
            str(tmp_path),
            context=_context(),
            cancellation=CancellationToken(),
        )
    pid = seen[1]["pid"]
    await _wait_for_process_exit(pid)
    assert seen[-1]["state"] == "unknown"
    await manager.close()


@pytest.mark.asyncio
async def test_task_cancellation_during_spawn_cleans_command(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    spawned = asyncio.Event()
    release = asyncio.Event()
    original = process_platform._spawn_process
    pids: list[int] = []

    async def delayed(
        command: str, cwd: str | None, *, log: object = None
    ) -> process_platform._SpawnedProcess:
        result = await original(command, cwd, log=cast(Any, log))
        pids.append(result.process.pid)
        spawned.set()
        await release.wait()
        return result

    monkeypatch.setattr(manager_module, "_spawn_process", delayed)
    manager = ProcessManager(logs_root=tmp_path / "processes")
    task = asyncio.create_task(
        manager.start(
            _command(windows="Start-Sleep 60", posix="sleep 60"),
            str(tmp_path),
            context=_context(),
            cancellation=CancellationToken(),
        )
    )
    await asyncio.wait_for(spawned.wait(), timeout=5)
    task.cancel()
    release.set()
    with pytest.raises(asyncio.CancelledError):
        await task
    await _wait_for_process_exit(pids[0])
    await manager.close()


@pytest.mark.asyncio
@pytest.mark.skipif(sys.platform != "win32", reason="Windows process creation flags")
async def test_windows_shell_hidden_suspended_until_job_assignment(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    if sys.platform != "win32":
        pytest.skip("Windows process creation flags")
    captured: dict[str, Any] = {}
    fake_process = cast(asyncio.subprocess.Process, SimpleNamespace(pid=4242))

    async def fake_spawn(*args: str, **kwargs: Any) -> asyncio.subprocess.Process:
        captured.update(kwargs)
        return fake_process

    monkeypatch.setattr(asyncio, "create_subprocess_exec", fake_spawn)
    monkeypatch.setattr(
        process_platform, "_create_windows_job", lambda pid: cast(Any, object())
    )
    result = await process_platform._spawn_process("echo hidden", None)
    assert result.process is fake_process
    assert captured["creationflags"] & subprocess.CREATE_NO_WINDOW
    assert captured["creationflags"] & subprocess.CREATE_NEW_PROCESS_GROUP
    assert captured["creationflags"] & 0x00000004


@pytest.mark.asyncio
async def test_split_secret_across_stdout_stderr_is_hidden_in_combined_log(tmp_path: Path) -> None:
    secret = "joined-secret"
    manager = ProcessManager(logs_root=tmp_path / "processes", protected_values=lambda: (secret,))
    command = _command(
        windows="[Console]::Write('joined-'); Start-Sleep -Milliseconds 200; "
        "[Console]::Error.Write('secret')",
        posix="printf joined-; sleep 0.2; printf secret >&2",
    )
    context = _context()
    start = await manager.start(
        command, str(tmp_path), context=context, cancellation=CancellationToken()
    )
    final = await manager.wait(
        start["processId"], context=context, cancellation=CancellationToken(), timeout_ms=5000
    )
    assert secret not in final["output"]
    await manager.close()


@pytest.mark.asyncio
async def test_terminal_facts_keep_flushed_output_and_bounded_number_of_records(
    tmp_path: Path,
) -> None:
    facts: list[JsonObject] = []
    manager = ProcessManager(logs_root=tmp_path / "processes", record=facts.append)
    context = _context()
    command = _command(
        windows="[Console]::Write(('z' * 2000000))",
        posix='head -c 2000000 /dev/zero | tr "\\0" z',
    )
    started = await manager.start(
        command, str(tmp_path), context=context, cancellation=CancellationToken()
    )
    final = await manager.wait(
        started["processId"], context=context, cancellation=CancellationToken(), timeout_ms=15000
    )
    assert final["state"] == "exited"
    expected = (
        "z" * (512 * 1024)
        + f"\n... {2_000_000 - 1024 * 1024} bytes omitted ...\n"
        + "z" * (512 * 1024)
    )
    assert facts[-1]["output"] == expected
    assert facts[-1]["truncated"] is True
    assert facts[-1]["outputBytes"] == 2_000_000
    assert facts[-1]["headBytes"] == 512 * 1024
    assert len(facts) <= 12
    assert len(facts[-1]["output"].encode()) <= 1024 * 1024 + 128
    await manager.close()


@pytest.mark.asyncio
async def test_run_closure_keeps_settled_results_readable(tmp_path: Path) -> None:
    facts: list[JsonObject] = []
    manager = ProcessManager(logs_root=tmp_path / "processes", record=facts.append)
    context = _context()
    start = await manager.start(
        "echo saved", str(tmp_path), context=context, cancellation=CancellationToken()
    )
    before = await manager.wait(
        start["processId"], context=context, cancellation=CancellationToken(), timeout_ms=5000
    )
    await manager.close_run(context.run_id)
    # Commands outlive their Run: settled output stays readable after closure.
    after = manager.read(start["processId"], context=context)
    assert after["output"] == before["output"]
    assert facts[-1]["output"] == before["output"]
    assert facts[-1]["state"] == "exited"
    await manager.close()


@pytest.mark.asyncio
async def test_record_failure_before_spawn_never_executes_command(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    spawned = False

    async def unexpected(command: str, cwd: str | None) -> process_platform._SpawnedProcess:
        nonlocal spawned
        spawned = True
        raise AssertionError("must not spawn without a persisted intent")

    def reject(fact: JsonObject) -> None:
        raise RuntimeError("storage unavailable")

    monkeypatch.setattr(manager_module, "_spawn_process", unexpected)
    manager = ProcessManager(logs_root=tmp_path / "processes", record=reject)
    with pytest.raises(RuntimeError, match="storage unavailable"):
        await manager.start(
            "echo no", str(tmp_path), context=_context(), cancellation=CancellationToken()
        )
    assert not spawned
    await manager.close()


@pytest.mark.asyncio
async def test_cancelled_wait_tool_retains_partial_output_for_turn_history(tmp_path: Path) -> None:
    from ikaros_runtime.tools import ToolExecutionCancelled

    facts: list[JsonObject] = []
    manager = ProcessManager(logs_root=tmp_path / "processes", record=facts.append)
    executor = _executor(manager)
    context = _context()
    start = await manager.start(
        _command(
            windows="Write-Output 'partial'; Start-Sleep 60", posix="printf 'partial\\n'; sleep 60"
        ),
        str(tmp_path),
        context=context,
        cancellation=CancellationToken(),
    )
    for _ in range(100):
        if "partial" in manager.read(start["processId"], context=context)["output"]:
            break
        await asyncio.sleep(0.05)
    else:
        raise AssertionError("command did not print its partial output")
    cancellation = CancellationToken()
    wait_task = asyncio.create_task(
        executor.execute(
            ToolCall(
                "wait",
                "process_wait",
                {
                    "processId": start["processId"],
                    "timeoutMs": 60000,
                },
            ),
            cancellation=cancellation,
            context=replace(context, item_id="item_wait"),
        )
    )
    await asyncio.sleep(0)
    cancellation.cancel()
    with pytest.raises(ToolExecutionCancelled) as interrupted:
        await wait_task
    assert interrupted.value.result.cancelled
    assert "partial" in interrupted.value.result.output
    assert interrupted.value.result.details["processId"] == start["processId"]
    await manager.close_run(context.run_id)
    assert manager.read(start["processId"], context=context)["state"] == "running"
    await _dispose(manager)
    assert facts[-1]["state"] == "terminated"
    assert "partial" in facts[-1]["output"]


@pytest.mark.asyncio
async def test_more_than_thirty_two_sequential_commands_keep_old_results_and_start_identity(
    tmp_path: Path,
) -> None:
    manager = ProcessManager(logs_root=tmp_path / "processes")
    cancellation = CancellationToken()
    command = _command(
        windows="Add-Content -LiteralPath commands.txt -Value 'done'; Write-Output 'done'",
        posix="printf 'done\\n' >> commands.txt; printf 'done\\n'",
    )
    first_context = _context(item="command-0")
    first_result: JsonObject = {}
    try:
        for index in range(35):
            context = _context(item=f"command-{index}")
            started = await manager.start(
                command, str(tmp_path), context=context, cancellation=cancellation
            )
            result = await manager.wait(
                started["processId"], context=context, cancellation=cancellation, timeout_ms=5000
            )
            assert result["state"] == "exited" and result["exitCode"] == 0
            if index == 0:
                first_result = result

        process_id = first_result["processId"]
        assert manager.read(process_id, context=first_context) == first_result
        assert await manager.wait(
            process_id, context=first_context, cancellation=cancellation
        ) == first_result
        assert await manager.stop(process_id, context=first_context) == first_result
        assert await manager.start(
            command, str(tmp_path), context=first_context, cancellation=cancellation
        ) == first_result
        assert (tmp_path / "commands.txt").read_text(encoding="utf-8").splitlines() == [
            "done"
        ] * 35
    finally:
        await manager.close()


@pytest.mark.asyncio
async def test_sixty_four_active_commands_block_the_sixty_fifth_until_one_finishes(
    tmp_path: Path,
) -> None:
    manager = ProcessManager(logs_root=tmp_path / "processes")
    cancellation = CancellationToken()
    command = _command(windows="Start-Sleep -Seconds 60", posix="sleep 60")
    running: list[JsonObject] = []
    try:
        for index in range(64):
            running.append(
                await manager.start(
                    command,
                    str(tmp_path),
                    context=_context(item=f"active-{index}"),
                    cancellation=cancellation,
                )
            )
        sixty_fifth_context = _context(item="active-64")
        with pytest.raises(ProcessError) as rejected:
            await manager.start(
                "echo released",
                str(tmp_path),
                context=sixty_fifth_context,
                cancellation=cancellation,
            )
        assert rejected.value.code == "process_limit"
        assert all(
            manager.read(result["processId"], context=_context())["state"] == "running"
            for result in running
        )
        stopped = await manager.stop(running[0]["processId"], context=_context())
        assert stopped["state"] == "terminated"
        sixty_fifth = await manager.start(
            "echo released",
            str(tmp_path),
            context=sixty_fifth_context,
            cancellation=cancellation,
        )
        result = await manager.wait(
            sixty_fifth["processId"],
            context=sixty_fifth_context,
            cancellation=cancellation,
            timeout_ms=5000,
        )
        assert result["state"] == "exited" and result["exitCode"] == 0
        assert "released" in result["output"]
    finally:
        await _dispose(manager)

def _slow_writer_command() -> str:
    return _command(
        windows=(
            "for ($i = 0; $i -lt 120; $i++) { [Console]::Out.WriteLine('tick' + $i); "
            "[Console]::Out.Flush(); Start-Sleep -Milliseconds 50 }"
        ),
        posix=(
            "i=0; while [ $i -lt 120 ]; do printf 'tick%s\\n' \"$i\"; "
            "i=$((i+1)); sleep 0.05; done"
        ),
    )


async def _read_until(
    manager: ProcessManager,
    process_id: str,
    context: ToolExecutionContext,
    predicate: Any,
    *,
    cursor: int = 0,
    budget_seconds: float = 15.0,
) -> tuple[str, int]:
    deadline = time.monotonic() + budget_seconds
    text = ""
    page_cursor = cursor
    while time.monotonic() < deadline:
        page = manager.read(process_id, context=context, cursor=page_cursor)
        text += str(page["output"])
        page_cursor = int(page["nextCursor"])
        if predicate(text):
            return text, page_cursor
        await asyncio.sleep(0.03)
    raise AssertionError(f"command did not produce expected output: {text[-200:]!r}")


def _kill_process(pid: int) -> None:
    if sys.platform == "win32":
        subprocess.run(
            ["taskkill.exe", "/PID", str(pid), "/T", "/F"],
            check=False,
            capture_output=True,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        return
    killpg = getattr(os, "killpg", None)
    sigkill = getattr(signal, "SIGKILL", None)
    with suppress(ProcessLookupError):
        if killpg is not None and sigkill is not None:
            killpg(pid, sigkill)
        else:
            os.kill(pid, signal.SIGTERM)


@pytest.mark.asyncio
async def test_output_is_file_backed_and_restart_reads_downtime_output(tmp_path: Path) -> None:
    logs_root = tmp_path / "processes"
    context = _context(item="item_durable_output", run="run_durable_output")
    manager = ProcessManager(logs_root=logs_root)
    started = await manager.start(
        _slow_writer_command(),
        str(tmp_path),
        context=context,
        cancellation=CancellationToken(),
        background=True,
    )
    process_id = started["processId"]
    log_path = logs_root / "thread_test" / f"{process_id}.log"
    metadata_path = logs_root / "thread_test" / f"{process_id}.json"
    assert log_path.is_file() and metadata_path.is_file()
    before, cursor = await _read_until(
        manager, process_id, context, lambda value: "tick3" in value
    )
    last_tick = int(re.findall(r"tick(\d+)", before)[-1])
    record = manager._entries[process_id].fact()
    await manager.close()  # Runtime exit; the command keeps writing to the log file
    await asyncio.sleep(0.4)
    restored = ProcessManager(logs_root=logs_root)
    restored.restore([record])
    assert restored.read(process_id, context=context)["state"] == "running"
    downtime, _ = await _read_until(
        restored,
        process_id,
        context,
        lambda value: bool(re.search(rf"tick({last_tick + 1})", value)),
        cursor=cursor,
    )
    combined = before + downtime
    assert combined.count("tick0") == 1
    assert re.findall(r"tick(\d+)", downtime)
    stopped = await restored.stop(process_id, context=context)
    assert stopped["state"] == "terminated"
    await restored.close()


@pytest.mark.asyncio
async def test_supervise_restarts_with_backoff_until_the_storm_budget(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(manager_module, "_SUPERVISE_MAX_RESTARTS", 2)
    monkeypatch.setattr(manager_module, "_SUPERVISE_BACKOFF_MAX_MS", 100)
    manager = ProcessManager(logs_root=tmp_path / "processes")
    context = _context(item="item_supervise_storm")
    started = await manager.start(
        _command(windows="Write-Output 'boom'; exit 3", posix="printf 'boom\\n'; exit 3"),
        str(tmp_path),
        context=context,
        cancellation=CancellationToken(),
        background=True,
        supervise="on-failure",
        backoff_ms=100,
    )
    final = await manager.wait(
        started["processId"], context=context, cancellation=CancellationToken(), timeout_ms=20000
    )
    assert final["state"] == "exited"
    assert final["exitCode"] == 3
    assert final["errorCode"] == "supervise_storm"
    assert final["restartCount"] == 2
    assert isinstance(final["lastRestartAt"], str) and final["lastRestartAt"]
    assert final["output"].count("boom") == 3
    await manager.close()


@pytest.mark.asyncio
async def test_supervise_on_failure_keeps_a_clean_exit_settled(tmp_path: Path) -> None:
    manager = ProcessManager(logs_root=tmp_path / "processes")
    context = _context(item="item_supervise_clean")
    started = await manager.start(
        _command(windows="Write-Output 'clean'; exit 0", posix="printf 'clean\\n'"),
        str(tmp_path),
        context=context,
        cancellation=CancellationToken(),
        background=True,
        supervise="on-failure",
        backoff_ms=100,
    )
    final = await manager.wait(
        started["processId"], context=context, cancellation=CancellationToken(), timeout_ms=8000
    )
    assert final["state"] == "exited" and final["exitCode"] == 0
    assert final["errorCode"] is None
    assert final["restartCount"] == 0
    assert final["supervise"] == "on-failure"
    await manager.close()


@pytest.mark.asyncio
async def test_stop_ends_supervision_and_keeps_it_stopped(tmp_path: Path) -> None:
    manager = ProcessManager(logs_root=tmp_path / "processes")
    context = _context(item="item_supervise_stop")
    started = await manager.start(
        _command(windows="Start-Sleep 60", posix="sleep 60"),
        str(tmp_path),
        context=context,
        cancellation=CancellationToken(),
        background=True,
        supervise="always",
        backoff_ms=100,
    )
    await asyncio.sleep(0.4)
    stopped = await manager.stop(started["processId"], context=context)
    assert stopped["state"] == "terminated"
    await asyncio.sleep(0.5)  # longer than one backoff window
    again = manager.read(started["processId"], context=context)
    assert again["state"] == "terminated"
    assert again["restartCount"] == 0
    await manager.close()


@pytest.mark.asyncio
async def test_restore_restarts_a_supervised_command_that_died_while_down(
    tmp_path: Path,
) -> None:
    logs_root = tmp_path / "processes"
    context = _context(item="item_supervise_crash", run="run_supervise_crash")
    manager = ProcessManager(logs_root=logs_root)
    started = await manager.start(
        _command(windows="Start-Sleep 60", posix="sleep 60"),
        str(tmp_path),
        context=context,
        cancellation=CancellationToken(),
        background=True,
        supervise="always",
        backoff_ms=100,
    )
    pid = int(started["pid"])
    record = manager._entries[started["processId"]].fact()
    await manager.close()
    await asyncio.to_thread(_kill_process, pid)
    await _wait_for_process_exit(pid)
    restored = ProcessManager(logs_root=logs_root)
    restored.restore([record])
    entry = restored._entries[started["processId"]]
    for _ in range(200):
        if (
            entry.restart_count >= 1
            and entry.state == "running"
            and entry.spawned is not None
            and entry.pid != pid
        ):
            break
        await asyncio.sleep(0.05)
    else:
        raise AssertionError("supervised command was not restarted after the Runtime restart")
    assert entry.error_code is None
    assert entry.last_restart_at
    stopped = await restored.stop(started["processId"], context=context)
    assert stopped["state"] == "terminated"
    await restored.close()


@pytest.mark.asyncio
async def test_process_log_file_is_bounded_after_large_output(tmp_path: Path) -> None:
    logs_root = tmp_path / "processes"
    manager = ProcessManager(logs_root=logs_root)
    context = _context(item="item_bounded_log")
    started = await manager.start(
        _command(
            windows="[Console]::Write(('y' * 3000000))",
            posix='head -c 3000000 /dev/zero | tr "\\0" y',
        ),
        str(tmp_path),
        context=context,
        cancellation=CancellationToken(),
    )
    process_id = started["processId"]
    final = await manager.wait(
        process_id, context=context, cancellation=CancellationToken(), timeout_ms=20000
    )
    assert final["state"] == "exited"
    assert final["truncated"] is True
    log_path = logs_root / "thread_test" / f"{process_id}.log"
    assert log_path.stat().st_size <= manager_module._PROCESS_OUTPUT_LIMIT
    first = manager.read(process_id, context=context, cursor=0)
    second = manager.read(process_id, context=context, cursor=0)
    assert first == second
    await manager.close()


def test_process_log_paths_stay_inside_the_process_root(tmp_path: Path) -> None:
    root = tmp_path / "processes"
    with pytest.raises(ProcessLogError):
        ProcessLog.open(root, "../escape", "process_" + "0" * 32)
    with pytest.raises(ProcessLogError):
        ProcessLog.open(root, "thread_test", "process_../../escape")
    log = ProcessLog.open(root, "thread_test", "process_" + "0" * 32)
    assert log.path.parent == (root.resolve() / "thread_test")
    assert log.path.parent.is_relative_to(root.resolve())

