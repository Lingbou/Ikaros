// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, IkarosPaths, Result, redact_secrets, reject_secret_like};
use ikaros_harness::{AuditEvent, AuditLog};
use ikaros_soul::{BehaviorRule, PersonaLoader, PersonaProfile, PersonalityTrait, load_or_default};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersonaPatch {
    pub name: Option<String>,
    pub role: Option<String>,
    pub description: Option<String>,
    pub tone_style: Option<String>,
    pub tone_language: Option<String>,
    pub relationship_stance: Option<String>,
    pub traits: Vec<String>,
    pub boundaries: Vec<String>,
    pub behavior_rules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersonaWriteReport {
    pub path: PathBuf,
    pub name: String,
    pub role: String,
    pub changed_fields: Vec<String>,
    pub audit_path: PathBuf,
}

pub fn update_persona(paths: &IkarosPaths, patch: PersonaPatch) -> Result<PersonaWriteReport> {
    paths.ensure()?;
    let current = load_or_default(&paths.persona)?;
    let mut changed_fields = Vec::new();
    let updated = apply_persona_patch(current, patch, &mut changed_fields)?;
    write_persona(paths, &updated, changed_fields, "persona updated")
}

pub fn reset_persona(paths: &IkarosPaths) -> Result<PersonaWriteReport> {
    paths.ensure()?;
    let persona = PersonaLoader::parse(PersonaLoader::default_markdown())?;
    write_persona(paths, &persona, vec!["reset".into()], "persona reset")
}

fn apply_persona_patch(
    mut persona: PersonaProfile,
    patch: PersonaPatch,
    changed_fields: &mut Vec<String>,
) -> Result<PersonaProfile> {
    if let Some(name) = patch.name {
        reject_secret_like(&name, "persona name")?;
        persona.identity.name = name;
        changed_fields.push("name".into());
    }
    if let Some(role) = patch.role {
        reject_secret_like(&role, "persona role")?;
        persona.identity.role = role;
        changed_fields.push("role".into());
    }
    if let Some(description) = patch.description {
        reject_secret_like(&description, "persona description")?;
        persona.identity.description = description;
        changed_fields.push("description".into());
    }
    if let Some(style) = patch.tone_style {
        reject_secret_like(&style, "persona tone style")?;
        persona.tone.style = style;
        changed_fields.push("tone_style".into());
    }
    if let Some(language) = patch.tone_language {
        reject_secret_like(&language, "persona tone language")?;
        persona.tone.language = Some(language);
        changed_fields.push("tone_language".into());
    }
    if let Some(stance) = patch.relationship_stance {
        reject_secret_like(&stance, "persona relationship stance")?;
        persona.relationship.stance = stance;
        changed_fields.push("relationship_stance".into());
    }
    if !patch.traits.is_empty() {
        reject_secret_like(&patch.traits.join("\n"), "persona traits")?;
        persona.traits = patch
            .traits
            .into_iter()
            .map(|name| PersonalityTrait {
                name,
                description: None,
            })
            .collect();
        changed_fields.push("traits".into());
    }
    if !patch.boundaries.is_empty() {
        reject_secret_like(
            &patch.boundaries.join("\n"),
            "persona relationship boundaries",
        )?;
        persona.relationship.boundaries = patch.boundaries;
        changed_fields.push("boundaries".into());
    }
    if !patch.behavior_rules.is_empty() {
        reject_secret_like(&patch.behavior_rules.join("\n"), "persona behavior rules")?;
        persona.behavior_rules = patch
            .behavior_rules
            .into_iter()
            .map(|text| BehaviorRule { text })
            .collect();
        changed_fields.push("behavior_rules".into());
    }
    if changed_fields.is_empty() {
        return Err(IkarosError::Message(
            "persona set requires at least one field to change".into(),
        ));
    }
    Ok(persona)
}

fn write_persona(
    paths: &IkarosPaths,
    persona: &PersonaProfile,
    changed_fields: Vec<String>,
    message: &str,
) -> Result<PersonaWriteReport> {
    let markdown = render_persona_markdown(persona);
    if redact_secrets(&markdown) != markdown {
        return Err(IkarosError::SecretRejected("persona markdown".into()));
    }
    fs::write(&paths.persona, &markdown)
        .map_err(|source| ikaros_core::IkarosError::io(&paths.persona, source))?;
    let audit = AuditLog::new(&paths.audit_dir);
    audit.append(AuditEvent::new(
        "persona_updated",
        None,
        message,
        json!({
            "path": &paths.persona,
            "name": persona.identity.name,
            "role": persona.identity.role,
            "changed_fields": changed_fields,
        }),
    )?)?;
    Ok(PersonaWriteReport {
        path: paths.persona.clone(),
        name: persona.identity.name.clone(),
        role: persona.identity.role.clone(),
        changed_fields,
        audit_path: audit.path().to_path_buf(),
    })
}

pub fn render_persona_markdown(persona: &PersonaProfile) -> String {
    let language = persona
        .tone
        .language
        .clone()
        .unwrap_or_else(|| "user-preferred".into());
    format!(
        "# Identity\nname: {}\nrole: {}\ndescription: {}\n\n# Traits\n{}\n\n# Tone\nstyle: {}\nlanguage: {}\n\n# Relationship\nstance: {}\n{}\n\n# Behavior Rules\n{}\n",
        persona.identity.name,
        persona.identity.role,
        persona.identity.description,
        render_list(persona.traits.iter().map(|item| item.name.as_str())),
        persona.tone.style,
        language,
        persona.relationship.stance,
        render_list(persona.relationship.boundaries.iter().map(String::as_str)),
        render_list(persona.behavior_rules.iter().map(|rule| rule.text.as_str())),
    )
}

fn render_list<'a>(items: impl Iterator<Item = &'a str>) -> String {
    let rendered = items
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n");
    if rendered.is_empty() {
        "- none".into()
    } else {
        rendered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_persona_writes_markdown_and_audit() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = IkarosPaths::from_home(temp.path().join("home"));

        let report = update_persona(
            &paths,
            PersonaPatch {
                name: Some("Ika".into()),
                tone_style: Some("concise and warm".into()),
                traits: vec!["focused".into(), "careful".into()],
                ..PersonaPatch::default()
            },
        )
        .expect("update");

        assert_eq!(report.name, "Ika");
        assert!(report.changed_fields.contains(&"name".into()));
        let loaded = PersonaLoader::load(&paths.persona).expect("load");
        assert_eq!(loaded.identity.name, "Ika");
        assert_eq!(loaded.tone.style, "concise and warm");
        assert!(report.audit_path.exists());
    }

    #[test]
    fn update_persona_rejects_secret_like_values() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = IkarosPaths::from_home(temp.path().join("home"));

        let err = update_persona(
            &paths,
            PersonaPatch {
                description: Some("token=abc123".into()),
                ..PersonaPatch::default()
            },
        )
        .expect_err("secret rejected");

        assert!(err.to_string().contains("persona description"));
    }
}
