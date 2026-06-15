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
    pub memory: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rag: Vec<String>,
}

impl ChatContext {
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
            ContextSectionKind::Memory,
            "memory",
            self.memory.clone(),
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
            references,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextSection {
    pub kind: ContextSectionKind,
    pub label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<String>,
    pub estimated_tokens: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ContextSectionKind {
    System,
    Developer,
    Relationship,
    References,
    History,
    Memory,
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
            ContextSectionKind::Memory => "memory",
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
    sections.push(ContextSection {
        kind,
        label: label.to_owned(),
        lines,
        estimated_tokens,
    });
}
