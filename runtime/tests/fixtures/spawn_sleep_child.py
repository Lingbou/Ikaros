from __future__ import annotations

import subprocess
import sys
from pathlib import Path


def main() -> int:
    if sys.platform != "win32":
        raise RuntimeError("this fixture starts a hidden Windows process")

    pythonw = Path(sys.executable).with_name("pythonw.exe")
    child_executable = pythonw if pythonw.is_file() else Path(sys.executable)
    child = subprocess.Popen(
        [
            str(child_executable),
            "-c",
            "import time; time.sleep(60)",
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
