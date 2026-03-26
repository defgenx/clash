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
    CycleSectionFilter,
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
    /// The attached session exited.
    SessionExited {
        session_id: String,
    },
    Quit,
    /// Actually quit (after confirmation).
    QuitConfirmed,
    /// Immediate quit, bypassing graceful shutdown (Ctrl+C escape hatch).
    ForceQuit,
    /// Show a picker dialog.
    ShowPicker {
        title: String,
        items: Vec<crate::application::state::PickerItem>,
        on_select: crate::application::state::PickerAction,
    },
    PickerUp,
    PickerDown,
    PickerSelect,
    PickerCancel,
    /// Manual diff refresh (from `r` key on Diff view).
    RefreshDiff,
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
