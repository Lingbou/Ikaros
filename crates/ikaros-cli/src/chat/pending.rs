// SPDX-License-Identifier: GPL-3.0-only

use super::{
    interactive::InteractiveChatRuntime,
    notice::{WorkbenchNotice, WorkbenchNoticeKind},
    turn::{InteractiveChatTurnContext, run_and_print_interactive_chat_turn_or_continue},
    workbench::{self, append_workbench_history, terminal_inline},
};
use anyhow::Result;
use ikaros_core::redact_secrets;
use ikaros_runtime::ChatRunOptions;
use std::collections::VecDeque;

pub(in crate::chat) async fn drain_pending_interactive_inputs(
    ctx: &InteractiveChatTurnContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
    mut fullscreen_terminal: Option<&mut workbench::PersistentWorkbenchTerminal>,
) -> Result<()> {
    let total = runtime.pending_inputs.len();
    if total == 0 {
        if !runtime.fullscreen_stdout_quiet() {
            println!("pending_input_run: pending_inputs=0 status=empty");
        }
        return Ok(());
    }
    let mut pending = VecDeque::new();
    std::mem::swap(&mut pending, &mut runtime.pending_inputs);
    let mut index = 0usize;
    while let Some(input) = pending.pop_front() {
        index += 1;
        if !runtime.fullscreen_stdout_quiet() {
            println!("pending_input: running index={} total={}", index, total);
            println!("pending_input_message: {}", redact_secrets(&input));
        }
        runtime.push_notice(WorkbenchNotice::new(
            WorkbenchNoticeKind::Continuation,
            "pending input",
            &format!("running queued input {index}/{total}"),
        ));
        append_workbench_history(ctx.paths, &input)?;
        let completed = run_and_print_interactive_chat_turn_or_continue(
            &input,
            ctx,
            runtime,
            options,
            fullscreen_terminal.as_deref_mut(),
        )
        .await?;
        if !completed {
            pending.push_front(input);
            restore_pending_inputs_after_failure(runtime, pending, "pending_input");
            if !runtime.fullscreen_stdout_quiet() {
                println!(
                    "pending_input_run: pending_inputs={} status=stopped reason=turn_failure",
                    runtime.pending_inputs.len()
                );
            }
            runtime.push_notice(WorkbenchNotice::error(
                "pending input stopped",
                "turn failed while draining queued input",
            ));
            break;
        }
    }
    if runtime.pending_inputs.is_empty() {
        if !runtime.fullscreen_stdout_quiet() {
            println!("pending_input_run: pending_inputs=0 status=completed");
        }
        runtime.push_notice(WorkbenchNotice::new(
            WorkbenchNoticeKind::Continuation,
            "pending input",
            "queue drain completed",
        ));
    }
    Ok(())
}

pub(in crate::chat) fn requeue_failed_interactive_input(
    runtime: &mut InteractiveChatRuntime,
    input: &str,
    source: &str,
) {
    if runtime
        .pending_inputs
        .front()
        .is_some_and(|queued| queued == input)
    {
        if !runtime.fullscreen_stdout_quiet() {
            println!(
                "pending_input_requeued: source={} position=front deduped=true pending_inputs={}",
                terminal_inline(source),
                runtime.pending_inputs.len()
            );
        }
        runtime.push_notice(WorkbenchNotice::new(
            WorkbenchNoticeKind::Continuation,
            "pending input requeued",
            &format!("source={source} deduped=true"),
        ));
        return;
    }
    runtime.pending_inputs.push_front(input.to_owned());
    if !runtime.fullscreen_stdout_quiet() {
        println!(
            "pending_input_requeued: source={} position=front deduped=false pending_inputs={} reason=turn_failure",
            terminal_inline(source),
            runtime.pending_inputs.len()
        );
    }
    runtime.push_notice(WorkbenchNotice::new(
        WorkbenchNoticeKind::Continuation,
        "pending input requeued",
        &format!("source={source} reason=turn_failure"),
    ));
}

fn restore_pending_inputs_after_failure(
    runtime: &mut InteractiveChatRuntime,
    mut remaining: VecDeque<String>,
    source: &str,
) {
    let restored = remaining.len();
    remaining.append(&mut runtime.pending_inputs);
    runtime.pending_inputs = remaining;
    if !runtime.fullscreen_stdout_quiet() {
        println!(
            "pending_inputs_restored: source={} restored={} pending_inputs={} reason=turn_failure",
            terminal_inline(source),
            restored,
            runtime.pending_inputs.len()
        );
    }
    runtime.push_notice(WorkbenchNotice::new(
        WorkbenchNoticeKind::Continuation,
        "pending inputs restored",
        &format!("source={source} restored={restored} reason=turn_failure"),
    ));
}
