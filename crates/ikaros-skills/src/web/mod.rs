// SPDX-License-Identifier: GPL-3.0-only

use crate::support::input_string;
use async_trait::async_trait;
use ikaros_core::{RemoteProviderConfig, Result, RiskLevel, redact_secrets};
use ikaros_tools::{NetworkEgressRequest, PolicyRequest, Skill, SkillContext, SkillOutput};
use serde_json::json;
use std::{collections::BTreeMap, path::Path};

mod content;
mod rate_limit;
mod search;
mod util;

use content::{
    citation, content_type_allowed, content_type_is_html, extract_html_title, html_to_text,
    normalize_content_type, normalize_text, response_content_type, retain_body_bytes,
    truncate_chars, validate_extract_url,
};
use rate_limit::wait_for_web_rate_limit;
use search::{
    build_duckduckgo_search_url, execute_api_web_search, parse_duckduckgo_results,
    redact_search_url,
};
use util::bounded_usize;

const DEFAULT_MAX_BYTES: usize = 64 * 1024;
const HARD_MAX_BYTES: usize = 256 * 1024;
const DEFAULT_MAX_CHARS: usize = 16 * 1024;
const HARD_MAX_CHARS: usize = 64 * 1024;
const DEFAULT_SEARCH_PROVIDER: &str = "duckduckgo-html";
const DEFAULT_SEARCH_ENDPOINT: &str = "https://duckduckgo.com/html/";
const BRAVE_SEARCH_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";
const BING_SEARCH_ENDPOINT: &str = "https://api.bing.microsoft.com/v7.0/search";
const SERPAPI_SEARCH_ENDPOINT: &str = "https://serpapi.com/search.json";
const TAVILY_SEARCH_ENDPOINT: &str = "https://api.tavily.com/search";

#[derive(Debug, Clone)]
pub struct WebExtractSkill;

#[derive(Debug, Clone, Default)]
pub struct WebSearchSkill {
    provider_settings: RemoteProviderConfig,
}

impl WebSearchSkill {
    pub fn new(provider_settings: RemoteProviderConfig) -> Self {
        Self { provider_settings }
    }
}

#[async_trait]
impl Skill for WebExtractSkill {
    fn name(&self) -> &'static str {
        "web_extract"
    }

    fn description(&self) -> &'static str {
        "Fetch and extract a single HTTP(S) page through governed NetworkEgress with citation metadata."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["url"],
            "properties": {
                "url": {
                    "type": "string",
                    "description": "HTTP or HTTPS URL to fetch."
                },
                "max_bytes": {
                    "type": "integer",
                    "minimum": 1024,
                    "maximum": HARD_MAX_BYTES,
                    "description": "Maximum response bytes retained in the extracted text."
                },
                "max_chars": {
                    "type": "integer",
                    "minimum": 256,
                    "maximum": HARD_MAX_CHARS,
                    "description": "Maximum extracted text characters returned to the model."
                }
            }
        })
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Network
    }

    fn policy_request(&self, input: &serde_json::Value, _workspace_root: &Path) -> PolicyRequest {
        PolicyRequest {
            action: self.name().into(),
            risk: RiskLevel::Network,
            path: None,
            command: input
                .get("url")
                .and_then(serde_json::Value::as_str)
                .map(redact_secrets),
            is_write: false,
        }
    }

    fn approval_context(
        &self,
        input: &serde_json::Value,
        _workspace_root: &Path,
    ) -> Option<serde_json::Value> {
        let url = input
            .get("url")
            .and_then(serde_json::Value::as_str)
            .map(redact_secrets)?;
        Some(json!({
            "kind": "web_extract",
            "url": url,
            "network_egress": true,
        }))
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let url = input_string(&input, "url")?;
        let url = validate_extract_url(&url)?;
        let max_bytes =
            bounded_usize(&input, "max_bytes", DEFAULT_MAX_BYTES, 1024, HARD_MAX_BYTES)?;
        let max_chars = bounded_usize(&input, "max_chars", DEFAULT_MAX_CHARS, 256, HARD_MAX_CHARS)?;

        let mut headers = BTreeMap::new();
        headers.insert(
            "accept".into(),
            "text/html, text/plain, text/markdown, application/json, application/xml, application/ld+json, */*;q=0.2".into(),
        );
        headers.insert("user-agent".into(), "Ikaros/0.1 web_extract".into());

        wait_for_web_rate_limit("extract", &url).await;
        let response = ctx
            .session
            .env
            .send_network_request(NetworkEgressRequest {
                method: "GET".into(),
                url: url.clone(),
                headers,
                body: None,
                body_bytes: None,
            })
            .await?;

        let content_type = response_content_type(&response.headers);
        let normalized_content_type = content_type
            .as_deref()
            .map(normalize_content_type)
            .unwrap_or_else(|| "unknown".into());
        let response_bytes = response
            .body_bytes
            .as_ref()
            .map(Vec::len)
            .unwrap_or_else(|| response.body.len());

        if !content_type_allowed(&normalized_content_type) {
            return Ok(SkillOutput::new(
                format!(
                    "web_extract skipped unsupported content type for {}",
                    redact_secrets(&url)
                ),
                json!({
                    "url": redact_secrets(&url),
                    "status": response.status,
                    "content_type": &normalized_content_type,
                    "body_bytes": response_bytes,
                    "skipped": true,
                    "reason": "unsupported_content_type",
                    "citation": citation(&url, response.status, &normalized_content_type, None),
                    "text": "",
                    "truncated": false,
                }),
            ));
        }

        let (retained_body, byte_truncated) = retain_body_bytes(&response.body, max_bytes);
        let extracted = if content_type_is_html(&normalized_content_type) {
            html_to_text(&retained_body)
        } else {
            normalize_text(&retained_body)
        };
        let title = if content_type_is_html(&normalized_content_type) {
            extract_html_title(&retained_body)
        } else {
            None
        };
        let redacted_text = redact_secrets(&extracted);
        let (text, char_truncated) = truncate_chars(&redacted_text, max_chars);
        let redacted_title = title.map(|value| redact_secrets(&value));
        let citation = citation(
            &url,
            response.status,
            &normalized_content_type,
            redacted_title.as_deref(),
        );
        let truncated = byte_truncated || char_truncated;

        Ok(SkillOutput::new(
            format!(
                "web_extract fetched {} status={} bytes={}",
                redact_secrets(&url),
                response.status,
                response_bytes
            ),
            json!({
                "url": redact_secrets(&url),
                "status": response.status,
                "content_type": &normalized_content_type,
                "body_bytes": response_bytes,
                "retained_bytes": retained_body.len(),
                "max_bytes": max_bytes,
                "max_chars": max_chars,
                "title": &redacted_title,
                "text": &text,
                "truncated": truncated,
                "byte_truncated": byte_truncated,
                "char_truncated": char_truncated,
                "skipped": false,
                "citation": &citation,
            }),
        ))
    }
}

