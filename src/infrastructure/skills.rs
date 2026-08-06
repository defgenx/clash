//! Embedded Claude Code skills, shipped inside the clash binary.
//!
//! Skills that clash features depend on (the `clash-workflow` executor, the
//! two reviewers, the explainer) are compiled in via `include_str!` from the
//! repo's `skills/` directory and installed under `<claude_dir>/skills/`.
//!
//! Installation is a **decision, not a side effect**. At startup clash only
//! installs skills that are *missing* (nothing to protect there — and a
//! missing skill breaks workflow agents outright). Everything else goes
//! through plan → decide → apply:
//!
//! - [`plan_install`] compares embedded ↔ installed ↔ manifest and
//!   categorizes each skill: missing, outdated (changed upstream, untouched
//!   locally) or locally-edited (changed upstream AND touched since clash
//!   last wrote it), plus any retired skill still present.
//! - The human (GUI popup, or the `general.skills_update` setting when it is
//!   not `ask`) picks an [`ApplyMode`]: overwrite everything, update only the
//!   untouched ones, or keep everything as is.
//! - [`apply_decision`] performs exactly that and stamps the manifest's
//!   `resolvedFingerprint`, so the question comes back only when the embedded
//!   set actually changes again (fingerprint, not version — dev builds change
//!   content without bumping the version).
//!
//! The manifest (`.clash-skills.json` next to the skill dirs) records the
//! clash version, a content hash per skill *as last written by clash* (the
//! "was it hand-edited since?" oracle — hashes are only updated for files
//! clash writes, so a kept local edit stays detectable), and the resolved
//! fingerprint.

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
    EmbeddedSkill {
        name: "clash-explain",
        content: include_str!("../../skills/clash-explain/SKILL.md"),
    },
];

/// Skills clash used to ship and no longer does. Removed when an update
/// decision applies (never on `keep`) — a retired skill left behind would
/// keep matching kickoff prompts its replacements now own (`clash-review`
/// was split into `clash-plan-review` + `clash-code-review`).
pub const RETIRED_SKILLS: &[&str] = &["clash-review"];

/// Manifest file written next to the skill directories.
const MANIFEST_FILE: &str = ".clash-skills.json";

/// How an update decision is applied. Parsed from the GUI popup's answer or
/// the `general.skills_update` setting (`ask` parses to `None`: no automatic
/// application — a human decides).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyMode {
    /// Overwrite every managed skill, local edits included.
    All,
    /// Update only skills untouched since clash last wrote them.
    Untouched,
    /// Touch nothing (missing skills are still installed at startup).
    Keep,
}

impl ApplyMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "all" => Some(Self::All),
            "untouched" => Some(Self::Untouched),
            "keep" => Some(Self::Keep),
            _ => None, // "ask" and anything unknown → a human decides
        }
    }
}

/// What a skills upgrade would touch — the popup's content. Computed fresh
/// from disk on demand; never cached.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsPlan {
    /// The running clash version (display).
    pub version: String,
    /// Fingerprint of the embedded skill set — the resolution key.
    pub fingerprint: String,
    /// Not installed at all. Installed silently at startup — there is
    /// nothing to protect, and workflow agents break without them.
    pub missing: Vec<String>,
    /// Changed upstream, untouched locally since clash last wrote them.
    pub outdated: Vec<String>,
    /// Changed upstream AND hand-edited locally (or predating the manifest,
    /// which is indistinguishable — treated as edited so nothing is clobbered
    /// silently).
    pub locally_edited: Vec<String>,
    /// Retired skills whose directories still exist.
    pub retired_present: Vec<String>,
    /// True when this exact embedded set was already decided on (manifest's
    /// `resolvedFingerprint`) — the popup must not return until the next
    /// upgrade.
    pub resolved: bool,
    /// The one bit the frontend actually branches on.
    pub needs_decision: bool,
}

/// What one [`apply_decision`] run changed.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsReport {
    /// The clash version that ran the update.
    pub version: String,
    /// Skills written (fresh installs and refreshes alike).
    pub updated: Vec<String>,
    /// Subset of `updated` whose local edits were overwritten (mode `all`).
    pub locally_edited: Vec<String>,
    /// Skills deliberately left as they are (locally-edited ones under
    /// `untouched`, everything under `keep`).
    pub kept: Vec<String>,
    /// Retired skills whose directories were removed.
    pub removed: Vec<String>,
}

