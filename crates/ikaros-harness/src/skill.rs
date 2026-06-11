// SPDX-License-Identifier: GPL-3.0-only

use crate::{PolicyRequest, policy::resolve_under_workspace, session::ExecutionSession};
use async_trait::async_trait;
use ikaros_core::{Plan, Result, RiskLevel};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path, path::PathBuf, sync::Arc};

#[derive(Debug, Clone)]
pub struct TaskGraph {
    pub plan: Plan,
}

#[derive(Clone)]
pub struct SkillContext {
    pub session: ExecutionSession,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillOutput {
    pub summary: String,
    pub output: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub risk_level: RiskLevel,
    pub kind: SkillDescriptorKind,
    pub disable_model_invocation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub support_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillDescriptorKind {
    ExecutableTool,
    PromptSkill,
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
        SkillDescriptor {
            name: self.name().into(),
            description: self.description().into(),
            input_schema: self.input_schema(),
            risk_level: self.risk_level(),
            kind: SkillDescriptorKind::ExecutableTool,
            disable_model_invocation: false,
            provenance: None,
            support_files: Vec::new(),
        }
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
    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput>;
}

#[derive(Clone, Default)]
pub struct SkillRegistry {
    skills: BTreeMap<String, Arc<dyn Skill>>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<S>(&mut self, skill: S)
    where
        S: Skill + 'static,
    {
        self.skills.insert(skill.name().into(), Arc::new(skill));
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Skill>> {
        self.skills.get(name).cloned()
    }

    pub fn names(&self) -> Vec<String> {
        self.skills.keys().cloned().collect()
    }

    pub fn descriptors(&self) -> Vec<SkillDescriptor> {
        self.skills
            .values()
            .map(|skill| skill.descriptor())
            .collect()
    }

    pub fn model_visible_names(&self) -> Vec<String> {
        self.skills
            .iter()
            .filter(|(_, skill)| !skill.descriptor().disable_model_invocation)
            .map(|(name, _)| name.clone())
            .collect()
    }
}

pub type ToolRegistry = SkillRegistry;
