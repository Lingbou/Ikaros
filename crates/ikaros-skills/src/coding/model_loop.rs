// SPDX-License-Identifier: GPL-3.0-only

use super::{bounded_redacted_text, primary_test_analysis, run_coding_test_matrix};
use ikaros_coding::{
    ChangePlan, ChangePlanner, CodeReviewAssistant, CodingLoopReport, CodingLoopStatus,
    CodingTurnContext, CodingTurnDiffReport, CodingTurnEvent, CodingTurnEventKind, CodingTurnInput,
    CodingTurnReport, GuardedPatchApplier, PatchApplyReport, PatchFailure, PatchIterationPlan,
    PatchIterationPlanner, RepoMap, RepoScanner, ReviewReport, TestCommand, TestFailureAnalysis,
    TestRunnerPlan, TurnDiffTracker,
};
use ikaros_core::{IkarosError, Result, redact_secrets};
use ikaros_harness::CancellationToken;
use ikaros_models::{ModelMessage, ModelProvider, ModelRequest, ModelRequestOptions};
use ikaros_toolkit::SkillContext;
use serde::Deserialize;
use serde_json::json;
use std::{path::Path, sync::Arc};

pub(super) async fn run_provider_coding_loop(
    input: CodingTurnInput,
    provider: Arc<dyn ModelProvider>,
    ctx: &SkillContext,
    runs_tests: bool,
    max_iterations: usize,
    model_token_budget: Option<u32>,
    cancellation: CancellationToken,
) -> Result<CodingTurnReport> {
    let max_iterations = max_iterations.clamp(1, 8);
    let mut events = Vec::new();
    events.push(CodingTurnEvent::new(
        CodingTurnEventKind::ContextPrepared,
        format!(
            "provider coding context prepared for {:?} mode",
            input.context.mode
        ),
        json!({
            "workspace_root": input.context.workspace_root,
            "session_id": input.context.session_id,
            "turn_id": input.context.turn_id,
            "instruction_count": input.context.instructions.len(),
            "git": input.context.git,
        }),
    ));
    events.push(CodingTurnEvent::new(
        CodingTurnEventKind::GitBaselineCaptured,
        "git baseline captured for provider coding turn",
        json!({
            "git_root": input.context.git.git_root,
            "head": input.context.git.head,
            "branch": input.context.git.branch,
            "detached": input.context.git.detached,
            "dirty": input.context.git.dirty,
            "has_staged_changes": input.context.git.has_staged_changes,
            "has_unstaged_changes": input.context.git.has_unstaged_changes,
            "has_untracked_files": input.context.git.has_untracked_files,
        }),
    ));

    let repo = RepoScanner::new(&input.context.workspace_root).scan()?;
    events.push(CodingTurnEvent::new(
        CodingTurnEventKind::RepoScanned,
        format!(
            "repo scanned: {} file(s), {} package file(s)",
            repo.files.len(),
            repo.package_files.len()
        ),
        json!({
            "files": repo.files.len(),
            "package_files": repo.package_files.len(),
        }),
    ));
    let change_plan = ChangePlanner::plan(input.context.objective.clone(), &repo);
    events.push(CodingTurnEvent::new(
        CodingTurnEventKind::PlanPrepared,
        format!(
            "change plan prepared with {} step(s)",
            change_plan.steps.len()
        ),
        json!({"step_count": change_plan.steps.len()}),
    ));

    let workspace_excerpts = collect_workspace_excerpts(&repo, ctx).await?;
    let mut tracker = TurnDiffTracker::new(input.context.workspace_root.clone());
    let mut patch_apply_report = None;
    let mut patch_failure = None;
    let mut all_tests = input.test_matrix.clone();
    let mut latest_test = primary_test_analysis(&all_tests).or(input.test_analysis.clone());
    let mut latest_review = CodeReviewAssistant::review(
        input.candidate_diff.as_deref().unwrap_or_default(),
        latest_test.clone(),
    );
    let mut latest_iteration =
        PatchIterationPlanner::plan(&input.context.objective, &latest_review, &repo);
    let mut latest_model_answer = None;
    let mut loop_status = CodingLoopStatus::AwaitingFollowUpPatch;
    let mut loop_reason = "provider coding loop did not produce a passing patch".to_owned();
    let mut iterations = 0usize;
    let mut consumed_tokens = 0u32;

    for iteration in 1..=max_iterations {
        iterations = iteration;
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::LoopIterationStarted,
            format!("provider coding loop iteration {iteration} started"),
            json!({
                "iteration": iteration,
                "max_iterations": max_iterations,
                "mode": input.context.mode,
            }),
        ));
        if cancellation.is_cancelled() {
            loop_status = CodingLoopStatus::Cancelled;
            loop_reason = format!("provider coding loop cancelled before iteration {iteration}");
            events.push(CodingTurnEvent::new(
                CodingTurnEventKind::CodingLoopCancelled,
                loop_reason.clone(),
                json!({
                    "iteration": iteration,
                    "phase": "before_model_request",
                }),
            ));
            break;
        }

        let current_diff = tracker
            .unified_diff()
            .or_else(|| input.candidate_diff.clone())
            .unwrap_or_default();
        let prompt = build_provider_coding_prompt(ProviderCodingPromptInput {
            context: &input.context,
            repo: &repo,
            change_plan: &change_plan,
            workspace_excerpts: &workspace_excerpts,
            current_diff: &current_diff,
            latest_test: latest_test.as_ref(),
            latest_review: &latest_review,
            iteration,
            max_iterations,
        })?;
        let request = ModelRequest {
            messages: vec![
                ModelMessage::system(
                    "You are the Ikaros coding runtime. Return only strict JSON with keys candidate_diff, final_answer, and stop. candidate_diff must be a unified diff applicable from the current workspace state. Do not use markdown fences, do not commit, do not access files outside the workspace, and do not reveal secrets.",
                ),
                ModelMessage::user(prompt.clone()),
            ],
            options: ModelRequestOptions {
                max_tokens: Some(2_000),
                temperature: Some(0.2),
                ..ModelRequestOptions::default()
            },
            tools: Vec::new(),
        };
        let estimated_tokens = provider.estimate_request_tokens(&request);
        if model_token_budget
            .is_some_and(|budget| consumed_tokens.saturating_add(estimated_tokens) > budget)
        {
            loop_status = CodingLoopStatus::BudgetExceeded;
            loop_reason =
                format!("provider coding loop token budget exceeded before iteration {iteration}");
            events.push(CodingTurnEvent::new(
                CodingTurnEventKind::ModelBudgetExceeded,
                loop_reason.clone(),
                json!({
                    "iteration": iteration,
                    "estimated_request_tokens": estimated_tokens,
                    "consumed_tokens": consumed_tokens,
                    "budget": model_token_budget,
                }),
            ));
            break;
        }
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::ModelRequestPrepared,
            format!("provider coding request prepared for iteration {iteration}"),
            json!({
                "iteration": iteration,
                "provider": provider.name(),
                "estimated_request_tokens": estimated_tokens,
                "consumed_tokens_before_request": consumed_tokens,
                "budget": model_token_budget,
                "prompt_chars": prompt.chars().count(),
                "instruction_count": input.context.instructions.len(),
                "workspace_excerpt_count": workspace_excerpts.len(),
            }),
        ));

        let response = tokio::select! {
            response = provider.generate(request) => response?,
            _ = cancellation.cancelled() => {
                loop_status = CodingLoopStatus::Cancelled;
                loop_reason =
                    format!("provider coding loop cancelled while awaiting iteration {iteration} response");
                events.push(CodingTurnEvent::new(
                    CodingTurnEventKind::CodingLoopCancelled,
                    loop_reason.clone(),
                    json!({
                        "iteration": iteration,
                        "phase": "awaiting_model_response",
                    }),
                ));
                break;
            }
        };
        let response_tokens = response.usage.total_or_prompt_completion();
        consumed_tokens = consumed_tokens
            .saturating_add(estimated_tokens)
            .saturating_add(response_tokens);
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::ModelResponseReceived,
            format!("provider coding response received for iteration {iteration}"),
            json!({
                "iteration": iteration,
                "provider": response.provider,
                "model": response.model,
                "usage": response.usage,
                "diagnostics": response.diagnostics,
                "content_chars": response.content.chars().count(),
                "consumed_tokens": consumed_tokens,
            }),
        ));

        let action = match parse_provider_coding_action(&response.content) {
            Ok(action) => action,
            Err(error) => {
                loop_status = CodingLoopStatus::ReviewBlocked;
                loop_reason = format!("provider returned invalid coding JSON: {error}");
                events.push(CodingTurnEvent::new(
                    CodingTurnEventKind::ModelResponseInvalid,
                    loop_reason.clone(),
                    json!({
                        "iteration": iteration,
                        "error": error.to_string(),
                        "content_excerpt": bounded_redacted_text(&response.content, 1200),
                    }),
                ));
                break;
            }
        };
        if cancellation.is_cancelled() {
            loop_status = CodingLoopStatus::Cancelled;
            loop_reason =
                format!("provider coding loop cancelled after iteration {iteration} response");
            events.push(CodingTurnEvent::new(
                CodingTurnEventKind::CodingLoopCancelled,
                loop_reason.clone(),
                json!({
                    "iteration": iteration,
                    "phase": "after_model_response",
                }),
            ));
            break;
        }
        latest_model_answer = action.final_answer.as_deref().map(redact_secrets);

        let candidate_diff = action
            .candidate_diff
            .as_deref()
            .filter(|diff| !diff.trim().is_empty());
        match candidate_diff {
            Some(diff) if input.apply_patch => {
                match GuardedPatchApplier::apply_unified_diff_with_env_checked(
                    &input.context.workspace_root,
                    diff,
                    ctx.session.env.as_ref(),
                )
                .await
                {
                    Ok(report) => {
                        tracker.track_patch_report(&report)?;
                        events.push(CodingTurnEvent::new(
                            CodingTurnEventKind::PatchApplied,
                            format!(
                                "provider patch applied to {} file(s) in iteration {iteration}",
                                report.files_changed
                            ),
                            json!({
                                "iteration": iteration,
                                "files_changed": report.files_changed,
                                "files_created": report.files_created,
                                "files_deleted": report.files_deleted,
                                "files_moved": report.files_moved,
                                "hunks_applied": report.hunks_applied,
                            }),
                        ));
                        if let Some(unified_diff) = tracker.unified_diff() {
                            events.push(CodingTurnEvent::new(
                                CodingTurnEventKind::DiffUpdated,
                                format!("turn diff updated after provider iteration {iteration}"),
                                json!({
                                    "iteration": iteration,
                                    "summary": tracker.summary(),
                                    "unified_diff": unified_diff,
                                }),
                            ));
                        }
                        patch_apply_report = Some(report);
                    }
                    Err(failure) => {
                        events.push(CodingTurnEvent::new(
                            CodingTurnEventKind::PatchFailed,
                            format!(
                                "provider patch failed in iteration {iteration}: {:?}",
                                failure.kind
                            ),
                            json!({
                                "iteration": iteration,
                                "kind": failure.kind,
                                "path": failure.path,
                                "message": failure.message,
                            }),
                        ));
                        loop_status = CodingLoopStatus::PatchFailed;
                        loop_reason = format!(
                            "provider patch failed in iteration {iteration} with {:?}",
                            failure.kind
                        );
                        patch_failure = Some(failure);
                        break;
                    }
                }
            }
            Some(_) => events.push(CodingTurnEvent::new(
                CodingTurnEventKind::PatchSkipped,
                format!("provider patch retained for review in iteration {iteration}"),
                json!({
                    "iteration": iteration,
                    "apply_patch": input.apply_patch,
                    "diff_chars": candidate_diff.map(|diff| diff.chars().count()).unwrap_or_default(),
                }),
            )),
            None => events.push(CodingTurnEvent::new(
                CodingTurnEventKind::PatchSkipped,
                format!("provider did not return a patch in iteration {iteration}"),
                json!({
                    "iteration": iteration,
                    "stop": action.stop.unwrap_or(false),
                }),
            )),
        }

        let iteration_tests = if runs_tests {
            run_coding_test_matrix(&input.context.test_commands, ctx).await?
        } else {
            Vec::new()
        };
        for test_analysis in &iteration_tests {
            events.push(CodingTurnEvent::new(
                CodingTurnEventKind::TestEvidenceRecorded,
                test_analysis.summary.clone(),
                json!({
                    "iteration": iteration,
                    "command": test_analysis.command,
                    "status": test_analysis.status,
                    "category": test_analysis.category,
                    "failed_tests": test_analysis.failed_tests,
                }),
            ));
        }
        let iteration_failed = iteration_tests.iter().any(|analysis| analysis.status != 0);
        if !iteration_tests.is_empty() {
            latest_test = primary_test_analysis(&iteration_tests);
            all_tests.extend(iteration_tests);
        }

        let review_diff = tracker
            .unified_diff()
            .or_else(|| candidate_diff.map(ToOwned::to_owned))
            .unwrap_or_default();
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::ReviewStarted,
            format!("provider coding review started for iteration {iteration}"),
            json!({
                "iteration": iteration,
                "diff_chars": review_diff.chars().count(),
                "has_test_analysis": latest_test.is_some(),
            }),
        ));
        latest_review = CodeReviewAssistant::review(&review_diff, latest_test.clone());
        for finding in &latest_review.findings {
            events.push(CodingTurnEvent::new(
                CodingTurnEventKind::ReviewFinding,
                finding.title.clone(),
                json!({
                    "iteration": iteration,
                    "severity": finding.severity.clone(),
                    "title": finding.title.clone(),
                    "detail": finding.detail.clone(),
                    "recommendation": finding.recommendation.clone(),
                    "file": finding.file.clone(),
                }),
            ));
        }
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::ReviewCompleted,
            latest_review.summary.clone(),
            json!({
                "iteration": iteration,
                "finding_count": latest_review.findings.len(),
                "changed_files": latest_review.changed_files,
            }),
        ));

        latest_iteration =
            PatchIterationPlanner::plan(&input.context.objective, &latest_review, &repo);
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::IterationPlanned,
            latest_iteration.guarded_edit_objective.clone(),
            json!({
                "iteration": iteration,
                "priority": latest_iteration.priority,
                "requires_guarded_edit": latest_iteration.requires_guarded_edit,
                "ready_for_approval": latest_iteration.ready_for_approval,
            }),
        ));

        if runs_tests && iteration_failed {
            loop_status = CodingLoopStatus::AwaitingFollowUpPatch;
            loop_reason =
                "test evidence still has failures; provider should generate a follow-up patch"
                    .into();
            continue;
        }
        if patch_apply_report.is_some() && (!runs_tests || !iteration_failed) {
            loop_status = CodingLoopStatus::Passed;
            loop_reason = format!("provider patch/test loop passed after {iteration} iteration(s)");
            break;
        }
        if action.stop.unwrap_or(false) {
            let provider_completed_plan =
                candidate_diff.is_none() && patch_apply_report.is_none() && !runs_tests;
            loop_status = if provider_completed_plan || latest_review.findings.is_empty() {
                CodingLoopStatus::Passed
            } else {
                CodingLoopStatus::ReviewBlocked
            };
            loop_reason = latest_model_answer
                .clone()
                .unwrap_or_else(|| "provider stopped coding loop".into());
            break;
        }
    }

    if iterations >= max_iterations && loop_status == CodingLoopStatus::AwaitingFollowUpPatch {
        loop_reason =
            format!("provider coding loop reached max_iterations={max_iterations} without passing");
    }
    let loop_report = CodingLoopReport {
        status: loop_status,
        iterations,
        max_iterations,
        reason: loop_reason,
    };
    events.push(CodingTurnEvent::new(
        CodingTurnEventKind::LoopTerminated,
        loop_report.reason.clone(),
        json!({
            "status": loop_report.status,
            "iterations": loop_report.iterations,
            "max_iterations": loop_report.max_iterations,
            "reason": loop_report.reason,
            "consumed_tokens": consumed_tokens,
            "budget": model_token_budget,
        }),
    ));

    let turn_diff = CodingTurnDiffReport {
        summary: tracker.summary(),
        unified_diff: tracker.unified_diff(),
    };
    let suggested_tests = if input.context.test_commands.is_empty() {
        TestRunnerPlan::infer(&repo)
    } else {
        input.context.test_commands.clone()
    };
    let final_report = render_provider_coding_turn_report(ProviderCodingReportInput {
        context: &input.context,
        change_plan: &change_plan,
        patch_apply_report: patch_apply_report.as_ref(),
        patch_failure: patch_failure.as_ref(),
        turn_diff: &turn_diff,
        review: &latest_review,
        iteration: &latest_iteration,
        loop_report: &loop_report,
        suggested_tests: &suggested_tests,
        model_answer: latest_model_answer.as_deref(),
        consumed_tokens,
    });
    events.push(CodingTurnEvent::new(
        CodingTurnEventKind::FinalReportPrepared,
        "provider coding turn report prepared",
        json!({
            "event_count": events.len() + 1,
            "has_patch": patch_apply_report.is_some(),
            "has_patch_failure": patch_failure.is_some(),
            "has_turn_diff": turn_diff.unified_diff.is_some(),
            "consumed_tokens": consumed_tokens,
        }),
    ));
    let test_analysis = primary_test_analysis(&all_tests);
    Ok(CodingTurnReport {
        context: input.context,
        repo,
        change_plan,
        patch_apply_report,
        patch_failure,
        turn_diff,
        suggested_tests,
        test_matrix: all_tests,
        test_analysis,
        review: latest_review,
        iteration: latest_iteration,
        loop_report,
        events,
        final_report,
    })
}

