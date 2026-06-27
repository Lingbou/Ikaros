// SPDX-License-Identifier: GPL-3.0-only

use crate::sanitize::terminal_inline;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchCellKind {
    Session,
    Model,
    Tool,
    Context,
    Memory,
    Coding,
    Audit,
    Continuation,
    Approval,
    Error,
}

impl WorkbenchCellKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Model => "model",
            Self::Tool => "tool",
            Self::Context => "context",
            Self::Memory => "memory",
            Self::Coding => "coding",
            Self::Audit => "audit",
            Self::Continuation => "continuation",
            Self::Approval => "approval",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchCell {
    pub kind: WorkbenchCellKind,
    pub title: String,
    pub detail: String,
}

impl WorkbenchCell {
    pub fn render(&self) -> String {
        format!(
            "cell kind={} title={} detail={}",
            self.kind.as_str(),
            terminal_inline(&self.title),
            terminal_inline(&self.detail)
        )
    }
}

pub fn render_workbench_snapshot(cells: &[WorkbenchCell], width: usize) -> String {
    let width = width.max(16);
    let mut output = format!("snapshot width={width}\n");
    for cell in cells {
        let prefix = format!("[{}]", cell.kind.as_str());
        let title = terminal_inline(&cell.title);
        if prefix.chars().count() + 1 + title.chars().count() <= width {
            output.push_str(&format!("{prefix} {title}\n"));
        } else {
            output.push_str(&prefix);
            output.push('\n');
            for line in wrap_snapshot_detail(&title, width.saturating_sub(2)) {
                output.push_str("  ");
                output.push_str(&line);
                output.push('\n');
            }
        }
        for line in wrap_snapshot_detail(&terminal_inline(&cell.detail), width.saturating_sub(2)) {
            output.push_str("  ");
            output.push_str(&line);
            output.push('\n');
        }
    }
    output
}

fn wrap_snapshot_detail(detail: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in detail.split_whitespace() {
        if word.chars().count() > width {
            if !current.is_empty() {
                lines.push(current);
                current = String::new();
            }
            let mut chunk = String::new();
            for ch in word.chars() {
                if chunk.chars().count() == width {
                    lines.push(chunk);
                    chunk = String::new();
                }
                chunk.push(ch);
            }
            if !chunk.is_empty() {
                current = chunk;
            }
            continue;
        }
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = word.to_owned();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push("none".into());
    }
    lines
}
