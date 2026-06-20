// SPDX-License-Identifier: GPL-3.0-only

use std::fs;

use crate::support::{
    TestHome, install_smoke_rust_crate, json_path_ends_with, parse_approval_id, skill_output_json,
};

#[test]
fn persona_and_relationship_paths_are_local_audited_and_searchable() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();

    let updated = env.run([
        "persona",
        "set",
        "--name",
        "SmokeIkaros",
        "--tone",
        "precise",
        "--trait",
        "local-first",
        "--boundary",
        "audited",
        "--rule",
        "Use harness policy.",
    ]);
    assert!(updated.contains("ok: true"));
    assert!(updated.contains("name: SmokeIkaros"));
    assert!(updated.contains("changed_fields:"));
    let persona_md = fs::read_to_string(env.home.join("persona.md")).expect("persona markdown");
    assert!(persona_md.contains("SmokeIkaros"));
    assert!(persona_md.contains("Use harness policy."));

    let shown = env.run(["persona", "show"]);
    assert!(shown.contains("name: SmokeIkaros"));
    assert!(shown.contains("tone: precise"));

    let reset = env.run(["persona", "reset"]);
    assert!(reset.contains("changed_fields: reset"));
    let reset_persona = env.run(["persona", "show"]);
    assert!(reset_persona.contains("name: Ikaros"));

    let remembered = env.run([
        "relationship",
        "remember",
        "--scope",
        "smoke",
        "--tag",
        "cli",
        "User likes local-first tests",
    ]);
    assert!(remembered.contains("summary: memory appended"));

    let relationship = env.run(["relationship", "show", "--scope", "smoke"]);
    assert!(relationship.contains("scope: smoke"));
    assert!(relationship.contains("notes: 1"));
    assert!(relationship.contains("User likes local-first tests"));
    assert!(relationship.contains("tags=cli,relationship"));

    let learned = env.run([
        "chat",
        "--message",
        "I prefer concise companionship updates.",
        "--no-context",
    ]);
    assert!(learned.contains("relationship_candidates_created=1"));
    let pending_candidate = env.run(["memory", "candidate", "list", "--status", "pending"]);
    assert!(pending_candidate.contains("User preference: concise companionship updates"));
    assert!(pending_candidate.contains("\"kind\": \"Relationship\""));

    let disabled_learning = env.run([
        "chat",
        "--message",
        "Call me smoke friend.",
        "--no-context",
        "--no-relationship-learning",
    ]);
    assert!(disabled_learning.contains("relationship_candidates_created=0"));
}

#[test]
fn mock_voice_tts_asr_and_output_approval_stay_local_and_redacted() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    fs::write(env.workspace.join("audio.wav"), b"fake audio bytes").expect("audio source");

    let tts = env.run([
        "voice",
        "tts",
        "--voice",
        "smoke",
        "--format",
        "wav",
        "--sample-rate-hz",
        "16000",
        "--language",
        "en",
        "hello voice token=abc123",
    ]);
    assert!(tts.contains("summary: mock-tts TTS synthesized"));
    assert!(tts.contains("\"redacted_text_preview\": \"hello voice token=[REDACTED_SECRET]\""));
    assert!(!tts.contains("abc123"));

    let requested = env.run(["voice", "tts", "--output", "out.wav", "write voice file"]);
    assert!(requested.contains("\"decision\": \"ask_user\""));
    assert!(!env.workspace.join("out.wav").exists());

    let approval_id = parse_approval_id(&requested);
    let approved = env.run(["approval", "approve", &approval_id]);
    assert!(approved.contains("summary: mock-tts TTS synthesized"));
    assert!(approved.contains("\"path\":"));
    assert!(env.workspace.join("out.wav").exists());

    let asr = env.run([
        "voice",
        "asr",
        "--format",
        "wav",
        "--sample-rate-hz",
        "16000",
        "--language",
        "en",
        "audio.wav",
    ]);
    assert!(asr.contains("summary: mock-asr ASR transcribed"));
    assert!(asr.contains("\"text\": \"mock transcript\""));
    assert!(!asr.contains("audio.wav"));
}

