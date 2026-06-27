// SPDX-License-Identifier: GPL-3.0-only

use super::terminal_inline;
use std::collections::BTreeMap;

const SLASH_COMMAND_REGISTRY_SCHEMA: &str = "ikaros-workbench-commands-v1";
const SLASH_COMMAND_REGISTRY_VERSION: u32 = 3;
const SLASH_COMMAND_REGISTRY_SOURCE: &str = "builtin";

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
    fn as_str(self) -> &'static str {
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
    Gateway,
    Acp,
}

impl SlashCommandSurface {
    fn as_str(self) -> &'static str {
        match self {
            Self::Workbench => "workbench",
            Self::Gateway => "gateway",
            Self::Acp => "acp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlashCommandArgumentModel {
    None,
    Optional,
    Required,
    Subcommand,
}

impl SlashCommandArgumentModel {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Optional => "optional",
            Self::Required => "required",
            Self::Subcommand => "subcommand",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlashCommandEffect {
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
    fn as_str(self) -> &'static str {
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
    fn argument_model(self) -> SlashCommandArgumentModel {
        match self.name {
            "/help" | "/agents" | "/status" | "/sessions" | "/new" | "/context" | "/memory"
            | "/rag" | "/tools" | "/model" | "/tasks" | "/diff" | "/multi" | "/clear" | "/quit"
            | "/exit" => SlashCommandArgumentModel::None,
            "/commands" | "/history" | "/resume" | "/session" | "/timeline" | "/replay"
            | "/debug" | "/trace" | "/mentions" | "/review" | "/sandbox" => {
                SlashCommandArgumentModel::Optional
            }
            "/agent" | "/rollback" => SlashCommandArgumentModel::Required,
            "/queue" | "/attach" | "/budget" | "/screen" | "/provider" | "/approval"
            | "/approvals" | "/cancel" | "/code" | "/mcp" | "/api" | "/browser" | "/web"
            | "/vision" | "/image" | "/gateway" => SlashCommandArgumentModel::Subcommand,
            _ if self.usage.contains('<') => SlashCommandArgumentModel::Required,
            _ if self.usage.contains('[') => SlashCommandArgumentModel::Optional,
            _ => SlashCommandArgumentModel::None,
        }
    }

    fn effect(self) -> SlashCommandEffect {
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
            "/mcp" | "/browser" | "/web" | "/vision" | "/image" => {
                SlashCommandEffect::ProviderProbe
            }
            "/code" | "/rollback" => SlashCommandEffect::WorkspaceMutation,
            "/quit" | "/exit" => SlashCommandEffect::Exit,
            _ => SlashCommandEffect::ReadOnly,
        }
    }

    fn outputs(self) -> &'static [&'static str] {
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
            "/api" => &["human", "api_status_json"],
            "/browser" => &["human", "browser_json"],
            "/web" => &["human", "web_result", "web_json"],
            "/vision" => &["human", "vision_model", "vision_content", "vision_usage"],
            "/image" => &[
                "human",
                "image_model",
                "image_count",
                "image_item",
                "image_json",
            ],
            "/approval" | "/approvals" => &["human", "approval_overlay_json"],
            "/diff" => &["human", "diff_status_json"],
            _ => &["human"],
        }
    }
}

const READ: &[SlashCommandPermission] = &[SlashCommandPermission::Read];
const SESSION_CONTROL: &[SlashCommandPermission] = &[SlashCommandPermission::SessionControl];
const AGENT_CONTROL: &[SlashCommandPermission] = &[SlashCommandPermission::AgentControl];
const PROVIDER_READ: &[SlashCommandPermission] = &[
    SlashCommandPermission::Provider,
    SlashCommandPermission::Network,
];
const MCP: &[SlashCommandPermission] = &[
    SlashCommandPermission::Read,
    SlashCommandPermission::Network,
    SlashCommandPermission::Approval,
];
const CONFIG_WRITE: &[SlashCommandPermission] = &[SlashCommandPermission::Config];
const APPROVAL: &[SlashCommandPermission] = &[SlashCommandPermission::Approval];
const WORKSPACE_READ: &[SlashCommandPermission] = &[SlashCommandPermission::Read];
const CODING_WRITE: &[SlashCommandPermission] = &[
    SlashCommandPermission::Coding,
    SlashCommandPermission::WorkspaceWrite,
    SlashCommandPermission::Shell,
];
const CODING_READ: &[SlashCommandPermission] =
    &[SlashCommandPermission::Read, SlashCommandPermission::Coding];
const WORKBENCH_ONLY: &[SlashCommandSurface] = &[SlashCommandSurface::Workbench];
const WORKBENCH_GATEWAY: &[SlashCommandSurface] =
    &[SlashCommandSurface::Workbench, SlashCommandSurface::Gateway];
