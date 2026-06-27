// SPDX-License-Identifier: GPL-3.0-only

use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const TARGET_CRATES: &[&str] = &[
    "ikaros-core",
    "ikaros-protocol",
    "ikaros-state",
    "ikaros-execution",
    "ikaros-host",
    "ikaros-providers",
    "ikaros-agent",
    "ikaros-skills",
    "ikaros-surfaces",
    "ikaros-terminal",
    "ikaros-cli",
];

const CLI_ALLOWED_IKAROS_PACKAGES: &[&str] = &[
    "ikaros-core",
    "ikaros-protocol",
    "ikaros-state",
    "ikaros-execution",
    "ikaros-host",
    "ikaros-providers",
    "ikaros-agent",
    "ikaros-skills",
    "ikaros-surfaces",
    "ikaros-terminal",
];

const FACADE_DEPENDENCY_RULES: &[(&str, &[&str])] = &[
    (
        "ikaros-state",
        &[
            "ikaros-agent",
            "ikaros-execution",
            "ikaros-providers",
            "ikaros-surfaces",
            "ikaros-terminal",
        ],
    ),
    (
        "ikaros-execution",
        &[
            "ikaros-agent",
            "ikaros-providers",
            "ikaros-surfaces",
            "ikaros-terminal",
        ],
    ),
    (
        "ikaros-providers",
        &[
            "ikaros-agent",
            "ikaros-execution",
            "ikaros-state",
            "ikaros-surfaces",
            "ikaros-terminal",
        ],
    ),
    ("ikaros-surfaces", &["ikaros-agent", "ikaros-terminal"]),
    (
        "ikaros-terminal",
        &[
            "ikaros-agent",
            "ikaros-execution",
            "ikaros-providers",
            "ikaros-state",
            "ikaros-surfaces",
        ],
    ),
];

const EXECUTION_CONSUMER_PACKAGES: &[&str] = &["ikaros-host", "ikaros-skills"];

const LEGACY_EXECUTION_PACKAGES: &[&str] = &["ikaros-harness", "ikaros-sandbox", "ikaros-toolkit"];

const STATE_CONSUMER_PACKAGES: &[&str] = &["ikaros-host", "ikaros-skills"];

const LEGACY_STATE_PACKAGES: &[&str] = &[
    "ikaros-automation",
    "ikaros-gateway",
    "ikaros-memory",
    "ikaros-rag",
    "ikaros-session",
];

const PROVIDER_CONSUMER_PACKAGES: &[&str] = &["ikaros-skills"];

const LEGACY_PROVIDER_PACKAGES: &[&str] = &["ikaros-models", "ikaros-voice"];

const HOST_FORBIDDEN_DEPENDENCIES: &[&str] = &[
    "ikaros-cli",
    "ikaros-runtime",
    "ikaros-surfaces",
    "ikaros-terminal",
];

#[test]
fn target_architecture_facade_crates_exist() {
    let metadata = workspace_metadata();
    let package_names = package_names(&metadata);

    let missing = TARGET_CRATES
        .iter()
        .filter(|name| !package_names.contains(**name))
        .copied()
        .collect::<Vec<_>>();

    assert!(
        missing.is_empty(),
        "target architecture crates are missing from the workspace: {missing:?}"
    );
}

#[test]
fn legacy_runtime_crate_is_removed_from_workspace_and_filesystem() {
    let metadata = workspace_metadata();
    let package_names = package_names(&metadata);
    assert!(
        !package_names.contains("ikaros-runtime"),
        "application use cases now live in ikaros-agent and host composition lives in ikaros-host; ikaros-runtime should not remain a workspace package"
    );
    assert!(
        !workspace_root().join("crates/ikaros-runtime").exists(),
        "migrated legacy runtime source directory should be deleted instead of lingering as an unused shim"
    );
}

#[test]
fn cli_depends_on_architecture_facades_instead_of_legacy_internal_crates() {
    let metadata = workspace_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let cli = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-cli"))
        .expect("ikaros-cli package should exist");
    let allowed = CLI_ALLOWED_IKAROS_PACKAGES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    let legacy = cli["dependencies"]
        .as_array()
        .expect("ikaros-cli dependencies should be an array")
        .iter()
        .filter_map(|dependency| {
            let package = dependency["package"]
                .as_str()
                .or_else(|| dependency["name"].as_str())?;
            package.starts_with("ikaros-").then_some(package)
        })
        .filter(|package| !allowed.contains(package))
        .collect::<Vec<_>>();

    assert!(
        legacy.is_empty(),
        "ikaros-cli should route internal dependencies through architecture facades, found direct legacy packages: {legacy:?}"
    );
}

#[test]
fn migrated_state_modules_are_owned_by_state_not_state_dependencies() {
    let metadata = workspace_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let state = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-state"))
        .expect("ikaros-state package should exist");
    let state_deps = package_dependency_names(state);

    for (legacy_package, legacy_lib, state_module) in [
        (
            "ikaros-automation",
            "crates/ikaros-automation/src/lib.rs",
            "automation",
        ),
        ("ikaros-memory", "crates/ikaros-memory/src/lib.rs", "memory"),
        ("ikaros-rag", "crates/ikaros-rag/src/lib.rs", "rag"),
        (
            "ikaros-session",
            "crates/ikaros-session/src/lib.rs",
            "session",
        ),
    ] {
        assert!(
            !state_deps.contains(legacy_package),
            "ikaros-state owns {state_module} and must not depend on the legacy {legacy_package} shim"
        );

        let legacy_lib_path = workspace_root().join(legacy_lib);
        if legacy_lib_path.exists() {
            let legacy_lib_source = fs::read_to_string(legacy_lib_path).expect("read legacy lib");
            let expected = format!("pub use ikaros_state::{state_module}::*");
            assert!(
                legacy_lib_source.contains(&expected),
                "{legacy_package} should remain only as a compatibility shim over ikaros-state::{state_module}"
            );
        }
    }
}

#[test]
fn session_projection_helpers_are_owned_by_state_not_runtime() {
    let state_projection =
        fs::read_to_string(workspace_root().join("crates/ikaros-state/src/session/projection.rs"))
            .expect("read ikaros-state session projection module");
    for exported in [
        "gateway_session_id",
        "gateway_session_source",
        "gateway_turn_id",
        "schedule_session_id",
        "schedule_session_source",
        "schedule_turn_id",
    ] {
        assert!(
            state_projection.contains(exported),
            "ikaros-state::session should own stable session projection helper {exported}"
        );
    }
}

#[test]
fn session_recording_helpers_are_owned_by_state_not_runtime() {
    let state_recording =
        fs::read_to_string(workspace_root().join("crates/ikaros-state/src/session/recording.rs"))
            .expect("read state session recording module");
    for expected in [
        "pub struct RuntimeSessionTarget",
        "pub fn upsert_runtime_session",
        "pub fn append_runtime_session_entry",
        "pub fn append_runtime_session_event",
        "pub fn delivery_payload",
    ] {
        assert!(
            state_recording.contains(expected),
            "ikaros-state should own session recording helper {expected}"
        );
    }
}

#[test]
fn gateway_queue_state_is_owned_by_state_not_gateway_surface_crate() {
    let metadata = workspace_metadata();
    let package_names = package_names(&metadata);
    assert!(
        !package_names.contains("ikaros-gateway"),
        "gateway queue/store state and adapter/webhook surfaces should live in ikaros-state and ikaros-surfaces, not the legacy ikaros-gateway crate"
    );

    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let state = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-state"))
        .expect("ikaros-state package should exist");
    let state_deps = package_dependency_names(state);

    assert!(
        !state_deps.contains("ikaros-gateway"),
        "ikaros-state owns gateway queue/store state and must not depend on the legacy ikaros-gateway surface crate"
    );
}

#[test]
fn gateway_stable_shapes_are_owned_by_protocol_not_state_store() {
    let protocol_gateway =
        fs::read_to_string(workspace_root().join("crates/ikaros-protocol/src/gateway.rs"))
            .expect("read ikaros-protocol gateway module");
    let protocol_lib =
        fs::read_to_string(workspace_root().join("crates/ikaros-protocol/src/lib.rs"))
            .expect("read ikaros-protocol lib");
    let state_gateway =
        fs::read_to_string(workspace_root().join("crates/ikaros-state/src/gateway/mod.rs"))
            .expect("read ikaros-state gateway module");

    for expected in [
        "pub enum GatewayMessageKind",
        "pub enum GatewayMessageStatus",
        "pub enum GatewayDeliveryStatus",
        "pub enum GatewayPairingStatus",
        "pub struct GatewayPairing",
        "pub struct GatewayRoute",
        "pub struct GatewayMessage",
        "pub struct GatewayDelivery",
    ] {
        assert!(
            protocol_gateway.contains(expected),
            "ikaros-protocol::gateway should own stable gateway shape {expected}"
        );
    }

    for expected in [
        "GatewayDelivery",
        "GatewayDeliveryStatus",
        "GatewayMessage",
        "GatewayMessageKind",
        "GatewayMessageStatus",
        "GatewayPairing",
        "GatewayPairingStatus",
        "GatewayRoute",
    ] {
        assert!(
            protocol_lib.contains(expected),
            "ikaros-protocol should re-export stable gateway shape {expected}"
        );
        assert!(
            state_gateway.contains(expected),
            "ikaros-state::gateway may only re-export stable gateway shape {expected} from ikaros-protocol while owning store behavior"
        );
    }

    assert!(
        !workspace_root()
            .join("crates/ikaros-state/src/gateway/types.rs")
            .exists(),
        "gateway stable shapes should not live in ikaros-state/src/gateway/types.rs"
    );

    assert_no_state_gateway_shape_imports("crates/ikaros-agent/src");
    assert_no_state_gateway_shape_imports("crates/ikaros-surfaces/src");
    assert_no_state_gateway_shape_imports("crates/ikaros-cli/src");
    assert_no_state_gateway_shape_imports("crates/ikaros-cli/tests");
}

#[test]
fn state_depends_only_on_core_protocol_and_storage_dependencies() {
    let metadata = workspace_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let state = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-state"))
        .expect("ikaros-state package should exist");
    let allowed_ikaros_deps = ["ikaros-core", "ikaros-protocol"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    let forbidden = package_dependency_names(state)
        .into_iter()
        .filter(|dependency| dependency.starts_with("ikaros-"))
        .filter(|dependency| !allowed_ikaros_deps.contains(dependency))
        .collect::<Vec<_>>();

    assert!(
        forbidden.is_empty(),
        "ikaros-state must not depend on migrated legacy or higher-level Ikaros crates: {forbidden:?}"
    );
}

#[test]
fn core_owns_soul_base_types_without_legacy_soul_crate() {
    let metadata = workspace_metadata();
    let package_names = package_names(&metadata);

    assert!(
        !package_names.contains("ikaros-soul"),
        "persona/emotion base types should live in ikaros-core, not the legacy ikaros-soul crate"
    );

    let core_lib = fs::read_to_string(workspace_root().join("crates/ikaros-core/src/lib.rs"))
        .expect("read ikaros-core lib");
    for exported in [
        "PersonaProfile",
        "PersonaLoader",
        "EmotionState",
        "RuntimeSignal",
    ] {
        assert!(
            core_lib.contains(exported),
            "ikaros-core should export {exported} after absorbing ikaros-soul"
        );
    }
}

#[test]
fn body_types_and_renderers_are_split_across_protocol_terminal_and_surfaces() {
    let metadata = workspace_metadata();
    let package_names = package_names(&metadata);
    assert!(
        !package_names.contains("ikaros-body"),
        "body wire types and renderers should be split into ikaros-protocol, ikaros-terminal, and ikaros-surfaces, not the legacy ikaros-body crate"
    );

    let protocol_body =
        fs::read_to_string(workspace_root().join("crates/ikaros-protocol/src/body.rs"))
            .expect("read ikaros-protocol body module");
    for exported in ["BodyKind", "BodyEventKind", "BodyStatus", "BodyFrame"] {
        assert!(
            protocol_body.contains(exported),
            "ikaros-protocol::body should export {exported}"
        );
    }
    assert!(
        workspace_root()
            .join("crates/ikaros-terminal/src/body.rs")
            .exists(),
        "ikaros-terminal should own CLI body rendering"
    );
    assert!(
        workspace_root()
            .join("crates/ikaros-surfaces/src/body.rs")
            .exists(),
        "ikaros-surfaces should own web dashboard body rendering"
    );
}

#[test]
fn body_status_application_helpers_are_owned_by_agent_not_runtime() {
    let agent_body_status =
        fs::read_to_string(workspace_root().join("crates/ikaros-agent/src/body_status.rs"))
            .expect("read agent body_status module");
    assert!(
        agent_body_status.contains("pub fn current_body_frame"),
        "ikaros-agent should own current body status frame assembly"
    );
}

#[test]
fn context_primitives_live_in_protocol_without_legacy_context_crate() {
    let metadata = workspace_metadata();
    let package_names = package_names(&metadata);
    assert!(
        !package_names.contains("ikaros-context"),
        "context primitives should live in ikaros-protocol::context, not the legacy ikaros-context crate"
    );

    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let agent = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-agent"))
        .expect("ikaros-agent package should exist");
    let deps = package_dependency_names(agent);
    assert!(
        !deps.contains("ikaros-context"),
        "ikaros-agent should consume context primitives from ikaros-protocol"
    );
    assert!(
        workspace_root()
            .join("crates/ikaros-protocol/src/context/mod.rs")
            .exists(),
        "ikaros-protocol should own the context module"
    );
}

#[test]
fn protocol_facade_does_not_reexport_context_or_session_root_globs() {
    let protocol_lib =
        fs::read_to_string(workspace_root().join("crates/ikaros-protocol/src/lib.rs"))
            .expect("read ikaros-protocol lib");

    for forbidden in ["pub use context::*", "pub use session::*"] {
        assert!(
            !protocol_lib.contains(forbidden),
            "ikaros-protocol should expose context and session through named modules instead of root-level glob exports: found {forbidden}"
        );
    }
}

#[test]
fn test_command_policy_is_owned_by_execution_not_coding_workflow() {
    let execution_testing =
        fs::read_to_string(workspace_root().join("crates/ikaros-execution/src/testing.rs"))
            .expect("read ikaros-execution testing module");
    for exported in [
        "TestFailureAnalysis",
        "TestFailureAnalyzer",
        "TestFailureCategory",
        "validate_test_command",
        "is_allowed_test_command",
    ] {
        assert!(
            execution_testing.contains(exported),
            "ikaros-execution::testing should own {exported}"
        );
    }
}

#[test]
fn repo_planning_primitives_are_owned_by_execution_not_coding_workflow() {
    let execution_repo =
        fs::read_to_string(workspace_root().join("crates/ikaros-execution/src/repo.rs"))
            .expect("read ikaros-execution repo module");
    for exported in [
        "RepoMap",
        "RepoFile",
        "RepoFileKind",
        "RepoScanner",
        "ChangePlan",
        "ChangePlanner",
        "TestCommand",
        "TestRunnerPlan",
    ] {
        assert!(
            execution_repo.contains(exported),
            "ikaros-execution::repo should own {exported}"
        );
    }

    for relative in [
        "crates/ikaros-skills/src/coding.rs",
        "crates/ikaros-skills/src/coding/model_loop.rs",
    ] {
        let source = fs::read_to_string(workspace_root().join(relative)).expect("read source");
        let coding_import = source.split("use ikaros_core::").next().unwrap_or(&source);
        assert!(
            source.contains("ikaros_execution::repo"),
            "{relative} should consume repo scanning and test planning primitives from ikaros_execution::repo"
        );
        assert!(
            !coding_import.contains("ChangePlanner, CodeReviewAssistant")
                && !coding_import.contains("ChangePlan, ChangePlanner")
                && !coding_import.contains("RepoMap, RepoScanner")
                && !coding_import.contains("RepoScanner, TestCommand")
                && !coding_import.contains("TestCommand, TestRunnerPlan"),
            "{relative} should not import repo scanning or test planning primitives from ikaros_coding"
        );
    }
}

#[test]
fn guarded_patch_primitives_are_owned_by_execution_not_coding_workflow() {
    let execution_patch =
        fs::read_to_string(workspace_root().join("crates/ikaros-execution/src/patch/mod.rs"))
            .expect("read ikaros-execution patch module");
    for exported in [
        "GuardedPatchApplier",
        "PatchApplyReport",
        "PatchFailure",
        "PatchFailureKind",
        "PatchFileChange",
        "PatchFileOperation",
        "parse_diff_path",
    ] {
        assert!(
            execution_patch.contains(exported),
            "ikaros-execution::patch should own {exported}"
        );
    }

    for relative in [
        "crates/ikaros-skills/src/coding.rs",
        "crates/ikaros-skills/src/coding/model_loop.rs",
    ] {
        let source = fs::read_to_string(workspace_root().join(relative)).expect("read source");
        let coding_import = source.split("use ikaros_core::").next().unwrap_or(&source);
        assert!(
            source.contains("ikaros_execution::patch"),
            "{relative} should consume guarded patch primitives from ikaros_execution::patch"
        );
        assert!(
            !coding_import.contains("GuardedPatchApplier")
                && !coding_import.contains("PatchApplyReport")
                && !coding_import.contains("PatchFailure"),
            "{relative} should not import guarded patch primitives from ikaros_coding"
        );
    }
}

#[test]
fn coding_analysis_primitives_are_owned_by_execution_not_coding_workflow() {
    for (module, exports) in [
        (
            "diff",
            &["TurnDiffSummary", "TurnDiffTracker", "UnifiedDiffRender"][..],
        ),
        (
            "review",
            &[
                "CodeReviewAssistant",
                "DiffSummarizer",
                "ReviewReport",
                "ReviewSeverity",
            ][..],
        ),
        (
            "iteration",
            &["PatchIterationPlan", "PatchIterationPlanner"][..],
        ),
    ] {
        let execution_source = fs::read_to_string(
            workspace_root().join(format!("crates/ikaros-execution/src/{module}.rs")),
        )
        .expect("read execution module");
        for exported in exports {
            assert!(
                execution_source.contains(exported),
                "ikaros-execution::{module} should own {exported}"
            );
        }
    }

    for relative in [
        "crates/ikaros-skills/src/coding.rs",
        "crates/ikaros-skills/src/coding/model_loop.rs",
    ] {
        let source = fs::read_to_string(workspace_root().join(relative)).expect("read source");
        let coding_import = source.split("use ikaros_core::").next().unwrap_or(&source);
        assert!(
            source.contains("ikaros_execution::review")
                || source.contains("ikaros_execution::iteration")
                || source.contains("ikaros_execution::diff"),
            "{relative} should consume coding analysis primitives from ikaros_execution"
        );
        assert!(
            !coding_import.contains("CodeReviewAssistant")
                && !coding_import.contains("DiffSummarizer")
                && !coding_import.contains("PatchIterationPlanner")
                && !coding_import.contains("ReviewReport")
                && !coding_import.contains("TurnDiffTracker"),
            "{relative} should not import coding analysis primitives from ikaros_coding"
        );
    }
}

#[test]
fn coding_context_primitives_are_owned_by_execution_not_coding_workflow() {
    let execution_context =
        fs::read_to_string(workspace_root().join("crates/ikaros-execution/src/coding_context.rs"))
            .expect("read ikaros-execution coding context module");
    for exported in [
        "CodingMode",
        "CodingPermissionProfile",
        "CodingModeCapabilities",
        "CodingTurnContextInput",
        "CodingTurnContext",
        "CodingGitState",
        "CodingDirtyState",
    ] {
        assert!(
            execution_context.contains(exported),
            "ikaros-execution::coding_context should own {exported}"
        );
    }

    for relative in [
        "crates/ikaros-skills/src/coding.rs",
        "crates/ikaros-skills/src/coding/model_loop.rs",
    ] {
        let source = fs::read_to_string(workspace_root().join(relative)).expect("read source");
        let coding_import = source.split("use ikaros_core::").next().unwrap_or(&source);
        assert!(
            source.contains("ikaros_execution::coding_context"),
            "{relative} should consume coding context primitives from ikaros_execution::coding_context"
        );
        assert!(
            !coding_import.contains("CodingMode")
                && !coding_import.contains("CodingTurnContext")
                && !coding_import.contains("CodingPermissionProfile"),
            "{relative} should not import coding context primitives from ikaros_coding"
        );
    }
}

#[test]
fn deterministic_coding_runtime_is_owned_by_execution_not_coding_workflow() {
    let execution_runtime =
        fs::read_to_string(workspace_root().join("crates/ikaros-execution/src/coding_runtime.rs"))
            .expect("read ikaros-execution coding runtime module");
    for exported in [
        "CodingTurnInput",
        "CodingTurnReport",
        "CodingLoopReport",
        "CodingTurnEvent",
        "CodingTurnEventKind",
        "DeterministicCodingRuntime",
        "MockModelCodingRuntime",
    ] {
        assert!(
            execution_runtime.contains(exported),
            "ikaros-execution::coding_runtime should own {exported}"
        );
    }
}

#[test]
fn coding_workflow_and_self_modify_are_owned_by_execution_not_coding_workflow() {
    let execution_workflow =
        fs::read_to_string(workspace_root().join("crates/ikaros-execution/src/coding_workflow.rs"))
            .expect("read ikaros-execution coding workflow module");
    for exported in [
        "CodingWorkflow",
        "CodingWorkflowInput",
        "CodingWorkflowReport",
        "CodingWorkflowStep",
    ] {
        assert!(
            execution_workflow.contains(exported),
            "ikaros-execution::coding_workflow should own {exported}"
        );
    }

    let execution_self_modify =
        fs::read_to_string(workspace_root().join("crates/ikaros-execution/src/self_modify.rs"))
            .expect("read ikaros-execution self-modify module");
    for exported in [
        "SelfModifyStore",
        "SelfModifyProposal",
        "SelfModifyApplyReport",
        "SelfModifyRollbackReport",
    ] {
        assert!(
            execution_self_modify.contains(exported),
            "ikaros-execution::self_modify should own {exported}"
        );
    }
}

#[test]
fn transitional_coding_crate_is_removed_after_execution_ownership_moves() {
    let metadata = workspace_metadata();
    let package_names = package_names(&metadata);
    assert!(
        !package_names.contains("ikaros-coding"),
        "coding execution primitives should live in ikaros-execution and application workflow should route through ikaros-agent, not the transitional ikaros-coding crate"
    );
    assert!(
        !workspace_root().join("crates/ikaros-coding").exists(),
        "crates/ikaros-coding should be deleted after ownership moves"
    );
}

#[test]
fn skills_no_longer_depend_on_transitional_coding_crate() {
    let metadata = workspace_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let skills = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-skills"))
        .expect("ikaros-skills package should exist");
    let deps = package_dependency_names(skills);
    assert!(
        !deps.contains("ikaros-coding"),
        "ikaros-skills should consume skill-facing coding primitives from ikaros-execution, not the transitional ikaros-coding crate"
    );
}

#[test]
fn surfaces_facade_does_not_export_gateway_state_store() {
    let surfaces_lib =
        fs::read_to_string(workspace_root().join("crates/ikaros-surfaces/src/lib.rs"))
            .expect("read ikaros-surfaces lib");
    let surfaces_gateway =
        fs::read_to_string(workspace_root().join("crates/ikaros-surfaces/src/gateway/mod.rs"))
            .expect("read ikaros-surfaces gateway module");
    let surfaces_adapter =
        fs::read_to_string(workspace_root().join("crates/ikaros-surfaces/src/gateway/adapter.rs"))
            .expect("read ikaros-surfaces gateway adapter");
    let surfaces_webhook_mod = fs::read_to_string(
        workspace_root().join("crates/ikaros-surfaces/src/gateway/webhook/mod.rs"),
    )
    .expect("read ikaros-surfaces gateway webhook module");

    assert!(
        !surfaces_lib.contains("pub use ikaros_gateway::*"),
        "ikaros-surfaces must not blanket-export ikaros-gateway; gateway queue/store ownership belongs to ikaros-state"
    );
    assert!(
        !surfaces_lib.contains("LocalGatewayStore"),
        "ikaros-surfaces must not export LocalGatewayStore; stateful gateway stores belong to ikaros-state"
    );
    assert!(
        !surfaces_gateway.contains("LocalGatewayStore"),
        "ikaros-surfaces::gateway must not re-export LocalGatewayStore"
    );
    assert!(
        !surfaces_adapter
            .contains("pub fn enqueue_gateway_adapter_message(\n    store: &LocalGatewayStore")
            && !surfaces_adapter
                .contains("pub fn render_gateway_adapter_delivery(\n    store: &LocalGatewayStore"),
        "ikaros-surfaces gateway adapter API must accept surface inputs such as gateway_dir, not expose LocalGatewayStore"
    );
    let exposes_store_backed_webhook_helpers = surfaces_webhook_mod.lines().any(|line| {
        line.contains("handle_webhook_stream")
            || (line.contains("webhook_http_response")
                && !line.contains("write_webhook_http_response"))
    });
    assert!(
        !exposes_store_backed_webhook_helpers,
        "ikaros-surfaces gateway webhook facade must not export lower-level helpers that expose LocalGatewayStore"
    );
}

#[test]
fn surfaces_owns_gateway_adapter_and_webhook_surface() {
    let metadata = workspace_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let surfaces = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-surfaces"))
        .expect("ikaros-surfaces package should exist");
    let deps = package_dependency_names(surfaces);

    assert!(
        !deps.contains("ikaros-gateway"),
        "ikaros-surfaces should own gateway adapter/webhook surface code instead of depending on the legacy ikaros-gateway crate"
    );
    assert!(
        workspace_root()
            .join("crates/ikaros-surfaces/src/gateway/mod.rs")
            .exists(),
        "ikaros-surfaces should expose gateway adapter/webhook code from its own gateway module"
    );
}

#[test]
fn surfaces_owns_service_manager_templates_without_legacy_service_crate() {
    let metadata = workspace_metadata();
    let package_names = package_names(&metadata);
    assert!(
        !package_names.contains("ikaros-service"),
        "service manager surface code should live in ikaros-surfaces, not the legacy ikaros-service crate"
    );

    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let surfaces = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-surfaces"))
        .expect("ikaros-surfaces package should exist");
    let deps = package_dependency_names(surfaces);
    assert!(
        !deps.contains("ikaros-service"),
        "ikaros-surfaces should own service manager templates directly instead of depending on ikaros-service"
    );
    assert!(
        workspace_root()
            .join("crates/ikaros-surfaces/src/service/mod.rs")
            .exists(),
        "ikaros-surfaces should expose service manager templates from its own service module"
    );
}

#[test]
fn surfaces_owns_openai_compatible_api_without_legacy_api_crate() {
    let metadata = workspace_metadata();
    let package_names = package_names(&metadata);
    assert!(
        !package_names.contains("ikaros-api"),
        "OpenAI-compatible API surface should live in ikaros-surfaces, not the legacy ikaros-api crate"
    );

    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let surfaces = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-surfaces"))
        .expect("ikaros-surfaces package should exist");
    let surfaces_deps = package_dependency_names(surfaces);
    assert!(
        !surfaces_deps.contains("ikaros-api"),
        "ikaros-surfaces should own API code directly instead of depending on ikaros-api"
    );
    assert!(
        workspace_root()
            .join("crates/ikaros-surfaces/src/api/mod.rs")
            .exists(),
        "ikaros-surfaces should own the API module"
    );
}

#[test]
fn host_owns_surfaces_api_agent_context_composition() {
    let api_dir = workspace_root().join("crates/ikaros-surfaces/src/api");
    let mut stack = vec![api_dir.clone()];
    let mut violations = Vec::new();
    let mut combined_source = String::new();

    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap_or_else(|error| {
            panic!(
                "read API source directory {}: {error}",
                dir.strip_prefix(workspace_root()).unwrap_or(&dir).display()
            )
        }) {
            let entry = entry.expect("read API source entry");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }
            let source = fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!(
                    "read API source file {}: {error}",
                    path.strip_prefix(workspace_root())
                        .unwrap_or(&path)
                        .display()
                )
            });
            for forbidden in [
                "IkarosConfig::load",
                "IkarosConfig::load_shape_checked",
                "resolve_agent_instance(",
            ] {
                if source.contains(forbidden) {
                    violations.push(format!(
                        "{} keeps API agent context composition via {forbidden}",
                        path.strip_prefix(workspace_root())
                            .unwrap_or(&path)
                            .display()
                    ));
                }
            }
            combined_source.push_str(&source);
        }
    }

    assert!(
        combined_source.contains("host_agent_context_shape_checked"),
        "ikaros-surfaces API routes should call ikaros-host for shape-checked agent context composition"
    );
    assert!(
        violations.is_empty(),
        "ikaros-surfaces API routes should delegate config loading and agent resolution to ikaros-host: {violations:?}"
    );
}