#[test]
fn engineering_assistant_read_only_paths_run_on_temp_rust_crate() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let agents = env.run(["agent", "list"]);
    assert!(agents.contains("\"default\": \"build\""));
    assert!(agents.contains("\"plan\""));

    let plan_agent = env.run(["agent", "show", "plan"]);
    assert!(plan_agent.contains("\"mode\": \"plan\""));
    assert!(plan_agent.contains("\"workspace_writes\": \"deny\""));

    let handoff = env.run([
        "agent",
        "run",
        "--profile",
        "plan",
        "--dry-run",
        "inspect crate",
    ]);
    assert!(handoff.contains("\"agent\": \"plan\""));
    assert!(handoff.contains("\"state\": \"Failed\""));
    assert!(handoff.contains("denies DatabaseWrite"));

    let policy = env.run([
        "policy",
        "explain",
        "write-file",
        "--risk",
        "local-write",
        "--path",
        "src/lib.rs",
        "--write",
    ]);
    assert!(policy.contains("\"decision\": \"AskUser\""));
    assert!(policy.contains("\"workspace_root\""));

    let self_modify_policy = env.run([
        "policy",
        "explain",
        "self_modify_apply",
        "--risk",
        "self-modify",
        "--path",
        "src/lib.rs",
        "--write",
    ]);
    assert!(self_modify_policy.contains("\"decision\": \"Deny\""));
    assert!(self_modify_policy.contains("self-modification is disabled by default"));

    let repo = env.run(["repo", "scan"]);
    assert!(repo.contains("summary: repo scanned"));
    let repo_json = skill_output_json(&repo);
    let files = repo_json["files"].as_array().expect("repo files");
    assert!(files.iter().any(|file| {
        file["path"]
            .as_str()
            .is_some_and(|path| json_path_ends_with(path, &["Cargo.toml"]))
    }));
    assert!(files.iter().any(|file| {
        file["path"]
            .as_str()
            .is_some_and(|path| json_path_ends_with(path, &["src", "lib.rs"]))
    }));

    let inferred = env.run(["test", "infer"]);
    assert!(inferred.contains("cargo test --workspace --all-features"));

    let test_run = env.run(["test", "run", "--command", "cargo test"]);
    assert!(test_run.contains("summary: test command completed"));
    assert!(test_run.contains("\"category\": \"Passed\""));
    assert!(test_run.contains("tests::adds"));

    let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
