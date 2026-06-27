// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SelfModifyConfig {
    pub check_profiles: BTreeMap<String, SelfModifyCheckProfileConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SelfModifyCheckProfileConfig {
    pub commands: Vec<String>,
    pub reason: Option<String>,
}
