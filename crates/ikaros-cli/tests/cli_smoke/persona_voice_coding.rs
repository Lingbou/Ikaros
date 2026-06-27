// SPDX-License-Identifier: GPL-3.0-only

use std::fs;

use crate::support::{
    TestHome, install_smoke_rust_crate, json_path_ends_with, parse_approval_id, skill_output_json,
};
use ikaros_memory::{
    JsonlMemoryCandidateStore, JsonlMemoryStore, JsonlWorkingMemoryStore, MemoryCandidate,
    MemoryCandidateReason, MemoryKind, MemoryRecord, MemoryStore, WorkingMemoryRecord,
};
use ikaros_session::{
    SessionContinuationInput, SessionContinuationKind, SessionId, SessionStore, SqliteSessionStore,
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
    fs::write(
        env.workspace.join("src/lib.rs"),
        "pub fn visible_memory_smoke() -> &'static str { \"ok\" }\n// sk-not-real-secret\n",
    )
    .expect("write redaction fixture");

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
    assert!(test_run.contains("test result: ok"));

    let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
-pub fn visible_memory_smoke() -> &'static str { \"ok\" }
+pub fn visible_memory_smoke() -> &'static str { \"safe-ok\" }
";
    let plan = env.run(["code", "plan", "make memory smoke safer", "--diff", diff]);
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
            .contains("\"ok\"")
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
            .contains("\"safe-ok\"")
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
            .contains("\"ok\"")
    );
}

#[test]
fn terminal_first_coding_commands_share_turn_timeline_and_rollback() {
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
    let debug_json: serde_json::Value = serde_json::from_str(&debug).expect("coding debug json");
    let expected_correlation_id = "session:chat-code-session:turn:chat-code-turn";
    for event in debug_json["events"].as_array().expect("coding events") {
        assert_eq!(
            event["correlation_id"]
                .as_str()
                .expect("coding event correlation id"),
            expected_correlation_id
        );
    }
    for entry in debug_json["entries"].as_array().expect("coding entries") {
        assert_eq!(
            entry["correlation_id"]
                .as_str()
                .expect("coding entry correlation id"),
            expected_correlation_id
        );
    }
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
fn chat_workbench_completes_coding_task_with_patch_test_review_and_rollback() {
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
    let escaped_diff = diff.replace('\n', "\\n");
    let input = format!(
        "/code plan \"prepare tui coding task\" --diff \"{escaped_diff}\" --session-id tui-coding-session --turn-id tui-coding-plan\n\
/code apply \"apply tui coding patch\" --diff \"{escaped_diff}\" --session-id tui-coding-session --turn-id tui-coding-apply\n\
/screen --focus side --select 1 approve-selected\n\
/code test \"run tui coding checks\" --test-command \"cargo test\" --session-id tui-coding-session --turn-id tui-coding-test\n\
/code review --diff \"{escaped_diff}\" --session-id tui-coding-session --turn-id tui-coding-review\n\
/code rollback tui-coding-session --turn-id tui-coding-apply --rollback-turn-id tui-coding-rollback\n\
/screen --focus side --select 1 approve-selected\n\
/timeline --kind coding\n\
/trace --kind approval\n\
/quit\n"
    );

    let output = env.run_with_stdin(["chat", "--chat-session", "tui-coding-session"], &input);

    assert!(output.contains("summary: coding turn completed"));
    assert!(
        output
            .matches("screen_approval_selected: action=approve")
            .count()
            >= 2
    );
    assert!(
        output
            .matches("workbench_approval_replay: executed")
            .count()
            >= 2
    );
    assert!(output.contains("patch_applied"));
    assert!(output.contains("test_evidence_recorded"));
    assert!(output.contains("\"category\": \"Passed\""));
    assert!(output.contains("review_completed"));
    assert!(output.contains("timeline_kind_filter: coding"));
    assert!(output.contains("coding_group: diff"));
    assert!(output.contains("coding_group: test"));
    assert!(output.contains("coding_group: review"));
    assert!(output.contains("trace_kind_filter: approval"));
    assert!(output.contains("approval_resolved"));
    assert!(
        fs::read_to_string(env.workspace.join("src/lib.rs"))
            .expect("rolled back source")
            .contains("a + b")
    );
}

#[test]
fn chat_workbench_exposes_top_level_coding_review_and_rollback_aliases() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "coding-alias-session"],
        "/review --diff \"diff --git a/src/lib.rs b/src/lib.rs\" --session-id coding-alias-session --turn-id coding-alias-review\n/timeline coding-alias-review --kind coding\n/commands rollback\n/quit\n",
    );

    assert!(output.contains("summary: coding turn completed"));
    assert!(output.contains("coding_group: review"));
    assert!(output.contains("timeline_kind_filter: coding"));
    assert!(output.contains("cell kind=coding title=coding review"));
    assert!(output.contains("- /rollback usage=/rollback <session-id> --turn-id <turn-id>"));
}

#[test]
fn chat_workbench_timeline_and_trace_can_filter_a_turn() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "filtered-workbench-session"],
        "/code plan \"prepare filtered turn one\" --session-id filtered-workbench-session --turn-id filtered-turn-one\n/code review --diff \"diff --git a/src/lib.rs b/src/lib.rs\" --session-id filtered-workbench-session --turn-id filtered-turn-two\n/timeline filtered-turn-one\n/trace filtered-turn-two --kind coding\n/trace --failed\n/quit\n",
    );

    assert!(output.contains("timeline: found"));
    assert!(output.contains("timeline_turn_filter: filtered-turn-one"));
    assert!(output.contains("filtered_entries:"));
    assert!(output.contains("filtered_agent_events:"));
    let timeline_section = output
        .split("timeline_turn_filter: filtered-turn-one")
        .nth(1)
        .expect("filtered timeline section")
        .split("trace_command: /trace filtered-turn-two --kind coding")
        .next()
        .expect("timeline before trace");
    assert!(timeline_section.contains("filtered-turn-one"));
    assert!(
        timeline_section
            .contains("correlation=session:filtered-workbench-session:turn:filtered-turn-one")
    );
    assert!(!timeline_section.contains("filtered-turn-two"));

    assert!(output.contains("trace_command: /trace filtered-turn-two --kind coding"));
    assert!(output.contains("trace_turn_filter: filtered-turn-two"));
    assert!(output.contains("trace_kind_filter: coding"));
    assert!(output.contains("trace_spans: 1"));
    let trace_section = output
        .split("trace_command: /trace filtered-turn-two --kind coding")
        .nth(1)
        .expect("filtered trace section");
    assert!(trace_section.contains("filtered-turn-two"));
    assert!(
        trace_section
            .contains("correlation=session:filtered-workbench-session:turn:filtered-turn-two")
    );
    assert!(!trace_section.contains("filtered-turn-one"));
    assert!(output.contains("trace_command: /trace --failed"));
    assert!(output.contains("trace_point_filter: failed"));
}

