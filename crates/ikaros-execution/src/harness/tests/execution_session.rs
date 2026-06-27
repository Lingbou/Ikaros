// SPDX-License-Identifier: GPL-3.0-only

use super::*;

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

#[tokio::test]
async fn execution_session_env_allows_workspace_write() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    let path = workspace.join("notes").join("inside.txt");

    session
        .env
        .write_string(&path, "inside".into())
        .await
        .expect("workspace write should be allowed");

    assert_eq!(fs::read_to_string(path).expect("inside file"), "inside");
}

#[tokio::test]
async fn execution_session_env_denies_workspace_external_writes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&outside).expect("outside");
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    let path = outside.join("owned.txt");

    let error = session
        .env
        .write_string(&path, "outside".into())
        .await
        .expect_err("workspace-external write should be rejected");

    assert!(matches!(error, IkarosError::OutOfScope(_)));
    assert!(!path.exists());
}

#[tokio::test]
async fn execution_session_env_denies_process_cwd_outside_workspace() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&outside).expect("outside");
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    let request = ProcessRequest::program("ikaros-command-that-must-not-run", Vec::new(), &outside);

    let error = session
        .env
        .run_process(request)
        .await
        .expect_err("workspace-external cwd should be rejected before spawn");

    assert!(matches!(error, IkarosError::OutOfScope(_)));
}

#[cfg(unix)]
#[tokio::test]
async fn execution_session_env_denies_symlink_write_escape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&outside).expect("outside");
    std::os::unix::fs::symlink(&outside, workspace.join("linked-outside")).expect("symlink");
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    let path = workspace.join("linked-outside").join("owned.txt");

    let error = session
        .env
        .write_string(&path, "outside".into())
        .await
        .expect_err("symlink write escape should be rejected");

    assert!(matches!(error, IkarosError::OutOfScope(_)));
    assert!(!outside.join("owned.txt").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn workspace_env_denies_final_symlink_swap_during_write() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(workspace.join("note.txt"), "inside\n").expect("inside");
    fs::write(outside.join("note.txt"), "outside\n").expect("outside");
    let env = WorkspaceExecutionEnv::new(
        &workspace,
        Arc::new(SwapSymlinkOnWriteEnv {
            outside_target: outside.join("note.txt"),
        }),
    );

    let error = env
        .write_string(&workspace.join("note.txt"), "owned\n".into())
        .await
        .expect_err("final symlink swap should be rejected");

    assert!(
        error.to_string().contains("symlink")
            || matches!(error, IkarosError::Io { .. } | IkarosError::OutOfScope(_)),
        "unexpected error: {error:?}"
    );
    assert_eq!(
        fs::read_to_string(outside.join("note.txt")).expect("outside unchanged"),
        "outside\n"
    );
}
