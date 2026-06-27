// SPDX-License-Identifier: GPL-3.0-only

use crate::TokenEstimator;
use ikaros_core::redact_secrets;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptBuildReport {
    pub prompt: String,
    pub sections: Vec<PromptSection>,
    pub estimated_tokens: usize,
}

impl PromptBuildReport {
    pub fn metadata(&self) -> PromptBuildMetadata {
        let stable_sections = self
            .sections
            .iter()
            .filter(|section| prompt_section_is_stable_cache_prefix(section))
            .cloned()
            .collect::<Vec<_>>();
        let mut stable_prefix_messages = Vec::new();
        push_rendered_prompt_message(&mut stable_prefix_messages, &stable_sections);
        let stable_prefix = stable_prefix_messages.join("\n\n");
        PromptBuildMetadata {
            estimated_tokens: self.estimated_tokens,
            section_count: self.sections.len(),
            stable_prefix_message_count: stable_prefix_messages.len(),
            stable_prefix_estimated_tokens: stable_sections
                .iter()
                .map(|section| section.estimated_tokens)
                .sum(),
            stable_prefix_hash: stable_prompt_prefix_hash(&stable_prefix),
            sections: self
                .sections
                .iter()
                .map(PromptSectionMetadata::from)
                .collect(),
        }
    }

    pub fn system_messages_for_prompt_cache(&self) -> Vec<String> {
        let mut stable = Vec::new();
        let mut dynamic = Vec::new();
        for section in &self.sections {
            if prompt_section_is_stable_cache_prefix(section) {
                stable.push(section.clone());
            } else {
                dynamic.push(section.clone());
            }
        }

        let mut messages = Vec::new();
        push_rendered_prompt_message(&mut messages, &stable);
        push_rendered_prompt_message(&mut messages, &dynamic);
        if messages.is_empty() && !self.prompt.trim().is_empty() {
            messages.push(self.prompt.clone());
        }
        messages
    }

