// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersonaProfile {
    pub identity: Identity,
    pub traits: Vec<PersonalityTrait>,
    pub tone: ToneConfig,
    pub relationship: RelationshipModel,
    pub behavior_rules: Vec<BehaviorRule>,
    pub documents: Vec<PersonaDocument>,
    pub raw_markdown: String,
    pub sections: BTreeMap<String, String>,
}

impl PersonaProfile {
    pub fn context_summary(&self) -> String {
        let traits = self
            .traits
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let rules = self
            .behavior_rules
            .iter()
            .map(|rule| format!("- {}", rule.text))
            .collect::<Vec<_>>()
            .join("\n");
        let mut summary = format!(
            "Persona: {}\nRole: {}\nTone: {}\nTraits: {}\nRelationship stance: {}\nRules:\n{}",
            self.identity.name,
            self.identity.role,
            self.tone.style,
            traits,
            self.relationship.stance,
            rules
        );
        if let Some(documents) = self.document_context(12_000) {
            summary.push_str("\n\nPersona source documents:\n");
            summary.push_str(&documents);
        }
        summary
    }

    fn document_context(&self, limit: usize) -> Option<String> {
        let mut rendered = String::new();
        for document in &self.documents {
            let content = document.content.trim();
            if content.is_empty() {
                continue;
            }
            if !rendered.is_empty() {
                rendered.push_str("\n\n");
            }
            rendered.push_str("## ");
            rendered.push_str(document.source.as_deref().unwrap_or("inline persona"));
            rendered.push('\n');
            rendered.push_str(content);
            if rendered.len() > limit {
                truncate_to_char_boundary(&mut rendered, limit);
                rendered.push_str("\n[persona documents truncated]");
                break;
            }
        }
        if rendered.trim().is_empty() {
            None
        } else {
            Some(rendered)
        }
    }
}

fn truncate_to_char_boundary(value: &mut String, limit: usize) {
    if value.len() <= limit {
        return;
    }

    let mut boundary = limit;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersonaDocument {
    pub source: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub role: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersonalityTrait {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToneConfig {
    pub style: String,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationshipModel {
    pub stance: String,
    pub boundaries: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BehaviorRule {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationshipMemory {
    pub user_name: Option<String>,
    pub trust_level: u8,
    pub notes: Vec<String>,
}

impl Default for RelationshipMemory {
    fn default() -> Self {
        Self {
            user_name: None,
            trust_level: 1,
            notes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EmotionState {
    Neutral,
    Focused,
    Curious,
    Confused,
    Concerned,
    Satisfied,
}

impl EmotionState {
    pub fn for_runtime_signal(signal: RuntimeSignal) -> Self {
        match signal {
            RuntimeSignal::Planning => Self::Focused,
            RuntimeSignal::Research => Self::Curious,
            RuntimeSignal::TestFailure => Self::Confused,
            RuntimeSignal::RiskAction => Self::Concerned,
            RuntimeSignal::TaskComplete => Self::Satisfied,
            RuntimeSignal::Idle => Self::Neutral,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RuntimeSignal {
    Planning,
    Research,
    TestFailure,
    RiskAction,
    TaskComplete,
    Idle,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_runtime_signals_to_emotions() {
        assert_eq!(
            EmotionState::for_runtime_signal(RuntimeSignal::RiskAction),
            EmotionState::Concerned
        );
        assert_eq!(
            EmotionState::for_runtime_signal(RuntimeSignal::TaskComplete),
            EmotionState::Satisfied
        );
    }

    #[test]
    fn persona_document_context_truncates_on_utf8_boundary() {
        let profile = PersonaProfile {
            identity: Identity {
                name: "测试人格".into(),
                role: "persona".into(),
                description: "多字节 persona".into(),
            },
            traits: Vec::new(),
            tone: ToneConfig {
                style: "测试语气".into(),
                language: Some("中文".into()),
            },
            relationship: RelationshipModel {
                stance: "测试关系".into(),
                boundaries: Vec::new(),
            },
            behavior_rules: Vec::new(),
            documents: vec![PersonaDocument {
                source: Some("voice.md".into()),
                content: "测".repeat(100),
            }],
            raw_markdown: String::new(),
            sections: BTreeMap::new(),
        };

        let context = profile
            .document_context(17)
            .expect("truncated document context");

        assert!(context.contains("[persona documents truncated]"));
        assert!(context.is_char_boundary(context.len()));
    }
}
