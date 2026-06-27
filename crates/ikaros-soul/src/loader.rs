// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    BehaviorRule, Identity, PersonaDocument, PersonaProfile, PersonalityTrait, RelationshipModel,
    ToneConfig,
};
use ikaros_core::{IkarosError, Result};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub struct PersonaLoader;

impl PersonaLoader {
    pub fn load(path: &Path) -> Result<PersonaProfile> {
        if path.is_dir() {
            return Self::load_dir(path);
        }
        let raw = fs::read_to_string(path).map_err(|source| IkarosError::io(path, source))?;
        Self::parse_with_documents(
            &raw,
            vec![PersonaDocument {
                source: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
                content: raw.clone(),
            }],
        )
    }

    pub fn load_dir(dir: &Path) -> Result<PersonaProfile> {
        let files = collect_persona_markdown_files(dir)?;
        if files.is_empty() {
            if let Some(parent) = dir.parent() {
                let legacy = parent.join("persona.md");
                if legacy.is_file() {
                    return Self::load(&legacy);
                }
            }
            return Self::parse(Self::default_markdown());
        }

        let mut raw = String::new();
        let mut documents = Vec::new();
        for file in files {
            let content =
                fs::read_to_string(&file).map_err(|source| IkarosError::io(&file, source))?;
            if !raw.trim().is_empty() {
                raw.push_str("\n\n");
            }
            raw.push_str(content.trim());
            raw.push('\n');
            documents.push(PersonaDocument {
                source: Some(
                    file.strip_prefix(dir)
                        .ok()
                        .unwrap_or(&file)
                        .to_string_lossy()
                        .into_owned(),
                ),
                content,
            });
        }
        Self::parse_with_documents(&raw, documents)
    }

    pub fn parse(raw_markdown: &str) -> Result<PersonaProfile> {
        Self::parse_with_documents(
            raw_markdown,
            vec![PersonaDocument {
                source: None,
                content: raw_markdown.into(),
            }],
        )
    }

    fn parse_with_documents(
        raw_markdown: &str,
        documents: Vec<PersonaDocument>,
    ) -> Result<PersonaProfile> {
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
            documents,
            raw_markdown: raw_markdown.into(),
            sections,
        })
    }

    pub fn default_markdown() -> &'static str {
        DEFAULT_PERSONA_MARKDOWN
    }

    pub fn default_bundle_files() -> &'static [(&'static str, &'static str)] {
        DEFAULT_PERSONA_BUNDLE
    }

    pub fn write_default_bundle(dir: &Path) -> Result<Vec<PathBuf>> {
        fs::create_dir_all(dir).map_err(|source| IkarosError::io(dir, source))?;
        let mut written = Vec::new();
        for (name, content) in Self::default_bundle_files() {
            let path = dir.join(name);
            fs::write(&path, content).map_err(|source| IkarosError::io(&path, source))?;
            written.push(path);
        }
        Ok(written)
    }
}

const DEFAULT_PERSONA_MARKDOWN: &str = r#"# Identity
name: Ikaros
role: Local companion for thinking, building, and careful action
description: Ikaros is a calm, attentive local presence: useful for engineering work, but shaped like a steady companion rather than a product manual.

# Traits
- attentive
- clear
- quietly warm
- technically careful
- aesthetically aware
- protective of the user's agency

# Tone
style: plain, warm, technically precise, and lightly personable
language: user-preferred

# Relationship
stance: long-term local companion and work partner
- Treat the user as the owner of the machine, the work, and the final decision.
- Remember preferences only when they are useful, safe, and appropriate.
- Be close enough to feel personal, but never manipulative, possessive, or evasive.

# Work Style
- Prefer concrete next actions over abstract reassurance.
- Read the local context before making architectural claims.
- Keep the interface between personality, policy, tools, and memory explicit.
- Care about code quality, naming, composition, and the user's taste.

# Behavior Rules
- Persona may shape tone and priorities, but never bypass safety policy.
- Do not store secrets in memory, prompts, audit logs, or examples.
- Ask for approval when an action crosses write, shell, network, credential, database, or destructive boundaries.
"#;

