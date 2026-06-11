// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    CancellationToken, ExecutablePlanStep, ExecutionOptions, ExecutionSession, GuardrailConfig,
    IkarosError, PlanStepStatus, Skill, SkillContext, SkillOutput, SkillRegistry,
};
use async_trait::async_trait;
use ikaros_core::{RiskLevel, TaskState};
use serde_json::json;
use std::sync::Arc;
use tokio::time::Duration;

#[tokio::test]
async fn task_runner_completes_safe_step_sequence() {
    #[derive(Debug)]
    struct ReadOnlySkill;

    #[async_trait]
    impl Skill for ReadOnlySkill {
        fn name(&self) -> &'static str {
            "task_read"
        }

        fn description(&self) -> &'static str {
            "test task read"
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
        ) -> crate::Result<SkillOutput> {
            Ok(SkillOutput::new("step done", json!({"value": 1})))
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let mut registry = SkillRegistry::new();
    registry.register(ReadOnlySkill);

    let report = session
        .execute_task_steps(
            &registry,
            "task-1",
            vec![ExecutablePlanStep::new(
                "read context",
                "task_read",
                json!({}),
                RiskLevel::SafeRead,
            )],
            ExecutionOptions::default(),
            CancellationToken::new(),
        )
        .await
        .expect("task runner");

    assert_eq!(report.state, TaskState::Completed);
    assert_eq!(report.steps[0].status, PlanStepStatus::Succeeded);
    assert_eq!(report.steps[0].attempts, 1);
    let kinds = session
        .audit
        .read_all()
        .expect("audit")
        .into_iter()
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert!(kinds.contains(&"task_execution_start".into()));
    assert!(kinds.contains(&"task_execution_end".into()));
}

