"""Offline-safe inspection, backup, and projection repair for Runtime state."""

from __future__ import annotations

import hashlib
import os
import sqlite3
import tempfile
import uuid
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path

from ..json_codec import dumps as json_dumps
from .journal import journal_sequence_high_water, latest_sequence, projection_events
from .projections import apply_event
from .schema import SCHEMA_VERSION

_PROJECTION_TABLES = (
    ("threads", "id"),
    ("branches", "id"),
    ("turns", "id"),
    ("runs", "id"),
    ("run_configs", "run_id"),
    ("context_revisions", "run_id, revision"),
    ("process_sessions", "process_id"),
    ("items", "id"),
    ("model_calls", "run_id, step_ordinal"),
    ("model_usages", "run_id, step_ordinal"),
    ("file_changes", "tool_call_item_id"),
)


@dataclass(frozen=True, slots=True)
class StateCheckReport:
    state_path: Path
    latest_seq: int
    event_count: int
    thread_count: int
    turn_count: int
    item_count: int

    def to_wire(self) -> dict[str, object]:
        return {
            "statePath": str(self.state_path),
            "latestSeq": self.latest_seq,
            "eventCount": self.event_count,
            "threadCount": self.thread_count,
            "turnCount": self.turn_count,
            "itemCount": self.item_count,
        }


@dataclass(frozen=True, slots=True)
class StateBackupReport:
    backup_path: Path
    latest_seq: int
    event_count: int
    size_bytes: int

    def to_wire(self) -> dict[str, object]:
        return {
            "backupPath": str(self.backup_path),
            "latestSeq": self.latest_seq,
            "eventCount": self.event_count,
            "sizeBytes": self.size_bytes,
        }


@dataclass(frozen=True, slots=True)
class ProjectionRepairReport:
    state: StateCheckReport
    backup: StateBackupReport

    def to_wire(self) -> dict[str, object]:
        return {
            "state": self.state.to_wire(),
            "backup": self.backup.to_wire(),
        }


def check_state(connection: sqlite3.Connection, state_path: Path) -> StateCheckReport:
    """Validate SQLite, Journal replay, and the current query projections."""

    _require_idle(connection)
    _assert_database_integrity(connection)
    events = projection_events(connection)
    current_digest = _projection_digest(connection)

    shadow = sqlite3.connect(":memory:")
    shadow.row_factory = sqlite3.Row
    shadow.execute("PRAGMA foreign_keys = ON")
    try:
        connection.backup(shadow)
        rebuild_projection_tables(shadow)
        if _projection_digest(shadow) != current_digest:
            raise RuntimeError("Runtime projections do not match the Journal; repair required")
    finally:
        shadow.close()

    return _state_report(connection, state_path.resolve(), len(events))


def create_state_backup(
    connection: sqlite3.Connection,
    state_path: Path,
    destination: Path | None = None,
    *,
    require_projection_integrity: bool = True,
    expected_schema_version: int = SCHEMA_VERSION,
) -> StateBackupReport:
    """Create a verified SQLite snapshot without copying WAL sidecars directly."""

    _require_idle(connection)
    _assert_database_integrity(
        connection,
        require_foreign_keys=require_projection_integrity,
        expected_schema_version=expected_schema_version,
    )
    events = projection_events(connection)
    source_digest = _journal_digest(connection)
    backup_path = _backup_destination(state_path, destination)
    backup_path.parent.mkdir(parents=True, exist_ok=True)
    if backup_path.exists():
        raise FileExistsError(f"backup destination already exists: {backup_path}")

    with tempfile.NamedTemporaryFile(
        prefix=f".{backup_path.name}.",
        suffix=".tmp",
        dir=backup_path.parent,
        delete=False,
    ) as temporary_handle:
        temporary_path = Path(temporary_handle.name)
    destination_created = False
    published = False
    try:
        target = sqlite3.connect(temporary_path)
        target.row_factory = sqlite3.Row
        target.execute("PRAGMA foreign_keys = ON")
        try:
            connection.backup(target)
            _assert_database_integrity(
                target,
                require_foreign_keys=require_projection_integrity,
                expected_schema_version=expected_schema_version,
            )
            copied_events = projection_events(target)
            if len(copied_events) != len(events) or _journal_digest(target) != source_digest:
                raise RuntimeError("state backup does not match the source Journal")
            _prepare_standalone_database(target)
        finally:
            target.close()

        _remove_sqlite_sidecars(temporary_path)
        with temporary_path.open("r+b") as backup_file:
            os.fsync(backup_file.fileno())
        report = StateBackupReport(
            backup_path=backup_path,
            latest_seq=latest_sequence(connection),
            event_count=len(events),
            size_bytes=temporary_path.stat().st_size,
        )
        _publish_new_file(temporary_path, backup_path)
        destination_created = True
        _fsync_directory(backup_path.parent)
        published = True
        return report
    finally:
        if not published:
            if destination_created:
                backup_path.unlink(missing_ok=True)
            temporary_path.unlink(missing_ok=True)
            _remove_sqlite_sidecars(temporary_path)


def repair_state_projections(
    connection: sqlite3.Connection,
    state_path: Path,
    backup_destination: Path | None = None,
) -> ProjectionRepairReport:
    """Back up state, rebuild projections atomically, and prove Journal immutability."""

    backup = create_state_backup(
        connection,
        state_path,
        backup_destination,
        require_projection_integrity=False,
    )
    journal_before = _journal_digest(connection)
    try:
        rebuild_projection_tables(connection)
        state = check_state(connection, state_path)
        if _journal_digest(connection) != journal_before:
            raise RuntimeError("projection repair modified the Journal")
    except BaseException as error:
        raise RuntimeError(
            f"projection repair failed; backup preserved at {backup.backup_path}"
        ) from error
    return ProjectionRepairReport(state=state, backup=backup)


