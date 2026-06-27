// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::notice::WorkbenchNoticeKind;
use ikaros_terminal::terminal_inline;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScreenOpenCommandStatus {
    Executed,
    ExplicitActionRequired,
    Unsupported,
}

impl ScreenOpenCommandStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Executed => "executed",
            Self::ExplicitActionRequired => "explicit_action_required",
            Self::Unsupported => "unsupported",
        }
    }
}

pub(super) fn screen_open_notice_kind(status: ScreenOpenCommandStatus) -> WorkbenchNoticeKind {
    match status {
        ScreenOpenCommandStatus::Executed => WorkbenchNoticeKind::Info,
        ScreenOpenCommandStatus::ExplicitActionRequired => WorkbenchNoticeKind::Progress,
        ScreenOpenCommandStatus::Unsupported => WorkbenchNoticeKind::Error,
    }
}

pub(super) fn print_screen_open_selected_status(status: ScreenOpenCommandStatus, command: &str) {
    match status {
        ScreenOpenCommandStatus::Executed => println!("screen_open_selected_status: executed"),
        ScreenOpenCommandStatus::ExplicitActionRequired => println!(
            "screen_open_selected_status: explicit_action_required command={}",
            terminal_inline(command)
        ),
        ScreenOpenCommandStatus::Unsupported => println!(
            "screen_open_selected_status: unsupported command={}",
            terminal_inline(command)
        ),
    }
}
