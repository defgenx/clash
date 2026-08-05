//! Embedded Claude Code skills, shipped inside the clash binary.
//!
//! Skills that clash features depend on (`clash-workflow`, the executor half
//! of Workflows, and the two reviewers `clash-plan-review` /
//! `clash-code-review`) are compiled in via `include_str!` from the repo's
//! `skills/` directory and installed into `<claude_dir>/skills/` on every
//! startup of either binary. Installation is compare-then-write: the file is
//! only rewritten when its content differs from the embedded version, so
//! skills are always up-to-date after a clash update without churning file
//! mtimes (or FS watchers) on every launch.
//!
//! The embedded copy is the source of truth — local edits to an installed
//! managed skill are overwritten on the next launch. Users who want a custom
//! variant should copy it under a different skill name.
//!
//! A manifest (`.clash-skills.json` next to the skill dirs) records which
//! clash version last installed and a content hash per skill. It exists for
//! *visibility*, not gating: [`install_skills`] returns a [`SkillsReport`] of
//! what changed this run — which skills were refreshed, which retired ones
//! were removed, and which had been hand-edited since clash last wrote them —
//! so the GUI can say "skills updated for clash vX" instead of rewriting
//! silently.

use std::collections::BTreeMap;
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
        name: "clash-plan-review",
        content: include_str!("../../skills/clash-plan-review/SKILL.md"),
    },
    EmbeddedSkill {
        name: "clash-code-review",
        content: include_str!("../../skills/clash-code-review/SKILL.md"),
    },
];

/// Skills clash used to ship and no longer does. Removed from the install dir
/// at startup — a retired skill left behind would keep matching kickoff
/// prompts its replacements now own (`clash-review` was split into
/// `clash-plan-review` + `clash-code-review`).
pub const RETIRED_SKILLS: &[&str] = &["clash-review"];

/// Manifest file written next to the skill directories.
const MANIFEST_FILE: &str = ".clash-skills.json";

/// What one [`install_skills`] run changed. Everything empty means the
/// installed skills already matched this binary.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsReport {
    /// The clash version that just ran the install.
    pub version: String,
    /// The version recorded by the previous install, when a manifest existed.
    pub previous_version: Option<String>,
    /// Skills written this run (new or refreshed).
    pub updated: Vec<String>,
    /// Retired skills whose directories were removed this run.
    pub removed: Vec<String>,
    /// Skills whose installed copy had been hand-edited since clash last
    /// wrote them. They are still overwritten (the embedded copy is the
    /// source of truth), but the report lets the GUI say so out loud.
    pub locally_edited: Vec<String>,
}

impl SkillsReport {
    /// True when the run changed nothing worth telling the user about.
    pub fn is_noop(&self) -> bool {
        self.updated.is_empty() && self.removed.is_empty() && self.locally_edited.is_empty()
    }
}

/// On-disk manifest shape. Lenient like every co-edited JSON file.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Manifest {
    clash_version: String,
    /// Skill name → FNV-1a hash of the content clash last installed.
    skills: BTreeMap<String, String>,
}

/// FNV-1a 64-bit over the full content, hex. Same hand-rolled function family
/// as `application::workflow::line_hash` (no hashing crate; stability across
/// versions matters more than speed).
fn content_hash(content: &str) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for b in content.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{:016x}", hash)
}

