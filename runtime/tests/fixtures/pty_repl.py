"""A tiny line-oriented REPL used by PTY tests: echoes input and reports its size."""

from __future__ import annotations

import os
import sys


def _size() -> str:
    try:
        columns, rows = os.get_terminal_size(sys.stdout.fileno())
    except OSError as error:
        return f"size-error:{error}"
    return f"size:{columns}x{rows}"


def main() -> int:
    print("repl-ready", flush=True)
    for line in sys.stdin:
        command = line.strip()
        if command == "exit":
            print("repl-exit", flush=True)
            return 0
        if command == "size":
            print(_size(), flush=True)
        else:
            print(f"repl-echo:{command}", flush=True)
    print("repl-eof", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
