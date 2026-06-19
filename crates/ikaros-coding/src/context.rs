// SPDX-License-Identifier: GPL-3.0-only

use crate::TestCommand;
use ikaros_core::{AgentPermission, IkarosError, Result, redact_secrets};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodingMode {
    #[default]
    Plan,
    Edit,
    Review,
    Test,
    SelfModify,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodingPermissionProfile {
    pub workspace_writes: AgentPermission,
    pub shell: AgentPermission,
    pub network: AgentPermission,
}

impl Default for CodingPermissionProfile {
    fn default() -> Self {
        Self {
            workspace_writes: AgentPermission::Ask,
            shell: AgentPermission::Ask,
            network: AgentPermission::Deny,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodingModeCapabilities {
    pub can_read_repo: bool,
    pub can_apply_patch: bool,
    pub can_run_tests: bool,
    pub can_use_network: bool,
    pub can_self_modify: bool,
    pub requires_self_modify_boundary: bool,
}

impl CodingModeCapabilities {
    pub fn for_mode(mode: CodingMode) -> Self {
        match mode {
            CodingMode::Plan => Self {
                can_read_repo: true,
                can_apply_patch: false,
                can_run_tests: false,
                can_use_network: false,
                can_self_modify: false,
                requires_self_modify_boundary: false,
            },
            CodingMode::Review => Self {
                can_read_repo: true,
                can_apply_patch: false,
                can_run_tests: false,
                can_use_network: false,
                can_self_modify: false,
                requires_self_modify_boundary: false,
            },
            CodingMode::Test => Self {
                can_read_repo: true,
                can_apply_patch: false,
                can_run_tests: true,
                can_use_network: false,
                can_self_modify: false,
                requires_self_modify_boundary: false,
            },
            CodingMode::Edit => Self {
                can_read_repo: true,
                can_apply_patch: true,
                can_run_tests: true,
                can_use_network: false,
                can_self_modify: false,
                requires_self_modify_boundary: false,
            },
            CodingMode::SelfModify => Self {
                can_read_repo: true,
                can_apply_patch: false,
                can_run_tests: false,
                can_use_network: false,
                can_self_modify: true,
                requires_self_modify_boundary: true,
            },
        }
    }

    pub fn validate_request(&self, apply_patch: bool, run_tests: bool) -> Result<()> {
        if self.requires_self_modify_boundary {
            return Err(IkarosError::Message(
                "self_modify mode requires the dedicated self-modify approval path".into(),
            ));
        }
        if apply_patch && !self.can_apply_patch {
            return Err(IkarosError::Message(
                "coding mode does not allow patch application".into(),
            ));
        }
        if run_tests && !self.can_run_tests {
            return Err(IkarosError::Message(
                "coding mode does not allow running test commands".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodingTurnContextInput {
    pub workspace_root: PathBuf,
    pub objective: String,
    pub mode: CodingMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instructions: Vec<String>,
    pub permission_profile: CodingPermissionProfile,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub test_commands: Vec<TestCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodingTurnContext {
    pub workspace_root: PathBuf,
    pub objective: String,
    pub mode: CodingMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instructions: Vec<String>,
    pub permission_profile: CodingPermissionProfile,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub test_commands: Vec<TestCommand>,
    pub git: CodingGitState,
}

impl CodingTurnContext {
    pub fn from_workspace(input: CodingTurnContextInput) -> Result<Self> {
        let workspace_root = canonical_workspace_root(&input.workspace_root)?;
        let git = CodingGitState::from_workspace(&workspace_root)?;
        Ok(Self {
            workspace_root,
            objective: redact_secrets(&input.objective),
            mode: input.mode,
            session_id: input.session_id.map(|value| redact_secrets(&value)),
            turn_id: input.turn_id.map(|value| redact_secrets(&value)),
            instructions: input
                .instructions
                .into_iter()
                .map(|value| redact_secrets(&value))
                .collect(),
            permission_profile: input.permission_profile,
            test_commands: input.test_commands,
            git,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodingGitState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default)]
    pub detached: bool,
    pub dirty: CodingDirtyState,
    #[serde(default)]
    pub has_staged_changes: bool,
    #[serde(default)]
    pub has_unstaged_changes: bool,
    #[serde(default)]
    pub has_untracked_files: bool,
}

impl CodingGitState {
    pub fn from_workspace(workspace_root: &Path) -> Result<Self> {
        let Some(git_root) = find_git_root(workspace_root)? else {
            return Ok(Self {
                git_root: None,
                head: None,
                branch: None,
                detached: false,
                dirty: CodingDirtyState::NotGit,
                has_staged_changes: false,
                has_unstaged_changes: false,
                has_untracked_files: false,
            });
        };
        let head_info = read_git_head(&git_root)?;
        let status = read_git_status(&git_root)?;
        Ok(Self {
            git_root: Some(git_root),
            head: head_info.oid,
            branch: status.branch.or(head_info.branch),
            detached: status.detached || head_info.detached,
            dirty: status.dirty,
            has_staged_changes: status.has_staged_changes,
            has_unstaged_changes: status.has_unstaged_changes,
            has_untracked_files: status.has_untracked_files,
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodingDirtyState {
    Clean,
    Dirty,
    NotGit,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitHeadInfo {
    oid: Option<String>,
    branch: Option<String>,
    detached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitStatusInfo {
    branch: Option<String>,
    detached: bool,
    dirty: CodingDirtyState,
    has_staged_changes: bool,
    has_unstaged_changes: bool,
    has_untracked_files: bool,
}

fn canonical_workspace_root(path: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path).map_err(|source| IkarosError::io(path, source))?;
    if !canonical.is_dir() {
        return Err(IkarosError::Message(format!(
            "coding workspace is not a directory: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn find_git_root(workspace_root: &Path) -> Result<Option<PathBuf>> {
    let mut current = Some(workspace_root);
    while let Some(path) = current {
        let git_marker = path.join(".git");
        if usable_git_marker(&git_marker) {
            return Ok(Some(path.to_path_buf()));
        }
        current = path.parent();
    }
    Ok(None)
}

fn usable_git_marker(marker: &Path) -> bool {
    marker.is_file() || marker.join("HEAD").is_file()
}

fn read_git_head(git_root: &Path) -> Result<GitHeadInfo> {
    let git_dir = git_directory(git_root)?;
    let head_path = git_dir.join("HEAD");
    let head = match fs::read_to_string(&head_path) {
        Ok(value) => value,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(GitHeadInfo {
                oid: None,
                branch: None,
                detached: false,
            });
        }
        Err(source) => return Err(IkarosError::io(&head_path, source)),
    };
    let head = head.trim();
    if let Some(reference) = head.strip_prefix("ref: ") {
        let reference_path = git_dir.join(reference);
        let oid = fs::read_to_string(&reference_path)
            .map_err(|source| IkarosError::io(&reference_path, source))?;
        return Ok(GitHeadInfo {
            oid: normalized_git_oid(&oid),
            branch: reference.strip_prefix("refs/heads/").map(ToOwned::to_owned),
            detached: false,
        });
    }
    Ok(GitHeadInfo {
        oid: normalized_git_oid(head),
        branch: None,
        detached: normalized_git_oid(head).is_some(),
    })
}

fn git_directory(git_root: &Path) -> Result<PathBuf> {
    let marker = git_root.join(".git");
    if marker.is_dir() {
        return Ok(marker);
    }
    let content = fs::read_to_string(&marker).map_err(|source| IkarosError::io(&marker, source))?;
    let gitdir = content
        .trim()
        .strip_prefix("gitdir:")
        .ok_or_else(|| IkarosError::Message(format!("invalid .git file: {}", marker.display())))?
        .trim();
    let path = PathBuf::from(gitdir);
    Ok(if path.is_absolute() {
        path
    } else {
        git_root.join(path)
    })
}

fn normalized_git_oid(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| redact_secrets(value))
}

fn read_git_status(git_root: &Path) -> Result<GitStatusInfo> {
    let git_dir = git_directory(git_root)?;
    let fixture = git_dir.join("status_porcelain_v1");
    if fixture.exists() {
        let output =
            fs::read_to_string(&fixture).map_err(|source| IkarosError::io(&fixture, source))?;
        return Ok(parse_git_status_porcelain(&output));
    }

    let output = Command::new("git")
        .arg("status")
        .arg("--porcelain=v1")
        .arg("--branch")
        .current_dir(git_root)
        .output();
    let Ok(output) = output else {
        return Ok(unknown_git_status());
    };
    if !output.status.success() {
        return Ok(unknown_git_status());
    }
    Ok(parse_git_status_porcelain(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn parse_git_status_porcelain(output: &str) -> GitStatusInfo {
    let mut branch = None;
    let mut detached = false;
    let mut has_staged_changes = false;
    let mut has_unstaged_changes = false;
    let mut has_untracked_files = false;

    for raw_line in output.lines() {
        let line = raw_line.trim_end_matches('\0');
        if let Some(header) = line.strip_prefix("## ") {
            let header = header.split("...").next().unwrap_or(header).trim();
            if header.eq_ignore_ascii_case("HEAD (no branch)") || header.starts_with("HEAD ") {
                detached = true;
            } else if !header.is_empty() {
                branch = Some(redact_secrets(header));
            }
            continue;
        }
        if line == "??" || line.starts_with("?? ") {
            has_untracked_files = true;
            continue;
        }
        let mut chars = line.chars();
        let index_status = chars.next().unwrap_or(' ');
        let worktree_status = chars.next().unwrap_or(' ');
        if index_status != ' ' && index_status != '?' && index_status != '!' {
            has_staged_changes = true;
        }
        if worktree_status != ' ' && worktree_status != '?' && worktree_status != '!' {
            has_unstaged_changes = true;
        }
    }

    let dirty = if has_staged_changes || has_unstaged_changes || has_untracked_files {
        CodingDirtyState::Dirty
    } else {
        CodingDirtyState::Clean
    };
    GitStatusInfo {
        branch,
        detached,
        dirty,
        has_staged_changes,
        has_unstaged_changes,
        has_untracked_files,
    }
}

fn unknown_git_status() -> GitStatusInfo {
    GitStatusInfo {
        branch: None,
        detached: false,
        dirty: CodingDirtyState::Unknown,
        has_staged_changes: false,
        has_unstaged_changes: false,
        has_untracked_files: false,
    }
}
