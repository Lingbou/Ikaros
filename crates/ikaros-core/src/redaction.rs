// SPDX-License-Identifier: GPL-3.0-only

use crate::{IkarosError, Result};
use std::fmt::Write as _;

pub fn contains_secret_like(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    value.contains("sk-")
        || lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("access_token")
        || lower.contains("auth_token")
        || lower.contains("token=")
        || lower.contains("password")
        || lower.contains("private_key")
        || lower.contains("secret")
}

pub fn reject_secret_like(value: &str, context: &str) -> Result<()> {
    if contains_secret_like(value) {
        Err(IkarosError::SecretRejected(context.into()))
    } else {
        Ok(())
    }
}

pub fn redact_secrets(input: &str) -> String {
    let mut output = String::new();
    let mut token = String::new();
    for ch in input.chars() {
        if ch.is_whitespace() {
            push_redacted_token(&mut output, &token);
            token.clear();
            output.push(ch);
        } else {
            token.push(ch);
        }
    }
    push_redacted_token(&mut output, &token);
    output
}

fn push_redacted_token(output: &mut String, token: &str) {
    if token.is_empty() {
        return;
    }
    let is_assignment_secret = token
        .split_once('=')
        .is_some_and(|(key, _)| is_secret_assignment_key(key));
    if token.contains("sk-") {
        output.push_str("[REDACTED_SECRET]");
    } else if is_assignment_secret {
        let key = token.split_once('=').map_or(token, |(key, _)| key);
        let _ = write!(output, "{key}=[REDACTED_SECRET]");
    } else {
        output.push_str(token);
    }
}

fn is_secret_assignment_key(key: &str) -> bool {
    let normalized: String = key
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    matches!(
        normalized.as_str(),
        "apikey" | "accesstoken" | "authtoken" | "token" | "password" | "secret" | "privatekey"
    ) || normalized.ends_with("apikey")
        || normalized.ends_with("accesstoken")
        || normalized.ends_with("authtoken")
        || normalized.ends_with("token")
        || normalized.ends_with("password")
        || normalized.ends_with("secret")
        || normalized.ends_with("privatekey")
}

pub fn redact_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => serde_json::Value::String(redact_secrets(&text)),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(redact_json).collect())
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    if is_secret_like_key(&key) {
                        (key, serde_json::Value::String("[REDACTED_SECRET]".into()))
                    } else {
                        (key, redact_json(value))
                    }
                })
                .collect(),
        ),
        other => other,
    }
}

fn is_secret_like_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    let normalized: String = lower
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    contains_secret_like(&lower)
        || matches!(
            normalized.as_str(),
            "apikey" | "token" | "password" | "secret" | "privatekey"
        )
        || normalized.ends_with("apikey")
        || normalized.ends_with("token")
        || normalized.ends_with("password")
        || normalized.ends_with("secret")
        || normalized.ends_with("privatekey")
}
