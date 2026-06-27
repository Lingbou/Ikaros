// SPDX-License-Identifier: GPL-3.0-only

use super::{
    TaskRunOptions, build_task_plan, execute_task_text, execute_task_text_with_options,
    report::task_emotion_signal, task_report_succeeded, task_report_summary, task_steps,
};
use ikaros_core::{IkarosPaths, RiskLevel};
use ikaros_harness::{PlanStepStatus, StepExecutionRecord, TaskExecutionReport};
use ikaros_soul::RuntimeSignal;

#[test]
fn task_steps_keep_memory_write_last() {
    let steps = task_steps("inspect", "inspect", "task-id");

    assert_eq!(steps.len(), 4);
    assert_eq!(steps[0].skill, "memory_search");
    assert_eq!(steps[1].skill, "rag_search");
    assert_eq!(steps[2].skill, "task_summarize");
    assert_eq!(steps[3].skill, "memory_append");
    assert_eq!(steps[3].risk, RiskLevel::DatabaseWrite);
}

#[test]
fn task_plan_keeps_display_and_execution_steps_in_sync() {
    let plan = build_task_plan("inspect repo", "inspect repo", "task-id");

    assert_eq!(plan.plan.task_id, "task-id");
    assert_eq!(plan.plan.steps.len(), plan.executable_steps.len());
    for (display, executable) in plan.plan.steps.iter().zip(&plan.executable_steps) {
        assert_eq!(display.id, executable.id);
        assert_eq!(display.description, executable.description);
        assert_eq!(display.risk, executable.risk);
        assert_eq!(display.tool.as_deref(), Some(executable.skill.as_str()));
    }
}

#[test]
fn task_report_summary_uses_first_non_successful_step() {
    let report = TaskExecutionReport {
        task_id: "task".into(),
        state: ikaros_core::TaskState::Failed,
        steps: vec![StepExecutionRecord {
            step_id: "step".into(),
            description: "write memory".into(),
            skill: "memory_append".into(),
            risk: RiskLevel::DatabaseWrite,
            status: PlanStepStatus::Failed,
            attempts: 1,
            summary: "write denied".into(),
            approval_id: None,
            started_at: None,
            completed_at: None,
        }],
        audit_path: None,
    };

    assert!(!task_report_succeeded(&report));
    assert_eq!(
        task_report_summary(&report, "completed 1 step"),
        "Failed: write denied"
    );
    assert_eq!(task_emotion_signal(&report), RuntimeSignal::TestFailure);
}

#[tokio::test]
async fn dry_run_task_execution_does_not_write_task_memory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);

    let run = execute_task_text("summarize runtime", true, &paths, &workspace, Some("build"))
        .await
        .expect("task run");

    assert!(run.dry_run);
    assert_eq!(run.agent.as_deref(), Some("build"));
    assert!(run.audit_path.exists());
    assert!(
        !paths.memory_dir.join("memory.jsonl").exists(),
        "dry-run task memory write should not mutate the local memory store"
    );
    let audit_events = ikaros_harness::AuditLog::new(&paths.audit_dir)
        .read_all()
        .expect("audit events");
    assert!(audit_events.iter().any(|event| {
        event.kind == crate::EMOTION_EVENT_KIND
            && event
                .data
                .get("emotion")
                .and_then(serde_json::Value::as_str)
                == Some("Satisfied")
    }));
}

#[tokio::test]
async fn task_execution_can_use_agent_loop_with_mock_provider() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);

    let run = execute_task_text_with_options(
        "summarize runtime",
        TaskRunOptions::agent_loop(true),
        &paths,
        &workspace,
        Some("build"),
    )
    .await
    .expect("agent loop task run");

    assert!(run.dry_run);
    assert!(run.agent_loop.is_some());
    assert_eq!(run.plan.steps.len(), 1);
    assert_eq!(run.plan.steps[0].tool.as_deref(), Some("agent_loop"));
    assert_eq!(run.report.state, ikaros_core::TaskState::Completed);
    assert_eq!(run.report.steps[0].skill, "agent_loop_final");
    assert_eq!(run.report.steps[0].status, PlanStepStatus::Succeeded);
    let audit_events = ikaros_harness::AuditLog::new(&paths.audit_dir)
        .read_all()
        .expect("audit events");
    assert!(
        audit_events
            .iter()
            .any(|event| event.kind == "agent_loop_start")
    );
    assert!(
        audit_events
            .iter()
            .any(|event| event.kind == "agent_loop_end")
    );
    assert!(audit_events.iter().any(|event| {
        event.kind == "task_execution_end"
            && event.data.get("mode").and_then(serde_json::Value::as_str) == Some("agent_loop")
    }));
}

fn write_offline_mock_config(paths: &IkarosPaths) {
    std::fs::create_dir_all(&paths.home).expect("home");
    std::fs::write(
        &paths.config,
        r#"schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag:
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
    .expect("mock config");
}