#[test]
fn chat_workbench_screen_footer_and_paged_timeline_are_navigable() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "screen-workbench-session"],
        "screen turn 1\nscreen turn 2\nscreen turn 3\n/history 2\n/queue pending screen followup token=sk-secret-value\n/screen --raw --focus side --select 1\n/screen --fullscreen\n/screen --inline --focus main --scroll 1\n/screen --down\n/timeline --page 2\n/timeline --kind model\n/timeline --failed\n/quit\n",
    );

    assert!(output.contains("workbench_input_history: 2"));
    assert!(output.contains("screen turn 2"));
    assert!(output.contains("screen turn 3"));
    assert!(output.contains("screen_mode: refreshed"));
    assert!(output.contains("\x1b[?1049h"));
    assert!(output.contains("\x1b[?1049l"));
    assert!(output.contains("screen_header: Ikaros Workbench"));
    assert!(output.contains("screen_sections: status approval timeline trace footer"));
    assert!(output.contains("screen_selected: panel=side row=1"));
    assert!(output.contains("screen_selected_actions: panel=side row=1"));
    assert!(output.contains("screen_selected_actions_json:"));
    assert!(output.contains("\"panel\":\"side\""));
    assert!(output.contains("\"commands\":["));
    assert!(output.contains("screen_json:"));
    assert!(output.contains("\"schema\":\"ikaros-workbench-screen-v1\""));
    assert!(output.contains("\"key_bindings\":"));
    assert!(output.contains("\"command\":\"/screen open-selected\""));
    assert!(output.contains("\"panels\":"));
    assert!(output.contains("\"selected\":"));
    assert!(output.contains("screen_timeline_command: /timeline --page 2"));
    assert!(output.contains("screen_trace_hint: /trace"));
    assert!(output.contains("screen_trace: found"));
    assert!(output.contains("screen_trace_counts:"));
    assert!(output.contains("screen_footer:"));
    assert!(output.contains("provider matrix"));
    assert!(output.contains("command=/provider matrix"));
    assert!(output.contains("live=/provider matrix --live"));
    assert!(output.contains("provider cost"));
    assert!(output.contains("provider health"));
    assert!(output.contains("screen_provider_health: health_status="));
    assert!(output.contains("provider fallback"));
    assert!(output.contains("debug=/provider debug"));
    assert!(output.contains("context budget"));
    assert!(output.contains("command=/context"));
    assert!(output.contains("Approvals / Queue*"));
    assert!(output.contains("input queue"));
    assert!(output.contains("pending_inputs=1"));
    assert!(output.contains("clear=/queue clear"));
    assert!(output.contains("[REDACTED_SECRET]"));
    assert!(!output.contains("sk-secret-value"));
    assert!(output.contains("focus=side"));
    assert!(output.contains("approval_action=/approval approve id"));
    assert!(output.contains("Main*"));
    assert!(output.contains("scroll=main:2"));
    assert!(output.contains("pending_approvals=0"));
    assert!(output.contains("timeline_page: 2"));
    assert!(output.contains("timeline_page_size:"));
    assert!(output.contains("recent_events:"));
    assert!(output.contains("cell kind=session"));
    assert!(output.contains("cell kind=model"));
    assert!(output.contains("timeline_kind_filter: model"));
    let model_timeline_section = output
        .split("timeline_kind_filter: model")
        .nth(1)
        .expect("model timeline section");
    assert!(model_timeline_section.contains("filtered_agent_events:"));
    assert!(model_timeline_section.contains("cell kind=model"));
    assert!(!model_timeline_section.contains("cell kind=session"));
    assert!(output.contains("timeline_point_filter: failed"));
}

#[test]
fn chat_workbench_screen_can_open_selected_timeline_cell() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "screen-open-selected-session"],
        "open selected timeline cell\n/screen --focus timeline --select-title entry UserMessage open-selected\n/quit\n",
    );

    assert!(output.contains("screen_open_selected: command=/timeline"));
    assert!(output.contains("screen_open_selected_status: executed"));
    assert!(output.contains("timeline_turn_filter:"));
    assert!(output.contains("timeline_turn: found"));
}

#[test]
fn chat_workbench_can_open_selected_provider_matrix_cell_without_live_probe() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "screen-open-provider-session"],
        "/screen --select-action /provider matrix open-selected\n/quit\n",
    );

    assert!(output.contains("screen_open_selected: command=/provider matrix"));
    assert!(output.contains("screen_open_selected_status: executed"));
    assert!(output.contains("provider_matrix: live=false"));
    assert!(output.contains("matrix_row: kind=model"));
    assert!(!output.contains("provider_matrix: live=true"));
}

#[test]
fn chat_workbench_can_open_selected_provider_cost_cell_as_debug_matrix() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        [
            "chat",
            "--chat-session",
            "screen-open-provider-cost-session",
        ],
        "/screen --focus main --select-action /provider debug open-selected\n/quit\n",
    );

    assert!(output.contains("screen_open_selected: command=/provider debug"));
    assert!(output.contains("screen_open_selected_status: executed"));
    assert!(output.contains("\"format\": \"ikaros-provider-debug-v1\""));
    assert!(output.contains("\"usage_today\""));
    assert!(output.contains("\"estimated_cost_today\""));
    assert!(!output.contains("screen_open_selected_status: unsupported"));
}

#[test]
fn chat_workbench_can_open_selected_provider_health_and_fallback_cells() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "screen-open-provider-health-session"],
        "/screen --select-action /provider health open-selected\n/screen --select-action /provider debug open-selected\n/quit\n",
    );

    assert!(output.contains("screen_open_selected: command=/provider health"));
    assert!(output.contains("health: Unknown"));
    assert!(output.contains("health_log:"));
    assert!(output.contains("screen_open_selected: command=/provider debug"));
    assert!(output.contains("\"format\": \"ikaros-provider-debug-v1\""));
    assert_eq!(
        output
            .matches("screen_open_selected_status: executed")
            .count(),
        2
    );
    assert!(!output.contains("screen_open_selected_status: unsupported"));
}

#[test]
fn chat_workbench_can_open_selected_coding_cell_as_read_only_diff() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "screen-open-coding-session"],
        "/screen --select-action /diff open-selected\n/quit\n",
    );

    assert!(output.contains("screen_open_selected: command=/diff"));
    assert!(output.contains("diff_status:"));
    assert!(output.contains("diff_status_json:"));
    assert!(output.contains("\"schema\":\"ikaros-workbench-diff-status-v1\""));
    assert!(output.contains("\"status\":"));
    assert!(output.contains("\"has_changes\":false"));
    assert!(output.contains("screen_open_selected_status: executed"));
    assert!(!output.contains("screen_open_selected_status: unsupported"));
}

