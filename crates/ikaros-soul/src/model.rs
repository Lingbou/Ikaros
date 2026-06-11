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
        format!(
            "Persona: {}\nRole: {}\nTone: {}\nTraits: {}\nRelationship stance: {}\nRules:\n{}",
            self.identity.name,
            self.identity.role,
            self.tone.style,
            traits,
            self.relationship.stance,
            rules
        )
    }
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
}
