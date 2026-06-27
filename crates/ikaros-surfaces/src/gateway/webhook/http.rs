// SPDX-License-Identifier: GPL-3.0-only

use super::response::MessageWebhookHttpResponse;
use ikaros_core::{IkarosError, Result};
use std::{
    io::{BufRead, Write},
    net::TcpStream,
};

pub fn parse_http_request_line(line: &str) -> Option<(&str, &str)> {
    let mut fields = line.split_whitespace();
    let method = fields.next()?;
    let target = fields.next()?;
    let version = fields.next()?;
    if fields.next().is_some() || !version.starts_with("HTTP/") {
        return None;
    }
    Some((method, target))
}

#[derive(Debug, Default)]
pub struct HttpHeaders {
    pub content_length: Option<usize>,
    pub content_type: Option<String>,
    pub ikaros_signature: Option<String>,
}

pub fn read_http_headers(reader: &mut impl BufRead) -> Result<HttpHeaders> {
    let mut line = String::new();
    let mut headers = HttpHeaders::default();
    for _ in 0..128 {
        line.clear();
        let bytes = reader.read_line(&mut line).map_err(|source| {
            IkarosError::Message(format!("failed to read message webhook headers: {source}"))
        })?;
        if bytes == 0 || line == "\r\n" || line == "\n" {
            return Ok(headers);
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            headers.content_length = Some(value.parse().map_err(|source| {
                IkarosError::Message(format!("invalid message webhook content length: {source}"))
            })?);
        } else if name.eq_ignore_ascii_case("content-type") {
            headers.content_type = Some(value.to_string());
        } else if name.eq_ignore_ascii_case("x-ikaros-signature") {
            headers.ikaros_signature = Some(value.to_string());
        }
    }
    Err(IkarosError::Message(
        "message webhook headers are too large".into(),
    ))
}

pub fn write_webhook_http_response(
    stream: &mut TcpStream,
    response: &MessageWebhookHttpResponse,
    send_body: bool,
) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
        response.status_code,
        response.reason,
        response.content_type,
        response.body.len()
    )
    .map_err(|source| {
        IkarosError::Message(format!(
            "failed to write message webhook response headers: {source}"
        ))
    })?;
    if let Some(allow) = response.allow {
        write!(stream, "Allow: {allow}\r\n").map_err(|source| {
            IkarosError::Message(format!(
                "failed to write message webhook Allow header: {source}"
            ))
        })?;
    }
    write!(stream, "\r\n").map_err(|source| {
        IkarosError::Message(format!(
            "failed to finish message webhook response headers: {source}"
        ))
    })?;
    if send_body {
        stream
            .write_all(response.body.as_bytes())
            .map_err(|source| {
                IkarosError::Message(format!(
                    "failed to write message webhook response body: {source}"
                ))
            })?;
    }
    stream.flush().map_err(|source| {
        IkarosError::Message(format!(
            "failed to flush message webhook response: {source}"
        ))
    })
}