#[test]
fn surfaces_do_not_depend_on_legacy_runtime() {
    let metadata = workspace_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let surfaces = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-surfaces"))
        .expect("ikaros-surfaces package should exist");
    let deps = package_dependency_names(surfaces);

    assert!(
        !deps.contains("ikaros-runtime"),
        "ikaros-surfaces should call host/agent APIs directly instead of depending on the legacy runtime crate"
    );
}

#[test]
fn mcp_wire_shapes_live_in_protocol_and_stdio_surface_lives_in_surfaces() {
    let metadata = workspace_metadata();
    let package_names = package_names(&metadata);
    assert!(
        !package_names.contains("ikaros-mcp"),
        "MCP wire shapes should live in ikaros-protocol and stdio serving should live in ikaros-surfaces, not the legacy ikaros-mcp crate"
    );

    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let surfaces = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-surfaces"))
        .expect("ikaros-surfaces package should exist");
    let skills = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-skills"))
        .expect("ikaros-skills package should exist");
    let surfaces_deps = package_dependency_names(surfaces);
    let skills_deps = package_dependency_names(skills);

    assert!(
        !surfaces_deps.contains("ikaros-mcp"),
        "ikaros-surfaces should own MCP stdio serving directly instead of depending on ikaros-mcp"
    );
    assert!(
        !skills_deps.contains("ikaros-mcp"),
        "ikaros-skills should consume MCP probe wire helpers from ikaros-protocol instead of ikaros-mcp"
    );
    assert!(
        skills_deps.contains("ikaros-protocol"),
        "ikaros-skills should depend on ikaros-protocol for MCP wire shapes"
    );
    assert!(
        workspace_root()
            .join("crates/ikaros-protocol/src/mcp.rs")
            .exists(),
        "ikaros-protocol should own stable MCP wire shapes"
    );
    assert!(
        workspace_root()
            .join("crates/ikaros-surfaces/src/mcp.rs")
            .exists(),
        "ikaros-surfaces should own MCP stdio and HTTP surface code"
    );
    let surfaces_mcp_root =
        fs::read_to_string(workspace_root().join("crates/ikaros-surfaces/src/mcp.rs"))
            .expect("read surfaces MCP module");
    let surfaces_mcp_tree = source_file_and_tree_contents(
        "crates/ikaros-surfaces/src/mcp.rs",
        "crates/ikaros-surfaces/src/mcp",
    );
    for expected in [
        "pub async fn serve_mcp_stdio",
        "pub async fn probe_mcp_http",
        "pub async fn call_mcp_http",
        "pub async fn call_mcp_http_with_arguments_json",
    ] {
        assert!(
            surfaces_mcp_tree.contains(expected),
            "ikaros-surfaces::mcp should own MCP surface entrypoint {expected}"
        );
    }
    for expected in ["mod http;", "mod stdio;", "mod types;"] {
        assert!(
            surfaces_mcp_root.contains(expected),
            "ikaros-surfaces MCP root should declare focused surface submodule {expected}"
        );
    }
    for expected in ["pub use http::{", "pub use stdio::{", "pub use types::{"] {
        assert!(
            surfaces_mcp_root.contains(expected),
            "ikaros-surfaces MCP root should re-export public APIs from focused submodules: missing {expected}"
        );
    }
    assert!(
        surfaces_mcp_root.lines().count() <= 60,
        "ikaros-surfaces MCP root should stay a thin facade after MCP surface splitting"
    );
    let surfaces_mcp_dir = workspace_root().join("crates/ikaros-surfaces/src/mcp");
    for module in ["http", "stdio", "types", "tests"] {
        assert!(
            surfaces_mcp_dir.join(format!("{module}.rs")).exists(),
            "mcp/{module}.rs should own a focused MCP surface slice"
        );
    }
    let surfaces_mcp_types =
        fs::read_to_string(surfaces_mcp_dir.join("types.rs")).expect("read surfaces MCP types");
    for expected in [
        "pub struct McpServerInfo",
        "pub struct McpHttpProbeRequest",
        "pub struct McpHttpCallRequest",
    ] {
        assert!(
            surfaces_mcp_types.contains(expected),
            "mcp/types.rs should own public MCP request/info type {expected}"
        );
    }
}

#[test]
fn cli_does_not_use_temporary_legacy_crate_aliases() {
    let main_rs = fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/main.rs"))
        .expect("read ikaros-cli main");
    let aliases = main_rs
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("extern crate ")
                && trimmed.contains(" as ikaros_")
                && trimmed.ends_with(';')
        })
        .collect::<Vec<_>>();

    assert!(
        aliases.is_empty(),
        "temporary CLI legacy crate aliases should be removed after imports move to facade modules: {aliases:?}"
    );
}

#[test]
fn cli_gateway_adapter_and_webhook_route_through_surfaces() {
    for relative in [
        "crates/ikaros-cli/src/gateway/adapter.rs",
        "crates/ikaros-cli/src/gateway/webhook.rs",
    ] {
        let source = fs::read_to_string(workspace_root().join(relative)).expect("read source file");
        assert!(
            source.contains("ikaros_surfaces::gateway"),
            "{relative} should import gateway adapter/webhook APIs from ikaros_surfaces::gateway"
        );
        assert!(
            !source.contains("ikaros_gateway::"),
            "{relative} should not call gateway adapter/webhook APIs through the legacy gateway crate alias"
        );
    }
}

#[test]
fn surface_and_terminal_facades_do_not_own_gateway_state_store() {
    for relative in [
        "crates/ikaros-surfaces/src/lib.rs",
        "crates/ikaros-terminal/src/lib.rs",
    ] {
        let source = fs::read_to_string(workspace_root().join(relative)).expect("read source file");
        assert!(
            !source.contains("LocalGatewayStore"),
            "{relative} must not expose or own gateway state store"
        );
    }
}

#[test]
fn facade_crates_do_not_depend_on_higher_level_facades() {
    let metadata = workspace_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");

    for (package_name, forbidden) in FACADE_DEPENDENCY_RULES {
        let package = packages
            .iter()
            .find(|package| package["name"].as_str() == Some(*package_name))
            .unwrap_or_else(|| panic!("{package_name} package should exist"));
        let forbidden = forbidden.iter().copied().collect::<BTreeSet<_>>();
        let actual = package["dependencies"]
            .as_array()
            .expect("dependencies should be an array")
            .iter()
            .filter_map(|dependency| {
                dependency["package"]
                    .as_str()
                    .or_else(|| dependency["name"].as_str())
            })
            .filter(|dependency| forbidden.contains(dependency))
            .collect::<Vec<_>>();

        assert!(
            actual.is_empty(),
            "{package_name} depends on forbidden higher-level facade crates: {actual:?}"
        );
    }
}

#[test]
fn execution_consumers_route_through_execution_facade() {
    let metadata = workspace_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let forbidden = LEGACY_EXECUTION_PACKAGES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    for package_name in EXECUTION_CONSUMER_PACKAGES {
        let package = packages
            .iter()
            .find(|package| package["name"].as_str() == Some(*package_name))
            .unwrap_or_else(|| panic!("{package_name} package should exist"));
        let deps = package_dependency_names(package);
        let legacy = deps
            .iter()
            .copied()
            .filter(|dependency| forbidden.contains(dependency))
            .collect::<Vec<_>>();

        assert!(
            legacy.is_empty(),
            "{package_name} should depend on ikaros-execution instead of legacy execution crates: {legacy:?}"
        );
        assert!(
            deps.contains("ikaros-execution"),
            "{package_name} should depend on the ikaros-execution facade"
        );
    }
}

#[test]
fn execution_legacy_crates_are_removed_after_execution_ownership_moves() {
    let metadata = workspace_metadata();
    let package_names = package_names(&metadata);
    let legacy = LEGACY_EXECUTION_PACKAGES
        .iter()
        .copied()
        .filter(|package| package_names.contains(package))
        .collect::<Vec<_>>();

    assert!(
        legacy.is_empty(),
        "execution implementations should live in ikaros-execution, not legacy workspace crates: {legacy:?}"
    );

    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let execution = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-execution"))
        .expect("ikaros-execution package should exist");
    let deps = package_dependency_names(execution);
    let legacy_deps = LEGACY_EXECUTION_PACKAGES
        .iter()
        .copied()
        .filter(|package| deps.contains(package))
        .collect::<Vec<_>>();

    assert!(
        legacy_deps.is_empty(),
        "ikaros-execution must own execution implementations directly instead of depending on legacy execution crates: {legacy_deps:?}"
    );
}

#[test]
fn execution_facade_exposes_named_modules_without_root_globs() {
    let execution_lib =
        fs::read_to_string(workspace_root().join("crates/ikaros-execution/src/lib.rs"))
            .expect("read ikaros-execution lib");

    for forbidden in [
        "#![allow(ambiguous_glob_reexports, unused_imports)]",
        "pub use coding_context::*",
        "pub use coding_runtime::*",
        "pub use coding_workflow::*",
        "pub use diff::*",
        "pub use harness::*",
        "pub use iteration::*",
        "pub use patch::*",
        "pub use repo::*",
        "pub use review::*",
        "pub use sandbox::*",
        "pub use self_modify::*",
        "pub use testing::*",
        "pub use toolkit::*",
    ] {
        assert!(
            !execution_lib.contains(forbidden),
            "ikaros-execution should expose named modules instead of root-level compatibility glob exports: found {forbidden:?}"
        );
    }
}

#[test]
fn state_consumers_route_through_state_facade() {
    let metadata = workspace_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let forbidden = LEGACY_STATE_PACKAGES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    for package_name in STATE_CONSUMER_PACKAGES {
        let package = packages
            .iter()
            .find(|package| package["name"].as_str() == Some(*package_name))
            .unwrap_or_else(|| panic!("{package_name} package should exist"));
        let deps = package_dependency_names(package);
        let legacy = deps
            .iter()
            .copied()
            .filter(|dependency| forbidden.contains(dependency))
            .collect::<Vec<_>>();

        assert!(
            legacy.is_empty(),
            "{package_name} should depend on ikaros-state instead of legacy state crates: {legacy:?}"
        );
        assert!(
            deps.contains("ikaros-state"),
            "{package_name} should depend on the ikaros-state facade"
        );
    }
}

#[test]
fn state_facade_exposes_named_modules_without_root_globs() {
    let state_lib = fs::read_to_string(workspace_root().join("crates/ikaros-state/src/lib.rs"))
        .expect("read ikaros-state lib");
    for expected in [
        "pub mod automation",
        "pub mod gateway",
        "pub mod memory",
        "pub mod rag",
        "pub mod session",
    ] {
        assert!(
            state_lib.contains(expected),
            "ikaros-state should expose named module {expected}"
        );
    }
    for forbidden in [
        "pub use automation::*",
        "pub use gateway::*",
        "pub use memory::*",
        "pub use rag::*",
        "pub use session::*",
        "allow(ambiguous_glob_reexports",
        "allow(unused_imports",
        "re-exports the legacy crates",
        "architecture transition",
    ] {
        assert!(
            !state_lib.contains(forbidden),
            "ikaros-state should not keep legacy root facade item {forbidden}"
        );
    }
}

#[test]
fn provider_consumers_route_through_provider_facade() {
    let metadata = workspace_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let forbidden = LEGACY_PROVIDER_PACKAGES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    for package_name in PROVIDER_CONSUMER_PACKAGES {
        let package = packages
            .iter()
            .find(|package| package["name"].as_str() == Some(*package_name))
            .unwrap_or_else(|| panic!("{package_name} package should exist"));
        let deps = package_dependency_names(package);
        let legacy = deps
            .iter()
            .copied()
            .filter(|dependency| forbidden.contains(dependency))
            .collect::<Vec<_>>();

        assert!(
            legacy.is_empty(),
            "{package_name} should depend on ikaros-providers instead of legacy provider crates: {legacy:?}"
        );
        assert!(
            deps.contains("ikaros-providers"),
            "{package_name} should depend on the ikaros-providers facade"
        );
    }
}

#[test]
fn provider_legacy_crates_are_removed_after_provider_ownership_moves() {
    let metadata = workspace_metadata();
    let package_names = package_names(&metadata);
    let legacy = LEGACY_PROVIDER_PACKAGES
        .iter()
        .copied()
        .filter(|package| package_names.contains(package))
        .collect::<Vec<_>>();

    assert!(
        legacy.is_empty(),
        "model and voice provider implementations should live in ikaros-providers, not legacy workspace crates: {legacy:?}"
    );

    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let providers = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-providers"))
        .expect("ikaros-providers package should exist");
    let deps = package_dependency_names(providers);
    let legacy_deps = LEGACY_PROVIDER_PACKAGES
        .iter()
        .copied()
        .filter(|package| deps.contains(package))
        .collect::<Vec<_>>();

    assert!(
        legacy_deps.is_empty(),
        "ikaros-providers must own provider implementations directly instead of depending on legacy provider crates: {legacy_deps:?}"
    );
}

#[test]
fn providers_facade_exposes_named_modules_without_legacy_models_alias() {
    let providers_lib =
        fs::read_to_string(workspace_root().join("crates/ikaros-providers/src/lib.rs"))
            .expect("read ikaros-providers lib");
    assert!(
        providers_lib.contains("pub mod model") && providers_lib.contains("pub mod voice"),
        "ikaros-providers should expose named provider modules"
    );
    for forbidden in [
        "pub mod models",
        "pub use model::*",
        "pub use voice::*",
        "allow(ambiguous_glob_reexports",
        "allow(unused_imports",
    ] {
        assert!(
            !providers_lib.contains(forbidden),
            "ikaros-providers should not keep legacy compatibility facade item {forbidden}"
        );
    }
    assert_source_tree_excludes(
        "crates",
        &[concat!("ikaros_providers::", "models")],
        "callers should import provider APIs from ikaros_providers::model instead of the legacy models alias",
    );
}

#[test]
fn agent_facade_does_not_assemble_host_resources() {
    let forbidden = [
        "IkarosConfig::load",
        "IkarosPaths::",
        "runtime_execution_env",
    ];
    assert_source_tree_excludes(
        "crates/ikaros-agent/src",
        &forbidden,
        "ikaros-agent should receive assembled dependencies instead of loading config, paths, or host resources",
    );
}

#[test]
fn agent_does_not_reexport_or_depend_on_host_composition_root() {
    let metadata = workspace_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let agent = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-agent"))
        .expect("ikaros-agent package should exist");
    let deps = package_dependency_names(agent);
    assert!(
        !deps.contains("ikaros-host"),
        "ikaros-agent should receive assembled dependencies and must not depend on the host composition root"
    );
    assert!(
        !deps.contains("ikaros-runtime"),
        "ikaros-agent should own application use cases directly and must not depend on the legacy runtime crate"
    );

    let agent_lib = fs::read_to_string(workspace_root().join("crates/ikaros-agent/src/lib.rs"))
        .expect("read ikaros-agent lib");
    for forbidden in ["pub use ikaros_host", "pub mod host"] {
        assert!(
            !agent_lib.contains(forbidden),
            "ikaros-agent must not re-export host composition APIs through the agent facade: found {forbidden}"
        );
    }
}

#[test]
fn agent_exposes_named_application_modules_instead_of_runtime_bucket() {
    let agent_lib = fs::read_to_string(workspace_root().join("crates/ikaros-agent/src/lib.rs"))
        .expect("read ikaros-agent lib");
    for forbidden in ["pub mod runtime", "pub use ikaros_runtime::*"] {
        assert!(
            !agent_lib.contains(forbidden),
            "ikaros-agent must not expose a blanket runtime facade; use named application modules instead: found {forbidden}"
        );
    }
    for expected in [
        "pub mod chat",
        "pub mod task_loop",
        "pub mod schedule",
        "pub mod gateway_drain",
    ] {
        assert!(
            agent_lib.contains(expected),
            "ikaros-agent should expose named application module {expected}"
        );
    }

    assert_source_tree_excludes(
        "crates/ikaros-cli/src",
        &["ikaros_agent::runtime"],
        "ikaros-cli should call named ikaros-agent application modules instead of the transitional runtime bucket",
    );
}

