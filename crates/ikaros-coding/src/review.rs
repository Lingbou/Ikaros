// SPDX-License-Identifier: GPL-3.0-only

use crate::{TestFailureCategory, patch::parse_diff_path, testing::TestFailureAnalysis};
use ikaros_core::{contains_secret_like, redact_secrets};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffSummary {
    pub files_changed: usize,
    pub insertions_hint: usize,
    pub deletions_hint: usize,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReviewSeverity {
    Info,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewFinding {
    pub severity: ReviewSeverity,
    pub title: String,
    pub detail: String,
    pub recommendation: String,
    pub file: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewReport {
    pub summary: String,
    pub diff_summary: DiffSummary,
    pub changed_files: Vec<PathBuf>,
    pub test_analysis: Option<TestFailureAnalysis>,
    pub findings: Vec<ReviewFinding>,
    pub suggested_next_steps: Vec<String>,
    pub markdown: String,
}

pub struct DiffSummarizer;

impl DiffSummarizer {
    pub fn summarize(diff: &str) -> DiffSummary {
        let mut files_changed = 0;
        let mut insertions_hint = 0;
        let mut deletions_hint = 0;
        for line in diff.lines() {
            if line.starts_with("diff --git ") {
                files_changed += 1;
            } else if line.starts_with('+') && !line.starts_with("+++") {
                insertions_hint += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                deletions_hint += 1;
            }
        }
        DiffSummary {
            files_changed,
            insertions_hint,
            deletions_hint,
            summary: format!(
                "{files_changed} files changed, about {insertions_hint} inserted lines and {deletions_hint} deleted lines"
            ),
        }
    }
}

pub struct CodeReviewAssistant;

impl CodeReviewAssistant {
    pub fn review(diff: &str, test_analysis: Option<TestFailureAnalysis>) -> ReviewReport {
        let diff = redact_secrets(diff);
        let diff_summary = DiffSummarizer::summarize(&diff);
        let changed_files = diff_file_paths(&diff);
        let added_lines = added_lines_by_file(&diff);
        let mut findings = Vec::new();

        if diff.trim().is_empty() || diff_summary.files_changed == 0 {
            findings.push(ReviewFinding {
                severity: ReviewSeverity::Info,
                title: "No diff detected".into(),
                detail: "The review input did not contain a unified diff.".into(),
                recommendation:
                    "Run `ikaros git diff` or pass `--diff` when reviewing pending changes.".into(),
                file: None,
            });
        }

        for (file, lines) in &added_lines {
            let joined = lines.join("\n");
            let lower = joined.to_ascii_lowercase();
            if contains_secret_like(&joined) {
                findings.push(ReviewFinding {
                    severity: ReviewSeverity::High,
                    title: "Secret-like content added".into(),
                    detail: "Added lines contain secret-like text after redaction.".into(),
                    recommendation:
                        "Remove credentials from the patch and route secrets through an adapter."
                            .into(),
                    file: Some(file.clone()),
                });
            }
            if lower.contains("unsafe") {
                findings.push(ReviewFinding {
                    severity: ReviewSeverity::High,
                    title: "Unsafe code added".into(),
                    detail: "The diff adds `unsafe` code.".into(),
                    recommendation:
                        "Justify the invariant, isolate the unsafe block, and add focused tests."
                            .into(),
                    file: Some(file.clone()),
                });
            }
            if lower.contains("todo!") || lower.contains("unimplemented!") {
                findings.push(ReviewFinding {
                    severity: ReviewSeverity::High,
                    title: "Placeholder runtime failure added".into(),
                    detail: "The diff adds `todo!` or `unimplemented!`.".into(),
                    recommendation: "Replace placeholders before merging the change.".into(),
                    file: Some(file.clone()),
                });
            }
            if joined.contains(".unwrap()")
                || joined.contains(".expect(")
                || joined.contains("panic!(")
            {
                findings.push(ReviewFinding {
                    severity: ReviewSeverity::Medium,
                    title: "Potential panic path added".into(),
                    detail: "The diff adds unwrap, expect, or panic usage.".into(),
                    recommendation:
                        "Prefer error propagation or document why the panic cannot occur.".into(),
                    file: Some(file.clone()),
                });
            }
            if joined.contains("dbg!(") || joined.contains("println!(") {
                findings.push(ReviewFinding {
                    severity: ReviewSeverity::Low,
                    title: "Debug output added".into(),
                    detail: "The diff adds debug or direct stdout output.".into(),
                    recommendation:
                        "Keep intentional CLI output, but remove temporary debugging before review."
                            .into(),
                    file: Some(file.clone()),
                });
            }
        }

        match &test_analysis {
            Some(analysis) if analysis.category != TestFailureCategory::Passed => {
                findings.push(ReviewFinding {
                    severity: ReviewSeverity::High,
                    title: "Tests are not passing".into(),
                    detail: analysis.summary.clone(),
                    recommendation:
                        "Resolve the failing test analysis before considering the change ready."
                            .into(),
                    file: None,
                });
            }
            None if diff_summary.files_changed > 0 => findings.push(ReviewFinding {
                severity: ReviewSeverity::Low,
                title: "No test analysis provided".into(),
                detail: "The review did not include a latest test result analysis.".into(),
                recommendation: "Run a focused `ikaros test run` command and include its analysis."
                    .into(),
                file: None,
            }),
            _ => {}
        }

        if findings.is_empty() {
            findings.push(ReviewFinding {
                severity: ReviewSeverity::Info,
                title: "No heuristic issues found".into(),
                detail: "The review assistant did not detect obvious diff or test risks.".into(),
                recommendation: "Proceed with human review of behavior and edge cases.".into(),
                file: None,
            });
        }

        let summary = review_summary(&diff_summary, &findings);
        let suggested_next_steps = review_next_steps(&findings);
        let markdown =
            render_review_markdown(&summary, &diff_summary, &findings, &suggested_next_steps);
        ReviewReport {
            summary,
            diff_summary,
            changed_files,
            test_analysis,
            findings,
            suggested_next_steps,
            markdown,
        }
    }
}

fn diff_file_paths(diff: &str) -> Vec<PathBuf> {
    let mut paths = diff
        .lines()
        .filter_map(|line| line.strip_prefix("+++ "))
        .filter_map(|raw| parse_diff_path(raw).ok())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn added_lines_by_file(diff: &str) -> Vec<(PathBuf, Vec<String>)> {
    let mut current = None::<PathBuf>;
    let mut files = Vec::<(PathBuf, Vec<String>)>::new();
    for line in diff.lines() {
        if let Some(raw) = line.strip_prefix("+++ ") {
            current = parse_diff_path(raw).ok();
            continue;
        }
        if line.starts_with("diff --git ") {
            current = None;
            continue;
        }
        if line.starts_with('+') && !line.starts_with("+++") {
            if let Some(path) = &current {
                if let Some((_, lines)) = files.iter_mut().find(|(candidate, _)| candidate == path)
                {
                    lines.push(redact_secrets(&line[1..]));
                } else {
                    files.push((path.clone(), vec![redact_secrets(&line[1..])]));
                }
            }
        }
    }
    files
}

fn review_summary(diff_summary: &DiffSummary, findings: &[ReviewFinding]) -> String {
    let high = findings
        .iter()
        .filter(|finding| finding.severity == ReviewSeverity::High)
        .count();
    let medium = findings
        .iter()
        .filter(|finding| finding.severity == ReviewSeverity::Medium)
        .count();
    format!(
        "{}; {} finding(s), {high} high, {medium} medium",
        diff_summary.summary,
        findings.len()
    )
}

fn review_next_steps(findings: &[ReviewFinding]) -> Vec<String> {
    if findings
        .iter()
        .any(|finding| finding.severity == ReviewSeverity::High)
    {
        return vec![
            "Address high-severity findings before approval or merge.".into(),
            "Rerun focused tests and regenerate the review report.".into(),
        ];
    }
    if findings
        .iter()
        .any(|finding| finding.severity == ReviewSeverity::Medium)
    {
        return vec![
            "Review medium-severity findings and decide whether a guarded edit is needed.".into(),
            "Run the narrowest relevant tests before broad workspace checks.".into(),
        ];
    }
    vec![
        "Confirm behavior manually where tests are indirect.".into(),
        "Summarize residual risk without committing.".into(),
    ]
}

fn render_review_markdown(
    summary: &str,
    diff_summary: &DiffSummary,
    findings: &[ReviewFinding],
    next_steps: &[String],
) -> String {
    let mut markdown = String::new();
    markdown.push_str("# Review Notes\n\n");
    markdown.push_str(&format!("Summary: {}\n\n", redact_secrets(summary)));
    markdown.push_str(&format!(
        "Diff: {}\n\n",
        redact_secrets(&diff_summary.summary)
    ));
    markdown.push_str("## Findings\n\n");
    for finding in findings {
        let file = finding
            .file
            .as_ref()
            .map(|path| format!(" ({})", path.display()))
            .unwrap_or_default();
        markdown.push_str(&format!(
            "- [{:?}] {}{}: {} Recommendation: {}\n",
            finding.severity,
            redact_secrets(&finding.title),
            file,
            redact_secrets(&finding.detail),
            redact_secrets(&finding.recommendation),
        ));
    }
    markdown.push_str("\n## Next Steps\n\n");
    for step in next_steps {
        markdown.push_str(&format!("- {}\n", redact_secrets(step)));
    }
    markdown
}
