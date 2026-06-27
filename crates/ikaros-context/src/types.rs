// SPDX-License-Identifier: GPL-3.0-only

use crate::{ContextCompressedSection, TokenEstimator, diff_chat_context};
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatContext {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relationship: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_projection: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub working_memory: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retrieved_memory: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rag: Vec<String>,
}

impl ChatContext {
    pub fn memory_hits(&self) -> usize {
        self.memory_projection.len() + self.working_memory.len() + self.retrieved_memory.len()
    }

    pub fn to_sections(&self, estimator: &dyn TokenEstimator) -> Vec<ContextSection> {
        let mut sections = Vec::new();
        push_section(
            &mut sections,
            ContextSectionKind::Relationship,
            "relationship",
            self.relationship.clone(),
            estimator,
        );
        push_section(
            &mut sections,
            ContextSectionKind::References,
            "references",
            self.references.clone(),
            estimator,
        );
        push_section(
            &mut sections,
            ContextSectionKind::History,
            "history",
            self.history.clone(),
            estimator,
        );
        push_section(
            &mut sections,
            ContextSectionKind::MemoryProjection,
            "memory_projection",
            self.memory_projection.clone(),
            estimator,
        );
        push_section(
            &mut sections,
            ContextSectionKind::WorkingMemory,
            "working_memory",
            self.working_memory.clone(),
            estimator,
        );
        push_section(
            &mut sections,
            ContextSectionKind::RetrievedMemory,
            "retrieved_memory",
            self.retrieved_memory.clone(),
            estimator,
        );
        push_section(
            &mut sections,
            ContextSectionKind::Rag,
            "rag",
            self.rag.clone(),
            estimator,
        );
        sections
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextBundle {
    pub context: ChatContext,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<ContextSection>,
    pub budget: ContextBudget,
    pub diff: ContextDiff,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compressed_sections: Vec<ContextCompressedSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<ContextReference>,
}

impl ContextBundle {
    pub fn from_context(
        before: ChatContext,
        after: ChatContext,
        budget: ContextBudget,
        references: Vec<ContextReference>,
        estimator: &dyn TokenEstimator,
    ) -> Self {
        let diff = diff_chat_context(&before, &after, estimator);
        let sections = after.to_sections(estimator);
        Self {
            context: after,
            sections,
            budget,
            diff,
            compressed_sections: Vec::new(),
            compression_summary: None,
            continuation_prompt: None,
            references,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextSection {
    pub kind: ContextSectionKind,
    pub label: String,
    pub trust_level: ContextTrustLevel,
    pub source_kind: ContextSourceKind,
    pub injection_reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<String>,
    pub estimated_tokens: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ContextTrustLevel {
    High,
    Medium,
    MediumLow,
    Low,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ContextSourceKind {
    Runtime,
    AcceptedMemory,
    ExplicitReference,
    SessionHistory,
    MemoryProjection,
    WorkingMemory,
    RetrievedMemory,
    RagIndex,
    ToolResult,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ContextSectionKind {
    System,
    Developer,
    Relationship,
    References,
    History,
    MemoryProjection,
    WorkingMemory,
    RetrievedMemory,
    Rag,
    ToolResults,
}

impl fmt::Display for ContextSectionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ContextSectionKind::System => "system",
            ContextSectionKind::Developer => "developer",
            ContextSectionKind::Relationship => "relationship",
            ContextSectionKind::References => "references",
            ContextSectionKind::History => "history",
            ContextSectionKind::MemoryProjection => "memory_projection",
            ContextSectionKind::WorkingMemory => "working_memory",
            ContextSectionKind::RetrievedMemory => "retrieved_memory",
            ContextSectionKind::Rag => "rag",
            ContextSectionKind::ToolResults => "tool_results",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextBudget {
    pub max_tokens: usize,
    pub used_tokens: usize,
    pub estimator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserved_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserved_system_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl ContextBudget {
    pub fn new(max_tokens: usize, estimator: impl Into<String>) -> Self {
        Self {
            max_tokens,
            used_tokens: 0,
            estimator: estimator.into(),
            requested_tokens: None,
            context_window: None,
            reserved_output_tokens: None,
            reserved_system_tokens: None,
            source: None,
        }
    }

    pub fn unbounded(estimator: impl Into<String>) -> Self {
        Self {
            max_tokens: 0,
            used_tokens: 0,
            estimator: estimator.into(),
            requested_tokens: None,
            context_window: None,
            reserved_output_tokens: None,
            reserved_system_tokens: None,
            source: None,
        }
    }

    pub fn with_model_window(
        mut self,
        requested_tokens: usize,
        context_window: u32,
        reserved_output_tokens: u32,
        reserved_system_tokens: u32,
        source: impl Into<String>,
    ) -> Self {
        self.requested_tokens = Some(requested_tokens);
        self.context_window = Some(context_window);
        self.reserved_output_tokens = Some(reserved_output_tokens);
        self.reserved_system_tokens = Some(reserved_system_tokens);
        self.source = Some(source.into());
        self
    }

    pub fn is_unbounded(&self) -> bool {
        self.max_tokens == 0
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextDiff {
    pub before_tokens: usize,
    pub after_tokens: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<ContextDiffItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<ContextDiffItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compressed: Vec<ContextDiffItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextDiffItem {
    pub section: ContextSectionKind,
    pub tokens: usize,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextLimitReport {
    pub max_tokens: usize,
    pub required_tokens: usize,
    pub protected_tokens: usize,
    pub estimator: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protected_sections: Vec<ContextSectionKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextReference {
    pub raw: String,
    pub kind: ContextReferenceKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ContextReferenceKind {
    File {
        path: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start_line: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end_line: Option<usize>,
    },
    Folder {
        path: PathBuf,
    },
    Git {
        rev: String,
    },
    Diff,
    Staged,
    Url {
        url: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedContextReference {
    pub reference: ContextReference,
    pub line: String,
}

struct ContextSectionContract {
    trust_level: ContextTrustLevel,
    source_kind: ContextSourceKind,
    injection_reason: &'static str,
    freshness: &'static str,
    scope: &'static str,
}

impl ContextSectionContract {
    fn for_kind(kind: ContextSectionKind) -> Self {
        match kind {
            ContextSectionKind::System | ContextSectionKind::Developer => Self {
                trust_level: ContextTrustLevel::High,
                source_kind: ContextSourceKind::Runtime,
                injection_reason: "runtime_prompt",
                freshness: "current",
                scope: "runtime",
            },
            ContextSectionKind::Relationship => Self {
                trust_level: ContextTrustLevel::High,
                source_kind: ContextSourceKind::AcceptedMemory,
                injection_reason: "relationship_core",
                freshness: "stable",
                scope: "user",
            },
            ContextSectionKind::References => Self {
                trust_level: ContextTrustLevel::High,
                source_kind: ContextSourceKind::ExplicitReference,
                injection_reason: "user_explicit_reference",
                freshness: "current",
                scope: "workspace",
            },
            ContextSectionKind::History => Self {
                trust_level: ContextTrustLevel::Medium,
                source_kind: ContextSourceKind::SessionHistory,
                injection_reason: "recent_episode_history",
                freshness: "recent",
                scope: "session",
            },
            ContextSectionKind::MemoryProjection => Self {
                trust_level: ContextTrustLevel::High,
                source_kind: ContextSourceKind::MemoryProjection,
                injection_reason: "accepted_memory_projection",
                freshness: "stable",
                scope: "user",
            },
            ContextSectionKind::WorkingMemory => Self {
                trust_level: ContextTrustLevel::Medium,
                source_kind: ContextSourceKind::WorkingMemory,
                injection_reason: "session_working_memory",
                freshness: "current",
                scope: "session",
            },
            ContextSectionKind::RetrievedMemory => Self {
                trust_level: ContextTrustLevel::MediumLow,
                source_kind: ContextSourceKind::RetrievedMemory,
                injection_reason: "on_demand_memory_search",
                freshness: "retrieved",
                scope: "user",
            },
            ContextSectionKind::Rag => Self {
                trust_level: ContextTrustLevel::MediumLow,
                source_kind: ContextSourceKind::RagIndex,
                injection_reason: "explicit_reference_retrieval",
                freshness: "retrieved",
                scope: "workspace",
            },
            ContextSectionKind::ToolResults => Self {
                trust_level: ContextTrustLevel::Medium,
                source_kind: ContextSourceKind::ToolResult,
                injection_reason: "tool_result",
                freshness: "current",
                scope: "session",
            },
        }
    }
}

fn push_section(
    sections: &mut Vec<ContextSection>,
    kind: ContextSectionKind,
    label: &str,
    lines: Vec<String>,
    estimator: &dyn TokenEstimator,
) {
    if lines.is_empty() {
        return;
    }
    let estimated_tokens = lines
        .iter()
        .map(|line| estimator.estimate_tokens(line))
        .sum();
    let contract = ContextSectionContract::for_kind(kind);
    sections.push(ContextSection {
        kind,
        label: label.to_owned(),
        trust_level: contract.trust_level,
        source_kind: contract.source_kind,
        injection_reason: contract.injection_reason.to_owned(),
        freshness: Some(contract.freshness.to_owned()),
        scope: Some(contract.scope.to_owned()),
        lines,
        estimated_tokens,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HeuristicTokenEstimator;

    #[test]
    fn context_sections_carry_trust_source_and_injection_reason() {
        let context = ChatContext {
            relationship: vec!["User prefers concise updates".into()],
            references: vec!["@file:README.md".into()],
            history: vec!["user: hello".into()],
            memory_projection: vec!["# User\n- Do not commit without approval".into()],
            working_memory: vec!["[working turn/default] current PR scope is docs".into()],
            retrieved_memory: vec!["[knowledge/default] RAG is reference context".into()],
            rag: vec!["docs/context.md: RAG reference".into()],
        };

        let sections = context.to_sections(&HeuristicTokenEstimator);
        let relationship = sections
            .iter()
            .find(|section| section.kind == ContextSectionKind::Relationship)
            .expect("relationship section");
        assert_eq!(relationship.trust_level, ContextTrustLevel::High);
        assert_eq!(relationship.source_kind, ContextSourceKind::AcceptedMemory);
        assert_eq!(relationship.injection_reason, "relationship_core");
        assert_eq!(relationship.freshness.as_deref(), Some("stable"));
        assert_eq!(relationship.scope.as_deref(), Some("user"));

        let memory_projection = sections
            .iter()
            .find(|section| section.kind == ContextSectionKind::MemoryProjection)
            .expect("memory projection section");
        assert_eq!(memory_projection.trust_level, ContextTrustLevel::High);
        assert_eq!(
            memory_projection.source_kind,
            ContextSourceKind::MemoryProjection
        );
        assert_eq!(
            memory_projection.injection_reason,
            "accepted_memory_projection"
        );
        assert_eq!(memory_projection.freshness.as_deref(), Some("stable"));
        assert_eq!(memory_projection.scope.as_deref(), Some("user"));

        let working_memory = sections
            .iter()
            .find(|section| section.kind == ContextSectionKind::WorkingMemory)
            .expect("working memory section");
        assert_eq!(working_memory.trust_level, ContextTrustLevel::Medium);
        assert_eq!(working_memory.source_kind, ContextSourceKind::WorkingMemory);
        assert_eq!(working_memory.injection_reason, "session_working_memory");
        assert_eq!(working_memory.freshness.as_deref(), Some("current"));
        assert_eq!(working_memory.scope.as_deref(), Some("session"));

        let retrieved_memory = sections
            .iter()
            .find(|section| section.kind == ContextSectionKind::RetrievedMemory)
            .expect("retrieved memory section");
        assert_eq!(retrieved_memory.trust_level, ContextTrustLevel::MediumLow);
        assert_eq!(
            retrieved_memory.source_kind,
            ContextSourceKind::RetrievedMemory
        );
        assert_eq!(retrieved_memory.injection_reason, "on_demand_memory_search");
        assert_eq!(retrieved_memory.freshness.as_deref(), Some("retrieved"));
        assert_eq!(retrieved_memory.scope.as_deref(), Some("user"));

        let rag = sections
            .iter()
            .find(|section| section.kind == ContextSectionKind::Rag)
            .expect("rag section");
        assert_eq!(rag.trust_level, ContextTrustLevel::MediumLow);
        assert_eq!(rag.source_kind, ContextSourceKind::RagIndex);
        assert_eq!(rag.injection_reason, "explicit_reference_retrieval");
        assert_eq!(rag.freshness.as_deref(), Some("retrieved"));
        assert_eq!(rag.scope.as_deref(), Some("workspace"));
    }
}
