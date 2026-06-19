// SPDX-License-Identifier: GPL-3.0-only

use super::{
    SelfModifyChangeKind, SelfModifyHeartbeatReport, SelfModifyOperationRecord, SelfModifyProposal,
    diff::workspace_relative_path,
};
use ikaros_core::{IkarosError, Result, now_rfc3339, redact_secrets};
use ikaros_harness::FileSystem as ExecutionFileSystem;
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SelfModifyStore {
    pub(super) workspace_root: PathBuf,
    pub(super) store_dir: PathBuf,
}

impl SelfModifyStore {
    pub fn new(workspace_root: impl Into<PathBuf>, store_dir: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            store_dir: store_dir.into(),
        }
    }

    pub fn proposal_path(&self) -> PathBuf {
        self.store_dir.join("proposals.jsonl")
    }

    pub fn operations_path(&self) -> PathBuf {
        self.store_dir.join("operations.jsonl")
    }

    #[cfg(test)]
    pub fn propose(
        &self,
        change_kind: SelfModifyChangeKind,
        target_path: impl AsRef<Path>,
        unified_diff: &str,
        proposer_task_id: Option<String>,
    ) -> Result<SelfModifyProposal> {
        let target_path = self.resolve_target(target_path.as_ref())?;
        fs::create_dir_all(&self.store_dir)
            .map_err(|source| IkarosError::io(&self.store_dir, source))?;
        let id = Uuid::new_v4().to_string();
        let rollback_plan = self.write_rollback_snapshot(&id, &target_path)?;
        let dry_run_report = self.dry_run_report(&target_path, unified_diff)?;
        let proposal = SelfModifyProposal {
            id,
            created_at: now_rfc3339()?,
            proposer_task_id: proposer_task_id.map(|value| redact_secrets(&value)),
            change_kind,
            target_path: workspace_relative_path(&target_path, &self.workspace_root),
            unified_diff: redact_secrets(unified_diff),
            dry_run_report,
            rollback_plan,
        };
        self.append(&proposal)?;
        Ok(proposal)
    }

    pub async fn propose_with_env(
        &self,
        change_kind: SelfModifyChangeKind,
        target_path: impl AsRef<Path>,
        unified_diff: &str,
        proposer_task_id: Option<String>,
        file_system: &dyn ExecutionFileSystem,
    ) -> Result<SelfModifyProposal> {
        let target_path = self
            .resolve_target_with_env(target_path.as_ref(), file_system)
            .await?;
        fs::create_dir_all(&self.store_dir)
            .map_err(|source| IkarosError::io(&self.store_dir, source))?;
        let id = Uuid::new_v4().to_string();
        let rollback_plan = self
            .write_rollback_snapshot_with_env(&id, &target_path, file_system)
            .await?;
        let dry_run_report = self.dry_run_report(&target_path, unified_diff)?;
        let proposal = SelfModifyProposal {
            id,
            created_at: now_rfc3339()?,
            proposer_task_id: proposer_task_id.map(|value| redact_secrets(&value)),
            change_kind,
            target_path: workspace_relative_path(&target_path, &self.workspace_root),
            unified_diff: redact_secrets(unified_diff),
            dry_run_report,
            rollback_plan,
        };
        self.append(&proposal)?;
        Ok(proposal)
    }

    pub fn list(&self) -> Result<Vec<SelfModifyProposal>> {
        let path = self.proposal_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&path).map_err(|source| IkarosError::io(&path, source))?;
        let mut proposals = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line.map_err(|source| IkarosError::io(&path, source))?;
            if line.trim().is_empty() {
                continue;
            }
            proposals.push(serde_json::from_str(&line)?);
        }
        Ok(proposals)
    }

    pub fn operations(&self) -> Result<Vec<SelfModifyOperationRecord>> {
        let path = self.operations_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&path).map_err(|source| IkarosError::io(&path, source))?;
        let mut operations = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line.map_err(|source| IkarosError::io(&path, source))?;
            if line.trim().is_empty() {
                continue;
            }
            operations.push(serde_json::from_str(&line)?);
        }
        Ok(operations)
    }

    pub fn get(&self, proposal_id: &str) -> Result<Option<SelfModifyProposal>> {
        Ok(self
            .list()?
            .into_iter()
            .find(|proposal| proposal.id == proposal_id))
    }

    pub fn heartbeat(&self) -> Result<SelfModifyHeartbeatReport> {
        fs::create_dir_all(&self.store_dir)
            .map_err(|source| IkarosError::io(&self.store_dir, source))?;
        let proposal_count = self.list()?.len();
        Ok(SelfModifyHeartbeatReport {
            at: now_rfc3339()?,
            status: "manual_apply_only".into(),
            proposal_count,
            proposal_store: self.proposal_path(),
            checks: vec![
                "autonomous self-modify apply path disabled".into(),
                "approval-gated manual apply path available".into(),
                "proposal store readable".into(),
                "rollback snapshots kept under local self-modify directory".into(),
            ],
        })
    }

    pub(super) fn append(&self, proposal: &SelfModifyProposal) -> Result<()> {
        let path = self.proposal_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| IkarosError::io(&path, source))?;
        let line = serde_json::to_string(proposal)?;
        writeln!(file, "{line}").map_err(|source| IkarosError::io(&path, source))
    }

    pub(super) fn append_operation(&self, operation: SelfModifyOperationRecord) -> Result<()> {
        let path = self.operations_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| IkarosError::io(&path, source))?;
        let line = serde_json::to_string(&operation)?;
        writeln!(file, "{line}").map_err(|source| IkarosError::io(&path, source))
    }
}
