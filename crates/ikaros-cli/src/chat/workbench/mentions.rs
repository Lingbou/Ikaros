// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use std::{collections::BTreeMap, fs, path::Path};

use super::terminal_inline;

const MAX_MENTION_CANDIDATES: usize = 40;
const MAX_SCAN_DEPTH: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
struct MentionCandidate {
    label: String,
    kind: &'static str,
}

pub(in crate::chat) fn print_context_mentions(workspace: &Path, query: Option<&str>) -> Result<()> {
    let query = query.map(str::trim).filter(|query| !query.is_empty());
    if let Some(query) = query {
        println!("mentions_query: {}", terminal_inline(query));
    } else {
        println!("mentions_query: all");
    }
    let candidates = context_mention_candidates(workspace, query)?;
    println!("mentions_found: {}", candidates.len());
    for candidate in candidates {
        println!(
            "- {} kind={}",
            terminal_inline(&candidate.label),
            candidate.kind
        );
    }
    Ok(())
}

#[allow(dead_code)]
pub(in crate::chat) fn print_context_mentions_for_human(
    workspace: &Path,
    query: Option<&str>,
) -> Result<()> {
    for line in context_mentions_human_lines(workspace, query)? {
        println!("{line}");
    }
    Ok(())
}

pub(in crate::chat) fn context_mentions_human_lines(
    workspace: &Path,
    query: Option<&str>,
) -> Result<Vec<String>> {
    let query = query.map(str::trim).filter(|query| !query.is_empty());
    let candidates = context_mention_candidates(workspace, query)?;

    let mut lines = vec![
        "• Mentions".to_owned(),
        format!(
            "  query: {}",
            query.map(terminal_inline).unwrap_or_else(|| "all".into())
        ),
    ];
    if candidates.is_empty() {
        lines.push("  matches: none".to_owned());
        lines.push("  examples: @diff, @staged, @git:HEAD, @file:path".to_owned());
        return Ok(lines);
    }

    lines.push(format!(
        "  matches: {}{}",
        candidates.len(),
        if candidates.len() == MAX_MENTION_CANDIDATES {
            " shown"
        } else {
            ""
        }
    ));
    for candidate in candidates {
        lines.push(format!(
            "  - {} ({})",
            terminal_inline(&candidate.label),
            candidate.kind
        ));
    }
    Ok(lines)
}

fn context_mention_candidates(
    workspace: &Path,
    query: Option<&str>,
) -> Result<Vec<MentionCandidate>> {
    let mut candidates = fixed_context_mentions(query);
    candidates.extend(workspace_mentions(workspace, query)?);
    candidates.sort_by(|left, right| left.label.cmp(&right.label).then(left.kind.cmp(right.kind)));
    candidates.dedup_by(|left, right| left.label == right.label && left.kind == right.kind);
    candidates.truncate(MAX_MENTION_CANDIDATES);
    Ok(candidates)
}

fn fixed_context_mentions(query: Option<&str>) -> Vec<MentionCandidate> {
    let fixed = [
        MentionCandidate {
            label: "@diff".into(),
            kind: "context",
        },
        MentionCandidate {
            label: "@staged".into(),
            kind: "context",
        },
        MentionCandidate {
            label: "@git:HEAD".into(),
            kind: "context",
        },
    ];
    let Some(query) = query else {
        return fixed.into_iter().collect();
    };
    let query = query.to_ascii_lowercase();
    let include_context_group = ["context", "diff", "staged", "git", "head"]
        .iter()
        .any(|tag| tag.contains(&query) || query.contains(tag));
    fixed
        .into_iter()
        .filter(|candidate| {
            include_context_group || candidate.label.to_ascii_lowercase().contains(&query)
        })
        .collect()
}

fn workspace_mentions(workspace: &Path, query: Option<&str>) -> Result<Vec<MentionCandidate>> {
    let mut candidates = BTreeMap::<String, MentionCandidate>::new();
    collect_workspace_mentions(workspace, workspace, query, 0, &mut candidates)?;
    Ok(candidates.into_values().collect())
}

