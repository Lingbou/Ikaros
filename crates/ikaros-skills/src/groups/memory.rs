// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    MemoryAppendSkill, MemoryCandidateCreateSkill, MemoryDeleteSkill, MemoryProjectionSkill,
    MemorySearchSkill, MemoryUpdateSkill, SkillEnvironment, WorkingMemoryListSkill,
};
use ikaros_toolkit::{SkillRegistry, Toolset};

pub(super) fn register(registry: &mut SkillRegistry, env: &SkillEnvironment) {
    registry.register_with_toolset(
        MemoryAppendSkill::new(env.memory_store.clone()),
        Toolset::Memory,
    );
    registry.register_with_toolset(
        MemorySearchSkill::new(env.memory_store.clone()),
        Toolset::Memory,
    );
    registry.register_with_toolset(
        MemoryCandidateCreateSkill::new(env.memory_store.clone()),
        Toolset::Memory,
    );
    registry.register_with_toolset(
        MemoryProjectionSkill::new(env.memory_store.clone()),
        Toolset::Memory,
    );
    registry.register_with_toolset(
        WorkingMemoryListSkill::new(env.memory_store.clone()),
        Toolset::Memory,
    );
    registry.register_with_toolset(
        MemoryUpdateSkill::new(env.memory_store.clone()),
        Toolset::Memory,
    );
    registry.register_with_toolset(
        MemoryDeleteSkill::new(env.memory_store.clone()),
        Toolset::Memory,
    );
}
