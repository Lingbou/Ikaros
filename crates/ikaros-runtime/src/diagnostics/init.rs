// SPDX-License-Identifier: GPL-3.0-only

use super::types::RuntimeInitReport;
use ikaros_core::{IkarosConfig, IkarosError, IkarosPaths, Result};
use ikaros_soul::PersonaLoader;
use std::fs;

pub fn initialize_runtime_home(paths: &IkarosPaths) -> Result<RuntimeInitReport> {
    initialize_runtime_home_with_options(paths, false)
}

pub fn initialize_runtime_home_with_options(
    paths: &IkarosPaths,
    full_config: bool,
) -> Result<RuntimeInitReport> {
    let legacy_persona = paths.home.join("persona.md");
    let persona_created = !paths.persona_profile.exists() && !legacy_persona.exists();
    paths.ensure()?;
    let config_created = !paths.config.exists();
    if config_created {
        if full_config {
            IkarosConfig::write_full_config(&paths.config)?;
        } else {
            IkarosConfig::write_default_config(&paths.config)?;
        }
    }
    if !paths.persona_profile.exists() && legacy_persona.exists() {
        fs::rename(&legacy_persona, &paths.persona_profile)
            .map_err(|source| IkarosError::io(&paths.persona_profile, source))?;
    } else if persona_created {
        PersonaLoader::write_default_bundle(&paths.persona_dir)?;
    }
    Ok(RuntimeInitReport {
        home: paths.home.clone(),
        config: paths.config.clone(),
        persona_dir: paths.persona_dir.clone(),
        persona_profile: paths.persona_profile.clone(),
        memory_dir: paths.memory_dir.clone(),
        rag_dir: paths.rag_dir.clone(),
        automation_dir: paths.automation_dir.clone(),
        gateway_dir: paths.gateway_dir.clone(),
        audit_dir: paths.audit_dir.clone(),
        config_created,
        persona_created,
    })
}