#[derive(Debug, Deserialize)]
struct ProviderCodingAction {
    #[serde(default)]
    candidate_diff: Option<String>,
    #[serde(default)]
    final_answer: Option<String>,
    #[serde(default)]
    stop: Option<bool>,
}

fn parse_provider_coding_action(content: &str) -> Result<ProviderCodingAction> {
    serde_json::from_str::<ProviderCodingAction>(content).map_err(|source| {
        IkarosError::Message(format!(
            "provider coding response must be strict JSON: {source}"
        ))
    })
}

struct ProviderCodingPromptInput<'a> {
    context: &'a CodingTurnContext,
    repo: &'a RepoMap,
    change_plan: &'a ChangePlan,
    workspace_excerpts: &'a [WorkspaceExcerpt],
    current_diff: &'a str,
    latest_test: Option<&'a TestFailureAnalysis>,
    latest_review: &'a ReviewReport,
    iteration: usize,
    max_iterations: usize,
}

fn build_provider_coding_prompt(input: ProviderCodingPromptInput<'_>) -> Result<String> {
    let repo_files = input
        .repo
        .files
        .iter()
        .take(120)
        .map(|file| {
            let path = file
                .path
                .strip_prefix(&input.repo.root)
                .unwrap_or(&file.path);
            format!("- {} ({:?})", path.display(), file.kind)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let instructions = if input.context.instructions.is_empty() {
        "none".to_owned()
    } else {
        input
            .context
            .instructions
            .iter()
            .map(|instruction| format!("- {}", bounded_redacted_text(instruction, 2000)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let excerpts = if input.workspace_excerpts.is_empty() {
        "none".to_owned()
    } else {
        input
            .workspace_excerpts
            .iter()
            .map(|excerpt| {
                format!(
                    "### {}\n```text\n{}\n```",
                    excerpt.path.display(),
                    bounded_redacted_text(&excerpt.content, 4000)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    let test = input
        .latest_test
        .map(serde_json::to_string_pretty)
        .transpose()?
        .unwrap_or_else(|| "none".into());
    let review = serde_json::to_string_pretty(input.latest_review)?;
    let plan = serde_json::to_string_pretty(input.change_plan)?;
    Ok(redact_secrets(&format!(
        "Objective:\n{}\n\nIteration: {}/{}\n\nMode: {:?}\n\nInstructions:\n{}\n\nChange plan:\n{}\n\nRepo files:\n{}\n\nWorkspace excerpts:\n{}\n\nCurrent turn diff:\n```diff\n{}\n```\n\nLatest test evidence:\n{}\n\nLatest heuristic review:\n{}\n\nReturn strict JSON only:\n{{\"candidate_diff\":\"<unified diff or null>\",\"final_answer\":\"<short explanation>\",\"stop\":false}}\n",
        input.context.objective,
        input.iteration,
        input.max_iterations,
        input.context.mode,
        instructions,
        plan,
        repo_files,
        excerpts,
        bounded_redacted_text(input.current_diff, 12_000),
        bounded_redacted_text(&test, 4000),
        bounded_redacted_text(&review, 4000),
    )))
}

#[derive(Debug, Clone)]
struct WorkspaceExcerpt {
    path: std::path::PathBuf,
    content: String,
}

async fn collect_workspace_excerpts(
    repo: &RepoMap,
    ctx: &SkillContext,
) -> Result<Vec<WorkspaceExcerpt>> {
    let mut excerpts = Vec::new();
    let mut total_chars = 0usize;
    for file in &repo.files {
        if excerpts.len() >= 8 || total_chars >= 20_000 {
            break;
        }
        if !is_prompt_excerpt_candidate(&file.path) {
            continue;
        }
        let Ok(content) = ctx.session.env.read_to_string(&file.path).await else {
            continue;
        };
        if content.trim().is_empty() {
            continue;
        }
        let relative = file
            .path
            .strip_prefix(&repo.root)
            .unwrap_or(&file.path)
            .to_path_buf();
        let bounded = bounded_redacted_text(&content, 4000);
        total_chars = total_chars.saturating_add(bounded.chars().count());
        excerpts.push(WorkspaceExcerpt {
            path: relative,
            content: bounded,
        });
    }
    Ok(excerpts)
}

fn is_prompt_excerpt_candidate(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "md" | "toml" | "yaml" | "yml" | "json")
    )
}

pub(super) async fn load_workspace_coding_instructions(ctx: &SkillContext) -> Result<Vec<String>> {
    let mut instructions = Vec::new();
    for relative in ["IKAROS.md", ".ikaros/instructions.md"] {
        let path = ctx.session.sandbox.workspace_root.join(relative);
        if let Ok(content) = ctx.session.env.read_to_string(&path).await {
            if !content.trim().is_empty() {
                instructions.push(format!(
                    "{relative}:\n{}",
                    bounded_redacted_text(&content, 6000)
                ));
            }
        }
    }
    Ok(instructions)
}

struct ProviderCodingReportInput<'a> {
    context: &'a CodingTurnContext,
    change_plan: &'a ChangePlan,
    patch_apply_report: Option<&'a PatchApplyReport>,
    patch_failure: Option<&'a PatchFailure>,
    turn_diff: &'a CodingTurnDiffReport,
    review: &'a ReviewReport,
    iteration: &'a PatchIterationPlan,
    loop_report: &'a CodingLoopReport,
    suggested_tests: &'a [TestCommand],
    model_answer: Option<&'a str>,
    consumed_tokens: u32,
}

fn render_provider_coding_turn_report(input: ProviderCodingReportInput<'_>) -> String {
    let mut report = String::new();
    report.push_str("# Provider Coding Turn Report\n\n");
    report.push_str(&format!(
        "Objective: {}\n\n",
        redact_secrets(&input.context.objective)
    ));
    report.push_str(&format!("Mode: {:?}\n\n", input.context.mode));
    report.push_str("## Model\n\n");
    report.push_str(&format!("- Consumed tokens: {}\n", input.consumed_tokens));
    if let Some(answer) = input.model_answer {
        report.push_str(&format!("- Final answer: {}\n", redact_secrets(answer)));
    }
    report.push_str("\n## Plan\n\n");
    for step in &input.change_plan.steps {
        report.push_str(&format!("- {}\n", redact_secrets(step)));
    }
    report.push_str("\n## Patch\n\n");
    match input.patch_apply_report {
        Some(patch) => report.push_str(&format!(
            "- Applied: {} file(s), {} hunk(s), {} insertion(s), {} deletion(s)\n",
            patch.files_changed, patch.hunks_applied, patch.insertions, patch.deletions
        )),
        None => report.push_str("- Applied: false\n"),
    }
    if let Some(failure) = input.patch_failure {
        report.push_str(&format!(
            "- Failure: {:?}: {}\n",
            failure.kind,
            redact_secrets(&failure.message)
        ));
    }
    report.push_str(&format!(
        "- Turn diff available: {}\n",
        input.turn_diff.unified_diff.is_some()
    ));
    report.push_str("\n## Tests\n\n");
    if input.suggested_tests.is_empty() {
        report.push_str("- none inferred\n");
    } else {
        for command in input.suggested_tests {
            report.push_str(&format!(
                "- `{}`: {}\n",
                redact_secrets(&command.command),
                redact_secrets(&command.reason)
            ));
        }
    }
    report.push_str("\n## Review\n\n");
    report.push_str(&format!("- {}\n", redact_secrets(&input.review.summary)));
    report.push_str("\n## Next Iteration\n\n");
    report.push_str(&format!(
        "- Requires guarded edit: {}\n",
        input.iteration.requires_guarded_edit
    ));
    report.push_str(&format!(
        "- Ready for approval: {}\n",
        input.iteration.ready_for_approval
    ));
    report.push_str("\n## Loop\n\n");
    report.push_str(&format!("- Status: {:?}\n", input.loop_report.status));
    report.push_str(&format!(
        "- Reason: {}\n",
        redact_secrets(&input.loop_report.reason)
    ));
    report
}
