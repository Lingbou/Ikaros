"""Platform process-tree ownership and pseudo-terminal primitives."""

from __future__ import annotations

import asyncio
import base64
import ctypes
import os
import signal
import struct
import subprocess
import sys
import threading
from collections.abc import Awaitable, Callable
from contextlib import suppress
from dataclasses import dataclass
from datetime import datetime
from typing import IO, Any

_CREATE_NEW_PROCESS_GROUP = 0x00000200
_CREATE_SUSPENDED = 0x00000004
_CREATE_NO_WINDOW = 0x08000000


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


def _shell_command(command: str) -> tuple[str, ...]:
    if os.name == "nt":
        return (
            "powershell.exe",
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$OutputEncoding = [Console]::OutputEncoding = "
            "[System.Text.UTF8Encoding]::new($false); " + command,
        )
    return ("/bin/sh", "-lc", command)


def _windows_creation_flags() -> int:
    return int(
        getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", _CREATE_NEW_PROCESS_GROUP)
        | getattr(subprocess, "CREATE_NO_WINDOW", _CREATE_NO_WINDOW)
        | _CREATE_SUSPENDED
    )


async def _spawn_process(
    command: str,
    cwd: str | None,
    *,
    log: IO[bytes] | None = None,
) -> _SpawnedProcess:
    """Start one command; ``log`` receives stdout/stderr instead of pipes."""
    arguments = _shell_command(command)
    stdout: Any = log if log is not None else asyncio.subprocess.PIPE
    if os.name == "nt":
        process = await asyncio.create_subprocess_exec(
            *arguments,
            cwd=cwd,
            env=_minimal_environment(),
            stdin=asyncio.subprocess.DEVNULL,
            stdout=stdout,
            stderr=stdout,
            creationflags=_windows_creation_flags(),
        )
        job: _WindowsJob | None = None
        try:
            job = _create_windows_job(process.pid)
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
        stdout=stdout,
        stderr=stdout,
        start_new_session=True,
    )
    return _SpawnedProcess(process=process)


def _resume_spawned_process(spawned: _SpawnedProcess) -> None:
    if os.name == "nt":
        _resume_windows_process(spawned.process.pid)


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
                creationflags=int(getattr(subprocess, "CREATE_NO_WINDOW", _CREATE_NO_WINDOW)),
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


def _create_windows_job(pid: int) -> _WindowsJob:
    if sys.platform != "win32":
        raise OSError("Windows Job Objects are unavailable on this platform")

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
    # Commands are thread-owned and survive a Runtime restart (B2); custody
    # (PID plus start time) re-identifies them instead of a kill-on-close job.
    information.basic_limit_information.limit_flags = 0
    if not kernel32.SetInformationJobObject(
        job_handle,
        9,
        ctypes.byref(information),
        ctypes.sizeof(information),
    ):
        error = ctypes.WinError(ctypes.get_last_error())
        job.close()
        raise error
    process_handle = kernel32.OpenProcess(0x0001 | 0x0100, False, pid)
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


def _resume_windows_process(pid: int) -> None:
    if sys.platform != "win32":
        raise OSError("Windows suspended processes are unavailable on this platform")

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
            if entry.th32OwnerProcessID == pid:
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
        raise OSError(f"could not find a suspended thread for process {pid}")


async def _await_uninterruptibly[T](operation: Awaitable[T]) -> T:
    task = asyncio.ensure_future(operation)
    while not task.done():
        try:
            await asyncio.shield(task)
        except asyncio.CancelledError:
            continue
    return await task


# --- Pseudo-terminal sessions -------------------------------------------------
#
# A PTY command is the same managed process as a pipe-backed one, except that the
# child owns an interactive terminal: process_write feeds input, process_resize
# changes the window size, and the merged terminal stream is read with the usual
# byte cursors.

_PTY_READ_SIZE = 16 * 1024
_PTY_LAUNCHER = "windows_pty_launcher.py"
_PTY_STREAM_LIMIT = 256 * 1024
_PTY_FINISH_TIMEOUT = 5.0
PTY_MIN_CELLS = 1
PTY_MAX_CELLS = 4_096
PTY_DEFAULT_COLUMNS = 80
PTY_DEFAULT_ROWS = 24

_CREATE_EXTENDED_STARTUPINFO_PRESENT = 0x00080000
_CREATE_UNICODE_ENVIRONMENT = 0x00000400
_PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE = 0x00020016
_ERROR_INSUFFICIENT_BUFFER = 122
_STILL_ACTIVE = 259


