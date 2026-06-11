// SPDX-License-Identifier: GPL-3.0-only

use crate::*;
use async_trait::async_trait;
use ikaros_core::{PolicyDecision, RiskLevel};
use serde_json::json;
use std::{
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct InterceptEnv {
    calls: Arc<AtomicUsize>,
}

impl FileSystem for InterceptEnv {
    fn read_to_string<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        LocalExecutionEnv.read_to_string(path)
    }

    fn write_string<'a>(
        &'a self,
        path: &'a Path,
        content: String,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.write_string(path, content)
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        LocalExecutionEnv.create_dir_all(path)
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        LocalExecutionEnv.read_dir(path)
    }
}

impl ProcessRunner for InterceptEnv {
    fn run_process<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProcessOutput>> + Send + 'a>> {
        LocalExecutionEnv.run_process(request)
    }
}

impl NetworkEgress for InterceptEnv {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        LocalExecutionEnv.send_network_request(request)
    }
}

impl ExecutionEnv for InterceptEnv {
    fn execute_skill<'a>(
        &'a self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        _session: &'a ExecutionSession,
    ) -> Pin<Box<dyn Future<Output = Result<SkillOutput>> + Send + 'a>> {
        let calls = self.calls.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(SkillOutput::new(
                format!("env executed {}", skill.name()),
                json!({"via_env": true, "input": input}),
            ))
        })
    }
}

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

