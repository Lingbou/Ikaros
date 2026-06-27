// SPDX-License-Identifier: GPL-3.0-only

pub(in crate::chat) fn slash_command_refreshes_screen(input: &str) -> bool {
    let command = input.split_whitespace().next().unwrap_or_default();
    matches!(
        command,
        "/screen"
            | "/queue"
            | "/attach"
            | "/attachments"
            | "/budget"
            | "/approval"
            | "/approvals"
            | "/cancel"
            | "/clear"
            | "/code"
            | "/review"
            | "/rollback"
            | "/provider"
    )
}

pub(in crate::chat) fn slash_command_runs_pending_inputs_after_success(input: &str) -> bool {
    let mut parts = input.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some("/budget"), Some("set" | "disable" | "off")) => true,
        (Some("/approval" | "/approvals"), Some("approve")) => true,
        (Some("/screen"), Some("approve-selected")) => true,
        (Some("/screen"), Some("open-selected")) => screen_open_selected_may_resume(input),
        _ => false,
    }
}

fn screen_open_selected_may_resume(input: &str) -> bool {
    input.contains("approve")
        || input.contains("budget")
        || input.contains("raise=")
        || input.contains("disable=")
}

pub(in crate::chat) fn queue_run_requested(input: &str) -> bool {
    let mut parts = input.split_whitespace();
    matches!(parts.next(), Some("/queue"))
        && matches!(parts.next(), Some("run" | "drain" | "continue"))
        && parts.next().is_none()
}
