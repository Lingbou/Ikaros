// SPDX-License-Identifier: GPL-3.0-only

use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use ikaros_core::{IkarosError, Result};
use ikaros_toolkit::AuditEvent;
use std::{
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use time::{
    OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339, macros::format_description,
};

const DEFAULT_AUDIT_ROTATION_MAX_BYTES: u64 = 16 * 1024 * 1024;
const ROTATED_AUDIT_TIMESTAMP_FORMAT: &[time::format_description::FormatItem<'_>] =
    format_description!("[year][month][day]T[hour][minute][second]Z");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditRotationPolicy {
    pub max_bytes: u64,
    pub rotate_on_date_change: bool,
}

impl AuditRotationPolicy {
    pub fn disabled() -> Self {
        Self {
            max_bytes: 0,
            rotate_on_date_change: false,
        }
    }
}

impl Default for AuditRotationPolicy {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_AUDIT_ROTATION_MAX_BYTES,
            rotate_on_date_change: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuditLog {
    path: PathBuf,
    rotation: AuditRotationPolicy,
    io_lock: Arc<Mutex<()>>,
}

impl AuditLog {
    pub fn new(audit_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: audit_dir.into().join("audit.jsonl"),
            rotation: AuditRotationPolicy::default(),
            io_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            rotation: AuditRotationPolicy::default(),
            io_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn with_rotation(mut self, rotation: AuditRotationPolicy) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, event: AuditEvent) -> Result<()> {
        let _guard = self
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.ensure_parent_dir()?;
        let encoded = serde_json::to_string(&event)?;
        self.rotate_if_needed(encoded.len() as u64, &event.at)?;
        let mut line = encoded;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| IkarosError::io(&self.path, source))?;
        file.write_all(line.as_bytes())
            .map_err(|source| IkarosError::io(&self.path, source))
    }

    pub fn read_all(&self) -> Result<Vec<AuditEvent>> {
        let _guard = self
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut events = Vec::new();
        for path in self.read_paths()? {
            read_events_from_path(&path, &mut events)?;
        }
        Ok(events)
    }

    fn ensure_parent_dir(&self) -> Result<()> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        Ok(())
    }

    fn rotate_if_needed(&self, encoded_len: u64, event_at: &str) -> Result<()> {
        if !self.should_rotate(encoded_len, event_at)? {
            return Ok(());
        }
        let rotated_path = self.next_rotated_path(event_at)?;
        fs::rename(&self.path, &rotated_path)
            .map_err(|source| IkarosError::io(&self.path, source))?;
        compress_rotated_file(&rotated_path)
    }

    fn should_rotate(&self, encoded_len: u64, event_at: &str) -> Result<bool> {
        let metadata = match fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(IkarosError::io(&self.path, error)),
        };
        if !metadata.is_file() || metadata.len() == 0 {
            return Ok(false);
        }
        if self.rotation.max_bytes > 0
            && metadata.len().saturating_add(encoded_len).saturating_add(1)
                > self.rotation.max_bytes
        {
            return Ok(true);
        }
        if self.rotation.rotate_on_date_change {
            let current_day = first_event_day(&self.path)?;
            let incoming_day = event_day(event_at);
            if current_day.is_some() && incoming_day.is_some() && current_day != incoming_day {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn read_paths(&self) -> Result<Vec<PathBuf>> {
        let parent = parent_dir(&self.path);
        if !parent.exists() {
            return Ok(Vec::new());
        }
        let mut archive_paths = Vec::new();
        for entry in fs::read_dir(parent).map_err(|source| IkarosError::io(parent, source))? {
            let entry = entry.map_err(|source| IkarosError::io(parent, source))?;
            let path = entry.path();
            if path == self.path {
                continue;
            }
            if self.is_archive_path(&path) {
                archive_paths.push(path);
            }
        }
        archive_paths.sort();
        if self.path.exists() {
            archive_paths.push(self.path.clone());
        }
        Ok(archive_paths)
    }

    fn next_rotated_path(&self, event_at: &str) -> Result<PathBuf> {
        let parent = parent_dir(&self.path);
        let (stem, extension) = active_file_parts(&self.path);
        let timestamp = archive_timestamp(event_at)?;
        for sequence in 0..10_000 {
            let suffix = if sequence == 0 {
                String::new()
            } else {
                format!("-{sequence:04}")
            };
            let candidate = parent.join(format!("{stem}-{timestamp}{suffix}.{extension}"));
            let compressed_candidate = gzip_path(&candidate)?;
            if !candidate.exists() && !compressed_candidate.exists() {
                return Ok(candidate);
            }
        }
        Err(IkarosError::Message(format!(
            "could not find available rotated audit log name for {}",
            self.path.display()
        )))
    }

    fn is_archive_path(&self, path: &Path) -> bool {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        let (stem, extension) = active_file_parts(&self.path);
        let archive_prefix = format!("{stem}-");
        let archive_suffix = format!(".{extension}");
        let compressed_archive_suffix = format!("{archive_suffix}.gz");
        file_name.starts_with(&archive_prefix)
            && (file_name.ends_with(&archive_suffix)
                || file_name.ends_with(&compressed_archive_suffix))
    }
}

fn read_events_from_path(path: &Path, events: &mut Vec<AuditEvent>) -> Result<()> {
    if path.extension().is_some_and(|extension| extension == "gz") {
        let file = fs::File::open(path).map_err(|source| IkarosError::io(path, source))?;
        let decoder = GzDecoder::new(file);
        read_events_from_reader(path, BufReader::new(decoder), events)
    } else {
        let file = fs::File::open(path).map_err(|source| IkarosError::io(path, source))?;
        read_events_from_reader(path, BufReader::new(file), events)
    }
}

fn read_events_from_reader<R: BufRead>(
    path: &Path,
    reader: R,
    events: &mut Vec<AuditEvent>,
) -> Result<()> {
    for (line_index, line) in reader.lines().enumerate() {
        let line = line.map_err(|source| IkarosError::io(path, source))?;
        if !line.trim().is_empty() {
            read_events_from_line(path, line_index + 1, &line, events)?;
        }
    }
    Ok(())
}

fn read_events_from_line(
    path: &Path,
    line_number: usize,
    line: &str,
    events: &mut Vec<AuditEvent>,
) -> Result<()> {
    for event in serde_json::Deserializer::from_str(line).into_iter::<AuditEvent>() {
        events.push(event.map_err(|source| {
            IkarosError::Message(format!(
                "failed to parse audit log {} line {}: {source}",
                path.display(),
                line_number
            ))
        })?);
    }
    Ok(())
}

fn first_event_day(path: &Path) -> Result<Option<String>> {
    let file = fs::File::open(path).map_err(|source| IkarosError::io(path, source))?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = line.map_err(|source| IkarosError::io(path, source))?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            return Ok(None);
        };
        return Ok(value
            .get("at")
            .and_then(serde_json::Value::as_str)
            .and_then(event_day));
    }
    Ok(None)
}

fn event_day(at: &str) -> Option<String> {
    OffsetDateTime::parse(at, &Rfc3339)
        .ok()
        .map(|datetime| datetime.date().to_string())
}

fn archive_timestamp(at: &str) -> Result<String> {
    let datetime =
        OffsetDateTime::parse(at, &Rfc3339).unwrap_or_else(|_| OffsetDateTime::now_utc());
    Ok(datetime
        .to_offset(UtcOffset::UTC)
        .format(ROTATED_AUDIT_TIMESTAMP_FORMAT)?)
}

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn active_file_parts(path: &Path) -> (String, String) {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("audit")
        .to_owned();
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("jsonl")
        .to_owned();
    (stem, extension)
}

fn gzip_path(path: &Path) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .ok_or_else(|| IkarosError::Message(format!("missing file name: {}", path.display())))?;
    let mut compressed_name = file_name.to_os_string();
    compressed_name.push(".gz");
    Ok(path.with_file_name(compressed_name))
}

fn compress_rotated_file(path: &Path) -> Result<()> {
    let gzip_path = gzip_path(path)?;
    let mut input = fs::File::open(path).map_err(|source| IkarosError::io(path, source))?;
    let output =
        fs::File::create(&gzip_path).map_err(|source| IkarosError::io(&gzip_path, source))?;
    let mut encoder = GzEncoder::new(output, Compression::default());
    io::copy(&mut input, &mut encoder).map_err(|source| IkarosError::io(path, source))?;
    encoder
        .finish()
        .map_err(|source| IkarosError::io(&gzip_path, source))?;
    fs::remove_file(path).map_err(|source| IkarosError::io(path, source))
}
