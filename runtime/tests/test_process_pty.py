"""Interactive terminal sessions: input, resize, and byte-cursor reads."""

from __future__ import annotations

import asyncio
import os
import re
import subprocess
import sys
from pathlib import Path

import pytest

from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.domain import JsonObject
from ikaros_runtime.tools import (
    FullAccessPolicy,
    ProcessManager,
    ProcessReadTool,
    ProcessResizeTool,
    ProcessStartTool,
    ProcessStopTool,
    ProcessWaitTool,
    ProcessWriteTool,
    ToolCall,
    ToolExecutionContext,
    ToolExecutor,
    ToolRegistry,
)
from ikaros_runtime.tools.process_platform import PTY_MAX_CELLS

_FIXTURE = Path(__file__).parent / "fixtures" / "pty_repl.py"
_READ_TIMEOUT = 30.0


def _context(*, item: str = "item_pty", run: str = "run_pty") -> ToolExecutionContext:
    return ToolExecutionContext(run, 1, item, "thread_pty", None)


def _executor(manager: ProcessManager) -> ToolExecutor:
    return ToolExecutor(
        ToolRegistry(
            [
                ProcessStartTool(manager),
                ProcessReadTool(manager),
                ProcessWaitTool(manager),
                ProcessStopTool(manager),
                ProcessWriteTool(manager),
                ProcessResizeTool(manager),
            ]
        ),
        FullAccessPolicy(),
    )


def _repl_command() -> str:
    quoted = f'"{sys.executable}" -u "{_FIXTURE}"'
    return f"& {quoted}" if os.name == "nt" else quoted


async def _run(
    executor: ToolExecutor,
    context: ToolExecutionContext,
    name: str,
    arguments: JsonObject,
    *,
    item: str,
) -> JsonObject:
    result = await executor.execute(
        ToolCall(f"call_{item}", name, arguments),
        cancellation=CancellationToken(),
        context=ToolExecutionContext(
            context.run_id,
            context.step_ordinal,
            item,
            context.thread_id,
            context.default_cwd,
        ),
    )
    assert result.ok, result.output
    return {**result.details, "output": result.output}


async def _read_until(
    executor: ToolExecutor,
    context: ToolExecutionContext,
    process_id: str,
    cursor: int,
    needle: str,
    *,
    item: str,
) -> tuple[str, int]:
    """Read pages from a byte cursor until the terminal produced ``needle``."""

    collected: list[str] = []
    deadline = asyncio.get_running_loop().time() + _READ_TIMEOUT
    while asyncio.get_running_loop().time() < deadline:
        page = await _run(
            executor,
            context,
            "process_read",
            {"processId": process_id, "cursor": cursor},
            item=item,
        )
        collected.append(str(page["output"]))
        cursor = int(page["nextCursor"])
        text = "".join(collected)
        if needle in text:
            return text, cursor
        await asyncio.sleep(0.05)
    raise AssertionError(f"{needle!r} never appeared in {''.join(collected)!r}")


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


async def _wait_for_exit(pid: int) -> None:
    for _ in range(200):
        if not await asyncio.to_thread(_process_is_alive, pid):
            return
        await asyncio.sleep(0.05)
    raise AssertionError(f"process {pid} is still alive")