class _WindowsCoord(ctypes.Structure):
    _fields_ = [("X", ctypes.c_int16), ("Y", ctypes.c_int16)]


class _WindowsStartupInfo(ctypes.Structure):
    _fields_ = [
        ("cb", ctypes.c_uint32),
        ("lpReserved", ctypes.c_wchar_p),
        ("lpDesktop", ctypes.c_wchar_p),
        ("lpTitle", ctypes.c_wchar_p),
        ("dwX", ctypes.c_uint32),
        ("dwY", ctypes.c_uint32),
        ("dwXSize", ctypes.c_uint32),
        ("dwYSize", ctypes.c_uint32),
        ("dwXCountChars", ctypes.c_uint32),
        ("dwYCountChars", ctypes.c_uint32),
        ("dwFillAttribute", ctypes.c_uint32),
        ("dwFlags", ctypes.c_uint32),
        ("wShowWindow", ctypes.c_uint16),
        ("cbReserved2", ctypes.c_uint16),
        ("lpReserved2", ctypes.c_void_p),
        ("hStdInput", ctypes.c_void_p),
        ("hStdOutput", ctypes.c_void_p),
        ("hStdError", ctypes.c_void_p),
    ]


class _WindowsStartupInfoEx(ctypes.Structure):
    _fields_ = [
        ("StartupInfo", _WindowsStartupInfo),
        ("lpAttributeList", ctypes.c_void_p),
    ]


class _WindowsProcessInformation(ctypes.Structure):
    _fields_ = [
        ("hProcess", ctypes.c_void_p),
        ("hThread", ctypes.c_void_p),
        ("dwProcessId", ctypes.c_uint32),
        ("dwThreadId", ctypes.c_uint32),
    ]


class _PtyFlowControl(asyncio.ReadTransport):
    """Bridge StreamReader backpressure onto a platform PTY reader.

    ``StreamReader`` pauses its transport while more than twice the stream limit
    is buffered. Honoring that pause stops pulling bytes from the terminal, so a
    chatty child blocks on its own buffer instead of growing Runtime memory.
    """

    def __init__(self) -> None:
        self._pause: Callable[[], None] | None = None
        self._resume: Callable[[], None] | None = None

    def bind(self, *, pause: Callable[[], None], resume: Callable[[], None]) -> None:
        self._pause = pause
        self._resume = resume

    def is_closing(self) -> bool:
        return False

    def close(self) -> None:
        return None

    def pause_reading(self) -> None:
        if self._pause is not None:
            self._pause()

    def resume_reading(self) -> None:
        if self._resume is not None:
            self._resume()


class _PtySession:
    """A child attached to a pseudo-terminal; the manager reads it like a pipe."""

    def __init__(self, pid: int, loop: asyncio.AbstractEventLoop) -> None:
        self._pid = pid
        self._loop = loop
        self._exit_code: int | None = None
        self._finished = False
        self._eof_delivered = False
        self._eof_event = asyncio.Event()
        self._flow = _PtyFlowControl()
        self._output = asyncio.StreamReader(limit=_PTY_STREAM_LIMIT, loop=loop)
        self._output.set_transport(self._flow)

    @property
    def pid(self) -> int:
        return self._pid

    @property
    def output(self) -> asyncio.StreamReader:
        """The merged terminal stream, read with the usual byte cursors."""
        return self._output

    @property
    def returncode(self) -> int | None:
        return self._exit_code

    def _bind_reader(self, *, pause: Callable[[], None], resume: Callable[[], None]) -> None:
        self._flow.bind(pause=pause, resume=resume)

    def _feed(self, chunk: bytes) -> None:
        if self._eof_delivered:
            return
        self._output.feed_data(chunk)

    def _feed_eof(self) -> None:
        if self._eof_delivered:
            return
        self._eof_delivered = True
        self._output.feed_eof()
        self._eof_event.set()

    async def _await_output_eof(self) -> None:
        # Never wait forever: the manager still has to release the session.
        with suppress(TimeoutError):
            await asyncio.wait_for(self._eof_event.wait(), timeout=_PTY_FINISH_TIMEOUT)

    async def write(self, data: bytes) -> None:
        raise NotImplementedError

    async def resize(self, columns: int, rows: int) -> None:
        raise NotImplementedError

    async def terminate_tree(self) -> None:
        raise NotImplementedError

    async def finish(self) -> None:
        """Deliver the last bytes and release platform resources; idempotent."""
        raise NotImplementedError

    def release(self) -> None:
        raise NotImplementedError


