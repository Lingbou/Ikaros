// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result};
use ikaros_harness::FileSystem as ExecutionFileSystem;
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatchApplyReport {
    pub files_changed: usize,
    pub files_created: usize,
    pub hunks_applied: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub paths: Vec<PathBuf>,
}

pub struct GuardedPatchApplier;

impl GuardedPatchApplier {
    pub fn apply_unified_diff(root: &Path, diff: &str) -> Result<PatchApplyReport> {
        let patches = parse_unified_diff(diff)?;
        if patches.is_empty() {
            return Err(IkarosError::Message(
                "diff did not contain any file hunks".into(),
            ));
        }

        let mut staged = Vec::<StagedFilePatch>::new();

        for patch in patches {
            let target = resolve_patch_path(root, &patch.new_path)?;
            if staged
                .iter()
                .any(|staged_patch| staged_patch.target == target)
            {
                return Err(IkarosError::Message(format!(
                    "guarded edit contains duplicate target: {}",
                    patch.new_path.display()
                )));
            }
            let existed = target.exists();
            let original = if existed {
                fs::read_to_string(&target).map_err(|source| IkarosError::io(&target, source))?
            } else {
                String::new()
            };
            let updated = apply_file_patch(&original, &patch)?;
            staged.push(StagedFilePatch {
                target,
                existed,
                original,
                updated,
                patch,
            });
        }

        let report = build_patch_report(&staged);
        let mut written = Vec::<StagedFilePatch>::new();
        for staged_patch in staged {
            if let Some(parent) = staged_patch.target.parent() {
                if let Err(source) = fs::create_dir_all(parent) {
                    rollback_staged_writes(&written);
                    return Err(IkarosError::io(parent, source));
                }
            }
            if let Err(source) = fs::write(&staged_patch.target, &staged_patch.updated) {
                let target = staged_patch.target.clone();
                rollback_staged_writes(&written);
                return Err(IkarosError::io(&target, source));
            }
            written.push(staged_patch);
        }

        Ok(report)
    }

    pub async fn apply_unified_diff_with_env(
        root: &Path,
        diff: &str,
        file_system: &dyn ExecutionFileSystem,
    ) -> Result<PatchApplyReport> {
        let staged = stage_unified_diff_with_env(root, diff, file_system).await?;
        let report = build_patch_report(&staged);
        let mut written = Vec::<StagedFilePatch>::new();
        for staged_patch in staged {
            if let Some(parent) = staged_patch.target.parent()
                && let Err(error) = file_system.create_dir_all(parent).await
            {
                rollback_staged_writes_with_env(file_system, &written).await;
                return Err(error);
            }
            if let Err(error) = file_system
                .write_string(&staged_patch.target, staged_patch.updated.clone())
                .await
            {
                rollback_staged_writes_with_env(file_system, &written).await;
                return Err(error);
            }
            written.push(staged_patch);
        }
        Ok(report)
    }
}

async fn stage_unified_diff_with_env(
    root: &Path,
    diff: &str,
    file_system: &dyn ExecutionFileSystem,
) -> Result<Vec<StagedFilePatch>> {
    let patches = parse_unified_diff(diff)?;
    if patches.is_empty() {
        return Err(IkarosError::Message(
            "diff did not contain any file hunks".into(),
        ));
    }

    let mut staged = Vec::<StagedFilePatch>::new();
    for patch in patches {
        let target = resolve_patch_path(root, &patch.new_path)?;
        if staged
            .iter()
            .any(|staged_patch| staged_patch.target == target)
        {
            return Err(IkarosError::Message(format!(
                "guarded edit contains duplicate target: {}",
                patch.new_path.display()
            )));
        }
        let existed = target.exists();
        let original = if existed {
            file_system.read_to_string(&target).await?
        } else {
            String::new()
        };
        let updated = apply_file_patch(&original, &patch)?;
        staged.push(StagedFilePatch {
            target,
            existed,
            original,
            updated,
            patch,
        });
    }
    Ok(staged)
}

