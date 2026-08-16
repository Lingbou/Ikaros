from __future__ import annotations

from pathlib import Path

from ikaros_runtime.paths import RuntimePaths


def test_runtime_paths_keep_session_memory_config_and_skills_separate(
    tmp_path: Path,
) -> None:
    paths = RuntimePaths.from_home(tmp_path / "nested" / ".." / "runtime-home")

    assert paths.home == (tmp_path / "runtime-home").resolve()
    assert paths.config == paths.home / "config.yaml"
    assert paths.state_db == paths.home / "state.db"
    assert paths.memory_db == paths.home / "memory.db"
    assert paths.skills == paths.home / "skills"
    assert len({paths.config, paths.state_db, paths.memory_db, paths.skills}) == 4