#[test]
fn agent_does_not_keep_low_level_coding_primitive_reexport_shim() {
    let agent_src = workspace_root().join("crates/ikaros-agent/src");
    let agent_lib = fs::read_to_string(agent_src.join("lib.rs")).expect("read ikaros-agent lib");
    let cli_self_modify =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/self_modify.rs"))
            .expect("read cli self-modify command");

    assert!(
        !agent_lib.contains("pub mod coding"),
        "ikaros-agent should not expose a pure low-level coding primitive re-export shim"
    );
    assert!(
        !agent_src.join("coding.rs").exists(),
        "ikaros-agent coding.rs should be removed until it owns real application workflow APIs"
    );
    assert!(
        !cli_self_modify.contains("ikaros_agent::coding"),
        "CLI self-modify should import execution-owned self-modify primitives directly"
    );
}

#[test]
fn agent_facade_uses_named_module_files_instead_of_inline_buckets() {
    let agent_src = workspace_root().join("crates/ikaros-agent/src");
    let agent_lib = fs::read_to_string(agent_src.join("lib.rs")).expect("read ikaros-agent lib");

    for module in [
        "agent_loop",
        "agent_pool",
        "body_status",
        "chat",
        "gateway_drain",
        "relationship",
        "schedule",
        "session_runner",
        "soul",
        "task_loop",
    ] {
        let declaration = format!("pub mod {module};");
        let inline_bucket = format!("pub mod {module} {{");
        assert!(
            agent_lib.contains(&declaration),
            "ikaros-agent should expose {module} through a named module file"
        );
        assert!(
            !agent_lib.contains(&inline_bucket),
            "ikaros-agent should not keep inline facade bucket {module} in lib.rs"
        );
        let module_file = agent_src.join(format!("{module}.rs"));
        let module_dir = agent_src.join(module).join("mod.rs");
        assert!(
            module_file.exists() || module_dir.exists(),
            "ikaros-agent module {module} should live in its own source file or module directory"
        );
    }
}

#[test]
fn agent_relationship_module_is_owned_by_agent_not_runtime_reexport() {
    let relationship =
        fs::read_to_string(workspace_root().join("crates/ikaros-agent/src/relationship.rs"))
            .expect("read ikaros-agent relationship module");
    assert!(
        !relationship.contains("ikaros_runtime"),
        "ikaros-agent::relationship should own relationship application helpers instead of re-exporting ikaros-runtime"
    );
    for expected in [
        "pub async fn relationship_snapshot_from_session",
        "pub fn relationship_context_lines",
        "pub type RelationshipNote",
        "pub type RelationshipReport",
    ] {
        assert!(
            relationship.contains(expected),
            "ikaros-agent::relationship should expose {expected}"
        );
    }
}

#[test]
fn agent_body_status_module_is_owned_by_agent_not_runtime_reexport() {
    let body_status =
        fs::read_to_string(workspace_root().join("crates/ikaros-agent/src/body_status.rs"))
            .expect("read ikaros-agent body_status module");
    assert!(
        !body_status.contains("ikaros_runtime"),
        "ikaros-agent::body_status should own body status helpers instead of re-exporting ikaros-runtime"
    );
    for expected in [
        "pub fn current_body_frame",
        "pub fn base_body_status",
        "pub fn audit_event_to_body_event",
        "pub fn audit_event_to_body_event_for_body",
        "pub fn body_event_kind_from_audit",
    ] {
        assert!(
            body_status.contains(expected),
            "ikaros-agent::body_status should expose {expected}"
        );
    }
}

#[test]
fn agent_chat_module_is_owned_by_agent_not_runtime_reexport() {
    let chat_dir = workspace_root().join("crates/ikaros-agent/src/chat");
    assert!(
        chat_dir.join("mod.rs").exists(),
        "ikaros-agent::chat should be a real owned module directory"
    );
    assert_source_tree_excludes(
        "crates/ikaros-agent/src/chat",
        &[
            "pub use ikaros_runtime",
            "ikaros_runtime",
            "ikaros_host",
            "IkarosConfig::load",
            "session_and_registry",
            "EgressModelHttpClient",
            "runtime_execution_env",
        ],
        "ikaros-agent::chat should own chat use-case modules and must not assemble host/config resources",
    );
    let chat_mod = fs::read_to_string(chat_dir.join("mod.rs")).expect("read chat mod");
    for expected in [
        "pub use context_engine::{",
        "pub use history::{",
        "pub use turn::{",
        "ChatRunOptions",
        "run_chat_turn_with_events",
    ] {
        assert!(
            chat_mod.contains(expected),
            "ikaros-agent::chat should expose owned chat item {expected}"
        );
    }
}

#[test]
fn acp_chat_prompts_do_not_use_interactive_cli_session_source() {
    let acp = fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/acp.rs"))
        .expect("read acp command");
    assert!(
        acp.contains("SessionSource::Service { name: \"acp\""),
        "ACP prompts should record a non-CLI session source so interactive chat does not auto-resume ACP timelines"
    );

    let chat_mod =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/chat/single.rs"))
            .expect("read single chat command");
    assert!(
        chat_mod.contains("options.session_source.clone().unwrap_or(SessionSource::Cli)"),
        "single-message chat helper should respect an explicit session source instead of always forcing CLI"
    );
}

