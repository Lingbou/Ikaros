// SPDX-License-Identifier: GPL-3.0-only

use super::BodyServe;
use anyhow::{Context, Result};
use ikaros_body::{BodyKind, DashboardRenderOptions, WebDashboardAdapter};
use ikaros_core::IkarosPaths;
use ikaros_runtime::current_body_frame;
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    time::Duration,
};

pub(super) fn serve_body_dashboard(
    args: BodyServe,
    paths: &IkarosPaths,
    workspace: &Path,
) -> Result<()> {
    paths.ensure()?;
    let listener = TcpListener::bind((args.host.as_str(), args.port)).with_context(|| {
        format!(
            "failed to bind dashboard server on {}:{}",
            args.host, args.port
        )
    })?;
    let local_addr = listener
        .local_addr()
        .with_context(|| "failed to read dashboard server address")?;
    println!("dashboard_server: http://{local_addr}/");
    println!("frame_json: http://{local_addr}/frame.json");
    println!("refresh_seconds: {}", args.refresh_seconds.max(1));
    println!("workspace: {}", workspace.display());
    for stream in listener.incoming() {
        let stream = stream.with_context(|| "failed to accept dashboard request")?;
        if let Err(error) = handle_dashboard_stream(
            stream,
            paths,
            workspace,
            args.events,
            args.refresh_seconds.max(1),
        ) {
            eprintln!("dashboard request failed: {error:#}");
        }
        if args.once {
            break;
        }
    }
    Ok(())
}

fn handle_dashboard_stream(
    mut stream: TcpStream,
    paths: &IkarosPaths,
    workspace: &Path,
    event_limit: usize,
    refresh_seconds: u64,
) -> Result<()> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .with_context(|| "failed to set dashboard read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .with_context(|| "failed to set dashboard write timeout")?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .with_context(|| "failed to clone dashboard stream")?,
    );
    let mut request_line = String::new();
    if reader
        .read_line(&mut request_line)
        .with_context(|| "failed to read dashboard request")?
        == 0
    {
        return Ok(());
    }
    let (method, target) = match parse_http_request_line(&request_line) {
        Some(request) => request,
        None => {
            let response = DashboardHttpResponse::plain(400, "Bad Request", "bad request\n");
            return write_dashboard_http_response(&mut stream, &response, true);
        }
    };
    discard_http_headers(&mut reader)?;
    let response = dashboard_http_response(
        method,
        target,
        paths,
        workspace,
        event_limit,
        refresh_seconds,
    )?;
    write_dashboard_http_response(&mut stream, &response, method != "HEAD")
}

pub(super) fn parse_http_request_line(line: &str) -> Option<(&str, &str)> {
    let mut fields = line.split_whitespace();
    let method = fields.next()?;
    let target = fields.next()?;
    let version = fields.next()?;
    if fields.next().is_some() || !version.starts_with("HTTP/") {
        return None;
    }
    Some((method, target))
}

fn discard_http_headers(reader: &mut impl BufRead) -> Result<()> {
    let mut line = String::new();
    for _ in 0..128 {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .with_context(|| "failed to read dashboard request headers")?;
        if bytes == 0 || line == "\r\n" || line == "\n" {
            return Ok(());
        }
    }
    anyhow::bail!("dashboard request headers are too large")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DashboardHttpResponse {
    pub(super) status_code: u16,
    pub(super) reason: &'static str,
    pub(super) content_type: &'static str,
    pub(super) body: String,
    pub(super) allow_get_head: bool,
}

impl DashboardHttpResponse {
    fn plain(status_code: u16, reason: &'static str, body: impl Into<String>) -> Self {
        Self {
            status_code,
            reason,
            content_type: "text/plain; charset=utf-8",
            body: body.into(),
            allow_get_head: false,
        }
    }

    fn method_not_allowed() -> Self {
        Self {
            status_code: 405,
            reason: "Method Not Allowed",
            content_type: "text/plain; charset=utf-8",
            body: "method not allowed\n".into(),
            allow_get_head: true,
        }
    }
}

pub(super) fn dashboard_http_response(
    method: &str,
    target: &str,
    paths: &IkarosPaths,
    _workspace: &Path,
    event_limit: usize,
    refresh_seconds: u64,
) -> Result<DashboardHttpResponse> {
    if method != "GET" && method != "HEAD" {
        return Ok(DashboardHttpResponse::method_not_allowed());
    }
    let route = target.split('?').next().unwrap_or(target);
    match route {
        "/" | "/dashboard.html" => {
            let frame = current_body_frame(paths, event_limit, BodyKind::Web)?;
            Ok(DashboardHttpResponse {
                status_code: 200,
                reason: "OK",
                content_type: "text/html; charset=utf-8",
                body: WebDashboardAdapter.render_frame_with_options(
                    &frame,
                    &DashboardRenderOptions {
                        refresh_seconds: Some(refresh_seconds.max(1)),
                        snapshot_path: Some("/frame.json".into()),
                    },
                ),
                allow_get_head: false,
            })
        }
        "/frame.json" => {
            let frame = current_body_frame(paths, event_limit, BodyKind::Web)?;
            Ok(DashboardHttpResponse {
                status_code: 200,
                reason: "OK",
                content_type: "application/json; charset=utf-8",
                body: serde_json::to_string_pretty(&frame)?,
                allow_get_head: false,
            })
        }
        "/healthz" => Ok(DashboardHttpResponse::plain(200, "OK", "ok\n")),
        _ => Ok(DashboardHttpResponse::plain(
            404,
            "Not Found",
            "not found\n",
        )),
    }
}

fn write_dashboard_http_response(
    stream: &mut TcpStream,
    response: &DashboardHttpResponse,
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
    .with_context(|| "failed to write dashboard response headers")?;
    if response.allow_get_head {
        write!(stream, "Allow: GET, HEAD\r\n")
            .with_context(|| "failed to write dashboard Allow header")?;
    }
    write!(stream, "\r\n").with_context(|| "failed to finish dashboard response headers")?;
    if send_body {
        stream
            .write_all(response.body.as_bytes())
            .with_context(|| "failed to write dashboard response body")?;
    }
    stream
        .flush()
        .with_context(|| "failed to flush dashboard response")
}