const WORKBENCH_GATEWAY_ACP: &[SlashCommandSurface] = &[
    SlashCommandSurface::Workbench,
    SlashCommandSurface::Gateway,
    SlashCommandSurface::Acp,
];

const SLASH_COMMANDS: &[SlashCommandDescriptor] = &[
    SlashCommandDescriptor {
        name: "/help",
        usage: "/help",
        summary: "show workbench commands",
        tags: &["commands", "help"],
        permissions: READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/commands",
        usage: "/commands [query] [--json|--markdown|--palette]",
        summary: "search slash command registry and export machine-readable metadata or command-palette groups",
        tags: &[
            "commands", "search", "fuzzy", "registry", "metadata", "palette",
        ],
        permissions: READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/queue",
        usage: "/queue [message|run|drain|continue|clear|remove N|retry ID|requeue ID]",
        summary: "inspect, run, enqueue, remove, clear, or requeue pending work",
        tags: &["input", "pending", "queue", "resume"],
        permissions: SESSION_CONTROL,
        surfaces: WORKBENCH_GATEWAY,
    },
    SlashCommandDescriptor {
        name: "/attach",
        usage: "/attach image <url-or-path> [--detail low|high|auto] | <audio|file> <url-or-path> | list | remove <index> | clear",
        summary: "queue multimodal content blocks for the next chat turn",
        tags: &[
            "input",
            "attachment",
            "image",
            "detail",
            "audio",
            "file",
            "multimodal",
        ],
        permissions: SESSION_CONTROL,
        surfaces: WORKBENCH_ONLY,
    },
    SlashCommandDescriptor {
        name: "/agents",
        usage: "/agents",
        summary: "list configured agent profiles and instances",
        tags: &["agent", "profile", "instance"],
        permissions: READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/agent",
        usage: "/agent <profile-or-instance>",
        summary: "switch active agent profile or instance",
        tags: &["agent", "profile", "instance"],
        permissions: AGENT_CONTROL,
        surfaces: WORKBENCH_ONLY,
    },
    SlashCommandDescriptor {
        name: "/status",
        usage: "/status",
        summary: "show unified workbench status",
        tags: &["session", "provider", "gateway", "approval"],
        permissions: READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/budget",
        usage: "/budget [show|set <tokens>|disable]",
        summary: "inspect or update model.default.daily_token_budget",
        tags: &["model", "budget", "config", "tokens"],
        permissions: CONFIG_WRITE,
        surfaces: WORKBENCH_ONLY,
    },
    SlashCommandDescriptor {
        name: "/screen",
        usage: "/screen [--focus status|timeline|main|side] [--scroll N] [--select N] [--select-title title] [--select-kind kind] [--select-action command] [--palette [query]|--palette-query query|--close-palette] [--down|--up|--page-down|--page-up|--top] [--fullscreen|--inline] [--raw|--rich] [approve-selected|deny-selected|cancel-selected|clear-selected|open-selected|confirm-selected]",
        summary: "refresh a navigable terminal workbench screen and open or act on the selected cell",
        tags: &[
            "terminal",
            "screen",
            "status",
            "timeline",
            "approval",
            "continuation",
            "queue",
            "open",
            "focus",
            "scroll",
            "select",
            "palette",
        ],
        permissions: READ,
        surfaces: WORKBENCH_ONLY,
    },
    SlashCommandDescriptor {
        name: "/history",
        usage: "/history [limit]",
        summary: "show persisted workbench input history",
        tags: &["terminal", "history", "readline"],
        permissions: READ,
        surfaces: WORKBENCH_ONLY,
    },
    SlashCommandDescriptor {
        name: "/sessions",
        usage: "/sessions",
        summary: "list recent chat sessions",
        tags: &["session", "history"],
        permissions: READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/session",
        usage: "/session status|resume|history|timeline|export [path]",
        summary: "inspect current session state, history, timeline, or explicit resume/export actions",
        tags: &["session", "resume", "timeline", "export"],
        permissions: SESSION_CONTROL,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/resume",
        usage: "/resume <session>",
        summary: "resume a session id",
        tags: &["session", "resume"],
        permissions: SESSION_CONTROL,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/new",
        usage: "/new",
        summary: "alias for /clear",
        tags: &["session", "alias"],
        permissions: SESSION_CONTROL,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/fork",
        usage: "/fork [summary]",
        summary: "branch the current session from its active leaf",
        tags: &["session", "branch", "tree"],
        permissions: SESSION_CONTROL,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/timeline",
        usage: "/timeline [turn] [--page N] [--kind KIND] [--failed|--approval]",
        summary: "show recent session timeline cells, optionally filtered by event kind or replay point",
        tags: &[
            "session", "replay", "debug", "kind", "filter", "failed", "approval",
        ],
        permissions: READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/replay",
        usage: "/replay [turn] [--page N] [--kind KIND] [--failed|--approval]",
        summary: "show a longer session replay view, optionally filtered by event kind or replay point",
        tags: &["session", "replay", "kind", "filter", "failed", "approval"],
        permissions: READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/debug",
        usage: "/debug [turn|readiness|sandbox [--probe]|logs|insights|dump|state-db|continuations] [--page N] [--kind KIND] [--failed|--approval]",
        summary: "show session debug cells, readiness, sandbox, logs, insights, dump, state-db, or continuation diagnostics",
        tags: &[
            "session",
            "debug",
            "readiness",
            "sandbox",
            "probe",
            "logs",
            "insights",
            "dump",
            "state-db",
            "continuations",
            "kind",
            "filter",
            "failed",
            "approval",
        ],
        permissions: READ,
        surfaces: WORKBENCH_ONLY,
    },
    SlashCommandDescriptor {
        name: "/sandbox",
        usage: "/sandbox [--probe]",
        summary: "show sandbox, process, environment, and network execution diagnostics",
        tags: &[
            "sandbox",
            "execution",
            "network",
            "process",
            "debug",
            "probe",
        ],
        permissions: READ,
        surfaces: WORKBENCH_ONLY,
    },
    SlashCommandDescriptor {
        name: "/trace",
        usage: "/trace [turn] [--kind KIND] [--failed|--approval]",
        summary: "show sanitized session trace spans, optionally filtered by event kind or replay point",
        tags: &[
            "session", "debug", "trace", "replay", "kind", "filter", "failed", "approval",
        ],
        permissions: READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/mentions",
        usage: "/mentions [query]",
        summary: "search file, folder, git, diff, and staged context mentions",
        tags: &["context", "file", "folder", "mention", "reference"],
        permissions: WORKSPACE_READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/context",
        usage: "/context",
        summary: "inspect context budget and prompt assembly state",
        tags: &["context"],
        permissions: READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/memory",
        usage: "/memory",
        summary: "inspect memory policy, projection, and working-memory state",
        tags: &["memory"],
        permissions: READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/rag",
        usage: "/rag",
        summary: "show RAG settings",
        tags: &["rag", "context"],
        permissions: READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/tools",
        usage: "/tools",
        summary: "show direct and deferred model toolsets",
        tags: &["tool", "toolset", "skill"],
        permissions: READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/mcp",
        usage: "/mcp status|call-stdio <command> <tool>|call-http <url> <tool>",
        summary: "show configured MCP servers or call stdio/HTTP MCP tools through harness boundaries",
        tags: &["mcp", "tool", "server", "status", "plugin", "stdio", "http"],
        permissions: MCP,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/api",
        usage: "/api status",
        summary: "show local OpenAI-compatible API routes and readiness",
        tags: &["api", "openai", "responses", "embedding", "status"],
        permissions: READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/browser",
        usage: "/browser [launch [url] [--profile NAME]|supervisor-status [--profile NAME]|stop [--profile NAME]|status|list|new <url>|activate <target-id>|close <target-id>|navigate <target-id> <url>|snapshot <target-id>|click <target-id> <x> <y>|type <target-id> <text>|scroll <target-id> [x] [y]|screenshot <target-id>|cdp <target-id> <method> [params-json]] [--endpoint URL]",
        summary: "launch or control a local Chrome DevTools endpoint with an isolated profile",
        tags: &[
            "browser",
            "cdp",
            "network",
            "debug",
            "supervisor",
            "profile",
        ],
        permissions: MCP,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/web",
        usage: "/web search <query> [--provider duckduckgo-html|brave|bing|serpapi|tavily] [--max-results N] | /web extract <url> [--max-bytes N] [--max-chars N]",
        summary: "run governed web search or single-page extraction from workbench",
        tags: &["web", "search", "extract", "network"],
        permissions: MCP,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/vision",
        usage: "/vision describe <image-path|url|data-url> [--prompt TEXT] [--detail low|high|auto]",
        summary: "describe an image through the active multimodal model",
        tags: &["vision", "image", "multimodal", "model"],
        permissions: PROVIDER_READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/image",
        usage: "/image generate <prompt> [--model MODEL] [--size 1024x1024] [--n N] [--response-format url|b64_json] [--output-dir PATH]",
        summary: "generate images through the active OpenAI-compatible provider endpoint",
        tags: &["image", "generation", "multimodal", "model"],
        permissions: PROVIDER_READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/model",
        usage: "/model",
        summary: "inspect active model descriptor",
        tags: &["provider", "model"],
        permissions: READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/provider",
        usage: "/provider [inspect|health [--live]|matrix [--live] [--json]|profiles|debug]",
        summary: "inspect provider metadata, health, matrix, or debug JSON",
        tags: &["provider", "model", "health", "matrix", "json", "debug"],
        permissions: PROVIDER_READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/gateway",
        usage: "/gateway [status|daemon status|daemon start|daemon stop|daemon restart|adapter list|adapter enqueue|adapter render-delivery]",
        summary: "show local gateway status, control the message daemon, or inspect platform adapters",
        tags: &["gateway", "ingress", "daemon", "adapter", "pairing"],
        permissions: READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/tasks",
        usage: "/tasks",
        summary: "show scheduled task status",
        tags: &["schedule", "task"],
        permissions: READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/approval",
        usage: "/approval [approve|deny <id>]",
        summary: "show or resolve pending approvals",
        tags: &["approval", "policy"],
        permissions: APPROVAL,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/approvals",
        usage: "/approvals [approve|deny <id>]",
        summary: "alias for /approval",
        tags: &["approval", "policy", "alias"],
        permissions: APPROVAL,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/cancel",
        usage: "/cancel [all|<continuation-id>]",
        summary: "cancel queued or running workbench continuations",
        tags: &["interrupt", "queue", "continuation", "cancel"],
        permissions: SESSION_CONTROL,
        surfaces: WORKBENCH_ONLY,
    },
    SlashCommandDescriptor {
        name: "/diff",
        usage: "/diff",
        summary: "show current git diff summary",
        tags: &["coding", "diff"],
        permissions: WORKSPACE_READ,
        surfaces: WORKBENCH_GATEWAY_ACP,
    },
    SlashCommandDescriptor {
        name: "/code",
        usage: "/code <plan|apply|test|review|rollback> ...",
        summary: "run coding workflow commands",
        tags: &["coding", "patch", "test", "review"],
        permissions: CODING_WRITE,
        surfaces: WORKBENCH_ONLY,
    },
    SlashCommandDescriptor {
        name: "/review",
        usage: "/review [--diff DIFF] [--test-analysis-json JSON]",
        summary: "alias for /code review",
        tags: &["coding", "review", "alias"],
        permissions: CODING_READ,
        surfaces: WORKBENCH_ONLY,
    },
    SlashCommandDescriptor {
        name: "/rollback",
        usage: "/rollback <session-id> --turn-id <turn-id>",
        summary: "alias for /code rollback",
        tags: &["coding", "rollback", "alias"],
        permissions: CODING_WRITE,
        surfaces: WORKBENCH_ONLY,
    },
    SlashCommandDescriptor {
        name: "/multi",
        usage: "/multi",
        summary: "enter multiline message mode",
        tags: &["input", "multiline"],
        permissions: READ,
        surfaces: WORKBENCH_ONLY,
    },
    SlashCommandDescriptor {
        name: "/clear",
        usage: "/clear",
        summary: "acknowledge a clear-screen request",
        tags: &["display"],
        permissions: READ,
        surfaces: WORKBENCH_ONLY,
    },
    SlashCommandDescriptor {
        name: "/quit",
        usage: "/quit",
        summary: "exit the workbench",
        tags: &["exit"],
        permissions: READ,
        surfaces: WORKBENCH_ONLY,
    },
    SlashCommandDescriptor {
        name: "/exit",
        usage: "/exit",
        summary: "alias for /quit",
        tags: &["exit", "alias"],
        permissions: READ,
        surfaces: WORKBENCH_ONLY,
    },
];

