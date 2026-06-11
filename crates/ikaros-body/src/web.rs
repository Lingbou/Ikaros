// SPDX-License-Identifier: GPL-3.0-only

use crate::{BodyAdapter, BodyEvent, BodyFrame, BodyKind, BodyStatus};
use ikaros_core::redact_secrets;

#[derive(Debug, Clone, Default)]
pub struct WebDashboardAdapter;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardRenderOptions {
    pub refresh_seconds: Option<u64>,
    pub snapshot_path: Option<String>,
}

impl WebDashboardAdapter {
    pub fn render_frame_with_options(
        &self,
        frame: &BodyFrame,
        options: &DashboardRenderOptions,
    ) -> String {
        render_dashboard(frame, options)
    }
}

impl BodyAdapter for WebDashboardAdapter {
    fn kind(&self) -> BodyKind {
        BodyKind::Web
    }

    fn render_status(&self, status: &BodyStatus) -> String {
        render_status_section(status)
    }

    fn render_event(&self, event: &BodyEvent) -> String {
        render_event_row(event)
    }

    fn render_frame(&self, frame: &BodyFrame) -> String {
        self.render_frame_with_options(frame, &DashboardRenderOptions::default())
    }
}

fn render_dashboard(frame: &BodyFrame, options: &DashboardRenderOptions) -> String {
    let adapter = WebDashboardAdapter;
    let status = adapter.render_status(&frame.status);
    let events = if frame.events.is_empty() {
        "<tr><td colspan=\"4\">No recent events</td></tr>".into()
    } else {
        frame
            .events
            .iter()
            .map(|event| adapter.render_event(event))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let refresh = options
        .refresh_seconds
        .map(|seconds| {
            format!(
                "\n<meta http-equiv=\"refresh\" content=\"{}\">",
                seconds.max(1)
            )
        })
        .unwrap_or_default();
    let snapshot = options
        .snapshot_path
        .as_deref()
        .map(|path| {
            format!(
                "<a class=\"link-pill\" href=\"{}\">BodyFrame JSON</a>",
                html_escape(path)
            )
        })
        .unwrap_or_default();
    let refresh_label = options
        .refresh_seconds
        .map(|seconds| format!("refresh={}s", seconds.max(1)))
        .unwrap_or_else(|| "refresh=manual".into());
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{}\n<title>Ikaros Dashboard</title>\n<style>{}</style>\n</head>\n<body>\n<main>\n<header class=\"topbar\"><div><h1>Ikaros Runtime</h1><p>Local body dashboard</p></div><div class=\"top-actions\"><span class=\"badge\">body=web</span><span class=\"badge muted\">{}</span>{}</div></header>\n{}\n<section class=\"runtime-map\" aria-label=\"runtime layers\"><span>Soul</span><span>Core</span><span>Harness</span><span>Memory</span><span>RAG</span><span>Body</span></section>\n<section class=\"panel\"><h2>Recent Events</h2><div class=\"table-wrap\"><table><thead><tr><th>Kind</th><th>Body</th><th>Message</th><th>Data</th></tr></thead><tbody>{}</tbody></table></div></section>\n</main>\n</body>\n</html>\n",
        refresh,
        dashboard_css(),
        html_escape(&refresh_label),
        snapshot,
        status,
        events
    )
}