#[test]
fn chat_workbench_can_open_selected_tools_cell() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "screen-open-tools-session"],
        "/screen --select-action /tools open-selected\n/quit\n",
    );

    assert!(output.contains("screen_open_selected: command=/tools"));
    assert!(output.contains("tools_agent:"));
    assert!(output.contains("tools_direct:"));
    assert!(output.contains("tools_status_json:"));
    assert!(output.contains("\"schema\":\"ikaros-workbench-tools-status-v1\""));
    assert!(output.contains("\"agent\":\"build\""));
    assert!(output.contains("\"direct\":"));
    assert!(output.contains("\"deferred\":"));
    assert!(output.contains("\"disabled\":"));
    assert!(output.contains("screen_open_selected_status: executed"));
    assert!(!output.contains("screen_open_selected_status: unsupported"));
}

#[test]
fn chat_workbench_can_clear_selected_pending_input() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "screen-clear-input-session"],
        "/queue first followup token=sk-secret-value\n/queue second followup\n/screen --focus side --select 2 clear-selected\n/queue\n/quit\n",
    );

    assert!(output.contains("screen_input_selected: action=clear index=2"));
    assert!(output.contains("pending_input_removed: index=2 remaining=1"));
    assert!(output.contains("pending_inputs: 1"));
    assert!(output.contains("pending_inputs_json:"));
    assert!(output.contains("\"schema\":\"ikaros-workbench-pending-inputs-v1\""));
    assert!(output.contains("\"pending_count\":1"));
    assert!(output.contains("\"remove\":\"/queue remove 1\""));
    assert!(output.contains("\"clear\":\"/queue clear\""));
    assert!(output.contains("- index=1 message=first followup [REDACTED_SECRET]"));
    assert!(output.contains("\"message\":\"first followup [REDACTED_SECRET]\""));
    assert!(!output.contains("second followup\n- index=2"));
    assert!(!output.contains("sk-secret-value"));
}

#[test]
fn chat_workbench_cancel_outputs_continuations_json_snapshot() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);
    let session_id = SessionId::from("cancel-json-workbench-session");
    let store = SqliteSessionStore::new(env.home.join("agents/build"));
    let mut continuation =
        SessionContinuationInput::new(session_id.clone(), SessionContinuationKind::NextTurn);
    continuation.payload = serde_json::json!({"prompt": "continue after interrupt", "api_key": "sk-continuation-secret"});
    store
        .enqueue_continuation(&continuation)
        .expect("queued continuation");

    let output = env.run_with_stdin(
        ["chat", "--chat-session", session_id.as_str()],
        "/cancel all\n/trace --kind continuation\n/timeline --kind continuation\n/quit\n",
    );

    assert!(output.contains("workbench_cancel: target=all cancelled=1 skipped=0 missing=0"));
    assert!(output.contains("continuations_json:"));
    assert!(output.contains("\"schema\":\"ikaros-workbench-continuations-v1\""));
    assert!(output.contains("\"queued\":0"));
    assert!(output.contains("\"cancelled\":1"));
    assert!(output.contains("\"active_count\":0"));
    assert!(output.contains("trace_kind_filter: continuation"));
    assert!(output.contains("cell kind=continuation title=event continuation_cancelled"));
    assert!(output.contains("continuation_id="));
    assert!(output.contains("continuation_kind=next_turn"));
    assert!(output.contains("reason=workbench cancel"));
    assert!(output.contains("status=cancelled"));
    assert!(output.contains("timeline_kind_filter: continuation"));
    assert!(output.contains("[REDACTED_SECRET]"));
    assert!(!output.contains("sk-continuation-secret"));
}

#[test]
fn chat_workbench_open_selected_does_not_clear_pending_input() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "screen-open-input-session"],
        "/queue queued followup token=sk-secret-value\n/screen --focus side --select-title input queue 1 open-selected\n/queue\n/quit\n",
    );

    assert!(output.contains("screen_open_selected: command=/queue remove 1"));
    assert!(output.contains("screen_open_selected_status: explicit_action_required"));
    assert!(output.contains("pending_inputs: 1"));
    assert!(output.contains("- index=1 message=queued followup [REDACTED_SECRET]"));
    assert!(!output.contains("pending_input_removed:"));
    assert!(!output.contains("sk-secret-value"));
}

