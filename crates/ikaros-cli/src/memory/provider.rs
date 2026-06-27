// SPDX-License-Identifier: GPL-3.0-only

use super::{MemoryProviderCommand, MemoryProviderShow};
use anyhow::Result;
use ikaros_core::IkarosPaths;
use ikaros_host::memory_provider_registry;
use serde_json::json;

pub(super) fn memory_provider_command(
    command: MemoryProviderCommand,
    paths: &IkarosPaths,
) -> Result<()> {
    let registry = memory_provider_registry(paths)?;
    match command {
        MemoryProviderCommand::List => {
            println!("{}", serde_json::to_string_pretty(&registry)?);
        }
        MemoryProviderCommand::Active => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "local": &registry.active_local,
                    "external": registry.active_external(),
                    "issues": &registry.issues,
                }))?
            );
        }
        MemoryProviderCommand::Show(MemoryProviderShow { id }) => {
            if registry.active_local.id == id {
                println!("{}", serde_json::to_string_pretty(&registry.active_local)?);
                return Ok(());
            }
            let provider = registry
                .external
                .iter()
                .find(|provider| provider.id == id)
                .ok_or_else(|| anyhow::anyhow!("memory provider not found: {}", id))?;
            println!("{}", serde_json::to_string_pretty(provider)?);
        }
    }
    Ok(())
}
