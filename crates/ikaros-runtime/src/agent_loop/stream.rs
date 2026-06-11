// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::redact_secrets;

pub(super) fn stream_chunks_for_final_content(
    raw_chunks: &[String],
    final_content: &str,
) -> Vec<String> {
    let final_content = redact_secrets(final_content);
    if final_content.is_empty() {
        return Vec::new();
    }
    let raw_content = raw_chunks.join("");
    if redact_secrets(&raw_content) == final_content {
        return raw_chunks
            .iter()
            .map(|chunk| redact_secrets(chunk))
            .filter(|chunk| !chunk.is_empty())
            .collect();
    }
    chunk_final_content(&final_content, 96)
}

fn chunk_final_content(content: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    for ch in content.chars() {
        current.push(ch);
        current_len += 1;
        if current_len >= max_chars {
            chunks.push(std::mem::take(&mut current));
            current_len = 0;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}
