// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result};
use std::path::{Path, PathBuf};

use super::path::validate_relative_patch_path;
use super::report::PatchFileOperation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FilePatch {
    pub(super) old_path: Option<PathBuf>,
    pub(super) new_path: Option<PathBuf>,
    pub(super) hunks: Vec<Hunk>,
}

impl FilePatch {
    pub(super) fn source_and_target_paths(&self) -> Result<(&Path, &Path)> {
        let source = self
            .old_path
            .as_ref()
            .or(self.new_path.as_ref())
            .ok_or_else(|| {
                IkarosError::Message("file patch missing source and target path".into())
            })?;
        let target = self
            .new_path
            .as_ref()
            .or(self.old_path.as_ref())
            .ok_or_else(|| {
                IkarosError::Message("file patch missing source and target path".into())
            })?;
        Ok((source.as_path(), target.as_path()))
    }

    pub(super) fn operation(&self) -> Result<PatchFileOperation> {
        match (&self.old_path, &self.new_path) {
            (None, Some(_)) => Ok(PatchFileOperation::Add),
            (Some(_), None) => Ok(PatchFileOperation::Delete),
            (Some(old), Some(new)) if old == new => Ok(PatchFileOperation::Update),
            (Some(old), Some(_)) => Ok(PatchFileOperation::Move { from: old.clone() }),
            (None, None) => Err(IkarosError::Message(
                "file patch missing source and target path".into(),
            )),
        }
    }

    pub(super) fn display_path(&self) -> PathBuf {
        self.new_path
            .clone()
            .or_else(|| self.old_path.clone())
            .unwrap_or_else(|| PathBuf::from("<unknown>"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Hunk {
    pub(super) old_start: usize,
    pub(super) lines: Vec<HunkLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HunkLine {
    Context(String),
    Add(String),
    Remove(String),
}

pub(super) fn parse_unified_diff(diff: &str) -> Result<Vec<FilePatch>> {
    let mut patches = Vec::<FilePatch>::new();
    let mut current_old_path = None::<PathBuf>;
    let mut current_new_path = None::<PathBuf>;
    let mut current_hunks = Vec::<Hunk>::new();
    let mut current_hunk = None::<Hunk>;

    for raw_line in diff.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.starts_with("diff --git ") {
            flush_hunk(&mut current_hunks, &mut current_hunk);
            flush_file(
                &mut patches,
                &mut current_old_path,
                &mut current_new_path,
                &mut current_hunks,
            )?;
            continue;
        }
        if let Some(path) = line.strip_prefix("rename from ") {
            current_old_path = Some(parse_diff_path(path)?);
            continue;
        }
        if let Some(path) = line.strip_prefix("rename to ") {
            current_new_path = Some(parse_diff_path(path)?);
            continue;
        }
        if let Some(path) = line.strip_prefix("--- ") {
            flush_hunk(&mut current_hunks, &mut current_hunk);
            current_old_path = parse_optional_diff_path(path)?;
            continue;
        }
        if let Some(path) = line.strip_prefix("+++ ") {
            flush_hunk(&mut current_hunks, &mut current_hunk);
            current_new_path = parse_optional_diff_path(path)?;
            continue;
        }
        if let Some(header) = line.strip_prefix("@@ ") {
            flush_hunk(&mut current_hunks, &mut current_hunk);
            current_hunk = Some(Hunk {
                old_start: parse_hunk_old_start(header)?,
                lines: Vec::new(),
            });
            continue;
        }
        if line == "\\ No newline at end of file" {
            continue;
        }
        if let Some(hunk) = &mut current_hunk {
            let Some(marker) = line.chars().next() else {
                return Err(IkarosError::Message("empty hunk line is invalid".into()));
            };
            let text = line
                .get(marker.len_utf8()..)
                .ok_or_else(|| IkarosError::Message("invalid hunk line".into()))?
                .to_string();
            match marker {
                ' ' => hunk.lines.push(HunkLine::Context(text)),
                '+' => hunk.lines.push(HunkLine::Add(text)),
                '-' => hunk.lines.push(HunkLine::Remove(text)),
                _ => {
                    return Err(IkarosError::Message(format!(
                        "unsupported hunk line marker: {marker}"
                    )));
                }
            }
        }
    }

    flush_hunk(&mut current_hunks, &mut current_hunk);
    flush_file(
        &mut patches,
        &mut current_old_path,
        &mut current_new_path,
        &mut current_hunks,
    )?;
    Ok(patches)
}

fn flush_hunk(hunks: &mut Vec<Hunk>, hunk: &mut Option<Hunk>) {
    if let Some(hunk) = hunk.take() {
        hunks.push(hunk);
    }
}

fn flush_file(
    patches: &mut Vec<FilePatch>,
    old_path: &mut Option<PathBuf>,
    new_path: &mut Option<PathBuf>,
    hunks: &mut Vec<Hunk>,
) -> Result<()> {
    if hunks.is_empty() && old_path.as_ref() == new_path.as_ref() {
        *old_path = None;
        *new_path = None;
        return Ok(());
    }
    if old_path.is_none() && new_path.is_none() {
        return Err(IkarosError::Message(
            "file patch missing source and target path".into(),
        ));
    }
    patches.push(FilePatch {
        old_path: old_path.take(),
        new_path: new_path.take(),
        hunks: std::mem::take(hunks),
    });
    Ok(())
}

pub(crate) fn parse_diff_path(raw: &str) -> Result<PathBuf> {
    parse_optional_diff_path(raw)?.ok_or_else(|| {
        IkarosError::Message("delete-only patches are not supported by this path parser".into())
    })
}

fn parse_optional_diff_path(raw: &str) -> Result<Option<PathBuf>> {
    let trimmed = raw.trim();
    if trimmed.contains('"') || (trimmed.contains(' ') && !trimmed.contains('\t')) {
        return Err(IkarosError::Message(format!(
            "patch path contains unsupported quoted or space-delimited form: {trimmed}"
        )));
    }
    let path = trimmed.split_whitespace().next().unwrap_or(trimmed);
    if path == "/dev/null" {
        return Ok(None);
    }
    let stripped = path
        .strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path);
    let path = PathBuf::from(stripped);
    validate_relative_patch_path(&path)?;
    Ok(Some(path))
}

fn parse_hunk_old_start(header: &str) -> Result<usize> {
    let mut parts = header.split_whitespace();
    let old_range = parts
        .next()
        .ok_or_else(|| IkarosError::Message("hunk header missing old range".into()))?;
    let new_range = parts
        .next()
        .ok_or_else(|| IkarosError::Message("hunk header missing new range".into()))?;
    let old_start = parse_hunk_range_start(old_range, '-')?;
    parse_hunk_range_start(new_range, '+')?;
    Ok(old_start)
}

fn parse_hunk_range_start(range: &str, marker: char) -> Result<usize> {
    let range = range
        .strip_prefix(marker)
        .ok_or_else(|| IkarosError::Message(format!("hunk range must start with '{marker}'")))?;
    let start = range
        .split(',')
        .next()
        .ok_or_else(|| IkarosError::Message("hunk range missing start".into()))?;
    let length = range.split(',').nth(1);
    let parsed_start = start
        .parse::<usize>()
        .map_err(|source| IkarosError::Message(format!("invalid hunk range start: {source}")))?;
    if let Some(length) = length {
        length.parse::<usize>().map_err(|source| {
            IkarosError::Message(format!("invalid hunk range length: {source}"))
        })?;
    }
    Ok(parsed_start)
}
