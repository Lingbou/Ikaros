// SPDX-License-Identifier: GPL-3.0-only

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::chat) enum SlashCommandFullscreenEffect {
    Inspect,
    ActionOrProbe,
    TerminalStdout,
}

pub(in crate::chat) fn slash_command_fullscreen_effect(
    input: &str,
) -> SlashCommandFullscreenEffect {
    let tokens = input.split_whitespace().collect::<Vec<_>>();
    let Some(command) = tokens.first().copied() else {
        return SlashCommandFullscreenEffect::TerminalStdout;
    };
    let subcommand = tokens.get(1).copied();
    match command {
        "/help" | "/commands" => SlashCommandFullscreenEffect::Inspect,
        "/model" | "/status" | "/context" | "/memory" => SlashCommandFullscreenEffect::Inspect,
        "/provider" => provider_fullscreen_effect(&tokens),
        "/session" => session_fullscreen_effect(subcommand),
        "/queue" => queue_fullscreen_effect(subcommand),
        "/clear" | "/new" => SlashCommandFullscreenEffect::ActionOrProbe,
        "/approval" | "/approvals" => approval_fullscreen_effect(subcommand),
        "/attach" | "/attachments" => attach_fullscreen_effect(subcommand),
        "/budget" => budget_fullscreen_effect(subcommand),
        "/web" => web_fullscreen_effect(subcommand),
        "/vision" => vision_fullscreen_effect(&tokens),
        "/image" => image_fullscreen_effect(&tokens),
        "/mcp" => mcp_fullscreen_effect(subcommand),
        "/browser" => browser_fullscreen_effect(subcommand),
        "/gateway" => gateway_fullscreen_effect(&tokens),
        "/sandbox" => sandbox_fullscreen_effect(&tokens),
        "/agents" | "/history" | "/sessions" | "/timeline" | "/replay" | "/debug" | "/trace"
        | "/mentions" | "/tasks" | "/rag" | "/tools" | "/api" | "/diff" => {
            SlashCommandFullscreenEffect::Inspect
        }
        _ => SlashCommandFullscreenEffect::TerminalStdout,
    }
}

fn provider_fullscreen_effect(tokens: &[&str]) -> SlashCommandFullscreenEffect {
    match tokens.get(1).copied() {
        Some("health" | "matrix") if has_token(tokens, "--live") => {
            SlashCommandFullscreenEffect::ActionOrProbe
        }
        Some("health" | "matrix")
        | None
        | Some("inspect" | "profiles" | "debug" | "help")
        | Some("--help") => SlashCommandFullscreenEffect::Inspect,
        _ => SlashCommandFullscreenEffect::Inspect,
    }
}

fn session_fullscreen_effect(subcommand: Option<&str>) -> SlashCommandFullscreenEffect {
    match subcommand {
        Some("resume") => SlashCommandFullscreenEffect::ActionOrProbe,
        None | Some("status" | "history" | "timeline" | "export" | "help" | "--help") => {
            SlashCommandFullscreenEffect::Inspect
        }
        _ => SlashCommandFullscreenEffect::Inspect,
    }
}

fn queue_fullscreen_effect(subcommand: Option<&str>) -> SlashCommandFullscreenEffect {
    match subcommand {
        None => SlashCommandFullscreenEffect::Inspect,
        Some(_) => SlashCommandFullscreenEffect::ActionOrProbe,
    }
}

fn approval_fullscreen_effect(subcommand: Option<&str>) -> SlashCommandFullscreenEffect {
    match subcommand {
        Some("approve" | "deny") => SlashCommandFullscreenEffect::ActionOrProbe,
        None | Some("list" | "help" | "--help") => SlashCommandFullscreenEffect::Inspect,
        _ => SlashCommandFullscreenEffect::Inspect,
    }
}

fn attach_fullscreen_effect(subcommand: Option<&str>) -> SlashCommandFullscreenEffect {
    match subcommand {
        Some("clear" | "remove" | "image" | "audio" | "file") => {
            SlashCommandFullscreenEffect::ActionOrProbe
        }
        None | Some("status" | "list" | "help" | "--help") => SlashCommandFullscreenEffect::Inspect,
        _ => SlashCommandFullscreenEffect::Inspect,
    }
}

fn budget_fullscreen_effect(subcommand: Option<&str>) -> SlashCommandFullscreenEffect {
    match subcommand {
        Some("set" | "disable" | "off") => SlashCommandFullscreenEffect::ActionOrProbe,
        None | Some("show" | "status" | "--json" | "help" | "--help") => {
            SlashCommandFullscreenEffect::Inspect
        }
        _ => SlashCommandFullscreenEffect::Inspect,
    }
}

