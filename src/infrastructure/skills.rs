//! Embedded Claude Code skills, shipped inside the clash binary.
//!
//! Skills that clash features depend on (`clash-workflow`, the executor half of
//! Workflows, and `clash-review`, its reviewer half) are compiled in via
//! `include_str!` from the
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
pub const SKILLS: &[EmbeddedSkill] = &[
    EmbeddedSkill {
        name: "clash-workflow",
        content: include_str!("../../skills/clash-workflow/SKILL.md"),
    },
    EmbeddedSkill {
        name: "clash-review",
        content: include_str!("../../skills/clash-review/SKILL.md"),
    },
];

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
        // Every embedded skill lands, not just the first.
        for skill in SKILLS {
            let path = dir.path().join("skills").join(skill.name).join("SKILL.md");
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                skill.content,
                "{} not installed",
                skill.name
            );
            // A locally-modified managed skill is overwritten on the next run —
            // the embedded copy is the source of truth.
            std::fs::write(&path, "tampered").unwrap();
            install_skills(dir.path());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), skill.content);
        }
    }

    #[test]
    fn embedded_skills_have_frontmatter_matching_their_directory() {
        // Each skill must stay loadable by Claude Code: frontmatter whose name
        // matches the directory it installs into, plus a description (that is
        // what the model matches a request against).
        for skill in SKILLS {
            let content = skill.content;
            assert!(
                content.starts_with("---\n"),
                "{}: SKILL.md needs frontmatter",
                skill.name
            );
            assert!(
                content.contains(&format!("name: {}", skill.name)),
                "{}: frontmatter name must match the directory",
                skill.name
            );
            assert!(
                content.contains("description:"),
                "{}: needs a description",
                skill.name
            );
        }
    }

    #[test]
    fn skill_names_are_unique() {
        // Two entries with the same name would silently install over each other.
        let mut names: Vec<&str> = SKILLS.iter().map(|s| s.name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate embedded skill name");
    }
}