#[test]
fn host_owns_acp_runtime_composition() {
    let relative = "crates/ikaros-cli/src/acp.rs";
    let source = fs::read_to_string(workspace_root().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"));
    for expected in ["host_agent_context", "runtime_harness"] {
        assert!(
            source.contains(expected),
            "{relative} should use ikaros-host ACP composition helper {expected}"
        );
    }
    for forbidden in [
        "IkarosConfig::load",
        "IkarosConfig::load_shape_checked",
        "resolve_agent_instance(",
        "session_and_registry_for_instance",
    ] {
        assert!(
            !source.contains(forbidden),
            "{relative} should delegate ACP runtime composition to ikaros-host instead of keeping {forbidden}"
        );
    }
}

#[test]
fn agent_loop_module_is_owned_by_agent_not_runtime_reexport() {
    let agent_loop_dir = workspace_root().join("crates/ikaros-agent/src/agent_loop");
    assert!(
        agent_loop_dir.join("mod.rs").exists(),
        "ikaros-agent::agent_loop should be a real owned module directory"
    );
    assert_source_tree_excludes(
        "crates/ikaros-agent/src/agent_loop",
        &["ikaros_runtime", "ikaros_host"],
        "ikaros-agent::agent_loop should own agent loop production code without depending on runtime or host",
    );
    let agent_loop =
        fs::read_to_string(agent_loop_dir.join("mod.rs")).expect("read agent_loop mod");
    for expected in [
        "pub use runtime::",
        "run_agent_loop",
        "run_agent_loop_with_events",
        "AgentRuntime",
        "HarnessAgentRuntime",
    ] {
        assert!(
            agent_loop.contains(expected),
            "ikaros-agent::agent_loop should expose {expected}"
        );
    }
}

#[test]
fn agent_pool_public_types_are_owned_by_agent_not_runtime_reexport() {
    let agent_pool =
        fs::read_to_string(workspace_root().join("crates/ikaros-agent/src/agent_pool.rs"))
            .expect("read ikaros-agent agent_pool module");
    assert!(
        !agent_pool.contains("pub use ikaros_runtime"),
        "ikaros-agent::agent_pool should own public pool report/task types instead of re-exporting runtime types"
    );
    for forbidden in ["ikaros_runtime", "runtime_harness", "IkarosConfig::load"] {
        assert!(
            !agent_pool.contains(forbidden),
            "ikaros-agent::agent_pool should receive assembled handoff dependencies instead of calling runtime or host composition: found {forbidden}"
        );
    }
    for expected in [
        "pub struct AgentHandoffReport",
        "pub struct AgentPoolTask",
        "pub struct AgentPoolItemReport",
        "pub struct AgentPoolReport",
        "pub struct AgentHandoffContext",
        "pub async fn run_agent_handoff_with_options",
    ] {
        assert!(
            agent_pool.contains(expected),
            "ikaros-agent::agent_pool should expose owned {expected}"
        );
    }
}

#[test]
fn agent_schedule_public_types_are_owned_by_agent_not_runtime_reexport() {
    let schedule = fs::read_to_string(workspace_root().join("crates/ikaros-agent/src/schedule.rs"))
        .expect("read ikaros-agent schedule module");
    assert!(
        !schedule.contains("pub use ikaros_runtime"),
        "ikaros-agent::schedule should own public report types instead of re-exporting runtime types"
    );
    for forbidden in ["ikaros_runtime", "runtime_harness", "IkarosConfig::load"] {
        assert!(
            !schedule.contains(forbidden),
            "ikaros-agent::schedule should receive assembled schedule dependencies instead of calling runtime or host composition: found {forbidden}"
        );
    }
    for expected in [
        "pub struct ScheduleDeliveryReport",
        "pub struct ScheduledJobRunReport",
        "pub struct ScheduleWorkerTickReport",
        "pub struct ScheduledJobExecutionContext",
        "pub async fn run_scheduled_job_with_context",
        "pub fn record_scheduled_job_preflight_failure",
    ] {
        assert!(
            schedule.contains(expected),
            "ikaros-agent::schedule should expose owned {expected}"
        );
    }
}

#[test]
fn agent_gateway_drain_public_types_are_owned_by_agent_not_runtime_reexport() {
    let gateway_drain_root_path = "crates/ikaros-agent/src/gateway_drain.rs";
    let gateway_drain_dir_path = "crates/ikaros-agent/src/gateway_drain";
    let gateway_drain_root = fs::read_to_string(workspace_root().join(gateway_drain_root_path))
        .expect("read ikaros-agent gateway_drain facade");
    let gateway_drain =
        source_file_and_tree_contents(gateway_drain_root_path, gateway_drain_dir_path);

    assert!(
        !gateway_drain.contains("pub use ikaros_runtime"),
        "ikaros-agent::gateway_drain should own public report types instead of re-exporting runtime types"
    );
    for forbidden in ["ikaros_runtime", "runtime_harness", "IkarosConfig::load"] {
        assert!(
            !gateway_drain.contains(forbidden),
            "ikaros-agent::gateway_drain should receive assembled gateway dependencies instead of calling runtime or host composition: found {forbidden}"
        );
    }

    for module in ["types", "execution", "status", "recording", "delivery"] {
        assert!(
            gateway_drain_root.contains(&format!("mod {module};")),
            "ikaros-agent::gateway_drain facade should declare focused submodule {module}"
        );
        assert!(
            workspace_root()
                .join(gateway_drain_dir_path)
                .join(format!("{module}.rs"))
                .exists(),
            "ikaros-agent::gateway_drain/{module}.rs should own a focused gateway drain slice"
        );
    }
    assert!(
        gateway_drain_root.lines().count() <= 80,
        "gateway_drain.rs should stay as a thin facade after splitting"
    );

    for expected in [
        "pub struct GatewayDrainContext",
        "pub struct GatewayDrainReport",
        "pub struct GatewayWorkerTickReport",
        "pub struct PlatformDeliveryReport",
        "pub enum GatewayMessageDrainContext",
        "pub async fn drain_gateway_message_with_context",
        "pub fn record_gateway_message_preflight_failure",
    ] {
        assert!(
            gateway_drain.contains(expected),
            "ikaros-agent::gateway_drain should expose owned {expected}"
        );
    }

    let types = fs::read_to_string(
        workspace_root()
            .join(gateway_drain_dir_path)
            .join("types.rs"),
    )
    .expect("read gateway_drain types module");
    for expected in [
        "pub struct GatewayDrainContext",
        "pub struct GatewayDrainReport",
        "pub struct GatewayWorkerTickReport",
        "pub enum GatewayMessageDrainContext",
    ] {
        assert!(
            types.contains(expected),
            "gateway_drain/types.rs should own public gateway drain type {expected}"
        );
    }

    let execution = fs::read_to_string(
        workspace_root()
            .join(gateway_drain_dir_path)
            .join("execution.rs"),
    )
    .expect("read gateway_drain execution module");
    for expected in [
        "pub async fn drain_gateway_message_with_context",
        "pub fn record_gateway_message_preflight_failure",
        "pub async fn run_gateway_worker_tick_with_contexts",
        "fn drain_chat_message_with_context",
        "fn drain_task_message_with_context",
    ] {
        assert!(
            execution.contains(expected),
            "gateway_drain/execution.rs should own gateway drain execution helper {expected}"
        );
    }

    let status = fs::read_to_string(
        workspace_root()
            .join(gateway_drain_dir_path)
            .join("status.rs"),
    )
    .expect("read gateway_drain status module");
    for expected in [
        "fn gateway_status_str",
        "fn record_gateway_status_for_drain",
        "fn record_gateway_failure_for_drain",
        "fn current_gateway_status",
        "fn record_gateway_error",
    ] {
        assert!(
            status.contains(expected),
            "gateway_drain/status.rs should own gateway status/store helper {expected}"
        );
    }

    let recording = fs::read_to_string(
        workspace_root()
            .join(gateway_drain_dir_path)
            .join("recording.rs"),
    )
    .expect("read gateway_drain recording module");
    for expected in [
        "fn record_gateway_delivery_session",
        "fn record_gateway_task_session",
        "fn gateway_input_payload",
    ] {
        assert!(
            recording.contains(expected),
            "gateway_drain/recording.rs should own gateway session recording helper {expected}"
        );
    }

    let delivery = fs::read_to_string(
        workspace_root()
            .join(gateway_drain_dir_path)
            .join("delivery.rs"),
    )
    .expect("read gateway_drain delivery module");
    for expected in [
        "pub struct PlatformDeliveryReport",
        "pub async fn deliver_to_platform",
        "pub fn platform_webhook_config_key",
        "fn build_platform_payload",
    ] {
        assert!(
            delivery.contains(expected),
            "gateway_drain/delivery.rs should own platform delivery helper {expected}"
        );
    }
}

#[test]
fn agent_session_runner_module_is_owned_by_agent_not_runtime_reexport() {
    let session_runner_dir = workspace_root().join("crates/ikaros-agent/src/session_runner");
    assert!(
        session_runner_dir.join("mod.rs").exists(),
        "ikaros-agent::session_runner should be a real owned module directory"
    );
    assert_source_tree_excludes(
        "crates/ikaros-agent/src/session_runner",
        &["ikaros_runtime"],
        "ikaros-agent::session_runner should own the harness implementation instead of re-exporting ikaros-runtime",
    );
    let session_runner =
        fs::read_to_string(session_runner_dir.join("mod.rs")).expect("read session_runner mod");
    for expected in [
        "pub struct AgentHarness",
        "pub use phase::AgentHarnessPhase",
        "AgentHarnessConfig",
        "AgentHarnessContinuation",
    ] {
        assert!(
            session_runner.contains(expected),
            "ikaros-agent::session_runner should expose {expected}"
        );
    }
}

#[test]
fn agent_owns_chat_attachment_content_block_helpers() {
    let agent_attachments =
        fs::read_to_string(workspace_root().join("crates/ikaros-agent/src/chat/attachments.rs"))
            .expect("read agent chat attachments module");
    for expected in [
        "pub fn content_blocks_from_args",
        "pub fn content_blocks_from_args_resolving_paths",
        "pub fn content_block_from_parts_resolving_path",
        "pub fn content_block_kind",
        "pub fn content_block_summary",
    ] {
        assert!(
            agent_attachments.contains(expected),
            "ikaros-agent should own chat attachment helper {expected}"
        );
    }

    let cli_attachments =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/chat/attachments.rs"))
            .expect("read CLI chat attachments module");
    assert!(
        cli_attachments.contains("pub(in crate::chat) use ikaros_agent::chat::attachments::"),
        "CLI attachments module should be a narrow re-export over ikaros-agent chat attachments"
    );
    for forbidden in [
        "fn local_attachment_data_url",
        "fn local_attachment_path",
        "fn guess_attachment_mime_type",
        "fn attachment_source_preview",
    ] {
        assert!(
            !cli_attachments.contains(forbidden),
            "CLI should not own attachment implementation helper {forbidden}"
        );
    }
}

#[test]
fn agent_owns_workbench_diff_execution_collection() {
    let agent_workbench_diff =
        fs::read_to_string(workspace_root().join("crates/ikaros-agent/src/chat/workbench_diff.rs"))
            .expect("read agent chat workbench diff module");
    for expected in [
        "pub async fn collect_workbench_diff_status",
        "ProcessRequest::program",
        "\"git\"",
        "\"diff\"",
    ] {
        assert!(
            agent_workbench_diff.contains(expected),
            "ikaros-agent should own workbench diff execution detail {expected}"
        );
    }

    let cli_diff = fs::read_to_string(
        workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/diff.rs"),
    )
    .expect("read CLI workbench diff status module");
    assert!(
        cli_diff.contains("collect_workbench_diff_status"),
        "CLI diff status should call the ikaros-agent workbench diff API"
    );
    for forbidden in [
        "ProcessRequest",
        "run_process(",
        "with_timeout_ms",
        "with_max_output_bytes",
        "struct DiffStatus",
        "struct DiffPreview",
        "DIFF_PREVIEW_MAX_BYTES",
        "fn diff_preview_text",
        "redact_secrets",
    ] {
        assert!(
            !cli_diff.contains(forbidden),
            "CLI should delegate workbench diff execution collection to ikaros-agent instead of keeping {forbidden}"
        );
    }

    assert_source_tree_excludes(
        "crates/ikaros-cli/src/chat/workbench/status",
        &["ProcessRequest", "run_process("],
        "CLI workbench status modules should not own execution process collection",
    );
    assert_source_tree_excludes(
        "crates/ikaros-terminal/src",
        &["ProcessRequest", "run_process("],
        "ikaros-terminal must stay free of execution process collection",
    );
}

#[test]
fn agent_owns_workbench_continuation_cancellation() {
    let agent_workbench_state = fs::read_to_string(
        workspace_root().join("crates/ikaros-agent/src/chat/workbench_state.rs"),
    )
    .expect("read agent chat workbench state module");
    for expected in [
        "pub enum WorkbenchCancelTarget",
        "pub struct WorkbenchCancelReport",
        "pub fn cancel_session_continuations",
    ] {
        assert!(
            agent_workbench_state.contains(expected),
            "ikaros-agent should own workbench continuation cancellation API {expected}"
        );
    }

    let cli_continuations = fs::read_to_string(
        workspace_root().join("crates/ikaros-cli/src/chat/interactive/continuations.rs"),
    )
    .expect("read CLI interactive continuations module");
    for forbidden in [
        "fn cancel_session_continuations",
        "fn record_workbench_continuation_cancelled_event",
    ] {
        assert!(
            !cli_continuations.contains(forbidden),
            "CLI should delegate continuation cancellation mutation to ikaros-agent instead of defining {forbidden}"
        );
    }
    assert_source_tree_excludes(
        "crates/ikaros-cli/src/chat/interactive/continuations",
        &[
            "fn cancel_session_continuations",
            "fn record_workbench_continuation_cancelled_event",
        ],
        "CLI continuation submodules should delegate cancellation mutation to ikaros-agent",
    );
}

#[test]
fn agent_owns_workbench_evidence_entry_mutation() {
    let agent_workbench_state = fs::read_to_string(
        workspace_root().join("crates/ikaros-agent/src/chat/workbench_state.rs"),
    )
    .expect("read agent chat workbench state module");
    for expected in [
        "pub fn append_workbench_evidence_entry",
        "\"operation\": \"workbench_evidence\"",
        "SessionEntryKind::Custom",
    ] {
        assert!(
            agent_workbench_state.contains(expected),
            "ikaros-agent should own workbench evidence entry mutation detail {expected}"
        );
    }

    let cli_evidence = fs::read_to_string(
        workspace_root().join("crates/ikaros-cli/src/chat/interactive/evidence.rs"),
    )
    .expect("read CLI interactive evidence module");
    assert!(
        cli_evidence.contains("append_workbench_evidence_entry"),
        "CLI evidence glue should call the ikaros-agent workbench evidence API"
    );
    for forbidden in [
        "SessionEntry::new",
        "SessionEntryKind::Custom",
        "\"operation\": \"workbench_evidence\"",
    ] {
        assert!(
            !cli_evidence.contains(forbidden),
            "CLI should delegate workbench evidence entry mutation to ikaros-agent instead of keeping {forbidden}"
        );
    }
}

#[test]
fn agent_owns_workbench_session_fork_mutation() {
    let agent_workbench_state = fs::read_to_string(
        workspace_root().join("crates/ikaros-agent/src/chat/workbench_state.rs"),
    )
    .expect("read agent chat workbench state module");
    for expected in [
        "pub enum WorkbenchForkReport",
        "pub fn fork_workbench_session",
        "SessionBranchSummaryInput",
        "\"command\": \"/fork\"",
    ] {
        assert!(
            agent_workbench_state.contains(expected),
            "ikaros-agent should own workbench session fork mutation detail {expected}"
        );
    }

    let cli_session = fs::read_to_string(
        workspace_root().join("crates/ikaros-cli/src/chat/interactive/session.rs"),
    )
    .expect("read CLI interactive session module");
    assert!(
        cli_session.contains("fork_workbench_session"),
        "CLI session command glue should call the ikaros-agent workbench fork API"
    );
    for forbidden in [
        "SessionBranchSummaryInput",
        "branch_from_entry",
        "\"command\": \"/fork\"",
    ] {
        assert!(
            !cli_session.contains(forbidden),
            "CLI should delegate workbench session fork mutation to ikaros-agent instead of keeping {forbidden}"
        );
    }
}

#[test]
fn agent_owns_workbench_continuation_requeue_mutation() {
    let agent_workbench_state = fs::read_to_string(
        workspace_root().join("crates/ikaros-agent/src/chat/workbench_state.rs"),
    )
    .expect("read agent chat workbench state module");
    for expected in [
        "pub fn requeue_workbench_continuation",
        "\"requeued_by\": \"workbench\"",
        "\"requeue_source\": \"/queue retry\"",
    ] {
        assert!(
            agent_workbench_state.contains(expected),
            "ikaros-agent should own workbench continuation requeue detail {expected}"
        );
    }

    let cli_continuations = fs::read_to_string(
        workspace_root().join("crates/ikaros-cli/src/chat/interactive/continuations.rs"),
    )
    .expect("read CLI interactive continuations module");
    assert!(
        cli_continuations.contains("requeue_workbench_continuation"),
        "CLI continuation command glue should call the ikaros-agent workbench requeue API"
    );
    for forbidden in [
        "store.requeue_continuation",
        "\"requeued_by\": \"workbench\"",
        "\"requeue_source\": \"/queue retry\"",
    ] {
        assert!(
            !cli_continuations.contains(forbidden),
            "CLI should delegate workbench continuation requeue mutation to ikaros-agent instead of keeping {forbidden}"
        );
    }
    assert_source_tree_excludes(
        "crates/ikaros-cli/src/chat/interactive/continuations",
        &[
            "store.requeue_continuation",
            "\"requeued_by\": \"workbench\"",
            "\"requeue_source\": \"/queue retry\"",
        ],
        "CLI continuation submodules should delegate workbench continuation requeue mutation to ikaros-agent",
    );
}

#[test]
fn cli_interactive_continuations_keep_cancel_output_queue_and_status_split() {
    let continuations_rs = fs::read_to_string(
        workspace_root().join("crates/ikaros-cli/src/chat/interactive/continuations.rs"),
    )
    .expect("read CLI interactive continuations facade");
    let continuations_dir =
        workspace_root().join("crates/ikaros-cli/src/chat/interactive/continuations");

    for module in ["cancel", "output", "queue", "status"] {
        assert!(
            continuations_dir.join(format!("{module}.rs")).exists(),
            "interactive/continuations/{module}.rs should own a focused continuation slice"
        );
        assert!(
            continuations_rs.contains(&format!("mod {module};")),
            "interactive/continuations.rs should declare the focused {module} submodule"
        );
    }
    assert!(
        continuations_rs.lines().count() <= 80,
        "interactive/continuations.rs should stay as a thin facade after continuation splitting"
    );

    let cancel =
        fs::read_to_string(continuations_dir.join("cancel.rs")).expect("read continuation cancel");
    let output =
        fs::read_to_string(continuations_dir.join("output.rs")).expect("read continuation output");
    let queue =
        fs::read_to_string(continuations_dir.join("queue.rs")).expect("read continuation queue");
    let status =
        fs::read_to_string(continuations_dir.join("status.rs")).expect("read continuation status");
    assert!(
        cancel.contains("handle_cancel_command")
            && cancel.contains("cancel_selected_screen_continuation"),
        "continuations/cancel.rs should own cancel command glue and selected-row cancellation"
    );
    assert!(
        output.contains("continuations_json_line")
            && output.contains("pending_inputs_json_line")
            && output.contains("continuation_status_count"),
        "continuations/output.rs should own continuation and pending-input rendering"
    );
    assert!(
        queue.contains("handle_queue_command")
            && queue.contains("clear_selected_screen_input")
            && queue.contains("requeue_workbench_continuation"),
        "continuations/queue.rs should own pending-input queue commands over agent requeue API"
    );
    assert!(
        status.contains("print_workbench_continuation_status")
            && status.contains("continuations_json_line"),
        "continuations/status.rs should own debug continuation status output"
    );
    for forbidden in [
        "fn handle_cancel_command",
        "fn continuations_json_line",
        "fn handle_queue_command",
        "fn print_workbench_continuation_status",
    ] {
        assert!(
            !continuations_rs.contains(forbidden),
            "interactive/continuations.rs should stay a facade; move {forbidden} into continuations/* modules"
        );
    }
}

#[test]
fn agent_owns_chat_history_delete_tombstone_mutation() {
    let agent_history_session = fs::read_to_string(
        workspace_root().join("crates/ikaros-agent/src/chat/history/session.rs"),
    )
    .expect("read agent chat history session module");
    for expected in [
        "pub fn append_chat_history_delete_tombstone",
        "CHAT_HISTORY_DELETE_SESSION_OPERATION",
        "chat history hidden from history projection",
    ] {
        assert!(
            agent_history_session.contains(expected),
            "ikaros-agent should own chat history delete tombstone detail {expected}"
        );
    }

    let cli_history =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/chat/history.rs"))
            .expect("read CLI chat history module");
    assert!(
        cli_history.contains("append_chat_history_delete_tombstone"),
        "CLI history glue should call the ikaros-agent chat history tombstone API"
    );
    for forbidden in [
        "SessionEntry::new",
        "SessionEntryKind::Custom",
        "CHAT_HISTORY_DELETE_SESSION_OPERATION",
        "chat history hidden from history projection",
    ] {
        assert!(
            !cli_history.contains(forbidden),
            "CLI should delegate chat history delete tombstone mutation to ikaros-agent instead of keeping {forbidden}"
        );
    }
}

#[test]
fn agent_task_loop_module_is_owned_by_agent_not_runtime_reexport() {
    let task_loop_dir = workspace_root().join("crates/ikaros-agent/src/task_loop");
    assert!(
        task_loop_dir.join("mod.rs").exists(),
        "ikaros-agent::task_loop should be a real owned module directory"
    );
    assert_source_tree_excludes(
        "crates/ikaros-agent/src/task_loop",
        &[
            "ikaros_runtime",
            "ikaros_host",
            "IkarosConfig::load",
            "runtime_harness",
            "session_and_registry",
            "EgressModelHttpClient",
        ],
        "ikaros-agent::task_loop should own task use cases and receive host-assembled dependencies",
    );
    let task_loop = fs::read_to_string(task_loop_dir.join("mod.rs")).expect("read task_loop mod");
    for expected in [
        "pub use execution::{",
        "TaskAgentLoopContext",
        "TaskExecutionContext",
        "execute_task_text_with_context",
        "TaskRunOptions",
    ] {
        assert!(
            task_loop.contains(expected),
            "ikaros-agent::task_loop should expose {expected}"
        );
    }
}

#[test]
fn agent_facade_does_not_keep_root_glob_reexports() {
    let agent_lib = fs::read_to_string(workspace_root().join("crates/ikaros-agent/src/lib.rs"))
        .expect("read ikaros-agent lib");
    for forbidden_module in ["automation", "body", "context"] {
        assert!(
            !agent_lib.contains(&format!("pub mod {forbidden_module};")),
            "ikaros-agent should not expose pure {forbidden_module} re-export modules"
        );
        assert!(
            !workspace_root()
                .join(format!("crates/ikaros-agent/src/{forbidden_module}.rs"))
                .exists(),
            "ikaros-agent pure {forbidden_module} re-export file should be removed"
        );
    }
    for forbidden in [
        "\npub use ikaros_core::{",
        "\npub use ikaros_execution::{",
        "\npub use ikaros_protocol::body::*;",
        "\npub use ikaros_protocol::context::*;",
        "\npub use ikaros_state::automation::*;",
    ] {
        assert!(
            !agent_lib.contains(forbidden),
            "ikaros-agent should expose owned application namespaces instead of root-level compatibility glob exports: found {forbidden:?}"
        );
    }
}

#[test]
fn cli_task_command_uses_host_composed_task_context() {
    let task_source = fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/task.rs"))
        .expect("read cli task command");
    assert!(
        task_source.contains("runtime_harness"),
        "CLI task command should assemble host resources at the CLI/host boundary"
    );
    assert!(
        task_source.contains("TaskExecutionContext"),
        "CLI task command should pass assembled dependencies into agent task execution"
    );
    assert!(
        task_source.contains("execute_task_text_with_context"),
        "CLI task command should call the context-based task use case"
    );
    assert!(
        !task_source.contains("execute_task_text_with_options"),
        "CLI task command should not call the legacy task wrapper that hides host assembly inside runtime"
    );
    for forbidden in [
        "EgressModelHttpClient",
        "governed_provider_from_config_with_http_client",
        "model_request_options_from_config",
    ] {
        assert!(
            !task_source.contains(forbidden),
            "CLI task command should delegate model provider composition to ikaros-host instead of keeping {forbidden}"
        );
    }
}

#[test]
fn host_owns_runtime_harness_model_provider_composition() {
    let host_builder =
        fs::read_to_string(workspace_root().join("crates/ikaros-host/src/builder.rs"))
            .expect("read host builder");
    for expected in [
        "pub struct RuntimeHarnessModelServices",
        "pub fn runtime_harness_model_services",
        "pub fn runtime_harness_model_provider",
    ] {
        assert!(
            host_builder.contains(expected),
            "ikaros-host should own runtime harness model provider composition item {expected}"
        );
    }

    for relative in [
        "crates/ikaros-cli/src/agent.rs",
        "crates/ikaros-cli/src/gateway/mod.rs",
        "crates/ikaros-cli/src/schedule.rs",
        "crates/ikaros-cli/src/task.rs",
    ] {
        let source = fs::read_to_string(workspace_root().join(relative))
            .unwrap_or_else(|error| panic!("read {relative}: {error}"));
        assert!(
            source.contains("runtime_harness_model_"),
            "{relative} should call ikaros-host for runtime harness model provider composition"
        );
        for forbidden in [
            "EgressModelHttpClient",
            "governed_provider_from_config_with_http_client",
            "model_request_options_from_config",
        ] {
            assert!(
                !source.contains(forbidden),
                "{relative} should delegate runtime harness model provider composition to ikaros-host instead of keeping {forbidden}"
            );
        }
    }
}

#[test]
fn host_is_composition_root_not_runtime_or_surface_code() {
    let metadata = workspace_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let host = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-host"))
        .expect("ikaros-host package should exist");
    let deps = package_dependency_names(host);
    let forbidden = HOST_FORBIDDEN_DEPENDENCIES
        .iter()
        .copied()
        .filter(|dependency| deps.contains(dependency))
        .collect::<Vec<_>>();
    assert!(
        forbidden.is_empty(),
        "ikaros-host should stay a composition root and must not depend on runtime, surfaces, terminal, or cli code: {forbidden:?}"
    );
}

#[test]
fn core_owns_agent_config_resolution_helpers() {
    let core_agent = fs::read_to_string(workspace_root().join("crates/ikaros-core/src/agent.rs"))
        .expect("read core agent module");
    for expected in ["pub fn resolve_agent(", "pub fn resolve_agent_instance("] {
        assert!(
            core_agent.contains(expected),
            "ikaros-core should own pure agent config resolution helper {expected}"
        );
    }

    let host_builder =
        fs::read_to_string(workspace_root().join("crates/ikaros-host/src/builder.rs"))
            .expect("read host builder");
    for forbidden in ["pub fn resolve_agent(", "pub fn resolve_agent_instance("] {
        assert!(
            !host_builder.contains(forbidden),
            "ikaros-host should not own pure agent config resolution implementation: found {forbidden}"
        );
    }

    let host_lib = fs::read_to_string(workspace_root().join("crates/ikaros-host/src/lib.rs"))
        .expect("read host lib");
    assert!(
        host_lib.contains("pub use ikaros_core::{resolve_agent, resolve_agent_instance}"),
        "ikaros-host may keep compatibility re-exports for agent resolution helpers"
    );
}

#[test]
fn host_owns_init_and_doctor_diagnostics() {
    assert!(
        workspace_root()
            .join("crates/ikaros-host/src/diagnostics/mod.rs")
            .exists(),
        "ikaros-host should own init/doctor diagnostics because they load config and assemble host resources"
    );
    assert!(
        !workspace_root()
            .join("crates/ikaros-runtime/src/diagnostics")
            .exists(),
        "ikaros-runtime should not own a diagnostics implementation directory"
    );
    assert!(
        !workspace_root()
            .join("crates/ikaros-runtime/src/diagnostics.rs")
            .exists(),
        "ikaros-runtime should not keep diagnostics compatibility shims; callers should use ikaros-host"
    );
}

#[test]
fn cli_diagnostics_keeps_setup_prompt_resources_yaml_and_output_split() {
    let diagnostics_rs =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/diagnostics.rs"))
            .expect("read CLI diagnostics root");
    let diagnostics_dir = workspace_root().join("crates/ikaros-cli/src/diagnostics");

    for module in ["output", "setup_prompt", "setup_resources", "setup_yaml"] {
        assert!(
            diagnostics_dir.join(format!("{module}.rs")).exists(),
            "diagnostics/{module}.rs should own a focused diagnostics command slice"
        );
        assert!(
            diagnostics_rs.contains(&format!("mod {module};")),
            "diagnostics.rs should declare the focused {module} submodule"
        );
    }
    assert!(
        diagnostics_rs.lines().count() <= 320,
        "diagnostics.rs should stay as a thin init/setup/doctor command root"
    );

    let output = fs::read_to_string(diagnostics_dir.join("output.rs"))
        .expect("read diagnostics output module");
    let setup_prompt = fs::read_to_string(diagnostics_dir.join("setup_prompt.rs"))
        .expect("read diagnostics setup prompt module");
    let setup_resources = fs::read_to_string(diagnostics_dir.join("setup_resources.rs"))
        .expect("read diagnostics setup resources module");
    let setup_yaml = fs::read_to_string(diagnostics_dir.join("setup_yaml.rs"))
        .expect("read diagnostics setup yaml module");

    assert!(
        output.contains("print_init_report")
            && output.contains("print_doctor_report")
            && output.contains("display_optional_model"),
        "diagnostics/output.rs should own init/doctor output formatting"
    );
    assert!(
        setup_prompt.contains("prompt_setup_args")
            && setup_prompt.contains("prompt_remote_resource")
            && setup_prompt.contains("prompt_yes_no"),
        "diagnostics/setup_prompt.rs should own interactive setup prompting"
    );
    assert!(
        setup_resources.contains("normalize_provider")
            && setup_resources.contains("setup_embedding")
            && setup_resources.contains("setup_voice")
            && setup_resources.contains("apply_reused_model_provider_resources"),
        "diagnostics/setup_resources.rs should own provider/resource setup normalization"
    );
    assert!(
        setup_yaml.contains("set_yaml_scalar")
            && setup_yaml.contains("yaml_key")
            && setup_yaml.contains("format_setup_validation_failure"),
        "diagnostics/setup_yaml.rs should own YAML patching and setup validation formatting"
    );

    for forbidden in [
        "fn prompt_setup_args",
        "fn normalize_provider",
        "fn setup_embedding",
        "fn print_doctor_report",
        "fn set_yaml_scalar",
        "fn yaml_key",
    ] {
        assert!(
            !diagnostics_rs.contains(forbidden),
            "diagnostics.rs should stay a thin command root; move {forbidden} into diagnostics/* modules"
        );
    }
}

#[test]
fn host_owns_relationship_composition() {
    let host_relationship =
        fs::read_to_string(workspace_root().join("crates/ikaros-host/src/relationship.rs"))
            .expect("read host relationship module");
    assert!(
        host_relationship.contains("session_and_registry"),
        "ikaros-host should own relationship commands that assemble sessions and registries"
    );
}

#[test]
fn host_owns_agent_profile_report_composition() {
    let host_agent = fs::read_to_string(workspace_root().join("crates/ikaros-host/src/agent.rs"))
        .expect("read host agent module");
    for expected in ["agent_profiles_report", "agent_profile_report"] {
        assert!(
            host_agent.contains(expected),
            "ikaros-host should own agent profile report helper {expected}"
        );
    }

    let cli_agent = fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/agent.rs"))
        .expect("read CLI agent module");
    assert!(
        cli_agent.contains("agent_profiles_report") && cli_agent.contains("agent_profile_report"),
        "CLI agent command should call host-owned profile report helpers"
    );
    for forbidden in [
        "IkarosConfig::load",
        "resolve_agent(",
        "fn agent_list",
        "fn agent_show",
        "redact_json",
    ] {
        assert!(
            !cli_agent.contains(forbidden),
            "CLI agent command should delegate profile report composition to ikaros-host instead of keeping {forbidden}"
        );
    }
}

#[test]
fn host_owns_surfaces_api_chat_model_composition() {
    let host_builder =
        fs::read_to_string(workspace_root().join("crates/ikaros-host/src/builder.rs"))
            .expect("read host builder module");
    for expected in [
        "pub struct ApiModelServices",
        "api_model_services_shape_checked",
    ] {
        assert!(
            host_builder.contains(expected),
            "ikaros-host should own OpenAI-compatible API chat model composition item {expected}"
        );
    }

    let chat_api =
        fs::read_to_string(workspace_root().join("crates/ikaros-surfaces/src/api/openai/chat.rs"))
            .expect("read API chat route");
    assert!(
        chat_api.contains("api_model_services_shape_checked"),
        "API chat routes should use host-owned model services"
    );
    for forbidden in [
        "session_and_registry_for_instance",
        "governed_provider_from_config_with_http_client",
        "EgressModelHttpClient",
    ] {
        assert!(
            !chat_api.contains(forbidden),
            "API chat routes should delegate model provider/session composition to ikaros-host instead of keeping {forbidden}"
        );
    }
}

#[test]
fn host_owns_surfaces_api_provider_resource_composition() {
    let host_api_provider =
        fs::read_to_string(workspace_root().join("crates/ikaros-host/src/api_provider.rs"))
            .expect("read host API provider module");
    for expected in [
        "api_embedding_services_shape_checked",
        "send_provider_json_request_for_instance",
        "send_provider_bytes_request_for_instance",
        "send_model_json_request_for_instance",
    ] {
        assert!(
            host_api_provider.contains(expected),
            "ikaros-host should own API provider/resource composition helper {expected}"
        );
    }

    assert_source_tree_excludes(
        "crates/ikaros-surfaces/src/api",
        &[
            "session_and_registry_for_instance",
            "EgressModelHttpClient",
            "with_execution_env_embedding_provider",
            "NetworkEgressRequest",
            "ModelHttpRequest",
            "ModelHttpClient",
            "send_api_json_provider_request",
            "send_api_bytes_provider_request",
            "provider_network_request",
        ],
        "ikaros-surfaces API routes should delegate provider/session/egress composition to ikaros-host",
    );
}

#[test]
fn host_owns_mcp_config_projection() {
    let host_mcp = fs::read_to_string(workspace_root().join("crates/ikaros-host/src/mcp.rs"))
        .expect("read host mcp module");
    for expected in ["mcp_servers_report", "configured_mcp_server"] {
        assert!(
            host_mcp.contains(expected),
            "ikaros-host should own MCP config projection helper {expected}"
        );
    }

    let cli_mcp_root = fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/mcp.rs"))
        .expect("read CLI MCP module");
    let cli_mcp_status =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/mcp/status.rs"))
            .expect("read CLI MCP status module");
    let cli_mcp_stdio =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/mcp/stdio.rs"))
            .expect("read CLI MCP stdio module");
    assert!(
        cli_mcp_root.contains("mod status;") && cli_mcp_root.contains("mod stdio;"),
        "CLI MCP root should stay a dispatch facade over focused MCP modules"
    );
    assert!(
        cli_mcp_status.contains("mcp_servers_report")
            && cli_mcp_stdio.contains("configured_mcp_server"),
        "CLI MCP focused modules should call host-owned config projection helpers"
    );
    for forbidden in [
        "IkarosConfig::load",
        "fn mcp_status_json",
        "NetworkEgressRequest",
        "NetworkEgressResponse",
        "send_network_request",
        "McpHttpProbeResponse",
        "fn mcp_initialize_request",
        "fn filter_mcp_tools",
    ] {
        for (relative, source) in [
            ("crates/ikaros-cli/src/mcp.rs", cli_mcp_root.as_str()),
            (
                "crates/ikaros-cli/src/mcp/status.rs",
                cli_mcp_status.as_str(),
            ),
            ("crates/ikaros-cli/src/mcp/stdio.rs", cli_mcp_stdio.as_str()),
        ] {
            assert!(
                !source.contains(forbidden),
                "{relative} should delegate config projection and MCP wire behavior instead of keeping {forbidden}"
            );
        }
    }
    let cli_mcp_http =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/mcp/http.rs"))
            .expect("read CLI MCP HTTP module");
    for forbidden in [
        "NetworkEgressRequest",
        "NetworkEgressResponse",
        "send_network_request",
        "McpHttpProbeResponse",
        "fn mcp_initialize_request",
        "fn filter_mcp_tools",
    ] {
        assert!(
            !cli_mcp_http.contains(forbidden),
            "CLI MCP HTTP command should delegate MCP wire behavior to ikaros-surfaces instead of keeping {forbidden}"
        );
    }
}

#[test]
fn host_owns_memory_resource_composition() {
    let host_memory = fs::read_to_string(workspace_root().join("crates/ikaros-host/src/memory.rs"))
        .expect("read host memory module");
    for expected in [
        "local_memory_store",
        "memory_projection_stores",
        "memory_candidate_stores",
        "memory_provider_registry",
    ] {
        assert!(
            host_memory.contains(expected),
            "ikaros-host should own memory resource composition helper {expected}"
        );
    }

    let mut cli_memory_tree =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/memory.rs"))
            .expect("read CLI memory module");
    let mut stack = vec![workspace_root().join("crates/ikaros-cli/src/memory")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap_or_else(|error| {
            panic!(
                "read CLI memory source directory {}: {error}",
                dir.strip_prefix(workspace_root()).unwrap_or(&dir).display()
            )
        }) {
            let path = entry.expect("read CLI memory source entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }
            cli_memory_tree.push('\n');
            cli_memory_tree.push_str(&fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!(
                    "read CLI memory source file {}: {error}",
                    path.strip_prefix(workspace_root())
                        .unwrap_or(&path)
                        .display()
                )
            }));
        }
    }
    for expected in [
        "local_memory_store",
        "memory_projection_stores",
        "memory_candidate_stores",
        "memory_provider_registry",
    ] {
        assert!(
            cli_memory_tree.contains(expected),
            "CLI memory command should call host-owned memory helper {expected}"
        );
    }
    for forbidden in [
        "IkarosConfig::load",
        "LocalMemoryStore::new(&paths.memory_dir",
        "MemoryProviderRegistry::from_config",
    ] {
        assert!(
            !cli_memory_tree.contains(forbidden),
            "CLI memory command should delegate resource composition to ikaros-host instead of keeping {forbidden}"
        );
    }
}

