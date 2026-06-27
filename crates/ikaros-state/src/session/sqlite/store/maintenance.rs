// SPDX-License-Identifier: GPL-3.0-only

use super::*;

impl SqliteSessionStore {
    pub fn operational_report(&self) -> Result<SqliteOperationalReport> {
        let conn = self.open()?;
        let schema_version = conn
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .map_err(|source| sqlite_error(&self.path, source))?;
        let journal_mode = conn
            .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
            .map_err(|source| sqlite_error(&self.path, source))?
            .to_lowercase();
        let foreign_keys = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get::<_, i64>(0))
            .map_err(|source| sqlite_error(&self.path, source))?
            != 0;
        let wal_checkpoint = wal_checkpoint(&conn, &self.path, "PASSIVE")?;
        Ok(SqliteOperationalReport {
            path: self.path.clone(),
            schema_version,
            journal_mode,
            foreign_keys,
            integrity_check: integrity_check(&conn, &self.path)?,
            write_policy: SqliteWritePolicyReport {
                transaction_begin: "BEGIN IMMEDIATE",
                busy_timeout_ms: SQLITE_BUSY_TIMEOUT_MS,
                busy_retry_attempts: SQLITE_BUSY_RETRY_ATTEMPTS,
                retry_jitter_ms: SQLITE_BUSY_RETRY_JITTER_MS,
            },
            wal_checkpoint,
            search_indexes: vec![
                search_index_report(
                    &conn,
                    &self.path,
                    "session_entries_fts",
                    SessionSearchIndex::Fts,
                ),
                search_index_report(
                    &conn,
                    &self.path,
                    "session_entries_trigram",
                    SessionSearchIndex::Trigram,
                ),
            ],
        })
    }

    pub fn checkpoint_wal(&self) -> Result<SqliteWalCheckpointReport> {
        let conn = self.open()?;
        wal_checkpoint(&conn, &self.path, "TRUNCATE")
    }

    pub fn vacuum(&self) -> Result<()> {
        let conn = self.open()?;
        retry_sqlite_busy(&self.path, "vacuum", || conn.execute_batch("VACUUM"))
    }

    pub fn backup_to(&self, destination: impl AsRef<Path>) -> Result<SqliteBackupReport> {
        let destination = destination.as_ref();
        if destination == self.path {
            return Err(IkarosError::Message(format!(
                "state.db backup destination must differ from source: {}",
                destination.display()
            )));
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let conn = self.open()?;
        retry_sqlite_busy(&self.path, "backup", || {
            conn.execute(
                "VACUUM INTO ?1",
                params![destination.to_string_lossy().as_ref()],
            )
        })?;
        Ok(SqliteBackupReport {
            path: destination.to_path_buf(),
            created: destination.is_file(),
        })
    }

    pub fn repair_to(&self, destination: impl AsRef<Path>) -> Result<SqliteRepairReport> {
        let backup = self.backup_to(destination)?;
        let repaired = SqliteSessionStore::from_file(backup.path.clone());
        let report = repaired.operational_report()?;
        Ok(SqliteRepairReport {
            path: backup.path,
            created: backup.created,
            integrity_check: report.integrity_check,
        })
    }

    pub fn restore_from(&self, source: impl AsRef<Path>) -> Result<SqliteRestoreReport> {
        let source = source.as_ref();
        if source == self.path {
            return Err(IkarosError::Message(format!(
                "state.db restore source must differ from destination: {}",
                source.display()
            )));
        }
        if !source.is_file() {
            return Err(IkarosError::Message(format!(
                "state.db restore source is not a file: {}",
                source.display()
            )));
        }
        let source_report = SqliteSessionStore::from_file(source).operational_report()?;
        if !source_report.integrity_check.ok {
            return Err(IkarosError::Message(format!(
                "state.db restore source failed integrity check: {}",
                source.display()
            )));
        }
        let pre_restore_backup = if self.path.is_file() {
            Some(self.backup_to(self.pre_restore_backup_path())?)
        } else {
            None
        };
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let temp_path = self.restore_temp_path();
        fs::copy(source, &temp_path).map_err(|source| IkarosError::io(&temp_path, source))?;
        self.remove_sidecar_files()?;
        if self.path.exists() {
            fs::remove_file(&self.path).map_err(|source| IkarosError::io(&self.path, source))?;
        }
        fs::rename(&temp_path, &self.path).map_err(|source| IkarosError::io(&self.path, source))?;
        let restored_report = self.operational_report()?;
        Ok(SqliteRestoreReport {
            source: source.to_path_buf(),
            destination: self.path.clone(),
            pre_restore_backup,
            restored: restored_report.integrity_check.ok,
            integrity_check: restored_report.integrity_check,
        })
    }

    pub fn prune_ended_sessions_before(&self, cutoff: OffsetDateTime) -> Result<SqlitePruneReport> {
        self.with_write_transaction("prune_ended_sessions", |conn| {
            prune_ended_sessions_before(conn, &self.path, cutoff)
        })
    }
}
