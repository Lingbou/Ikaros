// SPDX-License-Identifier: GPL-3.0-only

use std::{fs, path::Path};

use crate::{IkarosError, Result};

use super::{IkarosConfig, validation};

impl IkarosConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let mut config = Self::load_shape_checked(path)?;
        config.expand_presets();
        let report = config.validate();
        if !report.is_valid() {
            return Err(IkarosError::Message(validation::format_validation_failure(
                "configuration validation failed",
                &report,
            )));
        }
        Ok(config)
    }

    pub fn load_shape_checked(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(IkarosError::Message(format!(
                "config file not found: {}; run `ikaros init` to create config.yaml under IKAROS_HOME",
                path.display()
            )));
        }
        let raw = fs::read_to_string(path).map_err(|source| IkarosError::io(path, source))?;
        validation::load_yaml_shape_checked(&raw)
    }

    pub fn load_yaml_shape_checked(raw: &str) -> Result<Self> {
        validation::load_yaml_shape_checked(raw)
    }

    pub fn write_default_config(path: &Path) -> Result<()> {
        let raw = r#"schema_version: 1

model:
  default:
    preset: auto
    model: ""
    api_key: ""
    base_url: ""
"#;
        fs::write(path, raw).map_err(|source| IkarosError::io(path, source))
    }

    pub fn write_full_config(path: &Path) -> Result<()> {
        let raw = yaml_serde::to_string(&IkarosConfig::default()).map_err(|source| {
            IkarosError::Message(format!("failed to serialize default config: {source}"))
        })?;
        fs::write(path, raw).map_err(|source| IkarosError::io(path, source))
    }
}
