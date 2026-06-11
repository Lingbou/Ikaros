// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};
use time::OffsetDateTime;

pub(crate) fn collect_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            IkarosError::Message(format!(
                "RAG ingest path does not exist: {}",
                path.display()
            ))
        } else {
            IkarosError::io(path, source)
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(IkarosError::Message(format!(
            "RAG ingest rejects symlink path: {}",
            path.display()
        )));
    }
    if metadata.is_file() {
        if is_indexable(path) {
            files.push(path.to_path_buf());
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(IkarosError::Message(format!(
            "RAG ingest path does not exist: {}",
            path.display()
        )));
    }
    for entry in fs::read_dir(path).map_err(|source| IkarosError::io(path, source))? {
        let entry = entry.map_err(|source| IkarosError::io(path, source))?;
        let path = entry.path();
        let metadata =
            fs::symlink_metadata(&path).map_err(|source| IkarosError::io(&path, source))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if should_skip(&path) {
            continue;
        }
        if metadata.is_dir() {
            collect_files(&path, files)?;
        } else if metadata.is_file() && is_indexable(&path) {
            files.push(path);
        }
    }
    files.sort();
    Ok(())
}

fn should_skip(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name == ".git"
                || name == "target"
                || name == "node_modules"
                || name == ".temp"
                || name.starts_with(".")
        })
}

fn is_indexable(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some(
            "md" | "txt"
                | "rs"
                | "toml"
                | "json"
                | "yaml"
                | "yml"
                | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "py"
        )
    )
}

pub(crate) fn chunk_text(text: &str, max_lines: usize) -> Vec<(usize, usize, String)> {
    let max_lines = max_lines.max(1);
    let lines = text.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    for (idx, slice) in lines.chunks(max_lines).enumerate() {
        let line_start = idx * max_lines + 1;
        let line_end = line_start + slice.len() - 1;
        let content = slice.join("\n");
        if !content.trim().is_empty() {
            chunks.push((line_start, line_end, content));
        }
    }
    chunks
}

pub(crate) fn system_time_to_rfc3339(time: std::time::SystemTime) -> Option<String> {
    let duration = time.duration_since(std::time::UNIX_EPOCH).ok()?;
    let datetime = OffsetDateTime::from_unix_timestamp(duration.as_secs() as i64).ok()?;
    datetime
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
}

pub(crate) fn canonical_or_self(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub(crate) fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}
