// SPDX-License-Identifier: GPL-3.0-only

use ikaros_state::memory::{MemoryCandidateStatus, MemoryKind};

pub(super) fn parse_candidate_status(status: &str) -> ikaros_core::Result<MemoryCandidateStatus> {
    match status.to_ascii_lowercase().as_str() {
        "pending" => Ok(MemoryCandidateStatus::Pending),
        "accepted" => Ok(MemoryCandidateStatus::Accepted),
        "rejected" => Ok(MemoryCandidateStatus::Rejected),
        "expired" => Ok(MemoryCandidateStatus::Expired),
        other => Err(ikaros_core::IkarosError::Message(format!(
            "unsupported memory candidate status: {other}"
        ))),
    }
}

pub(super) fn parse_memory_kind(kind: &str) -> ikaros_core::Result<MemoryKind> {
    match kind.to_ascii_lowercase().as_str() {
        "user" => Ok(MemoryKind::User),
        "project" => Ok(MemoryKind::Project),
        "task" => Ok(MemoryKind::Task),
        "persona" => Ok(MemoryKind::Persona),
        "relationship" => Ok(MemoryKind::Relationship),
        "knowledge" => Ok(MemoryKind::Knowledge),
        other => Err(ikaros_core::IkarosError::Message(format!(
            "unsupported memory kind: {other}"
        ))),
    }
}