class _PosixPtySession(_PtySession):
    """A POSIX child that leads its own session with the PTY as controlling terminal."""

    def __init__(self, pid: int, master_fd: int, loop: asyncio.AbstractEventLoop) -> None:
        super().__init__(pid, loop)
        self._master_fd = master_fd
        self._write_lock = asyncio.Lock()
        self._reap_lock = threading.Lock()
        self._reading = False
        os.set_blocking(master_fd, False)
        self._bind_reader(pause=self._pause_reading, resume=self._resume_reading)
        self._resume_reading()

    @property
    def returncode(self) -> int | None:
        if self._exit_code is not None or not self._reap_lock.acquire(blocking=False):
            return self._exit_code
        try:
            return self._reap(block=False)
        finally:
            self._reap_lock.release()

    def _reap(self, *, block: bool) -> int | None:
        if self._exit_code is not None:
            return self._exit_code
        wnohang = getattr(os, "WNOHANG", 0)
        try:
            waited, status = os.waitpid(self._pid, 0 if block else wnohang)
        except ChildProcessError:
            # Only this Runtime reaps the child; a missing child is already terminal.
            self._exit_code = -1
            return self._exit_code
        if waited == 0:
            return None
        self._exit_code = os.waitstatus_to_exitcode(status)
        return self._exit_code

    def _pause_reading(self) -> None:
        self._stop_reading()

    def _resume_reading(self) -> None:
        if self._reading or self._master_fd < 0:
            return
        self._reading = True
        self._loop.add_reader(self._master_fd, self._on_readable)

    def _stop_reading(self) -> None:
        if not self._reading:
            return
        self._reading = False
        with suppress(Exception):
            self._loop.remove_reader(self._master_fd)

    def _on_readable(self) -> None:
        try:
            chunk = os.read(self._master_fd, _PTY_READ_SIZE)
        except BlockingIOError:
            return
        except OSError:
            chunk = b""
        if chunk:
            self._feed(chunk)
            return
        self._stop_reading()
        self._feed_eof()

    async def write(self, data: bytes) -> None:
        if not data:
            return
        async with self._write_lock:
            await asyncio.to_thread(self._write_blocking, data)

    def _write_blocking(self, data: bytes) -> None:
        import select

        view = memoryview(data)
        while view:
            try:
                written = os.write(self._master_fd, view)
            except BlockingIOError:
                select.select([], [self._master_fd], [], 0.5)
                continue
            except OSError as error:
                raise OSError(f"terminal input failed: {error}") from error
            view = view[written:]

    async def resize(self, columns: int, rows: int) -> None:
        _set_posix_pty_size(self._master_fd, columns, rows)

    async def terminate_tree(self) -> None:
        sigkill = getattr(signal, "SIGKILL", None)
        if sigkill is not None:
            killpg = getattr(os, "killpg", None)
            if killpg is not None:
                with suppress(ProcessLookupError):
                    killpg(self._pid, sigkill)
            with suppress(ProcessLookupError):
                os.kill(self._pid, sigkill)
        await asyncio.to_thread(self._reap, block=True)

    async def finish(self) -> None:
        if self._finished:
            return
        self._resume_reading()
        self._finished = True
        await self._await_output_eof()
        self.release()

    def release(self) -> None:
        self._stop_reading()
        self._feed_eof()
        if self._master_fd >= 0:
            master_fd, self._master_fd = self._master_fd, -1
            with suppress(OSError):
                os.close(master_fd)


def _set_posix_pty_size(master_fd: int, columns: int, rows: int) -> None:
    import fcntl
    import termios

    ioctl = getattr(fcntl, "ioctl", None)
    winsize = getattr(termios, "TIOCSWINSZ", None)
    if ioctl is None or winsize is None:
        raise OSError("pseudo-terminal resizing is unavailable on this platform")
    ioctl(master_fd, winsize, struct.pack("HHHH", rows, columns, 0, 0))


def _spawn_posix_pty(
    command: str,
    cwd: str | None,
    *,
    columns: int,
    rows: int,
    loop: asyncio.AbstractEventLoop,
) -> _PtySession:
    import pty as pty_module

    fork = getattr(pty_module, "fork", None)
    if fork is None:
        raise OSError("pseudo-terminals are unavailable on this platform")
    argv = ["/bin/sh", "-lc", command]
    environment = _pty_environment()
    pid, master_fd = fork()
    if pid == 0:  # pragma: no cover - the child replaces itself immediately
        try:
            if cwd is not None:
                os.chdir(cwd)
            os.execvpe(argv[0], argv, environment)
        except BaseException:
            os._exit(127)
    try:
        _set_posix_pty_size(master_fd, columns, rows)
    except OSError:
        sigkill = getattr(signal, "SIGKILL", None)
        if sigkill is not None:
            with suppress(ProcessLookupError):
                os.kill(pid, sigkill)
        with suppress(OSError):
            os.close(master_fd)
        raise
    return _PosixPtySession(pid, master_fd, loop)