#[test]
fn chat_workbench_exposes_session_provider_gateway_tasks_and_approval_status() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "workbench-status-session"],
        "status session seed\n/help\n/status\n/session status\n/provider\n/provider health\n/provider matrix --live\n/provider profiles\n/provider debug\n/gateway\n/tasks\n/approval\n/timeline\n/quit\n",
    );

    assert!(output.contains("/session status|resume|history"));
    assert!(
        output.contains(
            "/provider [inspect|health [--live]|matrix [--live] [--json]|profiles|debug]"
        )
    );
    assert!(output.contains("/gateway"));
    assert!(output.contains("/tasks"));
    assert!(output.contains("/approval"));
    assert!(output.contains("workbench_session: workbench-status-session"));
    let status_model = output
        .lines()
        .find(|line| line.starts_with("status_model:"))
        .expect("status model line");
    assert!(status_model.contains("provider=mock"));
    assert!(status_model.contains("profile=mock"));
    assert!(status_model.contains("profile_source=native"));
    assert!(status_model.contains("context_window=8192"));
    let status_policy = output
        .lines()
        .find(|line| line.starts_with("status_model_policy:"))
        .expect("status model policy line");
    assert!(status_policy.contains("temperature=mock"));
    assert!(status_policy.contains("reasoning=mock"));
    assert!(status_policy.contains("prompt_cache=none"));
    let status_budget = output
        .lines()
        .find(|line| line.starts_with("status_model_budget:"))
        .expect("status model budget line");
    assert!(status_budget.contains("daily_token_budget=disabled"));
    assert!(status_budget.contains("budget_status=unbounded"));
    let status_cost = output
        .lines()
        .find(|line| line.starts_with("status_model_cost:"))
        .expect("status model cost line");
    assert!(status_cost.contains("estimated_cost_today="));
    assert!(status_cost.contains("cache_read_tokens_today="));
    assert!(status_cost.contains("cache_write_tokens_today="));
    assert!(status_cost.contains("cache_accounting=tracked"));
    let status_fallbacks = output
        .lines()
        .find(|line| line.starts_with("status_model_fallbacks:"))
        .expect("status model fallbacks line");
    assert!(status_fallbacks.contains("fallback_count=0"));
    assert!(output.contains("status_workspace:"));
    assert!(output.contains("status_policy:"));
    assert!(output.contains("status_gateway_pending: 0"));
    assert!(output.contains("status_approvals_pending: 0"));
    assert!(output.contains("status_continuations: 0"));
    assert!(output.contains("workbench_status_json:"));
    assert!(output.contains("\"schema\":\"ikaros-workbench-status-v1\""));
    assert!(output.contains("\"session_id\":\"workbench-status-session\""));
    assert!(output.contains("\"provider\":\"mock\""));
    assert!(output.contains("\"budget_status\":\"unbounded\""));
    assert!(output.contains("\"gateway_pending\":0"));
    assert!(output.contains("\"approvals_pending\":0"));
    assert!(output.contains("\"continuations\":0"));
    assert!(output.contains("session_state_db:"));
    assert!(output.contains("session_active_leaf:"));
    assert!(output.contains("session_active_branch_entries:"));
    assert!(output.contains("session_continuations: 0"));
    assert!(output.contains("provider: mock"));
    assert!(output.contains("health: Unknown"));
    assert!(output.contains("provider_matrix: live=true"));
    assert!(output.contains("live_probe=ok"));
    assert!(output.contains("provider_profiles: openai-compatible"));
    assert!(output.contains("profile_row: provider=openai-compatible profile=moonshot-kimi"));
    assert!(output.contains("\"format\": \"ikaros-provider-debug-v1\""));
    assert!(output.contains("\"matrix\""));
    assert!(output.contains("\"fallback_chain\""));
    assert!(output.contains("gateway_pending: 0"));
    assert!(output.contains("gateway_dead_lettered: 0"));
    assert!(output.contains("tasks_enabled: 0"));
    assert!(output.contains("approvals_pending: 0"));
    assert!(output.contains("workbench_evidence: kind=provider"));
    assert!(output.contains("workbench_evidence: kind=gateway"));
    assert!(output.contains("workbench_evidence: kind=tasks"));
    assert!(output.contains("profile: mock"));
    assert!(output.contains("temperature_policy:"));
    assert!(output.contains("reasoning_policy:"));
    assert!(output.contains("request_body_policy:"));
    assert!(output.contains("retry_without_parameters:"));
    assert!(output.contains("context_window:"));
    assert!(output.contains("default_output_tokens:"));
    assert!(output.contains("tokenizer:"));
    assert!(output.contains("streaming:"));
    assert!(output.contains("tool_calls:"));
    assert!(output.contains("network:"));
    assert!(output.contains("timeline: found"));
    assert!(output.contains("workbench provider status queried"));
}

#[test]
fn chat_workbench_status_explains_openai_compatible_profile_without_live_call() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"schema_version: 1

providers:
  model:
    api_key: test-key
    base_url: https://api.moonshot.cn/v1

model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: kimi-k2.6
    compat_profile: auto

rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small

voice:
  tts:
    provider: mock
    model: mock-tts
    voice: default
  asr:
    provider: mock
    model: mock-asr
"#,
    )
    .expect("write openai-compatible config");

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "provider-profile-status-session"],
        "/status\n/quit\n",
    );

    let status_model = output
        .lines()
        .find(|line| line.starts_with("status_model:"))
        .expect("status model line");
    assert!(status_model.contains("provider=openai-compatible"));
    assert!(status_model.contains("model=kimi-k2.6"));
    assert!(status_model.contains("profile=moonshot-kimi"));
    assert!(status_model.contains("profile_source=auto-detected"));
    assert!(status_model.contains("context_window=128000"));
    assert!(status_model.contains("default_output_tokens=32000"));
    assert!(status_model.contains("tokenizer=OpenAiCompatible"));
    let status_policy = output
        .lines()
        .find(|line| line.starts_with("status_model_policy:"))
        .expect("status model policy line");
    assert!(status_policy.contains("temperature=omit"));
    assert!(status_policy.contains("reasoning=moonshot-kimi"));
    assert!(status_policy.contains("tool_schema=moonshot-subset"));
}

#[test]
fn chat_workbench_model_uses_active_runtime_descriptor_without_live_call() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"schema_version: 1

providers:
  model:
    api_key: test-key
    base_url: https://api.moonshot.cn/v1

model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: kimi-k2.6
    compat_profile: auto

rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small

voice:
  tts:
    provider: mock
    model: mock-tts
    voice: default
  asr:
    provider: mock
    model: mock-asr
"#,
    )
    .expect("write openai-compatible config");

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "runtime-model-status-session"],
        "/model\n/quit\n",
    );

    assert!(output.contains("model_source: active_runtime"));
    assert!(output.contains("provider: openai-compatible"));
    assert!(output.contains("model: kimi-k2.6"));
    assert!(output.contains("profile: moonshot-kimi"));
    assert!(output.contains("profile_source: auto-detected"));
    assert!(output.contains("temperature_policy: omit"));
    assert!(output.contains("reasoning_policy: moonshot-kimi"));
    assert!(output.contains("tool_schema_policy: moonshot-subset"));
}

#[test]
fn chat_workbench_model_explains_configured_fallback_chain() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"schema_version: 1

providers:
  model:
    api_key: test-key
    base_url: https://api.moonshot.cn/v1

model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: kimi-k2.6
    compat_profile: auto
    fallbacks:
      - provider: mock
        runtime: harness-agent-loop
        transport: mock
        model: fallback-mock

rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small

voice:
  tts:
    provider: mock
    model: mock-tts
    voice: default
  asr:
    provider: mock
    model: mock-asr
"#,
    )
    .expect("write fallback config");

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "runtime-model-fallback-session"],
        "/model\n/quit\n",
    );

    assert!(output.contains("model_source: active_runtime"));
    assert!(output.contains("provider: openai-compatible"));
    assert!(output.contains("fallback_count: 1"));
    assert!(output.contains(
        "fallback_row: index=0 provider=mock model=fallback-mock configured_profile=auto profile=mock live_smoke=offline"
    ));
    assert!(!output.contains("test-key"));
}

#[test]
fn chat_workbench_agent_switch_uses_agent_instance_model_for_status_and_turns() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"
schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: global-mock

agent:
  default: build
  instances:
    coder:
      profile: build
      model:
        provider: mock
        runtime: harness-agent-loop
        transport: mock
        model: instance-mock

rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small

voice:
  tts:
    provider: mock
    model: mock-tts
    voice: default
  asr:
    provider: mock
    model: mock-asr
"#,
    )
    .expect("write agent instance config");

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "agent-instance-switch-session"],
        "/model\n/agent coder\n/model\nhello from switched instance\n/session history\n/quit\n",
    );

    assert!(output.contains("model: global-mock"));
    assert!(output.contains("agent: coder mode=build"));
    assert!(output.contains("model: instance-mock"));
    assert!(output.contains("history_authority: session_store"));
    assert!(output.contains("provider=mock"));
    assert!(output.contains("model=instance-mock"));
    assert!(!output.contains("agent profile not found"));
}

