// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result, bail};
use ikaros_core::ConfigValidationReport;
use std::{fs, path::Path};

pub(super) fn config_has_setup_paths(path: &Path) -> Result<bool> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config: {}", path.display()))?;
    Ok(raw.contains("\nproviders:\n") && raw.contains("\n  embedding:\n"))
}

pub(super) fn format_setup_validation_failure(
    summary: &str,
    report: &ConfigValidationReport,
) -> String {
    let mut message = summary.to_owned();
    for issue in &report.errors {
        message.push_str(&format!("\nerror: {}: {}", issue.path, issue.message));
    }
    for issue in &report.warnings {
        message.push_str(&format!("\nwarning: {}: {}", issue.path, issue.message));
    }
    message
}

pub(super) fn set_yaml_scalar(raw: String, path: &[&str], value: &str) -> Result<String> {
    set_yaml_scalar_raw(raw, path, &serde_json::to_string(value)?)
}

pub(super) fn set_yaml_scalar_raw(raw: String, path: &[&str], value: &str) -> Result<String> {
    let mut found = false;
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut output = String::new();
    for line in raw.lines() {
        let mut next = line.to_owned();
        if let Some((indent, key, colon_index)) = yaml_key(line) {
            while stack.last().is_some_and(|(level, _)| *level >= indent) {
                stack.pop();
            }
            stack.push((indent, key.to_owned()));
            if stack
                .iter()
                .map(|(_, key)| key.as_str())
                .eq(path.iter().copied())
            {
                next = format!("{} {}", &line[..=colon_index], value);
                found = true;
            }
        }
        output.push_str(&next);
        output.push('\n');
    }
    if !found {
        bail!(
            "config path `{}` was not found in config.yaml",
            path.join(".")
        );
    }
    Ok(output)
}

fn yaml_key(line: &str) -> Option<(usize, &str, usize)> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
        return None;
    }
    let indent = line.len() - trimmed.len();
    let colon_relative = trimmed.find(':')?;
    let key = trimmed[..colon_relative].trim();
    if key.is_empty() || key.contains(' ') || key.contains('\t') {
        return None;
    }
    Some((indent, key, indent + colon_relative))
}
