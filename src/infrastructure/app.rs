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
        ];
        let watcher = FsWatcher::new(&watch_paths, fs_tx).ok();

        let mut state = AppState::new();

        // Show guided tour on first launch
        let tour_marker = data_dir.join("clash/.tour_done");
        if !tour_marker.exists() {
            state.tour_step = Some(0);
            if let Some(parent) = tour_marker.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&tour_marker, "1");
        }

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

        // Background update check (non-blocking)
        let mut update_check: Option<tokio::task::JoinHandle<_>> = Some(tokio::spawn(async {
            crate::infrastructure::update::check_for_update().await
        }));

        loop {
            // Poll background update check without blocking
            if let Some(ref handle) = update_check {
                if handle.is_finished() {
                    if let Some(handle) = update_check.take() {
                        if let Ok(Some(crate::infrastructure::update::UpdateCheck::Available {
                            version,
                            ..
                        })) = handle.await
                        {
                            self.state.toast =
                                Some(format!("v{} available — :update to install", version));
                        }
                    }
                }
            }

            // Keep vt100 parser size in sync with the actual render area every frame.
            // Use the real layout computation so the parser matches the widget area exactly.
            if self.state.input_mode == InputMode::Attached {
                if let Some(ref mut parser) = self.vt_parser {
                    let term_size = terminal.size().unwrap_or_default();
                    let term_rect =
                        ratatui::layout::Rect::new(0, 0, term_size.width, term_size.height);
                    let layout = crate::infrastructure::tui::layout::FrameLayout::new(term_rect);
                    let block = ratatui::widgets::Block::bordered();
                    let inner = block.inner(layout.body);
                    let expected_rows = inner.height;
                    let expected_cols = inner.width;
                    let (current_rows, current_cols) = parser.screen().size();
                    if current_rows != expected_rows || current_cols != expected_cols {
                        parser.set_size(expected_rows, expected_cols);
                    }
                }
            }

            let vt_screen = self.vt_parser.as_ref().map(|p| p.screen());
            terminal.draw(|f| renderer::draw_with_terminal(&self.state, f, vt_screen))?;

            // Non-blocking FS event check
            if let Some(ref mut rx) = fs_rx {
                let mut needs_refresh_all = false;
                let mut jsonl_changed = false;
                while let Ok(paths) = rx.try_recv() {
                    for p in &paths {
                        if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                            jsonl_changed = true;
                        } else {
                            needs_refresh_all = true;
                        }
                    }
                }
                if needs_refresh_all {
                    let _ = self.state.store.refresh_all(&self.backend);
                } else if jsonl_changed {
                    // JSONL changed → refresh session statuses immediately
                    self.refresh_daemon_sessions().await;
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

                        // Handle text input for command/filter/new-session mode
                        if matches!(
                            self.state.input_mode,
                            InputMode::Command | InputMode::Filter | InputMode::NewSession
                        ) {
                            use crate::application::actions::ui::InputEdit;
                            use crate::application::actions::UiAction;

                            let action = match key.code {
                                crossterm::event::KeyCode::Enter => {
                                    let input = self.state.input_buffer.clone();
                                    Action::Ui(UiAction::SubmitInput(input))
                                }
                                crossterm::event::KeyCode::Esc => {
                                    Action::Ui(UiAction::ExitInputMode)
                                }
                                crossterm::event::KeyCode::Backspace => {
                                    Action::Ui(UiAction::InputEdit(InputEdit::Backspace))
                                }
                                crossterm::event::KeyCode::Delete => {
                                    Action::Ui(UiAction::InputEdit(InputEdit::Delete))
                                }
                                crossterm::event::KeyCode::Left => {
                                    Action::Ui(UiAction::InputEdit(InputEdit::CursorLeft))
                                }
                                crossterm::event::KeyCode::Right => {
                                    Action::Ui(UiAction::InputEdit(InputEdit::CursorRight))
                                }
                                crossterm::event::KeyCode::Home => {
                                    Action::Ui(UiAction::InputEdit(InputEdit::CursorHome))
                                }
                                crossterm::event::KeyCode::End => {
                                    Action::Ui(UiAction::InputEdit(InputEdit::CursorEnd))
                                }
                                crossterm::event::KeyCode::Char(c) => {
                                    Action::Ui(UiAction::InputEdit(InputEdit::InsertChar(c)))
                                }
                                _ => {
                                    continue;
                                }
                            };

                            let effects = reducer::reduce(&mut self.state, action);
                            if self.execute_effects(effects, terminal).await {
                                return Ok(());
                            }
                            continue;
                        }

                        let action = input::handle_key(key, &self.state);
                        let effects = reducer::reduce(&mut self.state, action);
                        if self.execute_effects(effects, terminal).await {
                            return Ok(());
                        }
                    }
                    Event::Tick => {
                        let _ = reducer::reduce(
                            &mut self.state,
                            Action::Ui(crate::application::actions::UiAction::Tick),
                        );
                        // Refresh sessions every ~500ms (50 ticks) on session-related views
                        if self.state.tick.is_multiple_of(50)
                            && matches!(
                                self.state.current_view(),
                                crate::adapters::views::ViewKind::Sessions
                                    | crate::adapters::views::ViewKind::SessionDetail
                                    | crate::adapters::views::ViewKind::Subagents
                                    | crate::adapters::views::ViewKind::SubagentDetail
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
                        // Resize vt100 parser using real layout computation
                        if self.state.input_mode == InputMode::Attached {
                            let rect = ratatui::layout::Rect::new(0, 0, width, height);
                            let layout = crate::infrastructure::tui::layout::FrameLayout::new(rect);
                            let block = ratatui::widgets::Block::bordered();
                            let inner = block.inner(layout.body);
                            if let Some(ref mut parser) = self.vt_parser {
                                parser.set_size(inner.height, inner.width);
                            }
                            // Notify daemon to resize the PTY
                            if let Some(ref session_id) = self.state.attached_session {
                                let _ = self
                                    .daemon
                                    .resize(session_id, inner.width, inner.height)
                                    .await;
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

    /// Refresh sessions: load from disk, overlay hook statuses, then daemon.
    async fn refresh_daemon_sessions(&mut self) {
        use crate::domain::entities::SessionStatus;

        // 1. Read sessions from Claude's JSONL files (baseline status)
        let _ = self.state.store.refresh_sessions(&self.backend);

        // Preload subagents for sessions that have them (for tree view)
        self.state.store.refresh_all_subagents(&self.backend);

        // 2. Overlay hook-based statuses (instant, from Claude Code lifecycle events)
        let hook_statuses =
            crate::infrastructure::hooks::read_all_statuses(self.backend.base_dir());
        for session in &mut self.state.store.sessions {
            if let Some(&status) = hook_statuses.get(&session.id) {
                session.status = status;
                session.is_running = !matches!(status, SessionStatus::Idle);
            }
        }

        // 3. Overlay daemon status on matching sessions, add daemon-only sessions
        if self.daemon.is_connected() {
            if let Ok(infos) = self.daemon.list_sessions().await {
                for info in infos {
                    let status = match info.status.as_str() {
                        "running" => SessionStatus::Running,
                        "thinking" => SessionStatus::Thinking,
                        "waiting" => SessionStatus::Waiting,
                        "starting" => SessionStatus::Starting,
                        "prompting" => SessionStatus::Prompting,
                        _ => SessionStatus::Idle,
                    };
                    let is_running = !matches!(status, SessionStatus::Idle);

                    // Try to find matching disk session and update its status
                    if let Some(existing) = self
                        .state
                        .store
                        .sessions
                        .iter_mut()
                        .find(|s| s.id == info.session_id)
                    {
                        existing.status = status;
                        existing.is_running = is_running;
                        // Overlay daemon name if the session doesn't have one yet
                        if existing.name.is_none() && info.name.is_some() {
                            existing.name = info.name.clone();
                        }
                    } else {
                        // Daemon-only session (no disk file yet)
                        let created = chrono::DateTime::from_timestamp(info.created_at as i64, 0)
                            .map(|dt| {
                                dt.with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d %H:%M")
                                    .to_string()
                            })
                            .unwrap_or_default();

                        // Derive project name from cwd (last path component)
                        let project = if !info.cwd.is_empty() {
                            std::path::Path::new(&info.cwd)
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("")
                                .to_string()
                        } else {
                            String::new()
                        };

                        let git_branch =
                            crate::infrastructure::fs::backend::FsBackend::detect_git_branch(
                                &info.cwd,
                            );
                        let worktree =
                            crate::infrastructure::fs::backend::FsBackend::detect_worktree(
                                &info.cwd,
                            );

                        let summary = if !info.cwd.is_empty() {
                            format!("New session in {}", info.cwd)
                        } else {
                            let clients_info = if info.attached_clients > 0 {
                                format!("{} attached", info.attached_clients)
                            } else {
                                "detached".to_string()
                            };
                            format!("PID {} | {}", info.pid, clients_info)
                        };

                        self.state
                            .store
                            .sessions
                            .push(crate::domain::entities::Session {
                                id: info.session_id,
                                project,
                                project_path: info.cwd,
                                last_modified: created,
                                summary,
                                first_prompt: String::new(),
                                has_subagents: false,
                                subagent_count: 0,
                                message_count: 0,
                                git_branch,
                                is_running,
                                status,
                                worktree,
                                name: info.name,
                            });
                    }
                }
            }
        }

        // Flat subagents list is rebuilt inside refresh_all_subagents()
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
                    name,
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
                    match self
                        .daemon
                        .create_session(
                            &session_id,
                            &self.cli_runner.claude_bin,
                            &args,
                            &cwd,
                            name.clone(),
                        )
                        .await
                    {
                        Ok(()) => {
                            let label = if let Some(ref n) = name {
                                n.clone()
                            } else if session_id.len() > 8 {
                                session_id[..8].to_string()
                            } else {
                                session_id.clone()
                            };
                            self.state.toast = Some(format!("Session {} created", label));
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
                            .create_session(
                                &session_id,
                                &self.cli_runner.claude_bin,
                                &args,
                                "",
                                None,
                            )
                            .await;
                    }

                    match self.daemon.attach(&session_id).await {
                        Ok(()) => {
                            // Initialize vt100 parser sized to the actual inner render area
                            let term_size = terminal.size().unwrap_or_default();
                            let term_rect =
                                ratatui::layout::Rect::new(0, 0, term_size.width, term_size.height);
                            let layout =
                                crate::infrastructure::tui::layout::FrameLayout::new(term_rect);
                            let block = ratatui::widgets::Block::bordered();
                            let inner = block.inner(layout.body);
                            self.vt_parser = Some(vt100::Parser::new(inner.height, inner.width, 0));

                            // Resize the daemon PTY to match our terminal
                            let _ = self
                                .daemon
                                .resize(&session_id, inner.width, inner.height)
                                .await;

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
                    }
                }
                Effect::TerminateProcess { session_id } => {
                    terminate_claude_process(&session_id).await;
                }
                Effect::TerminateAllProcesses => {
                    // Terminate all Claude processes for all known sessions
                    for session in &self.state.store.sessions {
                        terminate_claude_process(&session.id).await;
                    }
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

                // ── UI state ────────────────────────────────────
                Effect::ShowSpinner(msg) => {
                    self.state.spinner = Some(msg);
                }
                Effect::PerformUpdate => {
                    self.state.toast = Some("Downloading update...".to_string());
                    match crate::infrastructure::update::perform_update().await {
                        Ok(version) => {
                            self.state.toast =
                                Some(format!("Updated to v{}! Restart clash to apply.", version));
                        }
                        Err(msg) => {
                            self.state.toast = Some(msg);
                        }
                    }
                }
            }
        }
        false
    }
}

/// Find and terminate external Claude Code processes for a session.
///
/// Strategy: search for `claude` processes whose command line contains the session ID
/// (e.g. `claude --resume <session_id>`). Uses `pgrep` on macOS/Linux.
async fn terminate_claude_process(session_id: &str) {
    // Try pgrep to find claude processes matching this session
    let output = tokio::process::Command::new("pgrep")
        .args(["-f", &format!("claude.*{}", session_id)])
        .output()
        .await;

    if let Ok(output) = output {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Ok(pid) = line.trim().parse::<u32>() {
                    // Don't kill ourselves
                    let our_pid = std::process::id();
                    if pid == our_pid {
                        continue;
                    }
                    tracing::info!(
                        "Terminating Claude process PID {} for session {}",
                        pid,
                        session_id
                    );
                    // SIGTERM first
                    let _ = tokio::process::Command::new("kill")
                        .args(["-TERM", &pid.to_string()])
                        .output()
                        .await;
                    // Give it a moment, then SIGKILL if still alive
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    let _ = tokio::process::Command::new("kill")
                        .args(["-KILL", &pid.to_string()])
                        .output()
                        .await;
                }
            }
        }
    }

    // Also try pkill as a fallback for any remaining processes
    let _ = tokio::process::Command::new("pkill")
        .args(["-f", &format!("claude.*{}", session_id)])
        .output()
        .await;
}