#[tokio::test]
async fn execution_session_routes_skill_execution_through_env() {
    #[derive(Debug)]
    struct ReadOnlySkill;

    #[async_trait]
    impl Skill for ReadOnlySkill {
        fn name(&self) -> &'static str {
            "read_only_env_test"
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
            panic!("custom execution env should own skill execution")
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let calls = Arc::new(AtomicUsize::new(0));
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"))
        .with_execution_env(Arc::new(InterceptEnv {
            calls: calls.clone(),
        }));
    let mut registry = SkillRegistry::new();
    registry.register(ReadOnlySkill);

    let result = session
        .execute_skill(
            &registry,
            "read_only_env_test",
            json!({"path": "README.md"}),
        )
        .await
        .expect("execute");

    assert!(result.ok);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(result.output["via_env"], true);
    let kinds = session
        .audit
        .read_all()
        .expect("audit")
        .into_iter()
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(kinds, vec!["tool_call", "policy_decision", "tool_result"]);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn local_execution_env_kills_timed_out_process() {
    let temp = tempfile::tempdir().expect("tempdir");
    let pid_file = temp.path().join("child.pid");
    let request = ProcessRequest::program(
        "sh",
        vec![
            "-c".into(),
            "printf '%s\n' $$ > \"$1\"; while :; do sleep 1; done".into(),
            "ikaros-timeout-test".into(),
            pid_file.to_string_lossy().to_string(),
        ],
        temp.path(),
    )
    .with_timeout_ms(200);

    let error = LocalExecutionEnv
        .run_process(request)
        .await
        .expect_err("command should time out");

    assert!(error.to_string().contains("timed out"));
    let pid = fs::read_to_string(&pid_file)
        .expect("pid file")
        .trim()
        .parse::<u32>()
        .expect("pid");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        !PathBuf::from(format!("/proc/{pid}")).exists(),
        "timed-out child process should be killed and reaped"
    );
}

#[test]
fn audit_log_rotates_by_size_and_reads_compressed_archive() {
    let temp = tempfile::tempdir().expect("tempdir");
    let audit =
        AuditLog::from_file(temp.path().join("audit.jsonl")).with_rotation(AuditRotationPolicy {
            max_bytes: 1,
            rotate_on_date_change: false,
        });
    let first = AuditEvent::new(
        "first",
        None,
        "first audit event",
        json!({"payload": "a".repeat(256)}),
    )
    .expect("first event");
    let first_id = first.id.clone();
    audit.append(first).expect("append first");

    let second =
        AuditEvent::new("second", None, "second audit event", json!({})).expect("second event");
    let second_id = second.id.clone();
    audit.append(second).expect("append second");

    let archives = compressed_audit_archives(temp.path());
    assert_eq!(archives.len(), 1);
    let active = fs::read_to_string(audit.path()).expect("active audit");
    assert!(!active.contains(&first_id));
    assert!(active.contains(&second_id));
    let ids = audit
        .read_all()
        .expect("events")
        .into_iter()
        .map(|event| event.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![first_id, second_id]);
}

#[test]
fn audit_log_rotates_by_event_date_and_reads_compressed_archive() {
    let temp = tempfile::tempdir().expect("tempdir");
    let audit =
        AuditLog::from_file(temp.path().join("audit.jsonl")).with_rotation(AuditRotationPolicy {
            max_bytes: 0,
            rotate_on_date_change: true,
        });
    let first = audit_event_at("first", "2026-06-10T23:59:00Z");
    let first_id = first.id.clone();
    audit.append(first).expect("append first");

    let second = audit_event_at("second", "2026-06-11T00:00:00Z");
    let second_id = second.id.clone();
    audit.append(second).expect("append second");

    assert_eq!(compressed_audit_archives(temp.path()).len(), 1);
    let ids = audit
        .read_all()
        .expect("events")
        .into_iter()
        .map(|event| event.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![first_id, second_id]);
}

#[tokio::test]
async fn safe_read_skill_can_use_redacted_audit_input() {
    #[derive(Debug)]
    struct PromptMatchingReadSkill;

    #[async_trait]
    impl Skill for PromptMatchingReadSkill {
        fn name(&self) -> &'static str {
            "prompt_matching_read"
        }

        fn description(&self) -> &'static str {
            "test redacted audit input"
        }

        fn input_schema(&self) -> serde_json::Value {
            json!({})
        }

        fn risk_level(&self) -> RiskLevel {
            RiskLevel::SafeRead
        }

        async fn execute(
            &self,
            input: serde_json::Value,
            _ctx: SkillContext,
        ) -> Result<SkillOutput> {
            Ok(SkillOutput::new(
                "done",
                json!({"matched_real_input": input["query"] == "actual chat prompt"}),
            ))
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let mut registry = SkillRegistry::new();
    registry.register(PromptMatchingReadSkill);
    let result = session
        .execute_read_skill_with_audit_input(
            &registry,
            "prompt_matching_read",
            json!({"query": "actual chat prompt"}),
            json!({"query": "<redacted chat query>"}),
        )
        .await
        .expect("execute");
    assert!(result.ok);
    assert_eq!(result.output["matched_real_input"], json!(true));

    let events = session.audit.read_all().expect("audit");
    let tool_call = events
        .iter()
        .find(|event| event.kind == "tool_call")
        .expect("tool_call");
    assert_eq!(
        tool_call.data["input"]["query"],
        json!("<redacted chat query>")
    );
    assert_eq!(tool_call.data["audit_input_redacted"], json!(true));
    let raw = fs::read_to_string(session.audit.path()).expect("audit file");
    assert!(!raw.contains("actual chat prompt"));
}

#[tokio::test]
async fn redacted_audit_input_rejects_non_safe_read_skills() {
    #[derive(Debug)]
    struct WriteSkill;

    #[async_trait]
    impl Skill for WriteSkill {
        fn name(&self) -> &'static str {
            "write_test"
        }

        fn description(&self) -> &'static str {
            "test write"
        }

        fn input_schema(&self) -> serde_json::Value {
            json!({})
        }

        fn risk_level(&self) -> RiskLevel {
            RiskLevel::LocalWrite
        }

        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: SkillContext,
        ) -> Result<SkillOutput> {
            Ok(SkillOutput::new("done", json!({})))
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let mut registry = SkillRegistry::new();
    registry.register(WriteSkill);
    let error = session
        .execute_read_skill_with_audit_input(
            &registry,
            "write_test",
            json!({"content": "real"}),
            json!({"content": "<redacted>"}),
        )
        .await
        .expect_err("non safe read should fail");
    assert!(error.to_string().contains("SafeRead"));
}

