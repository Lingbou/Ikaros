// SPDX-License-Identifier: GPL-3.0-only

mod webhook;

use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use ikaros_core::{IkarosPaths, contains_secret_like, redact_json, redact_secrets};
use ikaros_gateway::{
    GatewayDelivery, GatewayDeliveryStatus, GatewayInboundEnvelope, GatewayMessage,
    GatewayMessageKind, GatewayMessageStatus, GatewayOutboundEnvelope, GatewayPlatform,
    GatewayRoute, GatewaySessionSource, LocalGatewayStore, builtin_gateway_adapters,
};
use ikaros_runtime::{drain_gateway_messages, gateway_session_id, run_gateway_worker_tick};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
    time::Instant,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::time::sleep;
use webhook::{MessageWebhook, serve_message_webhook};

mod adapter;
mod commands;
mod daemon;
mod delivery;
mod pairing;
mod status;
mod workbench;

use self::{adapter::*, commands::*, daemon::*, delivery::*, pairing::*, status::*};
pub(crate) use commands::{MessageCommand, message_command};
pub(crate) use daemon::{message_worker_lock_is_stale, message_worker_lock_is_stale_label};
pub(crate) use status::{
    print_gateway_worker_forensics, print_gateway_worker_state, print_gateway_worker_stop,
};
pub(crate) use workbench::{
    run_gateway_adapter_workbench_command, run_gateway_daemon_workbench_command,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_worker_forensics_records_failed_stop_with_redacted_reason() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = IkarosPaths::from_home(temp.path().join("home"));
        let mut forensics =
            MessageWorkerForensics::start(&paths, 3, true).expect("start forensics");

        forensics
            .finish(
                "failed",
                "provider failed token=abc123 api_key=plain-secret",
            )
            .expect("finish forensics");

        let events = fs::read_to_string(paths.gateway_dir.join(MESSAGE_WORKER_EVENTS_FILE))
            .expect("worker events");
        assert!(events.contains("\"event\":\"started\""));
        assert!(events.contains("\"event\":\"stopped\""));
        assert!(events.contains("\"status\":\"failed\""));
        assert!(events.contains("provider failed token=[REDACTED_SECRET]"));
        assert!(events.contains("api_key=[REDACTED_SECRET]"));
        assert!(!events.contains("abc123"));
        assert!(!events.contains("plain-secret"));
    }
}
