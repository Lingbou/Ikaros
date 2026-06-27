// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn token_usage_parses_provider_cache_accounting_fields() {
    let openai_usage: TokenUsage = serde_json::from_value(serde_json::json!({
        "prompt_tokens": 100,
        "completion_tokens": 20,
        "total_tokens": 120,
        "prompt_tokens_details": {
            "cached_tokens": 64
        }
    }))
    .expect("openai usage");
    assert_eq!(openai_usage.cache_read_tokens, Some(64));
    assert_eq!(openai_usage.cache_write_tokens, None);

    let anthropic = r#"{
        "id": "msg_cache",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [{"type": "text", "text": "cached"}],
        "usage": {
            "input_tokens": 10,
            "output_tokens": 4,
            "cache_creation_input_tokens": 7,
            "cache_read_input_tokens": 9
        }
    }"#;
    let response = parse_messages_response(anthropic, "anthropic", "fallback").expect("response");
    assert_eq!(response.usage.cache_write_tokens, Some(7));
    assert_eq!(response.usage.cache_read_tokens, Some(9));
}
