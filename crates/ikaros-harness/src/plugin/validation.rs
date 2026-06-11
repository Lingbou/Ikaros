// SPDX-License-Identifier: GPL-3.0-only

use super::{
    manifest::{
        PLUGIN_COMMAND_MAX_ARG_BYTES, PLUGIN_COMMAND_MAX_ARGS, PLUGIN_COMMAND_MAX_TIMEOUT_MS,
        PluginCommandManifest, PluginManifest,
    },
    marketplace::{PluginMarketplace, PluginMarketplaceEntry},
};
use ikaros_core::{IkarosError, Result, redact_json, redact_secrets};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

pub fn validate_plugin_marketplace(
    marketplace: PluginMarketplace,
) -> Result<BTreeMap<String, PluginMarketplaceEntry>> {
    let mut entries = BTreeMap::new();
    for entry in marketplace.plugins {
        let entry = redact_marketplace_entry(entry);
        validate_marketplace_entry(&entry)?;
        if entries.insert(entry.name.clone(), entry.clone()).is_some() {
            return Err(IkarosError::Message(format!(
                "duplicate marketplace plugin entry: {}",
                entry.name
            )));
        }
    }
    Ok(entries)
}

fn validate_marketplace_entry(entry: &PluginMarketplaceEntry) -> Result<()> {
    validate_identifier(&entry.name, "marketplace plugin name")?;
    if entry.source.trim().is_empty() {
        return Err(IkarosError::Message(
            "marketplace plugin source is required".into(),
        ));
    }
    if let Some(path) = &entry.path {
        validate_marketplace_path(path)?;
    }
    Ok(())
}

fn validate_marketplace_path(path: &Path) -> Result<()> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(component, Component::ParentDir) || component.as_os_str() == ".temp"
        })
    {
        return Err(IkarosError::Message(
            "marketplace plugin path must be relative and must not target .temp".into(),
        ));
    }
    Ok(())
}

pub(super) fn validate_plugin_manifest(manifest: &PluginManifest) -> Result<()> {
    validate_identifier(&manifest.name, "plugin name")?;
    if manifest.version.trim().is_empty() {
        return Err(IkarosError::Message("plugin version is required".into()));
    }
    if manifest.description.trim().is_empty() {
        return Err(IkarosError::Message(
            "plugin description is required".into(),
        ));
    }
    if manifest.skills.is_empty() {
        return Err(IkarosError::Message(
            "plugin must declare at least one skill".into(),
        ));
    }
    let mut names = BTreeSet::new();
    for skill in &manifest.skills {
        validate_identifier(&skill.name, "skill name")?;
        if !names.insert(skill.name.clone()) {
            return Err(IkarosError::Message(format!(
                "duplicate plugin skill: {}",
                skill.name
            )));
        }
        if skill.description.trim().is_empty() {
            return Err(IkarosError::Message(format!(
                "skill {} description is required",
                skill.name
            )));
        }
        if !skill.input_schema.is_object() {
            return Err(IkarosError::Message(format!(
                "skill {} input_schema must be a JSON schema object",
                skill.name
            )));
        }
        for permission in &skill.permissions {
            validate_identifier(&permission.action, "permission action")?;
        }
        if let Some(command) = &skill.command {
            validate_plugin_command(command)?;
        }
    }
    Ok(())
}

fn validate_plugin_command(command: &PluginCommandManifest) -> Result<()> {
    if command.program.as_os_str().is_empty()
        || command.program.is_absolute()
        || command.program.components().any(|component| {
            matches!(component, Component::ParentDir) || component.as_os_str() == ".temp"
        })
    {
        return Err(IkarosError::Message(
            "plugin command program must be relative and must not target .temp".into(),
        ));
    }
    if command.timeout_ms == Some(0) {
        return Err(IkarosError::Message(
            "plugin command timeout_ms must be greater than zero".into(),
        ));
    }
    if command
        .timeout_ms
        .is_some_and(|timeout_ms| timeout_ms > PLUGIN_COMMAND_MAX_TIMEOUT_MS)
    {
        return Err(IkarosError::Message(format!(
            "plugin command timeout_ms must be at most {PLUGIN_COMMAND_MAX_TIMEOUT_MS}"
        )));
    }
    if command.args.len() > PLUGIN_COMMAND_MAX_ARGS {
        return Err(IkarosError::Message(format!(
            "plugin command args must contain at most {PLUGIN_COMMAND_MAX_ARGS} entries"
        )));
    }
    for arg in &command.args {
        if arg.len() > PLUGIN_COMMAND_MAX_ARG_BYTES {
            return Err(IkarosError::Message(format!(
                "plugin command argument must be at most {PLUGIN_COMMAND_MAX_ARG_BYTES} bytes"
            )));
        }
        if arg.chars().any(char::is_control) {
            return Err(IkarosError::Message(
                "plugin command argument must not contain control characters".into(),
            ));
        }
    }
    Ok(())
}

fn validate_identifier(value: &str, label: &str) -> Result<()> {
    let valid = !value.trim().is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'));
    let mut components = Path::new(value).components();
    let valid_path_name =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if valid && valid_path_name {
        Ok(())
    } else {
        Err(IkarosError::Message(format!(
            "{label} must contain only ASCII letters, numbers, dot, dash, or underscore and must not be a path-reserved dot component"
        )))
    }
}

pub(super) fn redact_plugin_manifest(mut manifest: PluginManifest) -> PluginManifest {
    manifest.description = redact_secrets(&manifest.description);
    manifest.capabilities = manifest
        .capabilities
        .into_iter()
        .map(|capability| redact_secrets(&capability))
        .collect();
    for skill in &mut manifest.skills {
        skill.description = redact_secrets(&skill.description);
        skill.input_schema = redact_json(skill.input_schema.clone());
        if let Some(command) = &mut skill.command {
            command.program = PathBuf::from(redact_secrets(&command.program.to_string_lossy()));
            command.args = command.args.iter().map(|arg| redact_secrets(arg)).collect();
        }
        for permission in &mut skill.permissions {
            permission.action = redact_secrets(&permission.action);
            permission.paths = permission
                .paths
                .iter()
                .map(|path| PathBuf::from(redact_secrets(&path.to_string_lossy())))
                .collect();
        }
    }
    manifest
}

fn redact_marketplace_entry(mut entry: PluginMarketplaceEntry) -> PluginMarketplaceEntry {
    entry.source = redact_secrets(&entry.source);
    entry.repository = entry.repository.as_deref().map(redact_secrets);
    entry.homepage = entry.homepage.as_deref().map(redact_secrets);
    entry.license = entry.license.as_deref().map(redact_secrets);
    entry.tags = entry
        .tags
        .into_iter()
        .map(|tag| redact_secrets(&tag))
        .collect();
    entry.notes = entry.notes.as_deref().map(redact_secrets);
    if let Some(path) = &entry.path {
        entry.path = Some(PathBuf::from(redact_secrets(&path.to_string_lossy())));
    }
    entry
}
