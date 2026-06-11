// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result, redact_secrets};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TestFailureCategory {
    Passed,
    Format,
    Lint,
    Compile,
    TestFailure,
    RuntimePanic,
    CommandFailure,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestFailureAnalysis {
    pub command: String,
    pub status: i32,
    pub category: TestFailureCategory,
    pub summary: String,
    pub failed_tests: Vec<String>,
    pub likely_causes: Vec<String>,
    pub suggested_next_steps: Vec<String>,
}

pub struct TestFailureAnalyzer;

impl TestFailureAnalyzer {
    pub fn analyze(
        command: impl Into<String>,
        status: i32,
        stdout: &str,
        stderr: &str,
    ) -> TestFailureAnalysis {
        let command = command.into();
        let combined = redact_secrets(&format!("{stdout}\n{stderr}"));
        let lower_command = command.to_ascii_lowercase();
        let lower = combined.to_ascii_lowercase();
        let failed_tests = failed_tests_from_output(&combined);

        let category = if status == 0 {
            TestFailureCategory::Passed
        } else if lower_command.contains("fmt") || lower.contains("diff in ") {
            TestFailureCategory::Format
        } else if lower_command.contains("clippy") || lower.contains("clippy") {
            TestFailureCategory::Lint
        } else if lower.contains("error[e")
            || lower.contains("could not compile")
            || lower.contains("compilation failed")
        {
            TestFailureCategory::Compile
        } else if !failed_tests.is_empty()
            || lower.contains("test result: failed")
            || lower.contains("assertion failed")
        {
            TestFailureCategory::TestFailure
        } else if lower.contains("panicked at") || lower.contains("thread '") {
            TestFailureCategory::RuntimePanic
        } else {
            TestFailureCategory::CommandFailure
        };

        let (summary, likely_causes, suggested_next_steps) =
            failure_guidance(&category, status, &failed_tests);
        TestFailureAnalysis {
            command: redact_secrets(&command),
            status,
            category,
            summary,
            failed_tests,
            likely_causes,
            suggested_next_steps,
        }
    }
}

pub fn validate_test_command(command: &str) -> Result<()> {
    if is_allowed_test_command(command) {
        Ok(())
    } else {
        Err(IkarosError::Message(format!(
            "test command is outside the allowed test/check command set: {command}"
        )))
    }
}

pub fn is_allowed_test_command(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty()
        || [";", "&&", "||", "|", ">", "<", "`", "$(", "\n", "\r"]
            .iter()
            .any(|needle| trimmed.contains(needle))
    {
        return false;
    }

    let parts = trimmed.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        ["cargo", subcommand, ..] => matches!(
            *subcommand,
            "test" | "fmt" | "clippy" | "check" | "doc" | "nextest"
        ),
        ["npm", "test", ..]
        | ["npm", "run", "test", ..]
        | ["npm", "run", "lint", ..]
        | ["npm", "run", "build", ..]
        | ["pnpm", "test", ..]
        | ["pnpm", "run", "test", ..]
        | ["pnpm", "run", "lint", ..]
        | ["pnpm", "run", "build", ..]
        | ["yarn", "test", ..]
        | ["yarn", "run", "test", ..]
        | ["yarn", "run", "lint", ..]
        | ["yarn", "run", "build", ..]
        | ["pytest", ..]
        | ["python", "-m", "pytest", ..]
        | ["python3", "-m", "pytest", ..]
        | ["uv", "run", "pytest", ..] => true,
        _ => false,
    }
}

fn failed_tests_from_output(output: &str) -> Vec<String> {
    let mut tests = Vec::new();
    let mut in_failures = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed == "failures:" {
            in_failures = true;
            continue;
        }
        if let Some(test_name) = trimmed
            .strip_prefix("test ")
            .and_then(|rest| rest.strip_suffix(" ... FAILED"))
        {
            tests.push(redact_secrets(test_name.trim()));
            continue;
        }
        if in_failures {
            if trimmed.is_empty()
                || trimmed.starts_with("test result:")
                || trimmed.starts_with("failures:")
            {
                in_failures = false;
                continue;
            }
            if !trimmed.contains(' ') && !trimmed.contains(':') {
                tests.push(redact_secrets(trimmed));
            }
        }
    }
    tests.sort();
    tests.dedup();
    tests
}

fn failure_guidance(
    category: &TestFailureCategory,
    status: i32,
    failed_tests: &[String],
) -> (String, Vec<String>, Vec<String>) {
    match category {
        TestFailureCategory::Passed => (
            "test command passed".into(),
            Vec::new(),
            vec![
                "Summarize the test command and continue with broader checks if risk increased."
                    .into(),
            ],
        ),
        TestFailureCategory::Format => (
            format!("formatting check failed with status {status}"),
            vec!["Source formatting differs from the project formatter.".into()],
            vec![
                "Run the formatter command locally through the test runner when appropriate."
                    .into(),
                "Review the resulting diff before asking for guarded edit approval.".into(),
            ],
        ),
        TestFailureCategory::Lint => (
            format!("lint check failed with status {status}"),
            vec!["The linter rejected style, correctness, or warning-level issues.".into()],
            vec![
                "Read the lint diagnostic around the first error.".into(),
                "Prepare a focused guarded edit that addresses the reported lint.".into(),
            ],
        ),
        TestFailureCategory::Compile => (
            format!("compile check failed with status {status}"),
            vec!["The code did not compile or dependency resolution failed.".into()],
            vec![
                "Inspect the first compiler error before later cascading errors.".into(),
                "Patch the smallest owning module and rerun the same command.".into(),
            ],
        ),
        TestFailureCategory::TestFailure => (
            if failed_tests.is_empty() {
                format!("test assertion failed with status {status}")
            } else {
                format!(
                    "{} test(s) failed with status {status}: {}",
                    failed_tests.len(),
                    failed_tests.join(", ")
                )
            },
            vec![
                "A test assertion or expected behavior no longer matches runtime behavior.".into(),
            ],
            vec![
                "Inspect the failing test body and the changed code it exercises.".into(),
                "Prefer fixing behavior before updating expectations.".into(),
            ],
        ),
        TestFailureCategory::RuntimePanic => (
            format!("runtime panic detected with status {status}"),
            vec!["The test process panicked before completing normally.".into()],
            vec![
                "Inspect the panic location and backtrace hint.".into(),
                "Add a focused regression test if the panic reveals a missing invariant.".into(),
            ],
        ),
        TestFailureCategory::CommandFailure => (
            format!("test command failed with status {status}"),
            vec!["The command exited unsuccessfully without a recognized test category.".into()],
            vec![
                "Inspect stderr/stdout around the first error line.".into(),
                "Rerun a narrower command if the failure source is ambiguous.".into(),
            ],
        ),
    }
}
