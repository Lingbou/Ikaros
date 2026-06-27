// SPDX-License-Identifier: GPL-3.0-only

use crate::{ContextError, ContextReference, ContextReferenceKind, ContextResult};
use ikaros_core::redact_secrets;
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub fn parse_context_references(input: &str) -> Vec<ContextReference> {
    input
        .split_whitespace()
        .filter_map(|raw| {
            let raw = raw.trim_matches(|ch: char| matches!(ch, ',' | ';' | ')' | ']' | '}'));
            parse_context_reference_token(raw)
        })
        .collect()
}

pub fn ensure_workspace_child(path: &Path, workspace_root: &Path) -> ContextResult<PathBuf> {
    let workspace = workspace_root
        .canonicalize()
        .map_err(|_| ContextError::MissingPath {
            path: workspace_root.display().to_string(),
        })?;
    let requested = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    };
    let canonical = requested
        .canonicalize()
        .map_err(|_| ContextError::MissingPath {
            path: requested.display().to_string(),
        })?;
    if !canonical.starts_with(&workspace) {
        return Err(ContextError::WorkspaceEscape {
            path: requested.display().to_string(),
        });
    }
    Ok(canonical)
}

pub fn resolve_context_references(
    references: &[ContextReference],
    workspace_root: &Path,
) -> ContextResult<Vec<String>> {
    references
        .iter()
        .map(|reference| resolve_context_reference(reference, workspace_root))
        .collect()
}

pub fn resolve_context_reference(
    reference: &ContextReference,
    workspace_root: &Path,
) -> ContextResult<String> {
    match &reference.kind {
        ContextReferenceKind::File {
            path,
            start_line,
            end_line,
        } => resolve_file_reference(path, *start_line, *end_line, workspace_root),
        ContextReferenceKind::Folder { path } => resolve_folder_reference(path, workspace_root),
        ContextReferenceKind::Git { rev } => resolve_git_reference(
            workspace_root,
            ["show", "--stat", "--oneline", rev.as_str()],
        ),
        ContextReferenceKind::Diff => resolve_git_reference(workspace_root, ["diff", "--", "."]),
        ContextReferenceKind::Staged => {
            resolve_git_reference(workspace_root, ["diff", "--cached", "--", "."])
        }
        ContextReferenceKind::Url { url } => Ok(redact_secrets(&format!(
            "[reference/url] {url} skipped: network context references are not enabled"
        ))),
    }
}

