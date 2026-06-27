// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result};

pub(in crate::web) fn bounded_usize(
    input: &serde_json::Value,
    field: &str,
    default: usize,
    minimum: usize,
    maximum: usize,
) -> Result<usize> {
    let Some(value) = input.get(field) else {
        return Ok(default);
    };
    let raw = value
        .as_u64()
        .ok_or_else(|| IkarosError::Message(format!("{field} must be a positive integer")))?;
    let value =
        usize::try_from(raw).map_err(|_| IkarosError::Message(format!("{field} is too large")))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(IkarosError::Message(format!(
            "{field} must be between {minimum} and {maximum}"
        )));
    }
    Ok(value)
}