def _configure_conpty_functions(kernel32: Any) -> None:
    """Declare the ctypes signatures used by the ConPTY session lifecycle."""
    kernel32.CreatePipe.argtypes = [
        ctypes.POINTER(ctypes.c_void_p),
        ctypes.POINTER(ctypes.c_void_p),
        ctypes.c_void_p,
        ctypes.c_uint32,
    ]
    kernel32.CreatePipe.restype = ctypes.c_int
    kernel32.CreatePseudoConsole.argtypes = [
        _WindowsCoord,
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.c_uint32,
        ctypes.POINTER(ctypes.c_void_p),
    ]
    kernel32.CreatePseudoConsole.restype = ctypes.c_int32
    kernel32.ResizePseudoConsole.argtypes = [ctypes.c_void_p, _WindowsCoord]
    kernel32.ResizePseudoConsole.restype = ctypes.c_int32
    kernel32.ClosePseudoConsole.argtypes = [ctypes.c_void_p]
    kernel32.ClosePseudoConsole.restype = None
    kernel32.InitializeProcThreadAttributeList.argtypes = [
        ctypes.c_void_p,
        ctypes.c_uint32,
        ctypes.c_uint32,
        ctypes.POINTER(ctypes.c_size_t),
    ]
    kernel32.InitializeProcThreadAttributeList.restype = ctypes.c_int
    kernel32.UpdateProcThreadAttribute.argtypes = [
        ctypes.c_void_p,
        ctypes.c_uint32,
        ctypes.c_size_t,
        ctypes.c_void_p,
        ctypes.c_size_t,
        ctypes.c_void_p,
        ctypes.c_void_p,
    ]
    kernel32.UpdateProcThreadAttribute.restype = ctypes.c_int
    kernel32.DeleteProcThreadAttributeList.argtypes = [ctypes.c_void_p]
    kernel32.DeleteProcThreadAttributeList.restype = None
    kernel32.CreateProcessW.argtypes = [
        ctypes.c_wchar_p,
        ctypes.c_wchar_p,
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.c_int,
        ctypes.c_uint32,
        ctypes.c_void_p,
        ctypes.c_wchar_p,
        ctypes.POINTER(_WindowsStartupInfoEx),
        ctypes.POINTER(_WindowsProcessInformation),
    ]
    kernel32.CreateProcessW.restype = ctypes.c_int
    kernel32.ReadFile.argtypes = [
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.c_uint32,
        ctypes.POINTER(ctypes.c_uint32),
        ctypes.c_void_p,
    ]
    kernel32.ReadFile.restype = ctypes.c_int
    kernel32.WriteFile.argtypes = [
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.c_uint32,
        ctypes.POINTER(ctypes.c_uint32),
        ctypes.c_void_p,
    ]
    kernel32.WriteFile.restype = ctypes.c_int
    kernel32.GetExitCodeProcess.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(ctypes.c_uint32),
    ]
    kernel32.GetExitCodeProcess.restype = ctypes.c_int
    kernel32.TerminateProcess.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
    kernel32.TerminateProcess.restype = ctypes.c_int
    kernel32.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
    kernel32.WaitForSingleObject.restype = ctypes.c_uint32
    kernel32.ResumeThread.argtypes = [ctypes.c_void_p]
    kernel32.ResumeThread.restype = ctypes.c_uint32
    kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
    kernel32.CloseHandle.restype = ctypes.c_int


def _windows_pty_launcher_path() -> str:
    from importlib.resources import files as resource_files

    return str(resource_files("ikaros_runtime.resources").joinpath(_PTY_LAUNCHER))


def _windows_pty_command_line(command: str) -> str:
    """Build the ConPTY client command line for one shell command.

    A client created with a pseudoconsole inherits the console host's standard
    handles - the pipes the Runtime itself was started with - so the bundled
    launcher reattaches the console devices first. The shell wrapper then uses
    ``-EncodedCommand`` to avoid a second round of command-line quoting, and its
    preamble switches the console to UTF-8 like the pipe-backed shell.
    """
    script = (
        "$OutputEncoding = [Console]::OutputEncoding = "
        "[System.Text.UTF8Encoding]::new($false); " + command
    )
    encoded = base64.b64encode(script.encode("utf-16-le")).decode("ascii")
    wrapper = f"powershell.exe -NoLogo -NoProfile -EncodedCommand {encoded}"
    return f'"{sys.executable}" "{_windows_pty_launcher_path()}" "{wrapper}"'


