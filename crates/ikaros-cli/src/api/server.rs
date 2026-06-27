// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::api) fn serve_api(
    args: ApiServe,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    paths.ensure()?;
    require_loopback_host(&args.host)?;
    let listener = TcpListener::bind((args.host.as_str(), args.port))
        .with_context(|| format!("failed to bind API server on {}:{}", args.host, args.port))?;
    let local_addr = listener
        .local_addr()
        .with_context(|| "failed to read API server address")?;
    println!("api_server: http://{local_addr}");
    println!("chat_completions: http://{local_addr}/v1/chat/completions");
    println!("responses: http://{local_addr}/v1/responses");
    println!("embeddings: http://{local_addr}/v1/embeddings");
    println!("images: http://{local_addr}/v1/images/generations");
    println!("audio_speech: http://{local_addr}/v1/audio/speech");
    println!("audio_transcriptions: http://{local_addr}/v1/audio/transcriptions");
    println!("models: http://{local_addr}/v1/models");
    println!("protocol: http://{local_addr}/v1/ikaros/protocol");
    println!("health: http://{local_addr}/healthz");
    println!("workspace: {}", workspace.display());
    let state = ApiServerState::new(args.bearer_token, args.rate_limit_per_minute);
    for stream in listener.incoming() {
        let stream = stream.with_context(|| "failed to accept API request")?;
        if let Err(error) = handle_api_stream(
            stream,
            paths,
            workspace,
            agent_override,
            args.max_body_bytes,
            &state,
        ) {
            eprintln!("api request failed: {error:#}");
        }
        if args.once {
            break;
        }
    }
    Ok(())
}

pub(in crate::api) fn require_loopback_host(host: &str) -> Result<()> {
    if host.eq_ignore_ascii_case("localhost") {
        return Ok(());
    }
    let parsed: IpAddr = host
        .parse()
        .with_context(|| format!("API server host must be loopback: {host}"))?;
    if !parsed.is_loopback() {
        anyhow::bail!(
            "API server only binds loopback addresses in the MVP; got {}",
            host
        );
    }
    Ok(())
}

pub(in crate::api) fn handle_api_stream(
    mut stream: TcpStream,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    max_body_bytes: usize,
    state: &ApiServerState,
) -> Result<()> {
    let peer_addr = stream.peer_addr().ok();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .with_context(|| "failed to set API read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .with_context(|| "failed to set API write timeout")?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .with_context(|| "failed to clone API stream")?,
    );
    let mut request_line = String::new();
    if reader
        .read_line(&mut request_line)
        .with_context(|| "failed to read API request")?
        == 0
    {
        return Ok(());
    }
    let Some((method, target)) = parse_api_request_line(&request_line) else {
        let response = ApiHttpResponse::json_error(400, "Bad Request", "bad request");
        audit_api_request(paths, peer_addr, "UNKNOWN", "UNKNOWN", &response, None);
        return write_api_http_response(&mut stream, &response, true);
    };
    let headers = read_api_headers(&mut reader)?;
    let route = target.split('?').next().unwrap_or(target);
    let mut body = Vec::new();
    if method == "POST" {
        let content_length = match headers.content_length {
            Some(value) => value,
            None => {
                let response =
                    ApiHttpResponse::json_error(411, "Length Required", "length required");
                audit_api_request(paths, peer_addr, method, route, &response, Some(&headers));
                return write_api_http_response(&mut stream, &response, true);
            }
        };
        if content_length > max_body_bytes {
            let response =
                ApiHttpResponse::json_error(413, "Payload Too Large", "payload too large");
            audit_api_request(paths, peer_addr, method, route, &response, Some(&headers));
            return write_api_http_response(&mut stream, &response, true);
        }
        body.resize(content_length, 0);
        reader
            .read_exact(&mut body)
            .with_context(|| "failed to read API body")?;
    }
    let response = match state.security_response(route, &headers) {
        Some(response) => response,
        None => {
            match api_http_response(
                method,
                route,
                &body,
                &headers,
                paths,
                workspace,
                agent_override,
            ) {
                Ok(response) => response,
                Err(error) => ApiHttpResponse::internal_error(error),
            }
        }
    };
    audit_api_request(paths, peer_addr, method, route, &response, Some(&headers));
    write_api_http_response(&mut stream, &response, method != "HEAD")
}

