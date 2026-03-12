#[derive(Debug, Clone)]
pub enum UiAction {
    HideHelp,
    ToggleHelp,
    ShowConfirm {
        message: String,
        on_confirm: Box<crate::application::actions::Action>,
    },
    ConfirmYes,
    ConfirmNo,
    Toast(String),
    EnterCommandMode,
    EnterFilterMode,
    /// Prompt for directory and spawn a new session.
    EnterNewSessionMode,
    ExitInputMode,
    SubmitInput(String),
    /// Text editing in command/filter/new-session input bar.
    InputEdit(InputEdit),
    CycleSessionFilter,
    SetSessionFilter(crate::application::state::SessionFilter),
    ScrollDown,
    ScrollUp,
    /// Toggle expand/collapse for the selected session's subagents.
    ToggleExpand,
    /// Start or restart the guided tour.
    StartTour,
    /// Advance to the next tour step (or finish).
    TourNext,
    /// Skip / close the tour.
    TourSkip,
    /// Trigger a self-update check + install.
    RequestUpdate,
    /// Detach from a daemon-managed session (Esc or Ctrl+B while attached).
    DetachSession,
    /// The attached session exited.
    SessionExited {
        session_id: String,
    },
    Quit,
    /// Tick event — advances animation frame counter, clears stale toasts.
    Tick,
}

/// Text editing operations for the input bar.
#[derive(Debug, Clone)]
pub enum InputEdit {
    InsertChar(char),
    Backspace,
    Delete,
    CursorLeft,
    CursorRight,
    CursorHome,
    CursorEnd,
}
