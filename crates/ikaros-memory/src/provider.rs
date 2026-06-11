// SPDX-License-Identifier: GPL-3.0-only

use crate::{LocalMemoryStore, MemoryKind, MemoryQuery, MemoryRecord, MemoryStore};
use ikaros_core::{ExternalMemoryProviderConfig, IkarosError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryProviderKind {
    BuiltinLocal,
    ExternalPlugin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryProviderState {
    Active,
    Disabled,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryProviderDescriptor {
    pub id: String,
    pub kind: MemoryProviderKind,
    pub backend: String,
    pub state: MemoryProviderState,
    pub path: Option<PathBuf>,
    pub endpoint: Option<String>,
    pub api_key_configured: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryProviderRegistry {
    pub active_local: MemoryProviderDescriptor,
    pub external: Vec<MemoryProviderDescriptor>,
    pub issues: Vec<String>,
}

pub trait MemoryProvider: Send + Sync {
    fn descriptor(&self) -> MemoryProviderDescriptor;
    fn turn_start(&self, _input: MemoryTurnStart) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport::noop("turn_start"))
    }

    fn prefetch(&self, input: MemoryPrefetchInput) -> Result<Vec<MemoryRecord>> {
        self.search(input.query)
    }

    fn sync_turn(&self, _turn: MemoryTurnRecord) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport::noop("sync_turn"))
    }

    fn pre_compress(&self, _input: MemoryPreCompressInput) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport::noop("pre_compress"))
    }

    fn session_switch(&self, _input: MemorySessionSwitch) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport::noop("session_switch"))
    }

    fn delegation_observation(
        &self,
        _input: MemoryDelegationObservation,
    ) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport::noop("delegation_observation"))
    }

    fn append(&self, record: MemoryRecord) -> Result<MemoryRecord>;
    fn search(&self, query: MemoryQuery) -> Result<Vec<MemoryRecord>>;
    fn update(
        &self,
        id: &str,
        content: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<Option<MemoryRecord>>;
    fn delete_by_id(&self, id: &str) -> Result<bool>;
    fn delete_scope(&self, query_kind: Option<MemoryKind>, scope: &str) -> Result<usize>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryLifecycleReport {
    pub phase: String,
    pub records_read: usize,
    pub records_written: usize,
    pub notes: Vec<String>,
}

impl MemoryLifecycleReport {
    pub fn noop(phase: impl Into<String>) -> Self {
        Self {
            phase: phase.into(),
            records_read: 0,
            records_written: 0,
            notes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryTurnStart {
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub user_input: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryPrefetchInput {
    pub query: MemoryQuery,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryTurnRecord {
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub user_input: String,
    pub assistant_output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryPreCompressInput {
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub budget_chars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemorySessionSwitch {
    pub from_session_id: Option<String>,
    pub to_session_id: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryDelegationObservation {
    pub parent_agent_id: Option<String>,
    pub child_agent_id: Option<String>,
    pub summary: String,
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

impl MemoryProviderDescriptor {
    pub fn active_local(backend: &str, path: PathBuf) -> Self {
        Self {
            id: format!("local-{backend}"),
            kind: MemoryProviderKind::BuiltinLocal,
            backend: backend.to_owned(),
            state: MemoryProviderState::Active,
            path: Some(path),
            endpoint: None,
            api_key_configured: false,
            notes: Vec::new(),
        }
    }
}

impl MemoryProvider for LocalMemoryStore {
    fn descriptor(&self) -> MemoryProviderDescriptor {
        MemoryProviderDescriptor::active_local(self.backend_name(), self.path().to_path_buf())
    }

    fn append(&self, record: MemoryRecord) -> Result<MemoryRecord> {
        MemoryStore::append(self, record)
    }

    fn search(&self, query: MemoryQuery) -> Result<Vec<MemoryRecord>> {
        MemoryStore::search(self, query)
    }

    fn update(
        &self,
        id: &str,
        content: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<Option<MemoryRecord>> {
        MemoryStore::update(self, id, content, tags)
    }

    fn delete_by_id(&self, id: &str) -> Result<bool> {
        MemoryStore::delete_by_id(self, id)
    }

    fn delete_scope(&self, query_kind: Option<MemoryKind>, scope: &str) -> Result<usize> {
        MemoryStore::delete_scope(self, query_kind, scope)
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
