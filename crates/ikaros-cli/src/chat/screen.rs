// SPDX-License-Identifier: GPL-3.0-only

use super::{interactive::InteractiveChatRuntime, workbench};
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_models::ModelUsageLedger;
use ikaros_runtime::ChatRunOptions;
use std::path::Path;

pub(in crate::chat) fn refresh_persistent_workbench_screen(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
) -> Result<()> {
    workbench::print_screen_status_with_state(
        config,
        paths,
        workspace,
        runtime,
        options,
        usage_ledger,
        &runtime.screen_state,
    )?;
    Ok(())
}