#[async_trait]
impl Skill for WebSearchSkill {
    fn name(&self) -> &'static str {
        "web_search"
    }

    fn description(&self) -> &'static str {
        "Search the web through a governed search provider and return citation metadata without fetching result pages."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 10
                },
                "provider": {
                    "type": "string",
                    "enum": [DEFAULT_SEARCH_PROVIDER, "brave", "bing", "serpapi", "tavily"],
                    "description": "Search provider implementation."
                },
                "endpoint": {
                    "type": "string",
                    "description": "Optional HTTP(S) endpoint override for the selected search provider."
                },
                "api_key": {
                    "type": "string",
                    "description": "Optional provider API key. Prefer BRAVE_SEARCH_API_KEY, BING_SEARCH_API_KEY, SERPAPI_API_KEY, or TAVILY_API_KEY in automation."
                }
            }
        })
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Network
    }

    fn policy_request(&self, input: &serde_json::Value, _workspace_root: &Path) -> PolicyRequest {
        PolicyRequest {
            action: self.name().into(),
            risk: RiskLevel::Network,
            path: None,
            command: input
                .get("query")
                .and_then(serde_json::Value::as_str)
                .map(|query| format!("query={}", redact_secrets(query))),
            is_write: false,
        }
    }

    fn approval_context(
        &self,
        input: &serde_json::Value,
        _workspace_root: &Path,
    ) -> Option<serde_json::Value> {
        let query = input
            .get("query")
            .and_then(serde_json::Value::as_str)
            .map(redact_secrets)?;
        let provider = input
            .get("provider")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(DEFAULT_SEARCH_PROVIDER);
        Some(json!({
            "kind": "web_search",
            "provider": provider,
            "query": query,
            "network_egress": true,
        }))
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let query = input_string(&input, "query")?;
        let provider = input
            .get("provider")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(DEFAULT_SEARCH_PROVIDER);
        let max_results = bounded_usize(&input, "max_results", 5, 1, 10)?;
        if provider != DEFAULT_SEARCH_PROVIDER {
            return execute_api_web_search(
                &query,
                provider,
                max_results,
                &input,
                &self.provider_settings,
                ctx,
            )
            .await;
        }
        let endpoint = input
            .get("endpoint")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                let endpoint = self.provider_settings.base_url.trim();
                if endpoint.is_empty() {
                    DEFAULT_SEARCH_ENDPOINT
                } else {
                    endpoint
                }
            });
        let search_url = build_duckduckgo_search_url(endpoint, &query)?;

        let mut headers = BTreeMap::new();
        headers.insert(
            "accept".into(),
            "text/html, application/xhtml+xml, text/plain;q=0.8, */*;q=0.2".into(),
        );
        headers.insert("user-agent".into(), "Ikaros/0.1 web_search".into());

        wait_for_web_rate_limit(provider, &search_url).await;
        let response = ctx
            .session
            .env
            .send_network_request(NetworkEgressRequest {
                method: "GET".into(),
                url: search_url.clone(),
                headers,
                body: None,
                body_bytes: None,
            })
            .await?;

        let content_type = response_content_type(&response.headers);
        let normalized_content_type = content_type
            .as_deref()
            .map(normalize_content_type)
            .unwrap_or_else(|| "unknown".into());
        let body_bytes = response
            .body_bytes
            .as_ref()
            .map(Vec::len)
            .unwrap_or_else(|| response.body.len());
        if !content_type_allowed(&normalized_content_type) {
            return Ok(SkillOutput::new(
                format!(
                    "web_search skipped unsupported content type for {}",
                    redact_search_url(&search_url)
                ),
                json!({
                    "provider": provider,
                    "query": redact_secrets(&query),
                    "search_url": redact_search_url(&search_url),
                    "status": response.status,
                    "content_type": normalized_content_type,
                    "body_bytes": body_bytes,
                    "results": [],
                    "result_count": 0,
                    "skipped": true,
                    "reason": "unsupported_content_type",
                }),
            ));
        }

        let (retained_body, truncated) = retain_body_bytes(&response.body, DEFAULT_MAX_BYTES);
        let results = parse_duckduckgo_results(&retained_body, max_results)
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
                "query": redact_secrets(&query),
                "search_url": redact_search_url(&search_url),
                "status": response.status,
                "content_type": normalized_content_type,
                "body_bytes": body_bytes,
                "retained_bytes": retained_body.len(),
                "truncated": truncated,
                "skipped": false,
                "result_count": results.len(),
                "results": results,
            }),
        ))
    }
}
