// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::ChatMessageContext;
use crate::task_loop::TaskExecutionContext;
use ikaros_execution::harness::TaskExecutionReport;
use ikaros_protocol::{GatewayDelivery, GatewayMessage, GatewayMessageStatus};
use ikaros_state::session::RuntimeSessionTarget;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayDrainContext {
    pub memory_hits: usize,
    pub rag_hits: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GatewayDrainReport {
    pub message_id: String,
    pub kind: String,
    pub status: GatewayMessageStatus,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<GatewayDrainContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streamed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_chunks: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_usage: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<GatewayMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<GatewayDelivery>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_report: Option<TaskExecutionReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GatewayWorkerTickReport {
    pub kind: String,
    pub pending: usize,
    pub drained: usize,
    pub reports: Vec<GatewayDrainReport>,
}

pub enum GatewayMessageDrainContext<'a> {
    Chat(GatewayChatDrainContext<'a>),
    Task(GatewayTaskDrainContext<'a>),
}

pub struct GatewayChatDrainContext<'a> {
    pub session_target: &'a RuntimeSessionTarget,
    pub chat: ChatMessageContext<'a>,
}

pub struct GatewayTaskDrainContext<'a> {
    pub session_target: &'a RuntimeSessionTarget,
    pub task: TaskExecutionContext<'a>,
}
