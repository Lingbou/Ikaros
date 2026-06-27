// SPDX-License-Identifier: GPL-3.0-only

use crate::debug::DebugLogSource;
use anyhow::{Context, Result, anyhow};

use super::terminal_inline;
use crate::chat::workbench::TimelineRequest;

pub(super) fn parse_timeline_request(args: Vec<&str>) -> Result<TimelineRequest> {
    let mut request = TimelineRequest::default();
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--page" => {
                let page = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /timeline [turn] [--page N] [--kind KIND]"))?;
                request.page = parse_timeline_page(page)?;
                index += 2;
            }
            "--kind" | "--category" => {
                let kind = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /timeline [turn] [--page N] [--kind KIND]"))?;
                request.kind_filter = Some(parse_timeline_kind(kind)?.to_owned());
                index += 2;
            }
            "--failed" | "--failure" => {
                request.point_filter = Some("failed".into());
                index += 1;
            }
            "--approval" | "--approvals" => {
                request.point_filter = Some("approval".into());
                index += 1;
            }
            "--help" | "help" => {
                return Err(anyhow!(
                    "usage: /timeline [turn] [--page N] [--kind session|model|tool|context|memory|coding|audit|continuation|approval|error] [--failed|--approval]"
                ));
            }
            value if value.starts_with("--") => {
                return Err(anyhow!(
                    "unknown /timeline argument '{}'; expected --page, --kind, --failed, or --approval",
                    terminal_inline(value)
                ));
            }
            turn_id => {
                if request.turn_filter.is_some() {
                    return Err(anyhow!(
                        "usage: /timeline accepts at most one turn id plus --page/--kind/--failed/--approval"
                    ));
                }
                request.turn_filter = Some((*turn_id).to_owned());
                index += 1;
            }
        }
    }
    Ok(request)
}

pub(super) fn parse_trace_request(args: Vec<&str>) -> Result<TimelineRequest> {
    let mut request = TimelineRequest::default();
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--kind" | "--category" => {
                let kind = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /trace [turn] [--kind KIND] [--failed|--approval]")
                })?;
                request.kind_filter = Some(parse_timeline_kind(kind)?.to_owned());
                index += 2;
            }
            "--failed" | "--failure" => {
                request.point_filter = Some("failed".into());
                index += 1;
            }
            "--approval" | "--approvals" => {
                request.point_filter = Some("approval".into());
                index += 1;
            }
            "--help" | "help" => {
                return Err(anyhow!(
                    "usage: /trace [turn] [--kind session|model|tool|context|memory|coding|audit|continuation|approval|error] [--failed|--approval]"
                ));
            }
            value if value.starts_with("--") => {
                return Err(anyhow!(
                    "unknown /trace argument '{}'; expected --kind, --failed, or --approval",
                    terminal_inline(value)
                ));
            }
            turn_id => {
                if request.turn_filter.is_some() {
                    return Err(anyhow!(
                        "usage: /trace accepts at most one turn id plus --kind/--failed/--approval"
                    ));
                }
                request.turn_filter = Some((*turn_id).to_owned());
                index += 1;
            }
        }
    }
    Ok(request)
}

pub(super) fn parse_debug_logs_args(args: &[&str]) -> Result<(DebugLogSource, usize, usize)> {
    let mut source = DebugLogSource::All;
    let mut page = 1usize;
    let mut page_size = 25usize;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--source" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!(
                        "usage: /debug logs [--source all|audit|model-usage|trace] [--page N] [--page-size N]"
                    )
                })?;
                source = parse_debug_log_source(value)?;
                index += 2;
            }
            "--page" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!(
                        "usage: /debug logs [--source all|audit|model-usage|trace] [--page N] [--page-size N]"
                    )
                })?;
                page = parse_timeline_page(value)?;
                index += 2;
            }
            "--page-size" | "--limit" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!(
                        "usage: /debug logs [--source all|audit|model-usage|trace] [--page N] [--page-size N]"
                    )
                })?;
                page_size = value
                    .parse::<usize>()
                    .with_context(|| "debug logs page size must be a positive number")?
                    .max(1);
                index += 2;
            }
            "--help" | "help" => {
                return Err(anyhow!(
                    "usage: /debug logs [--source all|audit|model-usage|trace] [--page N] [--page-size N]"
                ));
            }
            value if value.starts_with("--") => {
                return Err(anyhow!(
                    "unknown /debug logs argument '{}'; expected --source, --page, or --page-size",
                    terminal_inline(value)
                ));
            }
            value => {
                source = parse_debug_log_source(value)?;
                index += 1;
            }
        }
    }
    Ok((source, page, page_size))
}

pub(super) fn parse_debug_dump_recent_logs(args: &[&str]) -> Result<usize> {
    let mut recent_logs = 25usize;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--recent-logs" | "--limit" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /debug dump [--recent-logs N]"))?;
                recent_logs = value
                    .parse::<usize>()
                    .with_context(|| "debug dump recent log count must be a positive number")?
                    .max(1);
                index += 2;
            }
            "--output" => {
                return Err(anyhow!(
                    "/debug dump --output is only available from the top-level CLI; workbench dump is read-only"
                ));
            }
            "--help" | "help" => {
                return Err(anyhow!("usage: /debug dump [--recent-logs N]"));
            }
            value if value.starts_with("--") => {
                return Err(anyhow!(
                    "unknown /debug dump argument '{}'; expected --recent-logs",
                    terminal_inline(value)
                ));
            }
            value => {
                return Err(anyhow!(
                    "unknown /debug dump argument '{}'; expected --recent-logs",
                    terminal_inline(value)
                ));
            }
        }
    }
    Ok(recent_logs)
}

fn parse_debug_log_source(value: &str) -> Result<DebugLogSource> {
    match value {
        "all" => Ok(DebugLogSource::All),
        "audit" => Ok(DebugLogSource::Audit),
        "model-usage" | "model_usage" | "usage" => Ok(DebugLogSource::ModelUsage),
        "trace" => Ok(DebugLogSource::Trace),
        unknown => Err(anyhow!(
            "unknown debug log source '{}'; expected all, audit, model-usage, or trace",
            terminal_inline(unknown)
        )),
    }
}

fn parse_timeline_page(page: &str) -> Result<usize> {
    let page = page
        .parse::<usize>()
        .with_context(|| "timeline page must be a positive number")?;
    Ok(page.max(1))
}

fn parse_timeline_kind(kind: &str) -> Result<&'static str> {
    match kind {
        "session" => Ok("session"),
        "model" => Ok("model"),
        "tool" => Ok("tool"),
        "context" => Ok("context"),
        "memory" => Ok("memory"),
        "coding" => Ok("coding"),
        "audit" => Ok("audit"),
        "continuation" => Ok("continuation"),
        "approval" => Ok("approval"),
        "error" => Ok("error"),
        unknown => Err(anyhow!(
            "unknown timeline kind '{}'; expected session, model, tool, context, memory, coding, audit, continuation, approval, or error",
            terminal_inline(unknown)
        )),
    }
}
