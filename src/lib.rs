mod agent;
mod app;
mod config;
mod domain;
mod provider;
mod store;
mod terminal_text;
mod tools;
mod tui;

pub async fn run() -> std::process::ExitCode {
    match app::run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "error: {}",
                terminal_text::sanitize_terminal(&format!("{error:#}"))
            );
            std::process::ExitCode::FAILURE
        }
    }
}
