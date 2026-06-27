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
use ikaros_session::{CollectingAgentEventSink, FanoutAgentEventSink};
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

#[derive(Clone)]
pub struct RecordingAgentRuntime<R = HarnessAgentRuntime> {
    inner: R,
    collector: CollectingAgentEventSink,
}

impl RecordingAgentRuntime<HarnessAgentRuntime> {
    pub fn harness() -> Self {
        Self::new(HarnessAgentRuntime)
    }
}

impl<R> RecordingAgentRuntime<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            collector: CollectingAgentEventSink::default(),
        }
    }

    pub fn recorded_events(&self) -> Vec<ikaros_session::AgentEvent> {
        self.collector.events().unwrap_or_default()
    }

    pub fn clear_recording(&self) -> Result<()> {
        self.collector.clear()
    }
}

impl Default for RecordingAgentRuntime<HarnessAgentRuntime> {
    fn default() -> Self {
        Self::harness()
    }
}

impl<R> AgentRuntime for RecordingAgentRuntime<R>
where
    R: AgentRuntime,
{
    fn run_turn_with_events<'a>(
        &'a self,
        input: AgentLoopInput,
        provider: &'a dyn ModelProvider,
        session: &'a ExecutionSession,
        registry: &'a SkillRegistry,
        event_sink: &'a dyn AgentEventSink,
        options: AgentLoopOptions,
    ) -> Pin<Box<dyn Future<Output = Result<AgentLoopReport>> + Send + 'a>> {
        Box::pin(async move {
            self.clear_recording()?;
            let fanout =
                FanoutAgentEventSink::new([event_sink, &self.collector as &dyn AgentEventSink]);
            self.inner
                .run_turn_with_events(input, provider, session, registry, &fanout, options)
                .await
        })
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
