// SPDX-License-Identifier: GPL-3.0-only

use std::path::PathBuf;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, IkarosError>;

#[derive(Debug, Error)]
pub enum IkarosError {
    #[error("environment variable {0} is not set and no home directory could be inferred")]
    MissingHome(String),
    #[error("path is outside the configured scope: {0}")]
    OutOfScope(PathBuf),
    #[error("secret-like value rejected: {0}")]
    SecretRejected(String),
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("toml parse error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("toml serialize error: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("yaml parse error: {0}")]
    Yaml(#[from] yaml_serde::Error),
    #[error("time format error: {0}")]
    Time(#[from] time::error::Format),
    #[error("{0}")]
    Message(String),
}

impl IkarosError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