pub(super) fn slash_commands() -> &'static [SlashCommandDescriptor] {
    SLASH_COMMANDS
}

pub fn slash_command_completion_candidates(
    input: &str,
    limit: usize,
) -> Vec<SlashCommandCompletion> {
    let query = slash_completion_query(input);
    if query.is_empty() {
        return Vec::new();
    }
    let mut prefix_matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| {
            command.name.starts_with(query)
                && slash_command_visible_in_picker(*command, Some(query))
        })
        .collect::<Vec<_>>();
    prefix_matches.sort_by_key(|command| command.name);

    let mut fuzzy_matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| {
            !command.name.starts_with(query)
                && slash_command_visible_in_picker(*command, Some(query))
                && slash_command_matches(*command, query)
        })
        .collect::<Vec<_>>();
    fuzzy_matches.sort_by_key(|command| command.name);

    prefix_matches
        .into_iter()
        .chain(fuzzy_matches)
        .take(limit.max(1))
        .map(|command| SlashCommandCompletion {
            name: command.name,
            usage: command.usage,
            summary: command.summary,
            argument_model: command.argument_model().as_str(),
            effect: command.effect().as_str(),
        })
        .collect()
}

pub fn slash_completion_query(input: &str) -> &str {
    let input = input.trim_start();
    if !input.starts_with('/') {
        return "";
    }
    input.split_whitespace().next().unwrap_or(input)
}

