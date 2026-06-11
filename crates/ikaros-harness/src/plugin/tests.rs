// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use ikaros_core::RiskLevel;
use std::{fs, path::PathBuf};

#[test]
fn plugin_catalog_loads_nested_manifest_and_declared_skills() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plugin_dir = temp.path().join("hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Read-only sample plugin."
capabilities = ["demo"]

[[skills]]
name = "greet"
description = "Greet a user."
risk = "safe_read"
input_schema = { type = "object", properties = { name = { type = "string" } } }

[[skills.permissions]]
action = "greet"
risk = "safe_read"
"#,
    )
    .expect("manifest");

    let catalog = PluginCatalog::load(temp.path()).expect("catalog");
    assert_eq!(catalog.plugin_count(), 1);
    assert_eq!(catalog.enabled_plugin_count(), 1);
    assert_eq!(catalog.disabled_plugin_count(), 0);
    assert_eq!(catalog.declared_skill_count(), 1);
    assert_eq!(catalog.declared_skill_names(), vec!["hello.greet"]);
    let (_plugin, skill) = catalog.find_skill("hello.greet").expect("skill");
    assert_eq!(skill.risk, RiskLevel::SafeRead);
    assert!(catalog.warnings.is_empty());
}

#[test]
fn plugin_catalog_loads_command_backed_skill_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plugin_dir = temp.path().join("hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Command-backed sample plugin."

[[skills]]
name = "echo"
description = "Echo input."
risk = "safe_read"

[skills.command]
program = "bin/echo.sh"
args = ["--json"]
timeout_ms = 1000
"#,
    )
    .expect("manifest");

    let catalog = PluginCatalog::load(temp.path()).expect("catalog");
    let (_plugin, skill) = catalog.find_skill("hello.echo").expect("skill");
    let command = skill.command.as_ref().expect("command");
    assert_eq!(command.program, PathBuf::from("bin/echo.sh"));
    assert_eq!(command.args, vec!["--json"]);
    assert_eq!(command.timeout_ms, Some(1000));
}

#[test]
fn plugin_catalog_rejects_unsafe_command_program_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("bad.toml"),
        r#"
name = "bad"
version = "0.1.0"
description = "Unsafe command plugin."

[[skills]]
name = "run"
description = "Unsafe."
risk = "safe_read"

[skills.command]
program = "../outside.sh"
"#,
    )
    .expect("manifest");

    let catalog = PluginCatalog::load(temp.path()).expect("catalog");
    assert_eq!(catalog.plugin_count(), 0);
    assert_eq!(catalog.warnings.len(), 1);
    assert!(
        catalog.warnings[0]
            .message
            .contains("program must be relative")
    );
}

#[test]
fn plugin_catalog_rejects_command_timeout_above_limit() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("bad-timeout.toml"),
        format!(
            r#"
name = "bad-timeout"
version = "0.1.0"
description = "Timeout abuse plugin."

[[skills]]
name = "run"
description = "Unsafe timeout."
risk = "safe_read"

[skills.command]
program = "bin/run.sh"
timeout_ms = {}
"#,
            PLUGIN_COMMAND_MAX_TIMEOUT_MS + 1
        ),
    )
    .expect("manifest");

    let catalog = PluginCatalog::load(temp.path()).expect("catalog");
    assert_eq!(catalog.plugin_count(), 0);
    assert_eq!(catalog.warnings.len(), 1);
    assert!(catalog.warnings[0].message.contains("timeout_ms"));
    assert!(
        catalog.warnings[0]
            .message
            .contains(&PLUGIN_COMMAND_MAX_TIMEOUT_MS.to_string())
    );
}

#[test]
fn plugin_catalog_rejects_command_args_with_control_characters() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("bad-args.toml"),
        r#"
name = "bad-args"
version = "0.1.0"
description = "Argument abuse plugin."

[[skills]]
name = "run"
description = "Unsafe args."
risk = "safe_read"

[skills.command]
program = "bin/run.sh"
args = ["safe", "bad\narg"]
"#,
    )
    .expect("manifest");

    let catalog = PluginCatalog::load(temp.path()).expect("catalog");
    assert_eq!(catalog.plugin_count(), 0);
    assert_eq!(catalog.warnings.len(), 1);
    assert!(catalog.warnings[0].message.contains("control characters"));
}