#[test]
fn chat_workbench_tools_distinguishes_deferred_from_disabled_toolsets() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    let skill_dir = env.home.join("skills/rust_review");
    fs::create_dir_all(&skill_dir).expect("skill dir");
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
description: Review Rust runtime changes for replay evidence.
toolset: coding
provenance: local-skill-doc
support_files: [CHECKLIST.md]
---
# Rust Review

Check policy bypasses and replay evidence.
"#,
    )
    .expect("skill doc");
    fs::write(skill_dir.join("CHECKLIST.md"), "Review checklist").expect("support file");

    let default_output = env.run_with_stdin(
        ["chat", "--chat-session", "tools-default-session"],
        "/tools\n/quit\n",
    );
    assert!(default_output.contains("tools_direct:"));
    assert!(default_output.contains("- direct tool_search"));
    assert!(default_output.contains("tools_deferred:"));
    assert!(default_output.contains("tools_disabled:"));
    assert!(default_output.contains("- deferred rag_search"));
    assert!(default_output.contains("- deferred code_workflow"));
    assert!(default_output.contains("- deferred voice_tts"));
    assert!(default_output.contains("- deferred rust_review"));
    assert!(default_output.contains("kind=prompt_skill"));
    assert!(default_output.contains("callable=false"));
    assert!(default_output.contains("provenance=local-skill-doc"));
    assert!(default_output.contains("support_files=2"));

    fs::write(
        env.home.join("config.yaml"),
        r#"schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small

voice:
  tts:
    provider: mock
    model: mock-tts
    voice: default
  asr:
    provider: mock
    model: mock-asr

agent:
  default: build
  profiles:
    build:
      toolsets: [core, workspace, memory]
"#,
    )
    .expect("write restricted config");

    let restricted_output = env.run_with_stdin(
        ["chat", "--chat-session", "tools-restricted-session"],
        "/tools\n/quit\n",
    );
    assert!(restricted_output.contains("- disabled rag_search"));
    assert!(!restricted_output.contains("- deferred rag_search"));
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
    assert!(output.contains("context_memory_search_limit: 0"));
    assert!(output.contains("context_engine: deterministic"));
    assert!(output.contains("context_engine: llm-summary"));
    assert!(output.contains("memory_backend:"));
    assert!(output.contains("rag_backend:"));
    assert!(output.contains("rag_status_json:"));
    assert!(output.contains("\"schema\":\"ikaros-workbench-rag-status-v1\""));
    assert!(output.contains("\"backend\":\"jsonl\""));
    assert!(output.contains("\"embedding_provider\":\"hash\""));
    assert!(output.contains("\"rag_top_k\":0"));
    assert!(output.contains("diff_status:"));
    assert!(output.contains("timeline: not_found"));
    assert!(output.contains("replay: not_found"));
    assert!(output.contains("debug: not_found"));
    assert!(output.contains("screen_cleared: true"));
}

#[test]
fn chat_workbench_exports_redacted_session_artifact() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let export_path = env.home.join("exports/workbench-export-session.json");
    let input = format!(
        "export this session token=sk-secret-value\n/session export {}\n/quit\n",
        export_path.display()
    );

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "workbench-export-session"],
        &input,
    );

    assert!(output.contains("session_export: created"));
    assert!(output.contains("session_export_format: ikaros-session-export-v1"));
    assert!(output.contains("session_export_redacted: true"));
    assert!(output.contains("session_export_counts: entries="));
    assert!(output.contains("agent_events="));
    assert!(output.contains("approvals="));
    assert!(output.contains("session_export_path:"));

    let artifact = fs::read_to_string(&export_path).expect("session export artifact");
    assert!(artifact.contains("\"format\": \"ikaros-session-export-v1\""));
    assert!(artifact.contains("\"redacted\": true"));
    assert!(artifact.contains("\"session_id\": \"workbench-export-session\""));
    assert!(artifact.contains("[REDACTED_SECRET]"));
    assert!(!artifact.contains("sk-secret-value"));
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
    assert!(output.contains("commands_json:"));
    assert!(output.contains("\"name\":\"/sessions\""));
    assert!(output.contains("\"surfaces\":[\"workbench\",\"gateway\",\"acp\"]"));
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
fn default_entry_opens_human_fullscreen_tui_without_machine_readable_screen_noise() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let default_entry = env.run_with_stdin(std::iter::empty::<&str>(), "/quit\n");
    assert!(!default_entry.contains("Ikaros chat using"));
    assert!(!default_entry.contains("screen_mode:"));
    assert!(!default_entry.contains("screen_json:"));
    assert!(!default_entry.contains("\"schema\":\"ikaros-workbench-screen-v1\""));
    assert!(!default_entry.contains("workbench_status_json:"));
    assert!(!default_entry.contains("trace_command:"));

    let workbench = env.run_with_stdin(
        ["workbench", "--chat-session", "workbench-entry-session"],
        "/screen --raw --focus side --select 1\n/quit\n",
    );
    assert!(workbench.contains("Type /help for commands."));
    assert!(workbench.contains("screen_mode: refreshed"));
    assert!(workbench.contains("screen_json:"));
    assert!(workbench.contains("\"schema\":\"ikaros-workbench-screen-v1\""));
}

#[test]
fn default_entry_chat_turn_does_not_emit_screen_debug_snapshot() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        std::iter::empty::<&str>(),
        "refresh the fullscreen timeline\n/quit\n",
    );

    assert_eq!(output.matches("chat_turn: completed").count(), 1);
    assert!(!output.contains("screen_mode:"));
    assert!(!output.contains("screen_json:"));
    assert!(!output.contains("\"schema\":\"ikaros-workbench-screen-v1\""));
}

#[test]
fn default_entry_slash_command_does_not_emit_screen_debug_snapshot() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        std::iter::empty::<&str>(),
        "/queue queued fullscreen followup token=sk-secret-value\n/quit\n",
    );

    assert!(output.contains("pending_input_queued: 1"));
    assert!(!output.contains("screen_mode:"));
    assert!(!output.contains("screen_json:"));
    assert!(!output.contains("\"schema\":\"ikaros-workbench-screen-v1\""));
    assert!(!output.contains("sk-secret-value"));
}

#[test]
fn default_entry_plain_text_input_keeps_first_letter_when_it_matches_screen_shortcut() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let message = "coding input should keep its first letter";
    let output = env.run_with_stdin(std::iter::empty::<&str>(), &format!("{message}\n/quit\n"));

    assert!(output.contains("chat_turn: completed"));

    let history = env.run(["chat", "--history"]);
    assert!(history.contains(message));
}

