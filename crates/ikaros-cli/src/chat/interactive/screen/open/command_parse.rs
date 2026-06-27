// SPDX-License-Identifier: GPL-3.0-only

pub(super) fn normalize_screen_code_command(command: &str) -> Option<String> {
    let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
    let subcommand = args.first().copied()?;
    match subcommand {
        "apply" | "rollback" | "guarded-edit" | "guarded_edit" => None,
        "plan" => Some(normalize_code_command_with_default_objective(
            &args,
            "inspect the current workspace and propose the next coding plan",
        )),
        "workflow" => {
            if args.iter().any(|arg| *arg == "--apply-patch") {
                None
            } else {
                Some(normalize_code_command_with_default_objective(
                    &args,
                    "continue the current coding workflow",
                ))
            }
        }
        "test" | "review" | "iterate" => Some(args.join(" ")),
        _ => None,
    }
}

pub(super) fn command_tail(command: &str) -> String {
    command
        .split_whitespace()
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn screen_budget_command_resumes_pending_inputs(command: &str) -> bool {
    let mut parts = command.split_whitespace();
    matches!(parts.next(), Some("/budget"))
        && matches!(parts.next(), Some("set" | "disable" | "off"))
}

fn normalize_code_command_with_default_objective(args: &[&str], default_objective: &str) -> String {
    if code_args_have_positional_objective(args) {
        return args.join(" ");
    }
    let mut normalized = vec![args[0].to_owned(), shell_quote(default_objective)];
    normalized.extend(args.iter().skip(1).map(|arg| (*arg).to_owned()));
    normalized.join(" ")
}

fn code_args_have_positional_objective(args: &[&str]) -> bool {
    let mut index = 1;
    while index < args.len() {
        let arg = args[index];
        if arg == "--" {
            return args.get(index + 1).is_some();
        }
        if arg.starts_with('-') {
            if code_option_takes_value(arg) && !arg.contains('=') {
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        return true;
    }
    false
}

fn code_option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "--diff"
            | "--mode"
            | "--max-iterations"
            | "--model-token-budget"
            | "--test-command"
            | "--session-id"
            | "--turn-id"
            | "--test-analysis-json"
    )
}

fn shell_quote(input: &str) -> String {
    format!("\"{}\"", input.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(super) fn parse_debug_memory_lifecycle_args(
    args: &[&str],
    default_session_id: &str,
) -> (String, Option<String>) {
    let mut session_id = default_session_id.to_owned();
    let mut turn_id = None;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--turn-id" => {
                if let Some(value) = args.get(index + 1) {
                    turn_id = Some((*value).to_owned());
                    index += 2;
                } else {
                    index += 1;
                }
            }
            value if !value.starts_with('-') => {
                session_id = value.to_owned();
                index += 1;
            }
            _ => index += 1,
        }
    }
    (session_id, turn_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_plan_with_default_objective_when_missing() {
        let normalized =
            normalize_screen_code_command("/code plan --session-id session-one").expect("command");

        assert!(normalized.starts_with("plan \"inspect the current workspace"));
        assert!(normalized.contains("--session-id session-one"));
    }

    #[test]
    fn preserves_existing_code_objective_after_options() {
        let normalized =
            normalize_screen_code_command("/code workflow --mode review continue work")
                .expect("command");

        assert_eq!(normalized, "workflow --mode review continue work");
    }

    #[test]
    fn explicit_code_edits_require_confirmation() {
        assert!(normalize_screen_code_command("/code apply --diff patch").is_none());
        assert!(normalize_screen_code_command("/code workflow --apply-patch").is_none());
    }

    #[test]
    fn budget_resume_detection_matches_mutating_budget_commands() {
        assert!(screen_budget_command_resumes_pending_inputs(
            "/budget set 1000"
        ));
        assert!(screen_budget_command_resumes_pending_inputs(
            "/budget disable"
        ));
        assert!(!screen_budget_command_resumes_pending_inputs(
            "/budget status"
        ));
    }

    #[test]
    fn parses_debug_memory_lifecycle_session_and_turn_filters() {
        let (session_id, turn_id) = parse_debug_memory_lifecycle_args(
            &["session-two", "--turn-id", "turn-three"],
            "default",
        );

        assert_eq!(session_id, "session-two");
        assert_eq!(turn_id.as_deref(), Some("turn-three"));
    }
}
