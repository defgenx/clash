//! Application coordinator — the runtime event loop.
//!
//! This is infrastructure: it owns the terminal, the backends, and the
//! event loop. It translates abstract Effects from the reducer into real IO.
//!
//! Uses `EventLoop` (backed by crossterm's async `EventStream` and
//! `tokio::select!`) so terminal input and daemon output are processed
//! concurrently without blocking or starvation.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::adapters::{input, renderer};
use crate::application::actions::Action;
use crate::application::effects::Effect;
use crate::application::reducer;
use crate::application::state::{AppState, InputMode};
use crate::domain::entities::{Session, SessionSection};
use crate::domain::ports::{CliGateway, DataRepository};
use crate::infrastructure::cli::commands;
use crate::infrastructure::cli::runner::RealCliRunner;
use crate::infrastructure::daemon::client::DaemonClient;
use crate::infrastructure::event::{Event, EventLoop};
use crate::infrastructure::fs::backend::FsBackend;
use crate::infrastructure::fs::watcher::FsWatcher;
use tokio::sync::mpsc;

/// Main application coordinator.
pub struct App {
    state: AppState,
    backend: FsBackend,
    cli_runner: RealCliRunner,
    config: crate::infrastructure::config::Config,
    _watcher: Option<FsWatcher>,
    fs_event_rx: Option<tokio::sync::mpsc::UnboundedReceiver<Vec<PathBuf>>>,
    daemon: DaemonClient,
    ext_open_times: HashMap<String, Instant>,
    /// Persisted ratatui table state for the sessions view — preserves scroll offset across frames.
    sessions_visual_state: ratatui::widgets::TableState,
    /// Dirty flag: only redraw when something changed. Set by events, cleared after draw.
    needs_redraw: bool,
    /// Per-session streak counter: how many consecutive refresh cycles a session has
    /// been absent from the incoming list. Once the streak exceeds
    /// `MISSING_STREAK_THRESHOLD`, the session is removed.
    missing_streaks: HashMap<String, u8>,
    /// Sessions that were intentionally dropped/killed. Keyed by session ID, value
    /// is the age counter (incremented each refresh cycle). Prevents the merge from
    /// re-adding a session that was just dropped but may still appear from the daemon.
    recently_removed: HashMap<String, u8>,
    /// Cached session registry — avoids re-reading sessions.json from disk every cycle.
    registry_cache: crate::infrastructure::hooks::registry::RegistryCache,
    /// Tick at which the transient spinner should auto-clear. Set by execute_effects()
    /// when a pending_toast is present, so the busy overlay stays visible briefly.
    pending_spinner_clear: Option<usize>,
    /// Latest snapshot of wild claude processes from the background scan
    /// task. Read into `RefreshInput.wild_processes` each refresh cycle.
    wild_processes_rx:
        tokio::sync::watch::Receiver<Vec<crate::infrastructure::process_scan::WildProcess>>,
    /// Notify handle the background scan task listens on. `WakeWildScan`
    /// effects fire `notify_one()` so the task immediately re-scans
    /// instead of waiting for its next tick.
    wild_scan_wake: std::sync::Arc<tokio::sync::Notify>,
}

