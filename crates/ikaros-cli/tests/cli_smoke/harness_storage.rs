// SPDX-License-Identifier: GPL-3.0-only

use std::fs;

use crate::support::{TestHome, install_echo_plugin, parse_approval_id, write_echo_plugin};

#[test]
fn approval_replay_writes_only_after_user_approval() {
    let env = TestHome::new();
    env.init();

    let output = env.run(["fs", "write", "notes.txt", "hello approved smoke"]);
    let approval_id = parse_approval_id(&output);
    assert!(!env.workspace.join("notes.txt").exists());

    let approval = env.run(["approval", "approve", &approval_id]);
    assert!(approval.contains("\"status\": \"Approved\""));
    assert!(approval.contains("summary: wrote"));
    assert_eq!(
        fs::read_to_string(env.workspace.join("notes.txt")).expect("written file"),
        "hello approved smoke"
    );

    let approvals = env.run(["approval", "list", "--all"]);
    assert!(approvals.contains("\"status\": \"Executed\""));
    assert!(approvals.contains(&approval_id));
}

#[test]
fn local_rag_ingest_searches_after_approval_replay() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    fs::write(
        env.workspace.join("doc.txt"),
        "Ikaros local rag smoke document\n",
    )
    .expect("rag source");

    let ingest = env.run(["rag", "ingest", "doc.txt", "--scope", "smoke"]);
    assert!(ingest.contains("\"decision\": \"ask_user\""));
    assert!(!env.home.join("rag/index.jsonl").exists());

    let approval_id = parse_approval_id(&ingest);
    let approved = env.run(["approval", "approve", &approval_id]);
    assert!(approved.contains("summary: rag ingest complete"));
    assert!(env.home.join("rag/index.jsonl").exists());

    let search = env.run(["rag", "search", "--scope", "smoke", "Ikaros"]);
    assert!(search.contains("summary: rag search complete"));
    assert!(search.contains("Ikaros local rag smoke document"));
    assert!(search.contains("\"embedding_provider\": \"hash\""));
}

#[test]
fn sqlite_memory_and_rag_backends_are_configured_end_to_end() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

memory:
  backend: sqlite

chat_history:
  backend: sqlite

rag:
  backend: sqlite
  embedding_provider: hash
  embedding_model: text-embedding-3-small
"#,
    )
    .expect("sqlite config");

    let added = env.run([
        "memory",
        "add",
        "--kind",
        "project",
        "--scope",
        "sqlite-smoke",
        "Ikaros sqlite memory smoke",
    ]);
    assert!(added.contains("summary: memory appended"));
    assert!(added.contains("\"backend\": \"sqlite\""));
    assert!(env.home.join("memory/memory.sqlite").exists());
    assert!(!env.home.join("memory/memory.jsonl").exists());

    let memory = env.run([
        "memory",
        "search",
        "--kind",
        "project",
        "--scope",
        "sqlite-smoke",
        "sqlite memory",
    ]);
    assert!(memory.contains("Ikaros sqlite memory smoke"));

    fs::write(
        env.workspace.join("sqlite-doc.txt"),
        "Ikaros sqlite rag smoke document\n",
    )
    .expect("sqlite rag source");
    let ingest = env.run(["rag", "ingest", "sqlite-doc.txt", "--scope", "sqlite-smoke"]);
    assert!(ingest.contains("\"decision\": \"ask_user\""));
    assert!(!env.home.join("rag/index.sqlite").exists());

    let approval_id = parse_approval_id(&ingest);
    let approved = env.run(["approval", "approve", &approval_id]);
    assert!(approved.contains("summary: rag ingest complete"));
    assert!(env.home.join("rag/index.sqlite").exists());
    assert!(!env.home.join("rag/index.jsonl").exists());

    let rag = env.run(["rag", "search", "--scope", "sqlite-smoke", "sqlite rag"]);
    assert!(rag.contains("Ikaros sqlite rag smoke document"));
    assert!(rag.contains("\"embedding_provider\": \"hash\""));

    let chat = env.run([
        "chat",
        "--message",
        "sqlite chat history smoke",
        "--no-context",
    ]);
    assert!(chat.contains("provider: mock"));
    assert!(chat.contains("emotion: Satisfied"));
    assert!(chat.contains("chat_history:"));
    assert!(env.home.join("chat/history.sqlite").exists());
    assert!(!env.home.join("chat/history.jsonl").exists());
    let history = env.run(["chat", "--history", "--history-limit", "1"]);
    assert!(history.contains("chat_history_backend: sqlite"));
    assert!(history.contains("records: 1"));
    let history_search = env.run(["chat", "--history-search", "sqlite chat"]);
    assert!(history_search.contains("chat_history_backend: sqlite"));
    assert!(history_search.contains("records: 1"));
    assert!(history_search.contains("matches:"));
}

