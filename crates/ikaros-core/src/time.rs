// SPDX-License-Identifier: GPL-3.0-only

use crate::Result;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub fn now_rfc3339() -> Result<String> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}