-pub fn add(a: i32, b: i32) -> i32 { a + b }
+pub fn add(a: i32, b: i32) -> i32 { a.saturating_add(b) }
";
    let plan = env.run(["code", "plan", "make add safer", "--diff", diff]);
    assert!(plan.contains("summary: coding turn completed"));
    assert!(plan.contains("\"kind\": \"repo_scanned\""));
    assert!(plan.contains("\"kind\": \"final_report_prepared\""));

    let review = env.run(["code", "review", "--diff", diff]);
    assert!(review.contains("summary: coding turn completed"));
    assert!(review.contains("\"changed_files\""));
    assert!(review.contains("No test analysis provided"));

    let iteration = env.run(["code", "iterate", "prepare patch", "--diff", diff]);
    assert!(iteration.contains("summary: patch iteration plan prepared"));
    assert!(iteration.contains("\"ready_for_approval\": false"));
    assert!(iteration.contains("cargo test --workspace --all-features"));

    let workflow = env.run(["code", "workflow", "prepare patch", "--diff", diff]);
    assert!(workflow.contains("summary: coding turn completed"));
    assert!(workflow.contains("\"kind\": \"repo_scanned\""));
    assert!(workflow.contains("\"kind\": \"final_report_prepared\""));
    assert!(workflow.contains("\"requires_guarded_edit\":"));
    assert!(!workflow.contains("abc123"));

    let proposal = env.run([
        "self-modify",
        "propose",
        "--kind",
        "runtime-patch",
        "--target",
        "src/lib.rs",
        "--diff",
        diff,
    ]);
    assert!(proposal.contains("\"apply_available\": false"));
    assert!(proposal.contains("\"manual_apply_available\": true"));
    assert!(proposal.contains("\"ok_to_request_approval\": true"));
    assert!(proposal.contains("\"snapshot_required\": true"));
    assert!(env.home.join("self-modify/proposals.jsonl").exists());
    let proposal_id = proposal
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("\"id\": \"")
                .map(|id| id.trim_end_matches([',', '"']).to_owned())
        })
        .expect("proposal id");

    let proposals = env.run(["self-modify", "list"]);
    let proposals_json = skill_output_json(&proposals);
    let proposals = proposals_json.as_array().expect("self-modify proposals");
    assert!(proposals.iter().any(|proposal| {
        proposal["change_kind"] == "runtime_patch"
            && proposal["target_path"]
                .as_str()
                .is_some_and(|path| json_path_ends_with(path, &["src", "lib.rs"]))
    }));

    let heartbeat = env.run(["self-modify", "heartbeat"]);
    assert!(heartbeat.contains("\"status\": \"manual_apply_only\""));
    assert!(heartbeat.contains("\"proposal_count\": 1"));

    let apply_request = env.run(["self-modify", "request-apply", &proposal_id]);
    assert!(apply_request.contains("\"name\": \"self_modify_apply\""));
    assert!(apply_request.contains("approval: "));
    let apply_approval_id = parse_approval_id(&apply_request);

    let apply_approved = env.run(["approval", "approve", &apply_approval_id]);
    assert!(apply_approved.contains("approval is approved but not executed"));
    assert!(apply_approved.contains("self-modify apply-approved"));
    assert!(
        fs::read_to_string(env.workspace.join("src/lib.rs"))
            .expect("source")
            .contains("a + b")
    );

    let applied = env.run([
        "self-modify",
        "apply-approved",
        &proposal_id,
        "--approval-id",
        &apply_approval_id,
    ]);
    assert!(applied.contains("\"proposal_id\""));
    assert!(applied.contains("\"source\": \"default\""));
    assert!(applied.contains("\"patch_report\""));
    assert!(applied.contains("\"post_checks_passed\": true"));
    assert!(applied.contains("\"operation_id\""));
    assert!(applied.contains("\"command\": \"cargo check --workspace --all-features\""));
    assert!(
        fs::read_to_string(env.workspace.join("src/lib.rs"))
            .expect("updated")
            .contains("saturating_add")
    );

    let rolled_back = env.run(["self-modify", "rollback", &proposal_id]);
    assert!(rolled_back.contains("\"restored_snapshot\": true"));
    assert!(rolled_back.contains("\"operation_id\""));
    let operations = env.run(["self-modify", "operations"]);
    assert!(operations.contains("\"kind\": \"apply\""));
    assert!(operations.contains("\"kind\": \"rollback\""));
    assert!(
        fs::read_to_string(env.workspace.join("src/lib.rs"))
            .expect("restored")
            .contains("a + b")
    );
}

#[test]
fn terminal_first_coding_commands_share_turn_timeline_and_rollback() {
    let env = TestHome::new();
    env.init();
    install_smoke_rust_crate(&env.workspace);

    let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
-pub fn add(a: i32, b: i32) -> i32 { a + b }
+pub fn add(a: i32, b: i32) -> i32 { a.saturating_add(b) }
";
    let plan = env.run([
        "code",
        "plan",
        "prepare terminal coding plan",
        "--diff",
        diff,
        "--session-id",
        "terminal-code-session",
        "--turn-id",
        "terminal-plan-turn",
    ]);
    assert!(plan.contains("summary: coding turn completed"));
    assert!(plan.contains("\"plan_prepared\""));

    let test = env.run([
        "code",
        "test",
        "run terminal coding checks",
        "--test-command",
        "cargo test",
        "--session-id",
        "terminal-code-session",
        "--turn-id",
        "terminal-test-turn",
    ]);
    assert!(test.contains("summary: coding turn completed"));
    assert!(test.contains("\"test_evidence_recorded\""));
    assert!(test.contains("\"category\": \"Passed\""));

    let requested = env.run([
        "code",
        "apply",
        "apply terminal patch",
        "--diff",
        diff,
        "--session-id",
        "terminal-code-session",
        "--turn-id",
        "terminal-apply-turn",
    ]);
    assert!(requested.contains("\"decision\": \"ask_user\""));
    let approval_id = parse_approval_id(&requested);
    let approved = env.run(["approval", "approve", &approval_id]);
    assert!(approved.contains("summary: coding turn completed"));
    assert!(approved.contains("\"patch_applied\""));
    assert!(
        fs::read_to_string(env.workspace.join("src/lib.rs"))
            .expect("patched lib")
            .contains("a.saturating_add(b)")
    );

    let rollback_requested = env.run([
        "code",
        "rollback",
        "terminal-code-session",
        "--turn-id",
        "terminal-apply-turn",
        "--rollback-turn-id",
        "terminal-rollback-turn",
    ]);
    assert!(rollback_requested.contains("\"decision\": \"ask_user\""));
    let rollback_approval_id = parse_approval_id(&rollback_requested);
    let rollback = env.run(["approval", "approve", &rollback_approval_id]);
    assert!(rollback.contains("summary: coding turn completed"));
    assert!(rollback.contains("\"patch_applied\""));
    assert!(
        fs::read_to_string(env.workspace.join("src/lib.rs"))
            .expect("rolled back lib")
            .contains("a + b")
    );

    let debug = env.run([
        "debug",
        "coding-turn",
        "terminal-code-session",
        "--turn-id",
        "terminal-rollback-turn",
    ]);
    assert!(debug.contains("\"patch_applied\""));
    assert!(debug.contains("\"loop_terminated\""));
}

