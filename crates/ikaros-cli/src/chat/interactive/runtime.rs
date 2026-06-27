// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{ModelConfig, RemoteProviderConfig, ResolvedAgentProfile};
use ikaros_execution::harness::ExecutionSession;
use ikaros_providers::model::{ModelContentBlock, ModelProvider, ModelRequestOptions};
use std::{collections::VecDeque, io::IsTerminal, path::PathBuf};

use crate::chat::notice::WorkbenchNotice;
use crate::chat::progress::WorkbenchProgressSnapshot;
use crate::chat::workbench::WorkbenchScreenState;

pub(in crate::chat) struct InteractiveChatRuntime {
    pub(in crate::chat) agent: ResolvedAgentProfile,
    pub(in crate::chat) agent_id: String,
    pub(in crate::chat) state_dir: PathBuf,
    pub(in crate::chat) workspace: PathBuf,
    pub(in crate::chat) model_config: ModelConfig,
    pub(in crate::chat) model_provider: RemoteProviderConfig,
    pub(in crate::chat) provider: Box<dyn ModelProvider>,
    pub(in crate::chat) session: ExecutionSession,
    pub(in crate::chat) chat_session_id: String,
    pub(in crate::chat) request_options: ModelRequestOptions,
    pub(in crate::chat) pending_inputs: VecDeque<String>,
    pub(in crate::chat) pending_content_blocks: Vec<ModelContentBlock>,
    pub(in crate::chat) screen_state: WorkbenchScreenState,
    pub(in crate::chat) default_inline_ui: bool,
    pub(in crate::chat) last_progress: Option<WorkbenchProgressSnapshot>,
    pub(in crate::chat) notices: VecDeque<WorkbenchNotice>,
    pub(in crate::chat) pending_input_drain_requested: bool,
}

impl InteractiveChatRuntime {
    pub(in crate::chat) fn fullscreen_stdout_quiet(&self) -> bool {
        false
    }

    pub(in crate::chat) fn default_inline_stdout(&self) -> bool {
        should_emit_default_inline_stdout(self.default_inline_ui, std::io::stdout().is_terminal())
    }

    pub(in crate::chat) fn machine_stdout_quiet(&self) -> bool {
        self.fullscreen_stdout_quiet() || self.default_inline_stdout()
    }

    pub(in crate::chat) fn push_notice(&mut self, notice: WorkbenchNotice) {
        const MAX_NOTICES: usize = 24;
        self.notices.push_back(notice);
        while self.notices.len() > MAX_NOTICES {
            self.notices.pop_front();
        }
    }

    pub(in crate::chat) fn request_pending_input_drain(&mut self) {
        self.pending_input_drain_requested = true;
    }

    pub(in crate::chat) fn take_pending_input_drain_request(&mut self) -> bool {
        let requested = self.pending_input_drain_requested;
        self.pending_input_drain_requested = false;
        requested
    }
}

pub(in crate::chat) fn should_emit_default_inline_stdout(
    default_inline_ui: bool,
    _stdout_is_terminal: bool,
) -> bool {
    default_inline_ui
}