#[derive(Debug, Clone)]
struct StagedFilePatch {
    target: PathBuf,
    existed: bool,
    original: String,
    updated: String,
    patch: FilePatch,
}

fn build_patch_report(staged: &[StagedFilePatch]) -> PatchApplyReport {
    let mut report = PatchApplyReport {
        files_changed: 0,
        files_created: 0,
        hunks_applied: 0,
        insertions: 0,
        deletions: 0,
        paths: Vec::new(),
    };
    for staged_patch in staged {
        let patch = &staged_patch.patch;
        report.files_changed += 1;
        if !staged_patch.existed {
            report.files_created += 1;
        }
        report.hunks_applied += patch.hunks.len();
        report.insertions += patch
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .filter(|line| matches!(line, HunkLine::Add(_)))
            .count();
        report.deletions += patch
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .filter(|line| matches!(line, HunkLine::Remove(_)))
            .count();
        report.paths.push(staged_patch.target.clone());
    }
    report
}

fn rollback_staged_writes(written: &[StagedFilePatch]) {
    for staged_patch in written.iter().rev() {
        if staged_patch.existed {
            let _ = fs::write(&staged_patch.target, &staged_patch.original);
        } else {
            let _ = fs::remove_file(&staged_patch.target);
        }
    }
}

async fn rollback_staged_writes_with_env(
    file_system: &dyn ExecutionFileSystem,
    written: &[StagedFilePatch],
) {
    for staged_patch in written.iter().rev() {
        if staged_patch.existed {
            let _ = file_system
                .write_string(&staged_patch.target, staged_patch.original.clone())
                .await;
        } else {
            let _ = file_system.remove_file(&staged_patch.target).await;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FilePatch {
    new_path: PathBuf,
    hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Hunk {
    old_start: usize,
    lines: Vec<HunkLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HunkLine {
    Context(String),
    Add(String),
    Remove(String),
}

fn parse_unified_diff(diff: &str) -> Result<Vec<FilePatch>> {
    let mut patches = Vec::<FilePatch>::new();
    let mut current_path = None::<PathBuf>;
    let mut current_hunks = Vec::<Hunk>::new();
    let mut current_hunk = None::<Hunk>;

    for raw_line in diff.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.starts_with("diff --git ") {
            flush_hunk(&mut current_hunks, &mut current_hunk);
            flush_file(&mut patches, &mut current_path, &mut current_hunks)?;
            continue;
        }
        if let Some(path) = line.strip_prefix("+++ ") {
            flush_hunk(&mut current_hunks, &mut current_hunk);
            current_path = Some(parse_diff_path(path)?);
            continue;
        }
        if line.starts_with("--- ") {
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
    flush_file(&mut patches, &mut current_path, &mut current_hunks)?;
    Ok(patches)
}

fn flush_hunk(hunks: &mut Vec<Hunk>, hunk: &mut Option<Hunk>) {
    if let Some(hunk) = hunk.take() {
        hunks.push(hunk);
    }
}

fn flush_file(
    patches: &mut Vec<FilePatch>,
    path: &mut Option<PathBuf>,
    hunks: &mut Vec<Hunk>,
) -> Result<()> {
    if hunks.is_empty() {
        *path = None;
        return Ok(());
    }
    let path = path
        .take()
        .ok_or_else(|| IkarosError::Message("file patch missing +++ target path".into()))?;
    patches.push(FilePatch {
        new_path: path,
        hunks: std::mem::take(hunks),
    });
    Ok(())
}

pub(crate) fn parse_diff_path(raw: &str) -> Result<PathBuf> {
    let path = raw.split_whitespace().next().unwrap_or(raw);
    if path == "/dev/null" {
        return Err(IkarosError::Message(
            "delete-only patches are not supported by guarded edit".into(),
        ));
    }
    let stripped = path
        .strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path);
    let path = PathBuf::from(stripped);
    validate_relative_patch_path(&path)?;
    Ok(path)
}

fn parse_hunk_old_start(header: &str) -> Result<usize> {
    let old_range = header
        .split_whitespace()
        .next()
        .ok_or_else(|| IkarosError::Message("hunk header missing old range".into()))?;
    let old_range = old_range
        .strip_prefix('-')
        .ok_or_else(|| IkarosError::Message("hunk old range must start with '-'".into()))?;
    let start = old_range
        .split(',')
        .next()
        .ok_or_else(|| IkarosError::Message("hunk old range missing start".into()))?;
    start
        .parse::<usize>()
        .map_err(|source| IkarosError::Message(format!("invalid hunk old start: {source}")))
}

fn resolve_patch_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    validate_relative_patch_path(relative)?;
    let root = fs::canonicalize(root).map_err(|source| IkarosError::io(root, source))?;
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(name) => Ok(name.to_os_string()),
            _ => Err(IkarosError::Message(format!(
                "patch path contains unsupported component: {}",
                relative.display()
            ))),
        })
        .collect::<Result<Vec<OsString>>>()?;

    let mut target = root.clone();
    for (index, component) in components.iter().enumerate() {
        target.push(component);
        let is_leaf = index + 1 == components.len();
        match fs::symlink_metadata(&target) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(IkarosError::Message(format!(
                        "guarded edit rejects symlink patch target: {}",
                        target.display()
                    )));
                }
                if !is_leaf && !metadata.is_dir() {
                    return Err(IkarosError::Message(format!(
                        "patch path parent is not a directory: {}",
                        target.display()
                    )));
                }
                let canonical =
                    fs::canonicalize(&target).map_err(|source| IkarosError::io(&target, source))?;
                if !canonical.starts_with(&root) {
                    return Err(IkarosError::Message(format!(
                        "patch path escapes workspace: {}",
                        target.display()
                    )));
                }
            }
            Err(source) if source.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(IkarosError::io(&target, source)),
        }
    }

    Ok(target)
}

