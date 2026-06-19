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
    pub memory_projection_context: Vec<String>,
    pub working_memory_context: Vec<String>,
    pub retrieved_memory_context: Vec<String>,
    pub rag_context: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_continuation_prompt: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ContextBuilder {
    task: Option<Task>,
    persona_context: Option<String>,
    relationship_context: Vec<String>,
    reference_context: Vec<String>,
    chat_history_context: Vec<String>,
    memory_projection_context: Vec<String>,
    working_memory_context: Vec<String>,
    retrieved_memory_context: Vec<String>,
    rag_context: Vec<String>,
    context_continuation_prompt: Option<String>,
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

    pub fn memory_projection_context(mut self, context: Vec<String>) -> Self {
        self.memory_projection_context = context;
        self
    }

    pub fn working_memory_context(mut self, context: Vec<String>) -> Self {
        self.working_memory_context = context;
        self
    }

    pub fn retrieved_memory_context(mut self, context: Vec<String>) -> Self {
        self.retrieved_memory_context = context;
        self
    }

    pub fn rag_context(mut self, context: Vec<String>) -> Self {
        self.rag_context = context;
        self
    }

    pub fn context_continuation_prompt(mut self, prompt: Option<String>) -> Self {
        self.context_continuation_prompt = prompt;
        self
    }

    pub fn build(self) -> RuntimeContext {
        RuntimeContext {
            task: self.task,
            persona_context: self.persona_context.unwrap_or_default(),
            relationship_context: self.relationship_context,
            reference_context: self.reference_context,
            chat_history_context: self.chat_history_context,
            memory_projection_context: self.memory_projection_context,
            working_memory_context: self.working_memory_context,
            retrieved_memory_context: self.retrieved_memory_context,
            rag_context: self.rag_context,
            context_continuation_prompt: self.context_continuation_prompt,
        }
    }
}
