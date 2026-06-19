// SPDX-License-Identifier: GPL-3.0-only

use crate::PolicyDecision;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AgentConfig {
    pub default: String,
    pub profiles: BTreeMap<String, AgentProfile>,
    pub instances: BTreeMap<String, AgentInstanceConfig>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        let mut profiles = BTreeMap::new();
        profiles.insert("build".into(), AgentProfile::build());
        profiles.insert("plan".into(), AgentProfile::plan());
        profiles.insert("general".into(), AgentProfile::general());
        Self {
            default: "build".into(),
            profiles,
            instances: BTreeMap::new(),
        }
    }
}

impl AgentConfig {
    pub fn resolve(&self, requested: Option<&str>) -> Option<ResolvedAgentProfile> {
        let name = requested.unwrap_or(&self.default);
        self.profiles
            .get(name)
            .cloned()
            .map(|profile| ResolvedAgentProfile {
                name: name.into(),
                profile,
            })
            .or_else(|| {
                if requested.is_none() {
                    self.profiles
                        .get("build")
                        .cloned()
                        .map(|profile| ResolvedAgentProfile {
                            name: "build".into(),
                            profile,
                        })
                } else {
                    None
                }
            })
    }

    pub fn active(&self) -> ResolvedAgentProfile {
        self.resolve(None).unwrap_or_else(|| ResolvedAgentProfile {
            name: "build".into(),
            profile: AgentProfile::build(),
        })
    }

