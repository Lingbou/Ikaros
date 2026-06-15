// SPDX-License-Identifier: GPL-3.0-only

mod chat;
mod client;
mod profile;
mod request_builder;
mod schema_sanitizer;
mod stream;
mod tools;
mod types;

pub use client::OpenAiCompatibleProvider;

#[cfg(test)]
pub(crate) use chat::{
    parse_chat_completion_response, redacted_model_http_error, unsupported_parameter_to_omit,
};
#[cfg(test)]
pub(crate) use profile::OpenAiCompatProfile;
#[cfg(test)]
pub(crate) use stream::parse_stream_response;
