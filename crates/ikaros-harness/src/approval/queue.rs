// SPDX-License-Identifier: GPL-3.0-only

use super::{
    log::ApprovalLog,
    types::{ApprovalRecord, ApprovalRequest, ApprovalStatus},
};
use ikaros_core::{IkarosError, Result, ToolCall, ToolResult, now_rfc3339};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct ApprovalPolicy {
    queue: Arc<Mutex<VecDeque<ApprovalRequest>>>,
    log: Option<ApprovalLog>,
}

impl ApprovalPolicy {
    pub fn with_log(log: ApprovalLog) -> Self {
        Self {
            queue: Arc::new(Mutex::new(VecDeque::new())),
            log: Some(log),
        }
    }

    pub fn enqueue(
        &self,
        call: ToolCall,
        reason: String,
        workspace_root: PathBuf,
        context: Option<serde_json::Value>,
    ) -> Result<ApprovalRequest> {
        let request = ApprovalRequest {
            id: Uuid::new_v4().to_string(),
            call,
            reason,
            created_at: now_rfc3339()?,
            workspace_root: Some(workspace_root),
            context,
        };
        if let Some(log) = &self.log {
            log.append_request(request.clone())?;
        }
        self.queue
            .lock()
            .map_err(|_| IkarosError::Message("approval queue lock poisoned".into()))?
            .push_back(request.clone());
        Ok(request)
    }

    pub fn pending(&self) -> Result<Vec<ApprovalRequest>> {
        if let Some(log) = &self.log {
            return Ok(log
                .pending()?
                .into_iter()
                .map(|record| record.request)
                .collect());
        }
        Ok(self
            .queue
            .lock()
            .map_err(|_| IkarosError::Message("approval queue lock poisoned".into()))?
            .iter()
            .cloned()
            .collect())
    }

    pub fn log(&self) -> Option<&ApprovalLog> {
        self.log.as_ref()
    }

    pub fn records(&self) -> Result<Vec<ApprovalRecord>> {
        if let Some(log) = &self.log {
            log.records()
        } else {
            Ok(self
                .pending()?
                .into_iter()
                .map(|request| ApprovalRecord {
                    updated_at: request.created_at.clone(),
                    request,
                    status: ApprovalStatus::Pending,
                    note: None,
                    result: None,
                })
                .collect())
        }
    }

    pub fn get(&self, approval_id: &str) -> Result<Option<ApprovalRecord>> {
        Ok(self
            .records()?
            .into_iter()
            .find(|record| record.request.id == approval_id))
    }

    pub(crate) fn execution_request(&self, approval_id: &str) -> Result<Option<ApprovalRequest>> {
        if let Some(log) = &self.log {
            if let Some(request) = log.execution_request(approval_id)? {
                return Ok(Some(request));
            }
        }
        Ok(self
            .queue
            .lock()
            .map_err(|_| IkarosError::Message("approval queue lock poisoned".into()))?
            .iter()
            .find(|request| request.id == approval_id)
            .cloned())
    }

    pub fn decide(
        &self,
        approval_id: &str,
        status: ApprovalStatus,
        note: Option<String>,
    ) -> Result<ApprovalRecord> {
        if let Some(log) = &self.log {
            log.decide(approval_id, status, note)
        } else {
            Err(IkarosError::Message(
                "persistent approval log is not configured".into(),
            ))
        }
    }

    pub fn mark_executed(&self, approval_id: &str, result: ToolResult) -> Result<ApprovalRecord> {
        if let Some(log) = &self.log {
            log.mark_executed(approval_id, result)
        } else {
            Err(IkarosError::Message(
                "persistent approval log is not configured".into(),
            ))
        }
    }
}
