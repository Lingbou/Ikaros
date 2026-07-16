use crate::{
    agent::{Agent, ApprovalDecision, RuntimeYield},
    config::{AppConfig, AppPaths, ProviderConfig},
    provider::OpenAiProvider,
    store::SessionStore,
    terminal_text::sanitize_terminal,
    tools::ToolBox,
    tui,
};
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::{
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Parser)]
#[command(name = "ikaros", version, about = "A small terminal-first local agent")]
struct Cli {
    #[arg(long, global = true, value_name = "DIR")]
    home: Option<PathBuf>,
    #[arg(long, global = true, value_name = "DIR", default_value = ".")]
    workspace: PathBuf,
    #[arg(long, global = true, value_name = "ID")]
    session: Option<String>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a new minimal config.toml.
    Init {
        #[arg(long, default_value = "https://api.openai.com/v1")]
        base_url: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        force: bool,
    },
    /// Send one message without opening the full-screen terminal UI.
    Chat { message: String },
    /// List saved sessions for the active workspace.
    Sessions {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Validate local paths and provider configuration.
    Doctor,
}

pub(crate) async fn run() -> Result<()> {
    let cli = Cli::parse();
    tokio::select! {
        result = run_cli(cli) => result,
        signal = shutdown_signal() => {
            signal?;
            bail!("interrupted by shutdown signal")
        }
    }
}

async fn run_cli(cli: Cli) -> Result<()> {
    let paths = AppPaths::discover(cli.home)?;
    let _app_lock = paths.acquire_lock()?;
    let workspace = resolve_workspace(&cli.workspace)?;

    match cli.command {
        Some(Command::Init {
            base_url,
            model,
            force,
        }) => init(&paths, base_url, model, force),
        Some(Command::Sessions { limit }) => list_sessions(&paths, &workspace, limit),
        Some(Command::Doctor) => doctor(&paths, &workspace),
        Some(Command::Chat { message }) => {
            let (agent, model) = build_agent(&paths, &workspace)?;
            let session =
                resolve_session(agent.store(), &workspace, &model, cli.session.as_deref())?;
            resolve_existing_turn(&agent, &session.id).await?;
            run_one_shot(&agent, &session.id, &message).await
        }
        None => {
            let (agent, model) = build_agent(&paths, &workspace)?;
            tui::run(&agent, &workspace, &model, cli.session.as_deref()).await
        }
    }
}

async fn shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .context("failed to install SIGTERM handler")?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result.context("failed to install Ctrl-C handler")?;
            }
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("failed to install Ctrl-C handler")?;
    }
    Ok(())
}

fn init(paths: &AppPaths, base_url: String, model: String, force: bool) -> Result<()> {
    if model.trim().is_empty() {
        bail!("--model cannot be empty");
    }
    let config = AppConfig {
        provider: ProviderConfig { base_url, model },
    };
    config.save(paths, force)?;
    print_terminal(&format!("config: {}", paths.config.display()));
    print_terminal(&format!("database: {}", paths.database.display()));
    println!("next: ikaros");
    Ok(())
}

fn list_sessions(paths: &AppPaths, workspace: &Path, limit: usize) -> Result<()> {
    let store = SessionStore::open(&paths.database)?;
    let sessions = store.list_sessions(workspace, limit)?;
    if sessions.is_empty() {
        print_terminal(&format!("no sessions for {}", workspace.display()));
        return Ok(());
    }
    for summary in sessions {
        print_terminal(&format!(
            "{}\tmessages={}\tmodel={}\tupdated={}",
            summary.session.id,
            summary.message_count,
            summary.session.model,
            summary.session.updated_at
        ));
    }
    Ok(())
}

fn doctor(paths: &AppPaths, workspace: &Path) -> Result<()> {
    print_terminal(&format!("home: {}", paths.home.display()));
    print_terminal(&format!("workspace: {}", workspace.display()));
    print_terminal(&format!("config: {}", paths.config.display()));
    print_terminal(&format!("database: {}", paths.database.display()));
    let config = AppConfig::load(paths)?;
    let provider = config.resolve_provider()?;
    let store = SessionStore::open(&paths.database)?;
    let _tools = ToolBox::new(workspace)?;
    print_terminal(&format!("provider_base_url: {}", provider.base_url));
    print_terminal(&format!("provider_model: {}", provider.model));
    println!("provider_api_key: {}", provider.api_key.is_some());
    print_terminal(&format!("session_store: ok ({})", store.path().display()));
    println!("workspace_tools: ok");
    Ok(())
}

