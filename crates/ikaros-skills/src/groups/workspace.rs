// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    FsReadSkill, FsWriteGuardedSkill, GitDiffSkill, GitStatusSkill, ListDirSkill, ShellGuardedSkill,
};
use ikaros_toolkit::{SkillRegistry, Toolset};

pub(super) fn register(registry: &mut SkillRegistry) {
    registry.register_with_toolset(FsReadSkill, Toolset::Workspace);
    registry.register_with_toolset(FsWriteGuardedSkill, Toolset::Workspace);
    registry.register_with_toolset(ListDirSkill, Toolset::Workspace);
    registry.register_with_toolset(ShellGuardedSkill, Toolset::Workspace);
    registry.register_with_toolset(GitStatusSkill, Toolset::Workspace);
    registry.register_with_toolset(GitDiffSkill, Toolset::Workspace);
}
