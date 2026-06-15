// SPDX-License-Identifier: GPL-3.0-only

use super::{
    redaction::{redact_approval_note, redact_approval_request, redact_tool_result},
    types::{ApprovalEvent, ApprovalRecord, ApprovalRequest, ApprovalStatus},
};
use ikaros_core::{IkarosError, Result, ToolResult, now_rfc3339};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ApprovalLog {
    path: PathBuf,
    execution_path: PathBuf,
}

impl ApprovalLog {
    pub fn new(audit_dir: impl Into<PathBuf>) -> Self {
        let path = audit_dir.into().join("approvals.jsonl");
        Self {
            execution_path: execution_path_for(&path),
            path,
        }
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        Self {
            execution_path: execution_path_for(&path),
            path,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append_request(&self, request: ApprovalRequest) -> Result<()> {
        self.append_execution_request(&request)?;
        self.append(ApprovalEvent {
            id: Uuid::new_v4().to_string(),
            at: now_rfc3339()?,
            approval_id: request.id.clone(),
            kind: "request".into(),
            request: Some(redact_approval_request(request)),
            status: Some(ApprovalStatus::Pending),
            note: None,
            result: None,
        })
    }

    pub(crate) fn execution_request(&self, approval_id: &str) -> Result<Option<ApprovalRequest>> {
        if !self.execution_path.exists() {
            return Ok(None);
        }
        for event in read_jsonl::<ApprovalExecutionEvent>(&self.execution_path)? {
            if event.approval_id == approval_id {
                return Ok(Some(event.request));
            }
        }
        Ok(None)
    }

    pub fn decide(
        &self,
        approval_id: &str,
        status: ApprovalStatus,
        note: Option<String>,
    ) -> Result<ApprovalRecord> {
        if !matches!(status, ApprovalStatus::Approved | ApprovalStatus::Denied) {
            return Err(IkarosError::Message(
                "approval decision must be approved or denied".into(),
            ));
        }
        let current = self.get(approval_id)?.ok_or_else(|| {
            IkarosError::Message(format!("approval request not found: {approval_id}"))
        })?;
        if current.status != ApprovalStatus::Pending {
            return Err(IkarosError::Message(format!(
                "approval {approval_id} is {:?}, not pending",
                current.status
            )));
        }
        self.append(ApprovalEvent {
            id: Uuid::new_v4().to_string(),
            at: now_rfc3339()?,
            approval_id: approval_id.into(),
            kind: "decision".into(),
            request: None,
            status: Some(status),
            note: redact_approval_note(note),
            result: None,
        })?;
        self.get(approval_id)?.ok_or_else(|| {
            IkarosError::Message(format!(
                "approval request not found after decision: {approval_id}"
            ))
        })
    }

    pub fn mark_executed(&self, approval_id: &str, result: ToolResult) -> Result<ApprovalRecord> {
        let current = self.get(approval_id)?.ok_or_else(|| {
            IkarosError::Message(format!("approval request not found: {approval_id}"))
        })?;
        if current.status != ApprovalStatus::Approved {
            return Err(IkarosError::Message(format!(
                "approval {approval_id} is {:?}, not approved",
                current.status
            )));
        }
        self.append(ApprovalEvent {
            id: Uuid::new_v4().to_string(),
            at: now_rfc3339()?,
            approval_id: approval_id.into(),
            kind: "executed".into(),
            request: None,
            status: Some(ApprovalStatus::Executed),
            note: Some("approved request executed".into()),
            result: Some(redact_tool_result(result)),
        })?;
        self.get(approval_id)?.ok_or_else(|| {
            IkarosError::Message(format!(
                "approval request not found after execution: {approval_id}"
            ))
        })
    }

    pub fn pending(&self) -> Result<Vec<ApprovalRecord>> {
        Ok(self
            .records()?
            .into_iter()
            .filter(|record| record.status == ApprovalStatus::Pending)
            .collect())
    }

    pub fn get(&self, approval_id: &str) -> Result<Option<ApprovalRecord>> {
        Ok(self
            .records()?
            .into_iter()
            .find(|record| record.request.id == approval_id))
    }

    pub fn records(&self) -> Result<Vec<ApprovalRecord>> {
        let mut records = BTreeMap::<String, ApprovalRecord>::new();
        for event in self.read_all()? {
            match event.kind.as_str() {
                "request" => {
                    if let Some(request) = event.request {
                        records.insert(
                            event.approval_id.clone(),
                            ApprovalRecord {
                                request,
                                status: event.status.unwrap_or(ApprovalStatus::Pending),
                                updated_at: event.at,
                                note: event.note,
                                result: event.result,
                            },
                        );
                    }
                }
                "decision" | "executed" => {
                    if let Some(record) = records.get_mut(&event.approval_id) {
                        if let Some(status) = event.status {
                            record.status = status;
                        }
                        record.updated_at = event.at;
                        record.note = event.note;
                        record.result = event.result;
                    }
                }
                _ => {}
            }
        }
        Ok(records.into_values().collect())
    }

    fn append(&self, event: ApprovalEvent) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let encoded = serde_json::to_string(&event)?;
        let mut file = open_append_file(&self.path, false)?;
        writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.path, source))
    }

    fn read_all(&self) -> Result<Vec<ApprovalEvent>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file =
            fs::File::open(&self.path).map_err(|source| IkarosError::io(&self.path, source))?;
        let reader = BufReader::new(file);
        let mut events = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|source| IkarosError::io(&self.path, source))?;
            if !line.trim().is_empty() {
                events.push(serde_json::from_str(&line)?);
            }
        }
        Ok(events)
    }

    fn append_execution_request(&self, request: &ApprovalRequest) -> Result<()> {
        if let Some(parent) = self.execution_path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let event = ApprovalExecutionEvent {
            id: Uuid::new_v4().to_string(),
            at: now_rfc3339()?,
            approval_id: request.id.clone(),
            request: request.clone(),
        };
        let encoded = serde_json::to_string(&event)?;
        let mut file = open_append_file(&self.execution_path, true)?;
        writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.execution_path, source))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ApprovalExecutionEvent {
    id: String,
    at: String,
    approval_id: String,
    request: ApprovalRequest,
}

fn execution_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "approvals.jsonl".into());
    let execution_name = if file_name == "approvals.jsonl" {
        "approvals.execution.jsonl".into()
    } else {
        format!("{file_name}.execution")
    };
    path.with_file_name(execution_name)
}

fn open_append_file(path: &Path, sensitive: bool) -> Result<File> {
    #[cfg(not(unix))]
    let _ = sensitive;
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    if sensitive {
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|source| IkarosError::io(path, source))?;
    #[cfg(unix)]
    if sensitive {
        let mut permissions = file
            .metadata()
            .map_err(|source| IkarosError::io(path, source))?
            .permissions();
        permissions.set_mode(0o600);
        file.set_permissions(permissions)
            .map_err(|source| IkarosError::io(path, source))?;
    }
    Ok(file)
}

fn read_jsonl<T>(path: &Path) -> Result<Vec<T>>
where
    T: serde::de::DeserializeOwned,
{
    let file = fs::File::open(path).map_err(|source| IkarosError::io(path, source))?;
    let reader = BufReader::new(file);
    let mut items = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|source| IkarosError::io(path, source))?;
        if !line.trim().is_empty() {
            items.push(serde_json::from_str(&line)?);
        }
    }
    Ok(items)
}
