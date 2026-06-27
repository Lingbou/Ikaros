// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub const IKAROS_PROTOCOL_VERSION: u32 = 1;
pub const IKAROS_PROTOCOL_NAME: &str = "ikaros-protocol";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WireEnvelope<T> {
    pub protocol: String,
    pub version: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub kind: String,
    pub data: T,
}

impl<T> WireEnvelope<T> {
    pub fn new(kind: impl Into<String>, data: T) -> Self {
        Self {
            protocol: IKAROS_PROTOCOL_NAME.into(),
            version: IKAROS_PROTOCOL_VERSION,
            at: OffsetDateTime::now_utc(),
            kind: kind.into(),
            data,
        }
    }
}