@dataclass(slots=True)
class _WindowsPtyHandles:
    """Raw ConPTY ownership, built off-loop and adopted by the session."""

    kernel32: Any
    pid: int
    process_handle: int
    job: _WindowsJob | None
    pseudo_console: int
    input_handle: int
    output_handle: int


class _WindowsPtySession(_PtySession):
    """A Windows child attached to a ConPTY pseudo-console."""

    def __init__(
        self, *, handles: _WindowsPtyHandles, loop: asyncio.AbstractEventLoop
    ) -> None:
        super().__init__(handles.pid, loop)
        self._kernel32 = handles.kernel32
        self._process_handle: int | None = handles.process_handle
        self._job: _WindowsJob | None = handles.job
        self._pseudo_console: int | None = handles.pseudo_console
        self._input_handle: int | None = handles.input_handle
        self._output_handle: int | None = handles.output_handle
        self._write_lock = asyncio.Lock()
        self._room = threading.Event()
        self._room.set()
        self._reader = threading.Thread(
            target=self._read_loop, name="ikaros-pty-reader", daemon=True
        )
        self._bind_reader(pause=self._room.clear, resume=self._room.set)
        self._reader.start()

    @property
    def returncode(self) -> int | None:
        if self._exit_code is not None:
            return self._exit_code
        handle = self._process_handle
        if not handle:
            return None
        code = ctypes.c_uint32(0)
        if not self._kernel32.GetExitCodeProcess(handle, ctypes.byref(code)):
            raise OSError("GetExitCodeProcess failed for the terminal process")
        if code.value == _STILL_ACTIVE:
            return None
        self._exit_code = code.value
        return self._exit_code

    def _read_loop(self) -> None:
        buffer = ctypes.create_string_buffer(_PTY_READ_SIZE)
        while True:
            self._room.wait()
            handle = self._output_handle
            if not handle:
                break
            read = ctypes.c_uint32(0)
            succeeded = self._kernel32.ReadFile(
                handle,
                ctypes.cast(buffer, ctypes.c_void_p),
                _PTY_READ_SIZE,
                ctypes.byref(read),
                None,
            )
            if not succeeded or read.value == 0:
                break
            self._deliver(buffer.raw[: read.value])
        self._deliver_eof()

    def _deliver(self, chunk: bytes) -> None:
        with suppress(RuntimeError):
            self._loop.call_soon_threadsafe(self._feed, chunk)

    def _deliver_eof(self) -> None:
        with suppress(RuntimeError):
            self._loop.call_soon_threadsafe(self._feed_eof)

    async def write(self, data: bytes) -> None:
        if not data:
            return
        async with self._write_lock:
            await asyncio.to_thread(self._write_blocking, data)

    def _write_blocking(self, data: bytes) -> None:
        written = ctypes.c_uint32(0)
        # A console submits a line on carriage return, so accept plain newlines
        # the way a terminal would deliver them.
        view = memoryview(data.replace(b"\r\n", b"\n").replace(b"\n", b"\r\n"))
        while view:
            handle = self._input_handle
            if not handle:
                raise OSError("terminal input failed: the command has ended")
            chunk = bytes(view[: _PTY_READ_SIZE])
            buffer = ctypes.create_string_buffer(chunk, len(chunk))
            succeeded = self._kernel32.WriteFile(
                handle,
                ctypes.cast(buffer, ctypes.c_void_p),
                len(chunk),
                ctypes.byref(written),
                None,
            )
            if not succeeded or written.value == 0:
                raise OSError("terminal input failed: the command is not reading input")
            view = view[written.value :]

    async def resize(self, columns: int, rows: int) -> None:
        await asyncio.to_thread(self._resize_blocking, columns, rows)

    def _resize_blocking(self, columns: int, rows: int) -> None:
        handle = self._pseudo_console
        if not handle:
            raise OSError("the terminal has ended")
        result = self._kernel32.ResizePseudoConsole(handle, _WindowsCoord(columns, rows))
        if result < 0:
            raise OSError(f"ResizePseudoConsole failed with status 0x{result & 0xFFFFFFFF:08x}")

    async def terminate_tree(self) -> None:
        if self._job is not None:
            self._job.terminate()
        elif self._process_handle:
            with suppress(OSError):
                self._kernel32.TerminateProcess(self._process_handle, 1)
        await asyncio.to_thread(self._wait_for_exit)

    def _wait_for_exit(self) -> None:
        handle = self._process_handle
        if not handle:
            return
        self._kernel32.WaitForSingleObject(handle, int(_PTY_FINISH_TIMEOUT * 1000))
        with suppress(OSError):
            _ = self.returncode

    async def finish(self) -> None:
        if self._finished:
            return
        self._finished = True
        await asyncio.to_thread(self._finish_blocking)
        self.release()

    def _finish_blocking(self) -> None:
        # ClosePseudoConsole blocks until the console host has drained its output,
        # so it runs on its own thread while the reader keeps pulling bytes.
        closer = self._begin_close_pseudo_console()
        self._room.set()
        if self._reader.is_alive():
            self._reader.join(_PTY_FINISH_TIMEOUT)
        if closer is not None:
            closer.join(2.0)

    def _begin_close_pseudo_console(self) -> threading.Thread | None:
        handle, self._pseudo_console = self._pseudo_console, None
        if not handle:
            return None
        thread = threading.Thread(
            target=self._close_pseudo_console,
            args=(handle,),
            name="ikaros-pty-close",
            daemon=True,
        )
        thread.start()
        return thread

    def _close_pseudo_console(self, handle: int) -> None:
        with suppress(Exception):
            self._kernel32.ClosePseudoConsole(handle)

    def release(self) -> None:
        self._room.set()
        self._begin_close_pseudo_console()
        self._feed_eof()
        input_handle, self._input_handle = self._input_handle, None
        output_handle, self._output_handle = self._output_handle, None
        process_handle, self._process_handle = self._process_handle, None
        for handle in (input_handle, output_handle, process_handle):
            if handle:
                with suppress(Exception):
                    self._kernel32.CloseHandle(handle)
        job, self._job = self._job, None
        if job is not None:
            with suppress(OSError):
                job.close()


