// SPDX-License-Identifier: GPL-3.0-only

use super::terminal_inline;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SlashCommandDescriptor {
    pub(super) name: &'static str,
    pub(super) usage: &'static str,
    pub(super) summary: &'static str,
    pub(super) tags: &'static [&'static str],
}

const SLASH_COMMANDS: &[SlashCommandDescriptor] = &[
    SlashCommandDescriptor {
        name: "/help",
        usage: "/help",
        summary: "show workbench commands",
        tags: &["commands", "help"],
    },
    SlashCommandDescriptor {
        name: "/commands",
        usage: "/commands [query]",
        summary: "search slash commands",
        tags: &["commands", "search", "fuzzy"],
    },
    SlashCommandDescriptor {
        name: "/queue",
        usage: "/queue [message|clear]",
        summary: "inspect, enqueue, or clear pending input messages",
        tags: &["input", "pending", "queue"],
    },
    SlashCommandDescriptor {
        name: "/agents",
        usage: "/agents",
        summary: "list configured agent profiles",
        tags: &["agent", "profile"],
    },
    SlashCommandDescriptor {
        name: "/agent",
        usage: "/agent <profile>",
        summary: "switch active agent profile",
        tags: &["agent", "profile"],
    },
    SlashCommandDescriptor {
        name: "/status",
        usage: "/status",
        summary: "show unified workbench status",
        tags: &["session", "provider", "gateway", "approval"],
    },
    SlashCommandDescriptor {
        name: "/sessions",
        usage: "/sessions",
        summary: "list recent chat sessions",
        tags: &["session", "history"],
    },
    SlashCommandDescriptor {
        name: "/session",
        usage: "/session status|resume|history|timeline",
        summary: "inspect or change current session",
        tags: &["session", "resume", "timeline"],
    },
    SlashCommandDescriptor {
        name: "/resume",
        usage: "/resume <session>",
        summary: "resume a session id",
        tags: &["session", "resume"],
    },
    SlashCommandDescriptor {
        name: "/new",
        usage: "/new",
        summary: "start a fresh session id",
        tags: &["session"],
    },
    SlashCommandDescriptor {
        name: "/fork",
        usage: "/fork [summary]",
        summary: "branch the current session from its active leaf",
        tags: &["session", "branch", "tree"],
    },
    SlashCommandDescriptor {
        name: "/timeline",
        usage: "/timeline",
        summary: "show recent session timeline cells",
        tags: &["session", "replay", "debug"],
    },
    SlashCommandDescriptor {
        name: "/replay",
        usage: "/replay",
        summary: "show a longer session replay view",
        tags: &["session", "replay"],
    },
    SlashCommandDescriptor {
        name: "/debug",
        usage: "/debug",
        summary: "show verbose session debug cells",
        tags: &["session", "debug"],
    },
    SlashCommandDescriptor {
        name: "/trace",
        usage: "/trace",
        summary: "show sanitized session trace spans",
        tags: &["session", "debug", "trace", "replay"],
    },
    SlashCommandDescriptor {
        name: "/mentions",
        usage: "/mentions [query]",
        summary: "search file, folder, git, diff, and staged context mentions",
        tags: &["context", "file", "folder", "mention", "reference"],
    },
    SlashCommandDescriptor {
        name: "/context",
        usage: "/context",
        summary: "show context budget settings",
        tags: &["context"],
    },
    SlashCommandDescriptor {
        name: "/memory",
        usage: "/memory",
        summary: "show memory policy settings",
        tags: &["memory"],
    },
    SlashCommandDescriptor {
        name: "/rag",
        usage: "/rag",
        summary: "show RAG settings",
        tags: &["rag", "context"],
    },
    SlashCommandDescriptor {
        name: "/model",
        usage: "/model",
        summary: "inspect active model descriptor",
        tags: &["provider", "model"],
    },
    SlashCommandDescriptor {
        name: "/provider",
        usage: "/provider [inspect|health [--live]|matrix [--live]]",
        summary: "inspect provider metadata or health",
        tags: &["provider", "model", "health"],
    },
    SlashCommandDescriptor {
        name: "/gateway",
        usage: "/gateway",
        summary: "show local gateway queue status",
        tags: &["gateway", "ingress"],
    },
    SlashCommandDescriptor {
        name: "/tasks",
        usage: "/tasks",
        summary: "show scheduled task status",
        tags: &["schedule", "task"],
    },
    SlashCommandDescriptor {
        name: "/approval",
        usage: "/approval",
        summary: "show pending approvals",
        tags: &["approval", "policy"],
    },
    SlashCommandDescriptor {
        name: "/diff",
        usage: "/diff",
        summary: "show current git diff summary",
        tags: &["coding", "diff"],
    },
    SlashCommandDescriptor {
        name: "/code",
        usage: "/code <plan|apply|test|review|rollback> ...",
        summary: "run coding workflow commands",
        tags: &["coding", "patch", "test", "review"],
    },
    SlashCommandDescriptor {
        name: "/multi",
        usage: "/multi",
        summary: "enter multiline message mode",
        tags: &["input", "multiline"],
    },
    SlashCommandDescriptor {
        name: "/clear",
        usage: "/clear",
        summary: "acknowledge a clear-screen request",
        tags: &["display"],
    },
    SlashCommandDescriptor {
        name: "/quit",
        usage: "/quit",
        summary: "exit the workbench",
        tags: &["exit"],
    },
];

