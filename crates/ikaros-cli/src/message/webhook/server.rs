// SPDX-License-Identifier: GPL-3.0-only

use super::{
    MessageWebhook,
    http::{parse_http_request_line, read_http_headers, write_webhook_http_response},
    response::{MessageWebhookHttpResponse, is_message_route, webhook_http_response},
};
use anyhow::{Context, Result};
use ikaros_core::IkarosPaths;
use ikaros_gateway::LocalGatewayStore;
use std::{
    io::{BufRead, BufReader, Read},
    net::{IpAddr, TcpListener, TcpStream},
    time::Duration,
};

pub(crate) fn serve_message_webhook(args: MessageWebhook, paths: &IkarosPaths) -> Result<()> {
    paths.ensure()?;
    require_loopback_host(&args.host)?;
    let listener = TcpListener::bind((args.host.as_str(), args.port)).with_context(|| {
        format!(
            "failed to bind message webhook on {}:{}",
            args.host, args.port
        )
    })?;
    let local_addr = listener
        .local_addr()
        .with_context(|| "failed to read message webhook address")?;
    println!("message_webhook: http://{local_addr}/message");
    println!("health: http://{local_addr}/healthz");
    println!(
        "gateway_inbox: {}",
        paths.gateway_dir.join("inbox.jsonl").display()
    );
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    for stream in listener.incoming() {
        let stream = stream.with_context(|| "failed to accept message webhook request")?;
        if let Err(error) = handle_webhook_stream(stream, &store, args.max_body_bytes) {
            eprintln!("message webhook request failed: {error:#}");
        }
        if args.once {
            break;
        }
    }
    Ok(())
}

fn require_loopback_host(host: &str) -> Result<()> {
    if host.eq_ignore_ascii_case("localhost") {
        return Ok(());
    }
    let parsed: IpAddr = host
        .parse()
        .with_context(|| format!("message webhook host must be loopback: {host}"))?;
    if !parsed.is_loopback() {
        anyhow::bail!(
            "message webhook only binds loopback addresses in the MVP; got {}",
            host
        );
    }
    Ok(())
}

fn handle_webhook_stream(
    mut stream: TcpStream,
    store: &LocalGatewayStore,
    max_body_bytes: usize,
) -> Result<()> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .with_context(|| "failed to set message webhook read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .with_context(|| "failed to set message webhook write timeout")?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .with_context(|| "failed to clone message webhook stream")?,
    );
    let mut request_line = String::new();
    if reader
        .read_line(&mut request_line)
        .with_context(|| "failed to read message webhook request")?
        == 0
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
        reader
            .read_exact(&mut body)
            .with_context(|| "failed to read message webhook body")?;
    }
    let response =
        webhook_http_response(method, route, headers.content_type.as_deref(), &body, store)?;
    write_webhook_http_response(&mut stream, &response, method != "HEAD")
}
