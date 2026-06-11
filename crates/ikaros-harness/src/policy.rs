// SPDX-License-Identifier: GPL-3.0-only

mod engine;
mod overlay;
mod path;
mod rules;
mod types;

pub use engine::DefaultPolicyEngine;
pub(crate) use path::{canonicalize_path_for_policy, resolve_under_workspace};
pub use types::{
    AgentPolicyOverlay, CapabilityToken, PolicyEngine, PolicyEvaluation, PolicyRequest,
    SandboxProfile, ScopedPermission,
};

#[cfg(test)]
mod tests;
