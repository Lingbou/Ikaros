// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_core::redact_secrets;
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
        print_rendered_markdown_transcript(&result.content);
    } else {
        println!("{}", render_terminal_markdown(&result.content));
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
        println!("{}", render_terminal_markdown(&report.response.content));
    }
    Ok(())
}

fn print_rendered_markdown_transcript(content: &str) {
    println!("rendered_markdown:");
    println!("{}", render_terminal_markdown(content));
}

pub(crate) fn render_terminal_markdown(input: &str) -> String {
    let mut rendered = Vec::new();
    let mut code_language: Option<String> = None;
    let mut in_table = false;
    for raw_line in input.lines() {
        let line = redact_secrets(raw_line);
        if let Some(language) = line.trim().strip_prefix("```") {
            if let Some(language) = code_language.take() {
                if language == "diff" {
                    rendered.push("[/diff]".to_owned());
                } else {
                    rendered.push("[/code]".to_owned());
                }
            } else {
                let language = language.trim();
                if language.eq_ignore_ascii_case("diff") {
                    rendered.push("[diff]".to_owned());
                    code_language = Some("diff".into());
                } else if language.is_empty() {
                    rendered.push("[code]".to_owned());
                    code_language = Some(String::new());
                } else {
                    rendered.push(format!("[code {}]", language));
                    code_language = Some(language.to_owned());
                }
            }
            continue;
        }
        if code_language.as_deref() == Some("diff") {
            rendered.push(render_diff_line(&line));
            continue;
        }
        if code_language.is_some() {
            rendered.push(format!("  {line}"));
            continue;
        }
        if is_markdown_table_line(&line) {
            if !in_table {
                rendered.push("[table]".to_owned());
                in_table = true;
            }
            if !is_markdown_table_separator(&line) {
                rendered.push(format!("  {}", render_table_line(&line)));
            }
            continue;
        }
        if in_table {
            rendered.push("[/table]".to_owned());
            in_table = false;
        }
        rendered.push(render_markdown_line(&line));
    }
    if code_language.is_some() {
        rendered.push("[/code]".to_owned());
    }
    if in_table {
        rendered.push("[/table]".to_owned());
    }
    rendered.join("\n")
}

fn render_markdown_line(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed
        .chars()
        .all(|ch| ch == '-' || ch == '*' || ch == '_')
        && trimmed.len() >= 3
    {
        return "---".into();
    }
    if let Some(heading) = markdown_heading(trimmed) {
        return heading;
    }
    if let Some(quote) = trimmed.strip_prefix("> ") {
        return format!("> {}", render_inline_markdown_text(quote.trim()));
    }
    if let Some(item) = markdown_unordered_item(trimmed) {
        return format!("• {}", render_inline_markdown_text(item.trim()));
    }
    if let Some(item) = markdown_ordered_item(trimmed) {
        return format!("{}. {}", item.0, render_inline_markdown_text(item.1.trim()));
    }
    render_inline_markdown_text(line)
}

pub(crate) fn markdown_heading(trimmed: &str) -> Option<String> {
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if level == 0 || level > 6 || trimmed.as_bytes().get(level) != Some(&b' ') {
        return None;
    }
    let text = trimmed[level + 1..].trim();
    if text.is_empty() {
        None
    } else {
        Some(render_inline_markdown_text(text))
    }
}

fn render_inline_markdown_text(input: &str) -> String {
    input.replace("**", "").replace("__", "").replace('`', "")
}

fn markdown_unordered_item(trimmed: &str) -> Option<&str> {
    ["- ", "* ", "+ "]
        .into_iter()
        .find_map(|prefix| trimmed.strip_prefix(prefix))
}

fn markdown_ordered_item(trimmed: &str) -> Option<(&str, &str)> {
    let dot = trimmed.find('.')?;
    let (number, rest) = trimmed.split_at(dot);
    let rest = rest.strip_prefix(". ")?;
    if number.is_empty() || !number.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some((number, rest))
}

fn render_diff_line(line: &str) -> String {
    if line.starts_with('+') || line.starts_with('-') {
        line.to_owned()
    } else {
        format!("  {line}")
    }
}

pub(crate) fn is_markdown_table_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.matches('|').count() >= 2
}

fn is_markdown_table_separator(line: &str) -> bool {
    let trimmed = line.trim().trim_matches('|').trim();
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|ch| ch == '-' || ch == ':' || ch == '|' || ch.is_whitespace())
}

fn render_table_line(line: &str) -> String {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_markdown_renderer_formats_code_diff_table_and_redacts_secrets() {
        let input = r#"Here is a patch:

### Summary

- item one

```diff
- old token=sk-secret-value
+ new value
```

| File | Status |
| --- | --- |
| src/lib.rs | changed |

```rust
fn main() {}
```
"#;

        let rendered = render_terminal_markdown(input);

        assert!(rendered.contains("Summary"));
        assert!(!rendered.contains("### Summary"));
        assert!(rendered.contains("• item one"));
        assert!(rendered.contains("[diff]"));
        assert!(rendered.contains("[/diff]"));
        assert!(rendered.contains("- old [REDACTED_SECRET]"));
        assert!(rendered.contains("+ new value"));
        assert!(rendered.contains("[table]"));
        assert!(rendered.contains("File | Status"));
        assert!(rendered.contains("[code rust]"));
        assert!(rendered.contains("fn main() {}"));
        assert!(!rendered.contains("sk-secret-value"));
    }
}
