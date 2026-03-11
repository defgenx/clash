use crate::adapters::views::ViewKind;

#[derive(Debug, Clone)]
pub enum NavAction {
    /// Navigate to a specific view, replacing current.
    NavigateTo(ViewKind),
    /// Drill into a resource (push onto nav stack).
    DrillIn {
        view: ViewKind,
        context: String,
    },
    /// Go back (pop navigation stack).
    GoBack,
}