fn render_status_section(status: &BodyStatus) -> String {
    let task = status.task_id.as_deref().unwrap_or("none");
    let task_state = status
        .task_state
        .as_ref()
        .map(|state| format!("{state:?}"))
        .unwrap_or_else(|| "none".into());
    let policies = if status.policy_decisions.is_empty() {
        "none".into()
    } else {
        status
            .policy_decisions
            .iter()
            .map(|decision| format!("{decision:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let audit = status
        .audit_path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "none".into());
    let approvals = status
        .approvals_path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "none".into());
    format!(
        "<section class=\"status-grid\"><article class=\"status-main\"><p class=\"eyebrow\">Persona</p><h2>{}</h2><dl><div><dt>Emotion</dt><dd>{}</dd></div><div><dt>Task</dt><dd>{}</dd></div><div><dt>State</dt><dd>{}</dd></div></dl></article><article><p class=\"eyebrow\">Context</p><div class=\"metric-row\"><span>Memory</span><strong>{}</strong></div><div class=\"metric-row\"><span>RAG</span><strong>{}</strong></div></article><article><p class=\"eyebrow\">Policy</p><p class=\"policy-text\">{}</p></article><article><p class=\"eyebrow\">Local Paths</p><p class=\"path-text\">audit: {}</p><p class=\"path-text\">approvals: {}</p></article></section>",
        html_escape(&status.persona_name),
        html_escape(&status.emotion),
        html_escape(task),
        html_escape(&task_state),
        status.context_sources.memory.len(),
        status.context_sources.rag.len(),
        html_escape(&policies),
        html_escape(&audit),
        html_escape(&approvals),
    )
}

fn render_event_row(event: &BodyEvent) -> String {
    let data = if event.data.is_empty() {
        "none".into()
    } else {
        event
            .data
            .iter()
            .map(|(key, value)| format!("{}={}", html_escape(key), html_escape(value)))
            .collect::<Vec<_>>()
            .join("<br>")
    };
    format!(
        "<tr><td>{:?}</td><td>{:?}</td><td>{}</td><td>{}</td></tr>",
        event.kind,
        event.body,
        html_escape(&event.message),
        data,
    )
}

fn html_escape(input: &str) -> String {
    redact_secrets(input)
        .chars()
        .flat_map(|ch| match ch {
            '&' => "&amp;".chars().collect::<Vec<_>>(),
            '<' => "&lt;".chars().collect::<Vec<_>>(),
            '>' => "&gt;".chars().collect::<Vec<_>>(),
            '"' => "&quot;".chars().collect::<Vec<_>>(),
            '\'' => "&#39;".chars().collect::<Vec<_>>(),
            _ => vec![ch],
        })
        .collect()
}

fn dashboard_css() -> &'static str {
    r#":root{color-scheme:light;--ink:#1c2430;--muted:#586271;--line:#d7dde6;--bg:#f6f8fb;--panel:#ffffff;--blue:#2463eb;--green:#0f766e;--amber:#b45309;--red:#b91c1c}*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--ink);font:14px/1.45 system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}main{max-width:1180px;margin:0 auto;padding:28px 20px 40px}.topbar{display:flex;align-items:flex-start;justify-content:space-between;gap:16px;margin-bottom:18px}.topbar h1{margin:0;font-size:30px;letter-spacing:0}.topbar p{margin:4px 0 0;color:var(--muted)}.top-actions{display:flex;align-items:center;justify-content:flex-end;gap:8px;flex-wrap:wrap}.badge,.link-pill{border:1px solid var(--line);background:var(--panel);padding:6px 10px;border-radius:6px;color:var(--blue);font-weight:700;text-decoration:none}.badge.muted{color:var(--muted)}.status-grid{display:grid;grid-template-columns:2fr 1fr 1fr 1.7fr;gap:12px;margin-bottom:14px}.status-grid article,.panel{background:var(--panel);border:1px solid var(--line);border-radius:8px;padding:14px;min-width:0}.status-main h2{font-size:24px;margin:0 0 10px}.eyebrow{margin:0 0 8px;color:var(--muted);font-size:12px;text-transform:uppercase;font-weight:700;letter-spacing:.04em}dl{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:8px;margin:0}dt{color:var(--muted);font-size:12px}dd{margin:2px 0 0;font-weight:700;overflow-wrap:anywhere}.metric-row{display:flex;align-items:center;justify-content:space-between;border-top:1px solid var(--line);padding:9px 0}.metric-row:first-of-type{border-top:0}.metric-row strong{font-size:26px;color:var(--green)}.policy-text,.path-text{margin:0 0 8px;overflow-wrap:anywhere}.path-text{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12px;color:var(--muted)}.runtime-map{display:grid;grid-template-columns:repeat(6,minmax(0,1fr));gap:8px;margin:14px 0}.runtime-map span{display:block;min-height:42px;border-radius:6px;color:#fff;font-weight:800;text-align:center;padding:12px 6px;background:var(--blue)}.runtime-map span:nth-child(2){background:var(--green)}.runtime-map span:nth-child(3){background:var(--amber)}.runtime-map span:nth-child(4){background:#475569}.runtime-map span:nth-child(5){background:#7c3aed}.runtime-map span:nth-child(6){background:var(--red)}.panel h2{margin:0 0 12px;font-size:18px}.table-wrap{overflow:auto;border:1px solid var(--line);border-radius:6px}table{width:100%;border-collapse:collapse;min-width:720px}th,td{text-align:left;vertical-align:top;padding:10px;border-bottom:1px solid var(--line)}th{background:#eef2f7;color:#303947;font-size:12px;text-transform:uppercase;letter-spacing:.04em}td{overflow-wrap:anywhere}tr:last-child td{border-bottom:0}@media(max-width:820px){main{padding:18px 12px}.topbar{display:block}.top-actions{justify-content:flex-start;margin-top:10px}.badge,.link-pill{display:inline-block}.status-grid{grid-template-columns:1fr}dl{grid-template-columns:1fr}.runtime-map{grid-template-columns:repeat(2,minmax(0,1fr))}}"#
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BodyEventKind, BodyStatus};
    use ikaros_core::PolicyDecision;
    use std::collections::BTreeMap;

    #[test]
    fn web_dashboard_renders_frame_and_escapes_content() {
        let status = BodyStatus::new("Ikaros <core>", "Focused")
            .with_context_sources(vec!["memory token=abc123".into()], vec!["rag hit".into()])
            .with_policy_decisions(vec![PolicyDecision::Allow])
            .with_audit_path("/tmp/audit.jsonl")
            .with_approvals_path("/tmp/approvals.jsonl");
        let event = BodyEvent::new(
            BodyKind::Web,
            BodyEventKind::Audit,
            "audit <script> token=abc123",
            BTreeMap::from([("detail".into(), "api_key=abc123".into())]),
        );
        let frame = BodyFrame {
            body: BodyKind::Web,
            status,
            events: vec![event],
        };
        let html = WebDashboardAdapter.render_frame(&frame);
        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("Ikaros Runtime"));
        assert!(html.contains("Ikaros &lt;core&gt;"));
        assert!(html.contains("body=web"));
        assert!(!html.contains("<script>"));
        assert!(!html.contains("abc123"));
        assert!(html.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn web_dashboard_supports_refresh_and_snapshot_link() {
        let frame = BodyFrame {
            body: BodyKind::Web,
            status: BodyStatus::new("Ikaros", "Neutral"),
            events: Vec::new(),
        };
        let html = WebDashboardAdapter.render_frame_with_options(
            &frame,
            &DashboardRenderOptions {
                refresh_seconds: Some(5),
                snapshot_path: Some("frame.json?x=<bad>".into()),
            },
        );
        assert!(html.contains("http-equiv=\"refresh\" content=\"5\""));
        assert!(html.contains("refresh=5s"));
        assert!(html.contains("BodyFrame JSON"));
        assert!(html.contains("frame.json?x=&lt;bad&gt;"));
    }
}
