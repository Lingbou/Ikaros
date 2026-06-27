// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(super) fn begin_immediate_transaction(
    conn: &Connection,
    path: &Path,
    operation: &'static str,
) -> Result<()> {
    retry_sqlite_busy(path, operation, || {
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION")
    })
}

pub(super) fn commit_transaction(
    conn: &Connection,
    path: &Path,
    operation: &'static str,
) -> Result<()> {
    retry_sqlite_busy(path, operation, || conn.execute_batch("COMMIT"))
}
pub(super) fn sqlite_limit(value: usize) -> i64 {
    value.min(i64::MAX as usize) as i64
}

pub(super) fn sqlite_offset(value: usize) -> i64 {
    value.min(i64::MAX as usize) as i64
}

pub(super) fn retry_sqlite_busy<T>(
    path: &Path,
    operation: &'static str,
    mut action: impl FnMut() -> rusqlite::Result<T>,
) -> Result<T> {
    for attempt in 0..=SQLITE_BUSY_RETRY_ATTEMPTS {
        match action() {
            Ok(value) => return Ok(value),
            Err(source) if sqlite_is_busy(&source) && attempt < SQLITE_BUSY_RETRY_ATTEMPTS => {
                thread::sleep(sqlite_retry_delay(attempt));
            }
            Err(source) => return Err(sqlite_error_for_operation(path, operation, source)),
        }
    }
    Err(IkarosError::Message(format!(
        "sqlite busy error at {} during {operation}: retry attempts exhausted",
        path.display()
    )))
}

pub(super) fn sqlite_retry_delay(attempt: u32) -> StdDuration {
    let multiplier = u64::from(attempt % 3) + 1;
    StdDuration::from_millis(SQLITE_BUSY_RETRY_JITTER_MS * multiplier)
}

pub(super) fn sqlite_is_busy(source: &rusqlite::Error) -> bool {
    match source {
        rusqlite::Error::SqliteFailure(error, _) => matches!(
            error.code,
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
        ),
        _ => false,
    }
}

pub(super) fn sqlite_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}-{suffix}", path.display()))
}

pub(super) fn search_index_report(
    conn: &Connection,
    path: &Path,
    name: &'static str,
    index: SessionSearchIndex,
) -> SqliteSearchIndexReport {
    let available = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![name],
            |_| Ok(()),
        )
        .optional()
        .map(|value| value.is_some());
    match available {
        Ok(true) => {
            let probe = conn.query_row(&format!("SELECT count(*) FROM {name}"), [], |_| Ok(()));
            match probe {
                Ok(()) => SqliteSearchIndexReport {
                    name,
                    index,
                    available: true,
                    error: None,
                },
                Err(source) => SqliteSearchIndexReport {
                    name,
                    index,
                    available: false,
                    error: Some(sqlite_error(path, source).to_string()),
                },
            }
        }
        Ok(false) => SqliteSearchIndexReport {
            name,
            index,
            available: false,
            error: Some("index table is missing".into()),
        },
        Err(source) => SqliteSearchIndexReport {
            name,
            index,
            available: false,
            error: Some(sqlite_error(path, source).to_string()),
        },
    }
}

pub(super) fn integrity_check(
    conn: &Connection,
    path: &Path,
) -> Result<SqliteIntegrityCheckReport> {
    let mut stmt = conn
        .prepare("PRAGMA integrity_check")
        .map_err(|source| sqlite_error(path, source))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|source| sqlite_error(path, source))?;
    let mut messages = Vec::new();
    for row in rows {
        messages.push(row.map_err(|source| sqlite_error(path, source))?);
    }
    let ok = messages.len() == 1 && messages.first().is_some_and(|message| message == "ok");
    Ok(SqliteIntegrityCheckReport { ok, messages })
}

pub(super) fn wal_checkpoint(
    conn: &Connection,
    path: &Path,
    mode: &'static str,
) -> Result<SqliteWalCheckpointReport> {
    conn.query_row(&format!("PRAGMA wal_checkpoint({mode})"), [], |row| {
        Ok(SqliteWalCheckpointReport {
            busy_frames: row.get(0)?,
            log_frames: row.get(1)?,
            checkpointed_frames: row.get(2)?,
        })
    })
    .map_err(|source| sqlite_error(path, source))
}
