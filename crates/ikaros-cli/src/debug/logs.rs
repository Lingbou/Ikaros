// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::debug) struct DebugLogEntry {
    pub(in crate::debug) at: String,
    pub(in crate::debug) value: Value,
}

pub(in crate::debug) struct DebugLogCollection {
    pub(in crate::debug) audit_path: PathBuf,
    pub(in crate::debug) model_usage_path: PathBuf,
    pub(in crate::debug) trace_path: PathBuf,
    pub(in crate::debug) audit_count: usize,
    pub(in crate::debug) model_usage_count: usize,
    pub(in crate::debug) trace_count: usize,
    pub(in crate::debug) total_model_tokens: u64,
    pub(in crate::debug) cache_read_tokens: u64,
    pub(in crate::debug) cache_write_tokens: u64,
    pub(in crate::debug) entries: Vec<DebugLogEntry>,
}

pub(in crate::debug) fn collect_debug_logs(
    paths: &IkarosPaths,
    source: DebugLogSource,
) -> Result<DebugLogCollection> {
    let audit_log = AuditLog::new(&paths.audit_dir);
    let usage_ledger = ModelUsageLedger::new(&paths.audit_dir);
    let trace_path = paths.logs_dir.join("trace.jsonl");
    let audit_events = if source.includes_audit() {
        audit_log.read_all()?
    } else {
        Vec::new()
    };
    let usage_records = if source.includes_model_usage() {
        usage_ledger.read_all()?
    } else {
        Vec::new()
    };
    let trace_events = if source.includes_trace() {
        read_trace_log_entries(&trace_path)?
    } else {
        Vec::new()
    };
    let mut entries = Vec::new();
    for event in &audit_events {
        entries.push(DebugLogEntry {
            at: event.at.clone(),
            value: json!({
                "source": "audit",
                "at": event.at,
                "id": event.id,
                "kind": event.kind,
                "correlation_id": audit_event_correlation_id(event),
                "message": event.message,
                "decision": event.decision,
                "data": event.data,
            }),
        });
    }
    for record in &usage_records {
        let cache_read_tokens = record.cache_read_tokens.unwrap_or(0);
        let cache_write_tokens = record.cache_write_tokens.unwrap_or(0);
        entries.push(DebugLogEntry {
            at: record.at.clone(),
            value: json!({
                "source": "model_usage",
                "at": record.at,
                "id": record.id,
                "provider": record.provider,
                "model": record.model,
                "prompt_tokens": record.prompt_tokens,
                "completion_tokens": record.completion_tokens,
                "total_tokens": record.total_tokens,
                "cache_read_tokens": cache_read_tokens,
                "cache_write_tokens": cache_write_tokens,
                "estimated": record.estimated,
            }),
        });
    }
    for entry in &trace_events {
        entries.push(DebugLogEntry {
            at: trace_log_entry_timestamp(entry),
            value: trace_log_entry_json(entry),
        });
    }
    entries.sort_by(|left, right| left.at.cmp(&right.at));
    let total_model_tokens = usage_records
        .iter()
        .map(|record| record.total_tokens as u64)
        .sum();
    let cache_read_tokens = usage_records
        .iter()
        .map(|record| record.cache_read_tokens.unwrap_or(0) as u64)
        .sum();
    let cache_write_tokens = usage_records
        .iter()
        .map(|record| record.cache_write_tokens.unwrap_or(0) as u64)
        .sum();
    Ok(DebugLogCollection {
        audit_path: audit_log.path().to_path_buf(),
        model_usage_path: usage_ledger.path().to_path_buf(),
        trace_path,
        audit_count: audit_events.len(),
        model_usage_count: usage_records.len(),
        trace_count: trace_events.len(),
        total_model_tokens,
        cache_read_tokens,
        cache_write_tokens,
        entries,
    })
}

pub(in crate::debug) fn audit_event_correlation_id(
    event: &ikaros_harness::AuditEvent,
) -> Option<String> {
    event.correlation_id.clone().or_else(|| {
        event
            .data
            .get("correlation_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    })
}

pub(in crate::debug) fn read_trace_log_entries(path: &Path) -> Result<Vec<Value>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)?;
    let mut entries = Vec::new();
    for (index, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value = match serde_json::from_str::<Value>(trimmed) {
            Ok(value) => value,
            Err(error) => json!({
                "timestamp": "",
                "level": "ERROR",
                "target": "ikaros_cli::debug",
                "message": format!("invalid trace log line {}: {}", index + 1, error),
                "raw": redact_secrets(trimmed),
            }),
        };
        entries.push(redact_json(value));
    }
    Ok(entries)
}

pub(in crate::debug) fn trace_log_entry_timestamp(entry: &Value) -> String {
    entry
        .get("timestamp")
        .or_else(|| entry.get("at"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

pub(in crate::debug) fn trace_log_entry_json(entry: &Value) -> Value {
    let mut value = redact_json(entry.clone());
    if let Value::Object(map) = &mut value {
        map.insert("source".into(), Value::String("trace".into()));
    }
    value
}

pub(in crate::debug) fn debug_logs(args: DebugLogsArgs, paths: &IkarosPaths) -> Result<()> {
    let output = debug_logs_report(&args, paths)?;
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

pub(crate) fn debug_logs_json_line(
    paths: &IkarosPaths,
    source: DebugLogSource,
    page: usize,
    page_size: usize,
) -> Result<String> {
    let args = DebugLogsArgs {
        source,
        page,
        page_size,
    };
    let output = debug_logs_report(&args, paths)?;
    Ok(format!(
        "logs_json: {}",
        serde_json::to_string(&redact_json(output))?
    ))
}

pub(in crate::debug) fn debug_logs_report(
    args: &DebugLogsArgs,
    paths: &IkarosPaths,
) -> Result<Value> {
    let logs = collect_debug_logs(paths, args.source)?;
    let page = args.page.max(1);
    let page_size = args.page_size.max(1);
    let start = page_start(page, page_size).min(logs.entries.len());
    let end = start.saturating_add(page_size).min(logs.entries.len());
    let page_entries = logs.entries[start..end]
        .iter()
        .map(|entry| entry.value.clone())
        .collect::<Vec<_>>();
    Ok(json!({
        "format": "ikaros-logs-v1",
        "source": args.source.as_str(),
        "trace_schema": STRUCTURED_TRACE_SCHEMA,
        "audit_path": logs.audit_path.display().to_string(),
        "model_usage_path": logs.model_usage_path.display().to_string(),
        "trace_path": logs.trace_path.display().to_string(),
        "counts": {
            "audit": logs.audit_count,
            "model_usage": logs.model_usage_count,
            "trace": logs.trace_count,
            "total": logs.entries.len(),
        },
        "tokens": {
            "model_total": logs.total_model_tokens,
            "cache_read": logs.cache_read_tokens,
            "cache_write": logs.cache_write_tokens,
        },
        "pagination": {
            "page": page,
            "page_size": page_size,
            "entries": pagination_summary(logs.entries.len(), page, page_size),
            "has_next": end < logs.entries.len(),
            "has_previous": page > 1 && !logs.entries.is_empty(),
        },
        "entries": page_entries,
    }))
}