pub(super) fn slash_commands() -> &'static [SlashCommandDescriptor] {
    SLASH_COMMANDS
}

pub(in crate::chat) fn print_slash_commands(query: Option<&str>) {
    let query = query.map(str::trim).filter(|query| !query.is_empty());
    if let Some(query) = query {
        println!("commands_query: {}", terminal_inline(query));
    } else {
        println!("commands_query: all");
    }
    let mut matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| query.is_none_or(|query| slash_command_matches(*command, query)))
        .collect::<Vec<_>>();
    matches.sort_by_key(|command| command.name);
    println!("commands_found: {}", matches.len());
    for command in matches {
        println!(
            "- {} usage={} summary={}",
            terminal_inline(command.name),
            terminal_inline(command.usage),
            terminal_inline(command.summary)
        );
    }
}

pub(in crate::chat) fn suggest_slash_command(input: &str) -> Option<&'static str> {
    let command = input.split_whitespace().next().unwrap_or(input).trim();
    if command.is_empty() {
        return None;
    }
    slash_commands()
        .iter()
        .filter_map(|candidate| {
            let score = edit_distance(command, candidate.name);
            (score <= 2).then_some((score, candidate.name))
        })
        .min_by_key(|(score, name)| (*score, *name))
        .map(|(_, name)| name)
}

fn slash_command_matches(command: SlashCommandDescriptor, query: &str) -> bool {
    let query = query.to_ascii_lowercase();
    let name = command.name.to_ascii_lowercase();
    let usage = command.usage.to_ascii_lowercase();
    let summary = command.summary.to_ascii_lowercase();
    name.contains(&query)
        || usage.contains(&query)
        || summary.contains(&query)
        || command.tags.iter().any(|tag| tag.contains(&query))
        || fuzzy_subsequence(&name, &query)
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

fn edit_distance(left: &str, right: &str) -> usize {
    let right_len = right.chars().count();
    let mut previous = (0..=right_len).collect::<Vec<_>>();
    let mut current = vec![0; right_len + 1];
    for (left_index, left_ch) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_ch) in right.chars().enumerate() {
            let insert = current[right_index] + 1;
            let delete = previous[right_index + 1] + 1;
            let replace = previous[right_index] + usize::from(left_ch != right_ch);
            current[right_index + 1] = insert.min(delete).min(replace);
        }
        previous.clone_from(&current);
    }
    previous[right_len]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_search_matches_session_aliases() {
        let matches = slash_commands()
            .iter()
            .copied()
            .filter(|command| slash_command_matches(*command, "sess"))
            .map(|command| command.name)
            .collect::<Vec<_>>();

        assert!(matches.contains(&"/sessions"));
        assert!(matches.contains(&"/session"));
        assert!(matches.contains(&"/resume"));
    }

    #[test]
    fn command_suggestion_finds_near_miss() {
        assert_eq!(suggest_slash_command("/sesions"), Some("/sessions"));
    }
}
