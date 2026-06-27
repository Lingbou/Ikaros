// SPDX-License-Identifier: GPL-3.0-only

use super::*;

impl<'a> AgentHarness<'a> {
    pub async fn run_turn(&mut self, user_input: impl Into<String>) -> Result<AgentHarnessTurn> {
        self.run_user_message(user_input.into()).await
    }

    pub(super) async fn run_user_message(
        &mut self,
        user_input: String,
    ) -> Result<AgentHarnessTurn> {
        let session_id = self.config.session_id.clone();
        let turn_id = self.config.turn_id.take().unwrap_or_default();
        let input = AgentLoopInput {
            session_id: Some(session_id.as_str().to_owned()),
            turn_id: Some(turn_id.as_str().to_owned()),
            task_id: self.config.task_id.clone(),
            system_prompt: self.config.system_prompt.clone(),
            user_input,
        };
        let collector = CollectingAgentEventSink::default();
        let runtime = self.runtime;
        let provider = self.provider;
        let session = self.session;
        let registry = self.registry;
        let persistent_sink = self.event_sink;
        let options = self.config.options.clone();
        let event_sink =
            FanoutAgentEventSink::new([persistent_sink, &collector as &dyn AgentEventSink]);
        let result = {
            let _phase_guard =
                AgentHarnessPhaseGuard::enter(&mut self.phase, AgentHarnessPhase::Turn)?;
            runtime
                .run_turn_with_events(input, provider, session, registry, &event_sink, options)
                .await
        };
        let mut report = result?;
        let events = collector.events()?;
        report.events = events.clone();
        self.enqueue_recoverable_tool_result_continuations(&turn_id, &report)?;
        Ok(AgentHarnessTurn {
            session_id,
            turn_id,
            events,
            report,
        })
    }

    fn enqueue_recoverable_tool_result_continuations(
        &mut self,
        turn_id: &TurnId,
        report: &AgentLoopReport,
    ) -> Result<()> {
        for result in &report.tool_results {
            let Some((tool_name, tool_input)) = recoverable_tool_result_retry(result) else {
                continue;
            };
            self.enqueue_tool_result(turn_id.clone(), tool_name, tool_input)?;
        }
        Ok(())
    }
}