def _create_windows_pty(
    command: str,
    cwd: str | None,
    *,
    columns: int,
    rows: int,
) -> _WindowsPtyHandles:
    if sys.platform != "win32":
        raise OSError("ConPTY is unavailable on this platform")

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    if not all(
        hasattr(kernel32, name)
        for name in ("CreatePseudoConsole", "ResizePseudoConsole", "ClosePseudoConsole")
    ):
        raise OSError("ConPTY requires Windows 10 version 1809 or newer")
    _configure_conpty_functions(kernel32)

    input_reader = ctypes.c_void_p()
    input_writer = ctypes.c_void_p()
    output_reader = ctypes.c_void_p()
    output_writer = ctypes.c_void_p()
    if not kernel32.CreatePipe(
        ctypes.byref(input_reader), ctypes.byref(input_writer), None, 0
    ) or not kernel32.CreatePipe(
        ctypes.byref(output_reader), ctypes.byref(output_writer), None, 0
    ):
        raise ctypes.WinError(ctypes.get_last_error())

    pseudo_console = ctypes.c_void_p()
    try:
        result = kernel32.CreatePseudoConsole(
            _WindowsCoord(columns, rows),
            input_reader,
            output_writer,
            0,
            ctypes.byref(pseudo_console),
        )
        if result < 0 or not pseudo_console:
            raise OSError(
                f"CreatePseudoConsole failed with status 0x{result & 0xFFFFFFFF:08x}"
            )
        attribute_size = ctypes.c_size_t(0)
        if not kernel32.InitializeProcThreadAttributeList(
            None, 1, 0, ctypes.byref(attribute_size)
        ) and ctypes.get_last_error() != _ERROR_INSUFFICIENT_BUFFER:
            raise ctypes.WinError(ctypes.get_last_error())
        attribute_buffer = ctypes.create_string_buffer(attribute_size.value)
        attribute_list = ctypes.cast(attribute_buffer, ctypes.c_void_p)
        if not kernel32.InitializeProcThreadAttributeList(
            attribute_list, 1, 0, ctypes.byref(attribute_size)
        ):
            raise ctypes.WinError(ctypes.get_last_error())
        # Handle-valued attributes carry the handle itself in lpValue; a pointer
        # to a copy is silently ignored and the child never joins the console.
        if not kernel32.UpdateProcThreadAttribute(
            attribute_list,
            0,
            _PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
            ctypes.c_void_p(pseudo_console.value or 0),
            ctypes.sizeof(ctypes.c_void_p),
            None,
            None,
        ):
            raise ctypes.WinError(ctypes.get_last_error())

        startup = _WindowsStartupInfoEx()
        startup.StartupInfo.cb = ctypes.sizeof(_WindowsStartupInfoEx)
        startup.lpAttributeList = attribute_list
        environment = _pty_environment()
        environment_block = ctypes.create_unicode_buffer(
            "".join(f"{key}={value}\0" for key, value in environment.items())
        )
        command_line = ctypes.create_unicode_buffer(_windows_pty_command_line(command))
        process_information = _WindowsProcessInformation()
        created = kernel32.CreateProcessW(
            None,
            command_line,
            None,
            None,
            False,
            _CREATE_EXTENDED_STARTUPINFO_PRESENT
            | _CREATE_UNICODE_ENVIRONMENT
            | _CREATE_SUSPENDED,
            ctypes.cast(environment_block, ctypes.c_void_p),
            cwd,
            ctypes.byref(startup),
            ctypes.byref(process_information),
        )
        error = ctypes.get_last_error()
        kernel32.DeleteProcThreadAttributeList(attribute_list)
        if not created:
            raise ctypes.WinError(error)
    except BaseException:
        if pseudo_console.value:
            kernel32.ClosePseudoConsole(pseudo_console)
        kernel32.CloseHandle(input_reader)
        kernel32.CloseHandle(input_writer)
        kernel32.CloseHandle(output_reader)
        kernel32.CloseHandle(output_writer)
        raise
    # The pseudo-console owns its ends of both pipes; the parent keeps the rest.
    kernel32.CloseHandle(input_reader)
    kernel32.CloseHandle(output_writer)

    process_handle = int(process_information.hProcess or 0)
    thread_handle = int(process_information.hThread or 0)
    pid = int(process_information.dwProcessId)
    job: _WindowsJob | None = None
    try:
        job = _create_windows_job(pid)
        if kernel32.ResumeThread(thread_handle) == 0xFFFFFFFF:
            raise ctypes.WinError(ctypes.get_last_error())
    except BaseException:
        if job is not None:
            job.terminate()
        else:
            kernel32.TerminateProcess(process_handle, 1)
        kernel32.CloseHandle(thread_handle)
        kernel32.CloseHandle(process_handle)
        kernel32.CloseHandle(input_writer)
        kernel32.CloseHandle(output_reader)
        kernel32.ClosePseudoConsole(pseudo_console)
        raise
    kernel32.CloseHandle(thread_handle)
    return _WindowsPtyHandles(
        kernel32=kernel32,
        pid=pid,
        process_handle=process_handle,
        job=job,
        pseudo_console=int(pseudo_console.value or 0),
        input_handle=int(input_writer.value or 0),
        output_handle=int(output_reader.value or 0),
    )


