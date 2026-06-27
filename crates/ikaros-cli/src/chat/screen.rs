// SPDX-License-Identifier: GPL-3.0-only

use super::{interactive::InteractiveChatRuntime, workbench};
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_models::ModelUsageLedger;
use ikaros_runtime::ChatRunOptions;
use std::path::Path;

pub(in crate::chat) fn sync_fullscreen_terminal_session(
    runtime: &mut InteractiveChatRuntime,
    fullscreen_terminal: &mut Option<workbench::PersistentWorkbenchTerminal>,
) -> Result<()> {
    if !runtime.persistent_fullscreen || !runtime.screen_state.fullscreen() {
        *fullscreen_terminal = None;
        return Ok(());
    }
    if fullscreen_terminal.is_some() {
        return Ok(());
    }
    match workbench::PersistentWorkbenchTerminal::enter() {
        Ok(Some(session)) => {
            *fullscreen_terminal = Some(session);
            Ok(())
        }
        Ok(None) => {
            eprintln!("warning: fullscreen unavailable; using line input");
            runtime.persistent_fullscreen = false;
            workbench::apply_workbench_screen_args(&mut runtime.screen_state, &["--inline"])?;
            *fullscreen_terminal = None;
            Ok(())
        }
        Err(error) => {
            eprintln!(
                "warning: fullscreen unavailable; using line input: {}",
                workbench::terminal_inline(&error.to_string())
            );
            runtime.persistent_fullscreen = false;
            workbench::apply_workbench_screen_args(&mut runtime.screen_state, &["--inline"])?;
            *fullscreen_terminal = None;
            Ok(())
        }
    }
}

pub(in crate::chat) fn refresh_persistent_workbench_screen(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
    fullscreen_terminal: Option<&mut workbench::PersistentWorkbenchTerminal>,
) -> Result<()> {
    if runtime.persistent_fullscreen && runtime.screen_state.fullscreen() {
        if let Some(terminal) = fullscreen_terminal {
            workbench::draw_persistent_screen_status_with_state(
                workbench::WorkbenchScreenStatusContext {
                    config,
                    paths,
                    workspace,
                    runtime,
                    options,
                    usage_ledger,
                },
                &runtime.screen_state,
                terminal,
            )?;
        } else {
            workbench::print_persistent_screen_status_with_state(
                config,
                paths,
                workspace,
                runtime,
                options,
                usage_ledger,
                &runtime.screen_state,
            )?;
        }
    } else {
        workbench::print_screen_status_with_state(
            config,
            paths,
            workspace,
            runtime,
            options,
            usage_ledger,
            &runtime.screen_state,
        )?;
    }
    Ok(())
}