fn collect_workspace_mentions(
    workspace: &Path,
    current: &Path,
    query: Option<&str>,
    depth: usize,
    candidates: &mut BTreeMap<String, MentionCandidate>,
) -> Result<()> {
    if depth > MAX_SCAN_DEPTH || should_skip_path(current) {
        return Ok(());
    }
    let Ok(entries) = fs::read_dir(current) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if should_skip_path(&path) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let relative = relative_slash_path(workspace, &path);
        if relative.is_empty() {
            continue;
        }
        if metadata.is_dir() {
            let folder_label = format!("@folder:{relative}");
            if mention_matches(&folder_label, query) {
                insert_candidate(candidates, folder_label, "folder");
            }
            collect_workspace_mentions(workspace, &path, query, depth + 1, candidates)?;
        } else if metadata.is_file() {
            let file_label = format!("@file:{relative}");
            if mention_matches(&file_label, query) {
                insert_candidate(candidates, file_label, "file");
                if let Some(parent) = Path::new(&relative).parent() {
                    let parent = parent.to_string_lossy().replace('\\', "/");
                    if !parent.is_empty() {
                        insert_candidate(candidates, format!("@folder:{parent}"), "folder");
                    }
                }
            }
        }
    }
    Ok(())
}

fn insert_candidate(
    candidates: &mut BTreeMap<String, MentionCandidate>,
    label: String,
    kind: &'static str,
) {
    candidates
        .entry(label.clone())
        .or_insert(MentionCandidate { label, kind });
}

fn relative_slash_path(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
        .chars()
        .map(|ch| if ch.is_control() { '_' } else { ch })
        .collect()
}

fn should_skip_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name,
                ".git" | ".ikaros" | "target" | "node_modules" | ".next" | "dist"
            )
        })
}

fn mention_matches(label: &str, query: Option<&str>) -> bool {
    let Some(query) = query else {
        return true;
    };
    let query = query.to_ascii_lowercase();
    let label = label.to_ascii_lowercase();
    label.contains(&query) || fuzzy_subsequence(&label, &query)
}

fn fuzzy_subsequence(value: &str, query: &str) -> bool {
    let mut query_chars = query.chars();
    let Some(mut next) = query_chars.next() else {
        return true;
    };
    for ch in value.chars() {
        if ch == next {
            let Some(candidate) = query_chars.next() else {
                return true;
            };
            next = candidate;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn mention_search_includes_parent_folder_for_file_match() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("src")).expect("src");
        fs::write(temp.path().join("src/lib.rs"), "pub fn lib() {}\n").expect("lib");

        let mentions = workspace_mentions(temp.path(), Some("lib")).expect("mentions");
        let labels = mentions
            .into_iter()
            .map(|candidate| candidate.label)
            .collect::<Vec<_>>();

        assert!(labels.contains(&"@file:src/lib.rs".to_owned()));
        assert!(labels.contains(&"@folder:src".to_owned()));
    }

    #[test]
    fn diff_query_expands_context_group() {
        let labels = fixed_context_mentions(Some("diff"))
            .into_iter()
            .map(|candidate| candidate.label)
            .collect::<Vec<_>>();

        assert!(labels.contains(&"@diff".to_owned()));
        assert!(labels.contains(&"@staged".to_owned()));
        assert!(labels.contains(&"@git:HEAD".to_owned()));
    }

    #[cfg(unix)]
    #[test]
    fn mention_scan_does_not_follow_symlinked_external_directories() {
        let temp = tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&workspace).expect("workspace");
        fs::create_dir_all(&outside).expect("outside");
        fs::write(outside.join("leak.txt"), "secret outside workspace").expect("leak");
        std::os::unix::fs::symlink(&outside, workspace.join("linked")).expect("symlink");

        let mentions = workspace_mentions(&workspace, Some("leak")).expect("mentions");

        assert!(
            mentions.is_empty(),
            "workspace mention scan followed an external symlink: {mentions:?}"
        );
    }

    #[test]
    fn generated_workspace_mentions_are_single_line_terminal_candidates() {
        let temp = tempdir().expect("tempdir");
        for name in ["normal.rs", "with-space.rs"] {
            fs::write(temp.path().join(name), "pub fn sample() {}\n").expect("sample file");
        }

        let mentions = workspace_mentions(temp.path(), Some("with")).expect("mentions");

        assert!(!mentions.is_empty());
        for mention in mentions {
            assert!(
                mention
                    .label
                    .chars()
                    .all(|ch| !ch.is_control() && ch != '\n' && ch != '\r' && ch != '\t'),
                "mention candidate must be single-line safe: {mention:?}"
            );
        }
    }

    #[test]
    fn relative_slash_path_replaces_control_characters() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("with\ttab\nnewline\rcarriage.rs");

        let relative = relative_slash_path(temp.path(), &path);

        assert_eq!(relative, "with_tab_newline_carriage.rs");
    }
}
