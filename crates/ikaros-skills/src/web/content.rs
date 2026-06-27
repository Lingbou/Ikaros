// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result, redact_secrets};
use serde_json::json;
use std::collections::BTreeMap;
use url::Url;

pub(in crate::web) fn validate_extract_url(raw: &str) -> Result<String> {
    let parsed = Url::parse(raw)
        .map_err(|_| IkarosError::Message("web_extract url must be a valid URL".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(IkarosError::Message(format!(
            "web_extract url scheme is unsupported: {}",
            redact_secrets(parsed.scheme())
        )));
    }
    if parsed.host_str().is_none() {
        return Err(IkarosError::Message(
            "web_extract url must include a host".into(),
        ));
    }
    Ok(parsed.to_string())
}

pub(in crate::web) fn response_content_type(headers: &BTreeMap<String, String>) -> Option<String> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.clone())
}

pub(in crate::web) fn normalize_content_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase()
}

pub(in crate::web) fn content_type_allowed(content_type: &str) -> bool {
    content_type == "unknown"
        || content_type.starts_with("text/")
        || matches!(
            content_type,
            "application/json"
                | "application/ld+json"
                | "application/xml"
                | "application/yaml"
                | "application/x-yaml"
                | "application/javascript"
                | "application/x-javascript"
        )
}

pub(in crate::web) fn content_type_is_html(content_type: &str) -> bool {
    matches!(content_type, "text/html" | "application/xhtml+xml")
}

pub(in crate::web) fn retain_body_bytes(body: &str, max_bytes: usize) -> (String, bool) {
    if body.len() <= max_bytes {
        return (body.to_owned(), false);
    }
    let mut end = 0;
    for (index, character) in body.char_indices() {
        let next = index + character.len_utf8();
        if next > max_bytes {
            break;
        }
        end = next;
    }
    (body[..end].to_owned(), true)
}

pub(in crate::web) fn truncate_chars(text: &str, max_chars: usize) -> (String, bool) {
    let mut chars = text.chars();
    let mut output = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        output.push_str("\n[truncated]");
        return (output, true);
    }
    (output, false)
}

pub(in crate::web) fn html_to_text(input: &str) -> String {
    let without_script = remove_html_block(input, "script");
    let without_style = remove_html_block(&without_script, "style");
    normalize_text(&strip_html_tags(&without_style))
}

fn remove_html_block(input: &str, tag: &str) -> String {
    let mut output = input.to_owned();
    let open_pattern = format!("<{tag}");
    let close_pattern = format!("</{tag}>");
    loop {
        let lower = output.to_ascii_lowercase();
        let Some(open_start) = lower.find(&open_pattern) else {
            break;
        };
        let close_end = lower[open_start..]
            .find(&close_pattern)
            .map(|relative| open_start + relative + close_pattern.len());
        if let Some(close_end) = close_end {
            output.replace_range(open_start..close_end, " ");
        } else {
            output.replace_range(open_start.., " ");
            break;
        }
    }
    output
}

fn strip_html_tags(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut in_tag = false;
    for character in input.chars() {
        match character {
            '<' => {
                in_tag = true;
                output.push(' ');
            }
            '>' => {
                in_tag = false;
                output.push(' ');
            }
            _ if !in_tag => output.push(character),
            _ => {}
        }
    }
    decode_basic_html_entities(&output)
}

pub(in crate::web) fn extract_html_title(input: &str) -> Option<String> {
    let lower = input.to_ascii_lowercase();
    let open_start = lower.find("<title")?;
    let title_start = lower[open_start..]
        .find('>')
        .map(|index| open_start + index + 1)?;
    let title_end = lower[title_start..]
        .find("</title>")
        .map(|index| title_start + index)?;
    let title = normalize_text(&decode_basic_html_entities(&input[title_start..title_end]));
    (!title.is_empty()).then_some(title)
}

pub(in crate::web) fn decode_basic_html_entities(input: &str) -> String {
    input
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

pub(in crate::web) fn normalize_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(in crate::web) fn citation(
    url: &str,
    status: u16,
    content_type: &str,
    title: Option<&str>,
) -> serde_json::Value {
    json!({
        "url": redact_secrets(url),
        "status": status,
        "content_type": content_type,
        "title": title,
    })
}
