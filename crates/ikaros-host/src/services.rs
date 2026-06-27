// SPDX-License-Identifier: GPL-3.0-only

use crate::RuntimeLocation;
use ikaros_core::{AgentInstance, IkarosConfig, ResolvedAgentProfile};
use ikaros_execution::harness::{ExecutionSession, SkillRegistry};

pub struct HostAgentContext {
    pub config: IkarosConfig,
    pub agent_instance: AgentInstance,
    pub location: RuntimeLocation,
}

pub struct HostServices {
    pub session: ExecutionSession,
    pub registry: SkillRegistry,
}

pub struct RuntimeHarness {
    pub config: IkarosConfig,
    pub agent: ResolvedAgentProfile,
    pub agent_instance: AgentInstance,
    pub location: RuntimeLocation,
    pub session: ExecutionSession,
    pub registry: SkillRegistry,
}
