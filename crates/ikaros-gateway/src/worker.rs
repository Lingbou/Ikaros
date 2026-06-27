// SPDX-License-Identifier: GPL-3.0-only
//! Runtime-free worker metadata and daemon control files for the local gateway.

use crate::LocalGatewayStore;
use ikaros_core::{IkarosError, Result, contains_secret_like, redact_json, redact_secrets};
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub const MESSAGE_WORKER_LOCK_FILE: &str = "message-worker.lock";
pub const MESSAGE_WORKER_EVENTS_FILE: &str = "message-worker-events.jsonl";
pub const MESSAGE_WORKER_STOP_FILE: &str = "message-worker.stop";

#[derive(Debug, Clone)]
pub struct MessageWorkerStaleLockRecovery {
    pub lock_path: PathBuf,
    pub archived_path: PathBuf,
    pub owner: String,
}

pub struct MessageWorkerLock {
    path: PathBuf,
    body: String,
    stale_recovery: Option<MessageWorkerStaleLockRecovery>,
}

impl MessageWorkerLock {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn stale_recovery(&self) -> Option<&MessageWorkerStaleLockRecovery> {
        self.stale_recovery.as_ref()
    }
}

impl Drop for MessageWorkerLock {
    fn drop(&mut self) {
        let Ok(current) = fs::read_to_string(&self.path) else {
            return;
        };
        if current == self.body {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn acquire_message_worker_lock(gateway_dir: impl AsRef<Path>) -> Result<MessageWorkerLock> {
    let gateway_dir = gateway_dir.as_ref();
    fs::create_dir_all(gateway_dir).map_err(|source| IkarosError::io(gateway_dir, source))?;
    let path = gateway_dir.join(MESSAGE_WORKER_LOCK_FILE);
    let started_at = timestamp_now();
    let body = format!("pid={}\nstarted_at={started_at}\n", std::process::id());
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            if let Err(error) = file.write_all(body.as_bytes()) {
                let _ = fs::remove_file(&path);
                return Err(IkarosError::io(&path, error));
            }
            Ok(MessageWorkerLock {
                path,
                body,
                stale_recovery: None,
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let current = fs::read_to_string(&path)
                .unwrap_or_else(|read_error| format!("unreadable: {read_error}"));
            if message_worker_lock_is_stale(&current) {
                let archived = path.with_file_name(format!(
                    "{MESSAGE_WORKER_LOCK_FILE}.stale.{}",
                    OffsetDateTime::now_utc().unix_timestamp_nanos()
                ));
                fs::rename(&path, &archived).map_err(|source| {
                    IkarosError::Message(format!(
                        "failed to archive stale message worker lock {}: {source}",
                        path.display()
                    ))
                })?;
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|source| {
                        IkarosError::Message(format!(
                            "failed to create {} after stale lock recovery: {source}",
                            path.display()
                        ))
                    })?;
                if let Err(error) = file.write_all(body.as_bytes()) {
                    let _ = fs::remove_file(&path);
                    return Err(IkarosError::io(&path, error));
                }
                return Ok(MessageWorkerLock {
                    stale_recovery: Some(MessageWorkerStaleLockRecovery {
                        lock_path: path.clone(),
                        archived_path: archived,
                        owner: redacted_message_worker_lock_owner(&current),
                    }),
                    path,
                    body,
                });
            }
            let current = redacted_message_worker_lock_owner(&current);
            Err(IkarosError::Message(format!(
                "message worker already running: lock={} owner={}",
                path.display(),
                current
            )))
        }
        Err(error) => Err(IkarosError::io(&path, error)),
    }
}

pub fn message_worker_lock_is_stale(contents: &str) -> bool {
    let Some(pid) = contents
        .lines()
        .find_map(|line| line.trim().strip_prefix("pid="))
        .and_then(|pid| pid.parse::<u32>().ok())
    else {
        return false;
    };
    !pid_is_running(pid)
}

pub fn message_worker_lock_is_stale_label(contents: &str) -> &'static str {
    if message_worker_lock_is_stale(contents) {
        "true"
    } else {
        "false"
    }
}

pub fn pid_is_running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        true
    }
}

pub struct MessageWorkerForensics {
    path: PathBuf,
    run_id: String,
    finished: bool,
}