#[tokio::test]
async fn approved_request_executes_and_marks_record_executed() {
    #[derive(Debug)]
    struct WriteMarkerSkill {
        path: PathBuf,
    }

    #[async_trait]
    impl Skill for WriteMarkerSkill {
        fn name(&self) -> &'static str {
            "write_marker"
        }

        fn description(&self) -> &'static str {
            "test write skill"
        }

        fn input_schema(&self) -> serde_json::Value {
            json!({})
        }

        fn risk_level(&self) -> RiskLevel {
            RiskLevel::LocalWrite
        }

        async fn execute(
            &self,
            input: serde_json::Value,
            _ctx: SkillContext,
        ) -> Result<SkillOutput> {
            let content = input
                .get("content")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("missing");
            fs::write(&self.path, content).map_err(|source| IkarosError::io(&self.path, source))?;
            Ok(SkillOutput::new(
                "marker written",
                json!({"path": self.path}),
            ))
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let marker = workspace.join("marker.txt");
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    let mut registry = SkillRegistry::new();
    registry.register(WriteMarkerSkill {
        path: marker.clone(),
    });

    let result = session
        .execute_skill(
            &registry,
            "write_marker",
            json!({"path": "marker.txt", "content": "approved content"}),
        )
        .await
        .expect("ask");
    assert!(!result.ok);
    let approval_id = result
        .output
        .get("approval_id")
        .and_then(serde_json::Value::as_str)
        .expect("approval id")
        .to_string();
    session
        .decide_approval(
            &approval_id,
            ApprovalStatus::Approved,
            Some("test approval".into()),
        )
        .expect("approve");
    let approved = session
        .execute_approved_skill(&registry, &approval_id)
        .await
        .expect("execute approved");
    assert!(approved.ok);
    assert_eq!(
        fs::read_to_string(&marker).expect("marker"),
        "approved content"
    );
    let record = session
        .approvals
        .get(&approval_id)
        .expect("get")
        .expect("record");
    assert_eq!(record.status, ApprovalStatus::Executed);
}

#[tokio::test]
async fn approved_request_replays_original_execution_input() {
    #[derive(Debug)]
    struct InputCheckingSkill;

    #[async_trait]
    impl Skill for InputCheckingSkill {
        fn name(&self) -> &'static str {
            "write_original_input_test"
        }

        fn description(&self) -> &'static str {
            "test original approval input replay"
        }

        fn input_schema(&self) -> serde_json::Value {
            json!({})
        }

        fn risk_level(&self) -> RiskLevel {
            RiskLevel::LocalWrite
        }

        async fn execute(
            &self,
            input: serde_json::Value,
            _ctx: SkillContext,
        ) -> Result<SkillOutput> {
            Ok(SkillOutput::new(
                "checked input",
                json!({"received_original": input["content"] == "token=abc123"}),
            ))
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    let mut registry = SkillRegistry::new();
    registry.register(InputCheckingSkill);

    let result = session
        .execute_skill(
            &registry,
            "write_original_input_test",
            json!({"path": "note.txt", "content": "token=abc123"}),
        )
        .await
        .expect("approval request");
    let approval_id = result.output["approval_id"]
        .as_str()
        .expect("approval id")
        .to_owned();
    let listed = session.pending_approvals().expect("pending approvals");
    assert_eq!(
        listed[0].request.call.input["content"],
        json!("token=[REDACTED_SECRET]")
    );
    session
        .decide_approval(&approval_id, ApprovalStatus::Approved, None)
        .expect("approve");

    let approved = session
        .execute_approved_skill(&registry, &approval_id)
        .await
        .expect("execute approved");

    assert!(approved.ok);
    assert_eq!(approved.output["received_original"], json!(true));
}

