//! Scratch-note actions — create files/folders, rename, move, delete, and
//! open notes in an editor. Entry ids are POSIX paths relative to the scratch
//! root (`""` = root); see [`crate::domain::entities::ScratchNote`].

#[derive(Debug, Clone)]
pub enum ScratchAction {
    /// Create a new empty note titled `title` inside the folder at `parent`
    /// (relative path; `""` = root). The file name is derived from the title.
    Create { parent: String, title: String },
    /// Create a new folder named `name` inside the folder at `parent`.
    CreateDir { parent: String, name: String },
    /// Rename the entry at `id` (file or folder) to `new_name`.
    Rename { id: String, new_name: String },
    /// Delete the entry at `id` (folders are removed recursively).
    Delete { id: String },
    /// Open the note (by id) in an editor — resolves the file path, then
    /// reuses the IDE/editor picker (`Effect::DetectEditors`) to launch it.
    OpenInEditor { id: String },
    /// Expand/collapse the folder at `id` in the tree view.
    ToggleDir { id: String },
}