pub(in crate::api) fn parse_api_request_line(line: &str) -> Option<(&str, &str)> {
    let mut fields = line.split_whitespace();
    let method = fields.next()?;
    let target = fields.next()?;
    let version = fields.next()?;
    if fields.next().is_some() || !version.starts_with("HTTP/") {
        return None;
    }
    Some((method, target))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::api) struct ApiHeaders {
    pub(in crate::api) content_length: Option<usize>,
    pub(in crate::api) content_type: Option<String>,
    pub(in crate::api) authorization: Option<String>,
    pub(in crate::api) client_id: Option<String>,
}

pub(in crate::api) fn read_api_headers(reader: &mut impl BufRead) -> Result<ApiHeaders> {
    let mut headers = ApiHeaders::default();
    let mut line = String::new();
    for _ in 0..128 {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .with_context(|| "failed to read API request headers")?;
        if bytes == 0 || line == "\r\n" || line == "\n" {
            return Ok(headers);
        }
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim();
            let value = value.trim();
            if name.eq_ignore_ascii_case("content-length") {
                headers.content_length = Some(value.parse().with_context(|| {
                    format!("invalid content-length header `{}`", redact_secrets(value))
                })?);
            } else if name.eq_ignore_ascii_case("content-type") {
                headers.content_type = Some(value.to_owned());
            } else if name.eq_ignore_ascii_case("authorization") {
                headers.authorization = Some(value.to_owned());
            } else if name.eq_ignore_ascii_case("x-ikaros-client-id") {
                headers.client_id = Some(value.to_owned());
            }
        }
    }
    anyhow::bail!("API request headers are too large")
}

#[derive(Debug, Clone)]
pub(in crate::api) struct ApiServerState {
    pub(in crate::api) bearer_tokens: Vec<String>,
    pub(in crate::api) rate_limiter: Option<Arc<Mutex<ApiRateLimiter>>>,
}

impl ApiServerState {
    pub(in crate::api) fn new(bearer_tokens: Vec<String>, rate_limit_per_minute: u32) -> Self {
        Self {
            bearer_tokens: bearer_tokens
                .into_iter()
                .map(|token| token.trim().to_owned())
                .filter(|token| !token.is_empty())
                .collect(),
            rate_limiter: (rate_limit_per_minute > 0)
                .then(|| Arc::new(Mutex::new(ApiRateLimiter::new(rate_limit_per_minute)))),
        }
    }

    pub(in crate::api) fn security_response(
        &self,
        route: &str,
        headers: &ApiHeaders,
    ) -> Option<ApiHttpResponse> {
        if let Some(limiter) = &self.rate_limiter {
            let mut limiter = limiter
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Err(retry_after_seconds) = limiter.check() {
                return Some(ApiHttpResponse::rate_limited(retry_after_seconds));
            }
        }
        if !is_api_health_route(route)
            && !self.bearer_tokens.is_empty()
            && !bearer_token_matches_any(headers.authorization.as_deref(), &self.bearer_tokens)
        {
            return Some(ApiHttpResponse::unauthorized());
        }
        None
    }
}

#[derive(Debug, Clone)]
pub(in crate::api) struct ApiRateLimiter {
    pub(in crate::api) limit: u32,
    pub(in crate::api) window_started: Instant,
    pub(in crate::api) used: u32,
}

impl ApiRateLimiter {
    pub(in crate::api) fn new(limit: u32) -> Self {
        Self {
            limit,
            window_started: Instant::now(),
            used: 0,
        }
    }

    pub(in crate::api) fn check(&mut self) -> std::result::Result<(), u64> {
        let elapsed = self.window_started.elapsed();
        if elapsed >= Duration::from_secs(60) {
            self.window_started = Instant::now();
            self.used = 0;
        }
        if self.used >= self.limit {
            let retry_after = 60_u64.saturating_sub(elapsed.as_secs()).max(1);
            return Err(retry_after);
        }
        self.used = self.used.saturating_add(1);
        Ok(())
    }
}

pub(in crate::api) fn bearer_token_matches_any(
    header: Option<&str>,
    expected_tokens: &[String],
) -> bool {
    let Some(header) = header else {
        return false;
    };
    let Some(actual) = header.trim().strip_prefix("Bearer ") else {
        return false;
    };
    let actual = actual.trim().as_bytes();
    expected_tokens.iter().fold(false, |matched, expected| {
        matched | constant_time_eq(actual, expected.as_bytes())
    })
}

pub(in crate::api) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    for (left, right) in left.iter().zip(right.iter()) {
        diff |= usize::from(left ^ right);
    }
    for byte in left.iter().skip(right.len()) {
        diff |= usize::from(*byte);
    }
    for byte in right.iter().skip(left.len()) {
        diff |= usize::from(*byte);
    }
    diff == 0
}