    pub fn prompt_cache_plan(&self, provider_policy: impl Into<String>) -> PromptCachePlan {
        PromptCachePlan::from_report(self, provider_policy)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptBuildMetadata {
    pub estimated_tokens: usize,
    pub section_count: usize,
    pub stable_prefix_message_count: usize,
    pub stable_prefix_estimated_tokens: usize,
    pub stable_prefix_hash: String,
    pub sections: Vec<PromptSectionMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptCachePlan {
    pub schema: String,
    pub provider_policy: String,
    pub eligible: bool,
    pub reason: String,
    pub stable_prefix_hash: String,
    pub stable_prefix_message_count: usize,
    pub stable_prefix_estimated_tokens: usize,
    pub stable_prefix_section_count: usize,
    pub dynamic_section_count: usize,
    pub system_message_count: usize,
    pub byte_stable_prefix: bool,
    pub sections: Vec<PromptCacheSectionPlan>,
}

impl PromptCachePlan {
    pub fn from_report(report: &PromptBuildReport, provider_policy: impl Into<String>) -> Self {
        let provider_policy = provider_policy.into();
        let metadata = report.metadata();
        let system_message_count = report.system_messages_for_prompt_cache().len();
        let stable_prefix_section_count = metadata
            .sections
            .iter()
            .filter(|section| section.cache_stable_prefix)
            .count();
        let dynamic_section_count = metadata
            .section_count
            .saturating_sub(stable_prefix_section_count);
        let eligible = provider_policy != "none"
            && metadata.stable_prefix_message_count > 0
            && metadata.stable_prefix_estimated_tokens > 0;
        let reason = if provider_policy == "none" {
            "provider policy disables prompt caching"
        } else if metadata.stable_prefix_message_count == 0 {
            "no stable prefix system message"
        } else if metadata.stable_prefix_estimated_tokens == 0 {
            "stable prefix has no estimated tokens"
        } else {
            "stable prefix can be sent before dynamic context"
        }
        .to_owned();
        Self {
            schema: "ikaros-prompt-cache-plan-v1".into(),
            provider_policy,
            eligible,
            reason,
            stable_prefix_hash: metadata.stable_prefix_hash,
            stable_prefix_message_count: metadata.stable_prefix_message_count,
            stable_prefix_estimated_tokens: metadata.stable_prefix_estimated_tokens,
            stable_prefix_section_count,
            dynamic_section_count,
            system_message_count,
            byte_stable_prefix: true,
            sections: metadata
                .sections
                .iter()
                .map(PromptCacheSectionPlan::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptCacheSectionPlan {
    pub kind: PromptSectionKind,
    pub source: PromptSourceKind,
    pub title: String,
    pub cache_stable_prefix: bool,
    pub estimated_tokens: usize,
    pub redaction: PromptRedactionState,
}

impl From<&PromptSectionMetadata> for PromptCacheSectionPlan {
    fn from(section: &PromptSectionMetadata) -> Self {
        Self {
            kind: section.kind,
            source: section.source,
            title: section.title.clone(),
            cache_stable_prefix: section.cache_stable_prefix,
            estimated_tokens: section.estimated_tokens,
            redaction: section.redaction,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptSectionMetadata {
    pub kind: PromptSectionKind,
    pub title: String,
    pub source: PromptSourceKind,
    pub priority: u8,
    pub estimated_tokens: usize,
    pub redaction: PromptRedactionState,
    pub cache_stable_prefix: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptSection {
    pub kind: PromptSectionKind,
    pub title: String,
    pub content: String,
    pub source: PromptSourceKind,
    pub priority: u8,
    pub estimated_tokens: usize,
    pub redaction: PromptRedactionState,
}

impl From<&PromptSection> for PromptSectionMetadata {
    fn from(section: &PromptSection) -> Self {
        Self {
            kind: section.kind,
            title: section.title.clone(),
            source: section.source,
            priority: section.priority,
            estimated_tokens: section.estimated_tokens,
            redaction: section.redaction,
            cache_stable_prefix: prompt_section_is_stable_cache_prefix(section),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PromptSectionKind {
    System,
    Developer,
    Persona,
    Policy,
    Relationship,
    References,
    History,
    MemoryProjection,
    WorkingMemory,
    RetrievedMemory,
    Rag,
    ContextCompression,
    ToolGuidance,
    ToolResults,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PromptSourceKind {
    Runtime,
    Persona,
    Policy,
    Context,
    Memory,
    Rag,
    Tooling,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PromptRedactionState {
    Unchanged,
    Redacted,
}

pub struct PromptBuilder<'a> {
    estimator: &'a dyn TokenEstimator,
    sections: Vec<PromptSection>,
}

impl<'a> PromptBuilder<'a> {
    pub fn new(estimator: &'a dyn TokenEstimator) -> Self {
        Self {
            estimator,
            sections: Vec::new(),
        }
    }

    pub fn add_section(
        mut self,
        kind: PromptSectionKind,
        title: impl Into<String>,
        content: impl Into<String>,
        source: PromptSourceKind,
        priority: u8,
    ) -> Self {
        let original = content.into();
        let content = redact_secrets(&original);
        let redaction = if content == original {
            PromptRedactionState::Unchanged
        } else {
            PromptRedactionState::Redacted
        };
        let estimated_tokens = self.estimator.estimate_tokens(&content).max(1);
        self.sections.push(PromptSection {
            kind,
            title: title.into(),
            content,
            source,
            priority,
            estimated_tokens,
            redaction,
        });
        self
    }

    pub fn add_optional_section(
        self,
        kind: PromptSectionKind,
        title: impl Into<String>,
        content: impl Into<String>,
        source: PromptSourceKind,
        priority: u8,
    ) -> Self {
        let content = content.into();
        if content.trim().is_empty() {
            return self;
        }
        self.add_section(kind, title, content, source, priority)
    }

    pub fn build(self) -> PromptBuildReport {
        let prompt = render_prompt_sections(&self.sections);
        let estimated_tokens = self
            .sections
            .iter()
            .map(|section| section.estimated_tokens)
            .sum();
        PromptBuildReport {
            prompt,
            sections: self.sections,
            estimated_tokens,
        }
    }
}

fn render_prompt_sections(sections: &[PromptSection]) -> String {
    sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            if index == 0 && section.kind == PromptSectionKind::Persona {
                section.content.clone()
            } else {
                format!("{}:\n{}", section.title, section.content)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn push_rendered_prompt_message(messages: &mut Vec<String>, sections: &[PromptSection]) {
    if sections.is_empty() {
        return;
    }
    let rendered = render_prompt_sections(sections);
    if !rendered.trim().is_empty() {
        messages.push(rendered);
    }
}

fn prompt_section_is_stable_cache_prefix(section: &PromptSection) -> bool {
    matches!(
        section.source,
        PromptSourceKind::Runtime
            | PromptSourceKind::Persona
            | PromptSourceKind::Policy
            | PromptSourceKind::Tooling
    )
}

fn stable_prompt_prefix_hash(prompt: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    if prompt.trim().is_empty() {
        return "fnv1a64:0000000000000000".into();
    }

    let mut hash = FNV_OFFSET;
    for byte in prompt.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("fnv1a64:{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HeuristicTokenEstimator;
    use serde_json::json;

    #[test]
    fn prompt_section_protocol_kinds_are_stable_and_renderable() {
        let report = PromptBuilder::new(&HeuristicTokenEstimator)
            .add_section(
                PromptSectionKind::System,
                "System",
                "stable runtime instruction",
                PromptSourceKind::Runtime,
                100,
            )
            .add_section(
                PromptSectionKind::Developer,
                "Developer",
                "developer instruction",
                PromptSourceKind::Policy,
                95,
            )
            .add_section(
                PromptSectionKind::ToolResults,
                "Tool results",
                "tool result summary",
                PromptSourceKind::Tooling,
                80,
            )
            .build();

        assert_eq!(report.sections.len(), 3);
        assert!(
            report
                .prompt
                .contains("System:\nstable runtime instruction")
        );
        assert!(report.prompt.contains("Developer:\ndeveloper instruction"));
        assert!(report.prompt.contains("Tool results:\ntool result summary"));
        assert_eq!(
            serde_json::to_value(report.sections[0].kind).expect("serialize system kind"),
            json!("system")
        );
        assert_eq!(
            serde_json::to_value(report.sections[1].kind).expect("serialize developer kind"),
            json!("developer")
        );
        assert_eq!(
            serde_json::to_value(report.sections[2].kind).expect("serialize tool results kind"),
            json!("tool_results")
        );
        assert!(report.estimated_tokens >= report.sections.len());
    }

    #[test]
    fn prompt_build_metadata_excludes_section_content() {
        let report = PromptBuilder::new(&HeuristicTokenEstimator)
            .add_section(
                PromptSectionKind::References,
                "Local reference context",
                "line one token=abc123\nline two",
                PromptSourceKind::Context,
                95,
            )
            .build();

        let metadata = report.metadata();
        let metadata_json = serde_json::to_value(&metadata).expect("metadata json");

        assert_eq!(metadata.section_count, 1);
        assert_eq!(metadata.sections[0].kind, PromptSectionKind::References);
        assert_eq!(metadata.sections[0].source, PromptSourceKind::Context);
        assert!(!metadata.sections[0].cache_stable_prefix);
        assert!(metadata.sections[0].estimated_tokens > 0);
        assert!(metadata_json["sections"][0].get("content").is_none());
        assert_eq!(
            metadata_json["sections"][0]["cache_stable_prefix"],
            json!(false)
        );
        assert!(!metadata_json.to_string().contains("line one"));
        assert!(!metadata_json.to_string().contains("abc123"));
    }

    #[test]
    fn prompt_cache_messages_keep_dynamic_context_out_of_stable_prefix() {
        let report = PromptBuilder::new(&HeuristicTokenEstimator)
            .add_section(
                PromptSectionKind::Persona,
                "Persona",
                "stable persona",
                PromptSourceKind::Persona,
                100,
            )
            .add_section(
                PromptSectionKind::Policy,
                "Policy",
                "stable policy",
                PromptSourceKind::Runtime,
                100,
            )
            .add_section(
                PromptSectionKind::History,
                "Local chat history context",
                "dynamic history token=abc123",
                PromptSourceKind::Context,
                70,
            )
            .add_section(
                PromptSectionKind::RetrievedMemory,
                "Retrieved memory context",
                "dynamic memory",
                PromptSourceKind::Memory,
                60,
            )
            .build();

        let messages = report.system_messages_for_prompt_cache();

        assert_eq!(messages.len(), 2);
        assert!(messages[0].contains("stable persona"));
        assert!(messages[0].contains("stable policy"));
        assert!(!messages[0].contains("dynamic history"));
        assert!(!messages[0].contains("dynamic memory"));
        assert!(messages[1].contains("dynamic history"));
        assert!(messages[1].contains("[REDACTED_SECRET]"));
        assert!(messages[1].contains("dynamic memory"));
    }

    #[test]
    fn prompt_metadata_exposes_stable_prefix_hash_without_dynamic_context() {
        let first = PromptBuilder::new(&HeuristicTokenEstimator)
            .add_section(
                PromptSectionKind::Persona,
                "Persona",
                "stable persona",
                PromptSourceKind::Persona,
                100,
            )
            .add_section(
                PromptSectionKind::Policy,
                "Policy",
                "stable policy",
                PromptSourceKind::Runtime,
                100,
            )
            .add_section(
                PromptSectionKind::History,
                "Local chat history context",
                "dynamic history token=abc123",
                PromptSourceKind::Context,
                70,
            )
            .build()
            .metadata();
        let dynamic_changed = PromptBuilder::new(&HeuristicTokenEstimator)
            .add_section(
                PromptSectionKind::Persona,
                "Persona",
                "stable persona",
                PromptSourceKind::Persona,
                100,
            )
            .add_section(
                PromptSectionKind::Policy,
                "Policy",
                "stable policy",
                PromptSourceKind::Runtime,
                100,
            )
            .add_section(
                PromptSectionKind::History,
                "Local chat history context",
                "different dynamic history",
                PromptSourceKind::Context,
                70,
            )
            .build()
            .metadata();
        let stable_changed = PromptBuilder::new(&HeuristicTokenEstimator)
            .add_section(
                PromptSectionKind::Persona,
                "Persona",
                "different stable persona",
                PromptSourceKind::Persona,
                100,
            )
            .add_section(
                PromptSectionKind::Policy,
                "Policy",
                "stable policy",
                PromptSourceKind::Runtime,
                100,
            )
            .add_section(
                PromptSectionKind::History,
                "Local chat history context",
                "dynamic history token=abc123",
                PromptSourceKind::Context,
                70,
            )
            .build()
            .metadata();

        assert_eq!(first.stable_prefix_message_count, 1);
        assert!(first.stable_prefix_estimated_tokens > 0);
        assert_eq!(
            first
                .sections
                .iter()
                .map(|section| section.cache_stable_prefix)
                .collect::<Vec<_>>(),
            vec![true, true, false]
        );
        assert_eq!(first.stable_prefix_hash, dynamic_changed.stable_prefix_hash);
        assert_ne!(first.stable_prefix_hash, stable_changed.stable_prefix_hash);
        let metadata_json = serde_json::to_string(&first).expect("metadata json");
        assert!(metadata_json.contains("\"cache_stable_prefix\":true"));
        assert!(metadata_json.contains("\"cache_stable_prefix\":false"));
        assert!(!metadata_json.contains("dynamic history"));
        assert!(!metadata_json.contains("abc123"));
    }
}