@pytest.mark.asyncio
async def test_pty_session_drives_an_interactive_repl(tmp_path: Path) -> None:
    manager = ProcessManager(logs_root=tmp_path / "processes")
    executor = _executor(manager)
    context = _context()
    started = await _run(
        executor,
        context,
        "process_start",
        {"command": _repl_command(), "pty": True},
        item="start",
    )
    assert started["state"] == "running"
    process_id = str(started["processId"])

    _, cursor = await _read_until(executor, context, process_id, 0, "repl-ready", item="read_ready")
    # A cursor at the end of the retained output never repeats earlier bytes.
    empty = await _run(
        executor,
        context,
        "process_read",
        {"processId": process_id, "cursor": cursor},
        item="read_idle",
    )
    assert empty["output"] == ""
    assert empty["hasMore"] is False

    # A plain newline is delivered as the terminal's line terminator.
    await _run(
        executor,
        context,
        "process_write",
        {"processId": process_id, "data": "size\n", "cursor": cursor},
        item="write_size",
    )
    _, cursor = await _read_until(
        executor, context, process_id, cursor, "size:80x24", item="read_size"
    )

    resized = await _run(
        executor,
        context,
        "process_resize",
        {"processId": process_id, "columns": 100, "rows": 30, "cursor": cursor},
        item="resize",
    )
    assert resized["state"] == "running"
    await _run(
        executor,
        context,
        "process_write",
        {"processId": process_id, "data": "size\n", "cursor": cursor},
        item="write_size_again",
    )
    _, cursor = await _read_until(
        executor, context, process_id, cursor, "size:100x30", item="read_size_again"
    )

    await _run(
        executor,
        context,
        "process_write",
        {"processId": process_id, "data": "echo 123\n", "cursor": cursor},
        item="write_echo",
    )
    _, cursor = await _read_until(
        executor, context, process_id, cursor, "repl-echo:echo 123", item="read_echo"
    )

    await _run(
        executor,
        context,
        "process_write",
        {"processId": process_id, "data": "exit\n", "cursor": cursor},
        item="write_exit",
    )
    final = await _run(
        executor,
        context,
        "process_wait",
        {"processId": process_id, "timeoutMs": 30_000, "cursor": cursor},
        item="wait",
    )
    assert final["state"] == "exited"
    assert final["exitCode"] == 0
    text, _ = await _read_until(
        executor, context, process_id, cursor, "repl-exit", item="read_exit"
    )
    assert "repl-exit" in text
    await manager.close()


@pytest.mark.asyncio
async def test_pty_stop_terminates_the_terminal_and_its_children(tmp_path: Path) -> None:
    manager = ProcessManager(logs_root=tmp_path / "processes")
    executor = _executor(manager)
    context = _context()
    started = await _run(
        executor,
        context,
        "process_start",
        {"command": _repl_command(), "pty": True, "columns": 120, "rows": 40},
        item="start",
    )
    process_id = str(started["processId"])
    pid = int(started["pid"])
    await _read_until(executor, context, process_id, 0, "repl-ready", item="read_ready")

    stopped = await _run(
        executor, context, "process_stop", {"processId": process_id}, item="stop"
    )
    assert stopped["state"] == "terminated"
    await _wait_for_exit(pid)
    await manager.close()


@pytest.mark.asyncio
async def test_pty_arguments_are_validated_and_terminal_only(tmp_path: Path) -> None:
    manager = ProcessManager(logs_root=tmp_path / "processes")
    executor = _executor(manager)
    context = _context()

    with_columns = await executor.execute(
        ToolCall("call_bad_start", "process_start", {"command": "echo hi", "columns": 100}),
        cancellation=CancellationToken(),
        context=context,
    )
    assert not with_columns.ok
    assert with_columns.details["errorCode"] == "invalid_arguments"

    oversized = await executor.execute(
        ToolCall(
            "call_bad_size",
            "process_start",
            {"command": "echo hi", "pty": True, "columns": PTY_MAX_CELLS + 1, "rows": 24},
        ),
        cancellation=CancellationToken(),
        context=context,
    )
    assert not oversized.ok
    assert oversized.details["errorCode"] == "invalid_arguments"

    plain = await _run(
        executor, context, "process_start", {"command": "echo plain"}, item="plain"
    )
    plain_id = str(plain["processId"])
    written = await executor.execute(
        ToolCall("call_write", "process_write", {"processId": plain_id, "data": "hello\n"}),
        cancellation=CancellationToken(),
        context=_context(item="write"),
    )
    assert not written.ok
    assert written.details["errorCode"] == "process_not_terminal"
    resized = await executor.execute(
        ToolCall(
            "call_resize",
            "process_resize",
            {"processId": plain_id, "columns": 100, "rows": 30},
        ),
        cancellation=CancellationToken(),
        context=_context(item="resize"),
    )
    assert not resized.ok
    assert resized.details["errorCode"] == "process_not_terminal"
    await manager.close()


@pytest.mark.asyncio
async def test_pty_write_rejects_an_unknown_or_finished_command(tmp_path: Path) -> None:
    manager = ProcessManager(logs_root=tmp_path / "processes")
    executor = _executor(manager)
    context = _context()
    missing = await executor.execute(
        ToolCall("call_missing", "process_write", {"processId": "process_missing", "data": "x\n"}),
        cancellation=CancellationToken(),
        context=context,
    )
    assert not missing.ok
    assert missing.details["errorCode"] == "process_not_found"
    await manager.close()
