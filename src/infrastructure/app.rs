//! Application coordinator — the runtime event loop.
//!
//! This is infrastructure: it owns the terminal, the backends, and the
//! event loop. It translates abstract Effects from the reducer into real IO.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use crate::adapters::{input, renderer};
use crate::application::actions::Action;
use crate::application::effects::Effect;
use crate::application::reducer;
use crate::application::state::{AppState, InputMode};
use crate::domain::ports::{CliGateway, DataRepository};
use crate::infrastructure::cli::commands;
use crate::infrastructure::cli::runner::RealCliRunner;
use crate::infrastructure::daemon::client::DaemonClient;
use crate::infrastructure::event::{self, Event};
use crate::infrastructure::fs::backend::FsBackend;
use crate::infrastructure::fs::watcher::FsWatcher;

/// Main application coordinator.
pub struct App {
    state: AppState,
    backend: FsBackend,
    cli_runner: RealCliRunner,
    _watcher: Option<FsWatcher>,
    fs_event_rx: Option<tokio::sync::mpsc::UnboundedReceiver<Vec<PathBuf>>>,
    daemon: DaemonClient,
    /// vt100 terminal emulator for inline session rendering.
    vt_parser: Option<vt100::Parser>,
}

impl App {
    pub fn new(data_dir: PathBuf, claude_bin: String) -> Self {
        let backend = FsBackend::new(data_dir.clone());
        let cli_runner = RealCliRunner::with_bin(claude_bin);

        let (fs_tx, fs_rx) = tokio::sync::mpsc::unbounded_channel();
        let watch_paths = vec![
            backend.teams_dir(),
            backend.tasks_dir(),
            backend.projects_dir(),
        ];
        let watcher = FsWatcher::new(&watch_paths, fs_tx).ok();

        let mut state = AppState::new();

        if let Err(e) = state.store.refresh_all(&backend) {
            tracing::error!("Initial data load failed: {}", e);
        }
        // Sessions are loaded from daemon in run() (async)

        let daemon = DaemonClient::new(DaemonClient::default_socket_path());

        Self {
            state,
            backend,
            cli_runner,
            _watcher: watcher,
            fs_event_rx: Some(fs_rx),
            daemon,
            vt_parser: None,
        }
    }

