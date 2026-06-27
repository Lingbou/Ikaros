// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_runtime::{ChatMessageResult, ChatTurnReport};
use ikaros_tui::TerminalMarkdownRenderer;
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

pub(super) fn print_chat_content_for_human_transcript(
    report: &ChatTurnReport,
    already_streamed: bool,
) -> Result<()> {
    if human_transcript_already_rendered(report.streamed, already_streamed) {
        // TextDelta stdout already rendered the assistant answer. The turn separator owns the
        // following newline, so do not add another blank row here.
        return Ok(());
    }
    let rendered = render_assistant_markdown_transcript(&report.response.content);
    let rendered = color_assistant_bullet_for_terminal(&rendered, io::stdout().is_terminal());
    super::terminal::print_inline_history_text(&rendered)?;
    Ok(())
}

fn human_transcript_already_rendered(streamed: bool, already_streamed: bool) -> bool {
    streamed && already_streamed
}

fn print_rendered_markdown_transcript(content: &str) {
    println!("rendered_markdown:");
    println!("{}", render_terminal_markdown(content));
}

pub(crate) fn render_terminal_markdown(input: &str) -> String {
    TerminalMarkdownRenderer::new(Some(markdown_render_width())).render_text(input)
}

pub(crate) fn render_assistant_markdown_transcript(input: &str) -> String {
    prefix_assistant_transcript_lines(&render_terminal_markdown(input))
}

pub(crate) fn color_assistant_bullet_for_terminal(rendered: &str, terminal: bool) -> String {
    if !terminal {
        return rendered.to_owned();
    }
    rendered.replacen('•', "\x1b[32m•\x1b[0m", 1)
}

fn prefix_assistant_transcript_lines(rendered: &str) -> String {
    let mut saw_content = false;
    rendered
        .lines()
        .map(|line| {
            if line.trim().is_empty() {
                return String::new();
            }
            if saw_content {
                format!("  {}", assistant_child_line(line))
            } else {
                saw_content = true;
                format!("• {}", assistant_lead_line(line))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assistant_lead_line(line: &str) -> String {
    line.strip_prefix("• ")
        .map(str::to_owned)
        .unwrap_or_else(|| line.to_owned())
}

fn assistant_child_line(line: &str) -> String {
    line.to_owned()
}

fn markdown_render_width() -> usize {
    crossterm::terminal::size()
        .map(|(width, _)| usize::from(width).saturating_sub(2).max(24))
        .unwrap_or(80)
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
        assert!(rendered.contains("╭─ diff"));
        assert!(rendered.contains("╰─"));
        assert!(rendered.contains("│ - old [REDACTED_SECRET]"));
        assert!(rendered.contains("│ + new value"));
        assert_rendered_table_contains(&rendered, &["File", "Status"]);
        assert_rendered_table_contains(&rendered, &["src/lib.rs", "changed"]);
        assert!(rendered.contains("╭─ rust"));
        assert!(rendered.contains("│ fn main() {}"));
        assert!(!rendered.contains("[table]"));
        assert!(!rendered.contains("[code"));
        assert!(!rendered.contains("| --- |"));
        assert!(!rendered.contains("| File | Status |"));
        assert!(!rendered.contains("sk-secret-value"));
    }

    #[test]
    fn assistant_markdown_transcript_uses_codex_like_prefixes() {
        let rendered =
            render_assistant_markdown_transcript("### Summary\n\n- item one\n> quoted\nplain");

        assert_eq!(rendered, "• Summary\n\n  • item one\n  │ quoted\n  plain");
        assert!(!rendered.contains("###"));
    }

    #[test]
    fn assistant_markdown_transcript_uses_simple_child_markers_without_breaking_cjk() {
        let rendered =
            render_assistant_markdown_transcript("### 总结\n\n- 中文项目\n1. 第一项\n普通文本");

        assert_eq!(rendered, "• 总结\n\n  • 中文项目\n  1. 第一项\n  普通文本");
        assert!(!rendered.contains("###"));
        assert!(!rendered.contains('└'));
    }

    #[test]
    fn assistant_bullet_color_is_terminal_only() {
        let rendered = "• Summary\n  • item";

        assert_eq!(
            color_assistant_bullet_for_terminal(rendered, true),
            "\x1b[32m•\x1b[0m Summary\n  • item"
        );
        assert_eq!(
            color_assistant_bullet_for_terminal(rendered, false),
            rendered
        );
    }

    #[test]
    fn streamed_human_transcript_does_not_add_extra_terminator() {
        assert!(human_transcript_already_rendered(true, true));
        assert!(!human_transcript_already_rendered(true, false));
        assert!(!human_transcript_already_rendered(false, true));
        assert!(!human_transcript_already_rendered(false, false));
    }

    #[test]
    fn assistant_markdown_transcript_cleans_inline_markers_without_tree_artifacts() {
        let input = r#"### **Summary**

- Run `git status`
  - **nested** item
Plain **bold** and `inline code`.

```sh
echo hello
```
"#;

        let rendered = render_assistant_markdown_transcript(input);

        assert!(rendered.contains("• Summary"));
        assert!(rendered.contains("  • Run git status"));
        assert!(rendered.contains("  • nested item"));
        assert!(rendered.contains("  Plain bold and inline code."));
        assert!(rendered.contains("  ╭─ sh"));
        assert!(rendered.contains("  │ echo hello"));
        assert!(rendered.contains("  ╰─"));
        assert!(!rendered.contains("**"));
        assert!(!rendered.contains('`'));
        assert!(!rendered.contains("```"));
        assert!(!rendered.contains('└'));
    }

    #[test]
    fn assistant_markdown_transcript_formats_nonstream_markdown_like_live_output() {
        let input = r#"### Summary

> quoted

```rust
fn main() {}
```

| File | Status |
| --- | --- |
| src/lib.rs | changed |
"#;

        let rendered = render_assistant_markdown_transcript(input);

        assert!(rendered.contains("• Summary"));
        assert!(rendered.contains("  │ quoted"));
        assert!(rendered.contains("  ╭─ rust"));
        assert!(rendered.contains("  │ fn main() {}"));
        assert!(rendered.contains("  ╰─"));
        assert_rendered_table_contains(&rendered, &["File", "Status"]);
        assert_rendered_table_contains(&rendered, &["src/lib.rs", "changed"]);
        assert!(!rendered.contains("###"));
        assert!(!rendered.contains("> quoted"));
        assert!(!rendered.contains("```"));
    }

    fn assert_rendered_table_contains(rendered: &str, expected_cells: &[&str]) {
        let rows = rendered_table_rows(rendered);
        assert!(
            rows.iter().any(|row| row.as_slice() == expected_cells),
            "missing rendered table row {expected_cells:?} in:\n{rendered}"
        );
    }

    fn rendered_table_rows(rendered: &str) -> Vec<Vec<&str>> {
        rendered
            .lines()
            .filter_map(|line| {
                let cells = line
                    .split('│')
                    .map(str::trim)
                    .filter(|cell| !cell.is_empty())
                    .collect::<Vec<_>>();
                (cells.len() >= 2).then_some(cells)
            })
            .collect()
    }
}