def _pty_environment() -> dict[str, str]:
    environment = _minimal_environment()
    if os.name != "nt":
        environment["TERM"] = "xterm-256color"
    return environment


async def _spawn_pty(
    command: str,
    cwd: str | None,
    *,
    columns: int,
    rows: int,
) -> _PtySession:
    """Start a command attached to a platform pseudo-terminal."""
    loop = asyncio.get_running_loop()
    if os.name == "nt":
        handles = await asyncio.to_thread(
            _create_windows_pty,
            command,
            cwd,
            columns=columns,
            rows=rows,
        )
        return _WindowsPtySession(handles=handles, loop=loop)
    return _spawn_posix_pty(command, cwd, columns=columns, rows=rows, loop=loop)


# --- Custody: re-identify and settle commands after a Runtime restart ---------

_CUSTODY_START_TOLERANCE_SECONDS = 5.0


def _parse_started_at(value: str) -> float | None:
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None
    if parsed.tzinfo is None:
        return None
    return parsed.timestamp()


def _probe_windows_custody(pid: int, expected: float | None) -> str:
    from ctypes import wintypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.GetProcessTimes.argtypes = [
        wintypes.HANDLE,
        ctypes.POINTER(wintypes.FILETIME),
        ctypes.POINTER(wintypes.FILETIME),
        ctypes.POINTER(wintypes.FILETIME),
        ctypes.POINTER(wintypes.FILETIME),
    ]
    kernel32.GetProcessTimes.restype = wintypes.BOOL
    kernel32.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    kernel32.GetExitCodeProcess.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL

    process_query_limited_information = 0x1000
    handle = kernel32.OpenProcess(process_query_limited_information, False, pid)
    if not handle:
        # ERROR_INVALID_PARAMETER means there is no process with this PID.
        return "dead" if ctypes.get_last_error() == 87 else "unknown"
    try:
        # An exited process stays addressable while its handles are open; only
        # STILL_ACTIVE (259) means it is really running.
        exit_code = wintypes.DWORD()
        if not kernel32.GetExitCodeProcess(handle, ctypes.byref(exit_code)):
            return "unknown"
        if exit_code.value != 259:
            return "dead"
        creation = wintypes.FILETIME()
        exit_time = wintypes.FILETIME()
        kernel_time = wintypes.FILETIME()
        user_time = wintypes.FILETIME()
        if not kernel32.GetProcessTimes(
            handle,
            ctypes.byref(creation),
            ctypes.byref(exit_time),
            ctypes.byref(kernel_time),
            ctypes.byref(user_time),
        ):
            return "unknown"
    finally:
        kernel32.CloseHandle(handle)
    if expected is None:
        return "unknown"
    filetime = (creation.dwHighDateTime << 32) | creation.dwLowDateTime
    created_epoch = filetime / 10_000_000 - 11_644_473_600
    if abs(created_epoch - expected) <= _CUSTODY_START_TOLERANCE_SECONDS:
        return "alive"
    # The PID was reused: the original command is gone.
    return "dead"