    /// Run the main event loop.
    pub async fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> color_eyre::Result<()> {
        let tick_rate = Duration::from_millis(10);
        let mut fs_rx = self.fs_event_rx.take();

        // Auto-connect to daemon (best-effort, non-blocking)
        match self.daemon.connect().await {
            Ok(()) => tracing::info!("Connected to clash daemon"),
            Err(e) => tracing::info!("Daemon not available (legacy mode): {}", e),
        }

        // Load initial sessions (from daemon or disk fallback)
        self.refresh_daemon_sessions().await;

        loop {
            let vt_screen = self.vt_parser.as_ref().map(|p| p.screen());
            terminal.draw(|f| renderer::draw_with_terminal(&self.state, f, vt_screen))?;

            // Non-blocking FS event check
            if let Some(ref mut rx) = fs_rx {
                let mut needs_refresh_all = false;
                let mut needs_refresh_sessions = false;
                while let Ok(paths) = rx.try_recv() {
                    for p in &paths {
                        if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                            needs_refresh_sessions = true;
                        } else {
                            needs_refresh_all = true;
                        }
                    }
                }
                if needs_refresh_all {
                    let _ = self.state.store.refresh_all(&self.backend);
                }
                if needs_refresh_sessions {
                    // FS changes don't trigger daemon session refresh —
                    // daemon sessions are refreshed on their own schedule
                }
            }

            if let Some(event) = event::read_event(tick_rate).await {
                match event {
                    Event::Key(key) => {
                        // Handle attached mode — forward input to daemon
                        if self.state.input_mode == InputMode::Attached {
                            use crossterm::event::{KeyCode, KeyModifiers};
                            // Esc or Ctrl+B → detach
                            if key.code == KeyCode::Esc
                                || (key.code == KeyCode::Char('b')
                                    && key.modifiers.contains(KeyModifiers::CONTROL))
                            {
                                let action = Action::Ui(
                                    crate::application::actions::UiAction::DetachSession,
                                );
                                let effects = reducer::reduce(&mut self.state, action);
                                if self.execute_effects(effects, terminal).await {
                                    return Ok(());
                                }
                            } else if let Some(session_id) = self.state.attached_session.clone() {
                                // Forward keystroke to daemon
                                let bytes = crate::adapters::input::key_to_bytes(key);
                                if !bytes.is_empty() {
                                    let _ = self.daemon.send_input(&session_id, &bytes).await;
                                }
                            }
                            continue;
                        }

                        // Handle text input for command/filter mode
                        if matches!(
                            self.state.input_mode,
                            InputMode::Command | InputMode::Filter
                        ) {
                            match key.code {
                                crossterm::event::KeyCode::Enter => {
                                    let input = self.state.input_buffer.clone();
                                    let action =
                                        crate::application::actions::UiAction::SubmitInput(input);
                                    let effects =
                                        reducer::reduce(&mut self.state, Action::Ui(action));
                                    if self.execute_effects(effects, terminal).await {
                                        return Ok(());
                                    }
                                    continue;
                                }
                                crossterm::event::KeyCode::Esc => {
                                    let effects = reducer::reduce(
                                        &mut self.state,
                                        Action::Ui(
                                            crate::application::actions::UiAction::ExitInputMode,
                                        ),
                                    );
                                    if self.execute_effects(effects, terminal).await {
                                        return Ok(());
                                    }
                                    continue;
                                }
                                crossterm::event::KeyCode::Backspace => {
                                    self.state.input_buffer.pop();
                                    continue;
                                }
                                crossterm::event::KeyCode::Char(c) => {
                                    self.state.input_buffer.push(c);
                                    if self.state.input_mode == InputMode::Filter {
                                        self.state.filter = self.state.input_buffer.clone();
                                    }
                                    continue;
                                }
                                _ => continue,
                            }
                        }

                        let action = input::handle_key(key, &self.state);
                        let effects = reducer::reduce(&mut self.state, action);
                        if self.execute_effects(effects, terminal).await {
                            return Ok(());
                        }
                    }
                    Event::Tick => {
                        self.state.tick = self.state.tick.wrapping_add(1);
                        // Toast lasts ~3 seconds (300 ticks at 10ms)
                        if self.state.toast.is_some() && self.state.tick.is_multiple_of(300) {
                            self.state.toast = None;
                        }
                        // Refresh sessions every ~500ms (50 ticks)
                        if self.state.tick.is_multiple_of(50)
                            && matches!(
                                self.state.current_view(),
                                crate::adapters::views::ViewKind::Sessions
                            )
                        {
                            self.refresh_daemon_sessions().await;
                        }
                        // Refresh conversation every ~1s (100 ticks)
                        if self.state.tick.is_multiple_of(100) {
                            self.auto_refresh_conversation();
                        }
                        // Poll daemon events when attached
                        if self.state.input_mode == InputMode::Attached {
                            self.poll_daemon_events();
                        }
                    }
                    Event::Resize(width, height) => {
                        // Resize vt100 parser when terminal changes size
                        if self.state.input_mode == InputMode::Attached {
                            let rows = height.saturating_sub(4); // header + footer + borders
                            let cols = width.saturating_sub(2); // borders
                            if let Some(ref mut parser) = self.vt_parser {
                                parser.set_size(rows, cols);
                            }
                            // Notify daemon to resize the PTY
                            if let Some(ref session_id) = self.state.attached_session {
                                let _ = self.daemon.resize(session_id, cols, rows).await;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Poll daemon for output/exit events and feed into vt100 parser.
    fn poll_daemon_events(&mut self) {
        use crate::infrastructure::daemon::protocol::Event as DaemonEvent;
        while let Some(event) = self.daemon.try_recv_event() {
            match event {
                DaemonEvent::Output { session_id, data } => {
                    if self.state.attached_session.as_deref() == Some(&session_id) {
                        if let Ok(bytes) =
                            crate::infrastructure::daemon::protocol::decode_data(&data)
                        {
                            if let Some(ref mut parser) = self.vt_parser {
                                parser.process(&bytes);
                            }
                        }
                    }
                }
                DaemonEvent::Exited { session_id, .. } => {
                    let action = Action::Ui(crate::application::actions::UiAction::SessionExited {
                        session_id,
                    });
                    let _ = reducer::reduce(&mut self.state, action);
                    // Clean up vt100 parser when session exits
                    if self.state.attached_session.is_none() {
                        self.vt_parser = None;
                    }
                }
                _ => {} // Ok, Error, Pong, Sessions — handled elsewhere
            }
        }
    }

    /// Refresh sessions from the daemon (clash-managed sessions only).
    /// Falls back to disk-based sessions if daemon is not available.
    async fn refresh_daemon_sessions(&mut self) {
        if self.daemon.is_connected() {
            match self.daemon.list_sessions().await {
                Ok(infos) => {
                    use crate::domain::entities::SessionStatus;
                    let sessions: Vec<crate::domain::entities::Session> = infos
                        .into_iter()
                        .map(|info| {
                            let status = match info.status.as_str() {
                                "running" => SessionStatus::Running,
                                "thinking" => SessionStatus::Thinking,
                                "waiting" => SessionStatus::Waiting,
                                "starting" => SessionStatus::Starting,
                                "prompting" => SessionStatus::Prompting,
                                _ => SessionStatus::Idle,
                            };
                            let is_running = !matches!(status, SessionStatus::Idle);
                            let created =
                                chrono::DateTime::from_timestamp(info.created_at as i64, 0)
                                    .map(|dt| {
                                        dt.with_timezone(&chrono::Local)
                                            .format("%Y-%m-%d %H:%M")
                                            .to_string()
                                    })
                                    .unwrap_or_default();
                            let clients_info = if info.attached_clients > 0 {
                                format!("{} attached", info.attached_clients)
                            } else {
                                "detached".to_string()
                            };
                            crate::domain::entities::Session {
                                id: info.session_id,
                                project: String::new(),
                                project_path: String::new(),
                                last_modified: created,
                                summary: format!("PID {} | {}", info.pid, clients_info),
                                first_prompt: String::new(),
                                has_subagents: false,
                                subagent_count: 0,
                                message_count: 0,
                                git_branch: String::new(),
                                is_running,
                                status,
                            }
                        })
                        .collect();
                    self.state.store.set_sessions(sessions);
                    return;
                }
                Err(e) => {
                    tracing::warn!("Daemon list_sessions failed: {}", e);
                }
            }
        }
        // Fallback: load from disk
        let _ = self.state.store.refresh_sessions(&self.backend);
    }

    /// Auto-refresh conversation if viewing SessionDetail or SubagentDetail.
    fn auto_refresh_conversation(&mut self) {
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
                    }
                }
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
                    if let Some(sa) = self
                        .state
                        .store
                        .subagents
                        .iter()
                        .find(|s| s.id == agent_id)
                        .cloned()
                    {
                        let _ = self.state.store.load_subagent_conversation(
                            &self.backend,
                            &sa.project,
                            &sa.parent_session_id,
                            &sa.id,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    /// Execute effects — translates abstract Effects into real IO.
    ///
    /// This is the key clean architecture boundary: the reducer speaks in
    /// domain terms (PersistTask, RemoveTeam); this method translates to
    /// filesystem ops, CLI calls, etc.
    async fn execute_effects(
        &mut self,
        effects: Vec<Effect>,
        terminal: &mut ratatui::DefaultTerminal,
    ) -> bool {
        let mut queue = VecDeque::from(effects);

        while let Some(effect) = queue.pop_front() {
            match effect {
                Effect::Quit => return true,

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
                Effect::DeleteSession {
                    project,
                    session_id,
                } => {
                    if let Err(e) = self.backend.delete_session(&project, &session_id) {
                        self.state.toast = Some(format!("Delete failed: {}", e));
                    }
                }
                Effect::DeleteAllSessions => {
                    if let Err(e) = self.backend.delete_all_sessions() {
                        self.state.toast = Some(format!("Delete all failed: {}", e));
                    }
                }

                // ── Daemon-managed sessions ────────────────────
                Effect::DaemonCreateSession {
                    session_id,
                    args,
                    cwd,
                } => {
                    // Auto-start daemon if not connected
                    if !self.daemon.is_connected() {
                        self.state.toast = Some("Starting daemon...".to_string());
                        if let Err(e) = self.daemon.connect().await {
                            self.state.toast = Some(format!("Daemon failed to start: {}", e));
                            self.state.input_mode = InputMode::Normal;
                            self.state.attached_session = None;
                            continue;
                        }
                    }
                    // Resolve __CWD__ placeholder to actual current directory
                    let resolved_cwd = if cwd == "__CWD__" {
                        std::env::current_dir()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default()
                    } else {
                        cwd
                    };
                    match self
                        .daemon
                        .create_session(
                            &session_id,
                            &self.cli_runner.claude_bin,
                            &args,
                            &resolved_cwd,
                        )
                        .await
                    {
                        Ok(()) => {
                            let short = if session_id.len() > 8 {
                                &session_id[..8]
                            } else {
                                &session_id
                            };
                            self.state.toast = Some(format!("Session {} created", short));
                        }
                        Err(e) => {
                            self.state.toast = Some(format!("Create failed: {}", e));
                            self.state.input_mode = InputMode::Normal;
                            self.state.attached_session = None;
                        }
                    }
                }
                Effect::DaemonAttach { session_id } => {
                    // Auto-start daemon if not connected
                    if !self.daemon.is_connected() {
                        self.state.toast = Some("Starting daemon...".to_string());
                        if let Err(e) = self.daemon.connect().await {
                            self.state.toast = Some(format!("Daemon failed to start: {}", e));
                            self.state.input_mode = InputMode::Normal;
                            self.state.attached_session = None;
                            continue;
                        }
                    }

                    // For existing Claude sessions (not clash-created), ensure the daemon
                    // has a PTY session running with --resume.
                    if !session_id.starts_with("clash-") {
                        let args = commands::resume_session_args(&session_id);
                        // Ignore "already exists" error — that's fine, we just attach.
                        let _ = self
                            .daemon
                            .create_session(&session_id, &self.cli_runner.claude_bin, &args, "")
                            .await;
                    }

                    match self.daemon.attach(&session_id).await {
                        Ok(()) => {
                            // Initialize vt100 parser sized to the terminal body area
                            let term_size = terminal.size().unwrap_or_default();
                            // Body area = total height minus header(1) and footer(1) minus border(2)
                            let rows = term_size.height.saturating_sub(4);
                            let cols = term_size.width.saturating_sub(2);
                            self.vt_parser = Some(vt100::Parser::new(rows, cols, 0));

                            // Resize the daemon PTY to match our terminal
                            let _ = self.daemon.resize(&session_id, cols, rows).await;

                            let short = if session_id.len() > 8 {
                                &session_id[..8]
                            } else {
                                &session_id
                            };
                            self.state.toast =
                                Some(format!("Attached to {} | Esc/Ctrl+B detach", short));
                        }
                        Err(e) => {
                            self.state.toast = Some(format!("Attach failed: {}", e));
                            self.state.input_mode = InputMode::Normal;
                            self.state.attached_session = None;
                        }
                    }
                }
                Effect::DaemonDetach { session_id } => {
                    if self.daemon.is_connected() {
                        let _ = self.daemon.detach(&session_id).await;
                    }
                    self.vt_parser = None;
                }
                Effect::DaemonKill { session_id } => {
                    if self.daemon.is_connected() {
                        let _ = self.daemon.kill_session(&session_id).await;
                        self.state.toast = Some("Daemon session killed".to_string());
                    }
                }

                // ── UI state ────────────────────────────────────
                Effect::ShowSpinner(msg) => {
                    self.state.spinner = Some(msg);
                }
            }
        }
        false
    }
}
