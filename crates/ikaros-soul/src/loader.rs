// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    BehaviorRule, Identity, PersonaProfile, PersonalityTrait, RelationshipModel, ToneConfig,
};
use ikaros_core::{IkarosError, Result};
use std::{collections::BTreeMap, fs, path::Path};

pub struct PersonaLoader;

impl PersonaLoader {
    pub fn load(path: &Path) -> Result<PersonaProfile> {
        let raw = fs::read_to_string(path).map_err(|source| IkarosError::io(path, source))?;
        Self::parse(&raw)
    }

    pub fn parse(raw_markdown: &str) -> Result<PersonaProfile> {
        let sections = parse_sections(raw_markdown);
        let identity_text = section(&sections, "identity").unwrap_or(raw_markdown);
        let name = parse_field(identity_text, "name").unwrap_or_else(|| "Ikaros".into());
        let role = parse_field(identity_text, "role")
            .unwrap_or_else(|| "persona-first local agent runtime".into());
        let description = parse_field(identity_text, "description")
            .or_else(|| first_non_heading_line(identity_text))
            .unwrap_or_else(|| "A bounded, auditable AI companion and work assistant.".into());

        let traits = parse_list_section(section(&sections, "traits").unwrap_or(""))
            .into_iter()
            .map(|line| PersonalityTrait {
                name: line,
                description: None,
            })
            .collect::<Vec<_>>();
        let traits = if traits.is_empty() {
            vec![
                PersonalityTrait {
                    name: "warm".into(),
                    description: Some("empathetic without weakening safety boundaries".into()),
                },
                PersonalityTrait {
                    name: "rigorous".into(),
                    description: Some("careful with tools, memory, and auditability".into()),
                },
            ]
        } else {
            traits
        };

        let tone_text = section(&sections, "tone").unwrap_or("");
        let tone = ToneConfig {
            style: parse_field(tone_text, "style").unwrap_or_else(|| "calm, direct, kind".into()),
            language: parse_field(tone_text, "language"),
        };

        let relationship_text = section(&sections, "relationship").unwrap_or("");
        let relationship = RelationshipModel {
            stance: parse_field(relationship_text, "stance")
                .unwrap_or_else(|| "long-term companion with explicit boundaries".into()),
            boundaries: parse_list_section(section(&sections, "boundaries").unwrap_or(""))
                .into_iter()
                .chain(parse_list_section(relationship_text))
                .collect(),
        };

        let behavior_rules = parse_list_section(section(&sections, "behavior rules").unwrap_or(""))
            .into_iter()
            .map(|text| BehaviorRule { text })
            .collect::<Vec<_>>();
        let behavior_rules = if behavior_rules.is_empty() {
            vec![
                BehaviorRule {
                    text: "Never bypass harness policy or approval gates.".into(),
                },
                BehaviorRule {
                    text: "Never store secrets in memory, prompts, or audit logs.".into(),
                },
            ]
        } else {
            behavior_rules
        };

        Ok(PersonaProfile {
            identity: Identity {
                name,
                role,
                description,
            },
            traits,
            tone,
            relationship,
            behavior_rules,
            raw_markdown: raw_markdown.into(),
            sections,
        })
    }

    pub fn default_markdown() -> &'static str {
        r#"# Identity
name: Ikaros
role: Persona-first local agent runtime
description: A warm, bounded AI companion who can also perform engineering work through an auditable harness.

# Traits
- warm
- focused
- security-minded
- pragmatic

# Tone
style: calm, direct, caring, technically precise
language: user-preferred

# Relationship
stance: long-term companion and work assistant with explicit safety boundaries
- remembers durable preferences only when useful and safe
- asks for approval when actions cross risk boundaries

# Behavior Rules
- Persona may shape tone and priorities, but never bypass safety policy.
- Do not store secrets in memory, prompts, audit logs, or examples.
- Use local-first memory by default; use RAG only when reference retrieval is explicitly enabled for the turn or profile.
"#
    }
}

pub fn load_or_default(path: &Path) -> Result<PersonaProfile> {
    if path.exists() {
        PersonaLoader::load(path)
    } else {
        PersonaLoader::parse(PersonaLoader::default_markdown())
    }
}

fn parse_sections(raw: &str) -> BTreeMap<String, String> {
    let mut sections = BTreeMap::new();
    let mut current = String::from("root");
    let mut buffer = String::new();

    for line in raw.lines() {
        if let Some(title) = heading_title(line) {
            if !buffer.trim().is_empty() {
                sections.insert(current.clone(), buffer.trim().to_string());
                buffer.clear();
            }
            current = normalize_heading(title);
        } else {
            buffer.push_str(line);
            buffer.push('\n');
        }
    }

    if !buffer.trim().is_empty() {
        sections.insert(current, buffer.trim().to_string());
    }
    sections
}

fn heading_title(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let without_hash = trimmed.strip_prefix('#')?.trim_start_matches('#').trim();
    if without_hash.is_empty() {
        None
    } else {
        Some(without_hash)
    }
}

fn normalize_heading(title: &str) -> String {
    title.trim().to_ascii_lowercase()
}

fn section<'a>(sections: &'a BTreeMap<String, String>, title: &str) -> Option<&'a str> {
    sections.get(&normalize_heading(title)).map(String::as_str)
}

fn parse_field(text: &str, key: &str) -> Option<String> {
    let needle = format!("{key}:").to_ascii_lowercase();
    text.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.to_ascii_lowercase().starts_with(&needle) {
            trimmed
                .split_once(':')
                .map(|(_, value)| value.trim().to_string())
                .filter(|value| !value.is_empty())
        } else {
            None
        }
    })
}

fn first_non_heading_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && !line.contains(':'))
        .map(ToOwned::to_owned)
}

fn parse_list_section(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_and_unknown_sections() {
        let persona = PersonaLoader::parse(
            r#"# Identity
name: Test Ikaros
role: agent

# Unknown
kept

# Traits
- precise
"#,
        )
        .expect("persona");
        assert_eq!(persona.identity.name, "Test Ikaros");
        assert_eq!(persona.traits[0].name, "precise");
        assert!(persona.sections.contains_key("unknown"));
    }
}
