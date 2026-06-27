// SPDX-License-Identifier: GPL-3.0-only

use super::{
    BING_SEARCH_ENDPOINT, BRAVE_SEARCH_ENDPOINT, SERPAPI_SEARCH_ENDPOINT, TAVILY_SEARCH_ENDPOINT,
    content::{
        decode_basic_html_entities, html_to_text, normalize_content_type, normalize_text,
        response_content_type, truncate_chars, validate_extract_url,
    },
    rate_limit::wait_for_web_rate_limit,
};
use ikaros_core::{IkarosError, RemoteProviderConfig, Result, redact_secrets};
use ikaros_tools::{NetworkEgressRequest, SkillContext, SkillOutput};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::web) struct SearchResult {
    pub(in crate::web) title: String,
    pub(in crate::web) url: String,
    pub(in crate::web) snippet: Option<String>,
}

pub(in crate::web) async fn execute_api_web_search(
    query: &str,
    provider: &str,
    max_results: usize,
    input: &Value,
    provider_settings: &RemoteProviderConfig,
    ctx: SkillContext,
) -> Result<SkillOutput> {
    let request = build_api_search_request(provider, query, max_results, input, provider_settings)?;
    wait_for_web_rate_limit(provider, &request.url).await;
    let response = ctx
        .session
        .env
        .send_network_request(request.clone())
        .await?;
    let content_type = response_content_type(&response.headers);
    let normalized_content_type = content_type
        .as_deref()
        .map(normalize_content_type)
        .unwrap_or_else(|| "unknown".into());
    let body = response
        .body_bytes
        .as_deref()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or(response.body);
    let body_bytes = body.len();
    if response.status < 200 || response.status >= 300 {
        return Ok(SkillOutput::new(
            format!(
                "web_search provider={} failed status={}",
                provider, response.status
            ),
            json!({
                "provider": provider,
                "query": redact_secrets(query),
                "search_url": redact_search_url(&request.url),
                "status": response.status,
                "content_type": normalized_content_type,
                "body_bytes": body_bytes,
                "body_preview": redact_secrets(&truncate_chars(&body, 512).0),
                "results": [],
                "result_count": 0,
                "skipped": true,
                "reason": "provider_error",
            }),
        ));
    }
    let parsed: Value = serde_json::from_str(&body).map_err(|source| {
        IkarosError::Message(format!(
            "web_search provider `{}` returned invalid JSON: {source}",
            redact_secrets(provider)
        ))
    })?;
    let results = parse_api_search_results(provider, &parsed, max_results)
        .into_iter()
        .map(|result| {
            json!({
                "title": redact_secrets(&result.title),
                "url": redact_secrets(&result.url),
                "snippet": result.snippet.map(|snippet| redact_secrets(&snippet)),
                "citation": {
                    "url": redact_secrets(&result.url),
                    "title": redact_secrets(&result.title),
                    "provider": provider,
                }
            })
        })
        .collect::<Vec<_>>();
    Ok(SkillOutput::new(
        format!("web_search provider={} results={}", provider, results.len()),
        json!({
            "provider": provider,
            "query": redact_secrets(query),
            "search_url": redact_search_url(&request.url),
            "status": response.status,
            "content_type": normalized_content_type,
            "body_bytes": body_bytes,
            "skipped": false,
            "result_count": results.len(),
            "results": results,
        }),
    ))
}

fn build_api_search_request(
    provider: &str,
    query: &str,
    max_results: usize,
    input: &Value,
    provider_settings: &RemoteProviderConfig,
) -> Result<NetworkEgressRequest> {
    let provider = provider.trim().to_ascii_lowercase();
    match provider.as_str() {
        "brave" => build_brave_search_request(query, max_results, input, provider_settings),
        "bing" => build_bing_search_request(query, max_results, input, provider_settings),
        "serpapi" => build_serpapi_search_request(query, max_results, input, provider_settings),
        "tavily" => build_tavily_search_request(query, max_results, input, provider_settings),
        other => Err(IkarosError::Message(format!(
            "unsupported web_search provider `{}`",
            redact_secrets(other)
        ))),
    }
}

fn build_brave_search_request(
    query: &str,
    max_results: usize,
    input: &Value,
    provider_settings: &RemoteProviderConfig,
) -> Result<NetworkEgressRequest> {
    let mut url = parse_search_endpoint(
        input,
        provider_settings,
        BRAVE_SEARCH_ENDPOINT,
        "web_search brave endpoint",
    )?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("count", &max_results.to_string());
    let mut headers = search_json_headers();
    if let Some(api_key) = search_api_key(input, provider_settings, "BRAVE_SEARCH_API_KEY") {
        headers.insert("x-subscription-token".into(), api_key);
    }
    Ok(NetworkEgressRequest {
        method: "GET".into(),
        url: url.to_string(),
        headers,
        body: None,
        body_bytes: None,
    })
}