fn web_fullscreen_effect(subcommand: Option<&str>) -> SlashCommandFullscreenEffect {
    match subcommand {
        Some("search" | "extract") => SlashCommandFullscreenEffect::ActionOrProbe,
        None | Some("help" | "--help") => SlashCommandFullscreenEffect::Inspect,
        _ => SlashCommandFullscreenEffect::Inspect,
    }
}

fn vision_fullscreen_effect(tokens: &[&str]) -> SlashCommandFullscreenEffect {
    match tokens.get(1).copied() {
        Some("describe") if tokens.len() > 2 => SlashCommandFullscreenEffect::ActionOrProbe,
        None | Some("help" | "--help" | "describe") => SlashCommandFullscreenEffect::Inspect,
        _ => SlashCommandFullscreenEffect::Inspect,
    }
}

fn image_fullscreen_effect(tokens: &[&str]) -> SlashCommandFullscreenEffect {
    match tokens.get(1).copied() {
        Some("generate") if tokens.len() > 2 => SlashCommandFullscreenEffect::ActionOrProbe,
        None | Some("help" | "--help" | "generate") => SlashCommandFullscreenEffect::Inspect,
        _ => SlashCommandFullscreenEffect::Inspect,
    }
}

fn mcp_fullscreen_effect(subcommand: Option<&str>) -> SlashCommandFullscreenEffect {
    match subcommand {
        Some("call-stdio" | "call-http") => SlashCommandFullscreenEffect::ActionOrProbe,
        None | Some("status" | "help" | "--help") => SlashCommandFullscreenEffect::Inspect,
        _ => SlashCommandFullscreenEffect::Inspect,
    }
}

fn browser_fullscreen_effect(subcommand: Option<&str>) -> SlashCommandFullscreenEffect {
    match subcommand {
        None | Some("status" | "list" | "supervisor" | "supervisor-status" | "help" | "--help") => {
            SlashCommandFullscreenEffect::Inspect
        }
        Some(_) => SlashCommandFullscreenEffect::ActionOrProbe,
    }
}

fn gateway_fullscreen_effect(tokens: &[&str]) -> SlashCommandFullscreenEffect {
    match (tokens.get(1).copied(), tokens.get(2).copied()) {
        (Some("daemon"), Some("start" | "stop" | "restart")) => {
            SlashCommandFullscreenEffect::ActionOrProbe
        }
        (Some("adapter"), Some("enqueue" | "render-delivery" | "render_delivery")) => {
            SlashCommandFullscreenEffect::ActionOrProbe
        }
        (None, _) | (Some("status" | "help" | "--help"), _) => {
            SlashCommandFullscreenEffect::Inspect
        }
        (Some("daemon"), None | Some("status" | "help" | "--help")) => {
            SlashCommandFullscreenEffect::Inspect
        }
        (Some("adapter"), None | Some("list" | "status" | "help" | "--help")) => {
            SlashCommandFullscreenEffect::Inspect
        }
        _ => SlashCommandFullscreenEffect::Inspect,
    }
}

fn sandbox_fullscreen_effect(tokens: &[&str]) -> SlashCommandFullscreenEffect {
    if has_token(tokens, "--probe") {
        SlashCommandFullscreenEffect::ActionOrProbe
    } else {
        SlashCommandFullscreenEffect::Inspect
    }
}

fn has_token(tokens: &[&str], needle: &str) -> bool {
    tokens.iter().any(|token| *token == needle)
}

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
            | "/new"
            | "/code"
            | "/review"
            | "/rollback"
            | "/provider"
    )
}

pub(in crate::chat) fn slash_command_separates_inline_output(input: &str) -> bool {
    !matches!(
        slash_command_fullscreen_effect(input),
        SlashCommandFullscreenEffect::TerminalStdout
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_and_new_are_session_action_commands() {
        assert_eq!(
            slash_command_fullscreen_effect("/clear"),
            SlashCommandFullscreenEffect::ActionOrProbe
        );
        assert_eq!(
            slash_command_fullscreen_effect("/new"),
            SlashCommandFullscreenEffect::ActionOrProbe
        );
        assert!(slash_command_refreshes_screen("/clear"));
        assert!(slash_command_refreshes_screen("/new"));
        assert!(slash_command_separates_inline_output("/clear"));
    }
}