/// On-disk manifest shape. Lenient like every co-edited JSON file.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Manifest {
    clash_version: String,
    /// Skill name → FNV-1a hash of the content clash last wrote.
    skills: BTreeMap<String, String>,
    /// Fingerprint of the embedded set the user last decided on.
    resolved_fingerprint: String,
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

/// One value that changes iff any embedded skill changes — the resolution key.
fn embedded_fingerprint() -> String {
    let mut acc = String::new();
    for s in SKILLS {
        acc.push_str(s.name);
        acc.push(':');
        acc.push_str(&content_hash(s.content));
        acc.push('\n');
    }
    content_hash(&acc)
}

fn read_manifest(claude_dir: &Path) -> Option<Manifest> {
    let raw = std::fs::read_to_string(claude_dir.join("skills").join(MANIFEST_FILE)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Compare-then-write, like the skills themselves.
fn write_manifest(claude_dir: &Path, manifest: &Manifest) {
    let skills_dir = claude_dir.join("skills");
    let path = skills_dir.join(MANIFEST_FILE);
    let Ok(json) = serde_json::to_string_pretty(manifest) else {
        return;
    };
    if std::fs::read_to_string(&path).ok().as_deref() == Some(json.as_str()) {
        return;
    }
    if let Err(e) =
        std::fs::create_dir_all(&skills_dir).and_then(|()| write_atomic(&path, json.as_bytes()))
    {
        tracing::warn!("skill manifest write failed: {}", e);
    }
}

fn write_skill(claude_dir: &Path, skill: &EmbeddedSkill) -> bool {
    let dir = claude_dir.join("skills").join(skill.name);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("skill install: create {} failed: {}", dir.display(), e);
        return false;
    }
    match write_atomic(&dir.join("SKILL.md"), skill.content.as_bytes()) {
        Ok(()) => {
            tracing::info!("installed embedded skill '{}'", skill.name);
            true
        }
        Err(e) => {
            tracing::warn!("skill install: write {} failed: {}", dir.display(), e);
            false
        }
    }
}

/// What an upgrade would touch, computed fresh from disk.
pub fn plan_install(claude_dir: &Path) -> SkillsPlan {
    let manifest = read_manifest(claude_dir);
    let fingerprint = embedded_fingerprint();
    let mut plan = SkillsPlan {
        version: env!("CARGO_PKG_VERSION").to_string(),
        resolved: manifest
            .as_ref()
            .is_some_and(|m| m.resolved_fingerprint == fingerprint),
        fingerprint,
        ..SkillsPlan::default()
    };
    for skill in SKILLS {
        let path = claude_dir.join("skills").join(skill.name).join("SKILL.md");
        match std::fs::read_to_string(&path) {
            Err(_) => plan.missing.push(skill.name.to_string()),
            Ok(existing) if existing == skill.content => {}
            Ok(existing) => {
                let untouched = manifest
                    .as_ref()
                    .and_then(|m| m.skills.get(skill.name))
                    .is_some_and(|last| last == &content_hash(&existing));
                if untouched {
                    plan.outdated.push(skill.name.to_string());
                } else {
                    plan.locally_edited.push(skill.name.to_string());
                }
            }
        }
    }
    for name in RETIRED_SKILLS {
        if claude_dir.join("skills").join(name).is_dir() {
            plan.retired_present.push(name.to_string());
        }
    }
    plan.needs_decision = !plan.resolved
        && (!plan.outdated.is_empty()
            || !plan.locally_edited.is_empty()
            || !plan.retired_present.is_empty());
    plan
}

/// Install only the skills that are absent. Safe to run unconditionally at
/// every startup: nothing existing is touched and no decision is consumed.
/// Returns the names it installed.
pub fn install_missing(claude_dir: &Path) -> Vec<String> {
    let plan = plan_install(claude_dir);
    if plan.missing.is_empty() {
        return Vec::new();
    }
    let mut manifest = read_manifest(claude_dir).unwrap_or_default();
    let mut installed = Vec::new();
    for skill in SKILLS {
        if !plan.missing.iter().any(|m| m == skill.name) {
            continue;
        }
        if write_skill(claude_dir, skill) {
            manifest
                .skills
                .insert(skill.name.to_string(), content_hash(skill.content));
            installed.push(skill.name.to_string());
        }
    }
    if !installed.is_empty() {
        write_manifest(claude_dir, &manifest);
    }
    installed
}

/// Apply an update decision and stamp it, so the question only returns when
/// the embedded set changes again. `keep` writes nothing (beyond the stamp)
/// and removes nothing; `untouched` skips hand-edited skills; `all` restores
/// every managed skill to the embedded copy.
pub fn apply_decision(claude_dir: &Path, mode: ApplyMode) -> SkillsReport {
    let plan = plan_install(claude_dir);
    let mut manifest = read_manifest(claude_dir).unwrap_or_default();
    let mut report = SkillsReport {
        version: plan.version.clone(),
        ..SkillsReport::default()
    };

    for skill in SKILLS {
        let name = skill.name.to_string();
        let missing = plan.missing.contains(&name);
        let outdated = plan.outdated.contains(&name);
        let edited = plan.locally_edited.contains(&name);
        let write = missing
            || match mode {
                ApplyMode::All => outdated || edited,
                ApplyMode::Untouched => outdated,
                ApplyMode::Keep => false,
            };
        if write {
            if write_skill(claude_dir, skill) {
                manifest
                    .skills
                    .insert(name.clone(), content_hash(skill.content));
                report.updated.push(name.clone());
                if edited {
                    report.locally_edited.push(name);
                }
            }
        } else if outdated || edited {
            // Deliberately left alone — the manifest hash is NOT updated, so
            // the "hand-edited since clash last wrote it" oracle stays true.
            report.kept.push(name);
        }
    }

    if mode != ApplyMode::Keep {
        for name in &plan.retired_present {
            let dir = claude_dir.join("skills").join(name);
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => {
                    tracing::info!("removed retired skill '{}'", name);
                    report.removed.push(name.clone());
                }
                Err(e) => tracing::warn!("skill install: remove {} failed: {}", dir.display(), e),
            }
        }
    }

    manifest.clash_version = plan.version;
    manifest.resolved_fingerprint = plan.fingerprint;
    write_manifest(claude_dir, &manifest);
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn fresh_dir_is_missing_only_and_installs_silently() {
        let dir = TempDir::new().unwrap();
        let plan = plan_install(dir.path());
        assert_eq!(plan.missing.len(), SKILLS.len());
        // A fresh install is not a decision — nothing pre-existing to protect.
        assert!(!plan.needs_decision, "{:?}", plan);

        let installed = install_missing(dir.path());
        assert_eq!(installed.len(), SKILLS.len());
        for skill in SKILLS {
            let path = dir.path().join("skills").join(skill.name).join("SKILL.md");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), skill.content);
        }
        // Idempotent, and now fully clean.
        assert!(install_missing(dir.path()).is_empty());
        let plan = plan_install(dir.path());
        assert!(plan.missing.is_empty() && !plan.needs_decision);
    }

    /// Simulate "a new clash shipped different content": the file on disk
    /// differs from the embedded copy, and the manifest proves whether the
    /// user touched it since clash last wrote it.
    fn simulate(dir: &Path, name: &str, disk: &str, last_written_hash: &str) {
        let d = dir.join("skills").join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), disk).unwrap();
        let mut manifest = read_manifest(dir).unwrap_or_default();
        manifest
            .skills
            .insert(name.to_string(), last_written_hash.to_string());
        write_manifest(dir, &manifest);
    }

    #[test]
    fn outdated_vs_locally_edited_is_the_manifest_oracle() {
        let dir = TempDir::new().unwrap();
        install_missing(dir.path());
        let a = SKILLS[0].name;
        let b = SKILLS[1].name;
        // a: disk == what clash last wrote (old version) != embedded → outdated.
        simulate(
            dir.path(),
            a,
            "old shipped content",
            &content_hash("old shipped content"),
        );
        // b: disk differs from what clash last wrote → locally edited.
        simulate(
            dir.path(),
            b,
            "user's own variant",
            &content_hash("what clash wrote"),
        );

        let plan = plan_install(dir.path());
        assert_eq!(plan.outdated, vec![a.to_string()]);
        assert_eq!(plan.locally_edited, vec![b.to_string()]);
        assert!(plan.needs_decision);
    }

    #[test]
    fn untouched_mode_updates_outdated_and_keeps_edits() {
        let dir = TempDir::new().unwrap();
        install_missing(dir.path());
        let a = SKILLS[0].name;
        let b = SKILLS[1].name;
        simulate(
            dir.path(),
            a,
            "old shipped content",
            &content_hash("old shipped content"),
        );
        simulate(
            dir.path(),
            b,
            "user's own variant",
            &content_hash("what clash wrote"),
        );

        let report = apply_decision(dir.path(), ApplyMode::Untouched);
        assert_eq!(report.updated, vec![a.to_string()]);
        assert_eq!(report.kept, vec![b.to_string()]);
        assert!(report.locally_edited.is_empty());
        // The edit survives on disk, and stays detectable as an edit later
        // (its manifest hash was not updated).
        let b_path = dir.path().join("skills").join(b).join("SKILL.md");
        assert_eq!(
            std::fs::read_to_string(&b_path).unwrap(),
            "user's own variant"
        );
        let plan = plan_install(dir.path());
        assert_eq!(plan.locally_edited, vec![b.to_string()]);
        // …but the decision is stamped: no more popup until the next upgrade.
        assert!(plan.resolved && !plan.needs_decision);
    }

    #[test]
    fn all_mode_overwrites_and_reports_the_edits_it_ate() {
        let dir = TempDir::new().unwrap();
        install_missing(dir.path());
        let b = SKILLS[1].name;
        simulate(
            dir.path(),
            b,
            "user's own variant",
            &content_hash("what clash wrote"),
        );

        let report = apply_decision(dir.path(), ApplyMode::All);
        assert!(report.updated.contains(&b.to_string()));
        assert_eq!(report.locally_edited, vec![b.to_string()]);
        let b_path = dir.path().join("skills").join(b).join("SKILL.md");
        assert_eq!(std::fs::read_to_string(&b_path).unwrap(), SKILLS[1].content);
        assert!(!plan_install(dir.path()).needs_decision);
    }

    #[test]
    fn keep_mode_touches_nothing_but_still_resolves() {
        let dir = TempDir::new().unwrap();
        install_missing(dir.path());
        let a = SKILLS[0].name;
        simulate(
            dir.path(),
            a,
            "old shipped content",
            &content_hash("old shipped content"),
        );
        let retired = dir.path().join("skills").join("clash-review");
        std::fs::create_dir_all(&retired).unwrap();

        let report = apply_decision(dir.path(), ApplyMode::Keep);
        assert!(report.updated.is_empty());
        assert_eq!(report.kept, vec![a.to_string()]);
        assert!(
            report.removed.is_empty(),
            "keep must not delete retired dirs"
        );
        assert!(retired.exists());
        let a_path = dir.path().join("skills").join(a).join("SKILL.md");
        assert_eq!(
            std::fs::read_to_string(&a_path).unwrap(),
            "old shipped content"
        );
        // Decided: quiet until the embedded set changes again.
        assert!(!plan_install(dir.path()).needs_decision);
    }

    #[test]
    fn retired_dirs_trigger_the_decision_and_go_on_update() {
        let dir = TempDir::new().unwrap();
        install_missing(dir.path());
        let retired = dir.path().join("skills").join("clash-review");
        std::fs::create_dir_all(&retired).unwrap();
        std::fs::write(retired.join("SKILL.md"), "old harness").unwrap();

        let plan = plan_install(dir.path());
        assert_eq!(plan.retired_present, vec!["clash-review".to_string()]);
        assert!(plan.needs_decision);

        let report = apply_decision(dir.path(), ApplyMode::Untouched);
        assert_eq!(report.removed, vec!["clash-review".to_string()]);
        assert!(!retired.exists());
    }

    #[test]
    fn apply_mode_parses_the_setting_values() {
        assert_eq!(ApplyMode::parse("all"), Some(ApplyMode::All));
        assert_eq!(ApplyMode::parse(" Untouched "), Some(ApplyMode::Untouched));
        assert_eq!(ApplyMode::parse("KEEP"), Some(ApplyMode::Keep));
        // "ask" (and junk) means a human decides — no automatic application.
        assert_eq!(ApplyMode::parse("ask"), None);
        assert_eq!(ApplyMode::parse(""), None);
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