fn resolve_file_reference(
    path: &Path,
    start_line: Option<usize>,
    end_line: Option<usize>,
    workspace_root: &Path,
) -> ContextResult<String> {
    let path = ensure_workspace_child(path, workspace_root)?;
    let bytes = fs::read(&path).map_err(|error| ContextError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })?;
    let byte_len = bytes.len();
    let content = match String::from_utf8(bytes) {
        Ok(content) if is_text_reference_content(&content) => content,
        _ => {
            return Ok(redact_secrets(&format!(
                "[reference/file {}]\nskipped: non-text or non-utf8 file bytes={}",
                display_workspace_relative(&path, workspace_root),
                byte_len
            )));
        }
    };
    let start = start_line.unwrap_or(1);
    let end = end_line.unwrap_or(start.saturating_add(119));
    let selected = content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line_number = index + 1;
            (line_number >= start && line_number <= end).then(|| format!("{line_number}: {line}"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(redact_secrets(&format!(
        "[reference/file {}:{}-{}]\n{}",
        display_workspace_relative(&path, workspace_root),
        start,
        end,
        selected
    )))
}

fn is_text_reference_content(content: &str) -> bool {
    content
        .chars()
        .all(|ch| !ch.is_control() || matches!(ch, '\n' | '\r' | '\t'))
}

fn resolve_folder_reference(path: &Path, workspace_root: &Path) -> ContextResult<String> {
    let path = ensure_workspace_child(path, workspace_root)?;
    let mut entries = fs::read_dir(&path)
        .map_err(|error| ContextError::Io {
            path: path.display().to_string(),
            message: error.to_string(),
        })?
        .filter_map(|entry| entry.ok())
        .map(|entry| {
            let path = entry.path();
            let suffix = if path.is_dir() { "/" } else { "" };
            format!(
                "{}{}",
                path.file_name()
                    .and_then(OsStr::to_str)
                    .unwrap_or("<invalid-name>"),
                suffix
            )
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries.truncate(80);
    Ok(redact_secrets(&format!(
        "[reference/folder {}]\n{}",
        display_workspace_relative(&path, workspace_root),
        entries.join("\n")
    )))
}

fn resolve_git_reference<const N: usize>(
    workspace_root: &Path,
    args: [&str; N],
) -> ContextResult<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| ContextError::Io {
            path: workspace_root.display().to_string(),
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(ContextError::Git {
            command: format!("git -C {} {}", workspace_root.display(), args.join(" ")),
            stderr: redact_secrets(&String::from_utf8_lossy(&output.stderr)),
        });
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(redact_secrets(&format!(
        "[reference/git {}]\n{}",
        args.join(" "),
        truncate_reference_text(&text)
    )))
}

fn display_workspace_relative(path: &Path, workspace_root: &Path) -> String {
    let workspace = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    path.strip_prefix(&workspace)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn truncate_reference_text(text: &str) -> String {
    const MAX_CHARS: usize = 12_000;
    let mut output = text.chars().take(MAX_CHARS).collect::<String>();
    if text.chars().count() > MAX_CHARS {
        output.push_str("\n... [truncated]");
    }
    output
}

fn parse_context_reference_token(raw: &str) -> Option<ContextReference> {
    if let Some(rest) = raw.strip_prefix("@file:") {
        let (path, start_line, end_line) = parse_file_reference(rest)?;
        return Some(ContextReference {
            raw: raw.to_owned(),
            kind: ContextReferenceKind::File {
                path,
                start_line,
                end_line,
            },
        });
    }
    if let Some(rest) = raw.strip_prefix("@folder:") {
        if rest.trim().is_empty() {
            return None;
        }
        return Some(ContextReference {
            raw: raw.to_owned(),
            kind: ContextReferenceKind::Folder {
                path: PathBuf::from(rest),
            },
        });
    }
    if let Some(rest) = raw.strip_prefix("@git:") {
        if rest.trim().is_empty() {
            return None;
        }
        return Some(ContextReference {
            raw: raw.to_owned(),
            kind: ContextReferenceKind::Git {
                rev: rest.to_owned(),
            },
        });
    }
    if raw == "@diff" {
        return Some(ContextReference {
            raw: raw.to_owned(),
            kind: ContextReferenceKind::Diff,
        });
    }
    if raw == "@staged" {
        return Some(ContextReference {
            raw: raw.to_owned(),
            kind: ContextReferenceKind::Staged,
        });
    }
    if let Some(rest) = raw.strip_prefix("@url:") {
        if rest.trim().is_empty() {
            return None;
        }
        return Some(ContextReference {
            raw: raw.to_owned(),
            kind: ContextReferenceKind::Url {
                url: rest.to_owned(),
            },
        });
    }
    None
}

fn parse_file_reference(raw: &str) -> Option<(PathBuf, Option<usize>, Option<usize>)> {
    if raw.trim().is_empty() {
        return None;
    }
    let Some((path, range)) = raw.rsplit_once(':') else {
        return Some((PathBuf::from(raw), None, None));
    };
    let Some((start, end)) = range.split_once('-') else {
        return Some((PathBuf::from(raw), None, None));
    };
    let start = start.parse::<usize>().ok()?;
    let end = end.parse::<usize>().ok()?;
    if start == 0 || end < start {
        return None;
    }
    Some((PathBuf::from(path), Some(start), Some(end)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_context_references() {
        let refs = parse_context_references(
            "check @file:src/lib.rs:3-8 plus @folder:crates @git:HEAD @diff @staged",
        );

        assert_eq!(refs.len(), 5);
        assert!(matches!(
            refs[0].kind,
            ContextReferenceKind::File {
                start_line: Some(3),
                end_line: Some(8),
                ..
            }
        ));
        assert!(matches!(refs[3].kind, ContextReferenceKind::Diff));
        assert!(matches!(refs[4].kind, ContextReferenceKind::Staged));
    }

    #[test]
    fn workspace_child_rejects_escape() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("inside.txt"), "ok").expect("write");
        let inside = ensure_workspace_child(Path::new("inside.txt"), temp.path()).expect("inside");
        assert!(inside.ends_with("inside.txt"));
        let outside = ensure_workspace_child(Path::new("../missing.txt"), temp.path())
            .expect_err("outside should fail");
        assert!(matches!(
            outside,
            ContextError::MissingPath { .. } | ContextError::WorkspaceEscape { .. }
        ));
    }

    #[test]
    fn resolves_workspace_file_reference() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("notes.md"),
            "first line\nsecond line token=abc123\nthird line\n",
        )
        .expect("write");
        let references = parse_context_references("@file:notes.md:1-2");

        let resolved =
            resolve_context_references(&references, temp.path()).expect("resolved references");

        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].contains("1: first line"));
        assert!(resolved[0].contains("[REDACTED_SECRET]"));
        assert!(!resolved[0].contains("abc123"));
    }

    #[test]
    fn resolves_binary_file_reference_as_structured_notice() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("image.bin"), [0xff, 0x00, 0x89, 0x50])
            .expect("write binary");
        let references = parse_context_references("@file:image.bin");

        let resolved =
            resolve_context_references(&references, temp.path()).expect("resolved references");

        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].contains("[reference/file image.bin]"));
        assert!(resolved[0].contains("skipped: non-text or non-utf8 file"));
        assert!(resolved[0].contains("bytes=4"));
    }
}