#[test]
fn chat_repl_code_slash_command_uses_coding_turn_timeline() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat"],
        "/code plan \"prepare chat slash coding plan\" --session-id chat-code-session --turn-id chat-code-turn\n/quit\n",
    );
    assert!(output.contains("commands:") || output.contains("Ikaros chat using provider="));
    assert!(output.contains("summary: coding turn completed"));
    assert!(output.contains("coding_progress:"));
    assert!(output.contains("  - plan_prepared:"));
    assert!(output.contains("coding_result:"));
    assert!(output.contains("\"plan_prepared\""));

    let debug = env.run([
        "debug",
        "coding-turn",
        "chat-code-session",
        "--turn-id",
        "chat-code-turn",
    ]);
    assert!(debug.contains("\"session_id\": \"chat-code-session\""));
    assert!(debug.contains("\"turn_id\": \"chat-code-turn\""));
    assert!(debug.contains("\"plan_prepared\""));
    assert!(debug.contains("\"final_report_prepared\""));
}

#[test]
fn chat_workbench_timeline_groups_coding_diff_test_and_review_cells() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "coding-group-session"],
        "/code plan \"prepare grouped coding cells\" --diff \"diff --git a/src/lib.rs b/src/lib.rs\" --session-id coding-group-session --turn-id coding-group-plan\n/code test \"run grouped coding checks\" --test-command \"cargo test\" --session-id coding-group-session --turn-id coding-group-test\n/code review --diff \"diff --git a/src/lib.rs b/src/lib.rs\" --session-id coding-group-session --turn-id coding-group-review\n/timeline\n/quit\n",
    );

    assert!(output.contains("coding_group: progress"));
    assert!(output.contains("coding_group: diff"));
    assert!(output.contains("coding_group: test"));
    assert!(output.contains("coding_group: review"));
    assert!(output.contains("cell kind=coding title=coding diff"));
    assert!(output.contains("cell kind=coding title=coding review"));
}

#[test]
fn chat_workbench_exposes_session_provider_gateway_tasks_and_approval_status() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "workbench-status-session"],
        "/help\n/status\n/session status\n/provider\n/provider health\n/provider matrix --live\n/gateway\n/tasks\n/approval\n/timeline\n/quit\n",
    );

    assert!(output.contains("/session status|resume|history"));
    assert!(output.contains("/provider [inspect|health [--live]|matrix [--live]]"));
    assert!(output.contains("/gateway"));
    assert!(output.contains("/tasks"));
    assert!(output.contains("/approval"));
    assert!(output.contains("workbench_session: workbench-status-session"));
    assert!(output.contains("status_model: provider=mock"));
    assert!(output.contains("status_workspace:"));
    assert!(output.contains("status_policy:"));
    assert!(output.contains("status_gateway_pending: 0"));
    assert!(output.contains("status_approvals_pending: 0"));
    assert!(output.contains("status_continuations: 0"));
    assert!(output.contains("provider: mock"));
    assert!(output.contains("health: Unknown"));
    assert!(output.contains("provider_matrix: live=true"));
    assert!(output.contains("live_probe=ok"));
    assert!(output.contains("gateway_pending: 0"));
    assert!(output.contains("tasks_enabled: 0"));
    assert!(output.contains("approvals_pending: 0"));
    assert!(output.contains("workbench_evidence: kind=provider"));
    assert!(output.contains("workbench_evidence: kind=gateway"));
    assert!(output.contains("workbench_evidence: kind=tasks"));
    assert!(output.contains("timeline: found"));
    assert!(output.contains("workbench provider status queried"));
}

