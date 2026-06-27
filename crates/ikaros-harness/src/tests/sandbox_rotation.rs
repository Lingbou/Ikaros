// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn sandbox_debug_report_exposes_available_and_planned_isolation_levels() {
    let matrix = crate::sandbox_isolation_matrix();
    assert!(matrix.iter().any(|entry| {
        entry.level == crate::SandboxIsolationLevel::WorkspaceOnly
            && entry.status == crate::SandboxIsolationStatus::Available
    }));
    assert!(matrix.iter().any(|entry| {
        entry.level == crate::SandboxIsolationLevel::NetworkRestricted
            && entry.status == crate::SandboxIsolationStatus::Available
    }));
    assert!(matrix.iter().any(|entry| {
        entry.level == crate::SandboxIsolationLevel::Container
            && entry.status == crate::SandboxIsolationStatus::Available
    }));

    let local = crate::local_sandbox_debug_report("local", true, None);
    assert_eq!(local.level, crate::SandboxIsolationLevel::NetworkRestricted);
    assert!(local.cwd_enforced);
    assert!(local.env_allowlist);
    #[cfg(unix)]
    assert_eq!(local.process_timeout_strategy, "process_group_unix");
    #[cfg(not(unix))]
    assert_eq!(local.process_timeout_strategy, "direct_child_kill");
    assert_eq!(local.network_egress, "governed");
    assert!(!local.allow_provider_hosts);
    assert_eq!(local.configured_allowed_host_count, 0);
    assert_eq!(local.effective_allowed_host_count, 0);
    assert_eq!(local.host_allowlist_mode, "configured_hosts_only");
    assert!(local.restricted_ip_literal_block);
    assert!(local.dns_rebind_block);
    assert_eq!(local.loopback_exception, "explicit_loopback_hosts_only");
    assert_eq!(
        local.process_network_isolation,
        "not_enforced_without_container_backend"
    );
    assert!(
        local
            .notes
            .iter()
            .any(|note| note.contains("final-path symlink swaps")),
        "sandbox diagnostics should expose final-path symlink swap protection: {:?}",
        local.notes
    );
    assert!(
        local
            .notes
            .iter()
            .any(|note| note.contains("output caps kill the spawned process group")),
        "sandbox diagnostics should expose process-group limit behavior: {:?}",
        local.notes
    );

    let dry_run = crate::local_sandbox_debug_report("dry-run", false, None);
    assert_eq!(dry_run.level, crate::SandboxIsolationLevel::DryRun);
    assert_eq!(dry_run.network_egress, "deny_by_default");
    assert_eq!(dry_run.host_allowlist_mode, "deny_by_default");

    let docker = crate::local_sandbox_debug_report("docker", true, Some("rust:1.85-bookworm"));
    assert_eq!(docker.level, crate::SandboxIsolationLevel::Container);
    assert_eq!(
        docker.configured_image.as_deref(),
        Some("rust:1.85-bookworm")
    );
    assert_eq!(docker.workspace_mount.as_deref(), Some("/workspace"));
    assert_eq!(docker.plugin_mount.as_deref(), Some("/plugin"));
    assert_eq!(docker.process_network_isolation, "docker_network_none");
}

#[test]
fn audit_log_rotates_by_size_and_reads_compressed_archive() {
    let temp = tempfile::tempdir().expect("tempdir");
    let audit =
        AuditLog::from_file(temp.path().join("audit.jsonl")).with_rotation(AuditRotationPolicy {
            max_bytes: 1,
            rotate_on_date_change: false,
        });
    let first = AuditEvent::new(
        "first",
        None,
        "first audit event",
        json!({"payload": "a".repeat(256)}),
    )
    .expect("first event");
    let first_id = first.id.clone();
    audit.append(first).expect("append first");

    let second =
        AuditEvent::new("second", None, "second audit event", json!({})).expect("second event");
    let second_id = second.id.clone();
    audit.append(second).expect("append second");

    let archives = compressed_audit_archives(temp.path());
    assert_eq!(archives.len(), 1);
    let active = fs::read_to_string(audit.path()).expect("active audit");
    assert!(!active.contains(&first_id));
    assert!(active.contains(&second_id));
    let ids = audit
        .read_all()
        .expect("events")
        .into_iter()
        .map(|event| event.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![first_id, second_id]);
}

#[test]
fn audit_log_rotates_by_event_date_and_reads_compressed_archive() {
    let temp = tempfile::tempdir().expect("tempdir");
    let audit =
        AuditLog::from_file(temp.path().join("audit.jsonl")).with_rotation(AuditRotationPolicy {
            max_bytes: 0,
            rotate_on_date_change: true,
        });
    let first = audit_event_at("first", "2026-06-10T23:59:00Z");
    let first_id = first.id.clone();
    audit.append(first).expect("append first");

    let second = audit_event_at("second", "2026-06-11T00:00:00Z");
    let second_id = second.id.clone();
    audit.append(second).expect("append second");

    assert_eq!(compressed_audit_archives(temp.path()).len(), 1);
    let ids = audit
        .read_all()
        .expect("events")
        .into_iter()
        .map(|event| event.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![first_id, second_id]);
}
