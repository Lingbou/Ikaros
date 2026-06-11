// SPDX-License-Identifier: GPL-3.0-only
//! Harness, policy, approval, audit, and skill execution primitives.

mod approval;
mod audit;
mod execution_env;
mod guardrails;
mod plugin;
mod policy;
mod session;
mod skill;
mod task_runner;

pub use approval::{
    ApprovalEvent, ApprovalLog, ApprovalPolicy, ApprovalRecord, ApprovalRequest, ApprovalStatus,
};
pub use audit::{AuditEvent, AuditLog, AuditRotationPolicy};
pub use execution_env::{
    ExecutionEnv, FileSystem, LocalExecutionEnv, NetworkEgress, NetworkEgressRequest,
    NetworkEgressResponse, ProcessOutput, ProcessRequest, ProcessRunner,
};
pub use guardrails::{
    GuardrailConfig, GuardrailDecision, GuardrailObservation, GuardrailSignal, GuardrailSignalKind,
    GuardrailState,
};
pub use ikaros_core::{IkarosError, Result};
pub use plugin::{
    LoadedPluginManifest, PLUGIN_COMMAND_MAX_ARG_BYTES, PLUGIN_COMMAND_MAX_ARGS,
    PLUGIN_COMMAND_MAX_OUTPUT_BYTES, PLUGIN_COMMAND_MAX_STDIN_BYTES, PLUGIN_COMMAND_MAX_TIMEOUT_MS,
    PluginAuditMissingCommand, PluginAuditPlugin, PluginAuditReport, PluginCatalog,
    PluginCommandManifest, PluginInstallReport, PluginLoadIssue, PluginManifest, PluginMarketplace,
    PluginMarketplaceEntry, PluginMarketplaceUpdate, PluginPermissionDeclaration,
    PluginSkillManifest, PluginUninstallReport, PluginValidationReport, audit_plugins,
    install_local_plugin, set_plugin_enabled, uninstall_local_plugin, validate_plugin_file,
};
pub use policy::{
    AgentPolicyOverlay, CapabilityToken, DefaultPolicyEngine, PolicyEngine, PolicyEvaluation,
    PolicyRequest, SandboxProfile, ScopedPermission,
};
pub use session::ExecutionSession;
pub use skill::{
    Skill, SkillBundle, SkillContext, SkillDescriptor, SkillDescriptorKind, SkillOutput,
    SkillRegistry, TaskGraph, ToolRegistry,
};
pub use task_runner::{
    CancellationToken, ExecutablePlanStep, ExecutionOptions, PlanStepStatus, StepExecutionRecord,
    TaskExecutionReport,
};

#[cfg(test)]
mod task_runner_tests;
#[cfg(test)]
mod tests;
