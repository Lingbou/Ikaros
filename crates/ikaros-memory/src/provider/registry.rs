// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{ExternalMemoryProviderConfig, IkarosError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::{MemoryProviderDescriptor, MemoryProviderKind, MemoryProviderState};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryProviderRegistry {
    pub active_local: MemoryProviderDescriptor,
    pub external: Vec<MemoryProviderDescriptor>,
    pub issues: Vec<String>,
}

impl MemoryProviderRegistry {
    pub fn from_config(
        memory_dir: impl AsRef<Path>,
        backend: &str,
        external_configs: &[ExternalMemoryProviderConfig],
    ) -> Result<Self> {
        let memory_dir = memory_dir.as_ref();
        let active_local =
            MemoryProviderDescriptor::active_local(backend, local_path(memory_dir, backend)?);
        let active_external_count = external_configs
            .iter()
            .filter(|provider| provider.enabled)
            .count();
        let mut issues = Vec::new();
        if active_external_count > 1 {
            issues.push(format!(
                "only one external memory provider may be active, found {active_external_count}"
            ));
        }

        let external = external_configs
            .iter()
            .enumerate()
            .map(|(index, provider)| {
                let mut notes = Vec::new();
                let id = normalized_or_default(&provider.id, format!("external-{}", index + 1));
                if provider.id.trim().is_empty() {
                    notes.push("missing id; generated display id".into());
                    issues.push(format!("external memory provider {id} is missing id"));
                }
                let backend = normalized_or_default(&provider.provider, "plugin".to_owned());
                if provider.provider.trim().is_empty() {
                    notes.push("missing provider; treating as plugin placeholder".into());
                    issues.push(format!("external memory provider {id} is missing provider"));
                }
                let state = if provider.enabled && active_external_count > 1 {
                    notes.push("blocked because multiple external providers are enabled".into());
                    MemoryProviderState::Blocked
                } else if provider.enabled {
                    MemoryProviderState::Active
                } else {
                    MemoryProviderState::Disabled
                };
                MemoryProviderDescriptor {
                    id,
                    kind: MemoryProviderKind::ExternalPlugin,
                    backend,
                    state,
                    path: None,
                    endpoint: provider.endpoint.clone(),
                    api_key_configured: option_has_value(provider.api_key.as_deref()),
                    notes,
                }
            })
            .collect();

        Ok(Self {
            active_local,
            external,
            issues,
        })
    }

    pub fn active_external(&self) -> Option<&MemoryProviderDescriptor> {
        self.external
            .iter()
            .find(|provider| provider.state == MemoryProviderState::Active)
    }

    pub fn active_external_count(&self) -> usize {
        self.external
            .iter()
            .filter(|provider| provider.state == MemoryProviderState::Active)
            .count()
    }

    pub fn ensure_single_active_external(&self) -> Result<()> {
        let active_or_blocked_count = self
            .external
            .iter()
            .filter(|provider| {
                matches!(
                    provider.state,
                    MemoryProviderState::Active | MemoryProviderState::Blocked
                )
            })
            .count();
        if active_or_blocked_count > 1 {
            return Err(IkarosError::Message(format!(
                "only one external memory provider may be active, found {active_or_blocked_count}"
            )));
        }
        Ok(())
    }
}

fn local_path(memory_dir: &Path, backend: &str) -> Result<PathBuf> {
    match backend {
        "jsonl" => Ok(memory_dir.join("memory.jsonl")),
        "sqlite" => Ok(memory_dir.join("memory.sqlite")),
        other => Err(IkarosError::Message(format!(
            "unsupported memory backend: {other}"
        ))),
    }
}

fn normalized_or_default(value: &str, default: String) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        default
    } else {
        trimmed.to_owned()
    }
}

fn option_has_value(value: Option<&str>) -> bool {
    value.map(|value| !value.trim().is_empty()).unwrap_or(false)
}
