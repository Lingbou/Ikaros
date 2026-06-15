// SPDX-License-Identifier: GPL-3.0-only

use crate::Task;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeContext {
    pub task: Option<Task>,
    pub persona_context: String,
    pub relationship_context: Vec<String>,
    pub reference_context: Vec<String>,
    pub chat_history_context: Vec<String>,
    pub memory_context: Vec<String>,
    pub rag_context: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ContextBuilder {
    task: Option<Task>,
    persona_context: Option<String>,
    relationship_context: Vec<String>,
    reference_context: Vec<String>,
    chat_history_context: Vec<String>,
    memory_context: Vec<String>,
    rag_context: Vec<String>,
}

impl ContextBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn task(mut self, task: Task) -> Self {
        self.task = Some(task);
        self
    }

    pub fn persona_context(mut self, context: impl Into<String>) -> Self {
        self.persona_context = Some(context.into());
        self
    }

    pub fn relationship_context(mut self, context: Vec<String>) -> Self {
        self.relationship_context = context;
        self
    }

    pub fn reference_context(mut self, context: Vec<String>) -> Self {
        self.reference_context = context;
        self
    }

    pub fn chat_history_context(mut self, context: Vec<String>) -> Self {
        self.chat_history_context = context;
        self
    }

    pub fn memory_context(mut self, context: Vec<String>) -> Self {
        self.memory_context = context;
        self
    }

    pub fn rag_context(mut self, context: Vec<String>) -> Self {
        self.rag_context = context;
        self
    }

    pub fn build(self) -> RuntimeContext {
        RuntimeContext {
            task: self.task,
            persona_context: self.persona_context.unwrap_or_default(),
            relationship_context: self.relationship_context,
            reference_context: self.reference_context,
            chat_history_context: self.chat_history_context,
            memory_context: self.memory_context,
            rag_context: self.rag_context,
        }
    }
}
