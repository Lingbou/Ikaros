// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_core::{IkarosPaths, redact_secrets};
use ikaros_terminal::{terminal_inline, terminal_message};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub(in crate::chat) fn workbench_history_path(paths: &IkarosPaths) -> PathBuf {
    paths.home.join("workbench").join("history.txt")
}

pub(in crate::chat) fn append_workbench_history(
    paths: &IkarosPaths,
    input: &str,
) -> Result<PathBuf> {
    let path = workbench_history_path(paths);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    let input = redact_secrets(input.trim());
    writeln!(file, "{input}")?;
    writeln!(file, "---")?;
    Ok(path)
}

pub(in crate::chat) fn load_workbench_history_entries(
    paths: &IkarosPaths,
    limit: usize,
) -> Result<Vec<String>> {
    let path = workbench_history_path(paths);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)?;
    let mut entries = content
        .split("\n---\n")
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(terminal_message)
        .collect::<Vec<_>>();
    let limit = limit.max(1);
    let start = entries.len().saturating_sub(limit);
    Ok(entries.split_off(start))
}

pub(in crate::chat) fn print_workbench_input_history(
    paths: &IkarosPaths,
    limit: usize,
) -> Result<()> {
    let path = workbench_history_path(paths);
    let entries = load_workbench_history_entries(paths, limit)?;
    if entries.is_empty() {
        println!("workbench_input_history: 0");
        println!("workbench_history: {}", path_display(&path));
        return Ok(());
    }
    println!("workbench_input_history: {}", entries.len());
    println!("workbench_history: {}", path_display(&path));
    for (index, entry) in entries.iter().enumerate() {
        println!("- index={} input={}", index + 1, entry);
    }
    Ok(())
}

pub(in crate::chat) fn workbench_input_history_human_lines(
    paths: &IkarosPaths,
    limit: usize,
) -> Result<Vec<String>> {
    let entries = load_workbench_history_entries(paths, limit)?;
    let mut lines = vec!["* History".to_owned()];
    if entries.is_empty() {
        lines.push("  no previous input".to_owned());
        return Ok(lines);
    }
    for (index, entry) in entries.iter().enumerate() {
        lines.push(format!("  {}. {}", index + 1, terminal_inline(entry)));
    }
    Ok(lines)
}

pub(in crate::chat) fn normalize_session_id(input: &str) -> String {
    redact_secrets(input)
        .trim()
        .replace(['/', '\\', ':', '\n', '\r', '\t'], "_")
}

pub(in crate::chat) fn path_display(path: &Path) -> String {
    terminal_inline(&path.display().to_string())
}