#[tokio::test]
async fn task_runner_retries_transient_skill_failure() {
    #[derive(Debug)]
    struct FlakySkill {
        attempts: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl Skill for FlakySkill {
        fn name(&self) -> &'static str {
            "flaky"
        }

        fn description(&self) -> &'static str {
            "fails once"
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
        ) -> crate::Result<SkillOutput> {
            let previous = self
                .attempts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if previous == 0 {
                return Err(IkarosError::Message("transient failure".into()));
            }
            Ok(SkillOutput::new("retry succeeded", json!({})))
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut registry = SkillRegistry::new();
    registry.register(FlakySkill {
        attempts: attempts.clone(),
    });

    let report = session
        .execute_task_steps(
            &registry,
            "task-1",
            vec![ExecutablePlanStep::new(
                "retry flaky step",
                "flaky",
                json!({}),
                RiskLevel::SafeRead,
            )],
            ExecutionOptions {
                timeout_ms: None,
                max_retries: 1,
                retry_delay_ms: 0,
                ..ExecutionOptions::default()
            },
            CancellationToken::new(),
        )
        .await
        .expect("task runner");

    assert_eq!(report.state, TaskState::Completed);
    assert_eq!(report.steps[0].status, PlanStepStatus::Succeeded);
    assert_eq!(report.steps[0].attempts, 2);
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[tokio::test]
async fn task_runner_marks_timeout_as_failed() {
    #[derive(Debug)]
    struct SlowSkill;

    #[async_trait]
    impl Skill for SlowSkill {
        fn name(&self) -> &'static str {
            "slow"
        }

        fn description(&self) -> &'static str {
            "sleeps past timeout"
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
        ) -> crate::Result<SkillOutput> {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(SkillOutput::new("too late", json!({})))
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let mut registry = SkillRegistry::new();
    registry.register(SlowSkill);

    let report = session
        .execute_task_steps(
            &registry,
            "task-1",
            vec![ExecutablePlanStep::new(
                "run slow step",
                "slow",
                json!({}),
                RiskLevel::SafeRead,
            )],
            ExecutionOptions {
                timeout_ms: Some(5),
                max_retries: 0,
                retry_delay_ms: 0,
                ..ExecutionOptions::default()
            },
            CancellationToken::new(),
        )
        .await
        .expect("task runner");

    assert_eq!(report.state, TaskState::Failed);
    assert_eq!(report.steps[0].status, PlanStepStatus::Failed);
    assert!(report.steps[0].summary.contains("timed out"));
}

#[tokio::test]
async fn task_runner_honors_pre_cancelled_token() {
    #[derive(Debug)]
    struct ReadOnlySkill;

    #[async_trait]
    impl Skill for ReadOnlySkill {
        fn name(&self) -> &'static str {
            "cancel_read"
        }

        fn description(&self) -> &'static str {
            "should not run"
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
        ) -> crate::Result<SkillOutput> {
            Ok(SkillOutput::new("unexpected", json!({})))
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let mut registry = SkillRegistry::new();
    registry.register(ReadOnlySkill);
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let report = session
        .execute_task_steps(
            &registry,
            "task-1",
            vec![ExecutablePlanStep::new(
                "cancelled step",
                "cancel_read",
                json!({}),
                RiskLevel::SafeRead,
            )],
            ExecutionOptions::default(),
            cancellation,
        )
        .await
        .expect("task runner");

    assert_eq!(report.state, TaskState::Cancelled);
    assert_eq!(report.steps[0].status, PlanStepStatus::Cancelled);
    assert_eq!(report.steps[0].attempts, 0);
}

#[tokio::test]
async fn task_runner_warns_on_repeated_exact_failure() {
    #[derive(Debug)]
    struct AlwaysFails;

    #[async_trait]
    impl Skill for AlwaysFails {
        fn name(&self) -> &'static str {
            "always_fails"
        }

        fn description(&self) -> &'static str {
            "fails every attempt"
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
        ) -> crate::Result<SkillOutput> {
            Err(IkarosError::Message("same failure".into()))
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let mut registry = SkillRegistry::new();
    registry.register(AlwaysFails);

    let report = session
        .execute_task_steps(
            &registry,
            "task-guardrail-warning",
            vec![ExecutablePlanStep::new(
                "warn on repeated failure",
                "always_fails",
                json!({}),
                RiskLevel::SafeRead,
            )],
            ExecutionOptions {
                timeout_ms: None,
                max_retries: 2,
                retry_delay_ms: 0,
                guardrails: GuardrailConfig {
                    exact_failure_warn_after: 2,
                    exact_failure_halt_after: 10,
                    same_tool_failure_warn_after: 10,
                    same_tool_failure_halt_after: 10,
                    ..GuardrailConfig::default()
                },
            },
            CancellationToken::new(),
        )
        .await
        .expect("task runner");

    assert_eq!(report.state, TaskState::Failed);
    assert_eq!(report.steps[0].attempts, 3);
    let events = session.audit.read_all().expect("audit");
    assert!(events.iter().any(|event| {
        event.kind == "task_guardrail_warning"
            && event
                .data
                .get("signal")
                .and_then(|signal| signal.get("kind"))
                .and_then(serde_json::Value::as_str)
                == Some("ExactFailure")
    }));
}

#[tokio::test]
async fn task_runner_halts_on_repeated_failure_when_hard_stop_enabled() {
    #[derive(Debug)]
    struct AlwaysFails;

    #[async_trait]
    impl Skill for AlwaysFails {
        fn name(&self) -> &'static str {
            "hard_stop_fail"
        }

        fn description(&self) -> &'static str {
            "fails every attempt"
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
        ) -> crate::Result<SkillOutput> {
            Err(IkarosError::Message("hard stop failure".into()))
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let mut registry = SkillRegistry::new();
    registry.register(AlwaysFails);

    let report = session
        .execute_task_steps(
            &registry,
            "task-guardrail-halt",
            vec![ExecutablePlanStep::new(
                "halt on repeated failure",
                "hard_stop_fail",
                json!({}),
                RiskLevel::SafeRead,
            )],
            ExecutionOptions {
                timeout_ms: None,
                max_retries: 5,
                retry_delay_ms: 0,
                guardrails: GuardrailConfig {
                    hard_stop_enabled: true,
                    exact_failure_warn_after: 10,
                    exact_failure_halt_after: 2,
                    same_tool_failure_warn_after: 10,
                    same_tool_failure_halt_after: 10,
                    ..GuardrailConfig::default()
                },
            },
            CancellationToken::new(),
        )
        .await
        .expect("task runner");

    assert_eq!(report.state, TaskState::Blocked);
    assert_eq!(report.steps[0].status, PlanStepStatus::Failed);
    assert_eq!(report.steps[0].attempts, 2);
    assert!(report.steps[0].summary.contains("guardrail halted"));
    let events = session.audit.read_all().expect("audit");
    assert!(events.iter().any(|event| {
        event.kind == "task_guardrail_halt"
            && event
                .data
                .get("signal")
                .and_then(|signal| signal.get("kind"))
                .and_then(serde_json::Value::as_str)
                == Some("ExactFailure")
    }));
}

#[tokio::test]
async fn task_runner_halts_on_repeated_no_progress_when_enabled() {
    #[derive(Debug)]
    struct NoProgressSkill;

    #[async_trait]
    impl Skill for NoProgressSkill {
        fn name(&self) -> &'static str {
            "no_progress"
        }

        fn description(&self) -> &'static str {
            "reports no progress"
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
        ) -> crate::Result<SkillOutput> {
            Ok(SkillOutput::new(
                "no useful change",
                json!({"progress": false}),
            ))
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let mut registry = SkillRegistry::new();
    registry.register(NoProgressSkill);

    let report = session
        .execute_task_steps(
            &registry,
            "task-no-progress-halt",
            vec![
                ExecutablePlanStep::new(
                    "first no progress",
                    "no_progress",
                    json!({}),
                    RiskLevel::SafeRead,
                ),
                ExecutablePlanStep::new(
                    "second no progress",
                    "no_progress",
                    json!({}),
                    RiskLevel::SafeRead,
                ),
            ],
            ExecutionOptions {
                timeout_ms: None,
                max_retries: 0,
                retry_delay_ms: 0,
                guardrails: GuardrailConfig {
                    hard_stop_enabled: true,
                    no_progress_warn_after: 10,
                    no_progress_halt_after: 2,
                    ..GuardrailConfig::default()
                },
            },
            CancellationToken::new(),
        )
        .await
        .expect("task runner");

    assert_eq!(report.state, TaskState::Blocked);
    assert_eq!(report.steps[0].status, PlanStepStatus::Succeeded);
    assert_eq!(report.steps[1].status, PlanStepStatus::Failed);
    assert!(report.steps[1].summary.contains("NoProgress"));
    let events = session.audit.read_all().expect("audit");
    assert!(events.iter().any(|event| {
        event.kind == "task_guardrail_halt"
            && event
                .data
                .get("signal")
                .and_then(|signal| signal.get("kind"))
                .and_then(serde_json::Value::as_str)
                == Some("NoProgress")
    }));
}
