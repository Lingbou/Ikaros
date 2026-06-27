// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    acp::{AcpCommand, acp_command},
    agent::{AgentCommand, agent_command},
    api::{ApiCommand, api_command},
    approval::{ApprovalCommand, approval_command},
    body::{self, BodyCommand},
    browser::{BrowserCommand, browser_command},
    chat::{ChatArgs, chat_command, default_chat_command},
    code::{CodeCommand, code_command},
    config::{ConfigCommand, config_command},
    debug::{DebugCommand, debug_command},
    diagnostics::{DoctorArgs, InitArgs, SetupArgs, doctor, init, setup},
    fs::{FsCommand, fs_command},
    gateway::{MessageCommand, message_command},
    git::{GitCommand, git_command},
    image::{ImageCommand, image_command},
    mcp::{McpCommand, mcp_command},
    memory::{MemoryCommand, memory_command},
    persona::{PersonaCommand, persona_command},
    policy::{PolicyCommand, policy_command},
    provider::{ProviderCommand, provider_command},
    rag::{RagCommand, rag_command},
    relationship::{RelationshipCommand, relationship_command},
    repo::{RepoCommand, repo_command},
    schedule::{ScheduleCommand, schedule_command},
    self_modify::{SelfModifyCommand, self_modify_command},
    service::{ServiceCommand, service_command},
    skill::{SkillCommand, skill_command},
    task::{TaskCommand, task_command},
    testing::{TestCommand, test_command},
    vision::{VisionCommand, vision_command},
    voice::{VoiceCommand, voice_command},
    web::{WebCommand, web_command},
};
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use ikaros_core::{IkarosPaths, StructuredTraceEvent, StructuredTraceLog};
use std::{fs, path::PathBuf, sync::Mutex};

#[derive(Debug, Parser)]
#[command(name = "ikaros", version, about = "Persona-first local agent runtime")]
struct Cli {
    #[arg(long, global = true)]
    ikaros_home: Option<PathBuf>,
    #[arg(value_name = "PATH")]
    root: Option<PathBuf>,
    #[command(flatten)]
    chat: ChatArgs,
    #[arg(long, global = true, value_name = "PROFILE")]
    agent: Option<String>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Init(InitArgs),
    Setup(SetupArgs),
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Debug {
        #[command(subcommand)]
        command: DebugCommand,
    },
    Doctor(DoctorArgs),
    Persona {
        #[command(subcommand)]
        command: PersonaCommand,
    },
    Memory {
        #[command(subcommand)]
        command: MemoryCommand,
    },
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    Relationship {
        #[command(subcommand)]
        command: RelationshipCommand,
    },
    Rag {
        #[command(subcommand)]
        command: RagCommand,
    },
    Voice {
        #[command(subcommand)]
        command: VoiceCommand,
    },
    Vision {
        #[command(subcommand)]
        command: VisionCommand,
    },
    Body {
        #[command(subcommand)]
        command: BodyCommand,
    },
    Browser {
        #[command(subcommand)]
        command: BrowserCommand,
    },
    Web {
        #[command(subcommand)]
        command: WebCommand,
    },
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    Chat(ChatArgs),
    Fs {
        #[command(subcommand)]
        command: FsCommand,
    },
    Approval {
        #[command(subcommand)]
        command: ApprovalCommand,
    },
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    Policy {
        #[command(subcommand)]
        command: PolicyCommand,
    },
    Provider {
        #[command(subcommand)]
        command: ProviderCommand,
    },
    Git {
        #[command(subcommand)]
        command: GitCommand,
    },
    Image {
        #[command(subcommand)]
        command: ImageCommand,
    },
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
    Schedule {
        #[command(subcommand)]
        command: ScheduleCommand,
    },
    Message {
        #[command(subcommand)]
        command: MessageCommand,
    },
    Api {
        #[command(subcommand)]
        command: ApiCommand,
    },
    Acp {
        #[command(subcommand)]
        command: AcpCommand,
    },
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    SelfModify {
        #[command(subcommand)]
        command: SelfModifyCommand,
    },
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
    Test {
        #[command(subcommand)]
        command: TestCommand,
    },
    Code {
        #[command(subcommand)]
        command: CodeCommand,
    },
}

