// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn parses_only_canonical_json_tool_call_fallback() {
    let envelope = parse_agent_loop_model_envelope(
        r#"{"tool_calls":[{"id":"call_1","name":"loop_echo","input":{"text":"hello token=abc123"}}]}"#,
    )
    .expect("canonical tool call");
    assert_eq!(envelope.tool_calls.len(), 1);
    assert_eq!(
        envelope.parse_strategy,
        Some(AgentLoopToolCallParseStrategy::JsonFallback)
    );
    assert_eq!(envelope.tool_calls[0].id.as_deref(), Some("call_1"));
    assert_eq!(envelope.tool_calls[0].name, "loop_echo");
    assert_eq!(
        envelope.tool_calls[0].input["text"],
        "hello token=[REDACTED_SECRET]"
    );

    assert!(
        parse_agent_loop_model_envelope(
            r#"{"tool_calls":[{"function":{"name":"loop_echo","arguments":"{\"text\":\"hi\"}"}}]}"#,
        )
        .is_none()
    );
    assert!(
        parse_agent_loop_model_envelope(
            r#"Use this:
```json
[{"name":"loop_echo","args":"{\"text\":\"hello\"}"}]
```"#,
        )
        .is_none()
    );
    assert!(
        parse_agent_loop_model_envelope(
            r#"I will call this tool: {"tool_call":{"name":"loop_echo","args":{"text":"embedded"}}}"#,
        )
        .is_none()
    );
    assert!(
        parse_agent_loop_model_envelope(
            r#"{"tool_calls":[{"id":"call_1","name":"loop_echo","input":"{\"text\":\"stringified\"}"}]}"#,
        )
        .is_none()
    );
    assert!(
        parse_agent_loop_model_envelope(
            r#"{"tool_calls":[{"id":"call_1","name":"loop_echo","input":["not","object"]}]}"#,
        )
        .is_none()
    );
}

#[test]
fn rejects_non_canonical_json_tool_call_fallback_alias_corpus() {
    let invalid = [
        r#"{"final_answer":"done","answer":"alias should be rejected"}"#,
        r#"{"final_answer":"done","response":"alias should be rejected"}"#,
        r#"{"final_answer":"done","tools":[]}"#,
        r#"{"final_answer":"done","calls":[]}"#,
        r#"{"final_answer":"done","tool_calls":{"name":"loop_echo","input":{}}}"#,
        r#"{"tool_calls":[{"id":"call_1","name":"loop_echo","input":{},"args":{}}]}"#,
        r#"{"tool_calls":[{"id":"call_1","name":"loop_echo","input":{},"arguments":{}}]}"#,
        r#"{"tool_calls":[{"id":"call_1","name":"loop_echo","input":{},"function_call":{}}]}"#,
        r#"{"tool_calls":[{"id":"call_1","name":"loop_echo","input":{},"function":{"name":"loop_echo"}}]}"#,
    ];
    for content in invalid {
        assert!(
            parse_agent_loop_model_envelope(content).is_none(),
            "fallback parser accepted non-canonical envelope: {content}"
        );
    }
}

#[test]
fn rejects_non_canonical_json_tool_call_fallback_empty_required_strings() {
    let invalid = [
        r#"{"final_answer":""}"#,
        r#"{"final_answer":"   "}"#,
        r#"{"tool_calls":[{"id":"","name":"loop_echo","input":{}}]}"#,
        r#"{"tool_calls":[{"id":"   ","name":"loop_echo","input":{}}]}"#,
        r#"{"tool_calls":[{"id":"call_1","name":"","input":{}}]}"#,
        r#"{"tool_calls":[{"id":"call_1","name":"   ","input":{}}]}"#,
    ];
    for content in invalid {
        assert!(
            parse_agent_loop_model_envelope(content).is_none(),
            "fallback parser accepted empty required string: {content}"
        );
    }
}