fn build_agent(paths: &AppPaths, workspace: &Path) -> Result<(Agent<OpenAiProvider>, String)> {
    let config = AppConfig::load(paths)?;
    let provider_config = config.resolve_provider()?;
    let model = provider_config.model.clone();
    let provider = OpenAiProvider::new(
        &provider_config.base_url,
        provider_config.model,
        provider_config.api_key,
    )?;
    let store = SessionStore::open(&paths.database)?;
    let tools = ToolBox::new(workspace)?;
    Ok((Agent::new(provider, store, tools), model))
}

fn resolve_session(
    store: &SessionStore,
    workspace: &Path,
    model: &str,
    requested: Option<&str>,
) -> Result<crate::domain::Session> {
    match requested {
        Some(id) => store.session(id, workspace),
        None => store
            .latest_session(workspace)?
            .map_or_else(|| store.create_session(workspace, model), Ok),
    }
}

async fn resolve_existing_turn(agent: &Agent<OpenAiProvider>, session_id: &str) -> Result<()> {
    let mut outcome = agent.resume(session_id).await?;
    loop {
        match outcome {
            RuntimeYield::Ready => return Ok(()),
            RuntimeYield::Completed(content) => {
                print_terminal(&format!("ikaros> {content}"));
                return Ok(());
            }
            RuntimeYield::AwaitingApproval(invocation) => {
                let decision = prompt_approval(&invocation.call)?;
                outcome = agent.decide(session_id, &invocation.id, decision).await?;
            }
            RuntimeYield::RecoveryRequired(invocation) => {
                if io::stdin().is_terminal() {
                    eprint_terminal(&format!(
                        "interrupted tool {} {} may have side effects; marking it failed without replay",
                        invocation.call.name, invocation.call.id
                    ));
                }
                outcome = agent
                    .recover_interrupted(session_id, &invocation.id)
                    .await?;
            }
        }
    }
}

async fn run_one_shot(
    agent: &Agent<OpenAiProvider>,
    session_id: &str,
    message: &str,
) -> Result<()> {
    let mut outcome = agent.start_turn(session_id, message).await?;
    loop {
        match outcome {
            RuntimeYield::Ready => bail!("runtime returned ready before completing the turn"),
            RuntimeYield::Completed(content) => {
                print_terminal(&content);
                return Ok(());
            }
            RuntimeYield::AwaitingApproval(invocation) => {
                let decision = prompt_approval(&invocation.call)?;
                outcome = agent.decide(session_id, &invocation.id, decision).await?;
            }
            RuntimeYield::RecoveryRequired(invocation) => {
                outcome = agent
                    .recover_interrupted(session_id, &invocation.id)
                    .await?;
            }
        }
    }
}

fn prompt_approval(call: &crate::domain::ToolCall) -> Result<ApprovalDecision> {
    let arguments = serde_json::to_string_pretty(&call.arguments)?;
    eprint_terminal(&format!(
        "tool: {}\ncall: {}\narguments:\n{}",
        call.name, call.id, arguments
    ));
    if call.name == "run_command" {
        eprintln!("warning: this starts a host process; workspace cwd is not an OS sandbox");
    }
    if !io::stdin().is_terminal() {
        eprintln!("decision: denied because stdin is not interactive");
        return Ok(ApprovalDecision::Deny);
    }
    eprint!("approve? [y/N] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("failed to read approval")?;
    Ok(if answer.trim().eq_ignore_ascii_case("y") {
        ApprovalDecision::Approve
    } else {
        ApprovalDecision::Deny
    })
}

fn resolve_workspace(path: &Path) -> Result<PathBuf> {
    let workspace = path
        .canonicalize()
        .with_context(|| format!("failed to resolve workspace {}", path.display()))?;
    if !workspace.is_dir() {
        bail!("workspace is not a directory: {}", workspace.display());
    }
    Ok(workspace)
}

fn print_terminal(value: &str) {
    println!("{}", sanitize_terminal(value));
}

fn eprint_terminal(value: &str) {
    eprintln!("{}", sanitize_terminal(value));
}
