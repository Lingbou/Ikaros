// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    AgentEvent, AgentEventKind, ApprovalRecord, ApprovalStatus, ContinuationId, SessionBranch,
    SessionBranchSummaryInput, SessionCompactionInput, SessionContinuation,
    SessionContinuationClaim, SessionContinuationInput, SessionContinuationKind,
    SessionContinuationStatus, SessionContinuationStatusReason, SessionEntry, SessionEntryId,
    SessionEntryKind, SessionId, SessionInput, SessionInputAdmission, SessionInputId,
    SessionInputStatus, SessionRecord, SessionReplayPage, SessionRetryInput, SessionSearchHit,
    SessionSearchIndex, SessionSearchQuery, SessionSource, SessionStore, SessionTimelineItem,
    SessionTimelinePage, SessionTimelineQuery, SessionTimelineRecord, SessionTurnRecord,
    SessionTurnStatus, SessionWriter, TurnId,
};
use ikaros_core::{IkarosError, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    thread,
    time::Duration as StdDuration,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

mod approvals;
mod connection;
mod continuations;
mod convert;
mod entries;
mod events;
mod inputs;
mod replay;
mod report;
mod search;
mod sessions;
mod store;
mod timeline;
mod turns;
mod writer;

use self::{
    approvals::*, connection::*, continuations::*, convert::*, entries::*, events::*, inputs::*,
    replay::*, search::*, sessions::*, timeline::*, turns::*, writer::*,
};

pub use report::{
    SqliteBackupReport, SqliteIntegrityCheckReport, SqliteOperationalReport, SqlitePruneReport,
    SqliteRepairReport, SqliteRestoreReport, SqliteSearchIndexReport, SqliteWalCheckpointReport,
    SqliteWritePolicyReport,
};

const SESSION_SCHEMA_VERSION: i64 = 8;
const DEFAULT_CONTINUATION_LEASE_SECONDS: i64 = 300;
const SQLITE_BUSY_TIMEOUT_MS: u64 = 5_000;
const SQLITE_BUSY_RETRY_ATTEMPTS: u32 = 3;
const SQLITE_BUSY_RETRY_JITTER_MS: u64 = 25;

pub use store::SqliteSessionStore;