#[test]
fn chat_workbench_exposes_session_aliases_model_context_memory_rag_diff_and_timeline() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "workbench-alias-session"],
        "/help\n/sessions\n/model\n/new\n/resume alias-session\n/status\n/context\n/memory\n/rag\n/diff\n/timeline\n/replay\n/debug\n/clear\n/quit\n",
    );

    assert!(output.contains("/sessions"));
    assert!(output.contains("/resume <session>"));
    assert!(output.contains("/model"));
    assert!(output.contains("/diff"));
    assert!(output.contains("sessions: 0"));
    assert!(output.contains("provider: mock"));
    assert!(output.contains("session_new:"));
    assert!(output.contains("session_resumed: alias-session"));
    assert!(output.contains("workbench_session: alias-session"));
    assert!(output.contains("context_session: alias-session"));
    assert!(output.contains("memory_backend:"));
    assert!(output.contains("rag_backend:"));
    assert!(output.contains("diff_status:"));
    assert!(output.contains("timeline: not_found"));
    assert!(output.contains("replay: not_found"));
    assert!(output.contains("debug: not_found"));
    assert!(output.contains("screen_cleared: true"));
}

#[test]
fn chat_workbench_lists_and_suggests_slash_commands() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "command-catalog-session"],
        "/commands sess\n/sesions\n/quit\n",
    );

    assert!(output.contains("commands_query: sess"));
    assert!(output.contains("/sessions"));
    assert!(output.contains("/session"));
    assert!(output.contains("/resume"));
    assert!(output.contains("unknown command: /sesions"));
    assert!(output.contains("did_you_mean: /sessions"));
}

#[test]
fn chat_workbench_lists_file_and_context_mentions() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "mention-workbench-session"],
        "/mentions lib\n/mentions diff\n/quit\n",
    );

    assert!(output.contains("mentions_query: lib"));
    assert!(output.contains("@file:src/lib.rs"));
    assert!(output.contains("@folder:src"));
    assert!(output.contains("mentions_query: diff"));
    assert!(output.contains("@diff"));
    assert!(output.contains("@staged"));
    assert!(output.contains("@git:HEAD"));
}

#[test]
fn tui_and_default_entry_open_the_same_workbench() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let tui = env.run_with_stdin(["tui", "--chat-session", "tui-session"], "/status\n/quit\n");
    assert!(tui.contains("Type /help for commands."));
    assert!(tui.contains("workbench_session: tui-session"));

    let default_entry = env.run_with_stdin(std::iter::empty::<&str>(), "/status\n/quit\n");
    assert!(default_entry.contains("Type /help for commands."));
    assert!(default_entry.contains("workbench_session:"));
}

#[test]
fn chat_workbench_multiline_and_session_resume_write_one_history_turn() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "initial-workbench-session"],
        "/session resume resumed-workbench-session\n/multi\nfirst workbench line\nsecond workbench line\n.\n/quit\n",
    );

    assert!(output.contains("session_resumed: resumed-workbench-session"));
    assert!(output.contains("multiline: end with a single '.' line"));
    assert!(output.contains("context:"));

    let history = env.run([
        "chat",
        "--history",
        "--history-session",
        "resumed-workbench-session",
    ]);
    assert!(history.contains("records: 1"));
    assert!(history.contains("first workbench line"));
    assert!(history.contains("second workbench line"));

    let workbench_history =
        fs::read_to_string(env.home.join("workbench").join("history.txt")).expect("history");
    assert!(workbench_history.contains("first workbench line"));
    assert!(workbench_history.contains("second workbench line"));
}

#[test]
fn chat_workbench_bracketed_paste_writes_one_history_turn() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "paste-workbench-session"],
        "\u{1b}[200~first pasted line\nsecond pasted line\n\u{1b}[201~\n/quit\n",
    );

    assert!(output.contains("bracketed_paste: accepted"));
    assert!(output.contains("chat_turn: completed"));

    let history = env.run([
        "chat",
        "--history",
        "--history-session",
        "paste-workbench-session",
    ]);
    assert!(history.contains("records: 1"));
    assert!(history.contains("first pasted line"));
    assert!(history.contains("second pasted line"));
}

