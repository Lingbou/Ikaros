// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    BrowserActivateTargetSkill, BrowserCdpSkill, BrowserClickSkill, BrowserCloseTargetSkill,
    BrowserListSkill, BrowserNavigateSkill, BrowserNewTargetSkill, BrowserScreenshotSkill,
    BrowserScrollSkill, BrowserSnapshotSkill, BrowserStatusSkill, BrowserTypeSkill,
};
use ikaros_toolkit::{SkillRegistry, Toolset};

pub(super) fn register(registry: &mut SkillRegistry) {
    registry.register_with_toolset(BrowserStatusSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserListSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserNewTargetSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserActivateTargetSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserCloseTargetSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserNavigateSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserSnapshotSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserClickSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserTypeSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserScrollSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserScreenshotSkill, Toolset::Plugin);
    registry.register_with_toolset(BrowserCdpSkill, Toolset::Plugin);
}
