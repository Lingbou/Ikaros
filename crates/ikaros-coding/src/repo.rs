// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoMap {
    pub root: PathBuf,
    pub files: Vec<RepoFile>,
    pub package_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoFile {
    pub path: PathBuf,
    pub kind: RepoFileKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RepoFileKind {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Markdown,
    Config,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangePlan {
    pub objective: String,
    pub steps: Vec<String>,
    pub requires_approval: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestCommand {
    pub command: String,
    pub reason: String,
}

pub struct RepoScanner {
    root: PathBuf,
}

impl RepoScanner {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn scan(&self) -> Result<RepoMap> {
        let mut files = Vec::new();
        collect_repo_files(&self.root, &mut files)?;
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let package_files = files
            .iter()
            .filter(|file| is_package_file(&file.path))
            .map(|file| file.path.clone())
            .collect();
        Ok(RepoMap {
            root: self.root.clone(),
            files,
            package_files,
        })
    }
}

pub struct ChangePlanner;

impl ChangePlanner {
    pub fn plan(objective: impl Into<String>, repo: &RepoMap) -> ChangePlan {
        let objective = objective.into();
        let mut steps = vec![
            "Build a repo map and identify the owning crate or module.".into(),
            "Apply edits through guarded filesystem operations.".into(),
            "Run focused tests first, then broader workspace checks when risk increases.".into(),
            "Summarize diff and residual risk without committing.".into(),
        ];
        if repo
            .package_files
            .iter()
            .any(|path| path.ends_with("Cargo.toml"))
        {
            steps.insert(
                2,
                "Use cargo fmt, clippy, and tests for Rust changes.".into(),
            );
        }
        ChangePlan {
            objective,
            steps,
            requires_approval: true,
        }
    }
}

pub struct TestRunnerPlan;

impl TestRunnerPlan {
    pub fn infer(repo: &RepoMap) -> Vec<TestCommand> {
        let mut commands = Vec::new();
        if repo
            .package_files
            .iter()
            .any(|path| path.ends_with("Cargo.toml"))
        {
            commands.push(TestCommand {
                command: "cargo fmt --all -- --check".into(),
                reason: "Rust formatting gate".into(),
            });
            commands.push(TestCommand {
                command: "cargo clippy --workspace --all-targets --all-features -- -D warnings"
                    .into(),
                reason: "Rust lint gate".into(),
            });
            commands.push(TestCommand {
                command: "cargo test --workspace --all-features".into(),
                reason: "Rust unit/integration tests".into(),
            });
        }
        commands
    }
}

fn collect_repo_files(root: &PathBuf, files: &mut Vec<RepoFile>) -> Result<()> {
    if !root.exists() {
        return Err(IkarosError::Message(format!(
            "repo root does not exist: {}",
            root.display()
        )));
    }
    for entry in fs::read_dir(root).map_err(|source| IkarosError::io(root, source))? {
        let entry = entry.map_err(|source| IkarosError::io(root, source))?;
        let path = entry.path();
        if should_skip(&path) {
            continue;
        }
        let metadata =
            fs::symlink_metadata(&path).map_err(|source| IkarosError::io(&path, source))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_repo_files(&path, files)?;
        } else if metadata.is_file() {
            files.push(RepoFile {
                kind: classify(&path),
                path,
            });
        }
    }
    Ok(())
}

fn should_skip(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name == ".git" || name == ".temp" || name == "target" || name == "node_modules"
        })
}

fn classify(path: &Path) -> RepoFileKind {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("rs") => RepoFileKind::Rust,
        Some("ts" | "tsx") => RepoFileKind::TypeScript,
        Some("js" | "jsx") => RepoFileKind::JavaScript,
        Some("py") => RepoFileKind::Python,
        Some("md") => RepoFileKind::Markdown,
        Some("toml" | "yaml" | "yml" | "json") => RepoFileKind::Config,
        _ => RepoFileKind::Other,
    }
}

fn is_package_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name,
                "Cargo.toml" | "package.json" | "pyproject.toml" | "pnpm-lock.yaml"
            )
        })
}
