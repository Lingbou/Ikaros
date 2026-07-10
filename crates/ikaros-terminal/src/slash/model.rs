// SPDX-License-Identifier: GPL-3.0-only

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SlashCommandDescriptor {
    pub(super) name: &'static str,
    pub(super) usage: &'static str,
    pub(super) summary: &'static str,
    pub(super) tags: &'static [&'static str],
    pub(super) permissions: &'static [SlashCommandPermission],
    pub(super) surfaces: &'static [SlashCommandSurface],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommandCompletion {
    pub name: &'static str,
    pub usage: &'static str,
    pub summary: &'static str,
    pub argument_model: &'static str,
    pub effect: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommandPaletteItem {
    pub name: &'static str,
    pub usage: &'static str,
    pub summary: &'static str,
    pub argument_model: &'static str,
    pub effect: &'static str,
    pub permissions: String,
    pub surfaces: String,
    pub tags: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommandPaletteSummary {
    pub query: String,
    pub command_count: usize,
    pub total_commands: usize,
    pub effects: String,
    pub permissions: String,
    pub surfaces: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SlashCommandPermission {
    Read,
    SessionControl,
    AgentControl,
    Provider,
    Config,
    Network,
    Approval,
    WorkspaceWrite,
    Shell,
    Coding,
}

impl SlashCommandPermission {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::SessionControl => "session-control",
            Self::AgentControl => "agent-control",
            Self::Provider => "provider",
            Self::Config => "config",
            Self::Network => "network",
            Self::Approval => "approval",
            Self::WorkspaceWrite => "workspace-write",
            Self::Shell => "shell",
            Self::Coding => "coding",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SlashCommandSurface {
    Workbench,
}

impl SlashCommandSurface {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Workbench => "workbench",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SlashCommandArgumentModel {
    None,
    Optional,
    Required,
    Subcommand,
}

impl SlashCommandArgumentModel {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Optional => "optional",
            Self::Required => "required",
            Self::Subcommand => "subcommand",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SlashCommandEffect {
    ReadOnly,
    ContextInspection,
    SessionMutation,
    AgentMutation,
    ConfigMutation,
    ApprovalDecision,
    WorkspaceInspection,
    WorkspaceMutation,
    ProviderProbe,
    QueueMutation,
    Interrupt,
    Exit,
}

impl SlashCommandEffect {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::ContextInspection => "context-inspection",
            Self::SessionMutation => "session-mutation",
            Self::AgentMutation => "agent-mutation",
            Self::ConfigMutation => "config-mutation",
            Self::ApprovalDecision => "approval-decision",
            Self::WorkspaceInspection => "workspace-inspection",
            Self::WorkspaceMutation => "workspace-mutation",
            Self::ProviderProbe => "provider-probe",
            Self::QueueMutation => "queue-mutation",
            Self::Interrupt => "interrupt",
            Self::Exit => "exit",
        }
    }
}

impl SlashCommandDescriptor {
    pub(super) fn argument_model(self) -> SlashCommandArgumentModel {
        match self.name {
            "/help" | "/agents" | "/status" | "/sessions" | "/new" | "/context" | "/memory"
            | "/rag" | "/tools" | "/model" | "/diff" | "/multi" | "/clear" | "/quit" | "/exit" => {
                SlashCommandArgumentModel::None
            }
            "/commands" | "/history" | "/resume" | "/session" | "/timeline" | "/replay"
            | "/debug" | "/trace" | "/mentions" | "/review" | "/sandbox" => {
                SlashCommandArgumentModel::Optional
            }
            "/agent" | "/rollback" => SlashCommandArgumentModel::Required,
            "/queue" | "/attach" | "/budget" | "/screen" | "/provider" | "/approval"
            | "/approvals" | "/cancel" | "/code" | "/mcp" => SlashCommandArgumentModel::Subcommand,
            _ if self.usage.contains('<') => SlashCommandArgumentModel::Required,
            _ if self.usage.contains('[') => SlashCommandArgumentModel::Optional,
            _ => SlashCommandArgumentModel::None,
        }
    }

    pub(super) fn effect(self) -> SlashCommandEffect {
        match self.name {
            "/queue" | "/attach" => SlashCommandEffect::QueueMutation,
            "/agent" => SlashCommandEffect::AgentMutation,
            "/budget" => SlashCommandEffect::ConfigMutation,
            "/context" | "/memory" | "/session" => SlashCommandEffect::ContextInspection,
            "/resume" | "/new" | "/fork" | "/screen" => SlashCommandEffect::SessionMutation,
            "/approval" | "/approvals" => SlashCommandEffect::ApprovalDecision,
            "/cancel" => SlashCommandEffect::Interrupt,
            "/mentions" | "/diff" => SlashCommandEffect::WorkspaceInspection,
            "/provider" => SlashCommandEffect::ProviderProbe,
            "/mcp" => SlashCommandEffect::ProviderProbe,
            "/code" | "/rollback" => SlashCommandEffect::WorkspaceMutation,
            "/quit" | "/exit" => SlashCommandEffect::Exit,
            _ => SlashCommandEffect::ReadOnly,
        }
    }

    pub(super) fn outputs(self) -> &'static [&'static str] {
        match self.name {
            "/commands" => &[
                "human",
                "commands_json",
                "commands_markdown",
                "commands_palette_json",
            ],
            "/queue" => &["human", "pending_inputs_json"],
            "/attach" => &["human", "attachments_pending"],
            "/screen" => &["human", "screen_json", "screen_selected_actions_json"],
            "/status" => &["human", "workbench_status_json"],
            "/debug" => &[
                "human",
                "readiness_json",
                "sandbox_json",
                "logs_json",
                "insights_json",
                "dump_json",
                "state_db_json",
                "continuations_json",
            ],
            "/trace" => &["human", "trace_json"],
            "/sandbox" => &["human", "sandbox_json"],
            "/context" => &["human", "context_status_json"],
            "/memory" => &["human", "memory_status_json"],
            "/rag" => &["human", "rag_status_json"],
            "/tools" => &["human", "tools_status_json"],
            "/mcp" => &[
                "human",
                "mcp_status_json",
                "mcp_stdio_call_json",
                "mcp_http_call_json",
            ],
            "/approval" | "/approvals" => &["human", "approval_overlay_json"],
            "/diff" => &["human", "diff_status_json"],
            _ => &["human"],
        }
    }
}