pub fn slash_command_registry_summary() -> String {
    format!(
        "commands={} surfaces={} permissions={} command=/commands json=/commands --json markdown=/commands --markdown palette=/commands --palette help=/help",
        slash_commands().len(),
        command_metadata_list(slash_registry_surfaces().iter().copied()),
        command_metadata_list(slash_registry_permissions().iter().copied()),
    )
}

pub fn slash_command_palette_summary(query: Option<&str>) -> SlashCommandPaletteSummary {
    let query = query.map(str::trim).filter(|query| !query.is_empty());
    let matches = slash_command_palette_matches(query);
    let mut effects = matches
        .iter()
        .map(|command| command.effect().as_str())
        .collect::<Vec<_>>();
    effects.sort();
    effects.dedup();
    let mut permissions = matches
        .iter()
        .flat_map(|command| {
            command
                .permissions
                .iter()
                .map(|permission| permission.as_str())
        })
        .collect::<Vec<_>>();
    permissions.sort();
    permissions.dedup();
    let mut surfaces = matches
        .iter()
        .flat_map(|command| command.surfaces.iter().map(|surface| surface.as_str()))
        .collect::<Vec<_>>();
    surfaces.sort();
    surfaces.dedup();
    SlashCommandPaletteSummary {
        query: query.unwrap_or("all").to_owned(),
        command_count: matches.len(),
        total_commands: slash_commands()
            .iter()
            .copied()
            .filter(|command| slash_command_visible_in_picker(*command, None))
            .count(),
        effects: command_metadata_list(effects.into_iter()),
        permissions: command_metadata_list(permissions.into_iter()),
        surfaces: command_metadata_list(surfaces.into_iter()),
    }
}

