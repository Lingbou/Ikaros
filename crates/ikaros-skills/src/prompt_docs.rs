// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::RiskLevel;
use ikaros_harness::{
    PromptSkillDocument, PromptSkillSupportFile, SkillDescriptor, SkillDescriptorKind,
    ToolExecutionMode, Toolset,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

const SKILL_DOC_NAME: &str = "SKILL.md";
const MAX_SUPPORT_FILE_BYTES: usize = 64 * 1024;

pub(crate) fn load_prompt_skill_documents(skills_dir: &Path) -> Vec<PromptSkillDocument> {
    let Ok(entries) = fs::read_dir(skills_dir) else {
        return Vec::new();
    };
    let mut documents = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = prompt_skill_name_from_dir(&path) else {
            continue;
        };
        let doc_path = path.join(SKILL_DOC_NAME);
        if !doc_path.exists() {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&doc_path) else {
            continue;
        };
        documents.push(parse_prompt_skill_document(&name, &raw, &path));
    }
    documents.sort_by(|left, right| left.descriptor.name.cmp(&right.descriptor.name));
    documents
}

fn parse_prompt_skill_document(name: &str, raw: &str, skill_dir: &Path) -> PromptSkillDocument {
    let (metadata, instructions) = split_front_matter(raw);
    let description = metadata
        .get("description")
        .cloned()
        .or_else(|| infer_description(instructions))
        .unwrap_or_else(|| format!("Prompt instructions from {SKILL_DOC_NAME}."));
    let toolset = metadata
        .get("toolset")
        .and_then(|value| Toolset::parse(value))
        .unwrap_or(Toolset::Plugin);
    let provenance = metadata
        .get("provenance")
        .cloned()
        .unwrap_or_else(|| format!("local:{name}/{SKILL_DOC_NAME}"));
    let support_files = prompt_skill_support_files(&metadata, skill_dir);
    let support_file_contents =
        prompt_skill_support_file_contents(&support_files, skill_dir, instructions);

    PromptSkillDocument {
        descriptor: SkillDescriptor {
            name: name.to_owned(),
            description,
            input_schema: json!({"type": "object"}),
            risk_level: RiskLevel::SafeRead,
            kind: SkillDescriptorKind::PromptSkill,
            disable_model_invocation: true,
            execution_mode: ToolExecutionMode::Sequential,
            toolset,
            timeout_ms: None,
            provenance: Some(provenance),
            support_files,
        },
        instructions: instructions.trim().to_owned(),
        support_files: support_file_contents,
    }
}

fn prompt_skill_name_from_dir(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?.trim();
    is_prompt_skill_name(name).then(|| name.to_owned())
}

fn is_prompt_skill_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphanumeric() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn split_front_matter(raw: &str) -> (BTreeMap<String, String>, &str) {
    let mut metadata = BTreeMap::new();
    let mut lines = raw.lines();
    if lines.next() != Some("---") {
        return (metadata, raw);
    }

    let mut byte_offset = raw.find('\n').map_or(raw.len(), |index| index + 1);
    for line in lines {
        let line_len = line.len();
        if line == "---" {
            let body_start = byte_offset + line_len + newline_len(raw, byte_offset + line_len);
            return (metadata, raw.get(body_start..).unwrap_or_default());
        }
        if let Some((key, value)) = line.split_once(':') {
            metadata.insert(key.trim().to_ascii_lowercase(), unquote(value.trim()));
        }
        byte_offset += line_len + newline_len(raw, byte_offset + line_len);
    }

    (BTreeMap::new(), raw)
}

fn newline_len(raw: &str, newline_index: usize) -> usize {
    match raw.as_bytes().get(newline_index) {
        Some(b'\r') if raw.as_bytes().get(newline_index + 1) == Some(&b'\n') => 2,
        Some(b'\n') => 1,
        _ => 0,
    }
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
        .to_owned()
}

fn prompt_skill_support_files(
    metadata: &BTreeMap<String, String>,
    skill_dir: &Path,
) -> Vec<PathBuf> {
    let mut files = vec![PathBuf::from(SKILL_DOC_NAME)];
    let Some(raw) = metadata.get("support_files") else {
        return files;
    };
    for candidate in parse_support_file_list(raw) {
        if !is_safe_support_file_path(&candidate) {
            continue;
        }
        if !skill_dir.join(&candidate).is_file() {
            continue;
        }
        if !files.iter().any(|existing| existing == &candidate) {
            files.push(candidate);
        }
    }
    files
}

fn prompt_skill_support_file_contents(
    support_files: &[PathBuf],
    skill_dir: &Path,
    instructions: &str,
) -> Vec<PromptSkillSupportFile> {
    support_files
        .iter()
        .filter_map(|path| {
            let content = if path == Path::new(SKILL_DOC_NAME) {
                instructions.trim().to_owned()
            } else {
                fs::read_to_string(skill_dir.join(path)).ok()?
            };
            let (content, truncated) = truncate_support_file_content(content);
            Some(PromptSkillSupportFile {
                path: path.clone(),
                content,
                truncated,
            })
        })
        .collect()
}

fn truncate_support_file_content(content: String) -> (String, bool) {
    if content.len() <= MAX_SUPPORT_FILE_BYTES {
        return (content, false);
    }
    let mut end = 0;
    for (index, ch) in content.char_indices() {
        let next = index + ch.len_utf8();
        if next > MAX_SUPPORT_FILE_BYTES {
            break;
        }
        end = next;
    }
    (content[..end].to_owned(), true)
}

fn parse_support_file_list(raw: &str) -> Vec<PathBuf> {
    let trimmed = raw.trim();
    let list = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(trimmed);
    list.split(',')
        .map(str::trim)
        .map(unquote)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn is_safe_support_file_path(path: &Path) -> bool {
    path.is_relative()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn infer_description(instructions: &str) -> Option<String> {
    instructions
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('#').trim().to_owned())
        .filter(|line| !line.is_empty())
}
