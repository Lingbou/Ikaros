// SPDX-License-Identifier: GPL-3.0-only

use async_trait::async_trait;
use ikaros_core::{Result, RiskLevel};
use ikaros_soul::load_or_default;
use ikaros_toolkit::{Skill, SkillContext, SkillOutput};
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct PersonaLoadSkill {
    persona_path: PathBuf,
}

impl PersonaLoadSkill {
    pub(crate) fn new(persona_path: PathBuf) -> Self {
        Self { persona_path }
    }
}

#[async_trait]
impl Skill for PersonaLoadSkill {
    fn name(&self) -> &'static str {
        "persona_load"
    }

    fn description(&self) -> &'static str {
        "Load the active persona profile."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        let persona = load_or_default(&self.persona_path)?;
        let traits = persona
            .traits
            .iter()
            .map(|item| item.name.clone())
            .collect::<Vec<_>>();
        let behavior_rules = persona
            .behavior_rules
            .iter()
            .map(|rule| rule.text.clone())
            .collect::<Vec<_>>();
        let documents = persona
            .documents
            .iter()
            .map(|document| {
                json!({
                    "source": document.source.as_deref(),
                    "chars": document.content.chars().count(),
                })
            })
            .collect::<Vec<_>>();
        Ok(SkillOutput::new(
            format!("loaded persona {}", persona.identity.name),
            json!({
                "schema": "ikaros-persona-public-v1",
                "name": persona.identity.name,
                "role": persona.identity.role,
                "description": persona.identity.description,
                "tone": {
                    "style": persona.tone.style,
                    "language": persona.tone.language,
                },
                "traits": traits,
                "relationship": {
                    "stance": persona.relationship.stance,
                    "boundaries": persona.relationship.boundaries,
                },
                "behavior_rules": behavior_rules,
                "documents": documents,
            }),
        ))
    }
}
