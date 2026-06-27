// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    PersonaLoadSkill, SkillEnvironment, TaskSummarizeSkill, WebExtractSkill, WebSearchSkill,
};
use ikaros_toolkit::{SkillRegistry, Toolset};

pub(super) fn register(registry: &mut SkillRegistry, env: &SkillEnvironment) {
    registry.register_with_toolset(
        PersonaLoadSkill::new(env.persona_path.clone()),
        Toolset::Core,
    );
    registry.register_with_toolset(TaskSummarizeSkill, Toolset::Core);
    registry.register_with_toolset(WebExtractSkill, Toolset::Core);
    registry.register_with_toolset(
        WebSearchSkill::new(env.web_search_provider.clone()),
        Toolset::Core,
    );
}
