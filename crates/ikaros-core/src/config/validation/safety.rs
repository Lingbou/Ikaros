// SPDX-License-Identifier: GPL-3.0-only

use super::{super::ExecutionConfig, ConfigValidationReport, normalize, validate_timeout};

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
