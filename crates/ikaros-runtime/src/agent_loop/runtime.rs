// SPDX-License-Identifier: GPL-3.0-only

use super::{
    model_turn::run_agent_loop_turn,
    types::{AgentLoopInput, AgentLoopOptions, AgentLoopReport},
};
use ikaros_core::Result;
use ikaros_harness::{ExecutionSession, SkillRegistry};
use ikaros_models::ModelProvider;
use std::{future::Future, pin::Pin};

pub trait AgentRuntime: Send + Sync {
    fn run_turn<'a>(
        &'a self,
        input: AgentLoopInput,
        provider: &'a dyn ModelProvider,
        session: &'a ExecutionSession,
        registry: &'a SkillRegistry,
        options: AgentLoopOptions,
    ) -> Pin<Box<dyn Future<Output = Result<AgentLoopReport>> + Send + 'a>>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HarnessAgentRuntime;

impl AgentRuntime for HarnessAgentRuntime {
    fn run_turn<'a>(
        &'a self,
        input: AgentLoopInput,
        provider: &'a dyn ModelProvider,
        session: &'a ExecutionSession,
        registry: &'a SkillRegistry,
        options: AgentLoopOptions,
    ) -> Pin<Box<dyn Future<Output = Result<AgentLoopReport>> + Send + 'a>> {
        Box::pin(run_agent_loop_turn(
            input, provider, session, registry, options,
        ))
    }
}

pub async fn run_agent_loop(
    input: AgentLoopInput,
    provider: &dyn ModelProvider,
    session: &ExecutionSession,
    registry: &SkillRegistry,
    options: AgentLoopOptions,
) -> Result<AgentLoopReport> {
    HarnessAgentRuntime
        .run_turn(input, provider, session, registry, options)
        .await
}
