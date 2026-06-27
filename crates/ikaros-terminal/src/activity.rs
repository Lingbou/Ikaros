// SPDX-License-Identifier: GPL-3.0-only

use crate::terminal_inline;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolActivityStatus {
    Started,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolActivity {
    pub name: String,
    pub status: ToolActivityStatus,
    pub detail: Option<String>,
}

impl ToolActivity {
    pub fn new(name: impl Into<String>, status: ToolActivityStatus) -> Self {
        Self {
            name: name.into(),
            status,
            detail: None,
        }
    }

    pub fn completed(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: ToolActivityStatus::Completed,
            detail: Some(detail.into()),
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

pub fn render_tool_activity(activity: &ToolActivity) -> Vec<String> {
    let mut lines = vec![tool_activity_title(activity)];
    if let Some(detail) = activity
        .detail
        .as_deref()
        .map(terminal_inline)
        .filter(|detail| !detail.is_empty())
    {
        lines.push(format!("  -{detail}"));
    }
    lines
}

pub fn human_activity_line_for_terminal(line: &str, color: bool) -> String {
    if !color {
        return line.to_owned();
    }
    if let Some(rest) = line
        .strip_prefix("* ")
        .or_else(|| line.strip_prefix("\u{2022} "))
    {
        format!("\x1b[32m\u{2022}\x1b[0m {rest}")
    } else if let Some(rest) = line
        .strip_prefix('*')
        .filter(|rest| !rest.trim().is_empty())
    {
        format!("\x1b[32m\u{2022}\x1b[0m {}", rest.trim_start())
    } else if let Some(rest) = line
        .strip_prefix("  * ")
        .or_else(|| line.strip_prefix("  \u{2022} "))
    {
        format!("  \x1b[36m\u{2022}\x1b[0m {rest}")
    } else if let Some(rest) = line
        .strip_prefix("  -")
        .or_else(|| line.strip_prefix("  \u{2514} "))
        .filter(|rest| !rest.trim().is_empty())
    {
        format!("  \x1b[36m\u{2514}\x1b[0m {}", rest.trim_start())
    } else {
        line.to_owned()
    }
}

fn tool_activity_title(activity: &ToolActivity) -> String {
    let name = terminal_inline(&activity.name);
    match activity.status {
        ToolActivityStatus::Started => format!("*Running {name}"),
        ToolActivityStatus::Failed => format!("*Tool failed: {name}"),
        ToolActivityStatus::Cancelled => format!("*Cancelled {name}"),
        ToolActivityStatus::Completed => completed_tool_activity_title(&name),
    }
}

fn completed_tool_activity_title(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.contains("read")
        || lower.contains("search")
        || lower.contains("list")
        || lower.contains("grep")
        || lower.contains("rg")
        || lower.contains("find")
        || lower.contains("explore")
    {
        "*Explored".to_owned()
    } else if lower.contains("write")
        || lower.contains("edit")
        || lower.contains("patch")
        || lower.contains("apply")
        || lower.contains("create")
    {
        "*Edited".to_owned()
    } else if lower.contains("test")
        || lower.contains("check")
        || lower.contains("clippy")
        || lower.contains("build")
    {
        "*Checked".to_owned()
    } else {
        format!("*Ran {name}")
    }
}

#[cfg(test)]
mod tests {
    use super::human_activity_line_for_terminal;

    #[test]
    fn human_activity_line_keeps_plain_text_when_not_terminal() {
        assert_eq!(
            human_activity_line_for_terminal("* Explored", false),
            "* Explored"
        );
        assert_eq!(
            human_activity_line_for_terminal("*Explored", false),
            "*Explored"
        );
        assert_eq!(
            human_activity_line_for_terminal("\u{2022} Explored", false),
            "\u{2022} Explored"
        );
        assert_eq!(
            human_activity_line_for_terminal("  * Read SKILL.md", false),
            "  * Read SKILL.md"
        );
        assert_eq!(
            human_activity_line_for_terminal("  -Read SKILL.md", false),
            "  -Read SKILL.md"
        );
        assert_eq!(
            human_activity_line_for_terminal("  \u{2514} Read SKILL.md", false),
            "  \u{2514} Read SKILL.md"
        );
    }

    #[test]
    fn human_activity_line_colors_bullet_when_terminal() {
        assert_eq!(
            human_activity_line_for_terminal("* Explored", true),
            "\x1b[32m\u{2022}\x1b[0m Explored"
        );
        assert_eq!(
            human_activity_line_for_terminal("*Explored", true),
            "\x1b[32m\u{2022}\x1b[0m Explored"
        );
        assert_eq!(
            human_activity_line_for_terminal("\u{2022} Explored", true),
            "\x1b[32m\u{2022}\x1b[0m Explored"
        );
        assert_eq!(
            human_activity_line_for_terminal("  * Read SKILL.md", true),
            "  \x1b[36m\u{2022}\x1b[0m Read SKILL.md"
        );
        assert_eq!(
            human_activity_line_for_terminal("  -Read SKILL.md", true),
            "  \x1b[36m\u{2514}\x1b[0m Read SKILL.md"
        );
        assert_eq!(
            human_activity_line_for_terminal("  \u{2514} Read SKILL.md", true),
            "  \x1b[36m\u{2514}\x1b[0m Read SKILL.md"
        );
    }
}
