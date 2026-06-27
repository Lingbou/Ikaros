// SPDX-License-Identifier: GPL-3.0-only

use super::{
    super::{ExecutionConfig, SelfModifyConfig},
    ConfigValidationReport, normalize, validate_timeout,
};

pub(super) fn validate_self_modify_config(
    config: &SelfModifyConfig,
    report: &mut ConfigValidationReport,
) {
    for (name, profile) in &config.check_profiles {
        let path = format!("self_modify.check_profiles.{name}");
        if name.trim().is_empty() {
            report.error(&path, "check profile name must not be empty");
        }
        if profile.commands.is_empty() {
            report.error(
                format!("{path}.commands"),
                "must contain at least one command",
            );
        }
        for (index, command) in profile.commands.iter().enumerate() {
            if command.trim().is_empty() {
                report.error(format!("{path}.commands[{index}]"), "must not be empty");
            }
        }
    }
}

pub(super) fn validate_execution_config(
    config: &ExecutionConfig,
    report: &mut ConfigValidationReport,
) {
    validate_timeout(
        "execution.network.timeout_ms",
        config.network.timeout_ms,
        report,
    );
    for (index, host) in config.network.allowed_hosts.iter().enumerate() {
        if host.trim().is_empty() {
            report.error(
                format!("execution.network.allowed_hosts[{index}]"),
                "must not be empty",
            );
        }
        if host.contains('/') || host.contains(':') {
            report.error(
                format!("execution.network.allowed_hosts[{index}]"),
                "must be an exact host name, not a URL or host:port",
            );
        }
    }
    if normalize(&config.sandbox.backend) == "docker" && config.sandbox.image.trim().is_empty() {
        report.error(
            "execution.sandbox.image",
            "must not be empty when execution.sandbox.backend is docker",
        );
    }
    if normalize(&config.sandbox.read_scope) != "workspace" {
        report.error("execution.sandbox.read_scope", "must be workspace");
    }
}
