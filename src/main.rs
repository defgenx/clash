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

    /// Enable debug logging (writes verbose logs to clash.log)
    #[arg(long)]
    debug: bool,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run the daemon (session persistence server)
    Daemon,
    /// Update clash to the latest version
    Update,
    /// Attach to a running session (used by new-window spawning)
    Attach {
        /// The session ID to attach to
        session_id: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let log_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("clash");
    std::fs::create_dir_all(&log_dir)?;

    // Rotate stale log file. Default: 24h. Override with CLASH_LOG_RETENTION_HOURS.
    let log_path = log_dir.join("clash.log");
    let retention_hours: u64 = std::env::var("CLASH_LOG_RETENTION_HOURS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    if let Ok(meta) = std::fs::metadata(&log_path) {
        if let Ok(modified) = meta.modified() {
            if modified.elapsed().unwrap_or_default()
                > std::time::Duration::from_secs(retention_hours * 3600)
            {
                let _ = std::fs::remove_file(&log_path);
            }
        }
    }

    let args = Args::parse();

    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let max_level = if args.debug {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    tracing_subscriber::fmt()
        .with_writer(log_file)
        .with_ansi(false)
        .with_target(false)
        .with_max_level(max_level)
        .init();

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
        Some(Cmd::Attach { session_id }) => {
            return infrastructure::windowing::attach::run_attach_client(session_id).await;
        }
        None => {}
    }

    // Single-instance lock — prevent multiple clash TUIs from running
    let _instance_lock = match infrastructure::lock::SingleInstanceLock::acquire() {
        Ok(lock) => lock,
        Err(msg) => {
            eprintln!("{}", msg.red());
            std::process::exit(1);
        }
    };

    // Ensure terminal is restored on panic
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_panic(info);
    }));

    let config = infrastructure::config::Config::load();
    let data_dir = args.data_dir.unwrap_or_else(|| config.claude_dir());

    tracing::info!("clash starting, data_dir={:?}", data_dir);

    // Start the daemon server in-process as a background task.
    // This replaces the old approach of spawning a separate `clash daemon` process.
    // When Clash exits, the daemon shuts down with it — no orphan processes.
    let socket_path = infrastructure::daemon::client::DaemonClient::default_socket_path();
    let daemon_server = infrastructure::daemon::server::DaemonServer::new(socket_path);
    let daemon_shutdown = daemon_server.shutdown_handle();
    let daemon_task = tokio::spawn(async move {
        if let Err(e) = daemon_server.run().await {
            tracing::error!("Daemon server error: {}", e);
        }
    });
    // Brief wait for the daemon to bind the socket
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut terminal = ratatui::init();
    {
        use std::io::Write;
        // Use .ok() — if these fail, we must not skip restore_terminal()
        let _ = std::io::stdout().write_all(b"\x1b[?1000h\x1b[?1006h");
        let _ = std::io::stdout().flush();
    }
    let mut app = infrastructure::app::App::new(data_dir, args.claude_bin, args.debug, config);

    // Race the TUI against OS signals so we always restore the terminal
    let result = tokio::select! {
        result = app.run(&mut terminal) => result,
        _ = shutdown_signal() => {
            tracing::info!("Signal received, shutting down");
            Ok(())
        },
    };
    restore_terminal();

    // Graceful shutdown: signal daemon to stop, wait briefly
    daemon_shutdown.notify_one();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), daemon_task).await;

    result
}

/// Wait for any OS shutdown signal (SIGINT, SIGTERM, SIGHUP).
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigterm = signal(SignalKind::terminate()).expect("register SIGTERM");
    let mut sighup = signal(SignalKind::hangup()).expect("register SIGHUP");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = sigterm.recv() => {}
        _ = sighup.recv() => {}
    }
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
    // Reset terminal modes while still on the alternate screen
    let _ = std::io::stdout().write_all(
        concat!(
            "\x1b[?6l",    // Disable origin mode
            "\x1b[r",      // Reset scroll region to full terminal
            "\x1b[?1000l", // Disable mouse button tracking
            "\x1b[?1006l", // Disable SGR mouse mode
            "\x1b[<u",     // Pop Kitty keyboard protocol (if active)
        )
        .as_bytes(),
    );
    let _ = std::io::stdout().flush();

    // Leave alternate screen + disable raw mode
    ratatui::restore();

    // Clear the main screen so attach session output doesn't linger
    let _ = std::io::stdout().write_all(
        concat!(
            "\x1b[2J\x1b[H", // Clear screen + cursor home
            "\x1b[?25h",     // Show cursor (ratatui may have hidden it)
            "\x1b[0m",       // Reset text attributes
        )
        .as_bytes(),
    );
    let _ = std::io::stdout().flush();
}

/// Run the CLI update command.
async fn run_update() -> Result<()> {
    println!("clash v{}", env!("CARGO_PKG_VERSION"));
    println!("Checking for updates...");

    match infrastructure::update::perform_update_cli().await {
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