#[test]
fn cli_memory_keeps_provider_and_supersession_split() {
    let memory_rs = fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/memory.rs"))
        .expect("read CLI memory root");
    let memory_dir = workspace_root().join("crates/ikaros-cli/src/memory");
    for module in [
        "candidate",
        "journal",
        "parse",
        "projection",
        "provider",
        "supersession",
        "working",
    ] {
        assert!(
            memory_dir.join(format!("{module}.rs")).exists(),
            "memory/{module}.rs should own a focused memory command slice"
        );
        assert!(
            memory_rs.contains(&format!("mod {module};")),
            "memory.rs should declare the focused {module} submodule"
        );
    }
    assert!(
        memory_rs.lines().count() <= 360,
        "memory.rs should stay as a command definition and dispatch root after memory command splitting"
    );

    let candidate = fs::read_to_string(memory_dir.join("candidate.rs"))
        .expect("read CLI memory candidate module");
    let journal =
        fs::read_to_string(memory_dir.join("journal.rs")).expect("read CLI memory journal module");
    let parse =
        fs::read_to_string(memory_dir.join("parse.rs")).expect("read CLI memory parse module");
    let projection = fs::read_to_string(memory_dir.join("projection.rs"))
        .expect("read CLI memory projection module");
    let provider = fs::read_to_string(memory_dir.join("provider.rs"))
        .expect("read CLI memory provider module");
    let supersession = fs::read_to_string(memory_dir.join("supersession.rs"))
        .expect("read CLI memory supersession module");
    let working =
        fs::read_to_string(memory_dir.join("working.rs")).expect("read CLI memory working module");
    assert!(
        candidate.contains("memory_candidate_command")
            && candidate.contains("memory_candidate_stores"),
        "memory/candidate.rs should own candidate review/listing over host-composed stores"
    );
    assert!(
        journal.contains("append_candidate_journal")
            && journal.contains("append_working_memory_expired_journal"),
        "memory/journal.rs should own memory journal append helpers"
    );
    assert!(
        parse.contains("parse_candidate_status") && parse.contains("parse_memory_kind"),
        "memory/parse.rs should own memory CLI parse helpers"
    );
    assert!(
        projection.contains("memory_projection_command")
            && projection.contains("memory_projection_stores")
            && projection.contains("refresh_default_projection"),
        "memory/projection.rs should own projection commands over host-composed stores"
    );
    assert!(
        provider.contains("memory_provider_command")
            && provider.contains("memory_provider_registry"),
        "memory/provider.rs should own memory provider command output over host-composed registry"
    );
    assert!(
        supersession.contains("memory_supersession_command")
            && supersession.contains("memory_supersession_chain")
            && supersession.contains("local_memory_store"),
        "memory/supersession.rs should own supersession explanation over host-composed memory store"
    );
    assert!(
        working.contains("memory_working_command")
            && working.contains("JsonlWorkingMemoryStore")
            && working.contains("append_working_memory_expired_journal"),
        "memory/working.rs should own working memory commands and pruning output"
    );
    for forbidden in [
        "fn memory_candidate_command",
        "fn memory_projection_command",
        "fn memory_provider_command",
        "fn memory_supersession_command",
        "fn memory_supersession_chain",
        "fn memory_working_command",
        "fn parse_memory_kind",
    ] {
        assert!(
            !memory_rs.contains(forbidden),
            "memory.rs should stay a command dispatcher; move {forbidden} into memory/* modules"
        );
    }
}

#[test]
fn host_owns_approval_resolution_composition() {
    let host_approval =
        fs::read_to_string(workspace_root().join("crates/ikaros-host/src/approval.rs"))
            .expect("read host approval module");
    assert!(
        host_approval.contains("record_approval_resolution"),
        "ikaros-host should own approval resolution because it resolves agent session storage"
    );
    assert!(
        host_approval.contains("resolve_agent_instance"),
        "host approval resolution should compose the agent-local session store"
    );

    assert_source_tree_excludes(
        "crates/ikaros-agent/src",
        &["record_approval_resolution"],
        "ikaros-agent should not re-export approval resolution composition; CLI/surfaces should call host",
    );
}

#[test]
fn host_owns_egress_model_http_client_adapter() {
    let host_model_http =
        fs::read_to_string(workspace_root().join("crates/ikaros-host/src/model_http.rs"))
            .expect("read host model http module");
    assert!(
        host_model_http.contains("pub struct EgressModelHttpClient"),
        "ikaros-host should own the execution-egress to provider-http composition adapter"
    );
    assert!(
        host_model_http.contains("impl ModelHttpClient for EgressModelHttpClient"),
        "host model HTTP adapter should implement the provider HTTP trait"
    );

    assert!(
        !workspace_root()
            .join("crates/ikaros-runtime/src/model_http.rs")
            .exists(),
        "ikaros-runtime should not keep model_http compatibility shims; callers should use ikaros-host"
    );

    let agent_lib = fs::read_to_string(workspace_root().join("crates/ikaros-agent/src/lib.rs"))
        .expect("read agent lib");
    assert!(
        !agent_lib.contains("pub mod model_http"),
        "ikaros-agent should not expose host composition adapters"
    );
    assert_source_tree_excludes(
        "crates/ikaros-cli/src",
        &[
            "ikaros_agent::model_http",
            "ikaros_runtime::EgressModelHttpClient",
        ],
        "CLI should import EgressModelHttpClient from ikaros-host, not agent/runtime facades",
    );
    assert_source_tree_excludes(
        "crates/ikaros-surfaces/src",
        &["ikaros_runtime::EgressModelHttpClient"],
        "surfaces should import EgressModelHttpClient from ikaros-host",
    );
}

#[test]
fn host_owns_interactive_chat_runtime_composition() {
    let host_builder =
        fs::read_to_string(workspace_root().join("crates/ikaros-host/src/builder.rs"))
            .expect("read host builder");
    for expected in [
        "pub struct ChatModelServices",
        "pub struct ChatRuntimeServices",
        "pub fn chat_model_services_for_session",
        "pub fn chat_model_services_shape_checked",
        "pub fn chat_runtime_services_for_instance",
    ] {
        assert!(
            host_builder.contains(expected),
            "ikaros-host should own interactive chat runtime composition API {expected}"
        );
    }

    for relative in [
        "crates/ikaros-cli/src/chat/runtime.rs",
        "crates/ikaros-cli/src/chat/interactive/agent.rs",
        "crates/ikaros-cli/src/chat/interactive/provider.rs",
    ] {
        let source = fs::read_to_string(workspace_root().join(relative)).expect("read CLI source");
        for forbidden in [
            "governed_provider_from_config_with_http_client",
            "EgressModelHttpClient",
            "runtime_execution_env",
            "ExecutionSession::new_with_agent_instance",
            "model_request_options_from_config",
            "IkarosConfig::load",
            "IkarosConfig::load_shape_checked",
        ] {
            assert!(
                !source.contains(forbidden),
                "{relative} should delegate interactive chat composition to ikaros-host instead of using {forbidden}"
            );
        }
    }
}

