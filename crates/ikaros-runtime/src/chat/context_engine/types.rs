// SPDX-License-Identifier: GPL-3.0-only

use super::ChatRunOptions;
use ikaros_core::ResolvedAgentProfile;
use ikaros_harness::{ExecutionSession, SkillRegistry};
use ikaros_models::ModelContextProfile;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextEvent {
    pub kind: String,
    pub scope: Option<String>,
    pub content: String,
}

pub struct ContextAssembleInput<'a> {
    pub input: &'a str,
    pub agent: &'a ResolvedAgentProfile,
    pub session: &'a ExecutionSession,
    pub registry: &'a SkillRegistry,
    pub options: &'a ChatRunOptions,
    pub model_context: Option<&'a ModelContextProfile>,
    pub reserved_system_tokens: u32,
}

pub struct ContextModelBudget<'a> {
    pub model_context: &'a ModelContextProfile,
    pub reserved_system_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnRecord {
    pub session_id: Option<String>,
    pub user_input: String,
    pub assistant_output: String,
}
