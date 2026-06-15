// SPDX-License-Identifier: GPL-3.0-only

use crate::{LocalMemoryStore, MemoryKind, MemoryQuery, MemoryRecord, MemoryRef, MemoryStore};
use ikaros_core::{ExternalMemoryProviderConfig, IkarosError, Result, redact_secrets};
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
    fn turn_start(&self, input: MemoryTurnStart) -> Result<MemoryLifecycleReport>;
    fn prefetch(&self, input: MemoryPrefetchInput) -> Result<Vec<MemoryRecord>>;
    fn sync_turn(&self, turn: MemoryTurnRecord) -> Result<MemoryLifecycleReport>;
    fn pre_compress(&self, input: MemoryPreCompressInput) -> Result<MemoryLifecycleReport>;
    fn session_switch(&self, input: MemorySessionSwitch) -> Result<MemoryLifecycleReport>;
    fn delegation_observation(
        &self,
        input: MemoryDelegationObservation,
    ) -> Result<MemoryLifecycleReport>;
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
    pub turn_id: Option<String>,
    pub agent_id: Option<String>,
    pub user_input: String,
    pub assistant_output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryPreCompressInput {
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub budget_tokens: usize,
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

#[derive(Debug, Clone, Default)]
pub struct NoopMemoryProvider;

impl MemoryProvider for NoopMemoryProvider {
    fn descriptor(&self) -> MemoryProviderDescriptor {
        MemoryProviderDescriptor {
            id: "noop".into(),
            kind: MemoryProviderKind::BuiltinLocal,
            backend: "noop".into(),
            state: MemoryProviderState::Disabled,
            path: None,
            endpoint: None,
            api_key_configured: false,
            notes: vec!["explicit noop memory provider".into()],
        }
    }

    fn turn_start(&self, _input: MemoryTurnStart) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport::noop("turn_start"))
    }

    fn prefetch(&self, _input: MemoryPrefetchInput) -> Result<Vec<MemoryRecord>> {
        Ok(Vec::new())
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

    fn append(&self, record: MemoryRecord) -> Result<MemoryRecord> {
        Ok(record)
    }

    fn search(&self, _query: MemoryQuery) -> Result<Vec<MemoryRecord>> {
        Ok(Vec::new())
    }

    fn update(
        &self,
        _id: &str,
        _content: Option<String>,
        _tags: Option<Vec<String>>,
    ) -> Result<Option<MemoryRecord>> {
        Ok(None)
    }

    fn delete_by_id(&self, _id: &str) -> Result<bool> {
        Ok(false)
    }

    fn delete_scope(&self, _query_kind: Option<MemoryKind>, _scope: &str) -> Result<usize> {
        Ok(0)
    }
}

impl MemoryProvider for LocalMemoryStore {
    fn descriptor(&self) -> MemoryProviderDescriptor {
        MemoryProviderDescriptor::active_local(self.backend_name(), self.path().to_path_buf())
    }

    fn turn_start(&self, input: MemoryTurnStart) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport {
            phase: "turn_start".into(),
            records_read: 0,
            records_written: 0,
            notes: vec![format!(
                "session={} agent={}",
                input.session_id.as_deref().unwrap_or("none"),
                input.agent_id.as_deref().unwrap_or("none")
            )],
        })
    }

    fn prefetch(&self, input: MemoryPrefetchInput) -> Result<Vec<MemoryRecord>> {
        MemoryStore::search(self, input.query)
    }

    fn sync_turn(&self, turn: MemoryTurnRecord) -> Result<MemoryLifecycleReport> {
        let Some(session_id) = turn.session_id.clone() else {
            return Ok(MemoryLifecycleReport {
                phase: "sync_turn".into(),
                records_read: 0,
                records_written: 0,
                notes: vec!["skipped: missing session id".into()],
            });
        };
        let content = turn_summary_content(&turn);
        if content.trim().is_empty() {
            return Ok(MemoryLifecycleReport {
                phase: "sync_turn".into(),
                records_read: 0,
                records_written: 0,
                notes: vec!["skipped: empty turn".into()],
            });
        }
        if content.contains("[REDACTED_SECRET]") {
            return Ok(MemoryLifecycleReport {
                phase: "sync_turn".into(),
                records_read: 0,
                records_written: 0,
                notes: vec!["skipped: redacted secret marker present".into()],
            });
        }
        let record = MemoryRecord::new(MemoryKind::Task, session_id.clone(), content)?
            .with_tags(vec!["turn-summary".into(), "memory-lifecycle".into()])
            .with_source("memory_lifecycle")
            .with_source_ref(MemoryRef::SessionTurn {
                session_id,
                turn_id: turn.turn_id.clone(),
            });
        MemoryStore::append(self, record)?;
        Ok(MemoryLifecycleReport {
            phase: "sync_turn".into(),
            records_read: 0,
            records_written: 1,
            notes: Vec::new(),
        })
    }

    fn pre_compress(&self, input: MemoryPreCompressInput) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport {
            phase: "pre_compress".into(),
            records_read: 0,
            records_written: 0,
            notes: vec![format!("budget_tokens={}", input.budget_tokens)],
        })
    }

    fn session_switch(&self, input: MemorySessionSwitch) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport {
            phase: "session_switch".into(),
            records_read: 0,
            records_written: 0,
            notes: vec![format!(
                "from={} to={}",
                input.from_session_id.as_deref().unwrap_or("none"),
                input.to_session_id.as_deref().unwrap_or("none")
            )],
        })
    }

    fn delegation_observation(
        &self,
        input: MemoryDelegationObservation,
    ) -> Result<MemoryLifecycleReport> {
        if input.summary.trim().is_empty() {
            return Ok(MemoryLifecycleReport {
                phase: "delegation_observation".into(),
                records_read: 0,
                records_written: 0,
                notes: vec!["skipped: empty summary".into()],
            });
        }
        let scope = input
            .parent_agent_id
            .clone()
            .unwrap_or_else(|| "delegation".into());
        let record = MemoryRecord::new(
            MemoryKind::Task,
            scope,
            redact_secrets(&format!(
                "Delegation observation from child_agent={}: {}",
                input.child_agent_id.as_deref().unwrap_or("unknown"),
                input.summary
            )),
        )?
        .with_tags(vec![
            "delegation-observation".into(),
            "memory-lifecycle".into(),
        ])
        .with_source("memory_lifecycle");
        MemoryStore::append(self, record)?;
        Ok(MemoryLifecycleReport {
            phase: "delegation_observation".into(),
            records_read: 0,
            records_written: 1,
            notes: Vec::new(),
        })
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

fn turn_summary_content(turn: &MemoryTurnRecord) -> String {
    let user = truncate_memory_text(&redact_secrets(&turn.user_input), 1_200);
    let assistant = truncate_memory_text(&redact_secrets(&turn.assistant_output), 1_200);
    format!("Turn summary\nuser: {user}\nassistant: {assistant}")
}

fn truncate_memory_text(text: &str, max_chars: usize) -> String {
    let mut output = text.chars().take(max_chars).collect::<String>();
    if text.chars().count() > max_chars {
        output.push_str("... [truncated]");
    }
    output
}
