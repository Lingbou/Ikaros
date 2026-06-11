// SPDX-License-Identifier: GPL-3.0-only

use crate::{BodyEvent, BodyFrame, BodyKind, BodyStatus};

pub trait BodyAdapter {
    fn kind(&self) -> BodyKind;
    fn render_status(&self, status: &BodyStatus) -> String;
    fn render_event(&self, event: &BodyEvent) -> String;

    fn render_frame(&self, frame: &BodyFrame) -> String {
        std::iter::once(self.render_status(&frame.status))
            .chain(frame.events.iter().map(|event| self.render_event(event)))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, Default)]
pub struct CliBodyAdapter;

impl BodyAdapter for CliBodyAdapter {
    fn kind(&self) -> BodyKind {
        BodyKind::Cli
    }

    fn render_status(&self, status: &BodyStatus) -> String {
        let task = status.task_id.as_deref().unwrap_or("none");
        let task_state = status
            .task_state
            .as_ref()
            .map(|state| format!("{state:?}"))
            .unwrap_or_else(|| "none".into());
        let policies = if status.policy_decisions.is_empty() {
            "none".into()
        } else {
            status
                .policy_decisions
                .iter()
                .map(|decision| format!("{decision:?}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        let audit = status
            .audit_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "none".into());
        let approvals = status
            .approvals_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "none".into());
        format!(
            "body=cli persona={} emotion={} task={} state={} memory_sources={} rag_sources={} policy_decisions={} audit={} approvals={}",
            status.persona_name,
            status.emotion,
            task,
            task_state,
            status.context_sources.memory.len(),
            status.context_sources.rag.len(),
            policies,
            audit,
            approvals,
        )
    }

    fn render_event(&self, event: &BodyEvent) -> String {
        let data = if event.data.is_empty() {
            String::new()
        } else {
            let fields = event
                .data
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(" ");
            format!(" {fields}")
        };
        format!(
            "event={:?} body={:?} message={}{}",
            event.kind, event.body, event.message, data
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BodyEventKind;
    use ikaros_core::{PolicyDecision, TaskState};
    use std::collections::BTreeMap;

    #[test]
    fn cli_body_status_renders_runtime_context() {
        let status = BodyStatus::new("Ikaros", "Focused")
            .with_task("task-1", Some(TaskState::Running))
            .with_context_sources(vec!["memory hit".into()], vec!["rag hit".into()])
            .with_policy_decisions(vec![PolicyDecision::Allow])
            .with_audit_path("/tmp/audit.jsonl")
            .with_approvals_path("/tmp/approvals.jsonl");
        let rendered = CliBodyAdapter.render_status(&status);
        assert!(rendered.contains("body=cli"));
        assert!(rendered.contains("persona=Ikaros"));
        assert!(rendered.contains("state=Running"));
        assert!(rendered.contains("memory_sources=1"));
        assert!(rendered.contains("policy_decisions=Allow"));
    }

    #[test]
    fn cli_body_event_rendering_uses_redacted_data() {
        let event = BodyEvent::new(
            BodyKind::Cli,
            BodyEventKind::Message,
            "received sk-not-real",
            BTreeMap::from([("token".into(), "api_key=abc".into())]),
        );
        let rendered = CliBodyAdapter.render_event(&event);
        assert!(!rendered.contains("sk-not-real"));
        assert!(!rendered.contains("abc"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
    }
}
