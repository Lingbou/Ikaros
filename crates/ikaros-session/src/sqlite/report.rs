// SPDX-License-Identifier: GPL-3.0-only

use crate::SessionSearchIndex;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SqliteOperationalReport {
    pub path: PathBuf,
    pub schema_version: i64,
    pub journal_mode: String,
    pub foreign_keys: bool,
    pub integrity_check: SqliteIntegrityCheckReport,
    pub write_policy: SqliteWritePolicyReport,
    pub wal_checkpoint: SqliteWalCheckpointReport,
    pub search_indexes: Vec<SqliteSearchIndexReport>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SqliteIntegrityCheckReport {
    pub ok: bool,
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SqliteBackupReport {
    pub path: PathBuf,
    pub created: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SqliteRepairReport {
    pub path: PathBuf,
    pub created: bool,
    pub integrity_check: SqliteIntegrityCheckReport,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SqliteRestoreReport {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub pre_restore_backup: Option<SqliteBackupReport>,
    pub restored: bool,
    pub integrity_check: SqliteIntegrityCheckReport,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SqlitePruneReport {
    pub ended_before: String,
    pub sessions_pruned: usize,
    pub entries_pruned: usize,
    pub agent_events_pruned: usize,
    pub approvals_pruned: usize,
    pub timeline_items_pruned: usize,
    pub continuations_pruned: usize,
    pub inputs_pruned: usize,
    pub turns_pruned: usize,
    pub search_index_rows_pruned: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SqliteWritePolicyReport {
    pub transaction_begin: &'static str,
    pub busy_timeout_ms: u64,
    pub busy_retry_attempts: u32,
    pub retry_jitter_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SqliteWalCheckpointReport {
    pub busy_frames: i64,
    pub log_frames: i64,
    pub checkpointed_frames: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SqliteSearchIndexReport {
    pub name: &'static str,
    pub index: SessionSearchIndex,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