#[test]
fn chat_workbench_reports_turn_errors_without_exiting_repl() {
    let env = TestHome::new();
    env.init();
    install_smoke_rust_crate(&env.workspace);
    fs::write(
        env.home.join("config.yaml"),
        r#"schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros
    daily_token_budget: 1

rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small

voice:
  tts:
    provider: mock
    model: mock-tts
    voice: default
  asr:
    provider: mock
    model: mock-asr
"#,
    )
    .expect("write budget-limited config");

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "budget-error-session"],
        "hello over budget\n/screen --raw --focus timeline --select-action /status open-selected\n/status\n/quit\n",
    );

    assert!(output.contains("chat_turn: failed session=budget-error-session"));
    assert!(output.contains("chat_turn_error_json:"));
    assert!(output.contains("\"schema\":\"ikaros-workbench-chat-turn-error-v1\""));
    assert!(output.contains("\"error_kind\":\"budget_exceeded\""));
    assert!(output.contains("\"status\":\"failed\""));
    assert!(output.contains("\"command\":\"/status\""));
    assert!(output.contains("model daily token budget exceeded"));
    assert!(output.contains("chat_turn_recovery_hint: /status shows status_model_budget"));
    assert!(output.contains("model.default.daily_token_budget"));
    assert!(output.contains("cell kind=error title=event error"));
    assert!(output.contains("kind=budget_exceeded"));
    assert!(output.contains("command=/status"));
    assert!(output.contains("trace=/trace --failed"));
    assert!(output.contains("progress=chat_turn:failed"));
    assert!(output.contains("screen_open_selected: command=/status"));
    assert!(output.contains("screen_open_selected_status: executed"));
    assert!(output.contains("screen_trace_counts: session="));
    assert!(output.contains("error=1"));
    assert!(output.contains("workbench_session: budget-error-session"));
}

#[test]
fn chat_workbench_persists_outer_turn_errors_for_screen_recovery() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        [
            "chat",
            "--chat-session",
            "outer-error-session",
            "--context-engine",
            "bogus-engine",
        ],
        "hello with invalid context engine\n/screen --raw --focus timeline\n/quit\n",
    );

    assert!(output.contains("chat_turn: failed session=outer-error-session"));
    assert!(output.contains("unknown context engine"));
    assert!(output.contains("chat_turn_error_json:"));
    assert!(output.contains("\"error_kind\":\"unknown\""));
    assert!(output.contains("cell kind=error title=event error phase=interactive_chat_turn"));
    assert!(output.contains("phase=interactive_chat_turn"));
    assert!(output.contains("command=/trace --failed"));
    assert!(output.contains("screen_trace_counts: session="));
    assert!(output.contains("error=1"));
    assert!(output.contains("screen_selected_actions:"));
    assert!(!output.contains("sk-secret-value"));
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
fn chat_workbench_session_history_uses_session_replay_as_authority() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let session_id = "workbench-replay-history";
    env.run_with_stdin(
        ["chat", "--chat-session", session_id],
        "workbench replay history token=abc123\n/quit\n",
    );
    assert!(!env.home.join("chat/history.jsonl").exists());

    let output = env.run_with_stdin(
        ["chat", "--chat-session", session_id],
        "/session history\n/quit\n",
    );
    assert!(output.contains(&format!("session_history: {session_id}")));
    assert!(output.contains("history_source: session_replay"));
    assert!(output.contains("history_authority: session_store"));
    assert!(output.contains("records: 1"));
    assert!(output.contains("[REDACTED_SECRET]"));
    assert!(!output.contains("sk-not-real-secret"));

    let sessions = env.run_with_stdin(["chat", "--chat-session", session_id], "/sessions\n/quit\n");
    assert!(sessions.contains("history_source: session_replay"));
    assert!(sessions.contains("history_authority: session_store"));
    assert!(sessions.contains("sessions: 1"));
    assert!(sessions.contains(&format!("session={session_id}")));
    assert!(sessions.contains("turns=1"));
    assert!(sessions.contains("[REDACTED_SECRET]"));
    assert!(!sessions.contains("abc123"));
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
    assert!(output.contains("live_cells:"));
    assert!(output.contains("live_cell_summary:"));
    assert!(output.contains("live_cells_json:"));
    assert!(output.contains("\"schema\":\"ikaros-workbench-live-cells-v1\""));
    assert!(output.contains("\"cells\":["));
    assert!(output.contains("model_stream_suppressed="));
    assert!(output.contains("text_delta_chunks="));
    assert!(!output.contains("title=event model_stream"));
    assert!(output.contains("Mock Ikaros plan"));
}

#[test]
fn chat_workbench_streaming_turn_prints_rendered_markdown_transcript() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "markdown-workbench-session"],
        "/multi\nrender markdown\n```diff\n-old token=sk-secret-value\n+new\n```\n| File | Status |\n| --- | --- |\n| src/lib.rs | changed |\n.\n/quit\n",
    );

    assert!(output.contains("chat_stream: start"));
    assert!(output.contains("rendered_markdown:"));
    assert!(output.contains("[diff]"));
    assert!(output.contains("[/diff]"));
    assert!(output.contains("[table]"));
    assert!(output.contains("File | Status"));
    assert!(output.contains("[REDACTED_SECRET]"));
    assert!(!output.contains("sk-secret-value"));
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
    assert!(output.contains("correlation=session:timeline-workbench-session:turn:"));
}

