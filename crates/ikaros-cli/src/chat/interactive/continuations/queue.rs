// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_state::session::{ContinuationId, SessionId, SessionStore, SqliteSessionStore};
use std::collections::VecDeque;

use crate::chat::notice::{WorkbenchNotice, WorkbenchNoticeKind};

use super::super::{InteractiveChatRuntime, terminal_inline};
use super::output::{continuation_status_label, continuations_json_line, pending_inputs_json_line};
use super::requeue_workbench_continuation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::chat::interactive) struct WorkbenchSelectedInputClearReport {
    pub(in crate::chat::interactive) input_index: Option<usize>,
    pub(in crate::chat::interactive) removed: Option<String>,
    pub(in crate::chat::interactive) remaining: usize,
}

pub(in crate::chat::interactive) fn clear_selected_screen_input(
    approval_side_panel_rows: usize,
    pending_inputs: &mut VecDeque<String>,
    side_selection: usize,
) -> WorkbenchSelectedInputClearReport {
    let first_input_row = approval_side_panel_rows;
    let empty = |remaining| WorkbenchSelectedInputClearReport {
        input_index: None,
        removed: None,
        remaining,
    };
    if side_selection < first_input_row {
        return empty(pending_inputs.len());
    }
    let input_index = side_selection - first_input_row;
    if input_index >= pending_inputs.len().min(4) {
        return empty(pending_inputs.len());
    }
    let removed = pending_inputs.remove(input_index);
    WorkbenchSelectedInputClearReport {
        input_index: Some(input_index + 1),
        removed,
        remaining: pending_inputs.len(),
    }
}