#[test]
fn plugin_marketplace_applies_order_and_enabled_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("marketplace.toml"),
        r#"
[[plugins]]
name = "disabled"
enabled = false
priority = 1
source = "local"
path = "disabled"
repository = "https://example.invalid/disabled"
tags = ["sample"]

[[plugins]]
name = "enabled"
enabled = true
priority = 2
source = "local"
"#,
    )
    .expect("marketplace");
    for name in ["enabled", "disabled"] {
        let plugin_dir = temp.path().join(name);
        fs::create_dir_all(&plugin_dir).expect("plugin dir");
        fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                r#"
name = "{name}"
version = "0.1.0"
description = "Sample plugin."

[[skills]]
name = "run"
description = "Declared skill."
risk = "safe_read"
"#
            ),
        )
        .expect("manifest");
    }

    let catalog = PluginCatalog::load(temp.path()).expect("catalog");
    assert_eq!(catalog.plugin_count(), 2);
    assert_eq!(catalog.enabled_plugin_count(), 1);
    assert_eq!(catalog.disabled_plugin_count(), 1);
    assert_eq!(
        catalog
            .plugins
            .iter()
            .map(|plugin| plugin.manifest.name.as_str())
            .collect::<Vec<_>>(),
        vec!["disabled", "enabled"]
    );
    assert_eq!(catalog.declared_skill_names(), vec!["enabled.run"]);
    assert!(catalog.find_skill("disabled.run").is_none());
    let (plugin, _skill) = catalog
        .find_declared_skill("disabled.run")
        .expect("disabled declared skill");
    assert!(!plugin.marketplace.enabled);
    assert_eq!(plugin.marketplace.priority, 1);
    assert!(catalog.warnings.is_empty());
}

#[test]
fn plugin_marketplace_reports_unmatched_entries() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("marketplace.toml"),
        r#"
[[plugins]]
name = "missing"
enabled = true
"#,
    )
    .expect("marketplace");

    let catalog = PluginCatalog::load(temp.path()).expect("catalog");
    assert_eq!(catalog.plugin_count(), 0);
    assert_eq!(catalog.warnings.len(), 1);
    assert!(catalog.warnings[0].message.contains("no matching plugin"));
}

#[test]
fn plugin_management_enables_and_disables_installed_plugin() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plugin_dir = temp.path().join("hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Managed sample plugin."

[[skills]]
name = "run"
description = "Declared skill."
risk = "safe_read"
"#,
    )
    .expect("manifest");

    let disabled = set_plugin_enabled(temp.path(), "hello", false).expect("disable plugin");
    assert_eq!(disabled.name, "hello");
    assert!(!disabled.enabled);
    assert!(disabled.entry_added);
    let catalog = PluginCatalog::load(temp.path()).expect("catalog");
    assert_eq!(catalog.enabled_plugin_count(), 0);
    assert!(catalog.find_skill("hello.run").is_none());

    let enabled = set_plugin_enabled(temp.path(), "hello", true).expect("enable plugin");
    assert!(enabled.enabled);
    assert!(!enabled.entry_added);
    let catalog = PluginCatalog::load(temp.path()).expect("catalog");
    assert_eq!(catalog.enabled_plugin_count(), 1);
    assert!(catalog.find_skill("hello.run").is_some());

    let marketplace = fs::read_to_string(temp.path().join("marketplace.toml"))
        .expect("marketplace should be written");
    assert!(marketplace.contains("name = \"hello\""));
    assert!(marketplace.contains("enabled = true"));
    assert!(marketplace.contains("path = \"hello\""));
}

#[test]
fn plugin_validation_reports_command_metadata_without_executing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plugin_dir = temp.path().join("hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Validation sample plugin."

[[skills]]
name = "echo"
description = "Echo input."
risk = "safe_read"

[skills.command]
program = "bin/echo.sh"
"#,
    )
    .expect("manifest");

    let report = validate_plugin_file(&plugin_dir).expect("validation report");
    assert_eq!(report.name, "hello");
    assert_eq!(report.skill_count, 1);
    assert_eq!(report.command_skill_count, 1);
    assert_eq!(report.risk_levels, vec![RiskLevel::SafeRead]);
    assert_eq!(
        report.missing_command_paths,
        vec![PathBuf::from("bin/echo.sh")]
    );
}

