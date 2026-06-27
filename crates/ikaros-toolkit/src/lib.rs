// SPDX-License-Identifier: GPL-3.0-only
//! Tool contracts shared by harness, skill packs, runtime, and sandbox adapters.

mod audit;
mod execution;
mod skill;

pub use audit::AuditEvent;
pub use execution::{
    ExecutionEnv, FileMetadata, FileSystem, NetworkEgress, NetworkEgressRequest,
    NetworkEgressResponse, ProcessCwdScope, ProcessOutput, ProcessRequest, ProcessRunner,
};
pub use ikaros_core::{IkarosError, Result};
pub use skill::{
    PolicyRequest, PromptSkillDocument, PromptSkillSupportFile, Skill, SkillBundle, SkillContext,
    SkillDescriptor, SkillDescriptorKind, SkillOutput, SkillRegistry, SkillRuntime,
    SkillRuntimeSession, SkillSandbox, TaskGraph, ToolExecutionMode, ToolRegistry, ToolVisibility,
    Toolset, ToolsetSelection,
};