#[test]
fn chat_workbench_drains_pending_input_queue_after_turn() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "queue-workbench-session"],
        "/queue queued follow up\n/queue\nfirst turn before queue\n/quit\n",
    );

    assert!(output.contains("pending_input_queued: 1"));
    assert!(output.contains("pending_inputs: 1"));
    assert!(output.contains("pending_input: running index=1 total=1"));
    assert!(output.contains("queued follow up"));

    let history = env.run([
        "chat",
        "--history",
        "--history-session",
        "queue-workbench-session",
    ]);
    assert!(history.contains("records: 2"));
    assert!(history.contains("first turn before queue"));
    assert!(history.contains("queued follow up"));
}

#[test]
fn chat_workbench_defaults_to_streaming_and_prints_terminal_turn_events() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "stream-workbench-session"],
        "hello streamed workbench\n/quit\n",
    );

    assert!(output.contains("stream=true"));
    assert!(output.contains("chat_turn: started"));
    assert!(output.contains("chat_stream: start"));
    assert!(output.contains("chat_stream: done"));
    assert!(output.contains("chat_turn: completed"));
    assert!(output.contains("stream_chunks:"));
    assert!(output.contains("Mock Ikaros plan"));
}

#[test]
fn chat_workbench_timeline_shows_cells_after_a_turn() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "timeline-workbench-session"],
        "show timeline cells\n/timeline\n/quit\n",
    );

    assert!(output.contains("timeline: found"));
    assert!(output.contains("entries:"));
    assert!(output.contains("agent_events:"));
    assert!(output.contains("recent_entries:"));
    assert!(output.contains("recent_events:"));
    assert!(output.contains("cell kind=session"));
    assert!(output.contains("cell kind=model"));
}

#[test]
fn chat_workbench_trace_shows_session_spans_without_prompt_or_secret_leak() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "trace-workbench-session"],
        "trace this workbench turn with token=abc123\n/trace\n/quit\n",
    );

    assert!(output.contains("/trace"));
    assert!(output.contains("trace: found"));
    assert!(output.contains("trace_spans:"));
    assert!(output.contains("trace_event_counts:"));
    assert!(output.contains("context="));
    assert!(output.contains("model="));
    assert!(output.contains("cell kind=session title=trace span"));
    let trace_section = output
        .split("trace_command: /trace")
        .nth(1)
        .expect("trace output section");
    assert!(!trace_section.contains("abc123"));
    assert!(!trace_section.contains("trace this workbench turn"));
}

#[test]
fn chat_workbench_long_session_soak_keeps_trace_and_history_replayable() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "soak-workbench-session"],
        "soak turn 1 token=abc123\nsoak turn 2 token=abc123\nsoak turn 3 token=abc123\nsoak turn 4 token=abc123\nsoak turn 5 token=abc123\n/trace\n/timeline\n/quit\n",
    );

    assert!(output.contains("trace: found"));
    assert!(output.contains("trace_spans: 5"));
    assert!(output.contains("timeline: found"));
    assert!(output.contains("agent_events:"));

    let history = env.run([
        "chat",
        "--history",
        "--history-session",
        "soak-workbench-session",
    ]);
    assert!(history.contains("records: 5"));
    assert!(history.contains("[REDACTED_SECRET]"));
    assert!(!history.contains("abc123"));

    let trace = env.run(["debug", "trace", "soak-workbench-session"]);
    let trace_json: serde_json::Value = serde_json::from_str(&trace).expect("trace json");
    assert_eq!(trace_json["format"], "ikaros-trace-v1");
    assert_eq!(
        trace_json["turn_spans"]
            .as_array()
            .expect("turn spans array")
            .len(),
        5
    );
    assert!(!trace.contains("abc123"));
    assert!(!trace.contains("soak turn 1"));
}

#[test]
fn chat_workbench_context_and_memory_show_timeline_cells() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "visible-context-session"],
        "I prefer timeline-visible memory. Please inspect @file:src/lib.rs\n/context\n/memory\n/quit\n",
    );

    assert!(output.contains("context_timeline_events:"));
    assert!(output.contains("cell kind=context"));
    assert!(output.contains("memory_timeline_events:"));
    assert!(output.contains("cell kind=memory"));
}