#[test]
fn plugin_audit_reports_disabled_plugins_warnings_and_missing_commands() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("marketplace.toml"),
        r#"
[[plugins]]
name = "hello"
enabled = false
priority = 7
source = "local"
path = "hello"
"#,
    )
    .expect("marketplace");

    let plugin_dir = temp.path().join("hello");
    fs::create_dir_all(&plugin_dir).expect("plugin dir");
    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Audit sample plugin."

[[skills]]
name = "echo"
description = "Echo input."
risk = "safe_read"

[skills.command]
program = "bin/missing.sh"
"#,
    )
    .expect("manifest");
    fs::write(
        temp.path().join("bad.toml"),
        r#"
name = "bad"
version = "0.1.0"
description = "Missing declared skills."
"#,
    )
    .expect("bad manifest");

    let report = audit_plugins(temp.path()).expect("audit report");
    assert_eq!(report.plugin_count, 1);
    assert_eq!(report.enabled_plugin_count, 0);
    assert_eq!(report.disabled_plugin_count, 1);
    assert_eq!(report.skill_count, 1);
    assert_eq!(report.enabled_skill_count, 0);
    assert_eq!(report.command_skill_count, 1);
    assert_eq!(report.warning_count, 1);
    assert_eq!(report.missing_command_count, 1);

    let plugin = &report.plugins[0];
    assert_eq!(plugin.name, "hello");
    assert!(!plugin.enabled);
    assert_eq!(plugin.priority, 7);
    assert_eq!(plugin.skill_count, 1);
    assert_eq!(plugin.enabled_skill_count, 0);
    assert_eq!(plugin.command_skill_count, 1);
    assert_eq!(plugin.risk_levels, vec![RiskLevel::SafeRead]);
    assert_eq!(plugin.missing_commands.len(), 1);
    assert_eq!(plugin.missing_commands[0].skill_name, "hello.echo");
    assert_eq!(
        plugin.missing_commands[0].program,
        PathBuf::from("bin/missing.sh")
    );
    assert!(report.warnings[0].message.contains("at least one skill"));
}

#[test]
fn plugin_install_copies_local_plugin_disabled_by_default() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source_dir = temp.path().join("source/hello");
    fs::create_dir_all(source_dir.join("bin")).expect("plugin bin dir");
    fs::write(
        source_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Installable sample plugin."

[[skills]]
name = "echo"
description = "Echo input."
risk = "safe_read"

[skills.command]
program = "bin/echo.sh"
"#,
    )
    .expect("manifest");
    fs::write(source_dir.join("bin/echo.sh"), "cat\n").expect("command file");

    let skills_dir = temp.path().join("skills");
    let report =
        install_local_plugin(&skills_dir, &source_dir, false, false).expect("install plugin");
    assert_eq!(report.name, "hello");
    assert_eq!(report.version, "0.1.0");
    assert!(!report.enabled);
    assert!(!report.replaced);
    assert_eq!(report.skill_count, 1);
    assert_eq!(report.command_skill_count, 1);
    assert!(skills_dir.join("hello/plugin.toml").exists());
    assert!(skills_dir.join("hello/bin/echo.sh").exists());

    let catalog = PluginCatalog::load(&skills_dir).expect("catalog");
    assert_eq!(catalog.plugin_count(), 1);
    assert_eq!(catalog.enabled_plugin_count(), 0);
    assert!(catalog.find_skill("hello.echo").is_none());

    let update = set_plugin_enabled(&skills_dir, "hello", true).expect("enable installed plugin");
    assert!(update.enabled);
    let catalog = PluginCatalog::load(&skills_dir).expect("catalog");
    assert!(catalog.find_skill("hello.echo").is_some());
}

#[test]
fn plugin_install_rejects_missing_command_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source_dir = temp.path().join("source/hello");
    fs::create_dir_all(&source_dir).expect("plugin dir");
    fs::write(
        source_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Broken command plugin."

[[skills]]
name = "echo"
description = "Echo input."
risk = "safe_read"

[skills.command]
program = "bin/missing.sh"
"#,
    )
    .expect("manifest");

    let error = install_local_plugin(temp.path().join("skills"), &source_dir, false, false)
        .expect_err("missing command should reject install");
    assert!(error.to_string().contains("missing command path"));
}

