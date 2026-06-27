// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosConfig, IkarosPaths, Result};
use ikaros_state::memory::{
    JsonlMemoryCandidateStore, LocalMemoryStore, MemoryProjectionFileStore, MemoryProviderRegistry,
};

pub struct MemoryProjectionStores {
    pub memory: LocalMemoryStore,
    pub projection_files: MemoryProjectionFileStore,
}

pub struct MemoryCandidateStores {
    pub memory: LocalMemoryStore,
    pub candidates: JsonlMemoryCandidateStore,
}

pub fn local_memory_store(paths: &IkarosPaths) -> Result<LocalMemoryStore> {
    let config = IkarosConfig::load(&paths.config)?;
    LocalMemoryStore::new(&paths.memory_dir, &config.memory.backend)
}

pub fn memory_projection_stores(paths: &IkarosPaths) -> Result<MemoryProjectionStores> {
    Ok(MemoryProjectionStores {
        memory: local_memory_store(paths)?,
        projection_files: MemoryProjectionFileStore::new(&paths.memory_dir),
    })
}

pub fn memory_candidate_stores(paths: &IkarosPaths) -> Result<MemoryCandidateStores> {
    Ok(MemoryCandidateStores {
        memory: local_memory_store(paths)?,
        candidates: JsonlMemoryCandidateStore::new(&paths.memory_dir),
    })
}

pub fn memory_provider_registry(paths: &IkarosPaths) -> Result<MemoryProviderRegistry> {
    let config = IkarosConfig::load(&paths.config)?;
    MemoryProviderRegistry::from_config(
        &paths.memory_dir,
        &config.memory.backend,
        &config.memory.external_providers,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikaros_state::memory::MemoryStore;

    #[test]
    fn memory_store_helpers_use_configured_memory_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = IkarosPaths::from_home(temp.path().join("home"));
        paths.ensure().expect("paths");
        std::fs::write(
            &paths.config,
            r#"
schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag:
  embedding_provider: hash
  embedding_model: text-embedding-3-small

voice:
  tts:
    provider: mock
    model: mock-tts
    voice: default
  asr:
    provider: mock
    model: mock-asr
"#,
        )
        .expect("config");

        let stores = memory_candidate_stores(&paths).expect("candidate stores");
        assert!(stores.memory.path().starts_with(&paths.memory_dir));
        assert!(stores.candidates.path().starts_with(&paths.memory_dir));

        let projection = memory_projection_stores(&paths).expect("projection stores");
        assert!(projection.memory.path().starts_with(&paths.memory_dir));
        assert_eq!(
            projection.projection_files.dir(),
            paths.memory_dir.join("projections")
        );
    }
}