def _posix_process_state(pid: int) -> tuple[float, str] | None:
    try:
        with open(f"/proc/{pid}/stat", "rb") as handle:
            stat = handle.read()
        boot: float | None = None
        with open("/proc/stat", "rb") as handle:
            for line in handle:
                if line.startswith(b"btime "):
                    boot = float(line.split()[1])
                    break
        if boot is None:
            return None
    except (OSError, ValueError):
        return None
    try:
        fields = stat[stat.rindex(b")") + 2 :].split()
        state = fields[0].decode("ascii", errors="replace")
        start_ticks = int(fields[19])  # field 22: starttime in clock ticks
        sysconf = getattr(os, "sysconf", None)
        if sysconf is None:
            return None
        hz = float(sysconf("SC_CLK_TCK"))
    except (IndexError, OSError, ValueError):
        return None
    return boot + start_ticks / hz, state


def _probe_posix_custody(pid: int, expected: float | None) -> str:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return "dead"
    except PermissionError:
        pass
    except OSError:
        return "unknown"
    if expected is None:
        return "unknown"
    probed = _posix_process_state(pid)
    if probed is None:
        return "unknown"
    actual, state = probed
    if state in {"Z", "X"}:
        return "dead"
    if abs(actual - expected) <= _CUSTODY_START_TOLERANCE_SECONDS:
        return "alive"
    return "dead"


def probe_custody(pid: int, started_at: str) -> str:
    """Classify a persisted command by PID plus recorded start time.

    Returns ``alive`` only when a process with this PID started at the recorded
    time, ``dead`` when the PID is gone or was reused, and ``unknown`` when the
    platform cannot verify the identity.
    """
    expected = _parse_started_at(started_at)
    if os.name == "nt":
        return _probe_windows_custody(pid, expected)
    return _probe_posix_custody(pid, expected)


async def terminate_custody_tree(pid: int, started_at: str) -> str:
    """Stop a command re-identified by custody after a Runtime restart.

    Returns ``terminated``, ``gone`` (already exited) or ``unverifiable``
    (identity mismatch or the platform refused to kill).
    """
    outcome = probe_custody(pid, started_at)
    if outcome == "dead":
        return "gone"
    if outcome != "alive":
        return "unverifiable"
    if os.name == "nt":
        try:
            killer = await asyncio.create_subprocess_exec(
                "taskkill.exe",
                "/PID",
                str(pid),
                "/T",
                "/F",
                stdin=asyncio.subprocess.DEVNULL,
                stdout=asyncio.subprocess.DEVNULL,
                stderr=asyncio.subprocess.DEVNULL,
                creationflags=int(getattr(subprocess, "CREATE_NO_WINDOW", _CREATE_NO_WINDOW)),
            )
            await asyncio.wait_for(killer.wait(), timeout=10)
        except (OSError, TimeoutError):
            return "unverifiable"
        return "terminated"
    killpg = getattr(os, "killpg", None)
    sigkill = getattr(signal, "SIGKILL", None)
    if killpg is None or sigkill is None:
        return "unverifiable"
    with suppress(ProcessLookupError):
        killpg(pid, sigkill)
    return "terminated"