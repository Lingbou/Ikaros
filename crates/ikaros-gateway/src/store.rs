// SPDX-License-Identifier: GPL-3.0-only

use crate::{GatewayDelivery, GatewayMessage, GatewayMessageStatus, GatewayRoute};
use ikaros_core::{IkarosError, Result, now_rfc3339, redact_secrets};
use std::{
    fs::{self, OpenOptions},
    io::ErrorKind,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration as StdDuration, Instant, SystemTime},
};
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

const PROCESSING_CLAIM_TIMEOUT: Duration = Duration::minutes(15);
const LOCK_ACQUIRE_TIMEOUT: StdDuration = StdDuration::from_secs(30);
const LOCK_RETRY_INTERVAL: StdDuration = StdDuration::from_millis(25);
const LOCK_STALE_TIMEOUT: StdDuration = StdDuration::from_secs(60);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct LockMetadata {
    pid: u32,
    acquired_at: String,
}

#[derive(Debug, Clone)]
pub struct LocalGatewayStore {
    inbox_path: PathBuf,
    outbox_path: PathBuf,
}

impl LocalGatewayStore {
    pub fn new(gateway_dir: impl Into<PathBuf>) -> Self {
        let gateway_dir = gateway_dir.into();
        Self {
            inbox_path: gateway_dir.join("inbox.jsonl"),
            outbox_path: gateway_dir.join("outbox.jsonl"),
        }
    }

    pub fn from_files(inbox_path: impl Into<PathBuf>, outbox_path: impl Into<PathBuf>) -> Self {
        Self {
            inbox_path: inbox_path.into(),
            outbox_path: outbox_path.into(),
        }
    }

    pub fn inbox_path(&self) -> &Path {
        &self.inbox_path
    }

    pub fn outbox_path(&self) -> &Path {
        &self.outbox_path
    }

    pub fn enqueue(&self, route: GatewayRoute) -> Result<GatewayMessage> {
        with_jsonl_lock(&self.inbox_path, || {
            let mut messages: Vec<GatewayMessage> = read_jsonl(&self.inbox_path)?;
            if let Some(digest) = route.idempotency_key_digest.as_deref() {
                if let Some(existing) = messages
                    .iter()
                    .find(|message| idempotency_digest_matches(message, digest, &route))
                {
                    return Ok(existing.clone());
                }
            } else if let Some(key) = route.idempotency_key.as_deref() {
                if let Some(existing) = messages
                    .iter()
                    .find(|message| message.idempotency_key.as_deref() == Some(key))
                {
                    return Ok(existing.clone());
                }
            }
            let message = GatewayMessage::new(route)?;
            messages.push(message.clone());
            write_jsonl(&self.inbox_path, &messages)?;
            Ok(message)
        })
    }

    pub fn list(&self) -> Result<Vec<GatewayMessage>> {
        with_jsonl_lock(&self.inbox_path, || {
            let mut messages = read_jsonl(&self.inbox_path)?;
            sort_messages(&mut messages);
            Ok(messages)
        })
    }

    pub fn pending(&self, limit: usize) -> Result<Vec<GatewayMessage>> {
        with_jsonl_lock(&self.inbox_path, || {
            let mut messages = self
                .read_messages()?
                .into_iter()
                .filter(|message| message.status == GatewayMessageStatus::Pending)
                .collect::<Vec<_>>();
            sort_messages(&mut messages);
            messages.truncate(limit);
            Ok(messages)
        })
    }

    pub fn claim_pending(&self, limit: usize) -> Result<Vec<GatewayMessage>> {
        with_jsonl_lock(&self.inbox_path, || {
            let mut messages = self.read_messages()?;
            sort_messages(&mut messages);
            let now_at = OffsetDateTime::now_utc();
            let now = now_rfc3339()?;
            let mut claimed = Vec::new();
            for message in &mut messages {
                if claimed.len() >= limit {
                    break;
                }
                if claimable_message(message, now_at) {
                    message.status = GatewayMessageStatus::Processing;
                    message.updated_at = now.clone();
                    claimed.push(message.clone());
                }
            }
            if !claimed.is_empty() {
                write_jsonl(&self.inbox_path, &messages)?;
            }
            Ok(claimed)
        })
    }

