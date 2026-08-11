from __future__ import annotations

import asyncio
import locale
import os
import signal
import subprocess
import time
from collections.abc import Awaitable
from contextlib import suppress
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .cancellation import CancellationToken
from .domain import JsonObject
from .tools import (
    ToolCall,
    ToolDefinition,
    ToolExecutionCancelled,
    ToolResult,
    is_json_integer,
    require_exact_arguments,
)

_DEFAULT_TIMEOUT_MS = 120_000
_MAX_TIMEOUT_MS = 600_000
_OUTPUT_LIMIT_BYTES = 64 * 1024
_COMMAND_LIMIT = 32_768
_CWD_LIMIT = 4_096
_CREATE_NEW_PROCESS_GROUP = 0x00000200
_CREATE_SUSPENDED = 0x00000004


@dataclass(slots=True)
class _SpawnedProcess:
    process: asyncio.subprocess.Process
    windows_job: _WindowsJob | None = None


class _WindowsJob:
    def __init__(self, kernel32: Any, handle: Any) -> None:
        self._kernel32 = kernel32
        self._handle = handle

    def terminate(self) -> None:
        handle = self._handle
        if not handle:
            return
        terminated = self._kernel32.TerminateJobObject(handle, 1)
        self.close()
        if not terminated:
            raise OSError("TerminateJobObject failed for the process tree")

    def close(self) -> None:
        handle = self._handle
        if not handle:
            return
        self._handle = None
        if not self._kernel32.CloseHandle(handle):
            raise OSError("CloseHandle failed for the Windows Job Object")


class ProcessRunTool:
    definition = ToolDefinition(
        name="process_run",
        description="Run a command in the local operating system shell and capture its output.",
        input_schema={
            "type": "object",
            "properties": {
                "command": {"type": "string", "minLength": 1},
                "cwd": {"type": "string", "minLength": 1},
                "timeoutMs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": _MAX_TIMEOUT_MS,
                },
            },
            "required": ["command"],
            "additionalProperties": False,
        },
    )

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
    ) -> ToolResult:
        validated = _validate_arguments(call)
        if isinstance(validated, ToolResult):
            return validated
        command, cwd, timeout_ms = validated
        cancellation.raise_if_cancelled()
        started = time.monotonic()
        spawn_task = asyncio.create_task(_spawn_process(command, cwd))
        try:
            spawned = await asyncio.shield(spawn_task)
        except asyncio.CancelledError as cancellation_error:
            try:
                spawned = await _await_uninterruptibly(spawn_task)
            except BaseException:
                raise cancellation_error from None
            await _await_uninterruptibly(_terminate_process_tree(spawned))
            raise cancellation_error
        except (OSError, ValueError) as error:
            return _result(
                call,
                started=started,
                stdout="",
                stderr="",
                cwd=cwd,
                exit_code=None,
                timed_out=False,
                truncated=False,
                error_code="spawn_failed",
                error_message=str(error),
            )

        if cancellation.is_cancelled:
            await _await_uninterruptibly(_terminate_process_tree(spawned))
            result = _result(
                call,
                started=started,
                stdout="",
                stderr="",
                cwd=cwd,
                exit_code=spawned.process.returncode,
                timed_out=False,
                truncated=False,
                error_code="cancelled",
                cancelled=True,
            )
            raise ToolExecutionCancelled(result)

        try:
            _resume_spawned_process(spawned)
        except (OSError, ValueError) as error:
            await _await_uninterruptibly(_terminate_process_tree(spawned))
            return _result(
                call,
                started=started,
                stdout="",
                stderr="",
                cwd=cwd,
                exit_code=None,
                timed_out=False,
                truncated=False,
                error_code="spawn_failed",
                error_message=str(error),
            )

        process = spawned.process
        if process.stdout is None or process.stderr is None:
            await _terminate_process_tree(spawned)
            raise RuntimeError("process output pipes were not created")

        stdout_task = asyncio.create_task(_read_limited(process.stdout))
        stderr_task = asyncio.create_task(_read_limited(process.stderr))
        wait_task = asyncio.create_task(process.wait())
        completion_task = asyncio.create_task(
            _collect_process_output(wait_task, stdout_task, stderr_task)
        )
        cancel_task = asyncio.create_task(cancellation.wait())
        timed_out = False
        cancelled = False
        try:
            done, _ = await asyncio.wait(
                {completion_task, cancel_task},
                timeout=timeout_ms / 1000,
                return_when=asyncio.FIRST_COMPLETED,
            )
            if completion_task not in done:
                if cancel_task in done:
                    cancelled = True
                else:
                    timed_out = True
                await _terminate_process_tree(spawned)
            (
                (stdout_bytes, stdout_truncated),
                (stderr_bytes, stderr_truncated),
            ) = await completion_task
        finally:
            await _await_uninterruptibly(_terminate_process_tree(spawned))
            if not completion_task.done():
                await _await_uninterruptibly(completion_task)
            for task in (cancel_task, completion_task, wait_task, stdout_task, stderr_task):
                if not task.done():
                    task.cancel()
            for task in (cancel_task, completion_task, wait_task, stdout_task, stderr_task):
                with suppress(asyncio.CancelledError):
                    await task

        result = _result(
            call,
            started=started,
            stdout=_decode_output(stdout_bytes),
            stderr=_decode_output(stderr_bytes),
            cwd=cwd,
            exit_code=process.returncode,
            timed_out=timed_out,
            truncated=stdout_truncated or stderr_truncated,
            error_code=(
                "cancelled"
                if cancelled
                else "timeout"
                if timed_out
                else "non_zero_exit"
                if process.returncode != 0
                else None
            ),
            cancelled=cancelled,
        )
        if cancelled:
            raise ToolExecutionCancelled(result)
        return result


