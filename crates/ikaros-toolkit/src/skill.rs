// SPDX-License-Identifier: GPL-3.0-only

use crate::{AuditEvent, ExecutionEnv};
use async_trait::async_trait;
use ikaros_core::{IkarosError, Plan, Result, RiskLevel, ToolResult};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    future::Future,
    path::Path,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
};

#[derive(Debug, Clone)]
pub struct TaskGraph {
    pub plan: Plan,
}

#[derive(Clone)]
pub struct SkillContext {
    pub session: SkillRuntimeSession,
    pub toolsets: ToolsetSelection,
}

impl SkillContext {
    pub fn new(
        sandbox: SkillSandbox,
        env: Arc<dyn ExecutionEnv>,
        toolsets: ToolsetSelection,
        runtime: Arc<dyn SkillRuntime>,
    ) -> Self {
        Self {
            session: SkillRuntimeSession::new(sandbox, env, runtime),
            toolsets,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillSandbox {
    pub workspace_root: PathBuf,
}

impl SkillSandbox {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
        }
    }
}

#[derive(Clone)]
pub struct SkillRuntimeSession {
    pub sandbox: SkillSandbox,
    pub env: Arc<dyn ExecutionEnv>,
    pub audit: SkillAudit,
    runtime: Arc<dyn SkillRuntime>,
}

impl SkillRuntimeSession {
    fn new(
        sandbox: SkillSandbox,
        env: Arc<dyn ExecutionEnv>,
        runtime: Arc<dyn SkillRuntime>,
    ) -> Self {
        Self {
            sandbox,
            env,
            audit: SkillAudit {
                runtime: runtime.clone(),
            },
            runtime,
        }
    }

    pub fn disclose_deferred_tool(&self, name: impl Into<String>) {
        self.runtime.disclose_deferred_tool(name.into());
    }

    pub fn disclose_deferred_tools(&self, names: impl IntoIterator<Item = impl Into<String>>) {
        self.runtime
            .disclose_deferred_tools(names.into_iter().map(Into::into).collect());
    }

    pub fn is_deferred_tool_disclosed(&self, name: &str) -> bool {
        self.runtime.is_deferred_tool_disclosed(name)
    }

    pub fn execute_skill<'a>(
        &'a self,
        registry: &'a SkillRegistry,
        name: &'a str,
        input: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send + 'a>> {
        self.runtime.execute_skill(registry, name, input)
    }
}

#[derive(Clone)]
pub struct SkillAudit {
    runtime: Arc<dyn SkillRuntime>,
}

impl SkillAudit {
    pub fn append(&self, event: AuditEvent) -> Result<()> {
        self.runtime.append_audit_event(event)
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.runtime.audit_path()
    }
}

pub trait SkillRuntime: Send + Sync {
    fn append_audit_event(&self, event: AuditEvent) -> Result<()>;
    fn audit_path(&self) -> Option<PathBuf>;
    fn disclose_deferred_tool(&self, name: String);
    fn disclose_deferred_tools(&self, names: Vec<String>);
    fn is_deferred_tool_disclosed(&self, name: &str) -> bool;
    fn execute_skill<'a>(
        &'a self,
        registry: &'a SkillRegistry,
        name: &'a str,
        input: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult>> + Send + 'a>>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillOutput {
    pub summary: String,
    pub output: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyRequest {
    pub action: String,
    pub risk: RiskLevel,
    pub path: Option<PathBuf>,
    pub command: Option<String>,
    pub is_write: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub risk_level: RiskLevel,
    pub kind: SkillDescriptorKind,
    pub disable_model_invocation: bool,
    pub execution_mode: ToolExecutionMode,
    pub toolset: Toolset,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub support_files: Vec<PathBuf>,
}

impl SkillDescriptor {
    pub fn from_skill<S: Skill + ?Sized>(skill: &S) -> Self {
        let risk_level = skill.risk_level();
        Self {
            name: skill.name().into(),
            description: skill.description().into(),
            input_schema: skill.input_schema(),
            risk_level: risk_level.clone(),
            kind: SkillDescriptorKind::ExecutableTool,
            disable_model_invocation: false,
            execution_mode: ToolExecutionMode::default_for_risk(&risk_level),
            toolset: Toolset::Core,
            timeout_ms: None,
            provenance: None,
            support_files: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Toolset {
    Core,
    Workspace,
    Memory,
    Rag,
    Coding,
    Voice,
    Plugin,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolVisibility {
    Direct,
    Deferred,
    Disabled,
    Hidden,
}

impl Toolset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Workspace => "workspace",
            Self::Memory => "memory",
            Self::Rag => "rag",
            Self::Coding => "coding",
            Self::Voice => "voice",
            Self::Plugin => "plugin",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "core" => Some(Self::Core),
            "workspace" => Some(Self::Workspace),
            "memory" => Some(Self::Memory),
            "rag" => Some(Self::Rag),
            "coding" => Some(Self::Coding),
            "voice" => Some(Self::Voice),
            "plugin" => Some(Self::Plugin),
            _ => None,
        }
    }

    pub fn default_model_visible() -> &'static [Self] {
        &[Self::Core, Self::Workspace, Self::Memory]
    }
}

impl std::fmt::Display for Toolset {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolsetSelection {
    toolsets: BTreeSet<Toolset>,
}

impl Default for ToolsetSelection {
    fn default() -> Self {
        Self::new(Toolset::default_model_visible().iter().copied())
    }
}

impl ToolsetSelection {
    pub fn new(toolsets: impl IntoIterator<Item = Toolset>) -> Self {
        Self {
            toolsets: toolsets.into_iter().collect(),
        }
    }

