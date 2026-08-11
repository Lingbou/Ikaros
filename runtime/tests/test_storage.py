from __future__ import annotations

from pathlib import Path

from ikaros_runtime.storage import SqliteRuntimeStore


def test_projections_can_be_rebuilt_from_the_event_journal(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        expected, event = store.create_thread("Rebuild me")
        assert event.seq == 1

        store.rebuild_projections()

        assert store.list_threads() == [expected]
        replayed, latest_seq = store.replay_events(0, 100)
        assert replayed == [event]
        assert latest_seq == 1
    finally:
        store.close()