pub(crate) async fn run() -> Result<()> {
    let cli = Cli::parse();
    let paths = match &cli.ikaros_home {
        Some(home) => IkarosPaths::from_home(home),
        None => IkarosPaths::from_env()?,
    };
    init_tracing(&paths)?;
    let workspace = resolve_workspace(&cli)?;
    tracing::info!(
        event = "cli_started",
        home = %paths.home.display(),
        workspace = %workspace.display(),
        command = %cli_command_name(&cli.command),
        "ikaros cli started"
    );
    append_cli_start_trace(&paths, &workspace, cli_command_name(&cli.command));

    match cli.command {
        None => default_chat_command(cli.chat, &paths, &workspace, cli.agent.as_deref()).await?,
        Some(Commands::Init(args)) => init(args, &paths)?,
        Some(Commands::Setup(args)) => setup(args, &paths)?,
        Some(Commands::Config { command }) => config_command(command, &paths)?,
        Some(Commands::Debug { command }) => {
            debug_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Doctor(args)) => doctor(args, &paths, &workspace, cli.agent.as_deref())?,
        Some(Commands::Persona { command }) => persona_command(command, &paths)?,
        Some(Commands::Memory { command }) => {
            memory_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Mcp { command }) => {
            mcp_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Relationship { command }) => {
            relationship_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Rag { command }) => {
            rag_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Voice { command }) => {
            voice_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Vision { command }) => {
            vision_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Body { command }) => body::body_command(command, &paths, &workspace)?,
        Some(Commands::Browser { command }) => {
            browser_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Web { command }) => {
            web_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Task { command }) => {
            task_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Chat(args)) => {
            chat_command(args, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Fs { command }) => {
            fs_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Approval { command }) => {
            approval_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Agent { command }) => {
            agent_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Policy { command }) => {
            policy_command(command, &paths, &workspace, cli.agent.as_deref())?
        }
        Some(Commands::Provider { command }) => {
            provider_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Git { command }) => {
            git_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Image { command }) => {
            image_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Skill { command }) => {
            skill_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Schedule { command }) => {
            schedule_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Message { command }) => {
            message_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Api { command }) => {
            api_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Acp { command }) => {
            acp_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Service { command }) => {
            service_command(command, &paths, &workspace, cli.agent.as_deref())?
        }
        Some(Commands::SelfModify { command }) => {
            self_modify_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Repo { command }) => {
            repo_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Test { command }) => {
            test_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Some(Commands::Code { command }) => {
            code_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
    }
    Ok(())
}

fn requested_workspace_path(cli: &Cli) -> PathBuf {
    cli.root.clone().unwrap_or_else(|| PathBuf::from("."))
}

fn resolve_workspace(cli: &Cli) -> Result<PathBuf> {
    let workspace = requested_workspace_path(cli);
    let exists = workspace
        .try_exists()
        .with_context(|| format!("failed to inspect workspace path: {}", workspace.display()))?;
    if !exists {
        bail!("workspace path does not exist: {}", workspace.display());
    }
    workspace
        .canonicalize()
        .with_context(|| format!("failed to resolve workspace path: {}", workspace.display()))
}

fn append_cli_start_trace(paths: &IkarosPaths, workspace: &std::path::Path, command: &str) {
    let event = match StructuredTraceEvent::new(
        "INFO",
        "ikaros_cli::app",
        "cli_started",
        "ikaros cli started",
        serde_json::json!({
            "home": paths.home.display().to_string(),
            "workspace": workspace.display().to_string(),
            "command": command,
        }),
    ) {
        Ok(event) => event.with_command(command),
        Err(error) => {
            eprintln!("failed to build CLI trace event: {error}");
            return;
        }
    };
    if let Err(error) = StructuredTraceLog::new(&paths.logs_dir).append(event) {
        eprintln!("failed to append CLI trace event: {error}");
    }
}

fn cli_command_name(command: &Option<Commands>) -> &'static str {
    match command {
        None => "chat-default",
        Some(Commands::Init(_)) => "init",
        Some(Commands::Setup(_)) => "setup",
        Some(Commands::Config { .. }) => "config",
        Some(Commands::Debug { .. }) => "debug",
        Some(Commands::Doctor(_)) => "doctor",
        Some(Commands::Persona { .. }) => "persona",
        Some(Commands::Memory { .. }) => "memory",
        Some(Commands::Mcp { .. }) => "mcp",
        Some(Commands::Relationship { .. }) => "relationship",
        Some(Commands::Rag { .. }) => "rag",
        Some(Commands::Voice { .. }) => "voice",
        Some(Commands::Vision { .. }) => "vision",
        Some(Commands::Body { .. }) => "body",
        Some(Commands::Browser { .. }) => "browser",
        Some(Commands::Web { .. }) => "web",
        Some(Commands::Task { .. }) => "task",
        Some(Commands::Chat(_)) => "chat",
        Some(Commands::Fs { .. }) => "fs",
        Some(Commands::Approval { .. }) => "approval",
        Some(Commands::Agent { .. }) => "agent",
        Some(Commands::Policy { .. }) => "policy",
        Some(Commands::Provider { .. }) => "provider",
        Some(Commands::Git { .. }) => "git",
        Some(Commands::Image { .. }) => "image",
        Some(Commands::Skill { .. }) => "skill",
        Some(Commands::Schedule { .. }) => "schedule",
        Some(Commands::Message { .. }) => "message",
        Some(Commands::Api { .. }) => "api",
        Some(Commands::Service { .. }) => "service",
        Some(Commands::SelfModify { .. }) => "self-modify",
        Some(Commands::Repo { .. }) => "repo",
        Some(Commands::Test { .. }) => "test",
        Some(Commands::Code { .. }) => "code",
        Some(Commands::Acp { .. }) => "acp",
    }
}

fn init_tracing(paths: &IkarosPaths) -> Result<()> {
    fs::create_dir_all(&paths.logs_dir)?;
    let trace_path = paths.logs_dir.join("trace.jsonl");
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(trace_path)?;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(
            "warn,ikaros=info,ikaros_cli=info,ikaros_runtime=info,ikaros_models=info,ikaros_harness=info,ikaros_skills=info,ikaros_session=info,ikaros_context=info,ikaros_memory=info,ikaros_gateway=info,ikaros_mcp=info",
        )
    });
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_env_filter(filter)
        .with_writer(Mutex::new(file))
        .try_init()
        .ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn global_agent_override_parses_before_and_after_subcommand() {
        let before = Cli::try_parse_from(["ikaros", "--agent", "plan", "doctor"]).expect("before");
        assert_eq!(before.agent.as_deref(), Some("plan"));

        let after = Cli::try_parse_from([
            "ikaros",
            "chat",
            "--agent",
            "plan",
            "--message",
            "inspect without editing",
        ])
        .expect("after");
        assert_eq!(after.agent.as_deref(), Some("plan"));

        let agent_run = Cli::try_parse_from([
            "ikaros",
            "agent",
            "run",
            "--profile",
            "plan",
            "--dry-run",
            "inspect without editing",
        ])
        .expect("agent run");
        assert!(matches!(
            agent_run.command,
            Some(Commands::Agent {
                command: AgentCommand::Run(_)
            })
        ));

        let config_validate =
            Cli::try_parse_from(["ikaros", "config", "validate"]).expect("config validate");
        assert!(matches!(
            config_validate.command,
            Some(Commands::Config {
                command: ConfigCommand::Validate { json: false }
            })
        ));

        let no_command = Cli::try_parse_from(["ikaros"]).expect("default chat");
        assert!(no_command.command.is_none());
        assert_eq!(requested_workspace_path(&no_command), PathBuf::from("."));
        assert_eq!(cli_command_name(&no_command.command), "chat-default");

        let default_chat_session =
            Cli::try_parse_from(["ikaros", "--chat-session", "root-session"])
                .expect("default chat session");
        assert!(default_chat_session.command.is_none());
        assert_eq!(
            requested_workspace_path(&default_chat_session),
            PathBuf::from(".")
        );

        let root_workspace = Cli::try_parse_from(["ikaros", "/tmp"]).expect("root workspace");
        assert!(root_workspace.command.is_none());
        assert_eq!(root_workspace.root, Some(PathBuf::from("/tmp")));
        assert_eq!(
            requested_workspace_path(&root_workspace),
            PathBuf::from("/tmp")
        );

        let legacy_workspace = Cli::try_parse_from(["ikaros", "--workspace", "/tmp"]);
        assert!(legacy_workspace.is_err());

        let project_path = Cli::try_parse_from(["ikaros", "project"]).expect("project path");
        assert!(project_path.command.is_none());
        assert_eq!(
            requested_workspace_path(&project_path),
            PathBuf::from("project")
        );

        for workspace_word in ["tui", "workbench"] {
            let parsed =
                Cli::try_parse_from(["ikaros", workspace_word]).expect("workspace word as path");
            assert!(parsed.command.is_none());
            assert_eq!(parsed.root, Some(PathBuf::from(workspace_word)));
            assert_eq!(
                requested_workspace_path(&parsed),
                PathBuf::from(workspace_word)
            );
        }
    }

    #[test]
    fn user_help_exposes_only_path_workspace_entrypoint() {
        let help = Cli::command().render_long_help().to_string();

        assert!(help.contains("[PATH]"));
        assert!(!help.contains("tui"));
        assert!(!help.contains("workbench"));
        assert!(!help.contains("--workspace"));
    }
}
