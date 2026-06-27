use super::*;

#[test]
fn coding_turn_context_records_workspace_git_mode_and_redacts_secret() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join(".git/refs/heads")).expect("git refs");
    fs::write(temp.path().join(".git/HEAD"), "ref: refs/heads/main\n").expect("head");
    fs::write(
        temp.path().join(".git/refs/heads/main"),
        "0123456789abcdef0123456789abcdef01234567\n",
    )
    .expect("ref");
    fs::write(temp.path().join(".git/status_porcelain_v1"), "").expect("status");
    fs::write(temp.path().join("Cargo.toml"), "[workspace]\n").expect("cargo");
    let expected_root = canonical_or_original(temp.path());

    let context = CodingTurnContext::from_workspace(CodingTurnContextInput {
        workspace_root: temp.path().to_path_buf(),
        objective: "fix token=abc123 without leaking it".into(),
        mode: CodingMode::Edit,
        session_id: Some("session-1".into()),
        turn_id: Some("turn-1".into()),
        instructions: vec!["keep the runtime event-first".into()],
        permission_profile: CodingPermissionProfile::default(),
        test_commands: vec![TestCommand {
            command: "cargo test --workspace".into(),
            reason: "workspace regression check".into(),
        }],
    })
    .expect("context");

    assert_eq!(context.mode, CodingMode::Edit);
    assert_eq!(context.session_id.as_deref(), Some("session-1"));
    assert_eq!(context.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(context.workspace_root, expected_root);
    assert_eq!(
        context.git.git_root.as_deref(),
        Some(expected_root.as_path())
    );
    assert_eq!(
        context.git.head.as_deref(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );
    assert_eq!(context.git.branch.as_deref(), Some("main"));
    assert!(!context.git.detached);
    assert_eq!(context.git.dirty, CodingDirtyState::Clean);
    assert!(!context.git.has_staged_changes);
    assert!(!context.git.has_unstaged_changes);
    assert!(!context.git.has_untracked_files);
    assert!(context.objective.contains("[REDACTED_SECRET]"));
    assert!(
        !serde_json::to_string(&context)
            .expect("json")
            .contains("abc123")
    );
}

#[test]
fn coding_turn_context_classifies_git_dirty_state_without_shelling_out() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join(".git/refs/heads")).expect("git refs");
    fs::write(temp.path().join(".git/HEAD"), "ref: refs/heads/main\n").expect("head");
    fs::write(
        temp.path().join(".git/refs/heads/main"),
        "1111111111111111111111111111111111111111\n",
    )
    .expect("ref");
    fs::write(temp.path().join(".git/index"), "pretend index").expect("index");
    fs::write(temp.path().join("tracked.rs"), "old\n").expect("tracked");
    fs::write(temp.path().join("new.rs"), "new\n").expect("untracked");
    fs::create_dir_all(temp.path().join(".git/refs")).expect("refs");
    fs::write(
        temp.path().join(".git/status_porcelain_v1"),
        " M tracked.rs\nA  staged.rs\n?? new.rs\n",
    )
    .expect("status");

    let context = CodingTurnContext::from_workspace(CodingTurnContextInput {
        workspace_root: temp.path().to_path_buf(),
        objective: "classify dirty state".into(),
        mode: CodingMode::Edit,
        session_id: None,
        turn_id: None,
        instructions: Vec::new(),
        permission_profile: CodingPermissionProfile::default(),
        test_commands: Vec::new(),
    })
    .expect("context");

    assert_eq!(context.git.dirty, CodingDirtyState::Dirty);
    assert_eq!(context.git.branch.as_deref(), Some("main"));
    assert!(!context.git.detached);
    assert!(context.git.has_staged_changes);
    assert!(context.git.has_unstaged_changes);
    assert!(context.git.has_untracked_files);
}

