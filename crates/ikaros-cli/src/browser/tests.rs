// SPDX-License-Identifier: GPL-3.0-only

use super::http::{
    cdp_endpoint_url, cdp_new_target_path, cdp_target_path, truncated_redacted_body,
};

#[test]
fn cdp_endpoint_url_normalizes_slashes() {
    assert_eq!(
        cdp_endpoint_url("http://127.0.0.1:9222/", "/json/version"),
        "http://127.0.0.1:9222/json/version"
    );
}

#[test]
fn truncated_body_redacts_secret_like_text() {
    let preview = truncated_redacted_body("token sk-browser-secret");
    assert!(!preview.contains("sk-browser-secret"));
    assert!(preview.contains("[REDACTED_SECRET]"));
}

#[test]
fn cdp_new_target_path_encodes_target_url() {
    assert_eq!(
        cdp_new_target_path("https://example.com/a b?q=one&x=two").expect("path"),
        "/json/new?https%3A%2F%2Fexample.com%2Fa+b%3Fq%3Done%26x%3Dtwo"
    );
}

#[test]
fn cdp_target_path_rejects_path_injection() {
    assert!(cdp_target_path("/json/activate", "../target").is_err());
    assert_eq!(
        cdp_target_path("/json/activate", "ABC_123-def").expect("path"),
        "/json/activate/ABC_123-def"
    );
}
