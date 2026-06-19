// SPDX-License-Identifier: GPL-3.0-only

use crate::{MemoryKind, MemoryPerspective, MemoryRecord};
use ikaros_core::{IkarosError, Result, redact_secrets, reject_secret_like};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryProjectionInput {
    pub user_scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perspective: Option<MemoryPerspective>,
    pub records: Vec<MemoryRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryProjection {
    pub user: String,
    pub project: String,
    pub general: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectionRenderer {
    pub user_char_limit: usize,
    pub project_char_limit: usize,
    pub general_char_limit: usize,
}

#[derive(Debug, Clone)]
pub struct MemoryProjectionFileStore {
    dir: PathBuf,
}

impl MemoryProjectionFileStore {
    pub fn new(memory_dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: memory_dir.into().join("projections"),
        }
    }

    pub fn write(
        &self,
        projection: &MemoryProjection,
        project_scope: Option<&str>,
    ) -> Result<Vec<PathBuf>> {
        fs::create_dir_all(&self.dir).map_err(|source| IkarosError::io(&self.dir, source))?;
        let mut paths = Vec::new();
        paths.push(self.write_file("USER.md", &projection.user)?);
        paths.push(self.write_file("MEMORY.md", &projection.general)?);
        if let Some(project_scope) = project_scope {
            let file = format!("PROJECT.{}.md", projection_scope_file_part(project_scope)?);
            paths.push(self.write_file(&file, &projection.project)?);
        } else {
            paths.push(self.write_file("PROJECT.md", &projection.project)?);
        }
        Ok(paths)
    }

    pub fn read(&self, project_scope: Option<&str>) -> Result<MemoryProjection> {
        let user = self.read_file("USER.md")?;
        let general = self.read_file("MEMORY.md")?;
        let project = if let Some(project_scope) = project_scope {
            let file = format!("PROJECT.{}.md", projection_scope_file_part(project_scope)?);
            self.read_file(&file)?
        } else {
            self.read_file("PROJECT.md")?
        };
        Ok(MemoryProjection {
            user,
            project,
            general,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn write_file(&self, file_name: &str, content: &str) -> Result<PathBuf> {
        let path = self.dir.join(file_name);
        fs::write(&path, content).map_err(|source| IkarosError::io(&path, source))?;
        Ok(path)
    }

    fn read_file(&self, file_name: &str) -> Result<String> {
        let path = self.dir.join(file_name);
        fs::read_to_string(&path).map_err(|source| IkarosError::io(&path, source))
    }
}

impl Default for ProjectionRenderer {
    fn default() -> Self {
        Self {
            user_char_limit: 1_600,
            project_char_limit: 2_400,
            general_char_limit: 1_600,
        }
    }
}

impl ProjectionRenderer {
    pub fn render(&self, input: MemoryProjectionInput) -> Result<MemoryProjection> {
        reject_secret_like(&input.user_scope, "memory projection user scope")?;
        if let Some(scope) = &input.project_scope {
            reject_secret_like(scope, "memory projection project scope")?;
        }

        let stable = input
            .records
            .into_iter()
            .filter(|record| is_projection_record(record, input.perspective.as_ref()))
            .collect::<Vec<_>>();
        let user_lines = stable
            .iter()
            .filter(|record| {
                record.scope == input.user_scope
                    && matches!(record.kind, MemoryKind::User | MemoryKind::Relationship)
            })
            .map(memory_projection_line)
            .collect::<Vec<_>>();
        let project_lines = stable
            .iter()
            .filter(|record| {
                input
                    .project_scope
                    .as_ref()
                    .is_some_and(|scope| &record.scope == scope)
                    && record.kind == MemoryKind::Project
            })
            .map(memory_projection_line)
            .collect::<Vec<_>>();
        let general_lines = stable
            .iter()
            .filter(|record| {
                matches!(record.kind, MemoryKind::Knowledge | MemoryKind::Persona)
                    || (record.kind == MemoryKind::Project && input.project_scope.is_none())
            })
            .map(memory_projection_line)
            .collect::<Vec<_>>();

        let project_title = input
            .project_scope
            .as_deref()
            .map(|scope| format!("# Project Memory: {scope}"))
            .unwrap_or_else(|| "# Project Memory".into());
        Ok(MemoryProjection {
            user: render_markdown_section(
                "# User",
                &[("Stable Memory", user_lines)],
                self.user_char_limit,
            ),
            project: render_markdown_section(
                &project_title,
                &[("Stable Project Facts", project_lines)],
                self.project_char_limit,
            ),
            general: render_markdown_section(
                "# General Memory",
                &[("Durable Lessons", general_lines)],
                self.general_char_limit,
            ),
        })
    }
}

fn projection_scope_file_part(scope: &str) -> Result<String> {
    reject_secret_like(scope, "memory projection scope")?;
    let sanitized = scope
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        return Err(IkarosError::Message(
            "memory projection scope must contain a file-safe character".into(),
        ));
    }
    Ok(sanitized)
}

fn is_projection_record(record: &MemoryRecord, perspective: Option<&MemoryPerspective>) -> bool {
    if !record.active || record.sensitive || record.kind == MemoryKind::Task {
        return false;
    }
    if record.perspective.as_ref() != perspective {
        return false;
    }
    if record.tags.iter().any(|tag| {
        matches!(
            tag.as_str(),
            "turn-summary" | "memory-lifecycle" | "policy-demoted"
        )
    }) {
        return false;
    }
    true
}

fn memory_projection_line(record: &MemoryRecord) -> String {
    let content = redact_secrets(record.content.trim());
    if content.starts_with("- ") {
        content
    } else {
        format!("- {content}")
    }
}

fn render_markdown_section(
    title: &str,
    sections: &[(&str, Vec<String>)],
    char_limit: usize,
) -> String {
    let mut output = String::new();
    output.push_str(title);
    output.push('\n');
    for (heading, lines) in sections {
        output.push('\n');
        output.push_str("## ");
        output.push_str(heading);
        output.push('\n');
        if lines.is_empty() {
            output.push_str("- No accepted memory.\n");
        } else {
            for line in lines {
                output.push_str(line);
                output.push('\n');
            }
        }
    }
    truncate_chars(output.trim_end(), char_limit)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut output = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= max_chars {
            output.push_str("\n- ... [projection truncated]");
            return output;
        }
        output.push(ch);
    }
    output
}