    pub fn resolve_instance(
        &self,
        requested: Option<&str>,
        workspace: impl AsRef<Path>,
        state_root: impl AsRef<Path>,
    ) -> Option<AgentInstance> {
        let workspace = workspace.as_ref();
        let state_root = state_root.as_ref();
        let requested_id = requested.unwrap_or(&self.default);
        if let Some(config) = self.instances.get(requested_id) {
            let profile_name = if config.profile.trim().is_empty() {
                &self.default
            } else {
                &config.profile
            };
            let profile = self.resolve(Some(profile_name))?;
            return Some(AgentInstance::from_config(
                requested_id,
                profile,
                config,
                workspace,
                state_root,
            ));
        }
        self.resolve(requested)
            .map(|profile| AgentInstance::local(profile, workspace, state_root))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AgentProfile {
    pub mode: AgentMode,
    pub description: String,
    pub persona_overlay: String,
    pub memory_context: bool,
    pub rag_context: bool,
    pub workspace_writes: AgentPermission,
    pub shell: AgentPermission,
    pub network: AgentPermission,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AgentInstanceConfig {
    pub profile: String,
    pub workspace: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub session_policy: AgentSessionPolicy,
    pub auth_scope: AgentAuthScope,
    pub route_bindings: Vec<AgentRouteBinding>,
}

impl Default for AgentInstanceConfig {
    fn default() -> Self {
        Self {
            profile: "build".into(),
            workspace: None,
            state_dir: None,
            session_policy: AgentSessionPolicy::default(),
            auth_scope: AgentAuthScope::default(),
            route_bindings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentInstance {
    pub agent_id: String,
    pub profile_name: String,
    pub profile: AgentProfile,
    pub workspace: PathBuf,
    pub state_dir: PathBuf,
    pub session_policy: AgentSessionPolicy,
    pub auth_scope: AgentAuthScope,
    pub route_bindings: Vec<AgentRouteBinding>,
}

impl AgentInstance {
    pub fn local(
        profile: ResolvedAgentProfile,
        workspace: impl AsRef<Path>,
        state_root: impl AsRef<Path>,
    ) -> Self {
        let agent_id = profile.name.clone();
        Self {
            state_dir: state_root.as_ref().join("agents").join(&agent_id),
            workspace: workspace.as_ref().to_path_buf(),
            profile_name: profile.name,
            profile: profile.profile,
            agent_id,
            session_policy: AgentSessionPolicy::default(),
            auth_scope: AgentAuthScope::default(),
            route_bindings: Vec::new(),
        }
    }

    pub fn from_config(
        agent_id: impl Into<String>,
        profile: ResolvedAgentProfile,
        config: &AgentInstanceConfig,
        default_workspace: &Path,
        default_state_root: &Path,
    ) -> Self {
        let agent_id = agent_id.into();
        Self {
            workspace: config
                .workspace
                .clone()
                .unwrap_or_else(|| default_workspace.to_path_buf()),
            state_dir: config
                .state_dir
                .clone()
                .unwrap_or_else(|| default_state_root.join("agents").join(&agent_id)),
            profile_name: profile.name,
            profile: profile.profile,
            agent_id,
            session_policy: config.session_policy.clone(),
            auth_scope: config.auth_scope.clone(),
            route_bindings: config.route_bindings.clone(),
        }
    }

    pub fn ephemeral(
        profile: ResolvedAgentProfile,
        workspace: impl AsRef<Path>,
        state_root: impl AsRef<Path>,
    ) -> Self {
        let mut instance = Self::local(profile, workspace, state_root);
        instance.agent_id = Uuid::new_v4().to_string();
        instance.state_dir = instance.state_dir.join(&instance.agent_id);
        instance
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AgentSessionPolicy {
    pub history_scope: AgentHistoryScope,
    pub allow_session_switch: bool,
    pub max_parallel_subagents: usize,
}

impl Default for AgentSessionPolicy {
    fn default() -> Self {
        Self {
            history_scope: AgentHistoryScope::Workspace,
            allow_session_switch: true,
            max_parallel_subagents: 4,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentHistoryScope {
    Agent,
    Session,
    #[default]
    Workspace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AgentAuthScope {
    pub local_only: bool,
    pub allow_network: AgentPermission,
}

impl Default for AgentAuthScope {
    fn default() -> Self {
        Self {
            local_only: true,
            allow_network: AgentPermission::Ask,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AgentRouteBinding {
    pub channel: String,
    pub account: Option<String>,
    pub peer: Option<String>,
    pub thread: Option<String>,
}

impl AgentProfile {
    pub fn build() -> Self {
        Self {
            mode: AgentMode::Build,
            description: "Default implementation mode for ordinary local development work.".into(),
            persona_overlay:
                "Operate as the default local implementation agent. Use harnessed tools and keep writes approval-aware."
                    .into(),
            memory_context: true,
            rag_context: false,
            workspace_writes: AgentPermission::Ask,
            shell: AgentPermission::Allow,
            network: AgentPermission::Ask,
        }
    }

    pub fn plan() -> Self {
        Self {
            mode: AgentMode::Plan,
            description: "Read-only planning and code exploration mode.".into(),
            persona_overlay:
                "Operate in read-only planning mode. Prefer analysis, design notes, and explicit implementation plans; do not request file edits."
                    .into(),
            memory_context: true,
            rag_context: false,
            workspace_writes: AgentPermission::Deny,
            shell: AgentPermission::Ask,
            network: AgentPermission::Ask,
        }
    }

    pub fn general() -> Self {
        Self {
            mode: AgentMode::General,
            description: "General research mode for multi-step local questions.".into(),
            persona_overlay:
                "Operate as a general-purpose research agent. Gather local context first and keep recommendations grounded in available evidence."
                    .into(),
            memory_context: true,
            rag_context: false,
            workspace_writes: AgentPermission::Ask,
            shell: AgentPermission::Ask,
            network: AgentPermission::Ask,
        }
    }
}

impl Default for AgentProfile {
    fn default() -> Self {
        Self::build()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    #[default]
    Build,
    Plan,
    General,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentPermission {
    Allow,
    #[default]
    Ask,
    Deny,
}

impl AgentPermission {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }

    pub fn to_policy_decision(&self) -> PolicyDecision {
        match self {
            Self::Allow => PolicyDecision::Allow,
            Self::Ask => PolicyDecision::AskUser,
            Self::Deny => PolicyDecision::Deny,
        }
    }
}

impl fmt::Display for AgentPermission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AgentMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Plan => "plan",
            Self::General => "general",
        }
    }
}

impl fmt::Display for AgentMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAgentProfile {
    pub name: String,
    pub profile: AgentProfile,
}

impl ResolvedAgentProfile {
    pub fn mode(&self) -> &AgentMode {
        &self.profile.mode
    }
}
