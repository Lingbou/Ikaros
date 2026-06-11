// SPDX-License-Identifier: GPL-3.0-only

use crate::{IkarosError, Result};

pub fn resolve_config_secret(inline_secret: &str, label: impl AsRef<str>) -> Result<String> {
    resolve_config_value(inline_secret, label)
}

pub fn resolve_config_value(value: &str, label: impl AsRef<str>) -> Result<String> {
    let label = label.as_ref();
    let value = value.trim();
    if !value.is_empty() {
        return Ok(value.into());
    }
    Err(IkarosError::Message(format!(
        "{label} is not configured: set it in IKAROS_HOME/config.toml"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_config_secret() {
        let secret = resolve_config_secret(" inline-secret ", "test").expect("secret");
        assert_eq!(secret, "inline-secret");
    }

    #[test]
    fn rejects_empty_config_secret() {
        let error = resolve_config_secret(" ", "test").expect_err("secret error");
        assert!(error.to_string().contains("IKAROS_HOME/config.toml"));
    }
}
