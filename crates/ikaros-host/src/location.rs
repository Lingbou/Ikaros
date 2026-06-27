// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::AgentInstance;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLocation {
    pub agent_id: String,
    pub profile_name: String,
    pub workspace: PathBuf,
    pub state_dir: PathBuf,
    pub audit_dir: PathBuf,
}

impl RuntimeLocation {
    pub fn from_agent_instance(agent: &AgentInstance, audit_dir: impl Into<PathBuf>) -> Self {
        Self {
            agent_id: agent.agent_id.clone(),
            profile_name: agent.profile_name.clone(),
            workspace: agent.workspace.clone(),
            state_dir: agent.state_dir.clone(),
            audit_dir: audit_dir.into(),
        }
    }
}