def _validate_arguments(
    call: ToolCall,
) -> tuple[str, str | None, int] | ToolResult:
    mismatch = require_exact_arguments(
        call.arguments,
        required={"command"},
        optional={"cwd", "timeoutMs"},
    )
    if mismatch is not None:
        return ToolResult.rejected(call, code="invalid_arguments", message=mismatch)

    command = call.arguments["command"]
    if not isinstance(command, str) or not command.strip() or len(command) > _COMMAND_LIMIT:
        return ToolResult.rejected(
            call,
            code="invalid_arguments",
            message=f"command must be a non-empty string of at most {_COMMAND_LIMIT} characters",
        )

    cwd_value = call.arguments.get("cwd")
    cwd: str | None = None
    if cwd_value is not None:
        if not isinstance(cwd_value, str) or not cwd_value.strip() or len(cwd_value) > _CWD_LIMIT:
            return ToolResult.rejected(
                call,
                code="invalid_arguments",
                message=f"cwd must be a non-empty string of at most {_CWD_LIMIT} characters",
            )
        path = Path(cwd_value).expanduser()
        try:
            resolved = path.resolve(strict=True)
        except OSError as error:
            return ToolResult.rejected(
                call,
                code="invalid_cwd",
                message=f"cwd is not accessible: {error}",
            )
        if not resolved.is_dir():
            return ToolResult.rejected(
                call,
                code="invalid_cwd",
                message="cwd is not a directory",
            )
        cwd = str(resolved)

    timeout_value = call.arguments.get("timeoutMs", _DEFAULT_TIMEOUT_MS)
    if not is_json_integer(timeout_value) or timeout_value < 1 or timeout_value > _MAX_TIMEOUT_MS:
        return ToolResult.rejected(
            call,
            code="invalid_arguments",
            message=f"timeoutMs must be an integer between 1 and {_MAX_TIMEOUT_MS}",
        )
    return command, cwd, timeout_value


def _shell_command(command: str) -> tuple[str, ...]:
    if os.name == "nt":
        return (
            "powershell.exe",
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            command,
        )
    return ("/bin/sh", "-lc", command)


async def _spawn_process(
    command: str,
    cwd: str | None,
) -> _SpawnedProcess:
    arguments = _shell_command(command)
    if os.name == "nt":
        process = await asyncio.create_subprocess_exec(
            *arguments,
            cwd=cwd,
            env=_minimal_environment(),
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            creationflags=int(
                getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", _CREATE_NEW_PROCESS_GROUP)
                | _CREATE_SUSPENDED
            ),
        )
        job: _WindowsJob | None = None
        try:
            job = _create_windows_job(process)
        except BaseException:
            if job is not None:
                job.terminate()
            else:
                process.kill()
            await process.wait()
            raise
        return _SpawnedProcess(process=process, windows_job=job)
    process = await asyncio.create_subprocess_exec(
        *arguments,
        cwd=cwd,
        env=_minimal_environment(),
        stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        start_new_session=True,
    )
    return _SpawnedProcess(process=process)


def _resume_spawned_process(spawned: _SpawnedProcess) -> None:
    if os.name == "nt":
        _resume_windows_process(spawned.process)


def _minimal_environment() -> dict[str, str]:
    allowed = {
        "APPDATA",
        "COMSPEC",
        "HOME",
        "HOMEDRIVE",
        "HOMEPATH",
        "LANG",
        "LC_ALL",
        "LOCALAPPDATA",
        "PATH",
        "PATHEXT",
        "PROGRAMDATA",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "PSMODULEPATH",
        "SHELL",
        "SYSTEMDRIVE",
        "SYSTEMROOT",
        "TEMP",
        "TMP",
        "TMPDIR",
        "USER",
        "USERNAME",
        "USERPROFILE",
        "WINDIR",
    }
    environment = {key: value for key, value in os.environ.items() if key.upper() in allowed}
    environment["PYTHONIOENCODING"] = "utf-8"
    environment["PYTHONUTF8"] = "1"
    return environment


