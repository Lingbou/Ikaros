// SPDX-License-Identifier: GPL-3.0-only

use crate::{WorkbenchCell, WorkbenchCellKind, terminal_inline};
use ikaros_core::redact_secrets;

const MAX_NOTICE_DETAIL_CHARS: usize = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchNoticeKind {
    Info,
    Progress,
    Approval,
    Continuation,
    Context,
    Error,
}

impl WorkbenchNoticeKind {
    fn cell_kind(self) -> WorkbenchCellKind {
        match self {
            Self::Info => WorkbenchCellKind::Session,
            Self::Progress => WorkbenchCellKind::Continuation,
            Self::Approval => WorkbenchCellKind::Approval,
            Self::Continuation => WorkbenchCellKind::Continuation,
            Self::Context => WorkbenchCellKind::Context,
            Self::Error => WorkbenchCellKind::Error,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Progress => "progress",
            Self::Approval => "approval",
            Self::Continuation => "continuation",
            Self::Context => "context",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchNotice {
    pub kind: WorkbenchNoticeKind,
    pub title: String,
    pub detail: String,
}

impl WorkbenchNotice {
    pub fn new(kind: WorkbenchNoticeKind, title: &str, detail: &str) -> Self {
        Self {
            kind,
            title: terminal_inline(title),
            detail: truncate_notice_detail(&redact_secrets(detail)),
        }
    }

    pub fn info(title: &str, detail: &str) -> Self {
        Self::new(WorkbenchNoticeKind::Info, title, detail)
    }

    pub fn error(title: &str, detail: &str) -> Self {
        Self::new(WorkbenchNoticeKind::Error, title, detail)
    }

    pub fn to_cell(&self) -> WorkbenchCell {
        WorkbenchCell {
            kind: self.kind.cell_kind(),
            title: format!("notice {}", self.title),
            detail: format!(
                "notice_kind={} detail={}",
                self.kind.as_str(),
                terminal_inline(&self.detail)
            ),
        }
    }
}

fn truncate_notice_detail(input: &str) -> String {
    let mut output = String::new();
    for (index, ch) in input.replace(['\n', '\r'], " ").chars().enumerate() {
        if index >= MAX_NOTICE_DETAIL_CHARS {
            output.push_str("...");
            return output;
        }
        output.push(ch);
    }
    output
}
