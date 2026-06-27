// SPDX-License-Identifier: GPL-3.0-only

use super::matching::slash_command_matches;
use super::model::{SlashCommandPermission, SlashCommandSurface};
use super::output::slash_commands_json_line;
use super::registry::slash_commands;
use super::suggest_slash_command;

#[test]
fn command_search_matches_session_aliases() {
    let matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| slash_command_matches(*command, "sess"))
        .map(|command| command.name)
        .collect::<Vec<_>>();

    assert!(matches.contains(&"/sessions"));
    assert!(matches.contains(&"/session"));
    assert!(matches.contains(&"/resume"));
}

#[test]
fn command_search_matches_all_multiword_terms() {
    let matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| slash_command_matches(*command, "screen palette"))
        .map(|command| command.name)
        .collect::<Vec<_>>();

    assert!(matches.contains(&"/screen"));
    assert!(!matches.contains(&"/sessions"));
}

#[test]
fn command_suggestion_finds_near_miss() {
    assert_eq!(suggest_slash_command("/sesions"), Some("/sessions"));
}

#[test]
fn command_metadata_json_line_exports_registry_fields() {
    let line = slash_commands_json_line(Some("provider"));
    let payload = line
        .strip_prefix("commands_json: ")
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .expect("commands JSON payload");
    let commands = payload["commands"].as_array().expect("commands array");
    let provider = commands
        .iter()
        .find(|command| command["name"] == "/provider")
        .expect("provider command metadata");

    assert_eq!(payload["query"], "provider");
    assert_eq!(
        provider["usage"],
        "/provider [inspect|health [--live]|matrix [--live] [--json]|profiles|debug]"
    );
    assert_eq!(
        provider["summary"],
        "inspect provider metadata, health, matrix, or debug JSON"
    );
    assert_eq!(
        provider["permissions"],
        serde_json::json!(["provider", "network"])
    );
    assert_eq!(
        provider["surfaces"],
        serde_json::json!(["workbench", "gateway", "acp"])
    );
    assert!(
        provider["tags"]
            .as_array()
            .expect("tags")
            .contains(&serde_json::json!("health"))
    );
}

#[test]
fn sandbox_command_metadata_is_discoverable() {
    let line = slash_commands_json_line(Some("sandbox"));
    let payload = line
        .strip_prefix("commands_json: ")
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .expect("commands JSON payload");
    let commands = payload["commands"].as_array().expect("commands array");
    let sandbox = commands
        .iter()
        .find(|command| command["name"] == "/sandbox")
        .expect("sandbox command metadata");

    assert_eq!(sandbox["usage"], "/sandbox [--probe]");
    assert_eq!(sandbox["argument_model"], "optional");
    assert_eq!(
        sandbox["outputs"],
        serde_json::json!(["human", "sandbox_json"])
    );
    assert_eq!(sandbox["permissions"], serde_json::json!(["read"]));
    assert_eq!(sandbox["surfaces"], serde_json::json!(["workbench"]));
}

#[test]
fn commands_declare_permissions_and_surfaces() {
    for command in slash_commands() {
        assert!(
            !command.permissions.is_empty(),
            "{} must declare permissions",
            command.name
        );
        assert!(
            !command.surfaces.is_empty(),
            "{} must declare supported surfaces",
            command.name
        );
    }

    let code = slash_commands()
        .iter()
        .find(|command| command.name == "/code")
        .expect("code command");
    assert!(
        code.permissions
            .contains(&SlashCommandPermission::WorkspaceWrite)
    );
    assert!(code.permissions.contains(&SlashCommandPermission::Shell));
    assert!(code.surfaces.contains(&SlashCommandSurface::Workbench));

    let approval = slash_commands()
        .iter()
        .find(|command| command.name == "/approval")
        .expect("approval command");
    assert!(
        approval
            .permissions
            .contains(&SlashCommandPermission::Approval)
    );

    let provider = slash_commands()
        .iter()
        .find(|command| command.name == "/provider")
        .expect("provider command");
    assert!(
        provider
            .permissions
            .contains(&SlashCommandPermission::Network)
    );
}

#[test]
fn command_registry_includes_workbench_loop_aliases() {
    let command_names = slash_commands()
        .iter()
        .map(|command| command.name)
        .collect::<Vec<_>>();

    assert!(command_names.contains(&"/approvals"));
    assert!(command_names.contains(&"/exit"));
}

#[test]
fn command_search_matches_permission_and_surface_metadata() {
    let gateway_safe = slash_commands()
        .iter()
        .copied()
        .filter(|command| slash_command_matches(*command, "gateway"))
        .map(|command| command.name)
        .collect::<Vec<_>>();

    assert!(gateway_safe.contains(&"/help"));
    assert!(gateway_safe.contains(&"/commands"));
    assert!(gateway_safe.contains(&"/session"));
    assert!(gateway_safe.contains(&"/provider"));

    let workspace_write = slash_commands()
        .iter()
        .copied()
        .filter(|command| slash_command_matches(*command, "workspace-write"))
        .map(|command| command.name)
        .collect::<Vec<_>>();

    assert!(workspace_write.contains(&"/code"));
}

#[test]
fn cancel_command_declares_interrupt_metadata() {
    let cancel = slash_commands()
        .iter()
        .find(|command| command.name == "/cancel")
        .expect("cancel command");

    assert!(cancel.tags.contains(&"interrupt"));
    assert!(
        cancel
            .permissions
            .contains(&SlashCommandPermission::SessionControl)
    );
    assert!(cancel.surfaces.contains(&SlashCommandSurface::Workbench));
}
