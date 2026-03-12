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
                // Skip heavy refreshes while attached — they block the event loop
                // and prevent keystrokes from reaching the daemon promptly.
                if self.state.input_mode != InputMode::Attached {
                    if needs_refresh_all {
                        let _ = self.state.store.refresh_all(&self.backend);
                    } else if jsonl_changed {
                        self.refresh_daemon_sessions().await;
                    }
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
                                self.draw_if_spinner(terminal);
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
                            InputMode::Command
                                | InputMode::Filter
                                | InputMode::NewSession
                                | InputMode::NewSessionName
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
                            if self.state.spinner.is_some() {
                                let vt_screen = self.vt_parser.as_ref().map(|p| p.screen());
                                let _ = terminal.draw(|f| {
                                    renderer::draw_with_terminal(&self.state, f, vt_screen)
                                });
                            }
                            if self.execute_effects(effects, terminal).await {
                                return Ok(());
                            }
                            continue;
                        }

                        let action = input::handle_key(key, &self.state);
                        let effects = reducer::reduce(&mut self.state, action);
                        // Draw a frame before executing effects so spinners are visible
                        self.draw_if_spinner(terminal);
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
                            let inner = Self::compute_vt_inner_area(width, height);
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

    /// Draw a frame immediately if the spinner is active, so it's visible
    /// before potentially long-running effects execute.
    fn draw_if_spinner(&self, terminal: &mut ratatui::DefaultTerminal) {
        if self.state.spinner.is_some() {
            let vt_screen = self.vt_parser.as_ref().map(|p| p.screen());
            let _ = terminal.draw(|f| renderer::draw_with_terminal(&self.state, f, vt_screen));
        }
    }

    /// Compute the inner rendering area for the vt100 terminal emulator,
    /// accounting for the frame layout and bordered block.
    fn compute_vt_inner_area(width: u16, height: u16) -> ratatui::layout::Rect {
        let rect = ratatui::layout::Rect::new(0, 0, width, height);
        let layout = crate::infrastructure::tui::layout::FrameLayout::new(rect);
        let block = ratatui::widgets::Block::bordered();
        block.inner(layout.body)
    }

    /// Ensure the daemon is connected, auto-starting if needed.
    /// Returns `true` on success, `false` on failure (sets error toast).
    async fn ensure_daemon_connected(&mut self) -> bool {
        if self.daemon.is_connected() {
            return true;
        }
        self.state.spinner = Some("Starting daemon...".to_string());
        match self.daemon.connect().await {
            Ok(()) => true,
            Err(e) => {
                self.state.spinner = None;
                self.state.toast = Some(format!("Daemon failed to start: {}", e));
                self.state.input_mode = InputMode::Normal;
                self.state.attached_session = None;
                false
            }
        }
    }

    /// Refresh sessions: load from disk, overlay hook statuses, then daemon.
    async fn refresh_daemon_sessions(&mut self) {
        self.load_disk_sessions();
        self.overlay_hook_statuses();
        self.overlay_daemon_sessions().await;
        self.resolve_session_names().await;
    }

    /// Phase 1: Load sessions from JSONL files and preload subagents.
    fn load_disk_sessions(&mut self) {
        let _ = self.state.store.refresh_sessions(&self.backend);
        self.state.store.refresh_all_subagents(&self.backend);
    }

    /// Phase 2: Overlay hook-based statuses (instant, from Claude Code lifecycle events).
    fn overlay_hook_statuses(&mut self) {
        use crate::domain::entities::SessionStatus;

        let hook_statuses =
            crate::infrastructure::hooks::read_all_statuses(self.backend.base_dir());
        for session in &mut self.state.store.sessions {
            if let Some(&status) = hook_statuses.get(&session.id) {
                session.status = status;
                session.is_running = !matches!(status, SessionStatus::Idle);
            }
        }
    }

    /// Phase 3: Overlay daemon status on matching sessions, add daemon-only sessions.
    async fn overlay_daemon_sessions(&mut self) {
        use crate::domain::entities::SessionStatus;

        if !self.daemon.is_connected() {
            return;
        }
        let infos = match self.daemon.list_sessions().await {
            Ok(infos) => infos,
            Err(_) => return,
        };

        let mut claimed_indices = std::collections::HashSet::new();

        for info in infos {
            let status = info
                .status
                .parse::<SessionStatus>()
                .unwrap_or(SessionStatus::Idle);
            let is_running = !matches!(status, SessionStatus::Idle);

            // Try to find matching disk session by ID first
            let matched_by_id = self
                .state
                .store
                .sessions
                .iter()
                .position(|s| s.id == info.session_id);

            if let Some(idx) = matched_by_id {
                let existing = &mut self.state.store.sessions[idx];
                existing.status = status;
                existing.is_running = is_running;
                if existing.name.is_none() && info.name.is_some() {
                    existing.name = info.name.clone();
                }
                claimed_indices.insert(idx);
            } else if info.name.is_some() && !info.cwd.is_empty() {
                // Clash-created daemon session — try to match by CWD
                let daemon_cwd = info.cwd.trim_end_matches('/');
                let matched_by_cwd =
                    self.state
                        .store
                        .sessions
                        .iter()
                        .enumerate()
                        .find_map(|(idx, s)| {
                            let disk_path = s.project_path.trim_end_matches('/');
                            if disk_path == daemon_cwd
                                && s.name.is_none()
                                && !claimed_indices.contains(&idx)
                            {
                                Some(idx)
                            } else {
                                None
                            }
                        });

                if let Some(idx) = matched_by_cwd {
                    let existing = &mut self.state.store.sessions[idx];
                    existing.status = status;
                    existing.is_running = is_running;
                    existing.name = info.name.clone();
                    claimed_indices.insert(idx);
                } else {
                    // Truly daemon-only (no disk file yet)
                    self.state.store.sessions.push(session_from_daemon_info(
                        &info,
                        String::new(),
                        status,
                        is_running,
                    ));
                }
            } else {
                // External session resumed via daemon (no name, no cwd match)
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
                    .push(session_from_daemon_info(&info, summary, status, is_running));
            }
        }
    }

    /// Phase 4: Resolve session names from daemon (by project dir) and disk persistence.
    async fn resolve_session_names(&mut self) {
        // Pass 1: propagate daemon names to unnamed sessions by project directory
        if self.daemon.is_connected() {
            if let Ok(infos) = self.daemon.list_sessions().await {
                for info in &infos {
                    if let Some(ref daemon_name) = info.name {
                        if info.cwd.is_empty() {
                            continue;
                        }
                        let daemon_project = path_last_component(&info.cwd);
                        if daemon_project.is_empty() {
                            continue;
                        }
                        for session in &mut self.state.store.sessions {
                            if session.name.is_some() {
                                continue;
                            }
                            if path_last_component(&session.project_path) == daemon_project {
                                session.name = Some(daemon_name.clone());
                            }
                        }
                    }
                }
            }
        }

        // Pass 2: apply persisted names for sessions whose daemon has exited
        let saved_names =
            crate::infrastructure::hooks::read_all_session_names(self.backend.base_dir());
        for session in &mut self.state.store.sessions {
            if session.name.is_none() {
                if let Some(name) = saved_names.get(&session.id) {
                    session.name = Some(name.clone());
                }
            }
        }
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
                Effect::MarkSessionIdle { session_id } => {
                    crate::infrastructure::hooks::write_session_status(
                        self.backend.base_dir(),
                        &session_id,
                        "idle",
                    );
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

                // ── Daemon-managed sessions ────────────────────
                Effect::DaemonCreateSession {
                    session_id,
                    args,
                    cwd,
                    name,
                } => {
                    if !self.ensure_daemon_connected().await {
                        continue;
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
                                // Persist name to disk so it survives daemon restarts
                                crate::infrastructure::hooks::save_session_name(
                                    self.backend.base_dir(),
                                    &session_id,
                                    n,
                                );
                                n.clone()
                            } else {
                                crate::adapters::format::short_id(&session_id, 8).to_string()
                            };
                            self.state.toast = Some(format!("Session {} created", label));
                        }
                        Err(e) => {
                            self.state.spinner = None;
                            self.state.toast = Some(format!("Create failed: {}", e));
                            self.state.input_mode = InputMode::Normal;
                            self.state.attached_session = None;
                        }
                    }
                }
                Effect::DaemonAttach { session_id } => {
                    if !self.ensure_daemon_connected().await {
                        continue;
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
                            let inner =
                                Self::compute_vt_inner_area(term_size.width, term_size.height);
                            self.vt_parser = Some(vt100::Parser::new(inner.height, inner.width, 0));

                            // Resize the daemon PTY to match our terminal
                            let _ = self
                                .daemon
                                .resize(&session_id, inner.width, inner.height)
                                .await;

                            let short = crate::adapters::format::short_id(&session_id, 8);
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
                    // Spawn in background — don't block the event loop with the
                    // 5-second SIGKILL escalation sleep.
                    tokio::spawn(async move {
                        terminate_claude_process(&session_id).await;
                    });
                }
                Effect::TerminateAllProcesses => {
                    // Terminate all Claude processes for all known sessions
                    let ids: Vec<String> = self
                        .state
                        .store
                        .sessions
                        .iter()
                        .map(|s| s.id.clone())
                        .collect();
                    tokio::spawn(async move {
                        for id in ids {
                            terminate_claude_process(&id).await;
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
        // Clear spinner after all effects have executed
        self.state.spinner = None;
        false
    }
}

/// Build a `Session` from daemon `SessionInfo` for sessions with no disk file.
fn session_from_daemon_info(
    info: &crate::infrastructure::daemon::protocol::SessionInfo,
    summary: String,
    status: crate::domain::entities::SessionStatus,
    is_running: bool,
) -> crate::domain::entities::Session {
    crate::domain::entities::Session {
        id: info.session_id.clone(),
        project: path_last_component(&info.cwd).to_string(),
        project_path: info.cwd.clone(),
        summary,
        is_running,
        status,
        name: info.name.clone(),
        ..Default::default()
    }
}

/// Extract the last component of a path string (e.g. "/foo/bar" → "bar").
fn path_last_component(path: &str) -> &str {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
}

/// Gracefully stop external Claude Code processes for a session.
///
/// Strategy: SIGTERM first, then SIGKILL after 5 seconds.
/// Note: /exit is handled by the daemon PTY layer; this catches standalone processes.
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

            // Escalate to SIGKILL after 5 seconds for any survivors
            tokio::time::sleep(Duration::from_secs(5)).await;
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

    // Fallback: pkill -TERM for any remaining processes
    let _ = tokio::process::Command::new("pkill")
        .args(["-TERM", "-f", &format!("claude.*{}", session_id)])
        .output()
        .await;
}
