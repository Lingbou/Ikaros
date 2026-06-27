// SPDX-License-Identifier: GPL-3.0-only

use crate::AgentPermission;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PolicyConfig {
    pub workspace_writes: AgentPermission,
    pub network: AgentPermission,
    pub audit_redaction: bool,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            workspace_writes: AgentPermission::Ask,
            network: AgentPermission::Ask,
            audit_redaction: true,
        }
    }
}
