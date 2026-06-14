// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    agent::{AgentCommand, agent_command},
    approval::{ApprovalCommand, approval_command},
    body::{self, BodyCommand},
    chat::{ChatArgs, chat_command},
    code::{CodeCommand, code_command},
    config::{ConfigCommand, config_command},
    diagnostics::{DoctorArgs, doctor, init},
    fs::{FsCommand, fs_command},
    git::{GitCommand, git_command},
    memory::{MemoryCommand, memory_command},
    message::{MessageCommand, message_command},
    persona::{PersonaCommand, persona_command},
    policy::{PolicyCommand, policy_command},
    rag::{RagCommand, rag_command},
    relationship::{RelationshipCommand, relationship_command},
    repo::{RepoCommand, repo_command},
    schedule::{ScheduleCommand, schedule_command},
    self_modify::{SelfModifyCommand, self_modify_command},
    service::{ServiceCommand, service_command},
    skill::{SkillCommand, skill_command},
    task::{TaskCommand, task_command},
    testing::{TestCommand, test_command},
    voice::{VoiceCommand, voice_command},
};
use anyhow::Result;
use clap::{Parser, Subcommand};
use ikaros_core::IkarosPaths;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "ikaros", version, about = "Persona-first local agent runtime")]
struct Cli {
    #[arg(long, global = true)]
    ikaros_home: Option<PathBuf>,
    #[arg(long, global = true, default_value = ".")]
    workspace: PathBuf,
    #[arg(long, global = true, value_name = "PROFILE")]
    agent: Option<String>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Init,
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
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
    Body {
        #[command(subcommand)]
        command: BodyCommand,
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
    Git {
        #[command(subcommand)]
        command: GitCommand,
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
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .without_time()
        .try_init()
        .ok();

    let cli = Cli::parse();
    let paths = match &cli.ikaros_home {
        Some(home) => IkarosPaths::from_home(home),
        None => IkarosPaths::from_env()?,
    };
    let workspace = cli
        .workspace
        .canonicalize()
        .unwrap_or(cli.workspace.clone());

    match cli.command {
        Commands::Init => init(&paths)?,
        Commands::Config { command } => config_command(command, &paths)?,
        Commands::Doctor(args) => doctor(args, &paths, &workspace, cli.agent.as_deref())?,
        Commands::Persona { command } => persona_command(command, &paths)?,
        Commands::Memory { command } => {
            memory_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Commands::Relationship { command } => {
            relationship_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Commands::Rag { command } => {
            rag_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Commands::Voice { command } => {
            voice_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Commands::Body { command } => body::body_command(command, &paths, &workspace)?,
        Commands::Task { command } => {
            task_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Commands::Chat(args) => {
            chat_command(args, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Commands::Fs { command } => {
            fs_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Commands::Approval { command } => {
            approval_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Commands::Agent { command } => {
            agent_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Commands::Policy { command } => {
            policy_command(command, &paths, &workspace, cli.agent.as_deref())?
        }
        Commands::Git { command } => {
            git_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Commands::Skill { command } => {
            skill_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Commands::Schedule { command } => {
            schedule_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Commands::Message { command } => {
            message_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Commands::Service { command } => {
            service_command(command, &paths, &workspace, cli.agent.as_deref())?
        }
        Commands::SelfModify { command } => {
            self_modify_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Commands::Repo { command } => {
            repo_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Commands::Test { command } => {
            test_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
        Commands::Code { command } => {
            code_command(command, &paths, &workspace, cli.agent.as_deref()).await?
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
            Commands::Agent {
                command: AgentCommand::Run(_)
            }
        ));

        let config_validate =
            Cli::try_parse_from(["ikaros", "config", "validate"]).expect("config validate");
        assert!(matches!(
            config_validate.command,
            Commands::Config {
                command: ConfigCommand::Validate
            }
        ));
    }
}
