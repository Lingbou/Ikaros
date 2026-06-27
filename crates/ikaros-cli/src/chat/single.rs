// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_agent::chat::{
    ChatMessageContext, ChatMessageResult, ChatRunOptions, chat_memory_policy_from_config,
    new_chat_session_id, run_chat_message_with_context,
};
use ikaros_agent::soul::load_or_default;
use ikaros_core::IkarosPaths;
use ikaros_host::host_agent_context;
use ikaros_providers::model::ModelUsageLedger;
use ikaros_state::memory::{JsonlMemoryJournal, LocalMemoryStore};
use ikaros_state::session::{SessionSource, SessionStore, SqliteSessionStore};
use std::{path::Path, sync::Arc};

use super::runtime::initial_interactive_runtime;

pub(crate) async fn run_single_chat_message(
    message: &str,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    mut options: ChatRunOptions,
) -> Result<ChatMessageResult> {
    paths.ensure()?;
    let host = host_agent_context(paths, workspace, agent_override)?;
    let config = &host.config;
    let persona = load_or_default(&paths.persona_dir)?;
    let chat_session_id = options
        .session_id
        .clone()
        .unwrap_or_else(new_chat_session_id);
    options.session_id = Some(chat_session_id.clone());
    let (runtime, registry) =
        initial_interactive_runtime(paths, workspace, config, agent_override, chat_session_id)?;
    let session_store: Arc<dyn SessionStore> =
        Arc::new(SqliteSessionStore::new(&runtime.state_dir));
    let memory_provider = LocalMemoryStore::new(&paths.memory_dir, &config.memory.backend)?;
    let memory_journal = JsonlMemoryJournal::new(&paths.memory_dir);
    let memory_policy = chat_memory_policy_from_config(&config.memory.policy);
    let usage_ledger = ModelUsageLedger::new(&paths.audit_dir);
    let session_state_db = runtime.state_dir.join("state.db");
    let session_source = options.session_source.clone().unwrap_or(SessionSource::Cli);
    run_chat_message_with_context(
        message,
        options,
        ChatMessageContext {
            persona: &persona,
            provider: runtime.provider.as_ref(),
            agent: &runtime.agent,
            session: &runtime.session,
            registry: &registry,
            session_store,
            session_source,
            agent_id: &runtime.agent_id,
            workspace: &runtime.workspace,
            memory_provider: &memory_provider,
            memory_journal: &memory_journal,
            memory_policy: &memory_policy,
            request_options: &runtime.request_options,
            model_usage_path: usage_ledger.path().to_path_buf(),
            session_state_db,
        },
    )
    .await
    .map_err(Into::into)
}