#[test]
fn host_owns_chat_command_agent_context_composition() {
    let relative = "crates/ikaros-cli/src/chat/command.rs";
    let source = fs::read_to_string(workspace_root().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"));
    assert!(
        source.contains("host_agent_context"),
        "{relative} should call ikaros-host for chat command agent context composition"
    );
    for forbidden in [
        "IkarosConfig::load",
        "IkarosConfig::load_shape_checked",
        "resolve_agent_instance(",
    ] {
        assert!(
            !source.contains(forbidden),
            "{relative} should delegate chat command agent context composition to ikaros-host instead of keeping {forbidden}"
        );
    }
}

#[test]
fn host_owns_session_state_db_candidate_resolution_for_chat_history_and_workbench() {
    let host_builder =
        fs::read_to_string(workspace_root().join("crates/ikaros-host/src/builder.rs"))
            .expect("read host builder module");
    for expected in [
        "pub fn session_state_db_candidates",
        "pub fn session_state_db_candidates_with_config",
        "resolve_agent_instance",
        "agents_dir",
    ] {
        assert!(
            host_builder.contains(expected),
            "ikaros-host should own session state-db candidate resolution detail {expected}"
        );
    }

    let cli_history =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/chat/history.rs"))
            .expect("read CLI chat history module");
    assert!(
        cli_history.contains("session_state_db_candidates"),
        "CLI chat history should call the host session state-db candidate API"
    );
    for forbidden in [
        "IkarosConfig::load",
        "resolve_agent_instance",
        "fs::read_dir",
        "push_chat_state_db_candidate",
    ] {
        assert!(
            !cli_history.contains(forbidden),
            "CLI chat history should delegate state-db candidate resolution to ikaros-host instead of keeping {forbidden}"
        );
    }

    let workbench_status =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/chat/workbench/status.rs"))
            .expect("read CLI workbench status module");
    assert!(
        workbench_status.contains("session_state_db_candidates_with_config"),
        "CLI workbench status should call the host session state-db candidate API"
    );
    for forbidden in ["resolve_agent_instance", "fs::read_dir", "BTreeSet"] {
        assert!(
            !workbench_status.contains(forbidden),
            "CLI workbench status should delegate state-db candidate resolution to ikaros-host instead of keeping {forbidden}"
        );
    }

    let debug_session =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/debug/session.rs"))
            .expect("read CLI debug session module");
    assert!(
        debug_session.contains("session_state_db_candidates"),
        "CLI debug session should call the host session state-db candidate API"
    );
    for forbidden in [
        "IkarosConfig::load",
        "resolve_agent_instance",
        "fs::read_dir",
        "push_state_db_candidate",
    ] {
        assert!(
            !debug_session.contains(forbidden),
            "CLI debug session should delegate state-db candidate resolution to ikaros-host instead of keeping {forbidden}"
        );
    }
}

#[test]
fn host_owns_debug_agent_context_composition() {
    let host_builder =
        fs::read_to_string(workspace_root().join("crates/ikaros-host/src/builder.rs"))
            .expect("read host builder module");
    for expected in [
        "pub fn host_agent_context",
        "pub fn host_agent_context_shape_checked",
        "pub struct HostAgentContext",
    ] {
        assert!(
            host_builder.contains(expected)
                || fs::read_to_string(workspace_root().join("crates/ikaros-host/src/services.rs"))
                    .expect("read host services module")
                    .contains(expected),
            "ikaros-host should own debug agent context composition item {expected}"
        );
    }

    for relative in [
        "crates/ikaros-cli/src/debug/state_db.rs",
        "crates/ikaros-cli/src/debug/dump.rs",
        "crates/ikaros-cli/src/debug/insights.rs",
        "crates/ikaros-cli/src/debug/readiness.rs",
    ] {
        let source = fs::read_to_string(workspace_root().join(relative))
            .unwrap_or_else(|error| panic!("read {relative}: {error}"));
        assert!(
            source.contains("host_agent_context"),
            "{relative} should call ikaros-host for agent context composition"
        );
        for forbidden in [
            "IkarosConfig::load",
            "IkarosConfig::load_shape_checked",
            "resolve_agent_instance(",
        ] {
            assert!(
                !source.contains(forbidden),
                "{relative} should delegate debug agent context composition to ikaros-host instead of keeping {forbidden}"
            );
        }
    }

    let provider_source =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/debug/provider.rs"))
            .expect("read crates/ikaros-cli/src/debug/provider.rs");
    assert!(
        provider_source.contains("provider_debug_report"),
        "crates/ikaros-cli/src/debug/provider.rs should delegate provider diagnostics to ikaros-host"
    );
    for forbidden in [
        "IkarosConfig::load",
        "IkarosConfig::load_shape_checked",
        "resolve_agent_instance(",
    ] {
        assert!(
            !provider_source.contains(forbidden),
            "crates/ikaros-cli/src/debug/provider.rs should delegate debug agent context composition to ikaros-host instead of keeping {forbidden}"
        );
    }

    let sandbox_source =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/debug/sandbox.rs"))
            .expect("read crates/ikaros-cli/src/debug/sandbox.rs");
    assert!(
        sandbox_source.contains("host_debug_sandbox_report"),
        "crates/ikaros-cli/src/debug/sandbox.rs should delegate sandbox diagnostics to ikaros-host"
    );
    for forbidden in [
        "IkarosConfig::load",
        "IkarosConfig::load_shape_checked",
        "resolve_agent_instance(",
    ] {
        assert!(
            !sandbox_source.contains(forbidden),
            "crates/ikaros-cli/src/debug/sandbox.rs should delegate debug agent context composition to ikaros-host instead of keeping {forbidden}"
        );
    }
}

#[test]
fn host_owns_multimodal_cli_agent_context_composition() {
    let host_multimodal =
        fs::read_to_string(workspace_root().join("crates/ikaros-host/src/multimodal.rs"))
            .expect("read host multimodal module");
    for expected in [
        "pub async fn generate_image",
        "pub async fn describe_image",
        "HostImageGenerationRequest",
        "VisionDescribeRequest",
    ] {
        assert!(
            host_multimodal.contains(expected),
            "ikaros-host should own multimodal provider/session composition item {expected}"
        );
    }

    for (relative, expected_host_request) in [
        (
            "crates/ikaros-cli/src/image.rs",
            "HostImageGenerationRequest",
        ),
        ("crates/ikaros-cli/src/vision.rs", "VisionDescribeRequest"),
    ] {
        let source = fs::read_to_string(workspace_root().join(relative))
            .unwrap_or_else(|error| panic!("read {relative}: {error}"));
        assert!(
            source.contains(expected_host_request),
            "{relative} should call ikaros-host for multimodal request handling"
        );
        for forbidden in [
            "IkarosConfig::load",
            "IkarosConfig::load_shape_checked",
            "resolve_agent_instance(",
            "session_and_registry_for_instance",
            "EgressModelHttpClient",
            "governed_provider_from_config_with_http_client",
            "ModelHttpClient",
            "NetworkEgressRequest",
        ] {
            assert!(
                !source.contains(forbidden),
                "{relative} should delegate multimodal agent context composition to ikaros-host instead of keeping {forbidden}"
            );
        }
    }
}

#[test]
fn host_owns_cli_command_runtime_composition() {
    for relative in [
        "crates/ikaros-cli/src/skill.rs",
        "crates/ikaros-cli/src/self_modify.rs",
    ] {
        let source = fs::read_to_string(workspace_root().join(relative))
            .unwrap_or_else(|error| panic!("read {relative}: {error}"));
        assert!(
            source.contains("runtime_harness"),
            "{relative} should call ikaros-host for command runtime composition"
        );
        for forbidden in [
            "IkarosConfig::load",
            "IkarosConfig::load_shape_checked",
            "resolve_agent_instance(",
            "session_and_registry(",
        ] {
            assert!(
                !source.contains(forbidden),
                "{relative} should delegate command runtime composition to ikaros-host instead of keeping {forbidden}"
            );
        }
    }
}

#[test]
fn host_owns_provider_cli_agent_context_composition() {
    let host_probe =
        fs::read_to_string(workspace_root().join("crates/ikaros-host/src/provider_probe.rs"))
            .expect("read host provider probe module");
    for expected in [
        "pub struct ModelProviderLiveProbeReport",
        "pub async fn model_provider_live_probe",
        "pub async fn embedding_provider_live_probe",
        "pub async fn tts_provider_live_probe",
        "pub async fn asr_provider_live_probe",
    ] {
        assert!(
            host_probe.contains(expected),
            "ikaros-host should own provider live model probe composition item {expected}"
        );
    }

    let relative = "crates/ikaros-cli/src/provider.rs";
    let source = fs::read_to_string(workspace_root().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"));
    assert!(
        source.contains("provider_matrix_report"),
        "{relative} should call ikaros-host for provider command report composition"
    );
    assert!(
        source.contains("provider_health_report"),
        "{relative} should call ikaros-host for provider live model probe composition"
    );
    for forbidden in [
        "ExecutionEnv",
        "IkarosConfig::load",
        "IkarosConfig::load_shape_checked",
        "NetworkEgressRequest",
        "resolve_agent_instance(",
        "runtime_execution_env",
        "host_agent_context",
        "EgressModelHttpClient",
        "governed_provider_from_config_with_http_client",
        "ModelRequest::from_user_text",
        "model_provider_live_probe",
    ] {
        assert!(
            !source.contains(forbidden),
            "{relative} should delegate provider command agent context composition to ikaros-host instead of keeping {forbidden}"
        );
    }
}

#[test]
fn host_owns_provider_command_diagnostics() {
    let host_provider = source_file_and_tree_contents(
        "crates/ikaros-host/src/diagnostics/provider.rs",
        "crates/ikaros-host/src/diagnostics/provider",
    );
    for expected in [
        "pub struct ProviderInspectReport",
        "pub fn provider_inspect_report",
        "pub struct ProviderProfilesReport",
        "pub fn provider_profiles_report",
        "pub enum ProviderHealthReport",
        "pub async fn provider_health_report",
        "pub struct ProviderMatrixReport",
        "pub async fn provider_matrix_report",
        "openai_compatible_profile_catalog",
        "Ikaros provider health probe",
        "Ikaros provider matrix live probe",
    ] {
        assert!(
            host_provider.contains(expected),
            "ikaros-host should own provider command diagnostics item {expected}"
        );
    }

    let cli_provider =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/provider.rs"))
            .expect("read CLI provider command module");
    for expected in [
        "provider_inspect_report",
        "provider_profiles_report",
        "provider_health_report",
        "provider_matrix_report",
        "ProviderHealthReport",
        "ProviderMatrixRow",
    ] {
        assert!(
            cli_provider.contains(expected),
            "CLI provider command should call ikaros-host provider diagnostics item {expected}"
        );
    }
    for forbidden in [
        "fn print_fallback_inspect_rows",
        "fn format_marker_list",
        "fn provider_live_probe",
        "fn matrix_usage_summary",
        "fn matrix_estimated_cost_today",
        "fn apply_configured_model_cost",
        "fn live_smoke_state",
        "ProviderRegistry",
        "ProviderHealthLedger",
        "ModelProviderDescriptor",
        "ModelUsageLedger",
        "ModelUsageRecord",
        "RemoteProviderConfig",
        "openai_compatible_profile_catalog",
        "Ikaros provider health probe",
        "Ikaros provider matrix live probe",
    ] {
        assert!(
            !cli_provider.contains(forbidden),
            "CLI provider command should delegate inspect/profile/health diagnostics to ikaros-host instead of keeping {forbidden}"
        );
    }
}

#[test]
fn host_owns_workbench_provider_status_projection() {
    let host_provider = source_file_and_tree_contents(
        "crates/ikaros-host/src/diagnostics/provider.rs",
        "crates/ikaros-host/src/diagnostics/provider",
    );
    for expected in [
        "pub struct WorkbenchProviderStatusReport",
        "pub struct WorkbenchModelBudgetStatus",
        "pub struct WorkbenchModelCostStatus",
        "pub struct WorkbenchProviderHealthStatus",
        "pub fn workbench_provider_status_report",
        "pub fn model_budget_status_report",
        "ProviderRegistry",
        "ProviderHealthLedger",
        "ModelUsageLedger",
        "descriptor_with_profile",
        "apply_configured_model_cost",
    ] {
        assert!(
            host_provider.contains(expected),
            "ikaros-host should own workbench provider status projection item {expected}"
        );
    }

    let workbench_status_root =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/chat/workbench/status.rs"))
            .expect("read CLI workbench status root");
    assert!(
        workbench_status_root.contains("mod unified;"),
        "CLI workbench status root should route unified status through a focused submodule"
    );

    for relative in [
        "crates/ikaros-cli/src/chat/workbench/status/provider.rs",
        "crates/ikaros-cli/src/chat/workbench/status/screen.rs",
        "crates/ikaros-cli/src/chat/workbench/status/unified.rs",
    ] {
        let source = fs::read_to_string(workspace_root().join(relative))
            .unwrap_or_else(|error| panic!("read {relative}: {error}"));
        assert!(
            source.contains("WorkbenchProviderStatusReport")
                || source.contains("workbench_provider_status_report")
                || source.contains("active_provider_status_report"),
            "{relative} should consume host-owned workbench provider status projection"
        );
        for forbidden in [
            "ProviderRegistry",
            "ProviderHealthLedger",
            "ModelProviderDescriptor",
            "ModelUsageRecord",
            "descriptor_with_profile",
            "apply_configured_model_cost",
        ] {
            assert!(
                !source.contains(forbidden),
                "{relative} should delegate workbench provider status projection to ikaros-host instead of keeping {forbidden}"
            );
        }
    }
}

#[test]
fn host_owns_debug_provider_diagnostics() {
    let host_provider = source_file_and_tree_contents(
        "crates/ikaros-host/src/diagnostics/provider.rs",
        "crates/ikaros-host/src/diagnostics/provider",
    );
    for expected in [
        "pub fn provider_debug_report",
        "pub fn provider_debug_matrix_report",
        "ProviderRegistry",
        "ProviderHealthLedger",
        "ModelUsageLedger",
        "provider_debug_live_smoke_state",
    ] {
        assert!(
            host_provider.contains(expected),
            "ikaros-host should own provider diagnostics detail {expected}"
        );
    }

    for (relative, expected_call) in [
        (
            "crates/ikaros-cli/src/debug/provider.rs",
            "provider_debug_report",
        ),
        (
            "crates/ikaros-cli/src/debug/readiness.rs",
            "provider_debug_matrix_report",
        ),
        (
            "crates/ikaros-cli/src/debug/insights.rs",
            "provider_debug_matrix_report",
        ),
    ] {
        let source = fs::read_to_string(workspace_root().join(relative))
            .unwrap_or_else(|error| panic!("read {relative}: {error}"));
        assert!(
            source.contains(expected_call),
            "{relative} should call ikaros-host for provider diagnostics"
        );
        for forbidden in [
            "ProviderRegistry",
            "ProviderHealthLedger",
            "ModelProviderDescriptor",
            "provider_debug_matrix_row",
            "provider_debug_live_smoke_state",
            "provider_debug_usage_summary",
            "descriptor_with_profile",
        ] {
            assert!(
                !source.contains(forbidden),
                "{relative} should delegate provider diagnostics to ikaros-host instead of keeping {forbidden}"
            );
        }
    }
}

#[test]
fn host_owns_browser_cdp_http_egress_composition() {
    let host_browser =
        fs::read_to_string(workspace_root().join("crates/ikaros-host/src/browser.rs"))
            .expect("read host browser module");
    for expected in [
        "pub struct BrowserCdpHttpResponse",
        "pub async fn send_browser_cdp_http_request",
        "NetworkEgressRequest",
    ] {
        assert!(
            host_browser.contains(expected),
            "ikaros-host should own browser CDP HTTP egress item {expected}"
        );
    }

    let relative = "crates/ikaros-cli/src/browser/http.rs";
    let source = fs::read_to_string(workspace_root().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"));
    assert!(
        source.contains("send_browser_cdp_http_request"),
        "{relative} should call ikaros-host for browser CDP HTTP egress"
    );
    assert_source_tree_excludes(
        "crates/ikaros-cli/src/browser",
        &[
            "NetworkEgressRequest",
            "NetworkEgressResponse",
            "send_network_request",
        ],
        "CLI browser command modules should delegate browser CDP HTTP egress to ikaros-host",
    );
}

#[test]
fn host_owns_coding_workflow_runtime_composition() {
    let workflow_relative = "crates/ikaros-cli/src/code/workflow.rs";
    let workflow = fs::read_to_string(workspace_root().join(workflow_relative))
        .unwrap_or_else(|error| panic!("read {workflow_relative}: {error}"));
    let review_relative = "crates/ikaros-cli/src/code/review.rs";
    let review = fs::read_to_string(workspace_root().join(review_relative))
        .unwrap_or_else(|error| panic!("read {review_relative}: {error}"));
    let source = format!("{workflow}\n{review}");
    for expected in [
        "runtime_harness",
        "host_agent_context",
        "skill_environment",
        "chat_model_services_for_session",
    ] {
        assert!(
            source.contains(expected),
            "code workflow/review modules should use ikaros-host coding workflow composition helper {expected}"
        );
    }
    for forbidden in [
        "IkarosConfig::load",
        "IkarosConfig::load_shape_checked",
        "resolve_agent_instance(",
        "session_and_registry_for_instance",
        "governed_provider_from_config_with_http_client",
        "EgressModelHttpClient",
    ] {
        assert!(
            !source.contains(forbidden),
            "code workflow/review modules should delegate coding workflow runtime composition to ikaros-host instead of keeping {forbidden}"
        );
    }
    assert_source_tree_excludes(
        "crates/ikaros-cli/src/code",
        &[
            "IkarosConfig::load",
            "IkarosConfig::load_shape_checked",
            "resolve_agent_instance(",
            "session_and_registry_for_instance",
            "governed_provider_from_config_with_http_client",
            "EgressModelHttpClient",
        ],
        "CLI code command modules should not reassemble host runtime resources directly",
    );
}

#[test]
fn host_owns_persona_file_management() {
    let host_persona =
        fs::read_to_string(workspace_root().join("crates/ikaros-host/src/persona.rs"))
            .expect("read host persona module");
    for expected in [
        "pub struct PersonaPatch",
        "pub fn update_persona",
        "pub fn reset_persona",
        "pub fn render_persona_markdown",
    ] {
        assert!(
            host_persona.contains(expected),
            "ikaros-host should own persona file management item {expected}"
        );
    }

    assert!(
        !workspace_root()
            .join("crates/ikaros-runtime/src/persona.rs")
            .exists(),
        "ikaros-runtime should not keep persona file-management compatibility shims; callers should use ikaros-host"
    );

    let agent_lib = fs::read_to_string(workspace_root().join("crates/ikaros-agent/src/lib.rs"))
        .expect("read agent lib");
    assert!(
        !agent_lib.contains("pub mod persona"),
        "ikaros-agent should not expose host persona file-management APIs"
    );
    assert_source_tree_excludes(
        "crates/ikaros-cli/src",
        &["ikaros_agent::persona"],
        "CLI should import persona file-management APIs from ikaros-host",
    );
}

#[test]
fn execution_owns_recent_policy_decision_projection() {
    let execution_session =
        fs::read_to_string(workspace_root().join("crates/ikaros-execution/src/harness/session.rs"))
            .expect("read execution session module");
    assert!(
        execution_session.contains("pub fn recent_policy_decisions"),
        "ikaros-execution should own audit-derived policy decision projections"
    );

    let host_builder =
        fs::read_to_string(workspace_root().join("crates/ikaros-host/src/builder.rs"))
            .expect("read host builder");
    assert!(
        !host_builder.contains("pub fn recent_policy_decisions"),
        "ikaros-host should not own audit-derived policy decision projection logic"
    );

    let host_lib = fs::read_to_string(workspace_root().join("crates/ikaros-host/src/lib.rs"))
        .expect("read host lib");
    assert!(
        host_lib.contains("pub use ikaros_execution::harness::recent_policy_decisions"),
        "ikaros-host may keep only a compatibility re-export for recent policy decisions"
    );
}

#[test]
fn terminal_facade_does_not_own_runtime_state_or_providers() {
    let forbidden = [
        "ikaros_state",
        "ikaros_execution",
        "ikaros_providers",
        "runtime_execution_env",
        "IkarosConfig::load",
    ];
    assert_source_tree_excludes(
        "crates/ikaros-terminal/src",
        &forbidden,
        "ikaros-terminal should render protocol/agent values instead of owning runtime state, execution, provider setup, or config loading",
    );
}

#[test]
fn terminal_owns_workbench_screen_progress_and_palette_projection() {
    let terminal_status =
        fs::read_to_string(workspace_root().join("crates/ikaros-terminal/src/workbench_status.rs"))
            .expect("read terminal workbench status projection module");
    for expected in [
        "pub fn screen_command_palette_cells",
        "pub fn screen_active_work_cells",
        "pub fn screen_progress_status_cell",
        "pub fn progress_footer_summary",
        "pub fn workbench_screen_dimensions_from_values",
    ] {
        assert!(
            terminal_status.contains(expected),
            "ikaros-terminal should own pure workbench screen projection helper {expected}"
        );
    }

    let cli_screen = fs::read_to_string(
        workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/screen.rs"),
    )
    .expect("read CLI screen status module");
    for forbidden in [
        "fn screen_command_palette_cells",
        "fn screen_active_work_cells",
        "fn screen_progress_status_cell",
        "fn progress_footer_summary",
        "fn progress_recovery_commands",
        "fn workbench_screen_dimensions_from_values",
    ] {
        assert!(
            !cli_screen.contains(forbidden),
            "CLI screen status should call ikaros-terminal for pure projection helper {forbidden}"
        );
    }
}

#[test]
fn terminal_facade_does_not_expose_placeholder_run_loop() {
    let forbidden = ["TuiRunConfig", "TuiRunResult", "submitted_input: None"];
    assert_source_tree_excludes(
        "crates/ikaros-terminal/src",
        &forbidden,
        "ikaros-terminal should expose real TUI primitives, not an unused placeholder run-loop facade",
    );
}

#[test]
fn terminal_layout_keeps_dashboard_panel_detail_frame_and_wrapping_split() {
    let terminal_src = workspace_root().join("crates/ikaros-terminal/src");
    let layout_root =
        fs::read_to_string(terminal_src.join("layout.rs")).expect("read terminal layout facade");
    let layout_dir = terminal_src.join("layout");

    for module in ["dashboard", "detail", "frame", "panel", "wrapping"] {
        assert!(
            layout_dir.join(format!("{module}.rs")).exists(),
            "layout/{module}.rs should own a focused terminal layout slice"
        );
        assert!(
            layout_root.contains(&format!("mod {module};")),
            "layout.rs should declare focused {module} submodule"
        );
    }
    assert!(
        layout_root.lines().count() <= 40,
        "layout.rs should stay a thin facade after terminal layout splitting"
    );

    let dashboard =
        fs::read_to_string(layout_dir.join("dashboard.rs")).expect("read layout dashboard");
    let detail = fs::read_to_string(layout_dir.join("detail.rs")).expect("read layout detail");
    let frame = fs::read_to_string(layout_dir.join("frame.rs")).expect("read layout frame");
    let panel = fs::read_to_string(layout_dir.join("panel.rs")).expect("read layout panel");
    let wrapping =
        fs::read_to_string(layout_dir.join("wrapping.rs")).expect("read layout wrapping");

    assert!(
        dashboard.contains("screen_dashboard_model_json")
            && dashboard.contains("dashboard_item_json")
            && dashboard.contains("provider_panel_summary")
            && dashboard.contains("queue_panel_summary"),
        "layout/dashboard.rs should own dashboard model and panel summary helpers"
    );
    assert!(
        panel.contains("panel_paragraph")
            && panel.contains("panel_lines")
            && panel.contains("evidence_attention_summary"),
        "layout/panel.rs should own panel paragraph and attention projection"
    );
    assert!(
        detail.contains("human_cell_detail")
            && detail.contains("human_model_detail")
            && detail.contains("human_queue_detail"),
        "layout/detail.rs should own human cell detail conversion"
    );
    assert!(
        frame.contains("buffer_snapshot")
            && frame.contains("three_column_row")
            && frame.contains("column_widths"),
        "layout/frame.rs should own terminal frame and buffer helpers"
    );
    assert!(
        wrapping.contains("wrap_single_line")
            && wrapping.contains("wrap_markdown_detail")
            && wrapping.contains("markdown_feature_json"),
        "layout/wrapping.rs should own wrapping and markdown feature helpers"
    );

    for forbidden in [
        "fn screen_dashboard_model_json",
        "fn dashboard_item_json",
        "fn panel_paragraph",
        "fn human_cell_detail",
        "fn buffer_snapshot",
        "fn wrap_single_line",
    ] {
        assert!(
            !layout_root.contains(forbidden),
            "layout.rs should stay a thin facade; move {forbidden} into layout/* modules"
        );
    }
    for (name, source, max_lines) in [
        ("dashboard", dashboard.as_str(), 650usize),
        ("detail", detail.as_str(), 180usize),
        ("frame", frame.as_str(), 90usize),
        ("panel", panel.as_str(), 180usize),
        ("wrapping", wrapping.as_str(), 200usize),
    ] {
        assert!(
            source.lines().count() <= max_lines,
            "layout/{name}.rs should stay focused after terminal layout splitting"
        );
    }
}

#[test]
fn terminal_workbench_cell_model_lives_outside_root_facade() {
    let terminal_src = workspace_root().join("crates/ikaros-terminal/src");
    let terminal_lib =
        fs::read_to_string(terminal_src.join("lib.rs")).expect("read ikaros-terminal lib");
    let workbench_cell =
        fs::read_to_string(terminal_src.join("workbench_cell.rs")).expect("read workbench cell");
    let workbench_screen = fs::read_to_string(terminal_src.join("workbench_screen.rs"))
        .expect("read workbench screen");

    for expected in [
        "pub enum WorkbenchCellKind",
        "pub struct WorkbenchCell",
        "pub fn render_workbench_snapshot",
    ] {
        assert!(
            workbench_cell.contains(expected),
            "workbench_cell.rs should own terminal workbench cell API {expected}"
        );
        assert!(
            !contains_rust_declaration_prefix(&terminal_lib, expected),
            "ikaros-terminal lib.rs should stay a facade and not regain {expected}"
        );
    }
    assert!(
        terminal_lib.contains("mod workbench_cell;"),
        "ikaros-terminal lib.rs should declare the focused workbench_cell module"
    );
    for expected in [
        "pub struct WorkbenchScreen",
        "pub enum WorkbenchScreenPanel",
        "pub enum WorkbenchScreenAction",
    ] {
        assert!(
            workbench_screen.contains(expected),
            "workbench_screen.rs should own terminal workbench screen API {expected}"
        );
        assert!(
            !contains_rust_declaration_prefix(&terminal_lib, expected),
            "ikaros-terminal lib.rs should stay a facade and not regain {expected}"
        );
    }
    assert!(
        terminal_lib.contains("mod workbench_screen;"),
        "ikaros-terminal lib.rs should declare the focused workbench_screen module"
    );
    assert!(
        terminal_lib.lines().count() <= 580,
        "ikaros-terminal lib.rs should keep shrinking toward a facade"
    );
    assert!(
        workbench_cell.lines().count() <= 160,
        "workbench_cell.rs should stay focused"
    );
    assert!(
        workbench_screen.lines().count() <= 120,
        "workbench_screen.rs should stay focused"
    );
}

#[test]
fn cli_chat_workbench_does_not_keep_dead_human_output_shims() {
    assert_source_tree_excludes(
        "crates/ikaros-cli/src/chat/workbench",
        &[
            "fn print_tasks_status_for_human",
            "fn print_context_mentions_for_human",
            "fn print_session_history_for_human",
            "fn print_session_summaries_for_human",
        ],
        "CLI chat workbench should use shared human line builders instead of keeping unused print_*_for_human shims",
    );
}

#[test]
fn cli_interactive_dispatcher_keeps_command_families_split() {
    let interactive =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/chat/interactive.rs"))
            .expect("read CLI interactive dispatcher");

    for module in ["gateway", "mcp", "multimodal", "web"] {
        assert!(
            interactive.contains(&format!("mod {module};")),
            "interactive dispatcher should declare the {module} command-family module"
        );
        assert!(
            workspace_root()
                .join(format!(
                    "crates/ikaros-cli/src/chat/interactive/{module}.rs"
                ))
                .exists(),
            "interactive {module} command-family implementation should live in its own module"
        );
    }

    for forbidden in [
        "async fn handle_web_command",
        "fn parse_web_search_input",
        "async fn handle_mcp_command",
        "fn parse_mcp_call_http_args",
        "async fn handle_vision_command",
        "async fn handle_image_command",
        "fn parse_image_generate_args",
        "fn handle_gateway_command",
    ] {
        assert!(
            !interactive.contains(forbidden),
            "chat/interactive.rs should stay a dispatcher; move {forbidden} into a command-family module"
        );
    }
}

#[test]
fn cli_interactive_multimodal_keeps_image_vision_and_result_split() {
    let multimodal_rs = fs::read_to_string(
        workspace_root().join("crates/ikaros-cli/src/chat/interactive/multimodal.rs"),
    )
    .expect("read CLI interactive multimodal facade");
    let multimodal_dir = workspace_root().join("crates/ikaros-cli/src/chat/interactive/multimodal");

    for module in ["image", "result", "vision"] {
        assert!(
            multimodal_dir.join(format!("{module}.rs")).exists(),
            "interactive/multimodal/{module}.rs should own a focused multimodal slice"
        );
        assert!(
            multimodal_rs.contains(&format!("mod {module};")),
            "interactive/multimodal.rs should declare the focused {module} submodule"
        );
    }
    assert!(
        multimodal_rs.lines().count() <= 80,
        "interactive/multimodal.rs should stay as a thin facade after multimodal splitting"
    );

    let image = fs::read_to_string(multimodal_dir.join("image.rs")).expect("read image module");
    let result = fs::read_to_string(multimodal_dir.join("result.rs")).expect("read result module");
    let vision = fs::read_to_string(multimodal_dir.join("vision.rs")).expect("read vision module");
    assert!(
        image.contains("handle_image_command")
            && image.contains("parse_image_generate_args")
            && image.contains("image_generate_skill_input"),
        "multimodal/image.rs should own image command parsing and skill input shaping"
    );
    assert!(
        vision.contains("handle_vision_command") && vision.contains("parse_vision_describe_args"),
        "multimodal/vision.rs should own vision command parsing and output"
    );
    assert!(
        result.contains("redacted_output_json")
            && result.contains("redacted_value_json")
            && result.contains("output_str"),
        "multimodal/result.rs should own shared result extraction and redaction helpers"
    );

    for forbidden in [
        "async fn handle_image_command",
        "fn parse_image_generate_args",
        "async fn handle_vision_command",
        "fn parse_vision_describe_args",
        "fn redacted_output_json",
    ] {
        assert!(
            !multimodal_rs.contains(forbidden),
            "interactive/multimodal.rs should stay a facade; move {forbidden} into multimodal/* modules"
        );
    }
}

#[test]
fn cli_interactive_screen_keeps_open_routing_split() {
    let screen = fs::read_to_string(
        workspace_root().join("crates/ikaros-cli/src/chat/interactive/screen.rs"),
    )
    .expect("read CLI interactive screen module");
    let open_path = workspace_root().join("crates/ikaros-cli/src/chat/interactive/screen/open.rs");
    assert!(
        open_path.exists(),
        "screen open/confirm command routing should live in chat/interactive/screen/open.rs"
    );
    let open = fs::read_to_string(open_path).expect("read CLI interactive screen open module");
    let open_dir = workspace_root().join("crates/ikaros-cli/src/chat/interactive/screen/open");
    for module in [
        "code",
        "command_parse",
        "debug",
        "explicit",
        "outcome",
        "selected",
        "timeline",
    ] {
        assert!(
            open_dir.join(format!("{module}.rs")).exists(),
            "screen/open/{module}.rs should own a focused slice of screen-open behavior"
        );
        assert!(
            open.contains(&format!("mod {module};")),
            "screen/open.rs should declare the focused {module} submodule"
        );
    }
    let command_parse = fs::read_to_string(open_dir.join("command_parse.rs"))
        .expect("read screen open command parser");
    let code = fs::read_to_string(open_dir.join("code.rs")).expect("read screen open code");
    let debug = fs::read_to_string(open_dir.join("debug.rs")).expect("read screen open debug");
    let explicit =
        fs::read_to_string(open_dir.join("explicit.rs")).expect("read screen open explicit");
    let outcome =
        fs::read_to_string(open_dir.join("outcome.rs")).expect("read screen open outcome");
    let selected =
        fs::read_to_string(open_dir.join("selected.rs")).expect("read screen open selected");
    let timeline =
        fs::read_to_string(open_dir.join("timeline.rs")).expect("read screen open timeline");

    assert!(
        screen.contains("mod open;"),
        "screen.rs should declare the focused open routing submodule"
    );
    assert!(
        selected.contains("handle_screen_open_selected_action")
            && selected.contains("selected_screen_primary_action")
            && selected.contains("print_screen_open_selected_status"),
        "screen/open/selected.rs should own screen open selected action routing"
    );
    assert!(
        code.contains("execute_screen_code_command")
            && code.contains("parse_interactive_code_command")
            && code.contains("code_command"),
        "screen/open/code.rs should own selected screen code command execution"
    );
    assert!(
        timeline.contains("execute_replay_status_command")
            && timeline.contains("execute_trace_status_command")
            && timeline.contains("print_replay_status_for_human"),
        "screen/open/timeline.rs should own selected timeline/replay/trace status command execution"
    );
    assert!(
        command_parse.contains("normalize_screen_code_command")
            && command_parse.contains("parse_debug_memory_lifecycle_args")
            && command_parse.contains("screen_budget_command_resumes_pending_inputs"),
        "screen/open/command_parse.rs should own pure screen-open command parsing helpers"
    );
    assert!(
        debug.contains("execute_screen_open_debug_command")
            && debug.contains("debug_memory_lifecycle_json_line")
            && debug.contains("print_replay_status_for_human"),
        "screen/open/debug.rs should own debug selected-action command execution"
    );
    assert!(
        explicit.contains("execute_confirmed_explicit_command")
            && explicit.contains("append_workbench_evidence")
            && explicit.contains("handle_provider_command"),
        "screen/open/explicit.rs should own confirmed explicit selected-action commands"
    );
    assert!(
        outcome.contains("enum ScreenOpenCommandStatus")
            && outcome.contains("screen_open_notice_kind")
            && outcome.contains("print_screen_open_selected_status"),
        "screen/open/outcome.rs should own selected-action status and output helpers"
    );

    for forbidden in [
        "async fn execute_screen_open_command",
        "async fn execute_confirmed_explicit_command",
        "fn normalize_screen_code_command",
        "fn parse_debug_memory_lifecycle_args",
        "enum ScreenOpenCommandStatus",
        "fn screen_open_notice_kind",
    ] {
        assert!(
            !screen.contains(forbidden),
            "chat/interactive/screen.rs should stay focused on screen state actions; move {forbidden} into screen/open.rs"
        );
    }
    for forbidden in [
        "fn normalize_screen_code_command",
        "fn parse_debug_memory_lifecycle_args",
        "fn screen_budget_command_resumes_pending_inputs",
        "fn code_option_takes_value",
        "async fn execute_screen_code_command",
        "async fn handle_screen_open_selected_action",
        "async fn execute_confirmed_explicit_command",
        "async fn execute_screen_open_debug_command",
        "fn execute_replay_status_command",
        "fn execute_trace_status_command",
        "enum ScreenOpenCommandStatus",
        "fn screen_open_notice_kind",
    ] {
        assert!(
            !open.contains(forbidden),
            "screen/open.rs should stay a focused router; move {forbidden} into screen/open/* modules"
        );
    }
    assert!(
        open.lines().count() <= 460,
        "screen/open.rs should keep shrinking as selected-action families split out"
    );
    for (name, source, max_lines) in [
        ("code", code.as_str(), 80usize),
        ("command_parse", command_parse.as_str(), 220usize),
        ("debug", debug.as_str(), 180usize),
        ("explicit", explicit.as_str(), 120usize),
        ("outcome", outcome.as_str(), 80usize),
        ("selected", selected.as_str(), 120usize),
        ("timeline", timeline.as_str(), 90usize),
    ] {
        assert!(
            source.lines().count() <= max_lines,
            "screen/open/{name}.rs should stay focused after screen-open splitting"
        );
    }
}

#[test]
fn cli_workbench_status_timeline_keeps_projection_output_trace_split() {
    let timeline_file =
        workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/timeline.rs");
    assert!(
        !timeline_file.exists(),
        "workbench status timeline should be split into a focused timeline/ module directory, not kept as a large timeline.rs file"
    );

    let timeline_dir =
        workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/timeline");
    let mod_rs =
        fs::read_to_string(timeline_dir.join("mod.rs")).expect("read timeline module root");
    for module in [
        "cells",
        "classification",
        "events",
        "filter",
        "human",
        "output",
        "request",
        "trace",
    ] {
        assert!(
            timeline_dir.join(format!("{module}.rs")).exists(),
            "timeline/{module}.rs should own a focused slice of workbench timeline behavior"
        );
        assert!(
            mod_rs.contains(&format!("mod {module};")),
            "timeline/mod.rs should declare the focused {module} submodule"
        );
    }

    let cells = fs::read_to_string(timeline_dir.join("cells.rs")).expect("read timeline cells");
    let output = fs::read_to_string(timeline_dir.join("output.rs")).expect("read timeline output");
    let trace = fs::read_to_string(timeline_dir.join("trace.rs")).expect("read timeline trace");
    let filter = fs::read_to_string(timeline_dir.join("filter.rs")).expect("read timeline filter");
    let request =
        fs::read_to_string(timeline_dir.join("request.rs")).expect("read timeline request");

    assert!(
        cells.contains("screen_timeline_cells_from_replay")
            && cells.contains("screen_coding_cells_from_replay"),
        "timeline/cells.rs should own replay-to-workbench-cell projections"
    );
    assert!(
        output.contains("print_replay_status") && output.contains("print_replay_status_for_human"),
        "timeline/output.rs should own replay/timeline command output"
    );
    assert!(
        trace.contains("print_trace_status") && trace.contains("print_screen_trace_snapshot"),
        "timeline/trace.rs should own trace command output"
    );
    assert!(
        filter.contains("timeline_point_matches")
            && filter.contains("filtered_events_for_timeline"),
        "timeline/filter.rs should own timeline filtering and point matching"
    );
    assert!(
        request.contains("struct TimelineRequest") && request.contains("enum TimelineVerbosity"),
        "timeline/request.rs should own timeline command request types"
    );

    for (name, source) in [
        ("cells", cells.as_str()),
        ("output", output.as_str()),
        ("trace", trace.as_str()),
        ("filter", filter.as_str()),
    ] {
        assert!(
            source.lines().count() <= 600,
            "timeline/{name}.rs should stay below the large-file threshold after the split"
        );
    }
}

#[test]
fn cli_workbench_context_status_keeps_cells_output_and_value_split() {
    let context_rs = fs::read_to_string(
        workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/context.rs"),
    )
    .expect("read context status root");
    let context_dir = workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/context");

    for module in ["cells", "output", "value"] {
        assert!(
            context_dir.join(format!("{module}.rs")).exists(),
            "context/{module}.rs should own a focused slice of context status behavior"
        );
        assert!(
            context_rs.contains(&format!("mod {module};")),
            "status/context.rs should declare the focused {module} submodule"
        );
    }
    assert!(
        context_rs.lines().count() <= 180,
        "status/context.rs should stay as a thin command/status root after context splitting"
    );

    let cells = fs::read_to_string(context_dir.join("cells.rs")).expect("read context cells");
    let output = fs::read_to_string(context_dir.join("output.rs")).expect("read context output");
    let value = fs::read_to_string(context_dir.join("value.rs")).expect("read context value");

    assert!(
        cells.contains("screen_context_cells_from_replay")
            && cells.contains("screen_context_current_cells")
            && cells.contains("screen_context_budget_cell"),
        "context/cells.rs should own context replay/current screen-cell projection"
    );
    assert!(
        output.contains("context_status_human_lines")
            && output.contains("context_status_json_line")
            && output.contains("print_latest_prompt_sections"),
        "context/output.rs should own context human/json output and prompt-section dumping"
    );
    assert!(
        value.contains("json_str")
            && value.contains("json_array_len")
            && value.contains("format_named_counts"),
        "context/value.rs should own small JSON extraction/format helpers"
    );

    for (name, source, max_lines) in [
        ("cells", cells.as_str(), 500usize),
        ("output", output.as_str(), 450usize),
        ("value", value.as_str(), 120usize),
    ] {
        assert!(
            source.lines().count() <= max_lines,
            "context/{name}.rs should stay focused after the split"
        );
    }

    for forbidden in [
        "fn screen_context_budget_cell",
        "fn screen_context_section_cells",
        "fn context_status_json_line",
        "fn print_latest_prompt_sections",
        "fn format_named_counts",
    ] {
        assert!(
            !context_rs.contains(forbidden),
            "status/context.rs should stay a thin root; move {forbidden} into context/* modules"
        );
    }
}

#[test]
fn cli_workbench_screen_status_keeps_cache_cells_and_conversation_split() {
    let screen_rs = fs::read_to_string(
        workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/screen.rs"),
    )
    .expect("read screen status root");
    let screen_dir = workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/screen");

    for module in ["cache", "cells", "conversation"] {
        assert!(
            screen_dir.join(format!("{module}.rs")).exists(),
            "screen/{module}.rs should own a focused slice of screen status behavior"
        );
        assert!(
            screen_rs.contains(&format!("mod {module};")),
            "status/screen.rs should declare the focused {module} submodule"
        );
    }
    assert!(
        screen_rs.lines().count() <= 420,
        "status/screen.rs should stay as a thin screen build/render root after splitting"
    );

    let cache = fs::read_to_string(screen_dir.join("cache.rs")).expect("read screen cache");
    let cells = fs::read_to_string(screen_dir.join("cells.rs")).expect("read screen cells");
    let conversation =
        fs::read_to_string(screen_dir.join("conversation.rs")).expect("read screen conversation");

    assert!(
        cache.contains("apply_progress_to_cached_screen")
            && cache.contains("apply_pending_user_input_to_cached_screen")
            && cache.contains("apply_live_model_stream_to_cached_screen"),
        "screen/cache.rs should own cached-screen test update helpers"
    );
    assert!(
        cells.contains("screen_attachment_cells")
            && cells.contains("screen_bottom_pane_status_cell")
            && cells.contains("screen_sandbox_cell")
            && cells.contains("screen_observability_cell"),
        "screen/cells.rs should own static/system screen-cell projections"
    );
    assert!(
        conversation.contains("screen_conversation_cells")
            && conversation.contains("screen_conversation_cell"),
        "screen/conversation.rs should own session replay conversation projection"
    );

    for (name, source, max_lines) in [
        ("cache", cache.as_str(), 180usize),
        ("cells", cells.as_str(), 280usize),
        ("conversation", conversation.as_str(), 160usize),
    ] {
        assert!(
            source.lines().count() <= max_lines,
            "screen/{name}.rs should stay focused after the split"
        );
    }

    for forbidden in [
        "fn apply_progress_to_cached_screen",
        "fn screen_attachment_cells",
        "fn screen_bottom_pane_status_cell",
        "fn screen_conversation_cells",
        "fn screen_sandbox_cell",
        "fn screen_observability_cell",
    ] {
        assert!(
            !screen_rs.contains(forbidden),
            "status/screen.rs should stay a thin root; move {forbidden} into screen/* modules"
        );
    }
}

#[test]
fn cli_workbench_status_keeps_tests_out_of_production_module() {
    let status_rs =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/chat/workbench/status.rs"))
            .expect("read workbench status module");
    let unified_path =
        workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/unified.rs");
    let tests_path = workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/tests.rs");

    assert!(
        tests_path.exists(),
        "large workbench status tests should live in status/tests.rs, not inline in status.rs"
    );
    assert!(
        status_rs.contains("mod tests;"),
        "status.rs should keep a narrow cfg(test) module declaration"
    );
    assert!(
        !status_rs.contains("mod tests {"),
        "status.rs should not keep the large inline test module"
    );
    assert!(
        unified_path.exists(),
        "unified workbench status output should live in status/unified.rs instead of bloating status.rs"
    );
    assert!(
        status_rs.contains("mod unified;"),
        "status.rs should declare the focused unified status submodule"
    );
    assert!(
        status_rs.lines().count() <= 350,
        "status.rs should stay as a focused facade after status output splitting"
    );

    let unified = fs::read_to_string(unified_path).expect("read unified status module");
    for expected in [
        "fn workbench_status_json_line",
        "struct WorkbenchStatusJsonInput",
        "fn human_model_budget_status",
    ] {
        assert!(
            unified.contains(expected),
            "status/unified.rs should own unified workbench status detail {expected}"
        );
        assert!(
            !status_rs.contains(expected),
            "status.rs should not regain unified workbench status detail {expected}"
        );
    }
    assert!(
        unified.lines().count() <= 260,
        "status/unified.rs should stay focused after extraction"
    );
}

#[test]
fn cli_workbench_session_status_keeps_output_lineage_export_and_text_split() {
    let session_rs = fs::read_to_string(
        workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/session.rs"),
    )
    .expect("read session status root");
    let session_dir = workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/session");

    for module in [
        "export",
        "history_output",
        "lineage",
        "status_output",
        "summaries",
        "text",
    ] {
        assert!(
            session_dir.join(format!("{module}.rs")).exists(),
            "status/session/{module}.rs should own a focused session status slice"
        );
        assert!(
            session_rs.contains(&format!("mod {module};")),
            "status/session.rs should declare the focused {module} submodule"
        );
    }
    assert!(
        session_rs.lines().count() <= 80,
        "status/session.rs should stay as a thin facade after session status splitting"
    );

    let export = fs::read_to_string(session_dir.join("export.rs")).expect("read session export");
    let history = fs::read_to_string(session_dir.join("history_output.rs"))
        .expect("read session history output");
    let lineage = fs::read_to_string(session_dir.join("lineage.rs")).expect("read session lineage");
    let status =
        fs::read_to_string(session_dir.join("status_output.rs")).expect("read session status");
    let summaries =
        fs::read_to_string(session_dir.join("summaries.rs")).expect("read session summaries");
    let text = fs::read_to_string(session_dir.join("text.rs")).expect("read session text");

    assert!(
        export.contains("print_session_export") && export.contains("workbench_session_export_path"),
        "session/export.rs should own session export output and target path selection"
    );
    assert!(
        history.contains("print_session_history")
            && history.contains("session_history_human_lines")
            && history.contains("session_replay_history_records"),
        "session/history_output.rs should own session history terminal output and replay lookup"
    );
    assert!(
        lineage.contains("print_session_lineage_status"),
        "session/lineage.rs should own state-db lineage output"
    );
    assert!(
        status.contains("print_session_status") && status.contains("session_status_human_lines"),
        "session/status_output.rs should own current session status output"
    );
    assert!(
        summaries.contains("print_session_summaries")
            && summaries.contains("session_summaries_human_lines")
            && summaries.contains("session_replay_history_summaries"),
        "session/summaries.rs should own session summary listing"
    );
    assert!(
        text.contains("truncate_session_text"),
        "session/text.rs should own shared session text truncation"
    );

    for forbidden in [
        "fn print_session_export",
        "fn print_session_history",
        "fn print_session_lineage_status",
        "fn print_session_status",
        "fn print_session_summaries",
        "fn truncate_session_text",
    ] {
        assert!(
            !session_rs.contains(forbidden),
            "status/session.rs should stay a facade; move {forbidden} into status/session/* modules"
        );
    }
}

#[test]
fn cli_workbench_approval_status_keeps_cells_json_output_and_value_split() {
    let approval_rs = fs::read_to_string(
        workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/approval.rs"),
    )
    .expect("read approval status root");
    let approval_dir =
        workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/approval");

    for module in ["cells", "json", "output", "value"] {
        assert!(
            approval_dir.join(format!("{module}.rs")).exists(),
            "approval/{module}.rs should own a focused approval status slice"
        );
        assert!(
            approval_rs.contains(&format!("mod {module};")),
            "status/approval.rs should declare the focused {module} submodule"
        );
    }
    assert!(
        approval_rs.lines().count() <= 80,
        "status/approval.rs should stay as a thin facade after approval splitting"
    );

    let cells = fs::read_to_string(approval_dir.join("cells.rs")).expect("read approval cells");
    let json = fs::read_to_string(approval_dir.join("json.rs")).expect("read approval json");
    let output = fs::read_to_string(approval_dir.join("output.rs")).expect("read approval output");
    let value = fs::read_to_string(approval_dir.join("value.rs")).expect("read approval value");
    assert!(
        cells.contains("screen_approval_cells")
            && cells.contains("screen_approval_summary_cell")
            && cells.contains("approval_queue_summary"),
        "approval/cells.rs should own approval screen-cell projection and queue summary"
    );
    assert!(
        json.contains("approval_overlay_json_line") && json.contains("approval_overlay_item_json"),
        "approval/json.rs should own approval overlay JSON output"
    );
    assert!(
        output.contains("print_approval_status")
            && output.contains("print_approval_overlay")
            && output.contains("print_approval_shell_commands"),
        "approval/output.rs should own approval human/machine terminal output"
    );
    assert!(
        value.contains("approval_scope")
            && value.contains("approval_operations")
            && value.contains("approval_risk_level")
            && value.contains("approval_value"),
        "approval/value.rs should own approval context extraction and risk helpers"
    );
    for forbidden in [
        "fn screen_approval_cells",
        "fn approval_overlay_json_line",
        "fn print_approval_overlay",
        "fn approval_scope",
        "fn approval_value",
    ] {
        assert!(
            !approval_rs.contains(forbidden),
            "status/approval.rs should stay a thin facade; move {forbidden} into approval/* modules"
        );
    }
}

#[test]
fn cli_workbench_memory_status_keeps_cells_output_projection_and_value_split() {
    let memory_rs = fs::read_to_string(
        workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/memory.rs"),
    )
    .expect("read memory status root");
    let memory_dir = workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/memory");

    for module in ["cells", "output", "projection", "value"] {
        assert!(
            memory_dir.join(format!("{module}.rs")).exists(),
            "status/memory/{module}.rs should own a focused memory status slice"
        );
        assert!(
            memory_rs.contains(&format!("mod {module};")),
            "status/memory.rs should declare the focused {module} submodule"
        );
    }
    assert!(
        memory_rs.lines().count() <= 80,
        "status/memory.rs should stay as a thin facade after memory status splitting"
    );

    let cells = fs::read_to_string(memory_dir.join("cells.rs")).expect("read memory cells");
    let output = fs::read_to_string(memory_dir.join("output.rs")).expect("read memory output");
    let projection =
        fs::read_to_string(memory_dir.join("projection.rs")).expect("read memory projection");
    let value = fs::read_to_string(memory_dir.join("value.rs")).expect("read memory value");
    assert!(
        cells.contains("screen_memory_cell")
            && cells.contains("print_memory_candidate_cells")
            && cells.contains("print_memory_journal_cells"),
        "memory/cells.rs should own workbench memory cell projection"
    );
    assert!(
        output.contains("print_memory_status")
            && output.contains("memory_status_human_lines")
            && output.contains("memory_status_json_line"),
        "memory/output.rs should own memory status terminal output"
    );
    assert!(
        projection.contains("MemoryProjectionExplain")
            && projection.contains("memory_projection_explain")
            && projection.contains("superseded_memory_records"),
        "memory/projection.rs should own memory projection explanation and supersession reads"
    );
    assert!(
        value.contains("memory_projection_file_count")
            && value.contains("pending_memory_candidate_count")
            && value.contains("memory_terminal_snippet"),
        "memory/value.rs should own small memory status value helpers"
    );
    for forbidden in [
        "fn memory_status_json_line",
        "fn memory_projection_explain",
        "fn print_memory_candidate_cells",
        "fn memory_terminal_snippet",
    ] {
        assert!(
            !memory_rs.contains(forbidden),
            "status/memory.rs should stay a facade; move {forbidden} into status/memory/* modules"
        );
    }
}

#[test]
fn cli_workbench_tools_status_keeps_cells_output_registry_and_value_split() {
    let tools_rs = fs::read_to_string(
        workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/tools.rs"),
    )
    .expect("read tools status root");
    let tools_dir = workspace_root().join("crates/ikaros-cli/src/chat/workbench/status/tools");

    for module in ["cells", "output", "registry", "value"] {
        assert!(
            tools_dir.join(format!("{module}.rs")).exists(),
            "status/tools/{module}.rs should own a focused tools status slice"
        );
        assert!(
            tools_rs.contains(&format!("mod {module};")),
            "status/tools.rs should declare the focused {module} submodule"
        );
    }
    assert!(
        tools_rs.lines().count() <= 80,
        "status/tools.rs should stay as a thin facade after tools status splitting"
    );

    let cells = fs::read_to_string(tools_dir.join("cells.rs")).expect("read tools cells");
    let output = fs::read_to_string(tools_dir.join("output.rs")).expect("read tools output");
    let registry = fs::read_to_string(tools_dir.join("registry.rs")).expect("read tools registry");
    let value = fs::read_to_string(tools_dir.join("value.rs")).expect("read tools value");
    assert!(
        cells.contains("screen_rag_cell")
            && cells.contains("screen_mcp_cell")
            && cells.contains("screen_browser_cell"),
        "tools/cells.rs should own tool and model screen-cell projection"
    );
    assert!(
        output.contains("print_tools_status")
            && output.contains("tools_status_human_lines")
            && output.contains("print_mcp_status"),
        "tools/output.rs should own RAG/tools/MCP terminal output"
    );
    assert!(
        registry.contains("tool_descriptor_status_json")
            && registry.contains("tool_descriptor_status_line")
            && registry.contains("tool_descriptor_short_line"),
        "tools/registry.rs should own skill descriptor formatting"
    );
    assert!(
        value.contains("rag_status_json_line")
            && value.contains("mcp_status_json")
            && value.contains("tools_status_json_line"),
        "tools/value.rs should own tools status JSON/value helpers"
    );
    for forbidden in [
        "fn print_tools_status",
        "fn screen_rag_cell",
        "fn tool_descriptor_status_json",
        "fn tools_status_json_line",
    ] {
        assert!(
            !tools_rs.contains(forbidden),
            "status/tools.rs should stay a facade; move {forbidden} into status/tools/* modules"
        );
    }
}

#[test]
fn cli_browser_command_keeps_supervisor_cdp_and_workbench_split() {
    let browser_rs = fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/browser.rs"))
        .expect("read browser command root");
    let browser_dir = workspace_root().join("crates/ikaros-cli/src/browser");

    for module in ["cdp", "http", "supervisor", "workbench"] {
        assert!(
            browser_dir.join(format!("{module}.rs")).exists(),
            "browser/{module}.rs should own a focused slice of browser command behavior"
        );
        assert!(
            browser_rs.contains(&format!("mod {module};")),
            "browser.rs should declare the focused {module} submodule"
        );
    }
    assert!(
        browser_rs.lines().count() <= 350,
        "browser.rs should stay as a thin clap/dispatch root after browser command splitting"
    );

    let supervisor =
        fs::read_to_string(browser_dir.join("supervisor.rs")).expect("read browser supervisor");
    let cdp = fs::read_to_string(browser_dir.join("cdp.rs")).expect("read browser cdp");
    let workbench =
        fs::read_to_string(browser_dir.join("workbench.rs")).expect("read browser workbench");

    assert!(
        supervisor.contains("launch_browser_supervisor")
            && supervisor.contains("stop_browser_supervisor"),
        "browser/supervisor.rs should own browser process lifecycle"
    );
    assert!(
        cdp.contains("browser_cdp_action_json") && cdp.contains("send_cdp_commands"),
        "browser/cdp.rs should own websocket CDP action dispatch"
    );
    assert!(
        workbench.contains("run_browser_workbench_command")
            && workbench.contains("parse_browser_workbench_request"),
        "browser/workbench.rs should own slash workbench parsing and output"
    );

    for forbidden in [
        "fn launch_browser_supervisor",
        "async fn send_cdp_commands",
        "fn parse_browser_workbench_request",
        "fn validate_browser_target_url",
    ] {
        assert!(
            !browser_rs.contains(forbidden),
            "browser.rs should stay a thin dispatch root; move {forbidden} into browser/* modules"
        );
    }
}

#[test]
fn cli_code_command_keeps_workflow_review_rollback_output_and_parse_split() {
    let code_rs = fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/code.rs"))
        .expect("read code command root");
    let code_dir = workspace_root().join("crates/ikaros-cli/src/code");

    for module in [
        "interactive_parse",
        "output",
        "review",
        "rollback",
        "workflow",
    ] {
        assert!(
            code_dir.join(format!("{module}.rs")).exists(),
            "code/{module}.rs should own a focused slice of code command behavior"
        );
        assert!(
            code_rs.contains(&format!("mod {module};")),
            "code.rs should declare the focused {module} submodule"
        );
    }
    assert!(
        code_dir.join("tests.rs").exists(),
        "code command behavior tests should live in code/tests.rs, not inline in code.rs"
    );
    assert!(
        code_rs.contains("mod tests;") && !code_rs.contains("mod tests {"),
        "code.rs should keep only a narrow cfg(test) tests module declaration"
    );
    assert!(
        code_rs.lines().count() <= 450,
        "code.rs should stay as a thin clap/dispatch root after code command splitting"
    );

    let interactive_parse =
        fs::read_to_string(code_dir.join("interactive_parse.rs")).expect("read code parser");
    let output = fs::read_to_string(code_dir.join("output.rs")).expect("read code output");
    let review = fs::read_to_string(code_dir.join("review.rs")).expect("read code review");
    let rollback = fs::read_to_string(code_dir.join("rollback.rs")).expect("read code rollback");
    let workflow = fs::read_to_string(code_dir.join("workflow.rs")).expect("read code workflow");

    assert!(
        interactive_parse.contains("parse_interactive_code_command")
            && interactive_parse.contains("split_interactive_code_line"),
        "code/interactive_parse.rs should own /code command-line parsing"
    );
    assert!(
        output.contains("print_code_terminal_summary") && output.contains("print_coding_progress"),
        "code/output.rs should own CLI terminal summaries for code results"
    );
    assert!(
        review.contains("append_model_code_review_notes")
            && review.contains("build_model_code_review_prompt"),
        "code/review.rs should own model-assisted code review note generation"
    );
    assert!(
        rollback.contains("rollback_diff_for_coding_turn")
            && rollback.contains("reverse_unified_diff"),
        "code/rollback.rs should own coding rollback diff projection"
    );
    assert!(
        workflow.contains("run_coding_workflow_command")
            && workflow.contains("coding_session_and_registry_for_workflow_with_cancellation"),
        "code/workflow.rs should own workflow execution-session setup and dispatch"
    );

    for forbidden in [
        "fn split_interactive_code_line",
        "fn print_coding_progress",
        "fn append_model_code_review_notes",
        "fn reverse_unified_diff",
        "fn run_coding_workflow_command",
    ] {
        assert!(
            !code_rs.contains(forbidden),
            "code.rs should stay a thin dispatch root; move {forbidden} into code/* modules"
        );
    }
}

#[test]
fn cli_chat_live_keeps_sink_stream_activity_and_cells_split() {
    let live_rs = fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/chat/live.rs"))
        .expect("read chat live root");
    let live_dir = workspace_root().join("crates/ikaros-cli/src/chat/live");

    for module in ["activity", "cells", "sink"] {
        assert!(
            live_dir.join(format!("{module}.rs")).exists(),
            "chat/live/{module}.rs should own a focused slice of live chat behavior"
        );
        assert!(
            live_rs.contains(&format!("mod {module};")),
            "chat/live.rs should declare the focused {module} submodule"
        );
    }
    assert!(
        live_rs.lines().count() <= 80,
        "chat/live.rs should stay a thin module root after live chat splitting"
    );

    let sink = fs::read_to_string(live_dir.join("sink.rs")).expect("read live sink");
    let activity = fs::read_to_string(live_dir.join("activity.rs")).expect("read live activity");
    let cells = fs::read_to_string(live_dir.join("cells.rs")).expect("read live cells");
    let terminal_lib =
        fs::read_to_string(workspace_root().join("crates/ikaros-terminal/src/lib.rs"))
            .expect("read ikaros-terminal lib");
    let terminal_stream =
        fs::read_to_string(workspace_root().join("crates/ikaros-terminal/src/stream.rs"))
            .expect("read terminal stream");
    let terminal_activity =
        fs::read_to_string(workspace_root().join("crates/ikaros-terminal/src/activity.rs"))
            .expect("read terminal activity");

    assert!(
        sink.contains("struct WorkbenchLiveEventSink")
            && sink.contains("emit_interactive_chat_turn_failure_evidence"),
        "chat/live/sink.rs should own live event sink and failure evidence emission"
    );
    assert!(
        !live_dir.join("stream.rs").exists(),
        "pure assistant stream rendering state should live in ikaros-terminal, not chat/live/stream.rs"
    );
    assert!(
        !live_rs.contains("mod stream"),
        "chat/live.rs should import assistant stream rendering primitives directly from ikaros-terminal instead of keeping a private stream shim"
    );
    assert!(
        terminal_stream.contains("struct AgentTextDeltaState")
            && terminal_stream.contains("format_agent_text_delta"),
        "ikaros-terminal stream module should own assistant stream rendering state"
    );
    assert!(
        terminal_lib.contains("AgentTextDeltaState")
            && terminal_lib.contains("format_agent_text_delta")
            && terminal_lib.contains("finish_agent_text_delta"),
        "ikaros-terminal should export assistant stream rendering primitives"
    );
    assert!(
        activity.contains("human_activity_lines")
            && activity.contains("human_activity_lines_with_debug_context"),
        "chat/live/activity.rs should own AgentEvent to human activity text projection"
    );
    assert!(
        activity.contains("ikaros_terminal::human_activity_line_for_terminal")
            && !activity.contains("fn human_activity_line_for_terminal"),
        "chat/live/activity.rs should re-export terminal activity formatting instead of owning it"
    );
    assert!(
        terminal_activity.contains("fn human_activity_line_for_terminal")
            && terminal_activity.contains("fn tool_activity_title"),
        "ikaros-terminal should own human activity terminal formatting"
    );
    assert!(
        cells.contains("compact_live_event_cells_with_debug")
            && cells.contains("live_cells_json_line"),
        "chat/live/cells.rs should own live event cell projection and JSON lines"
    );

    for forbidden in [
        "struct WorkbenchLiveEventSink",
        "struct AgentTextDeltaState",
        "fn human_activity_lines_with_debug_context",
        "fn compact_live_event_cells_with_debug",
    ] {
        assert!(
            !live_rs.contains(forbidden),
            "chat/live.rs should stay a thin module root; move {forbidden} into chat/live/* modules"
        );
    }
}

#[test]
fn cli_chat_root_keeps_args_command_session_and_single_message_split() {
    let chat_root = fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/chat/mod.rs"))
        .expect("read chat root");
    let chat_dir = workspace_root().join("crates/ikaros-cli/src/chat");

    for module in ["args", "command", "session_id", "single"] {
        assert!(
            chat_dir.join(format!("{module}.rs")).exists(),
            "chat/{module}.rs should own a focused slice of chat command behavior"
        );
        assert!(
            chat_root.contains(&format!("mod {module};")),
            "chat/mod.rs should declare the focused {module} submodule"
        );
    }
    assert!(
        chat_root.lines().count() <= 80,
        "chat/mod.rs should stay a thin facade root after chat command splitting"
    );

    let args = fs::read_to_string(chat_dir.join("args.rs")).expect("read chat args");
    let command = fs::read_to_string(chat_dir.join("command.rs")).expect("read chat command");
    let session_id =
        fs::read_to_string(chat_dir.join("session_id.rs")).expect("read chat session id");
    let single = fs::read_to_string(chat_dir.join("single.rs")).expect("read single chat command");

    assert!(
        args.contains("pub(crate) struct ChatArgs") && args.contains("impl From<&ChatArgs>"),
        "chat/args.rs should own ChatArgs and ChatRunOptions conversion"
    );
    assert!(
        command.contains("pub(crate) async fn chat_command")
            && command.contains("async fn chat_command_inner")
            && command.contains("fn push_slash_command_transcript"),
        "chat/command.rs should own chat command dispatch and the interactive input loop"
    );
    assert!(
        session_id.contains("interactive_chat_session_id")
            && session_id.contains("recent_interactive_chat_session_id"),
        "chat/session_id.rs should own interactive chat session selection"
    );
    assert!(
        single.contains("pub(crate) async fn run_single_chat_message")
            && single.contains("run_chat_message_with_context"),
        "chat/single.rs should own one-shot chat execution"
    );

    for forbidden in [
        "struct ChatArgs",
        "async fn chat_command_inner",
        "async fn run_single_chat_message",
        "fn push_slash_command_transcript",
        "fn interactive_chat_session_id",
        "impl From<&ChatArgs>",
    ] {
        assert!(
            !chat_root.contains(forbidden),
            "chat/mod.rs should stay a thin facade root; move {forbidden} into chat/* modules"
        );
    }
}

#[test]
fn cli_chat_workbench_keeps_history_cells_and_tests_split() {
    let workbench_rs =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/chat/workbench.rs"))
            .expect("read chat workbench root");
    let workbench_dir = workspace_root().join("crates/ikaros-cli/src/chat/workbench");

    for module in ["history", "cells"] {
        assert!(
            workbench_dir.join(format!("{module}.rs")).exists(),
            "chat/workbench/{module}.rs should own a focused slice of workbench behavior"
        );
        assert!(
            workbench_rs.contains(&format!("mod {module};")),
            "chat/workbench.rs should declare the focused {module} submodule"
        );
    }
    assert!(
        workbench_dir.join("tests.rs").exists(),
        "large workbench behavior tests should live in chat/workbench/tests.rs, not inline in workbench.rs"
    );
    assert!(
        workbench_rs.contains("mod tests;") && !workbench_rs.contains("mod tests {"),
        "chat/workbench.rs should keep only a narrow cfg(test) tests module declaration"
    );
    assert!(
        workbench_rs.lines().count() <= 120,
        "chat/workbench.rs should stay as a thin facade root after workbench splitting"
    );

    let history =
        fs::read_to_string(workbench_dir.join("history.rs")).expect("read chat workbench history");
    let cells =
        fs::read_to_string(workbench_dir.join("cells.rs")).expect("read chat workbench cells");

    assert!(
        history.contains("append_workbench_history")
            && history.contains("load_workbench_history_entries")
            && history.contains("normalize_session_id"),
        "chat/workbench/history.rs should own input history persistence and session id normalization"
    );
    assert!(
        cells.contains("agent_event_cell")
            && cells.contains("session_entry_cell")
            && cells.contains("coding_event_cells")
            && cells.contains("model_stream_event_detail"),
        "chat/workbench/cells.rs should own session and event cell projection"
    );
    assert!(
        history.lines().count() <= 180,
        "chat/workbench/history.rs should stay focused after the split"
    );
    assert!(
        cells.lines().count() <= 650,
        "chat/workbench/cells.rs should stay below the large-file threshold after the split"
    );

    for forbidden in [
        "fn append_workbench_history",
        "fn load_workbench_history_entries",
        "fn agent_event_cell",
        "fn session_entry_cell",
        "fn model_stream_event_detail",
        "fn coding_event_cells",
    ] {
        assert!(
            !workbench_rs.contains(forbidden),
            "chat/workbench.rs should stay a thin facade root; move {forbidden} into chat/workbench/* modules"
        );
    }
}

#[test]
fn cli_chat_terminal_imports_terminal_input_primitives_directly() {
    assert!(
        !workspace_root()
            .join("crates/ikaros-cli/src/chat/workbench/input.rs")
            .exists(),
        "chat workbench should not keep a pure input re-export shim; input primitives belong to ikaros-terminal"
    );

    assert!(
        !workspace_root()
            .join("crates/ikaros-cli/src/chat/terminal.rs")
            .exists(),
        "CLI chat terminal module should be deleted after terminal UI ownership moves to ikaros-terminal"
    );
    let terminal_lib =
        fs::read_to_string(workspace_root().join("crates/ikaros-terminal/src/lib.rs"))
            .expect("read ikaros-terminal lib");
    assert!(
        terminal_lib.contains("handle_workbench_input_control"),
        "ikaros-terminal should own and export workbench input control handling"
    );
    for expected in [
        "BRACKETED_PASTE_START",
        "MULTILINE_TERMINATOR",
        "read_bracketed_paste_message",
        "read_multiline_message",
        "fullscreen_terminal_event_input_available",
        "WorkbenchTerminalInputSessionGuard",
        "FullscreenRawModeGuard",
        "SetScrollRegion",
        "ResetScrollRegion",
        "RunningTurnInputCapture",
        "RunningTurnTerminal",
        "normalize_raw_terminal_newlines",
        "WorkbenchLineEditorRenderState",
        "WorkbenchLineInputUi",
        "read_workbench_terminal_line_input",
        "print_inline_history_lines",
        "print_inline_history_text",
        "print_inline_turn_separator",
        "print_inline_turn_worked_separator",
        "clear_visible_terminal",
    ] {
        assert!(
            terminal_lib.contains(expected),
            "ikaros-terminal should own and export terminal input reader primitive {expected}"
        );
    }
}

#[test]
fn terminal_input_spine_keeps_workbench_and_line_input_split() {
    let terminal_src = workspace_root().join("crates/ikaros-terminal/src");
    let workbench_input =
        fs::read_to_string(terminal_src.join("workbench_input.rs")).expect("read workbench input");
    let workbench_dir = terminal_src.join("workbench_input");
    for module in ["control", "events", "state", "tests"] {
        assert!(
            workbench_dir.join(format!("{module}.rs")).exists(),
            "workbench_input/{module}.rs should own a focused terminal input slice"
        );
        assert!(
            workbench_input.contains(&format!("mod {module};")),
            "workbench_input.rs should declare focused {module} submodule"
        );
    }
    assert!(
        workbench_input.lines().count() <= 80,
        "workbench_input.rs should stay as a thin facade after terminal input splitting"
    );

    let workbench_events =
        fs::read_to_string(workbench_dir.join("events.rs")).expect("read workbench input events");
    let workbench_state =
        fs::read_to_string(workbench_dir.join("state.rs")).expect("read workbench input state");
    let workbench_control =
        fs::read_to_string(workbench_dir.join("control.rs")).expect("read workbench input control");
    assert!(
        workbench_events.contains("parse_workbench_input_event")
            && workbench_events.contains("apply_workbench_terminal_input_event"),
        "workbench_input/events.rs should own input event parsing and reducer dispatch"
    );
    assert!(
        workbench_state.contains("pub struct WorkbenchInputState")
            && workbench_state.contains("format_workbench_input_state"),
        "workbench_input/state.rs should own editor state and debug formatting"
    );
    assert!(
        workbench_control.contains("handle_workbench_input_control"),
        "workbench_input/control.rs should own high-level terminal input control handling"
    );

    let line_input =
        fs::read_to_string(terminal_src.join("line_input.rs")).expect("read line input facade");
    let line_dir = terminal_src.join("line_input");
    for module in [
        "display",
        "history",
        "inline_composer",
        "intro",
        "popup",
        "read",
        "render_state",
        "submitted",
        "ui",
    ] {
        assert!(
            line_dir.join(format!("{module}.rs")).exists(),
            "line_input/{module}.rs should own a focused line-input slice"
        );
        assert!(
            line_input.contains(&format!("mod {module};")),
            "line_input.rs should declare focused {module} submodule"
        );
    }
    assert!(
        !line_dir.join("tests.rs").exists(),
        "line_input should not keep an empty tests.rs placeholder after split cleanup"
    );
    assert!(
        line_input.lines().count() <= 80,
        "line_input.rs should stay as a thin facade after terminal line-input splitting"
    );

    let line_read = fs::read_to_string(line_dir.join("read.rs")).expect("read line input read");
    let line_composer = fs::read_to_string(line_dir.join("inline_composer.rs"))
        .expect("read line input inline composer");
    let line_history =
        fs::read_to_string(line_dir.join("history.rs")).expect("read line input history");
    let line_submitted =
        fs::read_to_string(line_dir.join("submitted.rs")).expect("read line input submitted");
    assert!(
        line_read.contains("read_workbench_terminal_line_input"),
        "line_input/read.rs should own the terminal line read loop"
    );
    assert!(
        line_composer.contains("render_workbench_inline_composer")
            && line_composer.contains("inline_composer_lines"),
        "line_input/inline_composer.rs should own inline composer rendering"
    );
    assert!(
        line_history.contains("print_inline_history_lines")
            && line_history.contains("clear_visible_terminal"),
        "line_input/history.rs should own inline history output"
    );
    assert!(
        line_submitted.contains("submitted_user_message_cell_lines"),
        "line_input/submitted.rs should own submitted-user history cells"
    );
}

#[test]
fn cli_chat_workbench_does_not_reexport_terminal_sanitize_helpers() {
    assert_source_tree_excludes(
        "crates/ikaros-cli/src/chat",
        &["workbench::terminal_inline", "workbench::terminal_message"],
        "chat modules should import terminal sanitize helpers directly from ikaros-terminal instead of routing them through chat::workbench",
    );

    let workbench_rs =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/chat/workbench.rs"))
            .expect("read chat workbench module");
    let mut in_terminal_reexport = false;
    for line in workbench_rs.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("pub(super) use ikaros_terminal::{") {
            in_terminal_reexport = true;
        }
        if in_terminal_reexport {
            for forbidden in ["terminal_inline", "terminal_message"] {
                assert!(
                    !trimmed.contains(forbidden),
                    "chat::workbench should not re-export {forbidden}; import it directly from ikaros-terminal"
                );
            }
        }
        if in_terminal_reexport && trimmed.ends_with("};") {
            in_terminal_reexport = false;
        }
    }
}

#[test]
fn cli_does_not_keep_terminal_markdown_rendering_helpers() {
    let output_rs =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/chat/output.rs"))
            .expect("read chat output module");
    for forbidden in [
        "TerminalMarkdownRenderer",
        "pub(crate) fn render_terminal_markdown",
        "pub(crate) fn render_assistant_markdown_transcript",
        "pub(crate) fn color_assistant_bullet_for_terminal",
        "fn prefix_assistant_transcript_lines",
        "fn markdown_render_width",
    ] {
        assert!(
            !output_rs.contains(forbidden),
            "terminal markdown rendering belongs in ikaros-terminal, not CLI output: found {forbidden}"
        );
    }
    let terminal_lib =
        fs::read_to_string(workspace_root().join("crates/ikaros-terminal/src/lib.rs"))
            .expect("read ikaros-terminal lib");
    for expected in [
        "render_terminal_markdown_for_current_width",
        "render_assistant_markdown_transcript_for_current_width",
        "color_assistant_bullet_for_terminal",
    ] {
        assert!(
            terminal_lib.contains(expected),
            "ikaros-terminal should export terminal markdown helper {expected}"
        );
    }
}

#[test]
fn cli_workbench_does_not_keep_terminal_input_behavior_tests() {
    let workbench_rs =
        fs::read_to_string(workspace_root().join("crates/ikaros-cli/src/chat/workbench.rs"))
            .expect("read chat workbench module");
    for forbidden in [
        "WorkbenchInputAction",
        "WorkbenchInputEvent",
        "WorkbenchInputState",
        "format_workbench_input_state",
        "parse_workbench_input_event",
        "workbench_input_state_",
        "workbench_input_event_adapter_",
    ] {
        assert!(
            !workbench_rs.contains(forbidden),
            "terminal input behavior tests belong in ikaros-terminal, not CLI workbench: found {forbidden}"
        );
    }
}

#[test]
fn cli_uses_terminal_facade_without_tui_namespace_alias() {
    assert!(
        !workspace_root()
            .join("crates/ikaros-cli/src/chat/tui.rs")
            .exists(),
        "chat/tui.rs should not remain as a pure ikaros-terminal re-export shim"
    );
    assert_source_tree_excludes(
        "crates/ikaros-cli/src",
        &["ikaros_terminal::tui"],
        "ikaros-cli should use ikaros-terminal exports directly instead of the old tui namespace alias",
    );
    assert_source_tree_excludes(
        "crates/ikaros-terminal/src",
        &["pub mod tui"],
        "ikaros-terminal should not keep a crate-wide tui compatibility namespace after owning the TUI implementation",
    );
}

#[test]
fn surfaces_facade_exposes_named_surface_modules_without_root_globs() {
    let surfaces_lib =
        fs::read_to_string(workspace_root().join("crates/ikaros-surfaces/src/lib.rs"))
            .expect("read ikaros-surfaces lib");
    for expected in [
        "pub mod api",
        "pub mod body",
        "pub mod gateway",
        "pub mod mcp",
        "pub mod service",
    ] {
        assert!(
            surfaces_lib.contains(expected),
            "ikaros-surfaces should expose named surface module {expected}"
        );
    }
    for forbidden in [
        "pub use api::*",
        "pub use body::*",
        "pub use gateway::*",
        "pub use mcp::*",
        "pub use service::*",
        "allow(ambiguous_glob_reexports",
    ] {
        assert!(
            !surfaces_lib.contains(forbidden),
            "ikaros-surfaces should avoid root-level compatibility glob exports: found {forbidden}"
        );
    }
}

#[test]
fn terminal_owns_tui_implementation_instead_of_legacy_tui_crate() {
    let metadata = workspace_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array");
    let terminal = packages
        .iter()
        .find(|package| package["name"].as_str() == Some("ikaros-terminal"))
        .expect("ikaros-terminal package should exist");
    let deps = package_dependency_names(terminal);

    assert!(
        !deps.contains("ikaros-tui"),
        "ikaros-terminal should own terminal implementation directly instead of depending on the legacy ikaros-tui crate"
    );

    let legacy_tui_lib = workspace_root().join("crates/ikaros-tui/src/lib.rs");
    if legacy_tui_lib.exists() {
        let tui_lib = fs::read_to_string(&legacy_tui_lib).expect("read ikaros-tui lib");
        assert!(
            tui_lib.contains("pub use ikaros_terminal::*"),
            "ikaros-tui should remain only as a compatibility shim over ikaros-terminal"
        );
    }
}

fn workspace_metadata() -> Value {
    let output = Command::new(cargo_bin())
        .arg("metadata")
        .arg("--no-deps")
        .arg("--format-version")
        .arg("1")
        .current_dir(workspace_root())
        .output()
        .expect("run cargo metadata");

    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    serde_json::from_slice(&output.stdout).expect("parse cargo metadata")
}

fn assert_source_tree_excludes(relative_dir: &str, forbidden: &[&str], message: &str) {
    let root = workspace_root().join(relative_dir);
    let mut stack = vec![root];
    let mut violations = Vec::new();

    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap_or_else(|error| {
            panic!(
                "read source directory {}: {error}",
                dir.strip_prefix(workspace_root()).unwrap_or(&dir).display()
            )
        }) {
            let entry = entry.expect("read source entry");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }
            let source = fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!(
                    "read source file {}: {error}",
                    path.strip_prefix(workspace_root())
                        .unwrap_or(&path)
                        .display()
                )
            });
            for needle in forbidden {
                if source.contains(needle) {
                    violations.push(format!(
                        "{} contains {needle}",
                        path.strip_prefix(workspace_root())
                            .unwrap_or(&path)
                            .display()
                    ));
                }
            }
        }
    }

    assert!(violations.is_empty(), "{message}: {violations:?}");
}