fn build_bing_search_request(
    query: &str,
    max_results: usize,
    input: &Value,
    provider_settings: &RemoteProviderConfig,
) -> Result<NetworkEgressRequest> {
    let mut url = parse_search_endpoint(
        input,
        provider_settings,
        BING_SEARCH_ENDPOINT,
        "web_search bing endpoint",
    )?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("count", &max_results.to_string());
    let mut headers = search_json_headers();
    if let Some(api_key) = search_api_key(input, provider_settings, "BING_SEARCH_API_KEY") {
        headers.insert("ocp-apim-subscription-key".into(), api_key);
    }
    Ok(NetworkEgressRequest {
        method: "GET".into(),
        url: url.to_string(),
        headers,
        body: None,
        body_bytes: None,
    })
}

fn build_serpapi_search_request(
    query: &str,
    max_results: usize,
    input: &Value,
    provider_settings: &RemoteProviderConfig,
) -> Result<NetworkEgressRequest> {
    let mut url = parse_search_endpoint(
        input,
        provider_settings,
        SERPAPI_SEARCH_ENDPOINT,
        "web_search serpapi endpoint",
    )?;
    url.query_pairs_mut()
        .append_pair("engine", "google")
        .append_pair("q", query)
        .append_pair("num", &max_results.to_string());
    if let Some(api_key) = search_api_key(input, provider_settings, "SERPAPI_API_KEY") {
        url.query_pairs_mut().append_pair("api_key", &api_key);
    }
    Ok(NetworkEgressRequest {
        method: "GET".into(),
        url: url.to_string(),
        headers: search_json_headers(),
        body: None,
        body_bytes: None,
    })
}

fn build_tavily_search_request(
    query: &str,
    max_results: usize,
    input: &Value,
    provider_settings: &RemoteProviderConfig,
) -> Result<NetworkEgressRequest> {
    let url = parse_search_endpoint(
        input,
        provider_settings,
        TAVILY_SEARCH_ENDPOINT,
        "web_search tavily endpoint",
    )?;
    let mut headers = search_json_headers();
    if let Some(api_key) = search_api_key(input, provider_settings, "TAVILY_API_KEY") {
        headers.insert("authorization".into(), format!("Bearer {api_key}"));
    }
    Ok(NetworkEgressRequest {
        method: "POST".into(),
        url: url.to_string(),
        headers,
        body: Some(
            json!({
                "query": query,
                "max_results": max_results,
                "search_depth": input.get("search_depth").and_then(Value::as_str).unwrap_or("basic"),
            })
            .to_string(),
        ),
        body_bytes: None,
    })
}

fn parse_search_endpoint(
    input: &Value,
    provider_settings: &RemoteProviderConfig,
    default: &str,
    label: &str,
) -> Result<Url> {
    let endpoint = input
        .get("endpoint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            let configured = provider_settings.base_url.trim();
            if configured.is_empty() {
                default
            } else {
                configured
            }
        });
    let parsed = Url::parse(endpoint)
        .map_err(|_| IkarosError::Message(format!("{label} must be a valid HTTP(S) URL")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(IkarosError::Message(format!(
            "{label} scheme is unsupported: {}",
            redact_secrets(parsed.scheme())
        )));
    }
    if parsed.host_str().is_none() {
        return Err(IkarosError::Message(format!("{label} must include a host")));
    }
    Ok(parsed)
}

fn search_api_key(
    input: &Value,
    provider_settings: &RemoteProviderConfig,
    env_name: &str,
) -> Option<String> {
    input
        .get("api_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            let configured = provider_settings.api_key.trim();
            (!configured.is_empty()).then(|| configured.to_owned())
        })
        .or_else(|| std::env::var(env_name).ok())
}

fn search_json_headers() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("accept".into(), "application/json".into()),
        ("content-type".into(), "application/json".into()),
        ("user-agent".into(), "Ikaros/0.1 web_search".into()),
    ])
}

pub(in crate::web) fn redact_search_url(raw: &str) -> String {
    let Ok(mut url) = Url::parse(raw) else {
        return redact_secrets(raw);
    };
    let pairs = url
        .query_pairs()
        .map(|(key, value)| {
            let value = if sensitive_search_param(&key) {
                "[REDACTED_SECRET]".into()
            } else {
                redact_secrets(&value)
            };
            (key.into_owned(), value)
        })
        .collect::<Vec<_>>();
    url.set_query(None);
    if !pairs.is_empty() {
        let mut query = url.query_pairs_mut();
        for (key, value) in pairs {
            query.append_pair(&key, &value);
        }
    }
    redact_secrets(url.as_str())
}

fn sensitive_search_param(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "api_key" | "apikey" | "key" | "token" | "access_token" | "subscription-key"
    )
}

