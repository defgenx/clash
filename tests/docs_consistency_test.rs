//! Doc-consistency test (Issue T4 / 12A from the plan-review).
//!
//! CLAUDE.md mandates that **every** new keybinding or session prefix is
//! reflected in three places: README.md, the help overlay, and the tour
//! widget. Future refactors can silently delete or rename one of those
//! references and the project would still compile and pass the unit
//! suite — this test catches that.
//!
//! Cheap insurance: it only does substring matches, runs in milliseconds,
//! but is reliable enough to fail loudly when one of the three drifts.

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    // The crate's CARGO_MANIFEST_DIR is the project root, since this is
    // an integration test under `tests/`.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("could not read {}: {}", path.display(), e))
}

#[test]
fn wild_session_prefix_documented_in_readme() {
    let readme = read("README.md");
    assert!(
        readme.contains('\u{1f33f}'),
        "README.md must mention the wild-session prefix glyph 🌿 \
         (added by the wild-session-adoption feature). Current content \
         does not — keep README in sync per CLAUDE.md doc rule."
    );
    assert!(
        readme.contains("Wild"),
        "README.md must describe the Wild source. Update the Session \
         Source Prefixes table when this fails."
    );
}

#[test]
fn wild_session_prefix_documented_in_help_overlay() {
    let help = read("src/infrastructure/tui/widgets/help_overlay.rs");
    assert!(
        help.contains("\\u{1f33f}") || help.contains('\u{1f33f}'),
        "help_overlay.rs must include the 🌿 wild-session legend entry. \
         Per CLAUDE.md, every new prefix gets a `?` legend line."
    );
}

#[test]
fn wild_session_prefix_documented_in_tour() {
    let tour = read("src/infrastructure/tui/widgets/tour.rs");
    assert!(
        tour.contains("\\u{1f33f}") || tour.contains('\u{1f33f}'),
        "tour.rs must include the 🌿 wild-session legend entry. \
         Per CLAUDE.md, every new prefix gets a tour step / legend line."
    );
}
