// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::workbench::{gateway_status_human_lines, print_gateway_status};
use crate::gateway::{run_gateway_adapter_workbench_command, run_gateway_daemon_workbench_command};
use anyhow::Result;
use ikaros_core::IkarosPaths;
use std::path::Path;

use super::{InteractiveChatRuntime, print_default_inline_lines};

pub(super) fn handle_gateway_command(
    args: Vec<&str>,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    match args.as_slice() {
        [] | ["status"] => {
            if runtime.default_inline_stdout() {
                print_default_inline_lines(gateway_status_human_lines(paths)?)?;
            } else {
                print_gateway_status(paths)?;
            }
        }
        ["daemon", rest @ ..] => {
            run_gateway_daemon_workbench_command(rest, paths, workspace, Some(&runtime.agent.name))?
        }
        ["adapter", rest @ ..] => {
            run_gateway_adapter_workbench_command(rest, paths)?;
        }
        ["help"] | ["--help"] => print_gateway_usage(runtime)?,
        _ => print_gateway_usage(runtime)?,
    }
    Ok(())
}

fn print_gateway_usage(runtime: &InteractiveChatRuntime) -> Result<()> {
    if runtime.default_inline_stdout() {
        print_default_inline_lines(vec![
            "* Gateway".to_owned(),
            "  commands: /gateway status, /gateway daemon status".to_owned(),
            "  controls: /gateway daemon start, /gateway daemon stop, /gateway daemon restart"
                .to_owned(),
            "  adapters: /gateway adapter list, /gateway adapter enqueue".to_owned(),
        ])?;
    } else {
        println!(
            "gateway_usage: /gateway [status|daemon status|daemon start|daemon stop|daemon restart|adapter list|adapter enqueue|adapter render-delivery]"
        );
    }
    Ok(())
}