    pub fn record_status(
        &self,
        id: &str,
        status: GatewayMessageStatus,
        summary: impl Into<String>,
    ) -> Result<Option<GatewayMessage>> {
        with_jsonl_lock(&self.inbox_path, || {
            let mut messages = self.read_messages()?;
            let now = now_rfc3339()?;
            let summary = redact_secrets(&summary.into());
            let mut updated = None;
            for message in &mut messages {
                if message.id == id {
                    message.status = status.clone();
                    message.summary = Some(summary.clone());
                    message.processed_at = Some(now.clone());
                    message.updated_at = now.clone();
                    updated = Some(message.clone());
                    break;
                }
            }
            self.write_messages(&messages)?;
            Ok(updated)
        })
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        with_jsonl_lock(&self.inbox_path, || {
            let messages = self.read_messages()?;
            let before = messages.len();
            let retained = messages
                .into_iter()
                .filter(|message| message.id != id)
                .collect::<Vec<_>>();
            self.write_messages(&retained)?;
            Ok(retained.len() != before)
        })
    }

    pub fn deliver(
        &self,
        message_id: impl Into<String>,
        kind: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<GatewayDelivery> {
        with_jsonl_lock(&self.outbox_path, || {
            let mut deliveries = self.read_deliveries()?;
            let delivery = GatewayDelivery::new(message_id, kind, content)?;
            deliveries.push(delivery.clone());
            self.write_deliveries(&deliveries)?;
            Ok(delivery)
        })
    }

    pub fn deliveries(&self) -> Result<Vec<GatewayDelivery>> {
        with_jsonl_lock(&self.outbox_path, || {
            let mut deliveries = self.read_deliveries()?;
            sort_deliveries(&mut deliveries);
            Ok(deliveries)
        })
    }

    fn read_messages(&self) -> Result<Vec<GatewayMessage>> {
        read_jsonl(&self.inbox_path)
    }

    fn write_messages(&self, messages: &[GatewayMessage]) -> Result<()> {
        write_jsonl(&self.inbox_path, messages)
    }

    fn read_deliveries(&self) -> Result<Vec<GatewayDelivery>> {
        read_jsonl(&self.outbox_path)
    }

    fn write_deliveries(&self, deliveries: &[GatewayDelivery]) -> Result<()> {
        write_jsonl(&self.outbox_path, deliveries)
    }
}

fn idempotency_digest_matches(
    message: &GatewayMessage,
    digest: &str,
    route: &GatewayRoute,
) -> bool {
    if message.idempotency_key_digest.as_deref() == Some(digest) {
        return true;
    }
    message.idempotency_key_digest.is_none()
        && route
            .idempotency_key
            .as_deref()
            .is_some_and(|key| !key.contains("[REDACTED_SECRET]"))
        && message.idempotency_key == route.idempotency_key
}

fn read_jsonl<T>(path: &Path) -> Result<Vec<T>>
where
    T: serde::de::DeserializeOwned,
{
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path).map_err(|source| IkarosError::io(path, source))?;
    let reader = BufReader::new(file);
    let mut items = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|source| IkarosError::io(path, source))?;
        if !line.trim().is_empty() {
            items.push(serde_json::from_str(&line)?);
        }
    }
    Ok(items)
}