#[test]
fn schedule_body_and_service_surfaces_stay_local_and_non_mutating_by_default() {
    let env = TestHome::new();
    env.init();

    let scheduled = env.run([
        "schedule",
        "add",
        "--profile",
        "plan",
        "summarize schedule smoke",
    ]);
    assert!(scheduled.contains("scheduled:"));
    assert!(scheduled.contains("\"agent\": \"plan\""));

    let due = env.run(["schedule", "run-due", "--dry-run"]);
    assert!(due.contains("summarize schedule smoke"));
    assert!(due.contains("\"local_file\""));
    assert!(due.contains("schedule_store:"));
    let schedules = fs::read_to_string(env.home.join("automation/schedules.jsonl"))
        .expect("schedule store should remain local");
    assert!(schedules.contains("summarize schedule smoke"));

    let run_due = env.run(["schedule", "run-due", "--limit", "1"]);
    assert!(run_due.contains("\"task_state\":"));
    assert!(run_due.contains("\"target\": \"local_file\""));
    assert!(env.home.join("automation/deliveries").exists());

    let gateway_scheduled = env.run([
        "schedule",
        "add",
        "--delivery",
        "gateway-outbox",
        "summarize gateway schedule smoke",
    ]);
    assert!(gateway_scheduled.contains("\"gateway_outbox\""));
    let gateway_run_due = env.run(["schedule", "run-due", "--limit", "1"]);
    assert!(gateway_run_due.contains("\"target\": \"gateway_outbox\""));
    let outbox =
        fs::read_to_string(env.home.join("gateway/outbox.jsonl")).expect("gateway schedule outbox");
    assert!(outbox.contains("\"kind\":\"schedule_report\""));
    assert!(outbox.contains("Ikaros Scheduled Job Result"));

    let body = env.run(["body", "status"]);
    assert!(body.contains("body=cli"));
    assert!(body.contains("persona=Ikaros"));

    let dashboard = env.run([
        "body",
        "dashboard",
        "--output",
        "dashboard/smoke.html",
        "--snapshot-output",
        "dashboard/frame.json",
        "--refresh-seconds",
        "5",
    ]);
    assert!(dashboard.contains("dashboard:"));
    assert!(dashboard.contains("snapshot:"));
    let html = fs::read_to_string(env.home.join("dashboard/smoke.html")).expect("dashboard html");
    let frame = fs::read_to_string(env.home.join("dashboard/frame.json")).expect("frame json");
    assert!(html.contains("Ikaros"));
    assert!(frame.contains("\"persona_name\": \"Ikaros\""));

    let service = env.run([
        "service",
        "render",
        "--kind",
        "message-webhook",
        "--manager",
        "systemd",
        "--output",
        "services/message-webhook.service",
    ]);
    assert!(service.contains("service_template:"));
    let service_template = fs::read_to_string(env.home.join("services/message-webhook.service"))
        .expect("service template");
    assert!(service_template.contains("message webhook"));
    assert!(!service_template.contains("systemctl"));
}

