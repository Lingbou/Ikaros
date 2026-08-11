from __future__ import annotations

import asyncio
import sys
from pathlib import Path

import pytest

from ikaros_runtime.runtime_lock import RuntimeHomeLock, RuntimeHomeLockError


@pytest.mark.asyncio
async def test_runtime_home_lock_times_out_and_release_is_idempotent(tmp_path: Path) -> None:
    first = await RuntimeHomeLock.acquire(tmp_path, timeout_seconds=0.5)
    lock_path = tmp_path / "runtime.lock"

    assert first.path == lock_path
    assert lock_path.exists()
    assert first.released is False

    with pytest.raises(RuntimeHomeLockError, match="already in use"):
        await RuntimeHomeLock.acquire(tmp_path, timeout_seconds=0.1)

    first.release()
    first.release()
    assert first.released is True

    second = await RuntimeHomeLock.acquire(tmp_path, timeout_seconds=0.5)
    second.release()

    assert lock_path.exists()


@pytest.mark.asyncio
async def test_runtime_home_lock_is_released_when_owner_process_exits(tmp_path: Path) -> None:
    child_source = """
import asyncio
import sys
from pathlib import Path

from ikaros_runtime.runtime_lock import RuntimeHomeLock


async def main() -> None:
    lock = await RuntimeHomeLock.acquire(Path(sys.argv[1]), timeout_seconds=2)
    print("locked", flush=True)
    await asyncio.Event().wait()
    lock.release()


asyncio.run(main())
"""
    process = await asyncio.create_subprocess_exec(
        sys.executable,
        "-c",
        child_source,
        str(tmp_path),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    try:
        assert process.stdout is not None
        ready = await asyncio.wait_for(process.stdout.readline(), timeout=5)
        if ready.strip() != b"locked":
            if process.returncode is None:
                process.kill()
                await process.wait()
            assert process.stderr is not None
            stderr = (await process.stderr.read()).decode(errors="replace")
            pytest.fail(f"lock owner did not start: {stderr}")

        with pytest.raises(RuntimeHomeLockError, match="already in use"):
            await RuntimeHomeLock.acquire(tmp_path, timeout_seconds=0.1)

        process.kill()
        await asyncio.wait_for(process.wait(), timeout=5)

        recovered = await RuntimeHomeLock.acquire(tmp_path, timeout_seconds=2)
        recovered.release()
        assert (tmp_path / "runtime.lock").exists()
    finally:
        if process.returncode is None:
            process.kill()
            await process.wait()
