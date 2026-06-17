// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_runtime::{ChatMessageResult, ChatTurnReport};
use std::io::{self, Write};

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
    } else {
        println!("{}", result.content);
    }
    println!("audit: {}", result.audit_path.display());
    println!("model_usage: {}", result.model_usage_path.display());
    println!("chat_session: {}", result.chat_session_id);
    println!("chat_history: {}", result.chat_history_path.display());
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
    } else {
        println!("{}", report.response.content);
    }
    Ok(())
}