fn write_jsonl<T>(path: &Path, items: &[T]) -> Result<()>
where
    T: serde::Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
    }
    let temp_path = temp_jsonl_path(path);
    let mut file = OpenOptions::new()
        .create(true)
        .create_new(true)
        .truncate(false)
        .write(true)
        .open(&temp_path)
        .map_err(|source| IkarosError::io(&temp_path, source))?;
    let write_result = (|| -> Result<()> {
        for item in items {
            writeln!(file, "{}", serde_json::to_string(item)?)
                .map_err(|source| IkarosError::io(&temp_path, source))?;
        }
        file.sync_all()
            .map_err(|source| IkarosError::io(&temp_path, source))?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    fs::rename(&temp_path, path).map_err(|source| IkarosError::io(path, source))?;
    Ok(())
}

struct JsonlFileLock {
    path: PathBuf,
}

impl JsonlFileLock {
    fn acquire(path: &Path) -> Result<Self> {
        let lock_path = sibling_path_with_suffix(path, ".lock");
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let started = Instant::now();
        loop {
            match OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&lock_path)
            {
                Ok(mut file) => {
                    let meta = LockMetadata {
                        pid: std::process::id(),
                        acquired_at: now_rfc3339()?,
                    };
                    let payload = serde_json::to_vec(&meta).map_err(|source| {
                        IkarosError::Message(format!(
                            "failed to serialize gateway lock metadata: {source}"
                        ))
                    })?;
                    let write_result = file
                        .write_all(&payload)
                        .and_then(|()| file.sync_all())
                        .map_err(|source| IkarosError::io(&lock_path, source));
                    if let Err(error) = write_result {
                        drop(file);
                        let _ = fs::remove_file(&lock_path);
                        return Err(error);
                    }
                    drop(file);
                    return Ok(Self { path: lock_path });
                }
                Err(source) if is_lock_contention(source.kind()) => {
                    if source.kind() == ErrorKind::AlreadyExists && lock_is_stale(&lock_path)? {
                        let stale_path = sibling_path_with_suffix(
                            path,
                            &format!(".lock.stale.{}", Uuid::new_v4()),
                        );
                        match fs::rename(&lock_path, &stale_path) {
                            Ok(()) => continue,
                            Err(error) if error.kind() == ErrorKind::NotFound => continue,
                            Err(error) if is_lock_contention(error.kind()) => {
                                // Another contender is renaming at the same time;
                                // re-check on the next iteration whether the lock
                                // is still stale after they finish.
                            }
                            Err(error) => return Err(IkarosError::io(&lock_path, error)),
                        }
                    }
                    if started.elapsed() >= LOCK_ACQUIRE_TIMEOUT {
                        return Err(IkarosError::Message(format!(
                            "timed out locking gateway store {}",
                            lock_path.display()
                        )));
                    }
                    thread::sleep(LOCK_RETRY_INTERVAL);
                }
                Err(source) => return Err(IkarosError::io(&lock_path, source)),
            }
        }
    }
}

impl Drop for JsonlFileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn with_jsonl_lock<T>(path: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let _lock = JsonlFileLock::acquire(path)?;
    f()
}

fn lock_is_stale(path: &Path) -> Result<bool> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) if is_lock_contention(error.kind()) => return Ok(false),
        Err(error) => return Err(IkarosError::io(path, error)),
    };
    let Ok(modified) = metadata.modified() else {
        return Ok(false);
    };
    Ok(SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|age| age >= LOCK_STALE_TIMEOUT))
}

fn is_lock_contention(kind: ErrorKind) -> bool {
    kind == ErrorKind::AlreadyExists || (cfg!(windows) && kind == ErrorKind::PermissionDenied)
}

fn temp_jsonl_path(path: &Path) -> PathBuf {
    sibling_path_with_suffix(path, &format!(".tmp.{}", Uuid::new_v4()))
}

fn sibling_path_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "gateway.jsonl".into());
    path.with_file_name(format!("{file_name}{suffix}"))
}

fn sort_messages(messages: &mut [GatewayMessage]) {
    messages.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn sort_deliveries(deliveries: &mut [GatewayDelivery]) {
    deliveries.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn claimable_message(message: &GatewayMessage, now: OffsetDateTime) -> bool {
    match message.status {
        GatewayMessageStatus::Pending => true,
        GatewayMessageStatus::Processing => processing_claim_expired(message, now),
        GatewayMessageStatus::Processed | GatewayMessageStatus::Failed => false,
    }
}

fn processing_claim_expired(message: &GatewayMessage, now: OffsetDateTime) -> bool {
    OffsetDateTime::parse(&message.updated_at, &Rfc3339)
        .map(|updated_at| now - updated_at >= PROCESSING_CLAIM_TIMEOUT)
        .unwrap_or(false)
}
