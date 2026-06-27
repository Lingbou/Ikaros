// SPDX-License-Identifier: GPL-3.0-only

use super::{
    acl::MessageWebhookAcl,
    http::{parse_http_request_line, read_http_headers, write_webhook_http_response},
    response::{
        MessageWebhookHttpResponse, MessageWebhookIngressPolicy, is_message_route,
        webhook_http_response,
    },
};
use crate::LocalGatewayStore;
use ikaros_core::{IkarosError, Result};
use ring::hmac;
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    net::{IpAddr, TcpListener, TcpStream},
    path::Path,
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct MessageWebhookServerConfig {
    pub host: String,
    pub port: u16,
    pub max_body_bytes: usize,
    pub hmac_secret: Option<String>,
    pub allow_sources: Vec<String>,
    pub allow_accounts: Vec<String>,
    pub allow_peers: Vec<String>,
    pub allow_threads: Vec<String>,
    pub require_pairing: bool,
    pub unsafe_tools: bool,
    pub once: bool,
}

pub fn serve_message_webhook(
    args: MessageWebhookServerConfig,
    gateway_dir: impl AsRef<Path>,
) -> Result<()> {
    let gateway_dir = gateway_dir.as_ref();
    fs::create_dir_all(gateway_dir).map_err(|source| IkarosError::io(gateway_dir, source))?;
    require_loopback_host(&args.host)?;
    let listener = TcpListener::bind((args.host.as_str(), args.port)).map_err(|source| {
        IkarosError::Message(format!(
            "failed to bind message webhook on {}:{}: {source}",
            args.host, args.port
        ))
    })?;
    let local_addr = listener.local_addr().map_err(|source| {
        IkarosError::Message(format!("failed to read message webhook address: {source}"))
    })?;
    println!("message_webhook: http://{local_addr}/message");
    println!("health: http://{local_addr}/healthz");
    let store = LocalGatewayStore::new(gateway_dir);
    println!("gateway_inbox: {}", store.inbox_path().display());
    let acl = MessageWebhookAcl::from_allow_lists(
        &args.allow_sources,
        &args.allow_accounts,
        &args.allow_peers,
        &args.allow_threads,
    );
    for stream in listener.incoming() {
        let stream = stream.map_err(|source| {
            IkarosError::Message(format!(
                "failed to accept message webhook request: {source}"
            ))
        })?;
        if let Err(error) = handle_webhook_stream(
            stream,
            &store,
            args.max_body_bytes,
            args.hmac_secret.as_deref(),
            acl.as_ref(),
            args.require_pairing,
            !args.unsafe_tools,
        ) {
            eprintln!("message webhook request failed: {error:#}");
        }
        if args.once {
            break;
        }
    }
    Ok(())
}

pub fn require_loopback_host(host: &str) -> Result<()> {
    if host.eq_ignore_ascii_case("localhost") {
        return Ok(());
    }
    let parsed: IpAddr = host.parse().map_err(|source| {
        IkarosError::Message(format!(
            "message webhook host must be loopback: {host}: {source}"
        ))
    })?;
    if !parsed.is_loopback() {
        return Err(IkarosError::Message(format!(
            "message webhook only binds loopback addresses in the MVP; got {}",
            host
        )));
    }
    Ok(())
}

pub fn handle_webhook_stream(
    mut stream: TcpStream,
    store: &LocalGatewayStore,
    max_body_bytes: usize,
    hmac_secret: Option<&str>,
    acl: Option<&MessageWebhookAcl>,
    require_pairing: bool,
    safe_tools: bool,
) -> Result<()> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|source| {
            IkarosError::Message(format!(
                "failed to set message webhook read timeout: {source}"
            ))
        })?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|source| {
            IkarosError::Message(format!(
                "failed to set message webhook write timeout: {source}"
            ))
        })?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|source| {
        IkarosError::Message(format!("failed to clone message webhook stream: {source}"))
    })?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).map_err(|source| {
        IkarosError::Message(format!("failed to read message webhook request: {source}"))
    })? == 0
    {
        return Ok(());
    }
    let Some((method, target)) = parse_http_request_line(&request_line) else {
        let response = MessageWebhookHttpResponse::plain(400, "Bad Request", "bad request\n");
        return write_webhook_http_response(&mut stream, &response, true);
    };
    let headers = read_http_headers(&mut reader)?;
    let route = target.split('?').next().unwrap_or(target);
    let mut body = Vec::new();
    if method == "POST" && is_message_route(route) {
        let content_length = match headers.content_length {
            Some(value) => value,
            None => {
                let response =
                    MessageWebhookHttpResponse::plain(411, "Length Required", "length required\n");
                return write_webhook_http_response(&mut stream, &response, true);
            }
        };
        if content_length > max_body_bytes {
            let response =
                MessageWebhookHttpResponse::plain(413, "Payload Too Large", "payload too large\n");
            return write_webhook_http_response(&mut stream, &response, true);
        }
        body.resize(content_length, 0);
        reader.read_exact(&mut body).map_err(|source| {
            IkarosError::Message(format!("failed to read message webhook body: {source}"))
        })?;
        if let Some(secret) = hmac_secret {
            if let Err(reason) =
                verify_webhook_signature(secret, headers.ikaros_signature.as_deref(), &body)
            {
                let response =
                    MessageWebhookHttpResponse::plain(401, "Unauthorized", format!("{reason}\n"));
                return write_webhook_http_response(&mut stream, &response, true);
            }
        }
    }
    let response = webhook_http_response(
        method,
        route,
        headers.content_type.as_deref(),
        &body,
        store,
        MessageWebhookIngressPolicy {
            acl,
            require_pairing,
            safe_tools,
        },
    )?;
    write_webhook_http_response(&mut stream, &response, method != "HEAD")
}

pub fn verify_webhook_signature(
    secret: &str,
    signature_header: Option<&str>,
    body: &[u8],
) -> std::result::Result<(), &'static str> {
    let signature_header = signature_header
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("missing webhook signature")?;
    let signature_hex = signature_header
        .strip_prefix("sha256=")
        .ok_or("invalid webhook signature")?;
    let signature = decode_hex(signature_hex).ok_or("invalid webhook signature")?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    hmac::verify(&key, body, &signature).map_err(|_| "invalid webhook signature")
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_nibble(pair[0])?;
            let low = hex_nibble(pair[1])?;
            Some((high << 4) | low)
        })
        .collect()
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
