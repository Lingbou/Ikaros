// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};

use super::{MemoryKind, MemoryPerspective};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryQuery {
    pub kind: Option<MemoryKind>,
    pub scope: Option<String>,
    pub perspective: Option<MemoryPerspective>,
    pub text: Option<String>,
    pub limit: Option<usize>,
    #[serde(default)]
    pub include_inactive: bool,
}