#[tokio::test]
async fn coding_turn_context_reads_git_status_through_process_runner() {
    let temp = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join(".git/refs/heads")).expect("git refs");
    fs::write(temp.path().join(".git/HEAD"), "ref: refs/heads/main\n").expect("head");
    fs::write(
        temp.path().join(".git/refs/heads/main"),
        "1111111111111111111111111111111111111111\n",
    )
    .expect("ref");
    let calls = Arc::new(AtomicUsize::new(0));
    let process_runner = GitStatusProcessEnv {
        calls: calls.clone(),
    };

    let context = CodingTurnContext::from_workspace_with_process(
        CodingTurnContextInput {
            workspace_root: temp.path().to_path_buf(),
            objective: "classify dirty state through env".into(),
            mode: CodingMode::Edit,
            session_id: None,
            turn_id: None,
            instructions: Vec::new(),
            permission_profile: CodingPermissionProfile::default(),
            test_commands: Vec::new(),
        },
        &process_runner,
    )
    .await
    .expect("context");

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(context.git.branch.as_deref(), Some("main"));
    assert_eq!(context.git.dirty, CodingDirtyState::Dirty);
    assert!(!context.git.has_staged_changes);
    assert!(context.git.has_unstaged_changes);
    assert!(context.git.has_untracked_files);
}

#[test]
fn coding_turn_context_marks_detached_and_non_git_workspaces() {
    let detached = tempfile::tempdir().expect("detached tempdir");
    fs::create_dir_all(detached.path().join(".git")).expect("git");
    fs::write(
        detached.path().join(".git/HEAD"),
        "2222222222222222222222222222222222222222\n",
    )
    .expect("head");
    fs::write(detached.path().join(".git/status_porcelain_v1"), "").expect("status");
    let detached_context = CodingTurnContext::from_workspace(CodingTurnContextInput {
        workspace_root: detached.path().to_path_buf(),
        objective: "detached".into(),
        mode: CodingMode::Plan,
        session_id: None,
        turn_id: None,
        instructions: Vec::new(),
        permission_profile: CodingPermissionProfile::default(),
        test_commands: Vec::new(),
    })
    .expect("detached context");

    assert!(detached_context.git.detached);
    assert_eq!(detached_context.git.branch, None);
    assert_eq!(detached_context.git.dirty, CodingDirtyState::Clean);

    let non_git = tempfile::tempdir().expect("non git tempdir");
    let non_git_context = CodingTurnContext::from_workspace(CodingTurnContextInput {
        workspace_root: non_git.path().to_path_buf(),
        objective: "non git".into(),
        mode: CodingMode::Plan,
        session_id: None,
        turn_id: None,
        instructions: Vec::new(),
        permission_profile: CodingPermissionProfile::default(),
        test_commands: Vec::new(),
    })
    .expect("non git context");

    assert_eq!(non_git_context.git.git_root, None);
    assert_eq!(non_git_context.git.dirty, CodingDirtyState::NotGit);
}

#[test]
fn coding_mode_capabilities_define_allowed_tools_per_mode() {
    let plan = CodingModeCapabilities::for_mode(CodingMode::Plan);
    assert!(plan.can_read_repo);
    assert!(!plan.can_apply_patch);
    assert!(!plan.can_run_tests);
    assert!(!plan.can_self_modify);
    assert!(plan.validate_request(false, false).is_ok());
    assert!(plan.validate_request(true, false).is_err());

    let review = CodingModeCapabilities::for_mode(CodingMode::Review);
    assert!(review.can_read_repo);
    assert!(!review.can_apply_patch);
    assert!(!review.can_run_tests);

    let test = CodingModeCapabilities::for_mode(CodingMode::Test);
    assert!(test.can_read_repo);
    assert!(!test.can_apply_patch);
    assert!(test.can_run_tests);
    assert!(test.validate_request(false, true).is_ok());
    assert!(test.validate_request(true, true).is_err());

    let edit = CodingModeCapabilities::for_mode(CodingMode::Edit);
    assert!(edit.can_apply_patch);
    assert!(edit.can_run_tests);
    assert!(edit.validate_request(true, true).is_ok());

    let self_modify = CodingModeCapabilities::for_mode(CodingMode::SelfModify);
    assert!(self_modify.can_self_modify);
    assert!(self_modify.requires_self_modify_boundary);
    assert!(self_modify.validate_request(true, false).is_err());
}
