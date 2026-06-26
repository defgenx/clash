//! Scratch-note actions — create, delete, and open notes in an editor.

#[derive(Debug, Clone)]
pub enum ScratchAction {
    /// Create a new empty note with the given title (file name derived from it).
    Create { title: String },
    /// Delete the note with the given id (file name).
    Delete { id: String },
    /// Open the note (by id) in an editor — resolves the file path, then
    /// reuses the IDE/editor picker (`Effect::DetectIdes`) to launch it.
    OpenInEditor { id: String },
}