impl MessageWorkerForensics {
    pub fn start(gateway_dir: impl AsRef<Path>, limit: usize, once: bool) -> Result<Self> {
        let gateway_dir = gateway_dir.as_ref();
        fs::create_dir_all(gateway_dir).map_err(|source| IkarosError::io(gateway_dir, source))?;
        let path = gateway_dir.join(MESSAGE_WORKER_EVENTS_FILE);
        let run_id = format!(
            "{}-{}",
            std::process::id(),
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        );
        append_message_worker_event(
            &path,
            serde_json::json!({
                "schema": "ikaros-message-worker-forensics-v1",
                "version": 1,
                "run_id": run_id,
                "event": "started",
                "status": "running",
                "at": timestamp_now(),
                "pid": std::process::id(),
                "limit": limit,
                "once": once,
            }),
        )?;
        Ok(Self {
            path,
            run_id,
            finished: false,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn finish(&mut self, status: &str, reason: &str) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        append_message_worker_event(
            &self.path,
            serde_json::json!({
                "schema": "ikaros-message-worker-forensics-v1",
                "version": 1,
                "run_id": self.run_id,
                "event": "stopped",
                "status": status,
                "at": timestamp_now(),
                "pid": std::process::id(),
                "reason": redact_secrets(reason),
            }),
        )?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for MessageWorkerForensics {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let _ = append_message_worker_event(
            &self.path,
            serde_json::json!({
                "schema": "ikaros-message-worker-forensics-v1",
                "version": 1,
                "run_id": self.run_id,
                "event": "stopped",
                "status": "aborted",
                "at": timestamp_now(),
                "pid": std::process::id(),
                "reason": "dropped_before_finish",
            }),
        );
    }
}

pub fn append_message_worker_event(path: &Path, value: Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
    }
    let value = redact_json(value);
    let line = serde_json::to_string(&value)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| IkarosError::io(path, source))?;
    writeln!(file, "{line}").map_err(|source| IkarosError::io(path, source))?;
    Ok(())
}

pub fn write_message_worker_stop_request(
    reason: &str,
    gateway_dir: impl AsRef<Path>,
) -> Result<Value> {
    let gateway_dir = gateway_dir.as_ref();
    fs::create_dir_all(gateway_dir).map_err(|source| IkarosError::io(gateway_dir, source))?;
    let path = gateway_dir.join(MESSAGE_WORKER_STOP_FILE);
    let payload = redact_json(serde_json::json!({
        "schema": "ikaros-message-worker-stop-v1",
        "version": 1,
        "at": timestamp_now(),
        "pid": std::process::id(),
        "reason": reason,
    }));
    let encoded = serde_json::to_string(&payload)?;
    fs::write(&path, format!("{encoded}\n")).map_err(|source| IkarosError::io(&path, source))?;
    Ok(payload)
}

pub fn clear_message_worker_stop_request(gateway_dir: impl AsRef<Path>) -> Result<()> {
    let path = gateway_dir.as_ref().join(MESSAGE_WORKER_STOP_FILE);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(IkarosError::io(path, error)),
    }
}

pub fn take_message_worker_stop_request(gateway_dir: impl AsRef<Path>) -> Result<Option<String>> {
    let path = gateway_dir.as_ref().join(MESSAGE_WORKER_STOP_FILE);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(IkarosError::io(&path, error)),
    };
    fs::remove_file(&path).map_err(|source| IkarosError::io(&path, source))?;
    let reason = serde_json::from_str::<Value>(&contents)
        .ok()
        .and_then(|value| {
            value
                .get("reason")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "stop requested".into());
    Ok(Some(reason))
}

pub fn message_daemon_status_label(store: &LocalGatewayStore) -> &'static str {
    let lock_path = gateway_worker_lock_path(store);
    match fs::read_to_string(&lock_path) {
        Ok(owner) => {
            if message_worker_lock_is_stale(&owner) {
                "stale"
            } else {
                "running"
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if gateway_worker_stop_path(store).exists() {
                "stopping"
            } else {
                "stopped"
            }
        }
        Err(_) => "unknown",
    }
}

pub fn gateway_worker_lock_path(store: &LocalGatewayStore) -> PathBuf {
    store
        .inbox_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(MESSAGE_WORKER_LOCK_FILE)
}

pub fn gateway_worker_stop_path(store: &LocalGatewayStore) -> PathBuf {
    store
        .inbox_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(MESSAGE_WORKER_STOP_FILE)
}

pub fn gateway_worker_events_path(store: &LocalGatewayStore) -> PathBuf {
    store
        .inbox_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(MESSAGE_WORKER_EVENTS_FILE)
}

pub fn message_daemon_log_path(gateway_dir: impl AsRef<Path>) -> PathBuf {
    gateway_dir.as_ref().join("message-worker-daemon.log")
}

pub fn redacted_message_worker_lock_owner(owner: &str) -> String {
    owner
        .lines()
        .map(|line| match line.split_once('=') {
            Some((key, value)) if contains_secret_like(value) => {
                format!("{}=[REDACTED_SECRET]", redact_secrets(key))
            }
            _ => redact_secrets(line),
        })
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_owned()
}

pub fn latest_nonempty_line(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()?
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(ToOwned::to_owned)
}

pub fn redacted_json_field(value: &Value, key: &str) -> String {
    let Some(value) = value.get(key) else {
        return "none".into();
    };
    let text = match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    redact_secrets(&text.replace(['\n', '\r'], " "))
}

fn timestamp_now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_worker_forensics_records_failed_stop_with_redacted_reason() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut forensics =
            MessageWorkerForensics::start(temp.path(), 3, true).expect("start forensics");

        forensics
            .finish(
                "failed",
                "provider failed token=abc123 api_key=plain-secret",
            )
            .expect("finish forensics");

        let events =
            fs::read_to_string(temp.path().join(MESSAGE_WORKER_EVENTS_FILE)).expect("events");
        assert!(events.contains("\"event\":\"started\""));
        assert!(events.contains("\"event\":\"stopped\""));
        assert!(events.contains("\"status\":\"failed\""));
        assert!(events.contains("provider failed token=[REDACTED_SECRET]"));
        assert!(events.contains("api_key=[REDACTED_SECRET]"));
        assert!(!events.contains("abc123"));
        assert!(!events.contains("plain-secret"));
    }
}