#[test]
fn chat_workbench_timeline_uses_store_level_replay_pagination() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "timeline-page-workbench-session"],
        "timeline page one\ntimeline page two\ntimeline page three\n/timeline --page 2\n/quit\n",
    );

    assert!(output.contains("timeline: found"));
    assert!(output.contains("timeline_page: 2"));
    assert!(output.contains("timeline_page_source: session_store_page"));
    assert!(output.contains("timeline_page_totals:"));
    assert!(output.contains("recent_entries:"));
    assert!(output.contains("recent_events:"));
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
    assert!(output.contains("correlation=session:trace-workbench-session:turn:"));
    let trace_section = output
        .split("trace_command: /trace")
        .nth(1)
        .expect("trace output section");
    assert!(trace_section.contains("trace_events:"));
    assert!(trace_section.contains("cell kind=model"));
    assert!(trace_section.contains("cell kind=session"));
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
    let memory_store = JsonlMemoryStore::new(env.home.join("memory"));
    memory_store
        .append(
            MemoryRecord::new(
                MemoryKind::User,
                "default",
                "User preference: timeline-visible memory",
            )
            .expect("memory"),
        )
        .expect("append memory");
    let old_memory = memory_store
        .append(
            MemoryRecord::new(
                MemoryKind::Relationship,
                "default",
                "Old relationship memory: verbose explanations are preferred",
            )
            .expect("old memory"),
        )
        .expect("append old memory");
    let replacement_memory = MemoryRecord::new(
        MemoryKind::Relationship,
        "default",
        "Current relationship memory: concise explanations are preferred",
    )
    .expect("replacement memory");
    let (_old, active_memory) = memory_store
        .supersede(&old_memory.id, replacement_memory)
        .expect("supersede memory")
        .expect("superseded memory");
    let candidate_store = JsonlMemoryCandidateStore::new(env.home.join("memory"));
    candidate_store
        .create(
            MemoryCandidate::new(
                MemoryKind::Project,
                "default",
                "Project convention: explain memory layers",
                MemoryCandidateReason::Manual,
                0.82,
            )
            .expect("candidate"),
        )
        .expect("create candidate");
    let working_store = JsonlWorkingMemoryStore::new(env.home.join("memory"));
    working_store
        .append(
            WorkingMemoryRecord::new(
                "visible-context-session",
                MemoryKind::Task,
                "visible-context-session",
                "Temporary turn scratchpad",
                None,
            )
            .expect("working memory"),
        )
        .expect("append working memory");
    env.run(["memory", "projection", "render"]);

    let output = env.run_with_stdin(
        ["chat", "--chat-session", "visible-context-session"],
        "Please inspect @file:src/lib.rs:1-2 for the visible memory smoke.\n/screen --focus timeline --select 1 open-selected\n/screen --focus main --select-kind memory open-selected\n/context\n/memory\n/quit\n",
    );

    assert!(output.contains("screen_open_selected: command=/timeline"));
    assert!(
        output.contains(
            "screen_open_selected: command=/debug memory-lifecycle visible-context-session"
        )
    );
    assert!(output.contains("screen_open_selected_status: executed"));
    assert!(output.contains("memory_lifecycle_json:"));
    assert!(output.contains("context_timeline_events:"));
    assert!(output.contains("cell kind=context"));
    assert!(output.contains("context_prompt_sections:"));
    assert!(output.contains("context_prompt_cache:"));
    assert!(output.contains("context_status_json:"));
    assert!(output.contains("\"schema\":\"ikaros-workbench-context-status-v1\""));
    assert!(output.contains("\"session_id\":\"visible-context-session\""));
    assert!(output.contains("\"prompt_cache\":"));
    assert!(output.contains("\"stable_prefix_hash\":"));
    assert!(output.contains("\"prompt_section_count\":"));
    assert!(output.contains("\"cache_stable_prefix\":"));
    assert!(output.contains("\"kind\":\"references\""));
    assert!(output.contains("prompt_section kind=references"));
    assert!(output.contains("prompt_section kind=tool_guidance"));
    assert!(output.contains("cache_stable_prefix=false"));
    assert!(output.contains("cache_stable_prefix=true"));
    assert!(output.contains("tokens="));
    assert!(!output.contains("prompt_section_content:"));
    assert!(!output.contains("sk-not-real-secret"));
    assert!(output.contains("memory_timeline_events:"));
    assert!(output.contains("cell kind=memory"));
    assert!(output.contains("memory_projection_files: 3"));
    assert!(output.contains("memory_candidates_pending: 1"));
    assert!(output.contains("memory_working_active: 1"));
    assert!(output.contains("memory_status_json:"));
    assert!(output.contains("\"schema\":\"ikaros-workbench-memory-status-v1\""));
    assert!(output.contains("command=/debug memory-lifecycle visible-context-session"));
    assert!(output.contains("\"backend\":\"jsonl\""));
    assert!(output.contains("\"projection_files\":3"));
    assert!(output.contains("\"pending_candidates\":1"));
    assert!(output.contains("\"working_active\":1"));
    assert!(output.contains("\"superseded_records\":1"));
    assert!(output.contains("\"supersession\":\"memory supersession <memory-id>\""));
    assert!(output.contains("memory_superseded_records: 1"));
    assert!(output.contains(&format!("memory supersession {}", active_memory.id)));
    assert!(output.contains("Project convention: explain memory layers"));
    assert!(output.contains("Temporary turn scratchpad"));
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
    assert!(output.contains("approval_overlay_json:"));
    assert!(output.contains("\"schema\":\"ikaros-workbench-approval-overlay-v1\""));
    assert!(output.contains("\"pending_count\":"));
    assert!(output.contains("approval_item:"));
    assert!(output.contains("provider_call: true"));
    assert!(output.contains("workspace_write: false"));
    assert!(output.contains("shell: false"));
    assert!(output.contains("network:"));
    assert!(output.contains("session: approval-overlay-session turn=approval-overlay-turn"));
    assert!(output.contains("diff_size:"));
    assert!(output.contains("approve: /approval approve"));
    assert!(output.contains("deny: /approval deny"));
    assert!(output.contains("external_replay: ikaros approval approve"));
}

#[test]
fn chat_workbench_can_approve_and_execute_pending_approval() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let requested = env.run([
        "voice",
        "tts",
        "--output",
        "workbench.wav",
        "write from workbench",
    ]);
    assert!(requested.contains("\"decision\": \"ask_user\""));
    assert!(!env.workspace.join("workbench.wav").exists());
    let approval_id = parse_approval_id(&requested);

    let input = "/approval\n/screen --focus side --select 1 approve-selected\n/approval\n/timeline\n/quit\n";
    let output = env.run_with_stdin(["chat", "--chat-session", "approval-action-session"], input);

    assert!(output.contains("approvals_pending: 1"));
    assert!(output.contains(&format!(
        "screen_approval_selected: action=approve id={approval_id}"
    )));
    assert!(output.contains("workbench_approval_decision: approved"));
    assert!(output.contains("workbench_approval_replay: executed"));
    assert!(output.contains("workbench_approval_continue_json:"));
    assert!(output.contains("\"schema\":\"ikaros-workbench-approval-continue-v1\""));
    assert!(output.contains("\"auto_continue_status\":\"completed\""));
    assert!(output.contains("\"pending_count\":0"));
    assert!(
        output.contains("workbench_approval_next: screen=/screen timeline=/timeline trace=/trace")
    );
    assert!(output.contains(
        "workbench_approval_continue: status=executed next=/screen timeline=/timeline trace=/trace pending=0"
    ));
    assert!(output.contains("workbench_approval_resume: none"));
    assert!(output.contains("workbench_evidence: kind=approval"));
    assert!(output.contains("workbench approval approved"));
    assert!(output.contains("summary: mock-tts TTS synthesized"));
    assert!(output.contains("approvals_pending: 0"));
    assert!(env.workspace.join("workbench.wav").exists());
}

