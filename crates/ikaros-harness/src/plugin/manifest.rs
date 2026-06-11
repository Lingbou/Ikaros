// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::RiskLevel;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;

pub const PLUGIN_COMMAND_MAX_ARGS: usize = 32;
pub const PLUGIN_COMMAND_MAX_ARG_BYTES: usize = 1024;
pub const PLUGIN_COMMAND_MAX_TIMEOUT_MS: u64 = 30_000;
pub const PLUGIN_COMMAND_MAX_STDIN_BYTES: usize = 64 * 1024;
pub const PLUGIN_COMMAND_MAX_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub skills: Vec<PluginSkillManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginSkillManifest {
    pub name: String,
    pub description: String,
    #[serde(deserialize_with = "deserialize_risk_level")]
    pub risk: RiskLevel,
    #[serde(default = "default_input_schema")]
    pub input_schema: serde_json::Value,
    #[serde(default)]
    pub permissions: Vec<PluginPermissionDeclaration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<PluginCommandManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginCommandManifest {
    pub program: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginPermissionDeclaration {
    pub action: String,
    #[serde(deserialize_with = "deserialize_risk_level")]
    pub risk: RiskLevel,
    #[serde(default)]
    pub paths: Vec<PathBuf>,
    #[serde(default)]
    pub network: bool,
}

fn default_input_schema() -> serde_json::Value {
    json!({"type": "object", "properties": {}})
}

fn deserialize_risk_level<'de, D>(deserializer: D) -> std::result::Result<RiskLevel, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    parse_risk_level(&value).map_err(serde::de::Error::custom)
}

fn parse_risk_level(value: &str) -> std::result::Result<RiskLevel, String> {
    match value
        .chars()
        .filter(|ch| *ch != '_' && *ch != '-' && !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .as_str()
    {
        "saferead" => Ok(RiskLevel::SafeRead),
        "localwrite" => Ok(RiskLevel::LocalWrite),
        "shellread" => Ok(RiskLevel::ShellRead),
        "shellwrite" => Ok(RiskLevel::ShellWrite),
        "network" => Ok(RiskLevel::Network),
        "databasewrite" => Ok(RiskLevel::DatabaseWrite),
        "remoteserver" => Ok(RiskLevel::RemoteServer),
        "destructive" => Ok(RiskLevel::Destructive),
        "secretaccess" => Ok(RiskLevel::SecretAccess),
        _ => Err(format!("unknown risk level: {value}")),
    }
}
