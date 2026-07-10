// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::debug) fn debug_readiness(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let output = debug_readiness_report(paths, workspace, agent_override)?;
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

pub(crate) fn debug_readiness_json_line(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<String> {
    let output = debug_readiness_report(paths, workspace, agent_override)?;
    Ok(format!(
        "readiness_json: {}",
        serde_json::to_string(&redact_json(output))?
    ))
}

pub(in crate::debug) fn debug_readiness_report(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Value> {
    let doctor = runtime_doctor_report(paths, workspace, agent_override)?;
    let host = host_agent_context_shape_checked(paths, workspace, agent_override)?;
    let config = &host.config;
    let agent = &host.agent_instance;
    let state_report = SqliteSessionStore::new(&agent.state_dir).operational_report()?;
    let logs = collect_debug_logs(paths, DebugLogSource::All)?;
    let provider_matrix = provider_debug_matrix_report(config, agent, &paths.audit_dir)?;
    let provider_rows = provider_matrix.rows;
    let provider_attention = provider_rows.iter().any(|row| {
        row.get("live_smoke")
            .and_then(Value::as_str)
            .is_some_and(|state| matches!(state, "missing-api-key" | "missing-base-url"))
    });
    let sandbox_backend = doctor.execution.sandbox_backend.as_str();
    let sandbox_status = match sandbox_backend {
        "docker" => "first_slice",
        "dry-run" | "dry_run" | "local" => "partial",
        _ => "partial",
    };
    let mut rows = vec![
        readiness_row(
            "m6_terminal_workbench",
            "full-screen terminal workbench with navigable timeline, panels, and inline actions",
            "first_slice",
            [
                "ratatui/crossterm screen renderer is available",
                "commands: ikaros, ikaros chat, /screen",
                "turn progress updates refresh the fullscreen status panel",
                "pending approvals expose modal metadata, a modal-marked side panel, and a centered ratatui approval overlay",
                "current limitation: not a complete async TUI application",
            ],
            [
                "fully async event-driven screen redraw",
                "PTY snapshot and long interactive soak coverage",
            ],
        ),
        readiness_row(
            "m6_readline_input",
            "terminal input editing with history, completion, multiline, paste, and cancellation",
            "first_slice",
            [
                "raw-mode line editor is enabled for TTY sessions",
                "non-TTY fallback remains available",
                "commands: /history, /multi, /cancel",
            ],
            ["real PTY smoke coverage", "long-running raw input soak"],
        ),
        readiness_row(
            "m6_approval",
            "inline approval handling and replay evidence",
            "first_slice",
            [
                "workbench approval overlay emits approval_overlay_json",
                "selected screen actions can approve or deny pending requests",
                "approval decisions are mirrored into session events",
                "approval continuation emits workbench_approval_continue_json",
            ],
            [
                "approval resume UX polishing for multi-step waits",
                "negative replay coverage for stale or mismatched approvals",
            ],
        ),
        readiness_row(
            "m6_provider_matrix",
            "provider status, cost, cache, health, fallback, and live probe visibility",
            if provider_attention {
                "partial"
            } else {
                "first_slice"
            },
            [
                "debug provider and /provider matrix expose model/embedding/TTS/ASR rows",
                "workbench screen includes provider cost, health, and fallback cells",
                "retry/fallback diagnostics are typed model events",
            ],
            [
                "broader vendor live matrix",
                "cost catalog hardening",
                "fallback debug navigation in fullscreen UI",
            ],
        ),
        readiness_row(
            "m6_session_replay",
            "session timeline and replay evidence for context, memory, tools, approvals, provider, and coding",
            if state_report.integrity_check.ok {
                "first_slice"
            } else {
                "partial"
            },
            [
                "state.db operational report is available",
                "debug trace/session/context-diff/memory-lifecycle/coding-turn commands exist",
                "screen timeline/replay cells are derived from session store",
            ],
            [
                "long-running replay pagination soak",
                "cross-entry replay consistency audit",
            ],
        ),
        readiness_row(
            "m6_context_memory_rag",
            "context, memory, and RAG evidence panels are visible without default RAG pollution",
            "first_slice",
            [
                "workbench screen includes context budget/reference cells plus memory and RAG status cells",
                "debug context and memory lifecycle commands expose structured evidence",
                "RAG remains explicit/profile-driven instead of default ordinary-chat injection",
                "runtime chat resolves context engines through ContextEngineRegistry and supports provider-backed llm-summary compaction",
            ],
            [
                "semantic compression quality and fallback hardening",
                "memory projection diff/debug polish",
                "real embedding/vector index hardening",
            ],
        ),
        readiness_row(
            "m6_chat_attachments",
            "chat attachment input for image, audio, and file content blocks",
            "first_slice",
            [
                "chat accepts pending image/audio/file attachments and emits attachments_json",
                "provider descriptors expose image_input, audio_input, and file_input capability flags",
                "fallback providers skip unsupported content blocks and surface typed diagnostics",
            ],
            [
                "image resize/compress helper before sending large local attachments",
                "provider-backed smoke coverage for supported content block inputs",
            ],
        ),
        readiness_row(
            "mcp_protocol",
            "Harness-managed MCP server/status/probe paths without passive external process startup",
            "partial",
            [
                "ikaros mcp serve-stdio exposes registered skills over stdio JSON-RPC",
                "ikaros mcp status and workbench /mcp status are read-only",
                "mcp stdio probe is executed as an explicit harness-managed skill",
                "mcp HTTP probe and one-shot tools/call go through runtime NetworkEgress",
            ],
            [
                "external MCP client lifecycle management",
                "dynamic discovery/reload/status/shutdown controls",
                "OAuth and credential flow",
            ],
        ),
        readiness_row(
            "m6_security_sandbox",
            "sandbox and ExecutionEnv boundaries for files, processes, network, and secrets",
            sandbox_status,
            [
                "ExecutionEnv is used for workspace/process/network guarded paths",
                "sandbox debug report and isolation matrix are available",
                "`ikaros debug sandbox --probe` verifies the configured process backend",
                "network egress is governed by deny-by-default exact-host allowlist",
                "restricted IP literals and DNS rebind to restricted addresses are blocked",
            ],
            [
                "true process/container sandbox hardening",
                "cross-platform long-running egress smoke",
                "cross-platform sandbox smoke",
            ],
        ),
        readiness_row(
            "m6_observability",
            "structured traces, logs, insights, dump, and correlation-visible debug surfaces",
            if logs.trace_count > 0 {
                "first_slice"
            } else {
                "partial"
            },
            [
                "debug logs, debug insights, and debug dump are available",
                "trace.jsonl is the structured CLI trace sink",
                "session/turn correlation ids are shown in trace and timeline views",
            ],
            [
                "OTel exporter",
                "Prometheus metrics",
                "broader trace capture tests",
            ],
        ),
        readiness_row(
            "m6_tests",
            "MVP stability coverage for TUI, parser fuzzing, provider live matrix, sandbox, and long-running flows",
            "needs_tests",
            [
                "CONTRIBUTING.md tracks deferred test debt",
                "targeted crate checks are used while functionality is moving",
            ],
            [
                "PTY TUI snapshot tests",
                "parser fuzz/property harness expansion",
                "live provider and long-running agent soak",
            ],
        ),
    ];
    let missing_rows = rows
        .iter()
        .filter(|row| row.get("status").and_then(Value::as_str) == Some("missing"))
        .count();
    if !doctor.config.valid {
        rows.push(readiness_row(
            "local_config",
            "current config.yaml is valid enough for runtime use",
            "partial",
            ["config validation reported errors or warnings"],
            ["run ikaros config validate and fix reported paths"],
        ));
    }
    Ok(json!({
        "format": "ikaros-readiness-v1",
        "status": if missing_rows == 0 { "in_progress" } else { "incomplete" },
        "home": paths.home.display().to_string(),
        "workspace": agent.workspace.display().to_string(),
        "agent": {
            "agent_id": agent.agent_id,
            "profile": agent.profile_name,
            "mode": doctor.agent.mode,
        },
        "config": {
            "valid": doctor.config.valid,
            "issue_count": doctor.config.issues.len(),
            "daily_token_budget_status": doctor.model.daily_token_budget_status,
        },
        "state_db": {
            "path": state_report.path.display().to_string(),
            "integrity_ok": state_report.integrity_check.ok,
            "journal_mode": state_report.journal_mode,
            "search_indexes": state_report.search_indexes,
        },
        "provider_summary": {
            "health_log": provider_matrix.health_log.display().to_string(),
            "row_count": provider_rows.len(),
            "attention": provider_attention,
        },
        "mcp_summary": mcp_debug_summary(config),
        "memory_summary": memory_debug_summary(config, paths)?,
        "rag_summary": rag_debug_summary(config, paths),
        "rows": rows,
        "note": "first_slice means usable but not complete; this report intentionally does not mark PRD completion.",
    }))
}

pub(in crate::debug) fn readiness_row<const E: usize, const N: usize>(
    area: &str,
    requirement: &str,
    status: &str,
    evidence: [&str; E],
    next: [&str; N],
) -> Value {
    json!({
        "area": area,
        "requirement": requirement,
        "status": status,
        "evidence": evidence.into_iter().collect::<Vec<_>>(),
        "next": next.into_iter().collect::<Vec<_>>(),
    })
}

pub(in crate::debug) fn mcp_debug_summary(config: &IkarosConfig) -> Value {
    json!({
        "servers_total": config.mcp.servers.len(),
        "servers_enabled": config.mcp.servers.iter().filter(|server| server.enabled).count(),
        "stdio_servers": config
            .mcp
            .servers
            .iter()
            .filter(|server| server.transport.trim() == "stdio")
            .count(),
        "probe_policy": "explicit_command_required",
        "servers": config.mcp.servers.iter().map(|server| {
            json!({
                "id": safe_debug_string(&server.id),
                "enabled": server.enabled,
                "transport": safe_debug_string(&server.transport),
                "command": safe_debug_string(&server.command),
                "args_count": server.args.len(),
                "include_tools": server.include_tools.iter().map(|tool| safe_debug_string(tool)).collect::<Vec<_>>(),
                "exclude_tools": server.exclude_tools.iter().map(|tool| safe_debug_string(tool)).collect::<Vec<_>>(),
                "timeout_ms": server.timeout_ms,
                "max_output_bytes": server.max_output_bytes,
            })
        }).collect::<Vec<_>>(),
    })
}

pub(in crate::debug) fn memory_debug_summary(
    config: &IkarosConfig,
    paths: &IkarosPaths,
) -> Result<Value> {
    let journal_entries = JsonlMemoryJournal::new(&paths.memory_dir).list()?.len();
    Ok(json!({
        "backend": safe_debug_string(&config.memory.backend),
        "memory_dir": paths.memory_dir.display().to_string(),
        "external_providers": config.memory.external_providers.len(),
        "projection_dir": paths.memory_dir.join("projections").display().to_string(),
        "journal_entries": journal_entries,
        "policy": {
            "promote_threshold": config.memory.policy.promote_threshold,
            "demote_threshold": config.memory.policy.demote_threshold,
            "forget_threshold": config.memory.policy.forget_threshold,
            "max_records_per_scope": config.memory.policy.max_records_per_scope,
        },
    }))
}

pub(in crate::debug) fn rag_debug_summary(config: &IkarosConfig, paths: &IkarosPaths) -> Value {
    json!({
        "backend": safe_debug_string(&config.rag.backend),
        "rag_dir": paths.rag_dir.display().to_string(),
        "embedding_provider": safe_debug_string(&config.rag.embedding_provider),
        "embedding_model": safe_debug_string(&config.rag.embedding_model),
    })
}

pub(in crate::debug) fn safe_debug_string(input: &str) -> String {
    redact_secrets(input)
        .chars()
        .map(|ch| if ch.is_control() { '_' } else { ch })
        .collect()
}
