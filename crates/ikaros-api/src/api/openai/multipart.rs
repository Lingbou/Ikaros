// SPDX-License-Identifier: GPL-3.0-only

use super::super::*;

#[derive(Debug)]
pub(in crate::api) struct ApiMultipartForm {
    pub(in crate::api) fields: BTreeMap<String, String>,
    pub(in crate::api) file: Vec<u8>,
    pub(in crate::api) file_name: String,
    pub(in crate::api) file_content_type: String,
}

pub(in crate::api) fn parse_api_multipart_form(
    body: &[u8],
    content_type: &str,
) -> Result<ApiMultipartForm> {
    let boundary = multipart_boundary(content_type)?;
    let delimiter = format!("--{boundary}").into_bytes();
    let mut form = ApiMultipartForm {
        fields: BTreeMap::new(),
        file: Vec::new(),
        file_name: "audio".into(),
        file_content_type: "application/octet-stream".into(),
    };
    for raw_part in split_bytes(body, &delimiter) {
        let part = trim_multipart_part(raw_part);
        if part.is_empty() || part == b"--" {
            continue;
        }
        let Some((raw_headers, raw_body)) = split_once_bytes(part, b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(raw_headers);
        let disposition = headers
            .lines()
            .find(|line| {
                line.to_ascii_lowercase()
                    .starts_with("content-disposition:")
            })
            .unwrap_or_default();
        let Some(name) = multipart_header_param(disposition, "name") else {
            continue;
        };
        let content_type = headers
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-type")
                        .then(|| value.trim().to_owned())
                })
            })
            .unwrap_or_else(|| "application/octet-stream".into());
        let body = trim_multipart_body(raw_body);
        if name == "file" {
            form.file = body.to_vec();
            form.file_name =
                multipart_header_param(disposition, "filename").unwrap_or_else(|| "audio".into());
            form.file_content_type = content_type;
        } else if let Ok(value) = std::str::from_utf8(body) {
            form.fields.insert(name, value.trim().to_owned());
        }
    }
    if form.file.is_empty() {
        anyhow::bail!("audio transcription multipart body must include a non-empty file part");
    }
    Ok(form)
}

pub(in crate::api) fn multipart_boundary(content_type: &str) -> Result<String> {
    if !content_type
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        anyhow::bail!("audio transcription requires multipart/form-data");
    }
    for part in content_type.split(';').skip(1) {
        let Some((name, value)) = part.trim().split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("boundary") {
            let boundary = value.trim().trim_matches('"').to_owned();
            if !boundary.is_empty() {
                return Ok(boundary);
            }
        }
    }
    anyhow::bail!("multipart/form-data content-type is missing boundary")
}

pub(in crate::api) fn api_asr_multipart_body(
    model: &str,
    form: &ApiMultipartForm,
) -> (String, Vec<u8>) {
    let boundary = "ikaros-api-asr-boundary";
    let mut body = Vec::new();
    push_api_multipart_text(&mut body, boundary, "model", model);
    for (name, value) in &form.fields {
        if name == "model" || name == "file" {
            continue;
        }
        push_api_multipart_text(&mut body, boundary, name, value);
    }
    push_api_multipart_file(
        &mut body,
        boundary,
        "file",
        &form.file_name,
        &form.file_content_type,
        &form.file,
    );
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

pub(in crate::api) fn push_api_multipart_text(
    body: &mut Vec<u8>,
    boundary: &str,
    name: &str,
    value: &str,
) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{}\"\r\n\r\n",
            sanitize_multipart_token(name)
        )
        .as_bytes(),
    );
    body.extend_from_slice(value.as_bytes());
    body.extend_from_slice(b"\r\n");
}

pub(in crate::api) fn push_api_multipart_file(
    body: &mut Vec<u8>,
    boundary: &str,
    name: &str,
    file_name: &str,
    content_type: &str,
    file: &[u8],
) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n",
            sanitize_multipart_token(name),
            sanitize_multipart_token(file_name)
        )
        .as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "Content-Type: {}\r\n\r\n",
            safe_multipart_content_type(content_type)
        )
        .as_bytes(),
    );
    body.extend_from_slice(file);
    body.extend_from_slice(b"\r\n");
}

pub(in crate::api) fn sanitize_multipart_token(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '"' | '\r' | '\n' => '_',
            _ => ch,
        })
        .collect()
}

pub(in crate::api) fn safe_multipart_content_type(value: &str) -> String {
    let cleaned = value
        .chars()
        .filter(|ch| !matches!(ch, '\r' | '\n'))
        .collect::<String>();
    if cleaned.trim().is_empty() {
        "application/octet-stream".into()
    } else {
        cleaned
    }
}

pub(in crate::api) fn multipart_header_param(header: &str, target: &str) -> Option<String> {
    for part in header.split(';').skip(1) {
        let Some((name, value)) = part.trim().split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(target) {
            return Some(value.trim().trim_matches('"').to_owned());
        }
    }
    None
}

pub(in crate::api) fn split_bytes<'a>(value: &'a [u8], delimiter: &[u8]) -> Vec<&'a [u8]> {
    let mut parts = Vec::new();
    let mut start = 0;
    while let Some(offset) = find_bytes(&value[start..], delimiter) {
        parts.push(&value[start..start + offset]);
        start += offset + delimiter.len();
    }
    parts.push(&value[start..]);
    parts
}

pub(in crate::api) fn split_once_bytes<'a>(
    value: &'a [u8],
    delimiter: &[u8],
) -> Option<(&'a [u8], &'a [u8])> {
    let offset = find_bytes(value, delimiter)?;
    Some((&value[..offset], &value[offset + delimiter.len()..]))
}

pub(in crate::api) fn find_bytes(value: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || value.len() < needle.len() {
        return None;
    }
    value
        .windows(needle.len())
        .position(|window| window == needle)
}

pub(in crate::api) fn trim_multipart_part(mut value: &[u8]) -> &[u8] {
    while value.starts_with(b"\r\n") {
        value = &value[2..];
    }
    if value.ends_with(b"--\r\n") {
        value = &value[..value.len().saturating_sub(4)];
    } else if value.ends_with(b"--") {
        value = &value[..value.len().saturating_sub(2)];
    }
    value
}

pub(in crate::api) fn trim_multipart_body(mut value: &[u8]) -> &[u8] {
    if value.ends_with(b"\r\n") {
        value = &value[..value.len().saturating_sub(2)];
    }
    value
}