async def _read_limited(
    stream: asyncio.StreamReader,
    limit: int = _OUTPUT_LIMIT_BYTES,
) -> tuple[bytes, bool]:
    retained = bytearray()
    truncated = False
    while True:
        chunk = await stream.read(16 * 1024)
        if not chunk:
            break
        remaining = limit - len(retained)
        if remaining > 0:
            retained.extend(chunk[:remaining])
        if len(chunk) > max(remaining, 0):
            truncated = True
    return bytes(retained), truncated


async def _collect_process_output(
    wait_task: asyncio.Task[int],
    stdout_task: asyncio.Task[tuple[bytes, bool]],
    stderr_task: asyncio.Task[tuple[bytes, bool]],
) -> tuple[tuple[bytes, bool], tuple[bytes, bool]]:
    await wait_task
    return await stdout_task, await stderr_task


async def _terminate_process_tree(spawned: _SpawnedProcess) -> None:
    process = spawned.process
    if os.name == "nt":
        if spawned.windows_job is not None:
            spawned.windows_job.terminate()
            if process.returncode is None:
                await process.wait()
            return
        try:
            killer = await asyncio.create_subprocess_exec(
                "taskkill.exe",
                "/PID",
                str(process.pid),
                "/T",
                "/F",
                stdin=asyncio.subprocess.DEVNULL,
                stdout=asyncio.subprocess.DEVNULL,
                stderr=asyncio.subprocess.DEVNULL,
            )
            await asyncio.wait_for(killer.wait(), timeout=5)
        except (OSError, TimeoutError):
            process.kill()
    else:
        killpg = getattr(os, "killpg", None)
        sigkill = getattr(signal, "SIGKILL", None)
        if killpg is None or sigkill is None:
            process.kill()
        else:
            with suppress(ProcessLookupError):
                killpg(process.pid, sigkill)
    if process.returncode is None:
        try:
            await asyncio.wait_for(process.wait(), timeout=5)
        except TimeoutError:
            process.kill()
            await process.wait()


def _create_windows_job(process: asyncio.subprocess.Process) -> _WindowsJob:
    import ctypes
    from ctypes import wintypes

    class BasicLimitInformation(ctypes.Structure):
        _fields_ = [
            ("per_process_user_time_limit", ctypes.c_longlong),
            ("per_job_user_time_limit", ctypes.c_longlong),
            ("limit_flags", wintypes.DWORD),
            ("minimum_working_set_size", ctypes.c_size_t),
            ("maximum_working_set_size", ctypes.c_size_t),
            ("active_process_limit", wintypes.DWORD),
            ("affinity", ctypes.c_size_t),
            ("priority_class", wintypes.DWORD),
            ("scheduling_class", wintypes.DWORD),
        ]

    class IoCounters(ctypes.Structure):
        _fields_ = [
            ("read_operation_count", ctypes.c_ulonglong),
            ("write_operation_count", ctypes.c_ulonglong),
            ("other_operation_count", ctypes.c_ulonglong),
            ("read_transfer_count", ctypes.c_ulonglong),
            ("write_transfer_count", ctypes.c_ulonglong),
            ("other_transfer_count", ctypes.c_ulonglong),
        ]

    class ExtendedLimitInformation(ctypes.Structure):
        _fields_ = [
            ("basic_limit_information", BasicLimitInformation),
            ("io_info", IoCounters),
            ("process_memory_limit", ctypes.c_size_t),
            ("job_memory_limit", ctypes.c_size_t),
            ("peak_process_memory_used", ctypes.c_size_t),
            ("peak_job_memory_used", ctypes.c_size_t),
        ]

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
    kernel32.CreateJobObjectW.restype = wintypes.HANDLE
    kernel32.SetInformationJobObject.argtypes = [
        wintypes.HANDLE,
        ctypes.c_int,
        ctypes.c_void_p,
        wintypes.DWORD,
    ]
    kernel32.SetInformationJobObject.restype = wintypes.BOOL
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
    kernel32.AssignProcessToJobObject.restype = wintypes.BOOL
    kernel32.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
    kernel32.TerminateJobObject.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL

    job_handle = kernel32.CreateJobObjectW(None, None)
    if not job_handle:
        raise ctypes.WinError(ctypes.get_last_error())
    job = _WindowsJob(kernel32, job_handle)
    information = ExtendedLimitInformation()
    information.basic_limit_information.limit_flags = 0x00002000
    if not kernel32.SetInformationJobObject(
        job_handle,
        9,
        ctypes.byref(information),
        ctypes.sizeof(information),
    ):
        error = ctypes.WinError(ctypes.get_last_error())
        job.close()
        raise error
    process_handle = kernel32.OpenProcess(0x0001 | 0x0100, False, process.pid)
    if not process_handle:
        error = ctypes.WinError(ctypes.get_last_error())
        job.close()
        raise error
    try:
        assigned = kernel32.AssignProcessToJobObject(job_handle, process_handle)
    finally:
        kernel32.CloseHandle(process_handle)
    if not assigned:
        error = ctypes.WinError(ctypes.get_last_error())
        job.close()
        raise error
    return job