#[test]
fn command_backed_plugin_runs_only_through_approval_and_redacts_io() {
    let env = TestHome::new();
    let plugin_source = env.workspace.join("plugin-source/hello");
    write_echo_plugin(&plugin_source);
    env.init();

    let plugin_source_arg = plugin_source.to_string_lossy().into_owned();
    let installed = env.run(["skill", "install", plugin_source_arg.as_str()]);
    assert!(installed.contains("plugin: hello"));
    assert!(installed.contains("enabled: false"));
    assert!(installed.contains("replaced: false"));
    assert!(installed.contains("command_skills: 1"));

    let audit = env.run(["skill", "audit"]);
    assert!(audit.contains("plugins: 1"));
    assert!(audit.contains("enabled: 0"));
    assert!(audit.contains("disabled: 1"));
    assert!(audit.contains("skills: 1"));
    assert!(audit.contains("command_skills: 1"));
    assert!(audit.contains("missing_commands: 0"));
    assert!(audit.contains("hello 0.1.0 [disabled"));

    let list = env.run(["skill", "list"]);
    assert!(list.contains("hello 0.1.0 [disabled"));
    assert!(list.contains("skills disabled by marketplace metadata"));

    let enabled = env.run(["skill", "enable", "hello"]);
    assert!(enabled.contains("plugin: hello"));
    assert!(enabled.contains("enabled: true"));
    let audit = env.run(["skill", "audit"]);
    assert!(audit.contains("enabled: 1"));
    assert!(audit.contains("disabled: 0"));
    assert!(audit.contains("enabled_skills: 1"));

    let duplicate = env.run_failure(["skill", "install", plugin_source_arg.as_str()]);
    assert!(duplicate.contains("plugin already installed"));

    install_echo_plugin(&env.home);
    let plugin_path = env.home.join("skills/hello");
    let plugin_path = plugin_path.to_string_lossy().into_owned();
    let validated = env.run(["skill", "validate", plugin_path.as_str()]);
    assert!(validated.contains("plugin: hello"));
    assert!(validated.contains("skills: 1"));
    assert!(validated.contains("command_skills: 1"));
    assert!(validated.contains("missing_commands: none"));

    let list = env.run(["skill", "list"]);
    assert!(list.contains("hello.echo [SafeRead command]"));

    let disabled = env.run(["skill", "disable", "hello"]);
    assert!(disabled.contains("plugin: hello"));
    assert!(disabled.contains("enabled: false"));
    let list = env.run(["skill", "list"]);
    assert!(list.contains("hello 0.1.0 [disabled"));
    assert!(list.contains("skills disabled by marketplace metadata"));
    let inspect = env.run(["skill", "inspect", "hello.echo"]);
    assert!(inspect.contains("enabled: false"));

    let enabled = env.run(["skill", "enable", "hello"]);
    assert!(enabled.contains("plugin: hello"));
    assert!(enabled.contains("enabled: true"));
    let list = env.run(["skill", "list"]);
    assert!(list.contains("hello.echo [SafeRead command]"));

    let inspect = env.run(["skill", "inspect", "hello.echo"]);
    assert!(inspect.contains("kind: plugin-manifest"));
    assert!(inspect.contains("enabled: true"));
    assert!(inspect.contains("command: bin/echo.sh") || inspect.contains("command: bin/echo.cmd"));

    let requested = env.run([
        "skill",
        "run",
        "hello.echo",
        "--input-json",
        r#"{"message":"hello plugin smoke token=abc123"}"#,
    ]);
    assert!(requested.contains("summary: workspace-external reads require approval"));
    assert!(!requested.contains("abc123"));

    let approval_id = parse_approval_id(&requested);
    let approved = env.run(["approval", "approve", &approval_id]);
    assert!(approved.contains("summary: plugin command executed: hello.echo"));
    assert!(approved.contains("[REDACTED_SECRET]"));
    assert!(!approved.contains("abc123"));
    assert!(approved.contains("\"plugin\": \"hello\""));

    let uninstalled = env.run(["skill", "uninstall", "hello"]);
    assert!(uninstalled.contains("plugin: hello"));
    assert!(uninstalled.contains("marketplace_entry_removed: true"));
    let audit = env.run(["skill", "audit"]);
    assert!(audit.contains("plugins: 0"));
    assert!(audit.contains("plugin_details: none"));
    let list = env.run(["skill", "list"]);
    assert!(!list.contains("hello.echo"));
}

#[test]
fn guarded_edit_applies_exact_diff_after_approval() {
    let env = TestHome::new();
    env.init();
    fs::write(env.workspace.join("notes.txt"), "one\n").expect("source file");

    let diff = "\
diff --git a/notes.txt b/notes.txt
--- a/notes.txt
+++ b/notes.txt
@@ -1 +1 @@
-one
+two
";
    let requested = env.run(["code", "guarded-edit", "change one to two", "--diff", diff]);
    assert!(requested.contains("\"decision\": \"ask_user\""));
    assert_eq!(
        fs::read_to_string(env.workspace.join("notes.txt")).expect("unchanged source"),
        "one\n"
    );

    let approval_id = parse_approval_id(&requested);
    let approved = env.run(["approval", "approve", &approval_id]);
    assert!(approved.contains("summary: guarded code edit applied"));
    assert!(approved.contains("\"files_changed\": 1"));
    assert_eq!(
        fs::read_to_string(env.workspace.join("notes.txt")).expect("patched source"),
        "two\n"
    );
}