#[test]
fn chat_workbench_can_approve_and_execute_pending_file_write() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let requested = env.run([
        "fs",
        "write",
        "workbench-note.txt",
        "approved from workbench token=abc123",
    ]);
    assert!(requested.contains("\"decision\": \"ask_user\""));
    assert!(!env.workspace.join("workbench-note.txt").exists());
    let approval_id = parse_approval_id(&requested);

    let input = "/screen --raw --focus side --select 1\n/screen --focus side --select 1 approve-selected\n/approval\n/quit\n";
    let output = env.run_with_stdin(
        ["chat", "--chat-session", "file-approval-action-session"],
        input,
    );

    assert!(output.contains("screen_selected: panel=side row=1 kind=approval"));
    assert!(output.contains("Approvals / Queue Modal"));
    assert!(output.contains("\"modal\":{\"actions\""));
    assert!(output.contains("\"approve_selected\":\"/screen approve-selected\""));
    assert!(output.contains("\"deny_selected\":\"/screen deny-selected\""));
    assert!(output.contains("tool=fs_write_guarded"));
    assert!(output.contains("scope=workspace"));
    assert!(output.contains("input_preview="));
    assert!(output.contains("[REDACTED_SECRET]"));
    assert!(!output.contains("abc123"));
    assert!(output.contains(&format!(
        "screen_approval_selected: action=approve id={approval_id}"
    )));
    assert!(output.contains("workbench_approval_decision: approved"));
    assert!(output.contains("workbench_approval_replay: executed"));
    assert!(output.contains("\"auto_continue_status\":\"completed\""));
    assert!(output.contains("summary: wrote"));
    assert!(output.contains("approvals_pending: 0"));
    assert_eq!(
        fs::read_to_string(env.workspace.join("workbench-note.txt")).expect("approved note"),
        "approved from workbench token=abc123"
    );
}

#[test]
fn chat_workbench_can_approve_and_execute_pending_shell_test() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let requested = env.run(["--agent", "plan", "test", "run", "--command", "cargo test"]);
    assert!(requested.contains("\"decision\": \"ask_user\""));
    assert!(!requested.contains("\"category\": \"Passed\""));
    let approval_id = parse_approval_id(&requested);

    let input = "/screen --raw --focus side --select 1\n/screen --focus side --select 1 approve-selected\n/approval\n/quit\n";
    let output = env.run_with_stdin(
        [
            "--agent",
            "plan",
            "chat",
            "--chat-session",
            "shell-approval-action-session",
        ],
        input,
    );

    assert!(output.contains("screen_selected: panel=side row=1 kind=approval"));
    assert!(output.contains("tool=run_tests"));
    assert!(output.contains("risk=ShellRead"));
    assert!(output.contains("input_preview="));
    assert!(output.contains("cargo test"));
    assert!(output.contains(&format!(
        "screen_approval_selected: action=approve id={approval_id}"
    )));
    assert!(output.contains("workbench_approval_decision: approved"));
    assert!(output.contains("workbench_approval_replay: executed"));
    assert!(output.contains("summary: test command completed"));
    assert!(output.contains("\"category\": \"Passed\""));
    assert!(output.contains("approvals_pending: 0"));
}

#[test]
fn chat_workbench_can_deny_pending_file_write_without_executing_it() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let requested = env.run([
        "fs",
        "write",
        "denied-workbench-note.txt",
        "do not write from workbench",
    ]);
    assert!(requested.contains("\"decision\": \"ask_user\""));
    assert!(!env.workspace.join("denied-workbench-note.txt").exists());
    let approval_id = parse_approval_id(&requested);

    let input = "/screen --raw --focus side --select 1\n/screen --focus side --select 1 deny-selected\n/approval\n/timeline\n/quit\n";
    let output = env.run_with_stdin(
        ["chat", "--chat-session", "file-deny-action-session"],
        input,
    );

    assert!(output.contains("screen_selected: panel=side row=1 kind=approval"));
    assert!(output.contains("tool=fs_write_guarded"));
    assert!(output.contains(&format!(
        "screen_approval_selected: action=deny id={approval_id}"
    )));
    assert!(output.contains("workbench_approval_decision: denied"));
    assert!(output.contains("workbench_approval_replay: denied"));
    assert!(output.contains("workbench_approval_continue_json:"));
    assert!(output.contains("\"auto_continue_status\":\"stopped\""));
    assert!(output.contains("workbench_evidence: kind=approval"));
    assert!(output.contains("workbench approval denied"));
    assert!(output.contains("approvals_pending: 0"));
    assert!(!env.workspace.join("denied-workbench-note.txt").exists());
}

#[test]
fn chat_workbench_can_deny_pending_shell_test_without_executing_it() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let requested = env.run(["--agent", "plan", "test", "run", "--command", "cargo test"]);
    assert!(requested.contains("\"decision\": \"ask_user\""));
    assert!(!requested.contains("\"category\": \"Passed\""));
    let approval_id = parse_approval_id(&requested);

    let input = "/screen --raw --focus side --select 1\n/screen --focus side --select 1 deny-selected\n/approval\n/quit\n";
    let output = env.run_with_stdin(
        [
            "--agent",
            "plan",
            "chat",
            "--chat-session",
            "shell-deny-action-session",
        ],
        input,
    );

    assert!(output.contains("screen_selected: panel=side row=1 kind=approval"));
    assert!(output.contains("tool=run_tests"));
    assert!(output.contains("risk=ShellRead"));
    assert!(output.contains(&format!(
        "screen_approval_selected: action=deny id={approval_id}"
    )));
    assert!(output.contains("workbench_approval_decision: denied"));
    assert!(output.contains("workbench_approval_replay: denied"));
    assert!(output.contains("approvals_pending: 0"));
    assert!(!output.contains("\"category\": \"Passed\""));
}

#[test]
fn chat_workbench_open_selected_does_not_execute_pending_approval() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    install_smoke_rust_crate(&env.workspace);

    let requested = env.run([
        "voice",
        "tts",
        "--output",
        "workbench-open-selected.wav",
        "do not execute from open selected",
    ]);
    assert!(requested.contains("\"decision\": \"ask_user\""));
    assert!(!env.workspace.join("workbench-open-selected.wav").exists());

    let input =
        "/approval\n/screen --select-action approval_approve open-selected\n/approval\n/quit\n";
    let output = env.run_with_stdin(
        ["chat", "--chat-session", "approval-open-selected-session"],
        &input,
    );

    assert!(
        output.contains("screen_open_selected: command=/approval approve ")
            || output.contains("screen_open_selected: command=/screen approve-selected")
    );
    assert!(output.contains("screen_open_selected_status: explicit_action_required"));
    assert!(output.contains("approvals_pending: 1"));
    assert!(!output.contains("workbench_approval_decision: approved"));
    assert!(!env.workspace.join("workbench-open-selected.wav").exists());
}
