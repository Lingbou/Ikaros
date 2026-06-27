// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_execution::harness::SkillRegistry;
use ikaros_providers::model::ModelUsageLedger;
use std::path::Path;

pub(in crate::chat) struct InteractiveCommandContext<'a> {
    pub(in crate::chat) config: &'a IkarosConfig,
    pub(in crate::chat) paths: &'a IkarosPaths,
    pub(in crate::chat) workspace: &'a Path,
    pub(in crate::chat) usage_ledger: &'a ModelUsageLedger,
    pub(in crate::chat) registry: &'a SkillRegistry,
}
