use crate::adapters::views::Keybinding;

pub struct DiffView;

impl DiffView {
    pub fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("r", "Refresh diff"),
            Keybinding::new("j/k", "Scroll"),
            Keybinding::new("Esc", "Go back"),
        ]
    }
}
