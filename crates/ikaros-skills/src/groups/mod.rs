// SPDX-License-Identifier: GPL-3.0-only

mod browser;
mod coding;
mod core;
mod memory;
mod plugin;
mod rag;
mod voice;
mod workspace;

use crate::{SkillEnvironment, ToolCallSkill, ToolDescribeSkill, ToolSearchSkill, prompt_docs};
use ikaros_toolkit::{SkillRegistry, Toolset};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BuiltinSkillGroup {
    Core,
    Workspace,
    Memory,
    Rag,
    Coding,
    Browser,
    Voice,
    Plugin,
    PromptSkills,
    ToolBridge,
}

impl BuiltinSkillGroup {
    pub const ALL: &'static [Self] = &[
        Self::Core,
        Self::Workspace,
        Self::Memory,
        Self::Rag,
        Self::Coding,
        Self::Browser,
        Self::Voice,
        Self::Plugin,
        Self::PromptSkills,
        Self::ToolBridge,
    ];
}

#[derive(Debug, Clone)]
pub struct BuiltinRegistryBuilder {
    env: SkillEnvironment,
    groups: BTreeSet<BuiltinSkillGroup>,
}

impl BuiltinRegistryBuilder {
    pub fn new(env: SkillEnvironment) -> Self {
        Self {
            env,
            groups: BuiltinSkillGroup::ALL.iter().copied().collect(),
        }
    }

    pub fn empty(env: SkillEnvironment) -> Self {
        Self {
            env,
            groups: BTreeSet::new(),
        }
    }

    pub fn with_group(mut self, group: BuiltinSkillGroup) -> Self {
        self.groups.insert(group);
        self
    }

    pub fn with_groups(mut self, groups: impl IntoIterator<Item = BuiltinSkillGroup>) -> Self {
        self.groups.extend(groups);
        self
    }

    pub fn without_group(mut self, group: BuiltinSkillGroup) -> Self {
        self.groups.remove(&group);
        self
    }

    pub fn build(self) -> SkillRegistry {
        let mut registry = SkillRegistry::new();
        for group in BuiltinSkillGroup::ALL {
            if !self.groups.contains(group) {
                continue;
            }
            match group {
                BuiltinSkillGroup::Core => core::register(&mut registry, &self.env),
                BuiltinSkillGroup::Workspace => workspace::register(&mut registry),
                BuiltinSkillGroup::Memory => memory::register(&mut registry, &self.env),
                BuiltinSkillGroup::Rag => rag::register(&mut registry, &self.env),
                BuiltinSkillGroup::Coding => coding::register(&mut registry, &self.env),
                BuiltinSkillGroup::Browser => browser::register(&mut registry),
                BuiltinSkillGroup::Voice => voice::register(&mut registry, &self.env),
                BuiltinSkillGroup::Plugin => plugin::register(&mut registry, &self.env),
                BuiltinSkillGroup::PromptSkills => {
                    register_prompt_skill_documents(&mut registry, &self.env)
                }
                BuiltinSkillGroup::ToolBridge => register_tool_bridge(&mut registry),
            }
        }
        registry
    }
}

fn register_prompt_skill_documents(registry: &mut SkillRegistry, env: &SkillEnvironment) {
    for document in prompt_docs::load_prompt_skill_documents(&env.skills_dir) {
        registry.register_prompt_skill_document(document);
    }
}

fn register_tool_bridge(registry: &mut SkillRegistry) {
    let deferred_registry = registry.clone();
    registry.register_with_toolset(
        ToolSearchSkill::new(deferred_registry.clone()),
        Toolset::Core,
    );
    registry.register_with_toolset(
        ToolDescribeSkill::new(deferred_registry.clone()),
        Toolset::Core,
    );
    registry.register_with_toolset(ToolCallSkill::new(deferred_registry), Toolset::Core);
}
