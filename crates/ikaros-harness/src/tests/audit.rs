// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn audit_log_records_policy_decision() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let request = PolicyRequest {
        action: "git commit".into(),
        risk: RiskLevel::ShellWrite,
        path: None,
        command: Some("git commit -m nope".into()),
        is_write: true,
    };
    let evaluation = session.evaluate(&request).expect("evaluate");
    assert_eq!(evaluation.decision, PolicyDecision::Deny);
    let events = session.audit.read_all().expect("audit");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Some(PolicyDecision::Deny));
}

#[tokio::test]
async fn audit_log_records_tool_call_policy_and_result() {
    #[derive(Debug)]
    struct ReadOnlySkill;

    #[async_trait]
    impl Skill for ReadOnlySkill {
        fn name(&self) -> &'static str {
            "read_only_test"
        }

        fn description(&self) -> &'static str {
            "test skill"
        }

        fn input_schema(&self) -> serde_json::Value {
            json!({})
        }

        fn risk_level(&self) -> RiskLevel {
            RiskLevel::SafeRead
        }

        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: SkillContext,
        ) -> Result<SkillOutput> {
            Ok(SkillOutput::new("done", json!({"value": 1})))
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let mut registry = SkillRegistry::new();
    registry.register(ReadOnlySkill);
    let result = session
        .execute_skill(&registry, "read_only_test", json!({}))
        .await
        .expect("execute");
    assert!(result.ok);
    let kinds = session
        .audit
        .read_all()
        .expect("audit")
        .into_iter()
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(kinds, vec!["tool_call", "policy_decision", "tool_result"]);
}

#[test]
fn audit_log_reads_concatenated_jsonl_events_from_parallel_append_race() {
    let temp = tempfile::tempdir().expect("tempdir");
    let audit = AuditLog::from_file(temp.path().join("audit.jsonl"));
    let first =
        AuditEvent::new("tool_call", None, "first", json!({"call_id": "a"})).expect("first event");
    let second = AuditEvent::new("policy_decision", None, "second", json!({"call_id": "b"}))
        .expect("second event");
    let first_id = first.id.clone();
    let second_id = second.id.clone();
    let raw = format!(
        "{}{}\n",
        serde_json::to_string(&first).expect("first json"),
        serde_json::to_string(&second).expect("second json")
    );
    std::fs::write(audit.path(), raw).expect("write old malformed audit line");

    let ids = audit
        .read_all()
        .expect("audit should tolerate concatenated JSON objects")
        .into_iter()
        .map(|event| event.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![first_id, second_id]);
}