#[tokio::test]
async fn approved_request_routes_skill_execution_through_env() {
    #[derive(Debug)]
    struct WriteSkill;

    #[async_trait]
    impl Skill for WriteSkill {
        fn name(&self) -> &'static str {
            "write_env_test"
        }

        fn description(&self) -> &'static str {
            "test approved env route"
        }

        fn input_schema(&self) -> serde_json::Value {
            json!({})
        }

        fn risk_level(&self) -> RiskLevel {
            RiskLevel::LocalWrite
        }

        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: SkillContext,
        ) -> Result<SkillOutput> {
            panic!("approved skill replay should route through execution env")
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let calls = Arc::new(AtomicUsize::new(0));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(
        Arc::new(InterceptEnv {
            calls: calls.clone(),
        }),
    );
    let mut registry = SkillRegistry::new();
    registry.register(WriteSkill);

    let result = session
        .execute_skill(&registry, "write_env_test", json!({"path": "marker.txt"}))
        .await
        .expect("approval request");

    assert!(!result.ok);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let approval_id = result.output["approval_id"]
        .as_str()
        .expect("approval id")
        .to_owned();
    session
        .decide_approval(&approval_id, ApprovalStatus::Approved, None)
        .expect("approve");
    let approved = session
        .execute_approved_skill(&registry, &approval_id)
        .await
        .expect("execute approved");

    assert!(approved.ok);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(approved.output["via_env"], true);
    let record = session
        .approvals
        .get(&approval_id)
        .expect("get")
        .expect("record");
    assert_eq!(record.status, ApprovalStatus::Executed);
}

#[cfg(unix)]
#[tokio::test]
async fn approved_request_revalidates_policy_before_replay() {
    #[derive(Debug)]
    struct WriteSkill;

    #[async_trait]
    impl Skill for WriteSkill {
        fn name(&self) -> &'static str {
            "write_revalidate_test"
        }

        fn description(&self) -> &'static str {
            "test approved replay policy revalidation"
        }

        fn input_schema(&self) -> serde_json::Value {
            json!({})
        }

        fn risk_level(&self) -> RiskLevel {
            RiskLevel::LocalWrite
        }

        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: SkillContext,
        ) -> Result<SkillOutput> {
            panic!("denied replay should not execute")
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(workspace.join("note.txt"), "inside\n").expect("inside");
    fs::write(outside.join("note.txt"), "outside\n").expect("outside");
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    let mut registry = SkillRegistry::new();
    registry.register(WriteSkill);

    let result = session
        .execute_skill(
            &registry,
            "write_revalidate_test",
            json!({"path": "note.txt"}),
        )
        .await
        .expect("approval request");
    let approval_id = result.output["approval_id"]
        .as_str()
        .expect("approval id")
        .to_owned();

    fs::remove_file(workspace.join("note.txt")).expect("remove inside");
    std::os::unix::fs::symlink(outside.join("note.txt"), workspace.join("note.txt"))
        .expect("replace with symlink");
    session
        .decide_approval(&approval_id, ApprovalStatus::Approved, None)
        .expect("approve");

    let error = session
        .execute_approved_skill(&registry, &approval_id)
        .await
        .expect_err("replay should be denied");

    assert!(error.to_string().contains("no longer allowed"));
    assert_eq!(
        fs::read_to_string(outside.join("note.txt")).expect("outside unchanged"),
        "outside\n"
    );
    let record = session
        .approvals
        .get(&approval_id)
        .expect("get")
        .expect("record");
    assert_eq!(record.status, ApprovalStatus::Approved);
}

fn audit_event_at(kind: &str, at: &str) -> AuditEvent {
    let mut event =
        AuditEvent::new(kind, None, format!("{kind} audit event"), json!({})).expect("audit event");
    event.at = at.into();
    event
}

fn compressed_audit_archives(dir: &Path) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(dir)
        .expect("read dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "gz"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}
