// SPDX-License-Identifier: GPL-3.0-only

mod context;
mod jsonl;
mod search;
mod sessions;
mod sqlite;
mod types;

pub use context::{
    build_chat_history_record_with_turn_id, chat_history_context_lines,
    chat_history_context_lines_with_summary, new_chat_session_id,
};
use ikaros_core::{Result, redact_secrets};
pub use types::{
    ChatHistoryAppend, ChatHistoryBackend, ChatHistoryRecord, ChatHistorySessionSummary,
    ChatHistoryStore,
};

use self::{search::search_chat_history_records, sessions::chat_history_session_summaries};
use std::path::{Path, PathBuf};

impl ChatHistoryStore {
    pub fn new(home: impl AsRef<Path>) -> Self {
        Self {
            path: home.as_ref().join("chat/history.jsonl"),
            backend: ChatHistoryBackend::Jsonl,
        }
    }

    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            backend: ChatHistoryBackend::Jsonl,
        }
    }

    pub fn new_with_backend(home: impl AsRef<Path>, backend: &str) -> Result<Self> {
        let backend = ChatHistoryBackend::parse(backend)?;
        let file_name = match backend {
            ChatHistoryBackend::Jsonl => "history.jsonl",
            ChatHistoryBackend::Sqlite => "history.sqlite",
        };
        Ok(Self {
            path: home.as_ref().join("chat").join(file_name),
            backend,
        })
    }

    pub fn from_path_with_backend(path: impl Into<PathBuf>, backend: &str) -> Result<Self> {
        Ok(Self {
            path: path.into(),
            backend: ChatHistoryBackend::parse(backend)?,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn backend_name(&self) -> &'static str {
        self.backend.as_str()
    }

    pub fn append(&self, record: &ChatHistoryRecord) -> Result<()> {
        let record = redacted_record(record);
        match self.backend {
            ChatHistoryBackend::Jsonl => self.append_jsonl(&record),
            ChatHistoryBackend::Sqlite => self.append_sqlite(&record),
        }
    }

    pub fn read_all(&self) -> Result<Vec<ChatHistoryRecord>> {
        match self.backend {
            ChatHistoryBackend::Jsonl => self.read_all_jsonl(),
            ChatHistoryBackend::Sqlite => self.read_all_sqlite(),
        }
    }

    pub fn read_session(&self, session_id: &str) -> Result<Vec<ChatHistoryRecord>> {
        let records = self.read_all()?;
        Ok(records
            .into_iter()
            .filter(|record| record.session_id == session_id)
            .collect())
    }

    pub fn search(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<ChatHistoryRecord>> {
        let records = match self.backend {
            ChatHistoryBackend::Jsonl => self.read_all_jsonl()?,
            ChatHistoryBackend::Sqlite => self.read_all_sqlite()?,
        };
        Ok(search_chat_history_records(
            records, query, limit, session_id,
        ))
    }

    pub fn session_summaries(&self, limit: usize) -> Result<Vec<ChatHistorySessionSummary>> {
        let records = self.read_all()?;
        Ok(chat_history_session_summaries(&records, limit))
    }

    pub fn delete_session(&self, session_id: &str) -> Result<usize> {
        match self.backend {
            ChatHistoryBackend::Jsonl => self.delete_session_jsonl(session_id),
            ChatHistoryBackend::Sqlite => self.delete_session_sqlite(session_id),
        }
    }

    pub fn clear(&self) -> Result<usize> {
        match self.backend {
            ChatHistoryBackend::Jsonl => self.clear_jsonl(),
            ChatHistoryBackend::Sqlite => self.clear_sqlite(),
        }
    }

    pub fn recent_context_lines(&self, limit: usize) -> Result<Vec<String>> {
        let records = self.read_all()?;
        Ok(chat_history_context_lines(&records, limit))
    }

    pub fn context_lines(&self, recent_limit: usize, summary_limit: usize) -> Result<Vec<String>> {
        let records = self.read_all()?;
        Ok(chat_history_context_lines_with_summary(
            &records,
            recent_limit,
            summary_limit,
        ))
    }

    pub fn context_lines_for_session(
        &self,
        session_id: &str,
        recent_limit: usize,
        summary_limit: usize,
    ) -> Result<Vec<String>> {
        let records = self.read_session(session_id)?;
        Ok(chat_history_context_lines_with_summary(
            &records,
            recent_limit,
            summary_limit,
        ))
    }
}

fn redacted_record(record: &ChatHistoryRecord) -> ChatHistoryRecord {
    ChatHistoryRecord {
        session_id: redact_secrets(&record.session_id),
        turn_id: redact_secrets(&record.turn_id),
        created_at: redact_secrets(&record.created_at),
        agent: redact_secrets(&record.agent),
        provider: redact_secrets(&record.provider),
        model: redact_secrets(&record.model),
        streamed: record.streamed,
        user_message: redact_secrets(&record.user_message),
        assistant_message: redact_secrets(&record.assistant_message),
        relationship_hits: record.relationship_hits,
        memory_hits: record.memory_hits,
        rag_hits: record.rag_hits,
    }
}