/// Install (or refresh) every embedded skill under `<claude_dir>/skills/`,
/// remove retired ones, and stamp the manifest. Best-effort: failures are
/// logged, never fatal — a missing skill degrades the workflow agent, not
/// clash itself.
pub fn install_skills(claude_dir: &Path) -> SkillsReport {
    let skills_dir = claude_dir.join("skills");
    let manifest_path = skills_dir.join(MANIFEST_FILE);
    let previous: Option<Manifest> = std::fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok());

    let mut report = SkillsReport {
        version: env!("CARGO_PKG_VERSION").to_string(),
        previous_version: previous
            .as_ref()
            .map(|m| m.clash_version.clone())
            .filter(|v| !v.is_empty()),
        ..SkillsReport::default()
    };

    for skill in SKILLS {
        let dir = skills_dir.join(skill.name);
        let path = dir.join("SKILL.md");
        let existing = std::fs::read_to_string(&path).ok();
        if existing.as_deref() == Some(skill.content) {
            continue; // current
        }
        // Hand-edited since clash last wrote it? Only decidable when the
        // previous manifest recorded what clash wrote.
        if let (Some(existing), Some(prev)) = (existing.as_deref(), previous.as_ref()) {
            if let Some(last_written) = prev.skills.get(skill.name) {
                if &content_hash(existing) != last_written {
                    report.locally_edited.push(skill.name.to_string());
                }
            }
        }
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("skill install: create {} failed: {}", dir.display(), e);
            continue;
        }
        match write_atomic(&path, skill.content.as_bytes()) {
            Ok(()) => {
                tracing::info!("installed embedded skill '{}'", skill.name);
                report.updated.push(skill.name.to_string());
            }
            Err(e) => tracing::warn!("skill install: write {} failed: {}", path.display(), e),
        }
    }

    for name in RETIRED_SKILLS {
        let dir = skills_dir.join(name);
        if !dir.is_dir() {
            continue;
        }
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {
                tracing::info!("removed retired skill '{}'", name);
                report.removed.push(name.to_string());
            }
            Err(e) => tracing::warn!("skill install: remove {} failed: {}", dir.display(), e),
        }
    }

    // Stamp what this binary installed. Compare-then-write like the skills
    // themselves, so a quiet startup never churns the file.
    let manifest = Manifest {
        clash_version: report.version.clone(),
        skills: SKILLS
            .iter()
            .map(|s| (s.name.to_string(), content_hash(s.content)))
            .collect(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&manifest) {
        let current = std::fs::read_to_string(&manifest_path).ok();
        if current.as_deref() != Some(json.as_str()) {
            if let Err(e) = std::fs::create_dir_all(&skills_dir)
                .and_then(|()| write_atomic(&manifest_path, json.as_bytes()))
            {
                tracing::warn!("skill manifest write failed: {}", e);
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn installs_and_refreshes_skills() {
        let dir = TempDir::new().unwrap();
        let report = install_skills(dir.path());
        // First run installs every skill and stamps the manifest.
        assert_eq!(report.updated.len(), SKILLS.len());
        assert!(report.previous_version.is_none());
        let manifest =
            std::fs::read_to_string(dir.path().join("skills").join(MANIFEST_FILE)).unwrap();
        assert!(manifest.contains(env!("CARGO_PKG_VERSION")));
        for skill in SKILLS {
            let path = dir.path().join("skills").join(skill.name).join("SKILL.md");
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                skill.content,
                "{} not installed",
                skill.name
            );
        }
        // A clean re-run is a no-op — nothing rewritten, nothing to report.
        let report = install_skills(dir.path());
        assert!(report.is_noop(), "{:?}", report);
        assert_eq!(
            report.previous_version.as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn local_edits_are_overwritten_and_reported() {
        let dir = TempDir::new().unwrap();
        install_skills(dir.path());
        let skill = &SKILLS[0];
        let path = dir.path().join("skills").join(skill.name).join("SKILL.md");
        std::fs::write(&path, "tampered").unwrap();

        let report = install_skills(dir.path());
        // The embedded copy is the source of truth…
        assert_eq!(std::fs::read_to_string(&path).unwrap(), skill.content);
        assert_eq!(report.updated, vec![skill.name.to_string()]);
        // …but the overwrite is called out instead of silent.
        assert_eq!(report.locally_edited, vec![skill.name.to_string()]);
    }

    #[test]
    fn retired_skills_are_removed() {
        let dir = TempDir::new().unwrap();
        let retired = dir.path().join("skills").join("clash-review");
        std::fs::create_dir_all(&retired).unwrap();
        std::fs::write(retired.join("SKILL.md"), "old harness").unwrap();

        let report = install_skills(dir.path());
        assert!(!retired.exists(), "retired skill dir must be removed");
        assert_eq!(report.removed, vec!["clash-review".to_string()]);
        // Gone means gone: the next run has nothing to remove.
        assert!(install_skills(dir.path()).removed.is_empty());
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
    fn skill_names_are_unique_and_never_retired() {
        // Two entries with the same name would silently install over each
        // other; a name both shipped and retired would install then delete
        // itself every launch.
        let mut names: Vec<&str> = SKILLS.iter().map(|s| s.name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate embedded skill name");
        for retired in RETIRED_SKILLS {
            assert!(
                !names.contains(retired),
                "{} is both shipped and retired",
                retired
            );
        }
    }
}
