mod adapters;
mod application;
mod domain;
mod infrastructure;

use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;

/// clash — K9s-style TUI for Claude Code Agent Teams
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Path to Claude data directory (default: ~/.claude)
    #[arg(long)]
    data_dir: Option<std::path::PathBuf>,

    /// Path to claude CLI binary
    #[arg(long, default_value = "claude")]
    claude_bin: String,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run the daemon (session persistence server)
    Daemon,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let log_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clash");
    std::fs::create_dir_all(&log_dir)?;

    let log_file = std::fs::File::create(log_dir.join("clash.log"))?;
    tracing_subscriber::fmt()
        .with_writer(log_file)
        .with_ansi(false)
        .with_target(false)
        .init();

    let args = Args::parse();

    // Handle daemon subcommand
    if matches!(args.command, Some(Cmd::Daemon)) {
        tracing::info!("Starting clash daemon");
        let socket_path = infrastructure::daemon::client::DaemonClient::default_socket_path();
        let server = infrastructure::daemon::server::DaemonServer::new(socket_path);
        server.run().await?;
        return Ok(());
    }

    let config = infrastructure::config::Config::load();
    let data_dir = args.data_dir.unwrap_or_else(|| config.claude_dir());

    tracing::info!("clash starting, data_dir={:?}", data_dir);

    let mut terminal = ratatui::init();
    let mut app = infrastructure::app::App::new(data_dir, args.claude_bin);
    let result = app.run(&mut terminal).await;
    ratatui::restore();

    result
}
