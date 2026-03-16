mod adapters;
mod application;
mod domain;
mod infrastructure;

use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;
use crossterm::style::Stylize;

const VERSION_INFO: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("GIT_HASH"),
    " ",
    env!("BUILD_DATE"),
    ")"
);

/// clash — Terminal UI for Claude Code Sessions & Agent Teams
#[derive(Parser, Debug)]
#[command(version = VERSION_INFO, about)]
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
    /// Update clash to the latest version
    Update,
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

    match args.command {
        Some(Cmd::Daemon) => {
            tracing::info!("Starting clash daemon");
            let socket_path = infrastructure::daemon::client::DaemonClient::default_socket_path();
            let server = infrastructure::daemon::server::DaemonServer::new(socket_path);
            server.run().await?;
            return Ok(());
        }
        Some(Cmd::Update) => {
            return run_update().await;
        }
        None => {}
    }

    // Ensure terminal is restored on panic
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_panic(info);
    }));

    let config = infrastructure::config::Config::load();
    let data_dir = args.data_dir.unwrap_or_else(|| config.claude_dir());

    tracing::info!("clash starting, data_dir={:?}", data_dir);

    let mut terminal = ratatui::init();
    // Enable mouse button tracking (1000) + SGR mode (1006) for scroll support.
    // Do NOT enable mode 1003 (any-event tracking) — it floods the event stream
    // with mouse movement events and causes keystroke lag.
    {
        use std::io::Write;
        std::io::stdout().write_all(b"\x1b[?1000h\x1b[?1006h")?;
        std::io::stdout().flush()?;
    }
    let mut app = infrastructure::app::App::new(data_dir, args.claude_bin);
    let result = app.run(&mut terminal).await;
    restore_terminal();

    result
}

/// Fully restore the terminal to a clean state.
///
/// Resets everything that Clash may have changed:
/// - Scroll region (from attached mode's header/footer)
/// - Mouse tracking
/// - Kitty keyboard protocol
/// - Raw mode + alternate screen (via ratatui::restore)
fn restore_terminal() {
    use std::io::Write;
    let _ = std::io::stdout().write_all(
        concat!(
            "\x1b[?6l",      // Disable origin mode
            "\x1b[r",        // Reset scroll region to full terminal
            "\x1b[?1000l",   // Disable mouse button tracking
            "\x1b[?1006l",   // Disable SGR mouse mode
            "\x1b[<u",       // Pop Kitty keyboard protocol (if active)
            "\x1b[2J\x1b[H", // Clear screen + cursor home
        )
        .as_bytes(),
    );
    let _ = std::io::stdout().flush();
    ratatui::restore();
}

/// Run the CLI update command.
async fn run_update() -> Result<()> {
    println!("clash v{}", env!("CARGO_PKG_VERSION"));
    println!("Checking for updates...");

    match infrastructure::update::perform_update().await {
        Ok(version) => {
            println!("{} Updated to v{}!", "✓".green(), version);
            println!("  Restart clash to use the new version.");
            Ok(())
        }
        Err(msg) => {
            println!("{}", msg);
            Ok(())
        }
    }
}
