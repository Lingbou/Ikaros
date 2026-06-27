// SPDX-License-Identifier: GPL-3.0-only

use fs4::FileExt;
use ikaros_core::{IkarosError, Result, now_rfc3339};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::ErrorKind,
    io::{BufRead, BufReader, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration as StdDuration, Instant},
};
use uuid::Uuid;

const LOCK_ACQUIRE_TIMEOUT: StdDuration = StdDuration::from_secs(30);
const LOCK_RETRY_INTERVAL: StdDuration = StdDuration::from_millis(25);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct LockMetadata {
    pid: u32,
    acquired_at: String,
}

pub(super) fn read_jsonl<T>(path: &Path) -> Result<Vec<T>>
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

pub(super) fn write_jsonl<T>(path: &Path, items: &[T]) -> Result<()>
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
    file: fs::File,
    _process_guard: ProcessFileLock,
}

struct ProcessFileLock {
    key: PathBuf,
}

impl JsonlFileLock {
    fn acquire(path: &Path) -> Result<Self> {
        Self::acquire_with_timeout(path, LOCK_ACQUIRE_TIMEOUT)
    }

    fn acquire_with_timeout(path: &Path, timeout: StdDuration) -> Result<Self> {
        let lock_path = sibling_path_with_suffix(path, ".lock");
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let process_guard = ProcessFileLock::acquire(lock_identity(&lock_path), timeout)?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|source| IkarosError::io(&lock_path, source))?;
        let started = Instant::now();
        loop {
            match FileExt::try_lock(&file).map_err(std::io::Error::from) {
                Ok(()) => {
                    if let Err(error) = write_lock_metadata(&mut file, &lock_path) {
                        let _ = FileExt::unlock(&file);
                        return Err(error);
                    }
                    return Ok(Self {
                        file,
                        _process_guard: process_guard,
                    });
                }
                Err(source) if is_lock_contention(source.kind()) => {
                    if started.elapsed() >= timeout {
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
        let _ = FileExt::unlock(&self.file);
    }
}

pub(super) fn with_jsonl_lock<T>(path: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let _lock = JsonlFileLock::acquire(path)?;
    f()
}

impl ProcessFileLock {
    fn acquire(key: PathBuf, timeout: StdDuration) -> Result<Self> {
        let started = Instant::now();
        loop {
            {
                let mut locked_paths = process_locks().lock().map_err(|_| {
                    IkarosError::Message("gateway lock registry is poisoned".into())
                })?;
                if locked_paths.insert(key.clone()) {
                    return Ok(Self { key });
                }
            }
            if started.elapsed() >= timeout {
                return Err(IkarosError::Message(format!(
                    "timed out locking gateway store {}",
                    key.display()
                )));
            }
            thread::sleep(LOCK_RETRY_INTERVAL);
        }
    }
}

impl Drop for ProcessFileLock {
    fn drop(&mut self) {
        if let Ok(mut locked_paths) = process_locks().lock() {
            locked_paths.remove(&self.key);
        }
    }
}

fn process_locks() -> &'static Mutex<HashSet<PathBuf>> {
    static LOCKS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn lock_identity(lock_path: &Path) -> PathBuf {
    let Some(parent) = lock_path.parent() else {
        return lock_path.to_path_buf();
    };
    let parent = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    match lock_path.file_name() {
        Some(file_name) => parent.join(file_name),
        None => parent,
    }
}

fn write_lock_metadata(file: &mut fs::File, path: &Path) -> Result<()> {
    let meta = LockMetadata {
        pid: std::process::id(),
        acquired_at: now_rfc3339()?,
    };
    let payload = serde_json::to_vec(&meta).map_err(|source| {
        IkarosError::Message(format!(
            "failed to serialize gateway lock metadata: {source}"
        ))
    })?;
    file.set_len(0)
        .and_then(|()| file.seek(SeekFrom::Start(0)).map(|_| ()))
        .and_then(|()| file.write_all(&payload))
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .map_err(|source| IkarosError::io(path, source))
}

fn is_lock_contention(kind: ErrorKind) -> bool {
    kind == ErrorKind::WouldBlock
        || kind == ErrorKind::AlreadyExists
        || kind == ErrorKind::Interrupted
        || (cfg!(windows) && kind == ErrorKind::PermissionDenied)
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

#[cfg(test)]
mod lock_tests {
    use super::*;

    #[test]
    fn jsonl_lock_blocks_same_process_takeover_until_release() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("inbox.jsonl");
        let first = JsonlFileLock::acquire_with_timeout(&path, StdDuration::from_secs(1))
            .expect("first lock");
        let lock_path = sibling_path_with_suffix(&path, ".lock");

        let error = match JsonlFileLock::acquire_with_timeout(&path, StdDuration::from_millis(30)) {
            Ok(_) => panic!("second lock acquired while first lock was held"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("timed out locking gateway store")
        );

        drop(first);
        let metadata = fs::read_to_string(&lock_path).expect("lock metadata");
        let metadata: LockMetadata = serde_json::from_str(&metadata).expect("metadata json");
        assert_eq!(metadata.pid, std::process::id());
        let _second = JsonlFileLock::acquire_with_timeout(&path, StdDuration::from_secs(1))
            .expect("second lock after release");
    }
}
