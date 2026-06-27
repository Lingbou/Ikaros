// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result};
use std::path::Path;

use super::parse::{FilePatch, Hunk, HunkLine};

pub(super) fn apply_file_patch(original: &str, patch: &FilePatch) -> Result<String> {
    let source = original.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    let mut output = Vec::<String>::new();
    let mut source_index = 0usize;

    for hunk in &patch.hunks {
        let hunk_start = hunk.old_start.saturating_sub(1);
        if hunk_start < source_index || hunk_start > source.len() {
            return Err(IkarosError::Message(format!(
                "hunk for {} is out of range",
                patch.display_path().display()
            )));
        }
        let anchor = hunk_source_anchor(hunk);
        let candidate_lines = find_line_sequence(&source, &anchor);
        if candidate_lines.len() > 1 {
            return Err(IkarosError::Message(format!(
                "ambiguous hunk anchor in {}: candidate source lines: {}",
                patch.display_path().display(),
                candidate_lines
                    .iter()
                    .map(|line| line.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        output.extend(source[source_index..hunk_start].iter().cloned());
        source_index = hunk_start;

        for line in &hunk.lines {
            match line {
                HunkLine::Context(text) => {
                    expect_source_line(&source, source_index, text, hunk, &patch.display_path())?;
                    output.push(text.clone());
                    source_index += 1;
                }
                HunkLine::Remove(text) => {
                    expect_source_line(&source, source_index, text, hunk, &patch.display_path())?;
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

fn expect_source_line(
    source: &[String],
    index: usize,
    expected: &str,
    hunk: &Hunk,
    path: &Path,
) -> Result<()> {
    let actual = source.get(index).ok_or_else(|| {
        IkarosError::Message(format!(
            "hunk for {} expected more source lines",
            path.display()
        ))
    })?;
    if actual != expected {
        let hunk_start = hunk.old_start.saturating_sub(1);
        let replacement = hunk_replacement_anchor(hunk);
        if sequence_matches_at(source, hunk_start, &replacement) {
            return Err(IkarosError::Message(format!(
                "hunk already applied in {} at source line {}",
                path.display(),
                index + 1
            )));
        }
        return Err(IkarosError::Message(format!(
            "hunk mismatch in {} at source line {}: expected `{}`, actual `{}`",
            path.display(),
            index + 1,
            expected.replace('`', "\\`"),
            actual.replace('`', "\\`")
        )));
    }
    Ok(())
}

fn hunk_source_anchor(hunk: &Hunk) -> Vec<String> {
    hunk.lines
        .iter()
        .filter_map(|line| match line {
            HunkLine::Context(text) | HunkLine::Remove(text) => Some(text.clone()),
            HunkLine::Add(_) => None,
        })
        .collect()
}

fn hunk_replacement_anchor(hunk: &Hunk) -> Vec<String> {
    hunk.lines
        .iter()
        .filter_map(|line| match line {
            HunkLine::Context(text) | HunkLine::Add(text) => Some(text.clone()),
            HunkLine::Remove(_) => None,
        })
        .collect()
}

fn find_line_sequence(source: &[String], needle: &[String]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > source.len() {
        return Vec::new();
    }
    source
        .windows(needle.len())
        .enumerate()
        .filter_map(|(index, window)| (window == needle).then_some(index + 1))
        .collect()
}

fn sequence_matches_at(source: &[String], start: usize, needle: &[String]) -> bool {
    if needle.is_empty() || start + needle.len() > source.len() {
        return false;
    }
    source[start..start + needle.len()] == *needle
}