impl App {
    pub fn new(
        data_dir: PathBuf,
        claude_bin: String,
        debug: bool,
        config: crate::infrastructure::config::Config,
    ) -> Self {
        let backend = FsBackend::new(data_dir.clone());
        let cli_runner = RealCliRunner::with_bin(claude_bin);

        // Install Claude Code hooks for instant status detection
        if let Err(e) = crate::infrastructure::hooks::install_hooks(&data_dir) {
            tracing::warn!("Failed to install hooks: {}", e);
        }

        let (fs_tx, fs_rx) = tokio::sync::mpsc::unbounded_channel();
        let status_dir = crate::infrastructure::hooks::status_dir(&data_dir);
        let watch_paths = vec![
            backend.teams_dir(),
            backend.tasks_dir(),
            backend.projects_dir(),
            status_dir,
            crate::infrastructure::hooks::registry::RegistryCache::watched_path(),
        ];
        let debounce = std::time::Duration::from_millis(config.debounce_ms);
        let watcher = FsWatcher::new(&watch_paths, fs_tx, debounce).ok();

        let mut state = AppState::new();
        state.debug_mode = debug;

        // Show guided tour on first launch (stored in clash's own data dir)
        let clash_data = crate::infrastructure::config::Config::clash_data_dir();
        let tour_marker = clash_data.join(".tour_done");
        if !tour_marker.exists() {
            state.tour_step = Some(0);
            let _ = std::fs::create_dir_all(&clash_data);
            let _ = std::fs::write(&tour_marker, "1");
        }

        if let Err(e) = state.store.refresh_all(&backend) {
            tracing::error!("Initial data load failed: {}", e);
        }

        // Restore UI state from previous session (best-effort)
        let ui_path = data_dir.join("clash/ui_state.json");
        if let Ok(content) = std::fs::read_to_string(&ui_path) {
            if let Ok(snapshot) =
                serde_json::from_str::<crate::application::state::UiSnapshot>(&content)
            {
                state.restore(snapshot);
            }
        }

        let daemon = DaemonClient::new(DaemonClient::default_socket_path());

        // Spawn the wild-process background scan task. Pushes the
        // latest Vec<WildProcess> into a watch channel; the refresh
        // cycle reads the borrowed snapshot without blocking.
        let (wild_processes_tx, wild_processes_rx) = tokio::sync::watch::channel(Vec::new());
        let wild_scan_wake = std::sync::Arc::new(tokio::sync::Notify::new());
        let wake_for_task = wild_scan_wake.clone();
        tokio::spawn(async move {
            use crate::infrastructure::process_scan::{default_fd_probe, gather_wild_processes};
            let probe = default_fd_probe();
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = wake_for_task.notified() => {}
                }
                let wild = gather_wild_processes(&probe);
                if wild_processes_tx.send(wild).is_err() {
                    // All receivers dropped — App is shutting down.
                    break;
                }
            }
        });

        Self {
            state,
            backend,
            cli_runner,
            config,
            _watcher: watcher,
            fs_event_rx: Some(fs_rx),
            daemon,
            ext_open_times: HashMap::new(),
            sessions_visual_state: ratatui::widgets::TableState::default(),
            needs_redraw: true,
            missing_streaks: HashMap::new(),
            recently_removed: HashMap::new(),
            registry_cache: crate::infrastructure::hooks::registry::RegistryCache::new(),
            pending_spinner_clear: None,
            wild_processes_rx,
            wild_scan_wake,
        }
    }

    /// Run the main event loop.
    ///
    /// When a session attach is requested, the event loop is fully torn down
    /// (killing crossterm's reader thread), a standalone sync loop takes over
    /// fd 0, and on Ctrl+B the event loop is rebuilt from scratch.
    pub async fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> color_eyre::Result<()> {
        let mut fs_rx = self.fs_event_rx.take();

        // Show loading overlay while startup tasks run
        self.state.spinner = Some("Loading sessions...".to_string());
        {
            let state = &self.state;
            let vs = &mut self.sessions_visual_state;
            let _ = terminal.draw(|f| renderer::draw(state, vs, f));
        }

        // Auto-connect to daemon (best-effort)
        let mut daemon_rx = None;
        match self.daemon.connect().await {
            Ok(()) => {
                tracing::info!("Connected to clash daemon");
                daemon_rx = self.daemon.take_stream_rx();
            }
            Err(e) => tracing::info!("Daemon not available (legacy mode): {}", e),
        }

        // Restore registered sessions in the daemon (resume Claude conversations)
        self.restore_sessions().await;

        // Load initial sessions
        self.refresh_daemon_sessions().await;

        // Clear the loading overlay
        self.state.spinner = None;

        // Background update check
        let mut update_check: Option<tokio::task::JoinHandle<_>> = Some(tokio::spawn(async {
            crate::infrastructure::update::check_for_update().await
        }));

        loop {
            // Create a fresh EventLoop for the TUI phase
            let mut events = EventLoop::new(Duration::from_millis(10));
            if let Some(rx) = daemon_rx.take() {
                events.set_daemon_rx(rx);
            }

            // The TUI event loop — runs until quit or attach
            let attach_result = loop {
                // Poll background update check
                if let Some(ref handle) = update_check {
                    if handle.is_finished() {
                        if let Some(handle) = update_check.take() {
                            if let Ok(Some(
                                crate::infrastructure::update::UpdateCheck::Available {
                                    version,
                                    ..
                                },
                            )) = handle.await
                            {
                                self.state.toast =
                                    Some(format!("v{} available — :update to install", version));
                                self.needs_redraw = true;
                            }
                        }
                    }
                }

                if self.needs_redraw {
                    let visual_state = &mut self.sessions_visual_state;
                    let state = &self.state;
                    terminal.draw(|f| renderer::draw(state, visual_state, f))?;
                    self.needs_redraw = false;
                }

                // Non-blocking FS event check
                if let Some(ref mut rx) = fs_rx {
                    let mut needs_refresh_all = false;
                    let mut changed_jsonl_paths: Vec<std::path::PathBuf> = Vec::new();
                    while let Ok(paths) = rx.try_recv() {
                        for p in &paths {
                            if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                                changed_jsonl_paths.push(p.clone());
                            } else {
                                needs_refresh_all = true;
                            }
                        }
                    }
                    if needs_refresh_all {
                        self.backend.invalidate_session_cache_all();
                        self.registry_cache.invalidate();
                        let _ = self.state.store.refresh_all(&self.backend);
                        self.needs_redraw = true;
                    } else if !changed_jsonl_paths.is_empty() {
                        // Invalidate only the affected project directories
                        self.backend.invalidate_session_cache(&changed_jsonl_paths);
                        self.refresh_daemon_sessions().await;
                        self.needs_redraw = true;
                    }
                }

                let maybe_event = events.next().await;

                if let Some(event) = maybe_event {
                    match event {
                        Event::Key(key) => {
                            if self
                                .handle_key_event(key, terminal, &mut events)
                                .await
                                .is_err()
                            {
                                return Ok(()); // Quit requested
                            }
                            self.needs_redraw = true;
                        }
                        Event::Tick => {
                            if self.handle_tick(terminal, &mut events).await {
                                return Ok(()); // Quit requested (shutdown complete)
                            }
                            // Only redraw on animation frames when something is actually
                            // animating (spinners, active session icons, update overlay).
                            // Static screens stay untouched — zero flicker.
                            if self.state.needs_animation() && self.state.tick.is_multiple_of(12) {
                                self.needs_redraw = true;
                            }
                        }
                        Event::Resize(width, height) => {
                            self.handle_resize(width, height).await;
                            self.needs_redraw = true;
                        }
                        Event::DaemonExited { session_id } => {
                            let action =
                                Action::Ui(crate::application::actions::UiAction::SessionExited {
                                    session_id,
                                });
                            let effects = reducer::reduce(&mut self.state, action);
                            if self.execute_effects(effects, terminal, &mut events).await {
                                return Ok(());
                            }
                            self.needs_redraw = true;
                        }
                        Event::DaemonOutput => {}
                        Event::Mouse(mouse) => {
                            self.handle_mouse(mouse, terminal, &mut events).await;
                            self.needs_redraw = true;
                        }
                        Event::UpdateProgress(phase) => {
                            use crate::application::state::UpdatePhase;
                            let is_terminal = matches!(
                                phase,
                                UpdatePhase::Done { .. } | UpdatePhase::Failed { .. }
                            );
                            self.state.update_progress = Some(phase);
                            if is_terminal {
                                // Auto-dismiss after a few seconds via toast
                                if let Some(UpdatePhase::Done { ref version }) =
                                    self.state.update_progress
                                {
                                    self.state.toast = Some(format!(
                                        "Updated to v{}! Restart clash to apply.",
                                        version
                                    ));
                                } else if let Some(UpdatePhase::Failed { ref message }) =
                                    self.state.update_progress
                                {
                                    self.state.toast = Some(message.clone());
                                }
                                self.state.update_progress = None;
                                self.state.spinner = None;
                            }
                            self.needs_redraw = true;
                        }
                    }
                } else {
                    return Ok(());
                }

                // Check if an attach was requested (set by DaemonAttach effect)
                if let Some(ref _session_id) = self.state.attached_session {
                    if self.state.input_mode == InputMode::Attached {
                        // Buffer daemon history while the TUI stays visible
                        // with its busy overlay. Only switch to raw mode once
                        // the session output has settled.
                        if let Some(history) =
                            self.buffer_attach_history(terminal, &mut events).await
                        {
                            break (self.state.attached_session.clone(), history);
                        }
                        // Session exited during loading — state was reset,
                        // continue TUI event loop.
                        self.needs_redraw = true;
                    }
                }
            };

            let (attach_request, pre_history) = attach_result;

            // ── Attach phase ────────────────────────────────────────
            // Save daemon_rx before dropping EventLoop
            daemon_rx = events.take_daemon_rx();

            // Drop EventLoop — crossterm's EventStream is released.
            // The attach loop reads from its own /dev/tty fd, so crossterm's
            // lingering reader thread on fd 0 doesn't interfere.
            drop(events);

            if let Some(ref session_id) = attach_request {
                // Leave TUI — switch to main screen for Claude Code.
                crossterm::execute!(
                    std::io::stdout(),
                    crossterm::terminal::LeaveAlternateScreen,
                    crossterm::event::DisableMouseCapture
                )
                .ok();

                // Clear the restored main screen so no pre-clash terminal
                // content bleeds through when replaying session history.
                // Paint with BUSY_BG (matches the TUI busy overlay) so the
                // transition from alt-screen overlay to raw screen stays in
                // the same dark shade instead of flashing to terminal default.
                {
                    use crate::infrastructure::windowing::attach::BUSY_BG;
                    use std::io::Write;
                    let mut bytes = BUSY_BG.as_bytes().to_vec();
                    bytes.extend_from_slice(b"\x1b[2J\x1b[H");
                    std::io::stdout().write_all(&bytes).ok();
                    std::io::stdout().flush().ok();
                }

                // Pass buffered history to attach_loop which sets up the scroll
                // region before replaying, so output stays above the status bar.
                let history = if pre_history.is_empty() {
                    None
                } else {
                    Some(pre_history)
                };

                // Run the attached session — pure sync loop on fd 0.
                // No crossterm, no EventStream, no race. Sole reader on stdin.
                self.run_attached(session_id, &mut daemon_rx, history).await;

                // Re-enter TUI on alternate screen.
                // First, clean up any terminal modes the attached Claude Code
                // session may have enabled (bracketed paste, focus reporting,
                // extra mouse modes, Kitty keyboard). These persist beyond
                // detach and would otherwise leak into the shell on quit.
                {
                    use crate::infrastructure::tui::terminal_reset::MODES_RESET;
                    use std::io::Write;
                    std::io::stdout().write_all(MODES_RESET).ok();
                    std::io::stdout().flush().ok();
                }
                crossterm::terminal::enable_raw_mode().ok();
                {
                    use std::io::Write;
                    std::io::stdout().write_all(b"\x1b[?1000h\x1b[?1006h").ok();
                    std::io::stdout().flush().ok();
                }
                crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)
                    .ok();

                // Force ratatui to repaint every cell on the next draw by
                // resizing the internal buffers. Unlike terminal.clear(), this
                // does NOT send a visible clear-screen escape to the terminal,
                // so the alternate screen stays blank until the first full
                // frame is flushed — eliminating the flash/flicker on detach.
                if let Ok(size) = terminal.size() {
                    let _ =
                        terminal.resize(ratatui::layout::Rect::new(0, 0, size.width, size.height));
                }

                self.state.input_mode = InputMode::Normal;
                self.state.attached_session = None;
                self.state.terminal_screen = None;
                self.state.spinner = None;

                // Draw cached state immediately so the user sees content right
                // away instead of a blank/splash screen during the async refresh.
                {
                    let state = &self.state;
                    let vs = &mut self.sessions_visual_state;
                    let _ = terminal.draw(|f| renderer::draw(state, vs, f));
                }

                self.refresh_daemon_sessions().await;
                self.needs_redraw = true;
            }
            // Loop back → creates a fresh EventLoop with a new crossterm
        }
    }

    /// Run the attached session loop — delegates to the shared `attach_loop`.
    ///
    /// crossterm is fully dead at this point. We are the sole reader on stdin.
    /// Ctrl+B detaches. Everything else is forwarded to the daemon PTY.
    async fn run_attached(
        &mut self,
        session_id: &str,
        daemon_rx: &mut Option<
            mpsc::UnboundedReceiver<crate::infrastructure::daemon::protocol::Event>,
        >,
        pre_history: Option<Vec<u8>>,
    ) {
        use crate::infrastructure::windowing::attach::{
            attach_loop, display_name, AttachInfo, AttachResult,
        };

        // Gather session metadata for the status bar.
        let session = self.state.store.find_session(session_id);
        let project = session
            .map(|s| {
                s.project_path
                    .rsplit('/')
                    .next()
                    .filter(|p| !p.is_empty())
                    .unwrap_or(&s.project_path)
                    .to_string()
            })
            .unwrap_or_default();
        let branch = session.map(|s| s.git_branch.clone()).unwrap_or_default();
        let info = AttachInfo {
            name: display_name(
                session.and_then(|s| s.name.as_deref()),
                &project,
                &branch,
                session_id,
            ),
            project,
            branch,
        };

        let result = attach_loop(&mut self.daemon, session_id, &info, daemon_rx, pre_history).await;

        if result == AttachResult::SessionExited {
            self.state.toast = Some("Session exited".to_string());
        }

        // Log detach failures instead of swallowing them. Silent failures
        // used to wedge the next attach with "Already attached" — the
        // server is now idempotent, but diagnostics still matter.
        if let Err(e) = self.daemon.detach(session_id).await {
            tracing::warn!("detach {} failed: {}", session_id, e);
        }
    }

    /// Buffer the daemon's replay-buffer Output events into a Vec while
    /// the TUI's busy overlay stays visible. The bytes are then handed
    /// to `attach_loop` as `pre_history`, which renders the snapshot via
    /// vt100 so the user sees Claude's last screen state (including
    /// background and SGR) the moment the alt-screen exits.
    ///
    /// Returns `None` if the session exited during the buffering window.
    async fn buffer_attach_history(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        events: &mut EventLoop,
    ) -> Option<Vec<u8>> {
        use crate::infrastructure::daemon::protocol;
        use tokio::time::{interval, MissedTickBehavior};

        const REDRAW_MS: u64 = 50;
        // History-buffer pacing — mirrors the standalone client's loop
        // in attach::buffer_history. Kept small so attach feels instant.
        const MIN_VISIBLE_MS: u64 = 150;
        const HARD_LIMIT_MS: u64 = 500;
        const EMPTY_TIMEOUT_MS: u64 = 80;
        const IDLE_MS: u64 = 80;

        // Drain stale events from any prior attach on this persistent
        // connection. The server-side forwarder for the previous session
        // may have pushed bytes between our Detach RPC and the forwarder
        // task actually being aborted; those bytes belong to the OLD
        // session. Toss them.
        if let Some(rx) = events.daemon_rx_mut() {
            while rx.try_recv().is_ok() {}
        }

        let session_id_owned = self.state.attached_session.clone();
        let session_id_for_filter: Option<&str> = session_id_owned.as_deref();

        let mut daemon_rx = events.take_daemon_rx();
        let mut history: Vec<u8> = Vec::new();
        let mut got_output = false;
        let started = tokio::time::Instant::now();
        let mut last_output = started;
        let mut session_exited = false;

        // Draw the busy overlay immediately so it's visible from the start.
        {
            let state = &self.state;
            let vs = &mut self.sessions_visual_state;
            let _ = terminal.draw(|f| renderer::draw(state, vs, f));
        }

        let mut redraw = interval(std::time::Duration::from_millis(REDRAW_MS));
        redraw.set_missed_tick_behavior(MissedTickBehavior::Skip);
        redraw.tick().await; // consume the immediate first tick

        loop {
            tokio::select! {
                biased;

                Some(ev) = async {
                    match daemon_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match ev {
                        protocol::Event::Output { session_id: ev_sid, data } => {
                            if Some(ev_sid.as_str()) != session_id_for_filter {
                                continue;
                            }
                            if let Ok(bytes) = protocol::decode_data(&data) {
                                history.extend_from_slice(&bytes);
                            }
                            got_output = true;
                            last_output = tokio::time::Instant::now();
                        }
                        protocol::Event::Exited { session_id: ev_sid, .. }
                            if Some(ev_sid.as_str()) == session_id_for_filter =>
                        {
                            session_exited = true;
                            break;
                        }
                        _ => {}
                    }
                }

                Some(event) = events.next() => {
                    if let Event::Resize(w, h) = event {
                        self.handle_resize(w, h).await;
                    }
                }

                _ = redraw.tick() => {
                    self.state.tick = self.state.tick.wrapping_add(1);

                    let now = tokio::time::Instant::now();
                    let elapsed_ms = now.duration_since(started).as_millis() as u64;
                    let idle_ms = now.duration_since(last_output).as_millis() as u64;
                    let break_now = elapsed_ms >= MIN_VISIBLE_MS
                        && (elapsed_ms >= HARD_LIMIT_MS
                            || (!got_output && elapsed_ms >= EMPTY_TIMEOUT_MS)
                            || (got_output && idle_ms >= IDLE_MS));
                    if break_now {
                        break;
                    }

                    let state = &self.state;
                    let vs = &mut self.sessions_visual_state;
                    let _ = terminal.draw(|f| renderer::draw(state, vs, f));
                }
            }
        }

        // Put daemon_rx back for the live phase
        if let Some(rx) = daemon_rx {
            events.set_daemon_rx(rx);
        }

        if session_exited {
            self.state.toast = Some("Session exited".to_string());
            self.state.input_mode = InputMode::Normal;
            self.state.attached_session = None;
            self.state.spinner = None;
            self.state.pending_toast = None;
            None
        } else {
            Some(history)
        }
    }

    async fn handle_key_event(
        &mut self,
        key: crossterm::event::KeyEvent,
        terminal: &mut ratatui::DefaultTerminal,
        events: &mut EventLoop,
    ) -> color_eyre::Result<()> {
        // During graceful shutdown: only Ctrl+C → ForceQuit allowed
        if self.state.shutting_down.is_some() {
            if key.code == crossterm::event::KeyCode::Char('c')
                && key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL)
            {
                let effects = reducer::reduce(
                    &mut self.state,
                    Action::Ui(crate::application::actions::UiAction::ForceQuit),
                );
                if self.execute_effects(effects, terminal, events).await {
                    return Err(color_eyre::eyre::eyre!("quit"));
                }
            }
            return Ok(());
        }

        // Ctrl+C: cancel current mode, or quit from normal mode
        if key.code == crossterm::event::KeyCode::Char('c')
            && key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
        {
            let action = match self.state.input_mode {
                InputMode::Normal => Action::Ui(crate::application::actions::UiAction::Quit),
                InputMode::Confirm => Action::Ui(crate::application::actions::UiAction::ConfirmNo),
                _ => Action::Ui(crate::application::actions::UiAction::ExitInputMode),
            };
            let effects = reducer::reduce(&mut self.state, action);
            if self.execute_effects(effects, terminal, events).await {
                return Err(color_eyre::eyre::eyre!("quit"));
            }
            return Ok(());
        }

        // Text input mode (command, filter, new-session). Submit/cancel flow
        // through the reducer as actions; everything else (typing, cursor
        // motion, word jump, kill-word, kill-line, …) is delegated to
        // `tui-input`'s crossterm handler so we don't have to enumerate every
        // modifier+key combo by hand.
        if matches!(
            self.state.input_mode,
            InputMode::Command
                | InputMode::Filter
                | InputMode::NewSession
                | InputMode::NewSessionName
                | InputMode::NewSessionWorktree
        ) {
            use crate::adapters::input::key_to_input_request;
            use crate::application::actions::UiAction;

            match key.code {
                crossterm::event::KeyCode::Enter => {
                    let input = self.state.input.value().to_string();
                    let effects =
                        reducer::reduce(&mut self.state, Action::Ui(UiAction::SubmitInput(input)));
                    if self.execute_effects(effects, terminal, events).await {
                        return Err(color_eyre::eyre::eyre!("quit"));
                    }
                }
                crossterm::event::KeyCode::Esc => {
                    let effects =
                        reducer::reduce(&mut self.state, Action::Ui(UiAction::ExitInputMode));
                    if self.execute_effects(effects, terminal, events).await {
                        return Err(color_eyre::eyre::eyre!("quit"));
                    }
                }
                _ => {
                    if let Some(req) = key_to_input_request(key) {
                        self.state.input.handle(req);
                        // Live-filter sessions while typing in Filter mode.
                        if self.state.input_mode == InputMode::Filter {
                            self.state.filter = self.state.input.value().to_string();
                            self.state.table_state.selected = 0;
                        }
                    }
                    if self.state.spinner.is_some() {
                        let state = &self.state;
                        let vs = &mut self.sessions_visual_state;
                        let _ = terminal.draw(|f| renderer::draw(state, vs, f));
                    }
                }
            }
            return Ok(());
        }

        // Normal mode
        let action = input::handle_key(key, &self.state);
        let effects = reducer::reduce(&mut self.state, action);
        self.draw_if_spinner(terminal);
        if self.execute_effects(effects, terminal, events).await {
            return Err(color_eyre::eyre::eyre!("quit"));
        }
        Ok(())
    }

    /// Handle periodic tick events. Returns `true` when the app should quit.
    async fn handle_tick(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        events: &mut EventLoop,
    ) -> bool {
        let effects = reducer::reduce(
            &mut self.state,
            Action::Ui(crate::application::actions::UiAction::Tick),
        );
        // Refresh sessions every ~500ms (50 ticks) on session-related views.
        // Also refresh during shutdown to detect dead sessions regardless of view.
        let is_shutting_down = self.state.shutting_down.is_some();
        if self.state.input_mode != InputMode::Attached
            && self.state.tick.is_multiple_of(50)
            && (is_shutting_down
                || matches!(
                    self.state.current_view(),
                    crate::adapters::views::ViewKind::Sessions
                        | crate::adapters::views::ViewKind::SessionDetail
                        | crate::adapters::views::ViewKind::Subagents
                        | crate::adapters::views::ViewKind::SubagentDetail
                        | crate::adapters::views::ViewKind::Diff
                ))
        {
            self.refresh_daemon_sessions().await;
            self.needs_redraw = true;
        }
        // Refresh conversation every ~1s (100 ticks)
        if self.state.tick.is_multiple_of(100) && self.auto_refresh_conversation() {
            self.needs_redraw = true;
        }
        // Auto-clear transient spinner after the scheduled delay
        if let Some(clear_at) = self.pending_spinner_clear {
            if self.state.tick >= clear_at {
                self.state.spinner = None;
                if let Some(toast) = self.state.pending_toast.take() {
                    self.state.toast = Some(toast);
                }
                self.pending_spinner_clear = None;
                self.needs_redraw = true;
            }
        }
        if !effects.is_empty() {
            return self.execute_effects(effects, terminal, events).await;
        }
        false
    }

    /// Handle mouse events (scroll).
    ///
    /// When attached: forward scroll as escape sequences to the PTY
    /// with coordinates adjusted for the body area (row offset by 1 for header).
    /// When not attached: translate scroll into table navigation actions.
    async fn handle_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        terminal: &mut ratatui::DefaultTerminal,
        events: &mut EventLoop,
    ) {
        use crossterm::event::MouseEventKind;

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if self.state.input_mode == InputMode::Attached {
                    // Adjust row: body starts at row 1 (after header), SGR is 1-indexed
                    // so PTY row = mouse.row (header offset cancels with 1-indexing)
                    let pty_row = mouse.row;
                    let pty_col = mouse.column + 1;
                    if mouse.row >= 1 {
                        let seq = format!("\x1b[<64;{};{}M", pty_col, pty_row);
                        if let Some(ref session_id) = self.state.attached_session.clone() {
                            let _ = self.daemon.send_input(session_id, seq.as_bytes()).await;
                        }
                    }
                } else {
                    let action = Action::Ui(crate::application::actions::UiAction::ScrollUp);
                    let effects = reducer::reduce(&mut self.state, action);
                    let _ = self.execute_effects(effects, terminal, events).await;
                }
            }
            MouseEventKind::ScrollDown => {
                if self.state.input_mode == InputMode::Attached {
                    let pty_row = mouse.row;
                    let pty_col = mouse.column + 1;
                    if mouse.row >= 1 {
                        let seq = format!("\x1b[<65;{};{}M", pty_col, pty_row);
                        if let Some(ref session_id) = self.state.attached_session.clone() {
                            let _ = self.daemon.send_input(session_id, seq.as_bytes()).await;
                        }
                    }
                } else {
                    let action = Action::Ui(crate::application::actions::UiAction::ScrollDown);
                    let effects = reducer::reduce(&mut self.state, action);
                    let _ = self.execute_effects(effects, terminal, events).await;
                }
            }
            _ => {}
        }
    }

    /// Handle terminal resize events.
    async fn handle_resize(&mut self, width: u16, height: u16) {
        // When attached, resize the PTY to match the full terminal
        if self.state.input_mode == InputMode::Attached {
            if let Some(ref session_id) = self.state.attached_session.clone() {
                let _ = self.daemon.resize(session_id, width, height).await;
            }
        }
        // Normal mode resize is handled by ratatui automatically.
    }

    /// Draw a frame immediately if the spinner is active.
    fn draw_if_spinner(&mut self, terminal: &mut ratatui::DefaultTerminal) {
        if self.state.spinner.is_some() {
            let state = &self.state;
            let vs = &mut self.sessions_visual_state;
            let _ = terminal.draw(|f| renderer::draw(state, vs, f));
        }
    }

    /// Restore registered sessions by creating daemon PTY sessions.
    /// Called once at startup — resumes Claude conversations from where they left off.
    /// Sessions that were stashed (status file = "idle") are skipped so they remain
    /// stashed across restarts instead of being automatically restarted.
    async fn restore_sessions(&mut self) {
        if !self.daemon.is_connected() {
            return;
        }

        let registry = crate::infrastructure::hooks::registry::load();
        if registry.is_empty() {
            return;
        }

        let existing: std::collections::HashSet<String> = match self.daemon.list_sessions().await {
            Ok(infos) => infos.into_iter().map(|i| i.session_id).collect(),
            Err(_) => std::collections::HashSet::new(),
        };

        // Read quit stash marker (takes + deletes) and status files.
        // The marker is the authoritative source for quit-stashed sessions because
        // dying Claude processes can overwrite status files with "waiting".
        let quit_stashed: std::collections::HashSet<String> =
            crate::infrastructure::hooks::take_quit_stashed()
                .into_iter()
                .collect();
        let statuses = crate::infrastructure::hooks::read_all_statuses(self.backend.base_dir());

        // Re-write "idle" for quit-stashed sessions whose status was overwritten
        // by a dying Claude hook (e.g. "waiting") so the session refresh sees them
        // as stashed.
        for id in &quit_stashed {
            let needs_repair = statuses
                .get(id.as_str())
                .map(|(s, _)| *s != crate::domain::entities::SessionStatus::Stashed)
                .unwrap_or(true);
            if needs_repair {
                tracing::info!("Repairing stash status for session {}", id);
                crate::infrastructure::hooks::write_session_status(
                    self.backend.base_dir(),
                    id,
                    "idle",
                );
            }
        }

        for (id, entry) in &registry {
            if existing.contains(id) {
                continue;
            }

            // All sessions start stashed on launch. The user explicitly
            // unstashes or attaches when they want a session running.
            // Ensure the status file says "idle" so the refresh pipeline
            // picks them up as stashed.
            let is_stashed = quit_stashed.contains(id)
                || statuses
                    .get(id.as_str())
                    .is_some_and(|(s, _)| *s == crate::domain::entities::SessionStatus::Stashed);
            if !is_stashed {
                tracing::info!(
                    "Marking session {} ({}) as stashed on startup",
                    id,
                    entry.name
                );
                crate::infrastructure::hooks::write_session_status(
                    self.backend.base_dir(),
                    id,
                    "idle",
                );
            }
        }
    }

    /// Ensure a session exists in the daemon (idempotent). Creates it if needed.
    async fn ensure_daemon_session(
        &mut self,
        session_id: &str,
        terminal: &mut ratatui::DefaultTerminal,
    ) {
        let resolved_cwd = self.state.store.find_session(session_id).and_then(|s| {
            s.cwd
                .clone()
                .filter(|c| !c.is_empty())
                .or_else(|| Some(s.project_path.clone()).filter(|p| !p.is_empty()))
        });
        let cmd_args = vec!["--resume".to_string(), session_id.to_string()];
        let size = terminal
            .size()
            .unwrap_or(ratatui::layout::Size::new(120, 40));

        let _ = self
            .daemon
            .create_session(
                session_id,
                &self.cli_runner.claude_bin,
                &cmd_args,
                resolved_cwd.as_deref(),
                None,
                size.width,
                size.height,
                HashMap::new(),
            )
            .await;
    }

    /// Refresh sessions: gather input, build session list (pure), merge in-place.
    /// Preserves the selected session by ID across the refresh.
    async fn refresh_daemon_sessions(&mut self) {
        use crate::infrastructure::session_refresh;

        // Save the selected session ID before refresh
        let selected_id = self
            .state
            .filtered_sessions()
            .get(self.state.table_state.selected)
            .map(|s| s.id.clone());

        // Save snapshot for subagent delta reload (before merge modifies the list)
        let previous_for_subagents = self.state.store.sessions.clone();

        // Gather all input (IO)
        let previous = &self.state.store.sessions;
        let registry = self.registry_cache.get();
        let mut input = session_refresh::gather_sync_input(&self.backend, previous, registry);
        let daemon_infos = session_refresh::gather_daemon_input(&mut self.daemon).await;
        input.daemon_infos = daemon_infos.clone();
        // Snapshot the latest wild-process list from the background scan
        // task. `borrow()` is non-blocking; the watch keeps only the
        // newest value so we never accumulate stale snapshots.
        input.wild_processes = self.wild_processes_rx.borrow().clone();
        // Clone the in-memory externally_opened set so build_session_list
        // can apply the External precedence rule purely.
        input.externally_opened = self.state.externally_opened.clone();

        // Build complete session list (pure, no IO)
        let new_sessions = session_refresh::build_session_list(&input);

        // Merge incoming into existing list (in-place, streak-based removal)
        let recently_removed_set: std::collections::HashSet<String> =
            self.recently_removed.keys().cloned().collect();
        session_refresh::merge_sessions(
            &mut self.state.store.sessions,
            new_sessions,
            &mut self.missing_streaks,
            &recently_removed_set,
        );

        // Auto-collapse sessions that transitioned from Active to Done/Fail
        let old_by_id: std::collections::HashMap<&str, &Session> = previous_for_subagents
            .iter()
            .map(|s| (s.id.as_str(), s))
            .collect();
        for session in &self.state.store.sessions {
            if self.state.expanded_sessions.contains(&session.id) {
                let was_active = old_by_id
                    .get(session.id.as_str())
                    .is_some_and(|old| old.status.section() == SessionSection::Active);
                if was_active && session.status.section() != SessionSection::Active {
                    self.state.expanded_sessions.remove(&session.id);
                }
            }
        }
        self.state
            .store
            .refresh_changed_subagents(&self.backend, &previous_for_subagents);
        self.state.store.rebuild_all_members();

        // Only redraw if sessions actually changed (PartialEq comparison)
        if session_refresh::sessions_changed(&previous_for_subagents, &self.state.store.sessions) {
            self.needs_redraw = true;
        }

        // Tick recently_removed counters: increment all, remove expired
        self.recently_removed
            .values_mut()
            .for_each(|v| *v = v.saturating_add(1));
        self.recently_removed
            .retain(|_, v| *v <= session_refresh::MISSING_STREAK_THRESHOLD);

        // Clean up externally_opened (uses App-owned state, stays here)
        if let Some(ref infos) = daemon_infos {
            cleanup_externally_opened(
                &mut self.state.externally_opened,
                &mut self.ext_open_times,
                infos,
                Duration::from_secs(15),
            );
        }

        // Restore selection: prefer pending_selection_id (from a restored
        // snapshot, not yet resolved) over the pre-refresh selected_id.
        let pending = self.state.pending_selection_id.take();
        let target_id = pending.as_ref().or(selected_id.as_ref());
        if let Some(id) = target_id {
            let sessions = self.state.filtered_sessions();
            if let Some(pos) = sessions.iter().position(|s| s.id == *id) {
                self.state.table_state.selected = pos;
            } else if pending.is_some() {
                // Pending selection not found yet — daemon sessions may still
                // be loading. Put it back for the next refresh cycle.
                self.state.pending_selection_id = pending;
            } else {
                // Session was removed — clamp to valid range
                let count = sessions.len();
                if count > 0 && self.state.table_state.selected >= count {
                    self.state.table_state.selected = count - 1;
                }
            }
        }
    }

    /// Auto-refresh conversation and subagents if viewing SessionDetail or SubagentDetail.
    /// Returns `true` if a refresh was performed (caller should redraw).
    fn auto_refresh_conversation(&mut self) -> bool {
        use crate::adapters::views::ViewKind;
        match self.state.current_view() {
            ViewKind::SessionDetail => {
                if let Some(session_id) = self.state.current_session().map(|s| s.to_string()) {
                    if let Some(session) = self.state.store.find_session(&session_id).cloned() {
                        let _ = self.state.store.load_conversation(
                            &self.backend,
                            &session.project,
                            &session.id,
                        );
                        // Also refresh subagents so the detail view stays current
                        let _ = self.state.store.refresh_subagents(
                            &self.backend,
                            &session.project,
                            &session.id,
                        );
                        return true;
                    } else {
                        // Session no longer exists — clear stale data
                        self.state.store.conversation.clear();
                        self.state.store.conversation_loaded = true;
                        self.state.store.subagents.clear();
                        return true;
                    }
                }
                false
            }
            ViewKind::SubagentDetail => {
                if let Some(agent_id) = self
                    .state
                    .nav
                    .current()
                    .context
                    .as_deref()
                    .map(|s| s.to_string())
                {
                    if let Some(sa) = self.state.store.find_subagent(&agent_id).cloned() {
                        let _ = self.state.store.load_subagent_conversation(
                            &self.backend,
                            &sa.project,
                            &sa.parent_session_id,
                            &sa.id,
                        );
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Execute effects — translates abstract Effects into real IO.
    async fn execute_effects(
        &mut self,
        effects: Vec<Effect>,
        terminal: &mut ratatui::DefaultTerminal,
        events: &mut EventLoop,
    ) -> bool {
        let mut queue = VecDeque::from(effects);

        while let Some(effect) = queue.pop_front() {
            match effect {
                Effect::Quit => {
                    // Persist UI state for next startup
                    let snapshot = self.state.snapshot();
                    let path = self.backend.base_dir().join("clash/ui_state.json");
                    if let Ok(json) = serde_json::to_string_pretty(&snapshot) {
                        let _ =
                            crate::infrastructure::fs::atomic::write_atomic(&path, json.as_bytes());
                    }
                    return true;
                }

                // ── Domain persistence → filesystem IO ──────────
                Effect::PersistTask { team, task } => {
                    if let Err(e) = self.backend.write_task(&team, &task) {
                        self.state.toast = Some(format!("Write failed: {}", e));
                    }
                }
                Effect::RemoveTeam { name } => {
                    if let Err(e) = self.backend.delete_team(&name) {
                        self.state.toast = Some(format!("Delete failed: {}", e));
                    }
                }
                // ── CLI commands → subprocess ───────────────────
                Effect::RunCli {
                    command,
                    on_complete,
                } => {
                    let args = commands::to_args(&command);
                    let result = self.cli_runner.run(&args).await;
                    let (success, output) = match result {
                        Ok(out) => (
                            out.success,
                            if out.success { out.stdout } else { out.stderr },
                        ),
                        Err(e) => (false, e.to_string()),
                    };
                    let follow_up_effects = reducer::reduce(
                        &mut self.state,
                        Action::CliResult {
                            success,
                            output,
                            follow_up: Box::new(on_complete),
                        },
                    );
                    for (i, e) in follow_up_effects.into_iter().enumerate() {
                        queue.insert(i, e);
                    }
                }
                // ── Data refresh ────────────────────────────────
                Effect::RefreshAll => {
                    if let Err(e) = self.state.store.refresh_all(&self.backend) {
                        tracing::warn!("Refresh failed: {}", e);
                    }
                }
                Effect::RefreshSessions => {
                    self.refresh_daemon_sessions().await;
                }
                Effect::RefreshTeamTasks { team } => {
                    let _ = self.state.store.refresh_tasks(&self.backend, &team);
                }
                Effect::RefreshSubagents {
                    project,
                    session_id,
                } => {
                    let _ =
                        self.state
                            .store
                            .refresh_subagents(&self.backend, &project, &session_id);
                }
                Effect::LoadRepoConfig { session_id } => {
                    if let Some(session) = self
                        .state
                        .store
                        .sessions
                        .iter_mut()
                        .find(|s| s.id == session_id)
                    {
                        // Skip if already loaded (cache)
                        if session.repo_config.is_none() {
                            let dir = session
                                .cwd
                                .as_deref()
                                .or(Some(&session.project_path))
                                .filter(|p| !p.is_empty());
                            if let Some(d) = dir {
                                session.repo_config =
                                    Some(crate::infrastructure::fs::repo_config::load_repo_config(
                                        std::path::Path::new(d),
                                    ));
                            }
                        }
                    }
                }
                Effect::LoadDiff { session_id } => {
                    let dir = self.state.store.find_session(&session_id).and_then(|s| {
                        s.cwd
                            .as_deref()
                            .or(Some(&s.project_path))
                            .filter(|p| !p.is_empty())
                            .map(|p| p.to_string())
                    });
                    if let Some(dir) = dir {
                        let start = Instant::now();
                        let output = tokio::process::Command::new("git")
                            .args(["diff", "HEAD"])
                            .current_dir(&dir)
                            .output()
                            .await;
                        let elapsed = start.elapsed();
                        tracing::debug!("git diff HEAD in {} took {:?}", dir, elapsed);
                        match output {
                            Ok(out) if out.status.success() => {
                                let raw = String::from_utf8_lossy(&out.stdout);
                                self.state.diff.lines =
                                    crate::infrastructure::tui::widgets::diff_widget::parse_diff_lines(&raw);
                            }
                            Ok(out) => {
                                let err = String::from_utf8_lossy(&out.stderr);
                                self.state.diff.lines = vec![crate::application::state::DiffLine {
                                    kind: crate::application::state::DiffLineKind::Context,
                                    content: format!("Error: {}", err.trim()),
                                }];
                            }
                            Err(e) => {
                                self.state.diff.lines = vec![crate::application::state::DiffLine {
                                    kind: crate::application::state::DiffLineKind::Context,
                                    content: format!("Failed to run git: {}", e),
                                }];
                            }
                        }
                    } else {
                        self.state.diff.lines = vec![crate::application::state::DiffLine {
                            kind: crate::application::state::DiffLineKind::Context,
                            content: "No project directory for this session".to_string(),
                        }];
                    }
                    self.state.diff.files =
                        crate::infrastructure::tui::widgets::diff_widget::extract_files(
                            &self.state.diff.lines,
                        );
                    self.state.diff.selected_file = 0;
                    self.state.diff.file_scroll = 0;
                    self.state.diff.loaded = true;
                    self.state.diff.loading = false;
                }
                Effect::LoadConversation {
                    project,
                    session_id,
                } => {
                    if let Err(e) =
                        self.state
                            .store
                            .load_conversation(&self.backend, &project, &session_id)
                    {
                        tracing::warn!("Failed to load conversation: {}", e);
                        self.state.store.conversation_loaded = true;
                    }
                }
                Effect::LoadSubagentConversation {
                    project,
                    session_id,
                    agent_id,
                } => {
                    let _ = self.state.store.load_subagent_conversation(
                        &self.backend,
                        &project,
                        &session_id,
                        &agent_id,
                    );
                }
                Effect::RegisterSession {
                    session_id,
                    name,
                    cwd,
                    source_branch,
                } => {
                    crate::infrastructure::hooks::registry::register(
                        &session_id,
                        &name,
                        &cwd,
                        source_branch.as_deref(),
                    );
                }
                Effect::UnregisterSession { session_id } => {
                    crate::infrastructure::hooks::registry::unregister(&session_id);
                    // Prevent the merge from re-adding this session on the next cycle
                    self.recently_removed.insert(session_id, 0);
                }
                Effect::RenameSession { session_id, name } => {
                    crate::infrastructure::hooks::registry::rename(&session_id, &name);
                    let cwd = self
                        .state
                        .store
                        .sessions
                        .iter()
                        .find(|s| s.id == session_id)
                        .and_then(|s| s.cwd.as_deref());
                    crate::infrastructure::hooks::save_session_name(
                        self.backend.base_dir(),
                        &session_id,
                        &name,
                        cwd,
                    );
                }
                Effect::ClearSessionRegistry => {
                    // Mark all current sessions as recently removed before clearing
                    for session in &self.state.store.sessions {
                        self.recently_removed.insert(session.id.clone(), 0);
                    }
                    crate::infrastructure::hooks::registry::clear();
                }
                Effect::MarkSessionStarting { session_id } => {
                    crate::infrastructure::hooks::write_session_status(
                        self.backend.base_dir(),
                        &session_id,
                        "starting",
                    );
                }
                Effect::MarkSessionIdle { session_id } => {
                    crate::infrastructure::hooks::write_session_status(
                        self.backend.base_dir(),
                        &session_id,
                        "idle",
                    );
                }
                Effect::WriteQuitStash { session_ids } => {
                    crate::infrastructure::hooks::write_quit_stashed(&session_ids);
                }
                Effect::MarkAllSessionsIdle => {
                    for session in &self.state.store.sessions {
                        crate::infrastructure::hooks::write_session_status(
                            self.backend.base_dir(),
                            &session.id,
                            "idle",
                        );
                    }
                }

                // ── Session attach (daemon PTY + raw passthrough) ──
                //
                // Claude runs in a daemon PTY. We leave the alternate screen
                // and pipe daemon output directly to stdout for 100% fidelity.
                // Raw /dev/tty reader forwards input to daemon; Ctrl+B detaches.
                Effect::DaemonAttach {
                    session_id,
                    args,
                    cwd,
                    name,
                } => {
                    // Force a draw so the dark busy overlay is visible while
                    // the daemon session is being created/connected.
                    {
                        let state = &self.state;
                        let vs = &mut self.sessions_visual_state;
                        let _ = terminal.draw(|f| renderer::draw(state, vs, f));
                    }

                    // Save session name if provided (for new sessions)
                    if let Some(ref n) = name {
                        crate::infrastructure::hooks::save_session_name(
                            self.backend.base_dir(),
                            &session_id,
                            n,
                            cwd.as_deref(),
                        );
                    }

                    if !self.daemon.is_connected() {
                        self.state.toast = Some("Daemon not connected".to_string());
                        self.state.input_mode = InputMode::Normal;
                        self.state.attached_session = None;
                        self.state.spinner = None;
                        continue;
                    }

                    // Resolve cwd from session data: prefer the session's
                    // original cwd (from the registry), fall back to project_path.
                    let resolved_cwd = cwd.or_else(|| {
                        self.state.store.find_session(&session_id).and_then(|s| {
                            s.cwd
                                .clone()
                                .filter(|c| !c.is_empty())
                                .or_else(|| Some(s.project_path.clone()).filter(|p| !p.is_empty()))
                        })
                    });

                    // Build CLI args: provided args for new sessions,
                    // or --resume for existing sessions
                    let cmd_args = if args.is_empty() {
                        vec!["--resume".to_string(), session_id.clone()]
                    } else {
                        args
                    };

                    // PTY size = full terminal (Claude owns the whole screen)
                    let size = terminal
                        .size()
                        .unwrap_or(ratatui::layout::Size::new(120, 40));
                    let cols = size.width;
                    let rows = size.height;

                    // Create a daemon PTY session (or resize existing)
                    if let Err(e) = self
                        .daemon
                        .create_session(
                            &session_id,
                            &self.cli_runner.claude_bin,
                            &cmd_args,
                            resolved_cwd.as_deref(),
                            name,
                            cols,
                            rows,
                            HashMap::new(),
                        )
                        .await
                    {
                        tracing::debug!("Create session result: {}", e);
                        let _ = self.daemon.resize(&session_id, cols, rows).await;
                    }

                    // Attach to daemon output stream. The server replays
                    // the session's output history so the client can
                    // paint Claude's last screen state — including
                    // background colors and SGR — before live output
                    // begins. The size-toggle in attach_loop also fires
                    // SIGWINCH so Claude repaints on top.
                    if let Err(e) = self.daemon.attach(&session_id).await {
                        self.state.toast = Some(format!("Attach failed: {}", e));
                        self.state.input_mode = InputMode::Normal;
                        self.state.attached_session = None;
                        self.state.spinner = None;
                        continue;
                    }

                    // State is set to Attached — the outer loop in run() will
                    // detect this, break out of the event loop, drop crossterm
                    // entirely, and run the standalone attach loop.
                    tracing::info!(
                        "Attached to daemon session {} ({}x{})",
                        session_id,
                        cols,
                        rows
                    );
                }
                Effect::DaemonStart {
                    session_id,
                    args,
                    cwd,
                    name,
                } => {
                    // Start a session in the daemon without entering passthrough.
                    if let Some(ref n) = name {
                        crate::infrastructure::hooks::save_session_name(
                            self.backend.base_dir(),
                            &session_id,
                            n,
                            cwd.as_deref(),
                        );
                    }

                    // Clear the stale "idle" hook status so the daemon's
                    // Starting/Running status can take effect in reconciliation.
                    crate::infrastructure::hooks::write_session_status(
                        self.backend.base_dir(),
                        &session_id,
                        "starting",
                    );

                    if !self.daemon.is_connected() {
                        self.state.toast = Some("Daemon not connected".to_string());
                        continue;
                    }

                    let resolved_cwd = cwd.or_else(|| {
                        self.state.store.find_session(&session_id).and_then(|s| {
                            s.cwd
                                .clone()
                                .filter(|c| !c.is_empty())
                                .or_else(|| Some(s.project_path.clone()).filter(|p| !p.is_empty()))
                        })
                    });

                    let cmd_args = if args.is_empty() {
                        vec!["--resume".to_string(), session_id.clone()]
                    } else {
                        args
                    };

                    let size = terminal
                        .size()
                        .unwrap_or(ratatui::layout::Size::new(120, 40));
                    let cols = size.width;
                    let rows = size.height;

                    // Create or resume session; fall back to resize if it already exists
                    if let Err(e) = self
                        .daemon
                        .create_session(
                            &session_id,
                            &self.cli_runner.claude_bin,
                            &cmd_args,
                            resolved_cwd.as_deref(),
                            name,
                            cols,
                            rows,
                            HashMap::new(),
                        )
                        .await
                    {
                        tracing::debug!("Background start: create_session returned: {}", e);
                        let _ = self.daemon.resize(&session_id, cols, rows).await;
                    }

                    // Update in-memory state so the UI shows Starting immediately
                    if let Some(session) = self
                        .state
                        .store
                        .sessions
                        .iter_mut()
                        .find(|s| s.id == session_id)
                    {
                        session.status = crate::domain::entities::SessionStatus::Starting;
                        session.is_running = true;
                    }

                    self.state.toast = Some("Session restarted".to_string());
                    tracing::info!("Started daemon session {} in background", session_id);
                }
                Effect::AttachInNewWindow { session_id } => {
                    if !self.daemon.is_connected() {
                        self.state.toast = Some("Daemon not connected".to_string());
                        continue;
                    }

                    self.ensure_daemon_session(&session_id, terminal).await;

                    let term = std::env::var("TERM_PROGRAM").ok();
                    let in_tmux = std::env::var("TMUX").is_ok();
                    let (cols, rows) = crossterm::terminal::size().unwrap_or((120, 40));
                    match crate::infrastructure::windowing::terminal_spawn::open_session(
                        &session_id,
                        term.as_deref(),
                        in_tmux,
                        cols,
                        rows,
                    ) {
                        Ok(mode) => {
                            self.state.externally_opened.insert(session_id.clone());
                            self.ext_open_times
                                .insert(session_id.clone(), Instant::now());
                            let label = match mode {
                                crate::infrastructure::windowing::terminal_spawn::OpenMode::Pane => "pane",
                                crate::infrastructure::windowing::terminal_spawn::OpenMode::Tab => "tab",
                                crate::infrastructure::windowing::terminal_spawn::OpenMode::Window => "window",
                            };
                            self.state.toast = Some(format!("Opened in new {}", label));
                        }
                        Err(e) => {
                            self.state.toast = Some(format!("Failed: {}", e));
                        }
                    }
                    self.state.spinner = None;
                }
                Effect::AttachBatchInNewWindows { session_ids } => {
                    if !self.daemon.is_connected() {
                        self.state.toast = Some("Daemon not connected".to_string());
                        continue;
                    }

                    // Phase 1: ensure all sessions exist in daemon
                    for id in &session_ids {
                        self.ensure_daemon_session(id, terminal).await;
                    }

                    // Phase 2: spawn with smart pane/tab layout
                    let term = std::env::var("TERM_PROGRAM").ok();
                    let in_tmux = std::env::var("TMUX").is_ok();
                    let (cols, rows) = crossterm::terminal::size().unwrap_or((120, 40));
                    match crate::infrastructure::windowing::terminal_spawn::open_batch(
                        &session_ids,
                        term.as_deref(),
                        in_tmux,
                        cols,
                        rows,
                    ) {
                        Ok(result) => {
                            // Track all opened sessions
                            let now = Instant::now();
                            for id in &session_ids {
                                self.state.externally_opened.insert(id.clone());
                                self.ext_open_times.insert(id.clone(), now);
                            }
                            let msg = match (result.panes_opened, result.tabs_opened) {
                                (p, 0) => format!("Opened {} pane(s)", p),
                                (0, t) => format!("Opened {} tab(s)", t),
                                (p, t) => format!("Opened {} pane(s) + {} tab(s)", p, t),
                            };
                            self.state.toast = Some(msg);
                        }
                        Err(e) => {
                            self.state.toast = Some(format!("Failed: {}", e));
                        }
                    }
                    self.state.spinner = None;
                }
                Effect::CreateWorktreeAndAttach {
                    source_session_id,
                    cwd,
                    new_session_id,
                    name,
                } => {
                    // Resolve project_path and git_branch
                    let (project_path, git_branch) = if let Some(ref sid) = source_session_id {
                        match self.state.store.find_session(sid) {
                            Some(s) => (s.project_path.clone(), s.git_branch.clone()),
                            None => {
                                self.state.toast = Some("Source session not found".to_string());
                                self.state.input_mode = InputMode::Normal;
                                self.state.attached_session = None;
                                self.state.spinner = None;
                                continue;
                            }
                        }
                    } else if let Some(ref dir) = cwd {
                        let branch = tokio::process::Command::new("git")
                            .args(["rev-parse", "--abbrev-ref", "HEAD"])
                            .current_dir(dir)
                            .output()
                            .await
                            .ok()
                            .and_then(|o| {
                                if o.status.success() {
                                    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_default();
                        (dir.clone(), branch)
                    } else {
                        self.state.toast = Some("No project path".to_string());
                        self.state.input_mode = InputMode::Normal;
                        self.state.attached_session = None;
                        self.state.spinner = None;
                        continue;
                    };

                    // Compute worktree path: <project_path>/../<project_name>-worktrees/<name>/
                    let project_dir = std::path::Path::new(&project_path);
                    let project_name = project_dir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("project");
                    let worktree_base = project_dir
                        .parent()
                        .unwrap_or(project_dir)
                        .join(format!("{}-worktrees", project_name));
                    let worktree_path = worktree_base.join(&name);

                    // Create base dir
                    if let Err(e) = std::fs::create_dir_all(&worktree_base) {
                        self.state.toast = Some(format!("Failed to create worktree dir: {}", e));
                        self.state.input_mode = InputMode::Normal;
                        self.state.attached_session = None;
                        self.state.spinner = None;
                        continue;
                    }

                    // Run git worktree add
                    let mut git_args = vec![
                        "worktree".to_string(),
                        "add".to_string(),
                        worktree_path.to_string_lossy().to_string(),
                        "-b".to_string(),
                        name.clone(),
                    ];
                    if !git_branch.is_empty() {
                        git_args.push(git_branch.clone());
                    }
                    let git_result = tokio::process::Command::new("git")
                        .args(&git_args)
                        .current_dir(&project_path)
                        .output()
                        .await;

                    match git_result {
                        Ok(output) if output.status.success() => {
                            let wt_str = worktree_path.to_string_lossy().to_string();
                            // Register session with the source branch
                            let src_branch = if git_branch.is_empty() {
                                None
                            } else {
                                Some(git_branch.as_str())
                            };
                            crate::infrastructure::hooks::registry::register(
                                &new_session_id,
                                &name,
                                &wt_str,
                                src_branch,
                            );
                            // Save session name
                            crate::infrastructure::hooks::save_session_name(
                                self.backend.base_dir(),
                                &new_session_id,
                                &name,
                                Some(&wt_str),
                            );

                            // Create daemon session in the worktree
                            if !self.daemon.is_connected() {
                                self.state.toast = Some("Daemon not connected".to_string());
                                self.state.input_mode = InputMode::Normal;
                                self.state.attached_session = None;
                                self.state.spinner = None;
                                continue;
                            }

                            let cmd_args = vec!["--session-id".to_string(), new_session_id.clone()];
                            let size = terminal
                                .size()
                                .unwrap_or(ratatui::layout::Size::new(120, 40));

                            if let Err(e) = self
                                .daemon
                                .create_session(
                                    &new_session_id,
                                    &self.cli_runner.claude_bin,
                                    &cmd_args,
                                    Some(&wt_str),
                                    Some(name.clone()),
                                    size.width,
                                    size.height,
                                    HashMap::new(),
                                )
                                .await
                            {
                                tracing::debug!("Create worktree session: {}", e);
                                let _ = self
                                    .daemon
                                    .resize(&new_session_id, size.width, size.height)
                                    .await;
                            }

                            // Attach to daemon output stream — server
                            // replays history so the client paints
                            // Claude's last screen state.
                            if let Err(e) = self.daemon.attach(&new_session_id).await {
                                self.state.toast = Some(format!("Attach failed: {}", e));
                                self.state.input_mode = InputMode::Normal;
                                self.state.attached_session = None;
                                self.state.spinner = None;
                                continue;
                            }

                            tracing::info!(
                                "Created worktree and attached: {} at {}",
                                new_session_id,
                                wt_str
                            );
                        }
                        Ok(output) => {
                            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                            self.state.toast =
                                Some(format!("git worktree failed: {}", stderr.trim()));
                            self.state.input_mode = InputMode::Normal;
                            self.state.attached_session = None;
                            self.state.spinner = None;
                        }
                        Err(e) => {
                            self.state.toast = Some(format!("Failed to run git: {}", e));
                            self.state.input_mode = InputMode::Normal;
                            self.state.attached_session = None;
                            self.state.spinner = None;
                        }
                    }
                }
                Effect::DaemonKill { session_id } => {
                    if self.daemon.is_connected() {
                        let _ = self.daemon.kill_session(&session_id).await;
                    }
                }
                Effect::TerminateProcess {
                    session_id,
                    worktree,
                } => {
                    let base_dir = self.backend.base_dir().to_path_buf();
                    tokio::spawn(async move {
                        terminate_claude_process(&session_id).await;
                        if let Some(wt) = worktree {
                            kill_tmux_session(&wt).await;
                            remove_git_worktree(&wt).await;
                        }
                        // Re-write "idle" after the process has died, so that
                        // any Stop hook the dying Claude fires ("waiting") is
                        // overwritten and the session doesn't get stuck in Waiting.
                        crate::infrastructure::hooks::write_session_status(
                            &base_dir,
                            &session_id,
                            "idle",
                        );
                    });
                }
                Effect::TerminateAllProcesses => {
                    let base_dir = self.backend.base_dir().to_path_buf();
                    let sessions: Vec<(String, Option<String>)> = self
                        .state
                        .store
                        .sessions
                        .iter()
                        .map(|s| (s.id.clone(), s.worktree.clone()))
                        .collect();
                    tokio::spawn(async move {
                        for (id, worktree) in sessions {
                            terminate_claude_process(&id).await;
                            if let Some(wt) = worktree {
                                kill_tmux_session(&wt).await;
                            }
                            // Re-write "idle" after process dies, so any Stop hook
                            // the dying Claude fires ("waiting") is overwritten.
                            crate::infrastructure::hooks::write_session_status(
                                &base_dir, &id, "idle",
                            );
                        }
                    });
                }
                Effect::DaemonKillAll => {
                    if self.daemon.is_connected() {
                        if let Ok(infos) = self.daemon.list_sessions().await {
                            for info in infos {
                                let _ = self.daemon.kill_session(&info.session_id).await;
                            }
                        }
                    }
                }

                // ── IDE effects ────────────────────────────────
                Effect::DetectIdes { project_dir } => {
                    tracing::debug!("DetectIdes effect: project_dir={}", project_dir);
                    let items = crate::infrastructure::ide::detect_ides(&self.config.ides);
                    tracing::debug!("DetectIdes: found {} IDEs", items.len());
                    if items.is_empty() {
                        self.state.toast = Some("No IDEs detected".to_string());
                    } else {
                        let follow_up_effects = reducer::reduce(
                            &mut self.state,
                            Action::Ui(crate::application::actions::UiAction::ShowPicker {
                                title: "Open in IDE".to_string(),
                                items,
                                on_select: crate::application::state::PickerAction::OpenInIde {
                                    project_dir,
                                },
                            }),
                        );
                        for (i, e) in follow_up_effects.into_iter().enumerate() {
                            queue.insert(i, e);
                        }
                    }
                }
                Effect::OpenIde {
                    command,
                    project_dir,
                    terminal,
                } => {
                    if terminal {
                        let term = std::env::var("TERM_PROGRAM").ok();
                        let in_tmux = std::env::var("TMUX").is_ok();
                        let (cols, rows) = crossterm::terminal::size().unwrap_or((120, 40));
                        match crate::infrastructure::windowing::terminal_spawn::open_command(
                            &command,
                            &[&project_dir],
                            term.as_deref(),
                            in_tmux,
                            cols,
                            rows,
                        ) {
                            Ok(mode) => {
                                let label = match mode {
                                    crate::infrastructure::windowing::terminal_spawn::OpenMode::Pane => "pane",
                                    crate::infrastructure::windowing::terminal_spawn::OpenMode::Tab => "tab",
                                    crate::infrastructure::windowing::terminal_spawn::OpenMode::Window => "window",
                                };
                                self.state.toast =
                                    Some(format!("Opened {} in new {}", command, label));
                            }
                            Err(e) => {
                                self.state.toast = Some(format!("Failed: {}", e));
                            }
                        }
                    } else {
                        match crate::infrastructure::ide::open_ide(&command, &project_dir) {
                            Ok(()) => {
                                // Toast already set by reducer
                            }
                            Err(e) => {
                                self.state.toast = Some(e);
                            }
                        }
                    }
                }

                // ── UI state ────────────────────────────────────
                Effect::ShowSpinner(msg) => {
                    self.state.spinner = Some(msg);
                }
                Effect::WakeWildScan => {
                    // Nudge the background scan task — it will rescan
                    // immediately instead of waiting for its next tick.
                    self.wild_scan_wake.notify_one();
                }
                Effect::TakeoverWildSession {
                    session_id,
                    pid,
                    cwd,
                } => {
                    use crate::infrastructure::process_scan::{
                        should_signal, LiveProcessProbe, ProcessProbe, SignalDecision,
                    };
                    let probe = LiveProcessProbe;
                    match should_signal(pid, &probe) {
                        SignalDecision::Allow => {
                            // SIGTERM, then poll up to 2s for exit, then
                            // SIGKILL if it's still alive. Poll cadence
                            // is 100ms — keeps the perceived latency low
                            // for cooperative quitters while bounding the
                            // worst case.
                            let pid_i = pid as i32;
                            let kill = |sig: libc::c_int| unsafe {
                                libc::kill(pid_i, sig);
                            };
                            kill(libc::SIGTERM);
                            let deadline =
                                std::time::Instant::now() + std::time::Duration::from_secs(2);
                            let mut exited = false;
                            while std::time::Instant::now() < deadline {
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                                if !probe.is_alive(pid) {
                                    exited = true;
                                    break;
                                }
                            }
                            if !exited {
                                kill(libc::SIGKILL);
                                // Give the kernel one tick to reap so
                                // --resume doesn't race the lock.
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                            // Re-spawn under the daemon as --resume <id>.
                            let size = terminal
                                .size()
                                .unwrap_or(ratatui::layout::Size::new(120, 40));
                            let cmd_args = vec!["--resume".to_string(), session_id.clone()];
                            let resolved_cwd = if cwd.is_empty() {
                                None
                            } else {
                                Some(cwd.as_str())
                            };
                            if let Err(e) = self
                                .daemon
                                .create_session(
                                    &session_id,
                                    &self.cli_runner.claude_bin,
                                    &cmd_args,
                                    resolved_cwd,
                                    None,
                                    size.width,
                                    size.height,
                                    HashMap::new(),
                                )
                                .await
                            {
                                tracing::warn!("Takeover create_session failed: {}", e);
                                self.state.toast = Some(format!("Takeover failed: {}", e));
                            } else {
                                let short = crate::adapters::format::short_id(&session_id, 8);
                                self.state.toast =
                                    Some(format!("Took over wild session ({})", short));
                            }
                        }
                        SignalDecision::ProcessExited => {
                            self.state.toast =
                                Some("Wild process is no longer running".to_string());
                        }
                        SignalDecision::CmdlineChanged => {
                            self.state.toast = Some(
                                "PID was reused by another process — refusing takeover".to_string(),
                            );
                        }
                    }
                }
                Effect::PerformUpdate => {
                    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                    events.set_update_rx(rx);
                    self.state.update_progress =
                        Some(crate::application::state::UpdatePhase::Checking);
                    tokio::spawn(async move {
                        crate::infrastructure::update::perform_update(tx).await;
                    });
                }

                // ── Preset effects ────────────────────────────────
                Effect::LoadPresets { project_dir } => {
                    let global_config_dir = dirs::config_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("."))
                        .join("clash");
                    self.state.store.presets = crate::infrastructure::fs::presets::load_presets(
                        std::path::Path::new(&project_dir),
                        &global_config_dir,
                    );
                    tracing::debug!(
                        "Loaded {} presets from {}",
                        self.state.store.presets.len(),
                        project_dir
                    );
                }
                Effect::RunSetupScripts {
                    session_id,
                    scripts,
                    cwd,
                } => {
                    for script in &scripts {
                        tracing::debug!("Running setup script: {} in {}", script, cwd);
                        let result = tokio::time::timeout(
                            Duration::from_secs(30),
                            tokio::process::Command::new("sh")
                                .args(["-c", script])
                                .current_dir(&cwd)
                                .env("CLASH_ROOT_PATH", &cwd)
                                .env("CLASH_SESSION_ID", &session_id)
                                .output(),
                        )
                        .await;
                        match result {
                            Ok(Ok(out)) if out.status.success() => {
                                tracing::debug!("Setup script succeeded: {}", script);
                            }
                            Ok(Ok(out)) => {
                                let err = String::from_utf8_lossy(&out.stderr);
                                self.state.toast =
                                    Some(format!("Setup script failed: {}", err.trim()));
                                tracing::warn!("Setup script failed: {} — {}", script, err.trim());
                                break;
                            }
                            Ok(Err(e)) => {
                                self.state.toast = Some(format!("Setup script error: {}", e));
                                tracing::warn!("Setup script error: {} — {}", script, e);
                                break;
                            }
                            Err(_) => {
                                self.state.toast =
                                    Some(format!("Setup script timed out: {}", script));
                                tracing::warn!("Setup script timed out (30s): {}", script);
                                break;
                            }
                        }
                    }
                    if self.state.toast.is_none() && !scripts.is_empty() {
                        self.state.toast = Some("Setup scripts completed".to_string());
                    }
                }
                Effect::RunTeardownScripts {
                    scripts,
                    cwd,
                    on_complete,
                } => {
                    for script in &scripts {
                        tracing::debug!("Running teardown script: {} in {}", script, cwd);
                        let result = tokio::time::timeout(
                            Duration::from_secs(30),
                            tokio::process::Command::new("sh")
                                .args(["-c", script])
                                .current_dir(&cwd)
                                .output(),
                        )
                        .await;
                        match result {
                            Ok(Ok(out)) if out.status.success() => {
                                tracing::debug!("Teardown script succeeded: {}", script);
                            }
                            Ok(Ok(out)) => {
                                let err = String::from_utf8_lossy(&out.stderr);
                                tracing::warn!(
                                    "Teardown script failed: {} — {}",
                                    script,
                                    err.trim()
                                );
                            }
                            Ok(Err(e)) => {
                                tracing::warn!("Teardown script error: {} — {}", script, e);
                            }
                            Err(_) => {
                                tracing::warn!("Teardown script timed out (30s): {}", script);
                            }
                        }
                    }
                    // Dispatch the follow-up action (e.g. DropSessionAfterTeardown)
                    let follow_up_effects = reducer::reduce(&mut self.state, on_complete);
                    for (i, e) in follow_up_effects.into_iter().enumerate() {
                        queue.insert(i, e);
                    }
                }
            }
        }
        // Clear spinner after all effects have executed.
        // Exceptions: during graceful shutdown the spinner must persist until
        // quit, and during attach the spinner must persist until
        // buffer_attach_history completes (so the busy overlay stays visible).
        if self.state.shutting_down.is_none() && self.state.input_mode != InputMode::Attached {
            if self.state.pending_toast.is_some() {
                // Keep spinner alive briefly (~500ms) so the busy overlay is
                // visible for transient operations (stash, unstash, etc.).
                self.pending_spinner_clear = Some(self.state.tick.wrapping_add(50));
            } else {
                self.state.spinner = None;
            }
        }
        false
    }
}

/// Gracefully stop external Claude Code processes for a session.
async fn terminate_claude_process(session_id: &str) {
    let output = tokio::process::Command::new("pgrep")
        .args(["-f", &format!("claude.*{}", session_id)])
        .output()
        .await;

    if let Ok(output) = output {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Ok(pid) = line.trim().parse::<u32>() {
                    let our_pid = std::process::id();
                    if pid == our_pid {
                        continue;
                    }
                    tracing::info!(
                        "Sending SIGTERM to Claude process PID {} for session {}",
                        pid,
                        session_id
                    );
                    let _ = tokio::process::Command::new("kill")
                        .args(["-TERM", &pid.to_string()])
                        .output()
                        .await;
                }
            }

            // Escalate to SIGKILL after 5 seconds
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            for line in stdout.lines() {
                if let Ok(pid) = line.trim().parse::<u32>() {
                    let our_pid = std::process::id();
                    if pid == our_pid {
                        continue;
                    }
                    let _ = tokio::process::Command::new("kill")
                        .args(["-KILL", &pid.to_string()])
                        .output()
                        .await;
                }
            }
        }
    }

    let _ = tokio::process::Command::new("pkill")
        .args(["-TERM", "-f", &format!("claude.*{}", session_id)])
        .output()
        .await;
}

/// Remove a git worktree if the path is one (`.git` is a file, not a directory).
async fn remove_git_worktree(worktree_path: &str) {
    let git_file = std::path::Path::new(worktree_path).join(".git");
    if !git_file.is_file() {
        return; // not a worktree
    }
    let result = tokio::process::Command::new("git")
        .args(["worktree", "remove", "--force", worktree_path])
        .output()
        .await;
    match result {
        Ok(output) if output.status.success() => {
            tracing::info!("Removed git worktree '{}'", worktree_path);
        }
        Ok(output) => {
            tracing::debug!(
                "git worktree remove failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Err(e) => {
            tracing::debug!("Failed to run git worktree remove: {}", e);
        }
    }
}

/// Kill a tmux session by worktree name.
async fn kill_tmux_session(worktree: &str) {
    // Claude creates tmux sessions named after the worktree
    let result = tokio::process::Command::new("tmux")
        .args(["kill-session", "-t", worktree])
        .output()
        .await;
    match result {
        Ok(output) if output.status.success() => {
            tracing::info!("Killed tmux session '{}'", worktree);
        }
        Ok(_) => {
            tracing::debug!("No tmux session '{}' found", worktree);
        }
        Err(e) => {
            tracing::debug!("tmux not available: {}", e);
        }
    }
}

/// Remove externally-opened entries that are no longer attached AND past the grace period.
///
/// Newly opened sessions get a grace window to allow the `clash attach` process
/// time to connect before cleanup considers them stale.
fn cleanup_externally_opened(
    externally_opened: &mut std::collections::HashSet<String>,
    open_times: &mut HashMap<String, Instant>,
    infos: &[crate::infrastructure::daemon::protocol::SessionInfo],
    grace: Duration,
) {
    let now = Instant::now();
    externally_opened.retain(|id| {
        let is_attached = infos
            .iter()
            .any(|i| i.session_id == *id && i.attached_clients > 0);
        let within_grace = open_times
            .get(id)
            .map(|t| now.duration_since(*t) < grace)
            .unwrap_or(false);
        let keep = is_attached || within_grace;
        if !keep {
            open_times.remove(id);
        }
        keep
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::daemon::protocol::SessionInfo;
    use std::collections::HashSet;

    fn make_info(session_id: &str, attached: usize) -> SessionInfo {
        SessionInfo {
            session_id: session_id.to_string(),
            pid: 1,
            is_alive: true,
            attached_clients: attached,
            created_at: 0,
            status: String::new(),
            cwd: String::new(),
            name: None,
        }
    }

    #[test]
    fn cleanup_within_grace_no_attachment_kept() {
        let mut opened = HashSet::from(["s1".to_string()]);
        let mut times = HashMap::from([("s1".to_string(), Instant::now())]);
        let infos = vec![make_info("s1", 0)];

        cleanup_externally_opened(&mut opened, &mut times, &infos, Duration::from_secs(15));

        assert!(opened.contains("s1"));
        assert!(times.contains_key("s1"));
    }

    #[test]
    fn cleanup_past_grace_no_attachment_removed() {
        let mut opened = HashSet::from(["s1".to_string()]);
        let past = Instant::now() - Duration::from_secs(30);
        let mut times = HashMap::from([("s1".to_string(), past)]);
        let infos = vec![make_info("s1", 0)];

        cleanup_externally_opened(&mut opened, &mut times, &infos, Duration::from_secs(15));

        assert!(!opened.contains("s1"));
        assert!(!times.contains_key("s1"));
    }

    #[test]
    fn cleanup_past_grace_with_attachment_kept() {
        let mut opened = HashSet::from(["s1".to_string()]);
        let past = Instant::now() - Duration::from_secs(30);
        let mut times = HashMap::from([("s1".to_string(), past)]);
        let infos = vec![make_info("s1", 1)];

        cleanup_externally_opened(&mut opened, &mut times, &infos, Duration::from_secs(15));

        assert!(opened.contains("s1"));
    }

    #[test]
    fn cleanup_within_grace_with_attachment_kept() {
        let mut opened = HashSet::from(["s1".to_string()]);
        let mut times = HashMap::from([("s1".to_string(), Instant::now())]);
        let infos = vec![make_info("s1", 1)];

        cleanup_externally_opened(&mut opened, &mut times, &infos, Duration::from_secs(15));

        assert!(opened.contains("s1"));
        assert!(times.contains_key("s1"));
    }
}