pub fn slash_command_palette_items(
    query: Option<&str>,
    limit: usize,
) -> Vec<SlashCommandPaletteItem> {
    slash_command_palette_matches(query)
        .into_iter()
        .take(limit.max(1))
        .map(|command| SlashCommandPaletteItem {
            name: command.name,
            usage: command.usage,
            summary: command.summary,
            argument_model: command.argument_model().as_str(),
            effect: command.effect().as_str(),
            permissions: command_metadata_list(
                command
                    .permissions
                    .iter()
                    .map(|permission| permission.as_str()),
            ),
            surfaces: command_metadata_list(
                command.surfaces.iter().map(|surface| surface.as_str()),
            ),
            tags: command_metadata_list(command.tags.iter().copied()),
        })
        .collect()
}

pub fn format_slash_command_help() -> String {
    let usages = slash_commands()
        .iter()
        .map(|command| command.usage)
        .collect::<Vec<_>>()
        .join(", ");
    format!("commands: {usages}")
}

pub fn print_slash_commands(args: &[&str]) {
    let options = SlashCommandQueryOptions::parse(args);
    let query = options.query.as_deref();
    if options.palette {
        println!("{}", slash_commands_palette_json_line(query));
        return;
    }
    if options.markdown {
        println!("{}", slash_command_registry_markdown(query));
        return;
    }
    if options.json_only {
        println!("{}", slash_commands_json_line(query));
        return;
    }
    if let Some(query) = query {
        println!("commands_query: {}", terminal_inline(query));
    } else {
        println!("commands_query: all");
    }
    println!("{}", slash_command_registry_line());
    let mut matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| query.is_none_or(|query| slash_command_matches(*command, query)))
        .collect::<Vec<_>>();
    matches.sort_by_key(|command| command.name);
    println!("commands_found: {}", matches.len());
    for command in matches {
        println!(
            "- {} usage={} args={} effect={} outputs={} permissions={} surfaces={} summary={}",
            terminal_inline(command.name),
            terminal_inline(command.usage),
            command.argument_model().as_str(),
            command.effect().as_str(),
            command_metadata_list(command.outputs().iter().copied()),
            command_metadata_list(
                command
                    .permissions
                    .iter()
                    .map(|permission| permission.as_str())
            ),
            command_metadata_list(command.surfaces.iter().map(|surface| surface.as_str())),
            terminal_inline(command.summary)
        );
    }
    println!("{}", slash_commands_json_line(query));
}

pub fn slash_commands_human_lines(args: &[&str]) -> Vec<String> {
    let options = SlashCommandQueryOptions::parse(args);
    let query = options.query.as_deref();
    let mut lines = vec!["• Slash commands".to_owned()];
    if let Some(query) = query {
        lines.push(format!("  query: {}", terminal_inline(query)));
    }
    let mut matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| query.is_none_or(|query| slash_command_matches(*command, query)))
        .collect::<Vec<_>>();
    matches.sort_by_key(|command| command.name);
    for command in matches.into_iter().take(12) {
        lines.push(format!(
            "  {:<12} {}",
            terminal_inline(command.name),
            terminal_inline(command.summary)
        ));
    }
    lines.push("  open picker: F5".to_owned());
    lines
}