def rebuild_projection_tables(connection: sqlite3.Connection) -> None:
    """Rebuild all disposable query tables in one rollback-safe transaction."""

    _require_idle(connection)
    events = projection_events(connection)
    journal_before = _journal_digest(connection)
    connection.execute("BEGIN IMMEDIATE")
    try:
        for table, _key in reversed(_PROJECTION_TABLES):
            connection.execute(f"DELETE FROM {table}")
        for event in events:
            apply_event(connection, event)
        _assert_database_integrity(connection)
        if _journal_digest(connection) != journal_before:
            raise RuntimeError("projection rebuild modified the Journal")
        connection.commit()
    except BaseException:
        if connection.in_transaction:
            connection.rollback()
        raise


def _state_report(
    connection: sqlite3.Connection,
    state_path: Path,
    event_count: int,
) -> StateCheckReport:
    return StateCheckReport(
        state_path=state_path,
        latest_seq=latest_sequence(connection),
        event_count=event_count,
        thread_count=_table_count(connection, "threads"),
        turn_count=_table_count(connection, "turns"),
        item_count=_table_count(connection, "items"),
    )


def _table_count(connection: sqlite3.Connection, table: str) -> int:
    return int(connection.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0])


def _assert_database_integrity(
    connection: sqlite3.Connection,
    *,
    require_foreign_keys: bool = True,
    expected_schema_version: int = SCHEMA_VERSION,
) -> None:
    schema_version = int(connection.execute("PRAGMA user_version").fetchone()[0])
    if schema_version != expected_schema_version:
        raise RuntimeError(
            f"state database schema version {schema_version} does not match "
            f"{expected_schema_version}"
        )
    quick_check = tuple(str(row[0]) for row in connection.execute("PRAGMA quick_check"))
    if quick_check != ("ok",):
        raise RuntimeError("SQLite quick_check failed")
    if (
        require_foreign_keys
        and connection.execute("PRAGMA foreign_key_check").fetchone() is not None
    ):
        raise RuntimeError("SQLite foreign_key_check failed")


def _journal_digest(connection: sqlite3.Connection) -> bytes:
    digest = hashlib.sha256()
    rows_digest = _query_digest(
        connection,
        """
        SELECT seq, schema_version, event_type, thread_id, branch_id, turn_id,
               run_id, item_id, created_at, payload_json
        FROM events ORDER BY seq
        """,
    )
    digest.update(rows_digest)
    digest.update(journal_sequence_high_water(connection).to_bytes(8, "big"))
    return digest.digest()


def _projection_digest(connection: sqlite3.Connection) -> bytes:
    digest = hashlib.sha256()
    for table, key in _PROJECTION_TABLES:
        digest.update(table.encode("utf-8"))
        digest.update(_query_digest(connection, f"SELECT * FROM {table} ORDER BY {key}"))
    return digest.digest()


def _query_digest(connection: sqlite3.Connection, query: str) -> bytes:
    digest = hashlib.sha256()
    for row in connection.execute(query):
        encoded = json_dumps(list(row), ensure_ascii=False, separators=(",", ":")).encode()
        digest.update(len(encoded).to_bytes(8, "big"))
        digest.update(encoded)
    return digest.digest()


def _backup_destination(state_path: Path, destination: Path | None) -> Path:
    source = state_path.resolve()
    if destination is None:
        timestamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%S%fZ")
        candidate = source.parent / "backups" / f"state-{timestamp}-{uuid.uuid4().hex[:8]}.db"
    else:
        candidate = destination.expanduser().resolve()
    if candidate == source:
        raise ValueError("backup destination must differ from state.db")
    return candidate


def _require_idle(connection: sqlite3.Connection) -> None:
    if connection.in_transaction:
        raise RuntimeError("storage maintenance requires an idle SQLite connection")


def _prepare_standalone_database(connection: sqlite3.Connection) -> None:
    checkpoint = connection.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()
    if checkpoint is not None and int(checkpoint[0]) != 0:
        raise RuntimeError("SQLite backup checkpoint could not complete")
    journal_mode = str(connection.execute("PRAGMA journal_mode = DELETE").fetchone()[0])
    if journal_mode.lower() != "delete":
        raise RuntimeError("SQLite backup could not switch to standalone journal mode")


def _publish_new_file(source: Path, destination: Path) -> None:
    if os.name == "nt":
        os.rename(source, destination)
        return
    linked = False
    try:
        os.link(source, destination)
        linked = True
        source.unlink()
    except BaseException:
        if linked:
            destination.unlink(missing_ok=True)
        raise


def _remove_sqlite_sidecars(database_path: Path) -> None:
    for suffix in ("-wal", "-shm", "-journal"):
        Path(f"{database_path}{suffix}").unlink(missing_ok=True)


def _fsync_directory(path: Path) -> None:
    if os.name == "nt" or not hasattr(os, "O_DIRECTORY"):
        return
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


__all__ = [
    "ProjectionRepairReport",
    "StateBackupReport",
    "StateCheckReport",
    "check_state",
    "create_state_backup",
    "rebuild_projection_tables",
    "repair_state_projections",
]