fn assert_no_state_gateway_shape_imports(relative_dir: &str) {
    let stable_shapes = [
        "GatewayDelivery",
        "GatewayDeliveryStatus",
        "GatewayMessage",
        "GatewayMessageKind",
        "GatewayMessageStatus",
        "GatewayPairing",
        "GatewayPairingStatus",
        "GatewayRoute",
        "GatewaySessionSource",
    ];
    let root = workspace_root().join(relative_dir);
    let mut stack = vec![root];
    let mut violations = Vec::new();

    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap_or_else(|error| {
            panic!(
                "read source directory {}: {error}",
                dir.strip_prefix(workspace_root()).unwrap_or(&dir).display()
            )
        }) {
            let entry = entry.expect("read source entry");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.file_name().and_then(|name| name.to_str()) == Some("architecture_guard.rs") {
                continue;
            }
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }
            let source = fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!(
                    "read source file {}: {error}",
                    path.strip_prefix(workspace_root())
                        .unwrap_or(&path)
                        .display()
                )
            });

            for shape in stable_shapes {
                let direct = format!("ikaros_state::gateway::{shape}");
                if contains_direct_gateway_shape_path(&source, &direct) {
                    violations.push(format!(
                        "{} imports {shape} directly from ikaros_state::gateway",
                        path.strip_prefix(workspace_root())
                            .unwrap_or(&path)
                            .display()
                    ));
                }
            }

            let mut remaining = source.as_str();
            while let Some(start) = remaining.find("use ikaros_state::gateway::{") {
                let after_start = &remaining[start + "use ikaros_state::gateway::{".len()..];
                let Some(end) = after_start.find("};") else {
                    break;
                };
                let import_block = &after_start[..end];
                let tokens = import_block
                    .split(|ch: char| ch != '_' && !ch.is_ascii_alphanumeric())
                    .collect::<BTreeSet<_>>();
                for shape in stable_shapes {
                    if tokens.contains(shape) {
                        violations.push(format!(
                            "{} imports {shape} from ikaros_state::gateway; use ikaros_protocol instead",
                            path.strip_prefix(workspace_root())
                                .unwrap_or(&path)
                                .display()
                        ));
                    }
                }
                remaining = &after_start[end + 2..];
            }
        }
    }

    assert!(
        violations.is_empty(),
        "{relative_dir} should import stable gateway shapes from ikaros_protocol, not ikaros_state::gateway: {violations:?}"
    );
}

