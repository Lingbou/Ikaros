// SPDX-License-Identifier: GPL-3.0-only

use super::{
    model_turn::run_agent_loop_turn,
    types::{
        AgentEventSink, AgentLoopInput, AgentLoopOptions, AgentLoopReport, noop_agent_event_sink,
    },
};
use ikaros_core::Result;
use ikaros_harness::{ExecutionSession, SkillRegistry};
use ikaros_models::ModelProvider;
use std::{future::Future, pin::Pin};

pub trait AgentRuntime: Send + Sync {
    fn run_turn_with_events<'a>(
        &'a self,
        input: AgentLoopInput,
        provider: &'a dyn ModelProvider,
        session: &'a ExecutionSession,
        registry: &'a SkillRegistry,
        event_sink: &'a dyn AgentEventSink,
        options: AgentLoopOptions,
    ) -> Pin<Box<dyn Future<Output = Result<AgentLoopReport>> + Send + 'a>>;

    fn run_turn<'a>(
        &'a self,
        input: AgentLoopInput,
        provider: &'a dyn ModelProvider,
        session: &'a ExecutionSession,
        registry: &'a SkillRegistry,
        options: AgentLoopOptions,
    ) -> Pin<Box<dyn Future<Output = Result<AgentLoopReport>> + Send + 'a>> {
        self.run_turn_with_events(
            input,
            provider,
            session,
            registry,
            noop_agent_event_sink(),
            options,
        )
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HarnessAgentRuntime;

impl AgentRuntime for HarnessAgentRuntime {
    fn run_turn_with_events<'a>(
        &'a self,
        input: AgentLoopInput,
        provider: &'a dyn ModelProvider,
        session: &'a ExecutionSession,
        registry: &'a SkillRegistry,
        event_sink: &'a dyn AgentEventSink,
        options: AgentLoopOptions,
    ) -> Pin<Box<dyn Future<Output = Result<AgentLoopReport>> + Send + 'a>> {
        Box::pin(run_agent_loop_turn(
            input, provider, session, registry, event_sink, options,
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

pub async fn run_agent_loop_with_events(
    input: AgentLoopInput,
    provider: &dyn ModelProvider,
    session: &ExecutionSession,
    registry: &SkillRegistry,
    event_sink: &dyn AgentEventSink,
    options: AgentLoopOptions,
) -> Result<AgentLoopReport> {
    HarnessAgentRuntime
        .run_turn_with_events(input, provider, session, registry, event_sink, options)
        .await
}