pub fn print_slash_commands_for_human(args: &[&str]) {
    for line in slash_commands_human_lines(args) {
        println!("{line}");
    }
}

pub fn suggest_slash_command(input: &str) -> Option<&'static str> {
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
    let query = query.trim().to_ascii_lowercase();
    let mut terms = query.split_whitespace();
    if let (Some(first), Some(second)) = (terms.next(), terms.next()) {
        return slash_command_matches(command, first)
            && slash_command_matches(command, second)
            && terms.all(|term| slash_command_matches(command, term));
    }
    let name = command.name.to_ascii_lowercase();
    let usage = command.usage.to_ascii_lowercase();
    let summary = command.summary.to_ascii_lowercase();
    name.contains(&query)
        || usage.contains(&query)
        || summary.contains(&query)
        || command.tags.iter().any(|tag| tag.contains(&query))
        || command
            .permissions
            .iter()
            .any(|permission| permission.as_str().contains(&query))
        || command
            .surfaces
            .iter()
            .any(|surface| surface.as_str().contains(&query))
        || command.argument_model().as_str().contains(&query)
        || command.effect().as_str().contains(&query)
        || command
            .outputs()
            .iter()
            .any(|output| output.contains(&query))
        || fuzzy_subsequence(&name, &query)
}

fn command_metadata_list<'a>(values: impl Iterator<Item = &'a str>) -> String {
    values.map(terminal_inline).collect::<Vec<_>>().join(",")
}

fn slash_command_registry_line() -> String {
    format!(
        "commands_registry: schema={} version={} source={} commands={} surfaces={} permissions={}",
        SLASH_COMMAND_REGISTRY_SCHEMA,
        SLASH_COMMAND_REGISTRY_VERSION,
        SLASH_COMMAND_REGISTRY_SOURCE,
        slash_commands().len(),
        slash_registry_surfaces().join(","),
        slash_registry_permissions().join(",")
    )
}