fn contains_direct_gateway_shape_path(source: &str, direct: &str) -> bool {
    let mut remaining = source;
    while let Some(index) = remaining.find(direct) {
        let after = remaining[index + direct.len()..].chars().next();
        if !after.is_some_and(|ch| ch == '_' || ch.is_ascii_alphanumeric()) {
            return true;
        }
        remaining = &remaining[index + direct.len()..];
    }
    false
}

fn contains_rust_declaration_prefix(source: &str, declaration_prefix: &str) -> bool {
    source.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed == declaration_prefix
            || trimmed.starts_with(&format!("{declaration_prefix} "))
            || trimmed.starts_with(&format!("{declaration_prefix}("))
    })
}

fn package_names(metadata: &Value) -> BTreeSet<&str> {
    metadata["packages"]
        .as_array()
        .expect("metadata packages should be an array")
        .iter()
        .map(|package| package["name"].as_str().expect("package name"))
        .collect()
}

fn package_dependency_names(package: &Value) -> BTreeSet<&str> {
    package["dependencies"]
        .as_array()
        .expect("dependencies should be an array")
        .iter()
        .filter_map(|dependency| {
            dependency["package"]
                .as_str()
                .or_else(|| dependency["name"].as_str())
        })
        .collect()
}

fn cargo_bin() -> &'static str {
    option_env!("CARGO").unwrap_or("cargo")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate dir should have a parent")
        .parent()
        .expect("crates dir should have a parent")
        .to_path_buf()
}

fn source_file_and_tree_contents(relative_file: &str, relative_dir: &str) -> String {
    let mut contents = fs::read_to_string(workspace_root().join(relative_file))
        .unwrap_or_else(|error| panic!("read {relative_file}: {error}"));
    let root = workspace_root().join(relative_dir);
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap_or_else(|error| {
            panic!(
                "read source directory {}: {error}",
                dir.strip_prefix(workspace_root()).unwrap_or(&dir).display()
            )
        }) {
            let entry = entry.expect("read source entry");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }
            contents.push('\n');
            contents.push_str(&fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!(
                    "read source file {}: {error}",
                    path.strip_prefix(workspace_root())
                        .unwrap_or(&path)
                        .display()
                )
            }));
        }
    }
    contents
}
