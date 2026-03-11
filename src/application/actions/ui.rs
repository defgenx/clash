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
    ExitInputMode,
    SubmitInput(String),
    ToggleAllSessions,
    ScrollDown,
    ScrollUp,
    /// Detach from a daemon-managed session (Esc or Ctrl+B while attached).
    DetachSession,
    /// The attached session exited.
    SessionExited { session_id: String },
    Quit,
}
