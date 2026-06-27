// SPDX-License-Identifier: GPL-3.0-only

mod webhook;

use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use ikaros_core::{IkarosPaths, redact_secrets};
use ikaros_gateway::{
    GatewayDelivery, GatewayDeliveryStatus, GatewayDeliveryStatusCounts, GatewayMessage,
    GatewayMessageKind, GatewayMessageStatus, GatewayRoute, GatewaySessionSource,
    GatewayStatusSnapshot, LocalGatewayStore, MESSAGE_WORKER_LOCK_FILE, MESSAGE_WORKER_STOP_FILE,
    MessageWorkerForensics, acquire_message_worker_lock, builtin_gateway_adapters,
    clear_message_worker_stop_request, gateway_lease_is_stale, gateway_message_is_retryable,
    gateway_worker_events_path, gateway_worker_lock_path, gateway_worker_stop_path,
    latest_nonempty_line, message_daemon_log_path, message_daemon_status_label,
    redacted_json_field, redacted_message_worker_lock_owner, take_message_worker_stop_request,
    write_message_worker_stop_request,
};
use ikaros_runtime::{drain_gateway_messages, gateway_session_id, run_gateway_worker_tick};
use std::{
    fs::{self, OpenOptions},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::Duration,
    time::Instant,
};
use time::OffsetDateTime;
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
pub(crate) use ikaros_gateway::{message_worker_lock_is_stale, message_worker_lock_is_stale_label};
pub(crate) use status::{
    print_gateway_worker_forensics, print_gateway_worker_state, print_gateway_worker_stop,
};
pub(crate) use workbench::{
    run_gateway_adapter_workbench_command, run_gateway_daemon_workbench_command,
};
