"""Run one command line with the attached console as its standard handles.

A ConPTY client inherits the console host's standard handles, and the Runtime
starts that host with the pipes it was launched with. This launcher therefore
reopens the console devices with read access - so console queries such as the
window size keep working - and hands them to the command it starts.
"""

from __future__ import annotations

import ctypes
import msvcrt
import os
import subprocess
import sys
from ctypes import wintypes

_GENERIC_READ = 0x80000000
_GENERIC_WRITE = 0x40000000
_FILE_SHARE_READ_WRITE = 0x00000001 | 0x00000002
_OPEN_EXISTING = 3
_INVALID_HANDLE = ctypes.c_void_p(-1).value


class _SecurityAttributes(ctypes.Structure):
    _fields_ = [
        ("nLength", wintypes.DWORD),
        ("lpSecurityDescriptor", ctypes.c_void_p),
        ("bInheritHandle", wintypes.BOOL),
    ]


def _kernel32() -> ctypes.WinDLL:
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateFileW.argtypes = [
        wintypes.LPCWSTR,
        wintypes.DWORD,
        wintypes.DWORD,
        ctypes.POINTER(_SecurityAttributes),
        wintypes.DWORD,
        wintypes.DWORD,
        ctypes.c_void_p,
    ]
    kernel32.CreateFileW.restype = ctypes.c_void_p
    kernel32.SetConsoleCtrlHandler.argtypes = [ctypes.c_void_p, wintypes.BOOL]
    kernel32.SetConsoleCtrlHandler.restype = wintypes.BOOL
    return kernel32


def _open_console(kernel32: ctypes.WinDLL, name: str, attributes: _SecurityAttributes) -> int:
    handle = kernel32.CreateFileW(
        name,
        _GENERIC_READ | _GENERIC_WRITE,
        _FILE_SHARE_READ_WRITE,
        ctypes.byref(attributes),
        _OPEN_EXISTING,
        0,
        None,
    )
    if not handle or handle == _INVALID_HANDLE:
        raise OSError(f"could not open {name}: {ctypes.get_last_error()}")
    return int(handle)


def main() -> int:
    if len(sys.argv) != 2:
        return 2
    kernel32 = _kernel32()
    attributes = _SecurityAttributes(ctypes.sizeof(_SecurityAttributes), None, True)
    console_in = _open_console(kernel32, "CONIN$", attributes)
    console_out = _open_console(kernel32, "CONOUT$", attributes)
    # Keep waiting while the terminal delivers Ctrl+C to the whole console.
    kernel32.SetConsoleCtrlHandler(None, True)
    stdin_fd = msvcrt.open_osfhandle(console_in, os.O_RDWR)
    stdout_fd = msvcrt.open_osfhandle(console_out, os.O_RDWR)
    process = subprocess.Popen(
        sys.argv[1],
        stdin=stdin_fd,
        stdout=stdout_fd,
        stderr=stdout_fd,
        close_fds=True,
    )
    return process.wait()


if __name__ == "__main__":
    raise SystemExit(main())
