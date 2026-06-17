// SPDX-License-Identifier: GPL-3.0-only

use super::{
    model_turn::run_agent_loop_turn,
    types::{
        AgentEvent, AgentEventSink, AgentLoopInput, AgentLoopOptions, AgentLoopReport,
        noop_agent_event_sink,
    },
};
use ikaros_core::{IkarosError, Result};
use ikaros_harness::{ExecutionSession, SkillRegistry};
use ikaros_models::ModelProvider;
use ikaros_session::ApprovalRecord;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

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
    events: Arc<Mutex<Vec<AgentEvent>>>,
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
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn recorded_events(&self) -> Vec<AgentEvent> {
        self.events
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default()
    }

    pub fn clear_recording(&self) -> Result<()> {
        self.events
            .lock()
            .map_err(|_| IkarosError::Message("recording agent runtime lock poisoned".into()))?
            .clear();
        Ok(())
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
            let recording_sink = RecordingAgentEventSink {
                downstream: event_sink,
                events: self.events.clone(),
            };
            self.inner
                .run_turn_with_events(input, provider, session, registry, &recording_sink, options)
                .await
        })
    }
}

struct RecordingAgentEventSink<'a> {
    downstream: &'a dyn AgentEventSink,
    events: Arc<Mutex<Vec<AgentEvent>>>,
}

impl AgentEventSink for RecordingAgentEventSink<'_> {
    fn emit(&self, event: &AgentEvent) -> Result<()> {
        self.downstream.emit(event)?;
        self.events
            .lock()
            .map_err(|_| IkarosError::Message("recording agent runtime lock poisoned".into()))?
            .push(event.clone());
        Ok(())
    }

    fn emit_approval(&self, approval: &ApprovalRecord) -> Result<()> {
        self.downstream.emit_approval(approval)
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