fn slash_commands_json_line(query: Option<&str>) -> String {
    let query = query.map(str::trim).filter(|query| !query.is_empty());
    let matches = slash_command_query_matches(query);
    let commands = matches
        .iter()
        .map(|command| {
            serde_json::json!({
                "name": command.name,
                "usage": command.usage,
                "summary": command.summary,
                "tags": command.tags,
                "argument_model": command.argument_model().as_str(),
                "effect": command.effect().as_str(),
                "outputs": command.outputs(),
                "permissions": command.permissions.iter().map(|permission| permission.as_str()).collect::<Vec<_>>(),
                "surfaces": command.surfaces.iter().map(|surface| surface.as_str()).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    format!(
        "commands_json: {}",
        serde_json::json!({
            "schema": SLASH_COMMAND_REGISTRY_SCHEMA,
            "registry_version": SLASH_COMMAND_REGISTRY_VERSION,
            "source": SLASH_COMMAND_REGISTRY_SOURCE,
            "query": query.unwrap_or("all"),
            "command_count": commands.len(),
            "total_commands": slash_commands().len(),
            "surfaces": slash_registry_surfaces(),
            "permissions": slash_registry_permissions(),
            "commands": commands,
        })
    )
}

fn slash_commands_palette_json_line(query: Option<&str>) -> String {
    let query = query.map(str::trim).filter(|query| !query.is_empty());
    let matches = slash_command_palette_matches(query);

    let mut by_effect: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
    let mut by_permission: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
    let mut by_surface: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
    for command in &matches {
        by_effect
            .entry(command.effect().as_str())
            .or_default()
            .push(command.name);
        for permission in command.permissions {
            by_permission
                .entry(permission.as_str())
                .or_default()
                .push(command.name);
        }
        for surface in command.surfaces {
            by_surface
                .entry(surface.as_str())
                .or_default()
                .push(command.name);
        }
    }

    let items = matches
        .iter()
        .map(|command| {
            serde_json::json!({
                "name": command.name,
                "usage": command.usage,
                "summary": command.summary,
                "argument_model": command.argument_model().as_str(),
                "effect": command.effect().as_str(),
                "permissions": command.permissions.iter().map(|permission| permission.as_str()).collect::<Vec<_>>(),
                "surfaces": command.surfaces.iter().map(|surface| surface.as_str()).collect::<Vec<_>>(),
                "tags": command.tags,
                "action": command.name,
                "detail_action": format!("/commands {}", command.name),
            })
        })
        .collect::<Vec<_>>();

    format!(
        "commands_palette_json: {}",
        serde_json::json!({
            "schema": "ikaros-workbench-command-palette-v1",
            "version": 1,
            "query": query.unwrap_or("all"),
            "command_count": items.len(),
            "total_commands": slash_commands().len(),
            "groups": {
                "effect": by_effect,
                "permission": by_permission,
                "surface": by_surface,
            },
            "items": items,
        })
    )
}

fn slash_command_query_matches(query: Option<&str>) -> Vec<SlashCommandDescriptor> {
    let query = query.map(str::trim).filter(|query| !query.is_empty());
    let mut matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| query.is_none_or(|query| slash_command_matches(*command, query)))
        .collect::<Vec<_>>();
    matches.sort_by_key(|command| command.name);
    matches
}

fn slash_command_palette_matches(query: Option<&str>) -> Vec<SlashCommandDescriptor> {
    let query = query.map(str::trim).filter(|query| !query.is_empty());
    let mut matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| {
            slash_command_visible_in_picker(*command, query)
                && query.is_none_or(|query| slash_command_matches(*command, query))
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|command| {
        (
            command.effect().as_str(),
            command.argument_model().as_str(),
            command.name,
        )
    });
    matches
}

fn slash_command_visible_in_picker(command: SlashCommandDescriptor, query: Option<&str>) -> bool {
    if !slash_command_is_alias(command) {
        return true;
    }
    let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) else {
        return false;
    };
    let query = query.trim_start_matches('/');
    if query.len() < 2 {
        return false;
    }
    command.name.trim_start_matches('/').starts_with(query)
}

fn slash_command_is_alias(command: SlashCommandDescriptor) -> bool {
    command.tags.contains(&"alias")
}

fn slash_command_registry_markdown(query: Option<&str>) -> String {
    let mut matches = slash_commands()
        .iter()
        .copied()
        .filter(|command| query.is_none_or(|query| slash_command_matches(*command, query)))
        .collect::<Vec<_>>();
    matches.sort_by_key(|command| command.name);
    let mut output = String::new();
    output.push_str("commands_markdown:\n");
    output.push_str("# Ikaros Slash Commands\n\n");
    output.push_str(&format!(
        "- schema: `{}`\n- version: `{}`\n- source: `{}`\n- query: `{}`\n- commands: `{}`\n\n",
        SLASH_COMMAND_REGISTRY_SCHEMA,
        SLASH_COMMAND_REGISTRY_VERSION,
        SLASH_COMMAND_REGISTRY_SOURCE,
        query.unwrap_or("all"),
        matches.len()
    ));
    output.push_str("| Command | Usage | Effect | Permissions | Surfaces | Summary |\n");
    output.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for command in matches {
        output.push_str(&format!(
            "| `{}` | `{}` | `{}` | `{}` | `{}` | {} |\n",
            terminal_inline(command.name),
            terminal_inline(command.usage),
            command.effect().as_str(),
            command_metadata_list(
                command
                    .permissions
                    .iter()
                    .map(|permission| permission.as_str())
            ),
            command_metadata_list(command.surfaces.iter().map(|surface| surface.as_str())),
            terminal_inline(command.summary)
        ));
    }
    output
}

fn slash_registry_surfaces() -> Vec<&'static str> {
    let mut surfaces = slash_commands()
        .iter()
        .flat_map(|command| command.surfaces.iter().copied())
        .map(SlashCommandSurface::as_str)
        .collect::<Vec<_>>();
    surfaces.sort();
    surfaces.dedup();
    surfaces
}

fn slash_registry_permissions() -> Vec<&'static str> {
    let mut permissions = slash_commands()
        .iter()
        .flat_map(|command| command.permissions.iter().copied())
        .map(SlashCommandPermission::as_str)
        .collect::<Vec<_>>();
    permissions.sort();
    permissions.dedup();
    permissions
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SlashCommandQueryOptions {
    query: Option<String>,
    json_only: bool,
    markdown: bool,
    palette: bool,
}

impl SlashCommandQueryOptions {
    fn parse(args: &[&str]) -> Self {
        let mut query = Vec::new();
        let mut json_only = false;
        let mut markdown = false;
        let mut palette = false;
        for arg in args {
            match *arg {
                "--json" | "json" => json_only = true,
                "--markdown" | "--md" | "markdown" => markdown = true,
                "--palette" | "palette" => palette = true,
                value => query.push(value),
            }
        }
        Self {
            query: (!query.is_empty()).then(|| query.join(" ")),
            json_only,
            markdown,
            palette,
        }
    }
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
    fn command_search_matches_all_multiword_terms() {
        let matches = slash_commands()
            .iter()
            .copied()
            .filter(|command| slash_command_matches(*command, "screen palette"))
            .map(|command| command.name)
            .collect::<Vec<_>>();

        assert!(matches.contains(&"/screen"));
        assert!(!matches.contains(&"/sessions"));
    }

    #[test]
    fn command_suggestion_finds_near_miss() {
        assert_eq!(suggest_slash_command("/sesions"), Some("/sessions"));
    }

    #[test]
    fn command_metadata_json_line_exports_registry_fields() {
        let line = slash_commands_json_line(Some("provider"));
        let payload = line
            .strip_prefix("commands_json: ")
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
            .expect("commands JSON payload");
        let commands = payload["commands"].as_array().expect("commands array");
        let provider = commands
            .iter()
            .find(|command| command["name"] == "/provider")
            .expect("provider command metadata");

        assert_eq!(payload["query"], "provider");
        assert_eq!(
            provider["usage"],
            "/provider [inspect|health [--live]|matrix [--live] [--json]|profiles|debug]"
        );
        assert_eq!(
            provider["summary"],
            "inspect provider metadata, health, matrix, or debug JSON"
        );
        assert_eq!(
            provider["permissions"],
            serde_json::json!(["provider", "network"])
        );
        assert_eq!(
            provider["surfaces"],
            serde_json::json!(["workbench", "gateway", "acp"])
        );
        assert!(
            provider["tags"]
                .as_array()
                .expect("tags")
                .contains(&serde_json::json!("health"))
        );
    }

    #[test]
    fn sandbox_command_metadata_is_discoverable() {
        let line = slash_commands_json_line(Some("sandbox"));
        let payload = line
            .strip_prefix("commands_json: ")
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
            .expect("commands JSON payload");
        let commands = payload["commands"].as_array().expect("commands array");
        let sandbox = commands
            .iter()
            .find(|command| command["name"] == "/sandbox")
            .expect("sandbox command metadata");

        assert_eq!(sandbox["usage"], "/sandbox [--probe]");
        assert_eq!(sandbox["argument_model"], "optional");
        assert_eq!(
            sandbox["outputs"],
            serde_json::json!(["human", "sandbox_json"])
        );
        assert_eq!(sandbox["permissions"], serde_json::json!(["read"]));
        assert_eq!(sandbox["surfaces"], serde_json::json!(["workbench"]));
    }

    #[test]
    fn commands_declare_permissions_and_surfaces() {
        for command in slash_commands() {
            assert!(
                !command.permissions.is_empty(),
                "{} must declare permissions",
                command.name
            );
            assert!(
                !command.surfaces.is_empty(),
                "{} must declare supported surfaces",
                command.name
            );
        }

        let code = slash_commands()
            .iter()
            .find(|command| command.name == "/code")
            .expect("code command");
        assert!(
            code.permissions
                .contains(&SlashCommandPermission::WorkspaceWrite)
        );
        assert!(code.permissions.contains(&SlashCommandPermission::Shell));
        assert!(code.surfaces.contains(&SlashCommandSurface::Workbench));

        let approval = slash_commands()
            .iter()
            .find(|command| command.name == "/approval")
            .expect("approval command");
        assert!(
            approval
                .permissions
                .contains(&SlashCommandPermission::Approval)
        );

        let provider = slash_commands()
            .iter()
            .find(|command| command.name == "/provider")
            .expect("provider command");
        assert!(
            provider
                .permissions
                .contains(&SlashCommandPermission::Network)
        );
    }

    #[test]
    fn command_registry_includes_workbench_loop_aliases() {
        let command_names = slash_commands()
            .iter()
            .map(|command| command.name)
            .collect::<Vec<_>>();

        assert!(command_names.contains(&"/approvals"));
        assert!(command_names.contains(&"/exit"));
    }

    #[test]
    fn command_search_matches_permission_and_surface_metadata() {
        let gateway_safe = slash_commands()
            .iter()
            .copied()
            .filter(|command| slash_command_matches(*command, "gateway"))
            .map(|command| command.name)
            .collect::<Vec<_>>();

        assert!(gateway_safe.contains(&"/help"));
        assert!(gateway_safe.contains(&"/commands"));
        assert!(gateway_safe.contains(&"/session"));
        assert!(gateway_safe.contains(&"/provider"));

        let workspace_write = slash_commands()
            .iter()
            .copied()
            .filter(|command| slash_command_matches(*command, "workspace-write"))
            .map(|command| command.name)
            .collect::<Vec<_>>();

        assert!(workspace_write.contains(&"/code"));
    }

    #[test]
    fn cancel_command_declares_interrupt_metadata() {
        let cancel = slash_commands()
            .iter()
            .find(|command| command.name == "/cancel")
            .expect("cancel command");

        assert!(cancel.tags.contains(&"interrupt"));
        assert!(
            cancel
                .permissions
                .contains(&SlashCommandPermission::SessionControl)
        );
        assert!(cancel.surfaces.contains(&SlashCommandSurface::Workbench));
    }
}
