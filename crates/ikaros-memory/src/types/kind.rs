// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemoryKind {
    User,
    Project,
    Task,
    Persona,
    Relationship,
    Knowledge,
}
