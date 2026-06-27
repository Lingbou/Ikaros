// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_agent::chat::{ChatMessageResult, ChatTurnReport};
use ikaros_terminal::{
    color_assistant_bullet_for_terminal, print_inline_history_text,
    render_assistant_markdown_transcript_for_current_width,
    render_terminal_markdown_for_current_width,
};
use std::io::{self, IsTerminal, Write};

pub(super) fn print_chat_message_result(result: &ChatMessageResult) -> Result<()> {
    println!("ok: true");
    println!(
        "context: relationship={} references={} history={} memory={} rag={} relationship_candidates_created={}",
        result.relationship_hits,
        result.reference_hits,
        result.history_hits,
        result.memory_hits,
        result.rag_hits,
        result.relationship_candidates_created
    );
    println!("provider: {}", result.provider);
    println!("model: {}", result.model);
    println!("emotion: {:?}", result.emotion);
    println!("streamed: {}", result.streamed);
    if result.streamed {
        println!("stream_chunks: {}", result.stream_chunks.len());
    }
    if result.streamed {
        for chunk in &result.stream_chunks {
            print!("{chunk}");
            io::stdout().flush()?;
        }
        println!();
        print_rendered_markdown_transcript(&result.content);
    } else {
        println!(
            "{}",
            render_terminal_markdown_for_current_width(&result.content)
        );
    }
    println!("audit: {}", result.audit_path.display());
    println!("model_usage: {}", result.model_usage_path.display());
    println!("chat_session: {}", result.chat_session_id);
    println!("session_state_db: {}", result.session_state_db.display());
    println!("chat_timeline: session_store");
    Ok(())
}

pub(super) fn print_chat_content(report: &ChatTurnReport) -> Result<()> {
    println!("emotion: {:?}", report.emotion);
    if report.streamed {
        for chunk in &report.stream_chunks {
            print!("{chunk}");
            io::stdout().flush()?;
        }
        println!();
        print_rendered_markdown_transcript(&report.response.content);
    } else {
        println!(
            "{}",
            render_terminal_markdown_for_current_width(&report.response.content)
        );
    }
    Ok(())
}

pub(super) fn print_chat_content_for_human_transcript(
    report: &ChatTurnReport,
    already_streamed: bool,
) -> Result<()> {
    if human_transcript_already_rendered(report.streamed, already_streamed) {
        // TextDelta stdout already rendered the assistant answer. The turn separator owns the
        // following newline, so do not add another blank row here.
        return Ok(());
    }
    let rendered = render_assistant_markdown_transcript_for_current_width(&report.response.content);
    let rendered = color_assistant_bullet_for_terminal(&rendered, io::stdout().is_terminal());
    print_inline_history_text(&rendered)?;
    Ok(())
}

fn human_transcript_already_rendered(streamed: bool, already_streamed: bool) -> bool {
    streamed && already_streamed
}

fn print_rendered_markdown_transcript(content: &str) {
    println!("rendered_markdown:");
    println!("{}", render_terminal_markdown_for_current_width(content));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_human_transcript_does_not_add_extra_terminator() {
        assert!(human_transcript_already_rendered(true, true));
        assert!(!human_transcript_already_rendered(true, false));
        assert!(!human_transcript_already_rendered(false, true));
        assert!(!human_transcript_already_rendered(false, false));
    }
}
