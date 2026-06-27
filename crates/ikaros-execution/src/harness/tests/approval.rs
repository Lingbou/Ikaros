// SPDX-License-Identifier: GPL-3.0-only

use super::*;

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
async fn failed_approved_replay_remains_retryable() {
    #[derive(Debug)]
    struct FlakyApprovedSkill {
        attempts: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Skill for FlakyApprovedSkill {
        fn name(&self) -> &'static str {
            "flaky_approved"
        }

        fn description(&self) -> &'static str {
            "test retryable approved replay failure"
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
            let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                return Err(IkarosError::Message("transient provider failure".into()));
            }
            Ok(SkillOutput::new(
                "flaky approved replay succeeded",
                json!({"attempt": attempt + 1}),
            ))
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let attempts = Arc::new(AtomicUsize::new(0));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    let mut registry = SkillRegistry::new();
    registry.register(FlakyApprovedSkill {
        attempts: attempts.clone(),
    });

    let result = session
        .execute_skill(&registry, "flaky_approved", json!({"path": "marker.txt"}))
        .await
        .expect("approval request");
    let approval_id = result.output["approval_id"]
        .as_str()
        .expect("approval id")
        .to_owned();
    session
        .decide_approval(&approval_id, ApprovalStatus::Approved, None)
        .expect("approve");

    let error = session
        .execute_approved_skill(&registry, &approval_id)
        .await
        .expect_err("first replay should fail");
    assert!(error.to_string().contains("transient provider failure"));
    let record = session
        .approvals
        .get(&approval_id)
        .expect("get")
        .expect("record");
    assert_eq!(record.status, ApprovalStatus::Approved);

    let approved = session
        .execute_approved_skill(&registry, &approval_id)
        .await
        .expect("second replay should retry");
    assert!(approved.ok);
    assert_eq!(approved.output["attempt"], json!(2));
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