def _resume_windows_process(process: asyncio.subprocess.Process) -> None:
    import ctypes
    from ctypes import wintypes

    class ThreadEntry32(ctypes.Structure):
        _fields_ = [
            ("dwSize", wintypes.DWORD),
            ("cntUsage", wintypes.DWORD),
            ("th32ThreadID", wintypes.DWORD),
            ("th32OwnerProcessID", wintypes.DWORD),
            ("tpBasePri", wintypes.LONG),
            ("tpDeltaPri", wintypes.LONG),
            ("dwFlags", wintypes.DWORD),
        ]

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
    kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    kernel32.Thread32First.argtypes = [wintypes.HANDLE, ctypes.POINTER(ThreadEntry32)]
    kernel32.Thread32First.restype = wintypes.BOOL
    kernel32.Thread32Next.argtypes = [wintypes.HANDLE, ctypes.POINTER(ThreadEntry32)]
    kernel32.Thread32Next.restype = wintypes.BOOL
    kernel32.OpenThread.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenThread.restype = wintypes.HANDLE
    kernel32.ResumeThread.argtypes = [wintypes.HANDLE]
    kernel32.ResumeThread.restype = wintypes.DWORD
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL

    snapshot = kernel32.CreateToolhelp32Snapshot(0x00000004, 0)
    invalid_handle = ctypes.c_void_p(-1).value
    if snapshot == invalid_handle:
        raise ctypes.WinError(ctypes.get_last_error())
    resumed = False
    entry = ThreadEntry32()
    entry.dwSize = ctypes.sizeof(entry)
    try:
        has_entry = bool(kernel32.Thread32First(snapshot, ctypes.byref(entry)))
        while has_entry:
            if entry.th32OwnerProcessID == process.pid:
                thread_handle = kernel32.OpenThread(0x0002, False, entry.th32ThreadID)
                if not thread_handle:
                    raise ctypes.WinError(ctypes.get_last_error())
                try:
                    previous_count = kernel32.ResumeThread(thread_handle)
                finally:
                    kernel32.CloseHandle(thread_handle)
                if previous_count == 0xFFFFFFFF:
                    raise ctypes.WinError(ctypes.get_last_error())
                resumed = True
            has_entry = bool(kernel32.Thread32Next(snapshot, ctypes.byref(entry)))
    finally:
        kernel32.CloseHandle(snapshot)
    if not resumed:
        raise OSError(f"could not find a suspended thread for process {process.pid}")


async def _await_uninterruptibly[T](operation: Awaitable[T]) -> T:
    task = asyncio.ensure_future(operation)
    while not task.done():
        try:
            await asyncio.shield(task)
        except asyncio.CancelledError:
            continue
    return await task


def _decode_output(value: bytes) -> str:
    try:
        return value.decode("utf-8")
    except UnicodeDecodeError:
        return value.decode(locale.getpreferredencoding(False), errors="replace")


def _result(
    call: ToolCall,
    *,
    started: float,
    stdout: str,
    stderr: str,
    cwd: str | None,
    exit_code: int | None,
    timed_out: bool,
    truncated: bool,
    error_code: str | None,
    error_message: str | None = None,
    cancelled: bool = False,
) -> ToolResult:
    duration_ms = max(0, round((time.monotonic() - started) * 1000))
    pieces = [part for part in (stdout, stderr) if part]
    if error_message:
        pieces.append(error_message)
    output = "\n".join(piece.rstrip("\r\n") for piece in pieces)
    details: JsonObject = {
        "stdout": stdout,
        "stderr": stderr,
        "cwd": cwd if cwd is not None else str(Path.cwd()),
        "exitCode": exit_code,
        "durationMs": duration_ms,
        "timedOut": timed_out,
        "truncated": truncated,
    }
    if error_code is not None:
        details["errorCode"] = error_code
    return ToolResult(
        tool_call_id=call.id,
        tool_name=call.name,
        ok=not cancelled and not timed_out and error_code is None and exit_code == 0,
        output=output,
        details=details,
        cancelled=cancelled,
    )