    pub fn from_names(names: impl IntoIterator<Item = impl AsRef<str>>) -> Result<Self> {
        let mut toolsets = BTreeSet::new();
        for name in names {
            let name = name.as_ref();
            let toolset = Toolset::parse(name)
                .ok_or_else(|| IkarosError::Message(format!("unsupported toolset `{name}`")))?;
            toolsets.insert(toolset);
        }
        Ok(Self { toolsets })
    }

    pub fn contains(&self, toolset: Toolset) -> bool {
        self.toolsets.contains(&toolset)
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.toolsets
            .iter()
            .map(|toolset| toolset.as_str())
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillDescriptorKind {
    ExecutableTool,
    PromptSkill,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionMode {
    Parallel,
    Sequential,
}

impl ToolExecutionMode {
    pub fn default_for_risk(risk: &RiskLevel) -> Self {
        match risk {
            RiskLevel::SafeRead | RiskLevel::ShellRead => Self::Parallel,
            RiskLevel::LocalWrite
            | RiskLevel::ShellWrite
            | RiskLevel::Network
            | RiskLevel::DatabaseWrite
            | RiskLevel::RemoteServer
            | RiskLevel::Destructive
            | RiskLevel::SecretAccess
            | RiskLevel::SelfModify => Self::Sequential,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Parallel => "parallel",
            Self::Sequential => "sequential",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillBundle {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub descriptors: Vec<SkillDescriptor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub support_files: Vec<PathBuf>,
    pub disable_model_invocation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptSkillDocument {
    pub descriptor: SkillDescriptor,
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub support_files: Vec<PromptSkillSupportFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptSkillSupportFile {
    pub path: PathBuf,
    pub content: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

impl SkillBundle {
    pub fn explicit_only(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            descriptors: Vec::new(),
            support_files: Vec::new(),
            disable_model_invocation: true,
        }
    }
}

impl SkillOutput {
    pub fn new(summary: impl Into<String>, output: serde_json::Value) -> Self {
        Self {
            summary: summary.into(),
            output,
        }
    }
}

#[async_trait]
pub trait Skill: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn input_schema(&self) -> serde_json::Value;
    fn risk_level(&self) -> RiskLevel;
    fn descriptor(&self) -> SkillDescriptor {
        SkillDescriptor::from_skill(self)
    }
    fn policy_request(&self, input: &serde_json::Value, workspace_root: &Path) -> PolicyRequest {
        PolicyRequest {
            action: self.name().into(),
            risk: self.risk_level(),
            path: input
                .get("path")
                .and_then(serde_json::Value::as_str)
                .map(|path| resolve_under_workspace(Path::new(path), workspace_root)),
            command: input
                .get("command")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            is_write: matches!(
                self.risk_level(),
                RiskLevel::LocalWrite | RiskLevel::ShellWrite | RiskLevel::DatabaseWrite
            ),
        }
    }
    fn approval_context(
        &self,
        _input: &serde_json::Value,
        _workspace_root: &Path,
    ) -> Option<serde_json::Value> {
        None
    }
    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput>;
}

#[derive(Clone)]
struct RegisteredSkill {
    skill: Arc<dyn Skill>,
    toolset: Toolset,
}

#[derive(Clone, Default)]
pub struct SkillRegistry {
    skills: BTreeMap<String, RegisteredSkill>,
    prompt_skills: BTreeMap<String, PromptSkillDocument>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<S>(&mut self, skill: S)
    where
        S: Skill + 'static,
    {
        self.register_with_toolset(skill, Toolset::Core);
    }

    pub fn register_with_toolset<S>(&mut self, skill: S, toolset: Toolset)
    where
        S: Skill + 'static,
    {
        self.skills.insert(
            skill.name().into(),
            RegisteredSkill {
                skill: Arc::new(skill),
                toolset,
            },
        );
    }

    pub fn register_prompt_skill(
        &mut self,
        mut descriptor: SkillDescriptor,
        instructions: impl Into<String>,
    ) {
        descriptor.kind = SkillDescriptorKind::PromptSkill;
        descriptor.disable_model_invocation = true;
        self.prompt_skills.insert(
            descriptor.name.clone(),
            PromptSkillDocument {
                descriptor,
                instructions: instructions.into(),
                support_files: Vec::new(),
            },
        );
    }

    pub fn register_prompt_skill_document(&mut self, mut document: PromptSkillDocument) {
        document.descriptor.kind = SkillDescriptorKind::PromptSkill;
        document.descriptor.disable_model_invocation = true;
        self.prompt_skills
            .insert(document.descriptor.name.clone(), document);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Skill>> {
        self.skills.get(name).map(|entry| entry.skill.clone())
    }

    pub fn prompt_skill(&self, name: &str) -> Option<PromptSkillDocument> {
        self.prompt_skills.get(name).cloned()
    }

    pub fn names(&self) -> Vec<String> {
        let mut names = self.skills.keys().cloned().collect::<Vec<_>>();
        names.extend(self.prompt_skills.keys().cloned());
        names.sort();
        names
    }

    pub fn descriptors(&self) -> Vec<SkillDescriptor> {
        let mut descriptors = self
            .skills
            .values()
            .map(RegisteredSkill::descriptor)
            .collect::<Vec<_>>();
        descriptors.extend(
            self.prompt_skills
                .values()
                .map(|skill| skill.descriptor.clone()),
        );
        descriptors.sort_by(|left, right| left.name.cmp(&right.name));
        descriptors
    }

    pub fn model_visible_names(&self) -> Vec<String> {
        self.model_visible_names_for(&ToolsetSelection::default())
    }

    pub fn model_visible_names_for(&self, selection: &ToolsetSelection) -> Vec<String> {
        self.skills
            .iter()
            .filter(|(name, _)| {
                self.visibility_for(name, selection) == Some(ToolVisibility::Direct)
            })
            .map(|(name, _)| name.clone())
            .collect()
    }

    pub fn visibility_for(
        &self,
        name: &str,
        selection: &ToolsetSelection,
    ) -> Option<ToolVisibility> {
        if let Some(entry) = self.skills.get(name) {
            return Some(tool_visibility(&entry.descriptor(), selection));
        }
        self.prompt_skills
            .get(name)
            .map(|document| tool_visibility(&document.descriptor, selection))
    }

    pub fn tool_registry(&self) -> ToolRegistry {
        ToolRegistry {
            registry: self.clone(),
        }
    }
}

impl RegisteredSkill {
    fn descriptor(&self) -> SkillDescriptor {
        let mut descriptor = self.skill.descriptor();
        descriptor.toolset = self.toolset;
        descriptor
    }
}

#[derive(Clone, Default)]
pub struct ToolRegistry {
    registry: SkillRegistry,
}

impl ToolRegistry {
    pub fn new(registry: SkillRegistry) -> Self {
        Self { registry }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Skill>> {
        self.registry.get(name)
    }

    pub fn descriptors_for(&self, selection: &ToolsetSelection) -> Vec<SkillDescriptor> {
        let mut descriptors = self
            .registry
            .skills
            .values()
            .filter_map(|entry| {
                let descriptor = entry.descriptor();
                matches!(
                    tool_visibility(&descriptor, selection),
                    ToolVisibility::Direct | ToolVisibility::Deferred
                )
                .then_some(descriptor)
            })
            .collect::<Vec<_>>();
        descriptors.sort_by(|left, right| left.name.cmp(&right.name));
        descriptors
    }

    pub fn model_visible_names_for(&self, selection: &ToolsetSelection) -> Vec<String> {
        self.registry.model_visible_names_for(selection)
    }

    pub fn visibility_for(
        &self,
        name: &str,
        selection: &ToolsetSelection,
    ) -> Option<ToolVisibility> {
        self.registry.skills.get(name).map(|entry| {
            let descriptor = entry.descriptor();
            tool_visibility(&descriptor, selection)
        })
    }
}

fn tool_visibility(descriptor: &SkillDescriptor, selection: &ToolsetSelection) -> ToolVisibility {
    if descriptor.kind == SkillDescriptorKind::PromptSkill {
        return if selection.contains(descriptor.toolset) {
            ToolVisibility::Deferred
        } else {
            ToolVisibility::Disabled
        };
    }
    if descriptor.disable_model_invocation {
        return ToolVisibility::Hidden;
    }
    if !selection.contains(descriptor.toolset) {
        return ToolVisibility::Disabled;
    }
    if ToolsetSelection::default().contains(descriptor.toolset) {
        ToolVisibility::Direct
    } else {
        ToolVisibility::Deferred
    }
}

fn resolve_under_workspace(path: &Path, workspace_root: &Path) -> PathBuf {
    let workspace_root = canonicalize_path_for_policy(workspace_root);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    canonicalize_path_for_policy(&candidate)
}

fn canonicalize_path_for_policy(path: &Path) -> PathBuf {
    let normalized = normalize_path(path);
    if let Ok(canonical) = fs::canonicalize(&normalized) {
        return normalize_path(&canonical);
    }

    let mut missing = Vec::<OsString>::new();
    let mut current = normalized.as_path();
    loop {
        if let Some(name) = current.file_name() {
            missing.push(name.to_os_string());
        }

        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        if let Ok(canonical_parent) = fs::canonicalize(parent) {
            let mut rebuilt = canonical_parent;
            for component in missing.iter().rev() {
                rebuilt.push(component);
            }
            return normalize_path(&rebuilt);
        }
        current = parent;
    }

    normalized
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}