#[test]
fn plugin_install_rejects_path_reserved_dot_names() {
    for name in [".", ".."] {
        let temp = tempfile::tempdir().expect("tempdir");
        let source_dir = temp.path().join("source/plugin");
        fs::create_dir_all(&source_dir).expect("plugin dir");
        fs::write(
            source_dir.join("plugin.toml"),
            format!(
                r#"
name = "{name}"
version = "0.1.0"
description = "Invalid plugin name."

[[skills]]
name = "run"
description = "Declared skill."
risk = "safe_read"
"#
            ),
        )
        .expect("manifest");
        let skills_dir = temp.path().join("skills");
        fs::create_dir_all(&skills_dir).expect("skills dir");
        fs::write(skills_dir.join("keep.txt"), "keep").expect("keep");

        let error = install_local_plugin(&skills_dir, &source_dir, false, true)
            .expect_err("dot-only plugin name should reject install");

        assert!(error.to_string().contains("path-reserved dot component"));
        assert!(skills_dir.join("keep.txt").exists());
    }
}

#[test]
fn plugin_uninstall_removes_local_files_and_marketplace_entry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source_dir = temp.path().join("source/hello");
    fs::create_dir_all(source_dir.join("bin")).expect("plugin bin dir");
    fs::write(
        source_dir.join("plugin.toml"),
        r#"
name = "hello"
version = "0.1.0"
description = "Uninstallable sample plugin."

[[skills]]
name = "echo"
description = "Echo input."
risk = "safe_read"

[skills.command]
program = "bin/echo.sh"
"#,
    )
    .expect("manifest");
    fs::write(source_dir.join("bin/echo.sh"), "cat\n").expect("command file");

    let skills_dir = temp.path().join("skills");
    install_local_plugin(&skills_dir, &source_dir, true, false).expect("install plugin");
    assert!(skills_dir.join("hello/plugin.toml").exists());
    let catalog = PluginCatalog::load(&skills_dir).expect("catalog");
    assert!(catalog.find_skill("hello.echo").is_some());

    let report = uninstall_local_plugin(&skills_dir, "hello").expect("uninstall plugin");
    assert_eq!(report.name, "hello");
    assert!(report.marketplace_entry_removed);
    assert!(!skills_dir.join("hello").exists());
    let catalog = PluginCatalog::load(&skills_dir).expect("catalog");
    assert_eq!(catalog.plugin_count(), 0);
    let marketplace = fs::read_to_string(skills_dir.join("marketplace.toml")).expect("marketplace");
    assert!(!marketplace.contains("name = \"hello\""));
}

#[test]
fn plugin_management_rejects_missing_plugin() {
    let temp = tempfile::tempdir().expect("tempdir");
    let error = set_plugin_enabled(temp.path(), "missing", false).expect_err("missing plugin");
    assert!(error.to_string().contains("plugin not found"));
}

#[test]
fn plugin_catalog_keeps_invalid_manifest_as_warning() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("bad.toml"),
        r#"
name = "bad"
version = "0.1.0"
description = "Missing declared skills."
"#,
    )
    .expect("manifest");

    let catalog = PluginCatalog::load(temp.path()).expect("catalog");
    assert_eq!(catalog.plugin_count(), 0);
    assert_eq!(catalog.warnings.len(), 1);
    assert!(catalog.warnings[0].message.contains("at least one skill"));
}

#[test]
fn plugin_catalog_redacts_manifest_text_before_exposure() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(
        temp.path().join("redacted.toml"),
        r#"
name = "redacted"
version = "0.1.0"
description = "Example with sk-test-secret-value in prose."

[[skills]]
name = "show"
description = "Uses api_key=abc in an example."
risk = "SafeRead"
input_schema = { type = "object", properties = { token = { type = "string" } } }
"#,
    )
    .expect("manifest");

    let catalog = PluginCatalog::load(temp.path()).expect("catalog");
    let encoded = serde_json::to_string(&catalog).expect("json");
    assert!(!encoded.contains("sk-test-secret-value"));
    assert!(!encoded.contains("api_key=abc"));
    assert!(encoded.contains("[REDACTED_SECRET]"));
}