pub(in crate::chat::interactive) fn handle_queue_command(
    args: Vec<&str>,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    let quiet = runtime.fullscreen_stdout_quiet();
    let human = runtime.default_inline_stdout();
    match args.as_slice() {
        [] => {
            if quiet {
                runtime.push_notice(WorkbenchNotice::new(
                    WorkbenchNoticeKind::Continuation,
                    "queue",
                    &format!("pending_inputs={}", runtime.pending_inputs.len()),
                ));
            } else if human {
                println!("* Queue");
                println!("  pending: {}", runtime.pending_inputs.len());
                for (index, input) in runtime.pending_inputs.iter().take(5).enumerate() {
                    println!("  {}. {}", index + 1, terminal_inline(input));
                }
            } else {
                println!("pending_inputs: {}", runtime.pending_inputs.len());
                for (index, input) in runtime.pending_inputs.iter().enumerate() {
                    println!("- index={} message={}", index + 1, terminal_inline(input));
                }
            }
        }
        ["clear"] => {
            let cleared = runtime.pending_inputs.len();
            runtime.pending_inputs.clear();
            if quiet {
                runtime.push_notice(WorkbenchNotice::new(
                    WorkbenchNoticeKind::Continuation,
                    "queue cleared",
                    &format!("cleared={cleared} pending_inputs=0"),
                ));
            } else if human {
                println!("* Queue cleared");
                println!("  removed: {cleared}");
            } else {
                println!("pending_inputs_cleared: {cleared}");
            }
        }
        ["run" | "drain" | "continue"] => {
            if quiet {
                runtime.push_notice(WorkbenchNotice::new(
                    WorkbenchNoticeKind::Continuation,
                    "queue run",
                    &format!(
                        "pending_inputs={} status=handled_by_interactive_loop",
                        runtime.pending_inputs.len()
                    ),
                ));
            } else if human {
                println!("* Queue");
                println!("  run requested: {} pending", runtime.pending_inputs.len());
            } else {
                println!(
                    "pending_input_run: pending_inputs={} status=handled_by_interactive_loop",
                    runtime.pending_inputs.len()
                );
            }
        }
        ["retry" | "requeue", continuation_id] => {
            let session_id = SessionId::from(runtime.chat_session_id.as_str());
            let store = SqliteSessionStore::new(&runtime.state_dir);
            let continuation_id = ContinuationId::from(*continuation_id);
            let requeued = requeue_workbench_continuation(&store, &continuation_id)?;
            match requeued {
                Some(continuation) => {
                    if quiet {
                        runtime.push_notice(WorkbenchNotice::new(
                            WorkbenchNoticeKind::Continuation,
                            "continuation requeued",
                            &format!(
                                "id={} session={} status={} attempts={} run=/queue run",
                                terminal_inline(continuation.continuation_id.as_str()),
                                terminal_inline(session_id.as_str()),
                                continuation_status_label(continuation.status),
                                continuation.attempt_count,
                            ),
                        ));
                    } else if human {
                        println!("* Queue");
                        println!(
                            "  requeued: {}",
                            terminal_inline(continuation.continuation_id.as_str())
                        );
                        println!(
                            "  attempts: {} status: {}",
                            continuation.attempt_count,
                            continuation_status_label(continuation.status)
                        );
                    } else {
                        println!(
                            "continuation_requeued: id={} session={} status={} attempts={} run=/queue run cancel=/cancel {}",
                            terminal_inline(continuation.continuation_id.as_str()),
                            terminal_inline(session_id.as_str()),
                            continuation_status_label(continuation.status),
                            continuation.attempt_count,
                            terminal_inline(continuation.continuation_id.as_str()),
                        );
                    }
                }
                None => {
                    if quiet {
                        runtime.push_notice(WorkbenchNotice::error(
                            "continuation requeue",
                            &format!(
                                "id={} reason=not_requeueable_or_missing debug=/debug continuations",
                                terminal_inline(continuation_id.as_str()),
                            ),
                        ));
                    } else if human {
                        println!("* Queue");
                        println!(
                            "  continuation not found: {}",
                            terminal_inline(continuation_id.as_str())
                        );
                    } else {
                        println!(
                            "continuation_requeue_skipped: id={} reason=not_requeueable_or_missing debug=/debug continuations",
                            terminal_inline(continuation_id.as_str()),
                        );
                    }
                }
            }
            if !quiet && !human {
                println!(
                    "{}",
                    continuations_json_line(&store.continuations(&session_id)?)
                );
            }
        }
        ["retry" | "requeue"] => {
            if quiet {
                runtime.push_notice(WorkbenchNotice::error(
                    "continuation requeue",
                    "usage=/queue retry <continuation-id>",
                ));
            } else if human {
                println!("* Queue");
                println!("  usage: /queue retry <continuation-id>");
            } else {
                println!("continuation_requeue_error: usage=/queue retry <continuation-id>");
            }
        }
        ["remove", index] => match index.parse::<usize>() {
            Ok(0) | Err(_) => {
                if quiet {
                    runtime.push_notice(WorkbenchNotice::error(
                        "queue remove",
                        "index must be a positive number",
                    ));
                } else if human {
                    println!("* Queue");
                    println!("  remove needs a positive index");
                } else {
                    println!("pending_input_remove_error: index must be a positive number");
                }
            }
            Ok(index) => {
                let removed = runtime.pending_inputs.remove(index - 1);
                if let Some(removed) = removed {
                    if quiet {
                        runtime.push_notice(WorkbenchNotice::new(
                            WorkbenchNoticeKind::Continuation,
                            "queue remove",
                            &format!(
                                "index={} remaining={} message={}",
                                index,
                                runtime.pending_inputs.len(),
                                terminal_inline(&removed)
                            ),
                        ));
                    } else if human {
                        println!("* Queue");
                        println!("  removed: {}", terminal_inline(&removed));
                        println!("  remaining: {}", runtime.pending_inputs.len());
                    } else {
                        println!(
                            "pending_input_removed: index={} remaining={} message={}",
                            index,
                            runtime.pending_inputs.len(),
                            terminal_inline(&removed)
                        );
                    }
                } else {
                    if quiet {
                        runtime.push_notice(WorkbenchNotice::error(
                            "queue remove",
                            &format!(
                                "index={} reason=not_found pending_inputs={}",
                                index,
                                runtime.pending_inputs.len()
                            ),
                        ));
                    } else if human {
                        println!("* Queue");
                        println!("  input not found: {index}");
                        println!("  pending: {}", runtime.pending_inputs.len());
                    } else {
                        println!(
                            "pending_input_remove_error: index={} reason=not_found pending_inputs={}",
                            index,
                            runtime.pending_inputs.len()
                        );
                    }
                }
            }
        },
        _ => {
            let input = args.join(" ");
            runtime.pending_inputs.push_back(input);
            if quiet {
                runtime.push_notice(WorkbenchNotice::new(
                    WorkbenchNoticeKind::Continuation,
                    "queue input",
                    &format!("pending_input_queued={}", runtime.pending_inputs.len()),
                ));
            } else if human {
                println!("* Queue");
                println!("  queued: {}", runtime.pending_inputs.len());
            } else {
                println!("pending_input_queued: {}", runtime.pending_inputs.len());
            }
        }
    }
    if !quiet && !human {
        println!("{}", pending_inputs_json_line(&runtime.pending_inputs));
    }
    Ok(())
}
