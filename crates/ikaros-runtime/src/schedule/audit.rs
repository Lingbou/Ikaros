// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosPaths, Result};
use ikaros_harness::{AuditEvent, AuditLog};

pub(super) fn append_schedule_delivery_audit(
    paths: &IkarosPaths,
    kind: &str,
    message: &str,
    data: serde_json::Value,
) -> Result<()> {
    AuditLog::new(&paths.audit_dir).append(AuditEvent::new(kind, None, message, data)?)
}
