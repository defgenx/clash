//! Embedded Claude Code skills, shipped inside the clash binary.
//!
//! Skills that clash features depend on (currently `clash-workflow`, the
//! executor half of Workflows) are compiled in via `include_str!` from the
//! repo's `skills/` directory and installed into `<claude_dir>/skills/` on
//! every startup of either binary. Installation is compare-then-write: the
//! file is only rewritten when its content differs from the embedded
//! version, so skills are always up-to-date after a clash update without
//! churning file mtimes (or FS watchers) on every launch.
//!
//! The embedded copy is the source of truth — local edits to an installed
//! managed skill are overwritten on the next launch. Users who want a
//! custom variant should copy it under a different skill name.

use std::path::Path;

use crate::infrastructure::fs::atomic::write_atomic;

/// One skill compiled into the binary.
pub struct EmbeddedSkill {
    /// Directory name under `<claude_dir>/skills/`.
    pub name: &'static str,
    /// Full `SKILL.md` content.
    pub content: &'static str,
}

/// Every skill clash ships. Add new entries here and under `skills/` in the
/// repo; both binaries install them at startup.
pub const SKILLS: &[EmbeddedSkill] = &[EmbeddedSkill {
    name: "clash-workflow",
    content: include_str!("../../skills/clash-workflow/SKILL.md"),
}];

/// Install (or refresh) every embedded skill under `<claude_dir>/skills/`.
/// Best-effort: failures are logged, never fatal — a missing skill degrades
/// the workflow agent, not clash itself.
pub fn install_skills(claude_dir: &Path) {
    for skill in SKILLS {
        let dir = claude_dir.join("skills").join(skill.name);
        let path = dir.join("SKILL.md");
        match std::fs::read_to_string(&path) {
            Ok(existing) if existing == skill.content => continue, // current
            _ => {}
        }
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("skill install: create {} failed: {}", dir.display(), e);
            continue;
        }
        match write_atomic(&path, skill.content.as_bytes()) {
            Ok(()) => tracing::info!("installed embedded skill '{}'", skill.name),
            Err(e) => tracing::warn!("skill install: write {} failed: {}", path.display(), e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn installs_and_refreshes_skills() {
        let dir = TempDir::new().unwrap();
        install_skills(dir.path());
        let path = dir.path().join("skills/clash-workflow/SKILL.md");
        let installed = std::fs::read_to_string(&path).unwrap();
        assert_eq!(installed, SKILLS[0].content);

        // A locally-modified managed skill is overwritten on the next run —
        // the embedded copy is the source of truth.
        std::fs::write(&path, "tampered").unwrap();
        install_skills(dir.path());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SKILLS[0].content);
    }

    #[test]
    fn embedded_skill_has_frontmatter() {
        // The skill must stay loadable by Claude Code: frontmatter with a
        // name matching the directory and a description.
        let content = SKILLS[0].content;
        assert!(content.starts_with("---\n"), "SKILL.md needs frontmatter");
        assert!(content.contains("name: clash-workflow"));
        assert!(content.contains("description:"));
    }
}
