// SPDX-License-Identifier: GPL-3.0-only

use crate::workbench_cell::WorkbenchCell;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchScreen {
    pub title: String,
    pub status: Vec<WorkbenchCell>,
    pub timeline: Vec<WorkbenchCell>,
    pub main: Vec<WorkbenchCell>,
    pub side: Vec<WorkbenchCell>,
    pub footer: String,
    pub input_hint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchScreenPanel {
    Status,
    Timeline,
    Main,
    Side,
}

impl WorkbenchScreenPanel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Timeline => "timeline",
            Self::Main => "main",
            Self::Side => "side",
        }
    }

    pub(crate) fn next(self) -> Self {
        match self {
            Self::Status => Self::Timeline,
            Self::Timeline => Self::Main,
            Self::Main => Self::Side,
            Self::Side => Self::Status,
        }
    }

    pub(crate) fn previous(self) -> Self {
        match self {
            Self::Status => Self::Side,
            Self::Timeline => Self::Status,
            Self::Main => Self::Timeline,
            Self::Side => Self::Main,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchScreenAction {
    FocusNext,
    FocusPrevious,
    ScrollDown,
    ScrollUp,
    PageDown,
    PageUp,
    ScrollTop,
    SelectNext,
    SelectPrevious,
    SelectFirst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchScreenApprovalAction {
    Approve,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchScreenContinuationAction {
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchScreenInputAction {
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchScreenOpenAction {
    OpenSelected,
    ConfirmSelected,
}
