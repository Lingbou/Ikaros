// SPDX-License-Identifier: GPL-3.0-only

use super::types::RuntimeInitReport;
use ikaros_core::{IkarosConfig, IkarosError, IkarosPaths, Result};
use ikaros_soul::PersonaLoader;
use std::fs;

pub fn initialize_runtime_home(paths: &IkarosPaths) -> Result<RuntimeInitReport> {
    paths.ensure()?;
    let config_created = !paths.config.exists();
    if config_created {
        IkarosConfig::write_default_config(&paths.config)?;
    }
    let persona_created = !paths.persona.exists();
    if persona_created {
        fs::write(&paths.persona, PersonaLoader::default_markdown())
            .map_err(|source| IkarosError::io(&paths.persona, source))?;
    }
    Ok(RuntimeInitReport {
        home: paths.home.clone(),
        config: paths.config.clone(),
        persona: paths.persona.clone(),
        memory_dir: paths.memory_dir.clone(),
        rag_dir: paths.rag_dir.clone(),
        automation_dir: paths.automation_dir.clone(),
        gateway_dir: paths.gateway_dir.clone(),
        audit_dir: paths.audit_dir.clone(),
        config_created,
        persona_created,
    })
}
