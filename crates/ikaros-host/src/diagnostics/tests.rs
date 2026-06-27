// SPDX-License-Identifier: GPL-3.0-only

use super::{initialize_runtime_home, runtime_doctor_report};
use ikaros_core::IkarosPaths;
use std::fs;

#[test]
fn init_report_creates_config_persona_and_runtime_dirs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = IkarosPaths::from_home(temp.path().join("home"));

    let first = initialize_runtime_home(&paths).expect("init");
    let second = initialize_runtime_home(&paths).expect("second init");

    assert!(first.config_created);
    assert!(first.persona_created);
    assert!(!second.config_created);
    assert!(!second.persona_created);
    assert!(paths.config.exists());
    assert!(paths.persona_dir.exists());
    assert!(paths.persona_profile.exists());
}

#[test]
fn init_migrates_legacy_persona_file_into_persona_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = IkarosPaths::from_home(temp.path().join("home"));
    fs::create_dir_all(&paths.home).expect("home");
    fs::write(
        paths.home.join("persona.md"),
        "# Identity\nname: Legacy Persona\nrole: migrated\n",
    )
    .expect("legacy persona");

    let report = initialize_runtime_home(&paths).expect("init");

    assert!(!report.persona_created);
    assert!(!paths.home.join("persona.md").exists());
    assert!(paths.persona_profile.exists());
    let migrated = fs::read_to_string(&paths.persona_profile).expect("profile");
    assert!(migrated.contains("Legacy Persona"));
}

#[test]
fn doctor_report_uses_protocol_defaults_without_remote_credentials() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(temp.path().join("home"));
    initialize_runtime_home(&paths).expect("init");

    let report = runtime_doctor_report(&paths, &workspace, Some("plan")).expect("doctor");

    assert_eq!(report.agent.name, "plan");
    assert_eq!(report.agent.mode, "plan");
    assert_eq!(report.model.provider, "openai-compatible");
    assert_eq!(report.model.model, "");
    assert!(!report.model.api_key_configured);
    assert_eq!(report.memory.backend, "jsonl");
    assert_eq!(report.memory_providers.active_local.id, "local-jsonl");
    assert!(report.memory_providers.external.is_empty());
    assert_eq!(report.rag.embedding_provider, "hash");
    assert_eq!(report.rag.embedding_model, "text-embedding-3-small");
    assert!(!report.rag.embedding_api_key_configured);
    assert!(report.skills.iter().any(|name| name == "memory_search"));
    assert!(report.audit_path.ends_with("audit.jsonl"));
}
