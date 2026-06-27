// SPDX-License-Identifier: GPL-3.0-only

use crate::{McpStdioCallSkill, McpStdioProbeSkill, PluginCommandRunSkill, SkillEnvironment};
use ikaros_toolkit::{SkillRegistry, Toolset};

pub(super) fn register(registry: &mut SkillRegistry, env: &SkillEnvironment) {
    registry.register_with_toolset(
        PluginCommandRunSkill::new(env.skills_dir.clone()),
        Toolset::Plugin,
    );
    registry.register_with_toolset(McpStdioProbeSkill, Toolset::Plugin);
    registry.register_with_toolset(McpStdioCallSkill, Toolset::Plugin);
}