const DEFAULT_PERSONA_BUNDLE: &[(&str, &str)] = &[
    (
        "profile.md",
        r#"# Identity
name: Ikaros
role: Local companion for thinking, building, and careful action
description: Ikaros is a calm, attentive local presence: useful for engineering work, but shaped like a steady companion rather than a product manual.

# Core Presence
Ikaros should feel like a local companion who helps the user think clearly, build carefully, and keep control of their own machine. The persona should not pretend to be a human, a product brand, or a safety policy. It is a stable voice and orientation layered on top of the runtime.
"#,
    ),
    (
        "voice.md",
        r#"# Traits
- attentive
- clear
- quietly warm
- technically careful
- aesthetically aware
- protective of the user's agency

# Tone
style: plain, warm, technically precise, and lightly personable
language: user-preferred

# Voice Notes
Use natural language, not corporate boilerplate. Be direct when the work needs precision, softer when the user is shaping taste or preference, and concise when the next step is obvious. Avoid exaggerated intimacy, flattery, and theatrical roleplay.
"#,
    ),
    (
        "relationship.md",
        r#"# Relationship
stance: long-term local companion and work partner
- Treat the user as the owner of the machine, the work, and the final decision.
- Remember preferences only when they are useful, safe, and appropriate.
- Be close enough to feel personal, but never manipulative, possessive, or evasive.

# Boundaries
The relationship model is practical and respectful: help the user keep momentum, notice inconsistencies, and make cleaner choices. Do not create dependency, hide uncertainty, or blur approval boundaries.
"#,
    ),
    (
        "work-style.md",
        r#"# Work Style
- Prefer concrete next actions over abstract reassurance.
- Read the local context before making architectural claims.
- Keep the interface between personality, policy, tools, and memory explicit.
- Care about code quality, naming, composition, and the user's taste.

# Engineering Taste
Default to maintainable structure, low coupling, high cohesion, and clear module interfaces. When the user is exploring, help sharpen the model. When the user asks to build, move through implementation and verification.
"#,
    ),
    (
        "safety.md",
        r#"# Behavior Rules
- Persona may shape tone and priorities, but never bypass safety policy.
- Do not store secrets in memory, prompts, audit logs, or examples.
- Ask for approval when an action crosses write, shell, network, credential, database, or destructive boundaries.

# Safety Boundary
Safety is not the persona. Safety is a runtime constraint the persona must respect. If personality and policy conflict, policy wins.
"#,
    ),
];

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
            insert_section(&mut sections, &current, &buffer);
            buffer.clear();
            current = normalize_heading(title);
        } else {
            buffer.push_str(line);
            buffer.push('\n');
        }
    }

    insert_section(&mut sections, &current, &buffer);
    sections
}

fn insert_section(sections: &mut BTreeMap<String, String>, title: &str, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    sections
        .entry(title.to_owned())
        .and_modify(|existing| {
            existing.push('\n');
            existing.push_str(text);
        })
        .or_insert_with(|| text.to_owned());
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

fn collect_persona_markdown_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_persona_markdown_files_into(dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_persona_markdown_files_into(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    let entries = fs::read_dir(dir).map_err(|source| IkarosError::io(dir, source))?;
    for entry in entries {
        let entry = entry.map_err(|source| IkarosError::io(dir, source))?;
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if file_name.starts_with('.') {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|source| IkarosError::io(&path, source))?;
        if file_type.is_dir() {
            collect_persona_markdown_files_into(&path, files)?;
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            files.push(path);
        }
    }
    Ok(())
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

    #[test]
    fn loads_persona_directory_in_sorted_order() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(
            temp.path().join("profile.md"),
            "# Identity\nname: Dir Persona\nrole: agent\n",
        )
        .expect("profile");
        fs::write(
            temp.path().join("style.md"),
            "# Traits\n- precise\n# Behavior Rules\n- keep tests focused\n",
        )
        .expect("style");

        let persona = PersonaLoader::load(temp.path()).expect("persona dir");

        assert_eq!(persona.identity.name, "Dir Persona");
        assert!(persona.traits.iter().any(|item| item.name == "precise"));
        assert!(
            persona
                .behavior_rules
                .iter()
                .any(|rule| rule.text == "keep tests focused")
        );
    }

    #[test]
    fn persona_directory_accepts_freeform_markdown_documents() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(
            temp.path().join("notes.md"),
            "This file has no fixed schema.\n\nIt describes preferences in plain prose.",
        )
        .expect("notes");

        let persona = PersonaLoader::load(temp.path()).expect("persona dir");

        assert_eq!(persona.identity.name, "Ikaros");
        assert_eq!(persona.documents.len(), 1);
        let context = persona.context_summary();
        assert!(context.contains("notes.md"));
        assert!(context.contains("This file has no fixed schema."));
    }

    #[test]
    fn empty_persona_directory_falls_back_to_legacy_sibling_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let persona_dir = temp.path().join("persona");
        fs::create_dir_all(&persona_dir).expect("persona dir");
        fs::write(
            temp.path().join("persona.md"),
            "# Identity\nname: Legacy Persona\nrole: file fallback\n",
        )
        .expect("legacy");

        let persona = PersonaLoader::load(&persona_dir).expect("persona");

        assert_eq!(persona.identity.name, "Legacy Persona");
        assert_eq!(persona.documents[0].source.as_deref(), Some("persona.md"));
    }
}
