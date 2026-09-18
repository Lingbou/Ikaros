from __future__ import annotations

import asyncio
import json
import os
import re
import subprocess
import sys
from dataclasses import replace
from pathlib import Path
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


def test_restart_process_read_uses_unicode_character_cursors() -> None:
    class RestartStore:
        def process_records(self) -> list[JsonObject]:
            return [
                {
                    "processId": "process_restarted",
                    "threadId": "thread_test",
                    "output": "中文 output",
                }
            ]

    service = ProcessService(ProcessManager(), cast(Any, RestartStore()))

    page = service.read(
        {"threadId": "thread_test", "processId": "process_restarted", "cursor": 1}
    )

    assert page["output"] == "文 output"
    assert page["nextCursor"] == len("中文 output")


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
    manager = ProcessManager(record=facts.append)
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
    assert "中文" in facts[-1]["stdout"] and "error" in facts[-1]["stderr"]
    canonical_timestamp = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$")
    for fact in facts:
        assert canonical_timestamp.fullmatch(str(fact["startedAt"]))
        if fact["finishedAt"] is not None:
            assert canonical_timestamp.fullmatch(str(fact["finishedAt"]))
    await manager.close()


@pytest.mark.asyncio
async def test_repeated_start_and_read_do_not_replay_command(tmp_path: Path) -> None:
    manager = ProcessManager()
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
    with pytest.raises(ProcessError, match="belongs to this Run"):
        manager.read(first["processId"], context=_context(run="run_other"))
    await manager.close()


@pytest.mark.asyncio
async def test_wait_timeout_leaves_process_running_and_stop_is_terminal(tmp_path: Path) -> None:
    manager = ProcessManager()
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
    manager = ProcessManager()
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
    with pytest.raises(ProcessError, match="belongs to this Run"):
        manager.read(result["processId"], context=context)
    with pytest.raises(ProcessError, match="ended"):
        await manager.start(
            "echo forbidden",
            str(tmp_path),
            context=_context(item="item_late"),
            cancellation=CancellationToken(),
        )


@pytest.mark.asyncio
@pytest.mark.parametrize("inherit_output", [True, False])
async def test_root_exit_stops_descendants_without_pipe_hang(
    tmp_path: Path,
    inherit_output: bool,
) -> None:
    manager = ProcessManager()
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
    manager = ProcessManager()
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
    manager = ProcessManager()
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
    manager = ProcessManager()
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
    manager = ProcessManager(record=facts.append, protected_values=lambda: (secret,))
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
async def test_restore_running_becomes_unknown_without_spawning(tmp_path: Path) -> None:
    facts: list[JsonObject] = []
    manager = ProcessManager(record=facts.append)
    context = _context()
    started = await manager.start(
        _command(windows="Start-Sleep 60", posix="sleep 60"),
        str(tmp_path),
        context=context,
        cancellation=CancellationToken(),
    )
    running = dict(facts[-1])
    await manager.close()
    recovered: list[JsonObject] = []
    restored = ProcessManager(record=recovered.append)
    restored.restore([running])
    assert recovered[-1]["state"] == "unknown"
    assert recovered[-1]["exitCode"] is None
    assert recovered[-1]["errorCode"] == "runtime_interrupted"
    with pytest.raises(ProcessError, match="ended"):
        await restored.start(
            "echo must-not-run", str(tmp_path), context=context, cancellation=CancellationToken()
        )
    with pytest.raises(ProcessError, match="belongs to this Run"):
        restored.read(started["processId"], context=context)
    await restored.close()


@pytest.mark.asyncio
async def test_active_command_quota(tmp_path: Path) -> None:
    manager = ProcessManager()
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
    await manager.close()


@pytest.mark.asyncio
async def test_record_failure_after_spawn_cleans_tree(tmp_path: Path) -> None:
    seen: list[JsonObject] = []

    def record(fact: JsonObject) -> None:
        seen.append(fact)
        if fact["state"] == "running":
            raise RuntimeError("disk write failed")

    manager = ProcessManager(record=record)
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

    async def delayed(command: str, cwd: str | None) -> process_platform._SpawnedProcess:
        result = await original(command, cwd)
        pids.append(result.process.pid)
        spawned.set()
        await release.wait()
        return result

    monkeypatch.setattr(manager_module, "_spawn_process", delayed)
    manager = ProcessManager()
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
    fake_process = cast(asyncio.subprocess.Process, object())

    async def fake_spawn(*args: str, **kwargs: Any) -> asyncio.subprocess.Process:
        captured.update(kwargs)
        return fake_process

    monkeypatch.setattr(asyncio, "create_subprocess_exec", fake_spawn)
    monkeypatch.setattr(
        process_platform, "_create_windows_job", lambda process: cast(Any, object())
    )
    result = await process_platform._spawn_process("echo hidden", None)
    assert result.process is fake_process
    assert captured["creationflags"] & subprocess.CREATE_NO_WINDOW
    assert captured["creationflags"] & subprocess.CREATE_NEW_PROCESS_GROUP
    assert captured["creationflags"] & 0x00000004


@pytest.mark.asyncio
async def test_split_secret_across_stdout_stderr_is_hidden_in_combined_log(tmp_path: Path) -> None:
    secret = "joined-secret"
    manager = ProcessManager(protected_values=lambda: (secret,))
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
    manager = ProcessManager(record=facts.append)
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
        "z" * (128 * 1024)
        + f"\n... {2_000_000 - 256 * 1024} bytes omitted ...\n"
        + "z" * (128 * 1024)
    )
    assert facts[-1]["stdout"] == expected
    assert len(facts) <= 12
    assert len(facts[-1]["output"].encode()) <= 1024 * 1024 + 128
    await manager.close()


@pytest.mark.asyncio
async def test_run_closure_releases_buffers_without_overwriting_persisted_output(
    tmp_path: Path,
) -> None:
    facts: list[JsonObject] = []
    manager = ProcessManager(record=facts.append)
    context = _context()
    start = await manager.start(
        "echo saved", str(tmp_path), context=context, cancellation=CancellationToken()
    )
    before = await manager.wait(
        start["processId"], context=context, cancellation=CancellationToken(), timeout_ms=5000
    )
    await manager.close_run(context.run_id)
    with pytest.raises(ProcessError, match="belongs to this Run"):
        manager.read(start["processId"], context=context)
    assert facts[-1]["output"] == before["output"]
    assert facts[-1]["state"] == "exited"
    assert manager._entries == {}
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
    manager = ProcessManager(record=reject)
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
    manager = ProcessManager(record=facts.append)
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
    assert facts[-1]["state"] == "terminated"
    assert "partial" in facts[-1]["stdout"]


@pytest.mark.asyncio
async def test_more_than_thirty_two_sequential_commands_keep_old_results_and_start_identity(
    tmp_path: Path,
) -> None:
    manager = ProcessManager()
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
    manager = ProcessManager()
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
        await manager.close()
