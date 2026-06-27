// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn agent_text_delta_uses_codex_like_prefixes_across_chunks() {
    let mut state = AgentTextDeltaState::default();

    assert_eq!(format_agent_text_delta("hello", &mut state), "");
    assert_eq!(
        format_agent_text_delta(" world\n", &mut state),
        "* hello world\n"
    );
    assert_eq!(format_agent_text_delta("next line", &mut state), "");
    assert_eq!(finish_agent_text_delta(&mut state), "  next line");
}

#[test]
fn agent_text_delta_preserves_blank_lines_and_redacts_secrets() {
    let mut state = AgentTextDeltaState::default();
    let mut rendered = format_agent_text_delta("first\n\nsecond token=sk-secret-value", &mut state);
    rendered.push_str(&finish_agent_text_delta(&mut state));

    assert_eq!(rendered, "* first\n\n  second [REDACTED_SECRET]");
}

#[test]
fn agent_text_delta_collapses_excess_blank_lines_between_paragraphs() {
    let mut state = AgentTextDeltaState::default();
    let input = "hello again.\n\n\n\nlast time we talked about pineapple.";
    let mut rendered = format_agent_text_delta(input, &mut state);
    rendered.push_str(&finish_agent_text_delta(&mut state));

    assert_eq!(
        rendered,
        "* hello again.\n\n  last time we talked about pineapple."
    );
    assert!(!rendered.contains("\n\n\n"));
}

#[test]
fn agent_text_delta_trims_excess_trailing_blank_lines() {
    let mut state = AgentTextDeltaState::default();
    let mut rendered = format_agent_text_delta("answer\n\n\n", &mut state);
    rendered.push_str(&finish_agent_text_delta(&mut state));

    assert_eq!(rendered, "* answer\n");
    assert!(!rendered.contains("\n\n"));
}

#[test]
fn agent_text_delta_does_not_add_wide_paragraph_indent() {
    let mut state = AgentTextDeltaState::default();
    let input = "hello, I am Ikaros.\n\nwhat can I help you with today?";
    let mut rendered = format_agent_text_delta(input, &mut state);
    rendered.push_str(&finish_agent_text_delta(&mut state));

    assert!(
        rendered.contains("\n\n  what can I help you with today?"),
        "{rendered}"
    );
    assert!(
        !rendered.contains("\n\n                                     what"),
        "{rendered}"
    );
}

#[test]
fn agent_text_delta_simplifies_markdown_line_starts() {
    let mut state = AgentTextDeltaState::default();
    let mut rendered = format_agent_text_delta("### Summary\n- first\n> quoted\nplain", &mut state);
    rendered.push_str(&finish_agent_text_delta(&mut state));

    assert_eq!(rendered, "* Summary\n  * first\n  > quoted\n  plain");
}

#[test]
fn agent_text_delta_keeps_blank_line_before_markdown_heading() {
    let mut state = AgentTextDeltaState::default();
    let mut rendered = format_agent_text_delta("intro\n\n### Summary", &mut state);
    rendered.push_str(&finish_agent_text_delta(&mut state));

    assert_eq!(rendered, "* intro\n\n  Summary");
}

#[test]
fn agent_text_delta_simplifies_markdown_prefixes_split_across_chunks() {
    let mut state = AgentTextDeltaState::default();
    let mut rendered = String::new();

    rendered.push_str(&format_agent_text_delta("##", &mut state));
    rendered.push_str(&format_agent_text_delta("# Summary\n-", &mut state));
    rendered.push_str(&format_agent_text_delta(" item", &mut state));
    rendered.push_str(&finish_agent_text_delta(&mut state));

    assert_eq!(rendered, "* Summary\n  * item");
    assert!(!rendered.contains('#'));
}

#[test]
fn agent_text_delta_cleans_inline_markers_across_chunks() {
    let mut state = AgentTextDeltaState::default();
    let mut rendered = String::new();

    rendered.push_str(&format_agent_text_delta("Plain **bo", &mut state));
    rendered.push_str(&format_agent_text_delta("ld** and `in", &mut state));
    rendered.push_str(&format_agent_text_delta("line code`.", &mut state));
    rendered.push_str(&finish_agent_text_delta(&mut state));

    assert_eq!(rendered, "* Plain bold and inline code.");
    assert!(!rendered.contains("**"));
    assert!(!rendered.contains('`'));
}

#[test]
fn agent_text_delta_renders_fenced_code_without_raw_fence_markers() {
    let mut state = AgentTextDeltaState::default();

    let rendered = format_agent_text_delta("```rust\nfn main() {}\n```\n", &mut state);

    assert_eq!(rendered, "* --- rust\n  | fn main() {}\n  ---\n");
    assert!(!rendered.contains("```"));
}

#[test]
fn agent_text_delta_renders_table_rows_without_raw_table_markers() {
    let mut state = AgentTextDeltaState::default();
    let rendered = format_agent_text_delta(
        "| File | Status |\n| --- | --- |\n| src/lib.rs | changed |",
        &mut state,
    );
    assert!(rendered.is_empty());
    let rendered = finish_agent_text_delta(&mut state);

    assert!(rendered.contains("* File"));
    assert!(rendered.contains("Status"));
    assert!(rendered.contains("src/lib.rs | changed"));
    assert!(!rendered.contains("| --- |"));
}

#[test]
fn agent_text_delta_matches_final_assistant_markdown_for_terminal_shapes() {
    let input = "### 总结\n\n- 中文项目\n1. 第一项\n\n```rust\nfn main() {}\n```\n\n| File | Status |\n| --- | --- |\n| src/lib.rs | changed |";
    let mut state = AgentTextDeltaState::default();
    let mut rendered = String::new();

    let chars = input.chars().collect::<Vec<_>>();
    for chunk in chars.chunks(5) {
        let chunk = chunk.iter().collect::<String>();
        rendered.push_str(&format_agent_text_delta(&chunk, &mut state));
    }
    rendered.push_str(&finish_agent_text_delta(&mut state));

    assert_eq!(
        rendered,
        ikaros_terminal::render_assistant_markdown_transcript_for_current_width(input)
    );
    assert!(!rendered.contains("###"));
    assert!(!rendered.contains("```"));
    assert!(!rendered.contains("| --- |"));
}

#[test]
fn agent_text_delta_separates_activity_from_answer() {
    let mut state = AgentTextDeltaState {
        activity_since_last_text: true,
        ..Default::default()
    };

    let mut rendered = format_agent_text_delta("answer", &mut state);
    assert!(rendered.is_empty());
    rendered.push_str(&finish_agent_text_delta(&mut state));

    assert!(rendered.starts_with('\n'));
    assert!(rendered.ends_with("* answer"));
    let separator = rendered.lines().nth(1).expect("separator line");
    assert!(separator.chars().all(|ch| ch == '─'));
    assert!(separator.chars().count() >= 24);
    assert!(!state.activity_since_last_text);
}

#[test]
fn human_activity_line_keeps_plain_text_when_not_terminal() {
    assert_eq!(
        human_activity_line_for_terminal("* Explored", false),
        "* Explored"
    );
    assert_eq!(
        human_activity_line_for_terminal("  * Read SKILL.md", false),
        "  * Read SKILL.md"
    );
}

#[test]
fn human_activity_line_colors_bullet_when_terminal() {
    assert_eq!(
        human_activity_line_for_terminal("* Explored", true),
        "\x1b[32m•\x1b[0m Explored"
    );
    assert_eq!(
        human_activity_line_for_terminal("  * Read SKILL.md", true),
        "  \x1b[36m•\x1b[0m Read SKILL.md"
    );
}
