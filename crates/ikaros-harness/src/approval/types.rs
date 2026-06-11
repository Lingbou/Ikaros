// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{ToolCall, ToolResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApprovalRequest {
    pub id: String,
    pub call: ToolCall,
    pub reason: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Executed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApprovalRecord {
    pub request: ApprovalRequest,
    pub status: ApprovalStatus,
    pub updated_at: String,
    pub note: Option<String>,
    pub result: Option<ToolResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApprovalEvent {
    pub id: String,
    pub at: String,
    pub approval_id: String,
    pub kind: String,
    pub request: Option<ApprovalRequest>,
    pub status: Option<ApprovalStatus>,
    pub note: Option<String>,
    pub result: Option<ToolResult>,
}
