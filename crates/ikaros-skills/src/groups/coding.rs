// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    CodeEditGuardedSkill, CodeIterateSkill, CodeReviewSkill, CodeWorkflowSkill, RepoScanSkill,
    RunTestsSkill, SkillEnvironment,
};
use ikaros_toolkit::{SkillRegistry, Toolset};

pub(super) fn register(registry: &mut SkillRegistry, env: &SkillEnvironment) {
    registry.register_with_toolset(RepoScanSkill, Toolset::Coding);
    registry.register_with_toolset(RunTestsSkill, Toolset::Coding);
    registry.register_with_toolset(CodeEditGuardedSkill, Toolset::Coding);
    registry.register_with_toolset(CodeReviewSkill, Toolset::Coding);
    registry.register_with_toolset(CodeIterateSkill, Toolset::Coding);
    registry.register_with_toolset(
        CodeWorkflowSkill::new(env.coding_session.clone()),
        Toolset::Coding,
    );
}