#[test]
fn chat_workbench_fork_appends_branch_summary_to_session_tree() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "fork-workbench-session"],
        "prepare a branchable timeline\n/fork try alternate implementation\n/timeline\n/quit\n",
    );

    assert!(output.contains("session_forked: fork-workbench-session"));
    assert!(output.contains("fork_parent_entry:"));
    assert!(output.contains("fork_entry:"));
    assert!(output.contains("timeline: found"));
    assert!(output.contains("entry BranchSummary"));
    assert!(output.contains("try alternate implementation"));
}

#[test]
fn coding_workflow_persists_debuggable_turn_timeline() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);
    let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
-pub fn add(a: i32, b: i32) -> i32 { a + b }
+pub fn add(a: i32, b: i32) -> i32 { a.saturating_add(b) }
";

    let workflow = env.run([
        "code",
        "workflow",
        "prepare persistent coding timeline",
        "--session-id",
        "coding-cli-session",
        "--turn-id",
        "coding-cli-turn",
        "--diff",
        diff,
    ]);
    assert!(workflow.contains("summary: coding turn completed"));

    let debug = env.run([
        "debug",
        "coding-turn",
        "coding-cli-session",
        "--turn-id",
        "coding-cli-turn",
    ]);
    assert!(debug.contains("\"session_id\": \"coding-cli-session\""));
    assert!(debug.contains("\"turn_id\": \"coding-cli-turn\""));
    assert!(debug.contains("\"event_count\""));
    assert!(debug.contains("\"patch_skipped\""));
    assert!(debug.contains("\"review_started\""));
    assert!(debug.contains("\"final_report_prepared\""));
}

#[test]
fn coding_workflow_model_loop_requires_approval_and_replays_mock_provider_turn() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let requested = env.run([
        "code",
        "workflow",
        "prepare provider-backed coding turn",
        "--mode",
        "plan",
        "--model-loop",
        "--max-iterations",
        "1",
        "--session-id",
        "coding-model-session",
        "--turn-id",
        "coding-model-turn",
    ]);
    assert!(requested.contains("\"decision\": \"ask_user\""));
    assert!(requested.contains("\"approval_context\""));
    assert!(requested.contains("approval_scope:"));
    assert!(requested.contains("provider_call: true"));
    assert!(requested.contains("workspace_write: false"));
    assert!(requested.contains("shell: false"));
    assert!(requested.contains("shell_commands: none"));
    let approval_id = parse_approval_id(&requested);

    let approved = env.run(["approval", "approve", &approval_id]);
    assert!(approved.contains("summary: coding turn completed"));
    assert!(approved.contains("coding_progress:"));
    assert!(approved.contains("  - model_request_prepared:"));
    assert!(approved.contains("  - model_response_received:"));
    assert!(approved.contains("coding_result: status=passed"));
    assert!(approved.contains("\"model_request_prepared\""));
    assert!(approved.contains("\"model_response_received\""));
    assert!(approved.contains("\"status\": \"passed\""));
    assert!(approved.contains("\"loop_terminated\""));

    let debug = env.run([
        "debug",
        "coding-turn",
        "coding-model-session",
        "--turn-id",
        "coding-model-turn",
    ]);
    assert!(debug.contains("\"model_request_prepared\""));
    assert!(debug.contains("\"model_response_received\""));
    assert!(debug.contains("\"final_report_prepared\""));
}

#[test]
fn chat_workbench_approval_overlay_renders_combined_risk_context() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "approval-overlay-session"],
        "/code plan \"prepare approval overlay\" --model-loop --max-iterations 1 --session-id approval-overlay-session --turn-id approval-overlay-turn\n/approval\n/quit\n",
    );

    assert!(output.contains("\"decision\": \"ask_user\""));
    assert!(output.contains("approval_overlay:"));
    assert!(output.contains("approval_item:"));
    assert!(output.contains("provider_call: true"));
    assert!(output.contains("workspace_write: false"));
    assert!(output.contains("shell: false"));
    assert!(output.contains("network:"));
    assert!(output.contains("session: approval-overlay-session turn=approval-overlay-turn"));
    assert!(output.contains("diff_size:"));
    assert!(output.contains("replay: ikaros approval approve"));
}