fn validate_relative_patch_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(IkarosError::Message(format!(
            "patch path must be relative: {}",
            path.display()
        )));
    }
    for component in path.components() {
        match component {
            Component::Normal(name) if name == ".temp" => {
                return Err(IkarosError::Message(
                    "guarded edit cannot modify .temp".into(),
                ));
            }
            Component::Normal(_) => {}
            _ => {
                return Err(IkarosError::Message(format!(
                    "patch path contains unsupported component: {}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

fn apply_file_patch(original: &str, patch: &FilePatch) -> Result<String> {
    let source = original.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    let mut output = Vec::<String>::new();
    let mut source_index = 0usize;

    for hunk in &patch.hunks {
        let hunk_start = hunk.old_start.saturating_sub(1);
        if hunk_start < source_index || hunk_start > source.len() {
            return Err(IkarosError::Message(format!(
                "hunk for {} is out of range",
                patch.new_path.display()
            )));
        }
        output.extend(source[source_index..hunk_start].iter().cloned());
        source_index = hunk_start;

        for line in &hunk.lines {
            match line {
                HunkLine::Context(text) => {
                    expect_source_line(&source, source_index, text, &patch.new_path)?;
                    output.push(text.clone());
                    source_index += 1;
                }
                HunkLine::Remove(text) => {
                    expect_source_line(&source, source_index, text, &patch.new_path)?;
                    source_index += 1;
                }
                HunkLine::Add(text) => output.push(text.clone()),
            }
        }
    }

    output.extend(source[source_index..].iter().cloned());
    let mut rendered = output.join("\n");
    if !rendered.is_empty() {
        rendered.push('\n');
    }
    Ok(rendered)
}

fn expect_source_line(source: &[String], index: usize, expected: &str, path: &Path) -> Result<()> {
    let actual = source.get(index).ok_or_else(|| {
        IkarosError::Message(format!(
            "hunk for {} expected more source lines",
            path.display()
        ))
    })?;
    if actual != expected {
        return Err(IkarosError::Message(format!(
            "hunk mismatch in {} at source line {}",
            path.display(),
            index + 1
        )));
    }
    Ok(())
}
