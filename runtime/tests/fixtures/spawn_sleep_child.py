from __future__ import annotations

import subprocess
import sys
from pathlib import Path


def main() -> int:
    child = subprocess.Popen(
        [
            "powershell.exe",
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 60",
        ],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        close_fds=True,
        creationflags=subprocess.CREATE_NO_WINDOW,
    )
    Path(sys.argv[1]).write_text(str(child.pid), encoding="utf-8")
    if sys.argv[2] == "wait":
        return child.wait()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