fn parse_api_search_results(
    provider: &str,
    value: &Value,
    max_results: usize,
) -> Vec<SearchResult> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "brave" => parse_search_result_array(
            value.pointer("/web/results").and_then(Value::as_array),
            max_results,
            "title",
            "url",
            "description",
        ),
        "bing" => parse_search_result_array(
            value.pointer("/webPages/value").and_then(Value::as_array),
            max_results,
            "name",
            "url",
            "snippet",
        ),
        "serpapi" => parse_search_result_array(
            value.pointer("/organic_results").and_then(Value::as_array),
            max_results,
            "title",
            "link",
            "snippet",
        ),
        "tavily" => parse_search_result_array(
            value.pointer("/results").and_then(Value::as_array),
            max_results,
            "title",
            "url",
            "content",
        ),
        _ => Vec::new(),
    }
}

fn parse_search_result_array(
    items: Option<&Vec<Value>>,
    max_results: usize,
    title_field: &str,
    url_field: &str,
    snippet_field: &str,
) -> Vec<SearchResult> {
    items
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let title = item.get(title_field).and_then(Value::as_str)?;
            let url = item.get(url_field).and_then(Value::as_str)?;
            let url = validate_extract_url(url).ok()?;
            let snippet = item
                .get(snippet_field)
                .and_then(Value::as_str)
                .map(normalize_text)
                .filter(|value| !value.is_empty());
            let title = normalize_text(title);
            (!title.is_empty()).then_some(SearchResult {
                title,
                url,
                snippet,
            })
        })
        .take(max_results)
        .collect()
}

pub(in crate::web) fn build_duckduckgo_search_url(endpoint: &str, query: &str) -> Result<String> {
    let mut parsed = Url::parse(endpoint)
        .map_err(|_| IkarosError::Message("web_search endpoint must be a valid URL".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(IkarosError::Message(format!(
            "web_search endpoint scheme is unsupported: {}",
            redact_secrets(parsed.scheme())
        )));
    }
    if parsed.host_str().is_none() {
        return Err(IkarosError::Message(
            "web_search endpoint must include a host".into(),
        ));
    }
    parsed.query_pairs_mut().append_pair("q", query);
    Ok(parsed.to_string())
}

pub(in crate::web) fn parse_duckduckgo_results(
    html: &str,
    max_results: usize,
) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut cursor = 0;
    while results.len() < max_results {
        let Some(anchor_start) = find_case_insensitive(&html[cursor..], "<a") else {
            break;
        };
        let anchor_start = cursor + anchor_start;
        let Some(tag_end) = html[anchor_start..]
            .find('>')
            .map(|offset| anchor_start + offset)
        else {
            break;
        };
        let tag = &html[anchor_start..=tag_end];
        let Some(close_start) =
            find_case_insensitive(&html[tag_end + 1..], "</a>").map(|offset| tag_end + 1 + offset)
        else {
            break;
        };
        let inner = &html[tag_end + 1..close_start];
        cursor = close_start + "</a>".len();

        let class = html_attribute(tag, "class").unwrap_or_default();
        if !class.contains("result__a") {
            continue;
        }
        let Some(href) = html_attribute(tag, "href").and_then(normalize_search_result_url) else {
            continue;
        };
        let title = normalize_text(&html_to_text(inner));
        if title.is_empty() {
            continue;
        }
        let snippet = extract_following_snippet(&html[cursor..]);
        results.push(SearchResult {
            title,
            url: href,
            snippet,
        });
    }
    results
}

fn find_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
}

fn html_attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let pattern = format!("{name}=");
    let start = lower.find(&pattern)? + pattern.len();
    let quote = tag[start..].chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value_start = start + quote.len_utf8();
    let value_end = tag[value_start..]
        .find(quote)
        .map(|offset| value_start + offset)?;
    Some(decode_basic_html_entities(&tag[value_start..value_end]))
}

fn normalize_search_result_url(raw: String) -> Option<String> {
    let raw = raw.trim();
    let candidate = if raw.starts_with("//") {
        format!("https:{raw}")
    } else {
        raw.to_owned()
    };
    let parsed = Url::parse(&candidate).ok()?;
    if parsed
        .host_str()
        .map(|host| host.ends_with("duckduckgo.com"))
        .unwrap_or(false)
        && parsed.path().starts_with("/l/")
        && let Some(decoded) = parsed
            .query_pairs()
            .find(|(key, _)| key == "uddg")
            .map(|(_, value)| value.into_owned())
    {
        return validate_extract_url(&decoded).ok();
    }
    validate_extract_url(parsed.as_str()).ok()
}

fn extract_following_snippet(html_after_anchor: &str) -> Option<String> {
    let snippet_marker = find_case_insensitive(html_after_anchor, "result__snippet")?;
    let tag_start = html_after_anchor[..snippet_marker].rfind('<')?;
    let tag_end = html_after_anchor[snippet_marker..]
        .find('>')
        .map(|offset| snippet_marker + offset)?;
    if tag_start > 512 || tag_end > 2048 {
        return None;
    }
    let close = find_case_insensitive(&html_after_anchor[tag_end + 1..], "</")
        .map(|offset| tag_end + 1 + offset)?;
    let snippet = normalize_text(&html_to_text(&html_after_anchor[tag_end + 1..close]));
    (!snippet.is_empty()).then_some(snippet)
}
