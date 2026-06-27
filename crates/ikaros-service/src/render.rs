// SPDX-License-Identifier: GPL-3.0-only

use crate::{ServiceKind, ServiceTemplateConfig};
use ikaros_core::redact_secrets;

pub(crate) fn render_systemd(config: &ServiceTemplateConfig) -> String {
    let args = config
        .command_args()
        .into_iter()
        .map(|arg| systemd_quote_arg(&arg))
        .collect::<Vec<_>>()
        .join(" ");
    let stop_block = render_systemd_stop_block(config);
    let description = match config.kind {
        ServiceKind::ScheduleWorker => "Ikaros local schedule worker",
        ServiceKind::MessageWorker => "Ikaros local message worker",
        ServiceKind::MessageWebhook => "Ikaros local message webhook",
    };
    format!(
        "[Unit]\nDescription={}\nAfter=network-online.target\n\n[Service]\nType=simple\nEnvironment={}\nWorkingDirectory={}\nExecStart={}\n{}Restart=on-failure\nRestartSec=5s\nNoNewPrivileges=true\nPrivateTmp=true\n\n[Install]\nWantedBy=default.target\n",
        description,
        systemd_quote_env("IKAROS_HOME", &config.ikaros_home.display().to_string()),
        systemd_quote_arg(&config.workspace.display().to_string()),
        args,
        stop_block,
    )
}

fn render_systemd_stop_block(config: &ServiceTemplateConfig) -> String {
    if config.kind != ServiceKind::MessageWorker {
        return String::new();
    }
    let mut args = config.base_command_args();
    args.extend([
        "message".into(),
        "worker-stop".into(),
        "--reason".into(),
        "service manager stop".into(),
    ]);
    let command = args
        .into_iter()
        .map(|arg| systemd_quote_arg(&arg))
        .collect::<Vec<_>>()
        .join(" ");
    format!("ExecStop={command}\nTimeoutStopSec=30s\n")
}

pub(crate) fn render_launchd(config: &ServiceTemplateConfig) -> String {
    let args = config
        .command_args()
        .into_iter()
        .map(|arg| format!("    <string>{}</string>", xml_escape(&arg)))
        .collect::<Vec<_>>()
        .join("\n");
    let stdout_path = config
        .ikaros_home
        .join("logs")
        .join(format!("{}.out.log", config.label));
    let stderr_path = config
        .ikaros_home
        .join("logs")
        .join(format!("{}.err.log", config.label));
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>{}</string>\n  <key>ProgramArguments</key>\n  <array>\n{}\n  </array>\n  <key>WorkingDirectory</key>\n  <string>{}</string>\n  <key>EnvironmentVariables</key>\n  <dict>\n    <key>IKAROS_HOME</key>\n    <string>{}</string>\n  </dict>\n  <key>RunAtLoad</key>\n  <true/>\n  <key>KeepAlive</key>\n  <true/>\n  <key>StandardOutPath</key>\n  <string>{}</string>\n  <key>StandardErrorPath</key>\n  <string>{}</string>\n</dict>\n</plist>\n",
        xml_escape(&config.label),
        args,
        xml_escape(&config.workspace.display().to_string()),
        xml_escape(&config.ikaros_home.display().to_string()),
        xml_escape(&stdout_path.display().to_string()),
        xml_escape(&stderr_path.display().to_string()),
    )
}

fn systemd_quote_env(name: &str, value: &str) -> String {
    format!("{name}={}", systemd_quote_arg(value))
}

fn systemd_quote_arg(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':' | '@'))
    {
        return redact_secrets(value);
    }
    let escaped = redact_secrets(value)
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn xml_escape(value: &str) -> String {
    redact_secrets(value)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ServiceManager, ServiceTemplateConfig};

    fn base_config(manager: ServiceManager, kind: ServiceKind) -> ServiceTemplateConfig {
        ServiceTemplateConfig {
            manager,
            kind,
            label: ServiceTemplateConfig::default_label(kind).into(),
            binary_path: "/usr/local/bin/ikaros".into(),
            ikaros_home: "/home/user/.ikaros".into(),
            workspace: "/home/user/work space/project".into(),
            agent: Some("plan".into()),
            host: "127.0.0.1".into(),
            port: 8002,
            interval_seconds: 60,
            limit: 5,
        }
    }

    #[test]
    fn renders_systemd_schedule_worker() {
        let template = base_config(ServiceManager::Systemd, ServiceKind::ScheduleWorker).render();
        assert!(template.contains("Description=Ikaros local schedule worker"));
        assert!(template.contains("schedule worker"));
        assert!(template.contains("--interval-seconds 60"));
        assert!(template.contains("--limit 5"));
        assert!(template.contains("WorkingDirectory=\"/home/user/work space/project\""));
        assert!(
            template.contains("\"/home/user/work space/project\" --agent plan schedule worker")
        );
        assert!(!template.contains("--workspace"));
    }

    #[test]
    fn renders_launchd_message_webhook() {
        let template = base_config(ServiceManager::Launchd, ServiceKind::MessageWebhook).render();
        assert!(template.contains("<string>ikaros-message-webhook</string>"));
        assert!(template.contains("<string>message</string>"));
        assert!(template.contains("<string>webhook</string>"));
        assert!(template.contains("<string>/home/user/work space/project</string>"));
        assert!(template.contains("<string>127.0.0.1</string>"));
        assert!(template.contains("<string>8002</string>"));
        assert!(!template.contains("<string>--workspace</string>"));
    }

    #[test]
    fn renders_systemd_message_worker() {
        let template = base_config(ServiceManager::Systemd, ServiceKind::MessageWorker).render();
        assert!(template.contains("Description=Ikaros local message worker"));
        assert!(template.contains("message worker"));
        assert!(template.contains("--interval-seconds 60"));
        assert!(template.contains("--limit 5"));
        assert!(template.contains("ExecStop="));
        assert!(template.contains("message worker-stop"));
        assert!(template.contains("--reason \"service manager stop\""));
        assert!(template.contains("TimeoutStopSec=30s"));
    }

    #[test]
    fn systemd_schedule_worker_has_no_message_stop_hook() {
        let template = base_config(ServiceManager::Systemd, ServiceKind::ScheduleWorker).render();
        assert!(!template.contains("message worker-stop"));
        assert!(!template.contains("ExecStop="));
    }

    #[test]
    fn redacts_secret_like_values_in_templates() {
        let mut config = base_config(ServiceManager::Systemd, ServiceKind::MessageWebhook);
        config.agent = Some("token=abc123".into());
        config.host = "api_key=abc123".into();
        let template = config.render();
        assert!(!template.contains("abc123"));
        assert!(template.contains("[REDACTED_SECRET]"));
    }
}
