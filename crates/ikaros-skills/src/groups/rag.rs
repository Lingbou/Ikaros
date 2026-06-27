// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    RagDeletePathSkill, RagDeleteScopeSkill, RagIngestSkill, RagReindexSkill, RagSearchSkill,
    RagStaleSkill, SkillEnvironment,
};
use ikaros_toolkit::{SkillRegistry, Toolset};

pub(super) fn register(registry: &mut SkillRegistry, env: &SkillEnvironment) {
    registry.register_with_toolset(
        RagIngestSkill::new(
            env.rag_index.clone(),
            env.rag_config.clone(),
            env.rag_provider.clone(),
        ),
        Toolset::Rag,
    );
    registry.register_with_toolset(
        RagSearchSkill::new(
            env.rag_index.clone(),
            env.rag_config.clone(),
            env.rag_provider.clone(),
        ),
        Toolset::Rag,
    );
    registry.register_with_toolset(RagStaleSkill::new(env.rag_index.clone()), Toolset::Rag);
    registry.register_with_toolset(
        RagDeleteScopeSkill::new(env.rag_index.clone()),
        Toolset::Rag,
    );
    registry.register_with_toolset(RagDeletePathSkill::new(env.rag_index.clone()), Toolset::Rag);
    registry.register_with_toolset(
        RagReindexSkill::new(
            env.rag_index.clone(),
            env.rag_config.clone(),
            env.rag_provider.clone(),
        ),
        Toolset::Rag,
    );
}
