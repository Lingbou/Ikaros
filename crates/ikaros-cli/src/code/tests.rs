// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn model_code_review_prompt_redacts_and_truncates() {
    let diff = format!(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1,2 @@\n+token=abc123\n+{}\n",
        "x".repeat(13_000)
    );
    let review = json!({
        "summary": "review saw token=abc123",
        "findings": [{"title": "secret-like addition"}],
    });
    let prompt = review::build_model_code_review_prompt(&diff, &review).expect("prompt");
    assert!(prompt.contains("Heuristic review report"));
    assert!(prompt.contains("Guarded Patch Iteration"));
    assert!(prompt.contains("[REDACTED_SECRET]"));
    assert!(prompt.contains("[TRUNCATED]"));
    assert!(!prompt.contains("abc123"));
}

#[test]
fn reverse_unified_diff_swaps_headers_hunks_and_lines() {
    let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
-old
+new
";
    let reversed = rollback::reverse_unified_diff(diff).expect("reverse diff");
    assert!(reversed.contains("diff --git b/src/lib.rs a/src/lib.rs"));
    assert!(reversed.contains("--- b/src/lib.rs\n+++ a/src/lib.rs"));
    assert!(reversed.contains("@@ -1 +1 @@"));
    assert!(reversed.contains("-new\n+old"));
}

#[test]
fn interactive_code_parser_supports_quoted_objective() {
    let command = parse_interactive_code_command(
        r#"plan "prepare quoted objective" --session-id chat-code-session --turn-id chat-code-turn"#,
    )
    .expect("parse /code command");
    match command {
        CodeCommand::Plan {
            objective,
            session_id,
            turn_id,
            ..
        } => {
            assert_eq!(objective, "prepare quoted objective");
            assert_eq!(session_id.as_deref(), Some("chat-code-session"));
            assert_eq!(turn_id.as_deref(), Some("chat-code-turn"));
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn interactive_code_parser_decodes_escaped_newlines_in_quoted_diff() {
    let command = parse_interactive_code_command(
        r#"apply "apply escaped diff" --diff "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n""#,
    )
    .expect("parse /code apply");
    match command {
        CodeCommand::Apply {
            objective, diff, ..
        } => {
            assert_eq!(objective, "apply escaped diff");
            assert!(diff.contains("diff --git a/src/lib.rs b/src/lib.rs\n"));
            assert!(diff.contains("-old\n+new\n"));
            assert!(!diff.contains("\\n"));
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn interactive_code_parser_rejects_unterminated_quote() {
    let error = parse_interactive_code_command(r#"plan "missing end"#)
        .expect_err("unterminated quote should fail");
    assert!(error.to_string().contains("unterminated quote"));
}
