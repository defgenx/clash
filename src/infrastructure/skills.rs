//! Embedded skills, shipped inside the clash binary.
//!
//! Skills that clash features depend on (the `clash-workflow` executor, the
//! two reviewers, the explainer) are compiled in via `include_str!` from the
//! repo's `skills/` directory as templates, and installed rendered for each
//! agent CLI ([`AgentKind`]) into that CLI's skills folder — every function
//! here names the harness it writes for, and each folder has its own manifest.
//! Template syntax: `application::skill_template`.
//!
//! Installation only becomes a **decision when it could lose work**. At every
//! startup [`sync_unattended`] installs what is missing and refreshes every
//! skill whose file on disk is still exactly what clash last wrote. A new
//! clash shipping new skill text is not a question — there is nothing of the
//! user's to weigh, and asking anyway trained people to dismiss the popup.
//! What is left over is the real diff: a skill (or a retired directory)
//! hand-edited since clash wrote it, which an update would overwrite.
//!
//! - [`plan_install_in`] compares embedded ↔ installed ↔ manifest for one
//!   folder ([`plan_folders`] does it for every installed agent CLI) and
//!   categorizes each skill: missing, outdated (changed upstream, untouched
//!   locally) or locally-edited (changed upstream AND touched since clash
//!   last wrote it), plus any retired skill still present — flagging which of
//!   those clash did not write. Only the hand-edited ones ask.
//! - The human (GUI popup, or the `general.skills_update` setting when it is
//!   not `ask`) picks an [`ApplyMode`]: overwrite those edits, or keep them.
//! - [`apply_decision_folders`] performs exactly that, in the folders waiting
//!   for a decision only, and stamps each manifest's
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
use std::path::{Path, PathBuf};

use crate::application::skill_template;
use crate::domain::entities::AgentKind;
use crate::infrastructure::fs::atomic::write_atomic;

/// One skill compiled into the binary.
pub struct EmbeddedSkill {
    /// Directory name under `<claude_dir>/skills/`.
    pub name: &'static str,
    /// The `SKILL.md` template; install [`EmbeddedSkill::render`]ed text.
    pub content: &'static str,
}

impl EmbeddedSkill {
    /// The `SKILL.md` text for one harness. A malformed template (which the
    /// tests refuse) falls back to the raw text rather than installing nothing.
    pub fn render(&self, kind: AgentKind) -> String {
        skill_template::render(self.content, kind).unwrap_or_else(|e| {
            tracing::warn!("skill '{}' template: {}", self.name, e);
            self.content.to_string()
        })
    }
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
        name: "clash-explain",
        content: include_str!("../../skills/clash-explain/SKILL.md"),
    },
    EmbeddedSkill {
        name: "clash-drift-review",
        content: include_str!("../../skills/clash-drift-review/SKILL.md"),
    },
    EmbeddedSkill {
        name: "clash-pr-review",
        content: include_str!("../../skills/clash-pr-review/SKILL.md"),
    },
];

/// Skills clash used to ship and no longer does. Removed when an update
/// decision applies (never on `keep`) — a retired skill left behind would
/// keep matching kickoff prompts its replacements now own (`clash-review`
/// was split into `clash-plan-review` + `clash-code-review`, and
/// `clash-code-review` was folded into `clash-pr-review`).
pub const RETIRED_SKILLS: &[&str] = &["clash-review", "clash-code-review"];

/// Manifest file written next to the skill directories.
const MANIFEST_FILE: &str = ".clash-skills.json";

/// How an update decision is applied. Parsed from the GUI popup's answer or
/// the `general.skills_update` setting (`ask` parses to `None`: no automatic
/// application — a human decides).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyMode {
    /// Overwrite every managed skill, local edits included.
    All,
    /// Update only skills untouched since clash last wrote them, and remove
    /// only retired directories clash itself wrote. The unattended default.
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
    /// Refreshed by `sync_unattended` at startup — never a question.
    pub outdated: Vec<String>,
    /// Changed upstream AND hand-edited locally (or predating the manifest,
    /// which is indistinguishable — treated as edited so nothing is clobbered
    /// silently).
    pub locally_edited: Vec<String>,
    /// Retired skills whose directories still exist.
    pub retired_present: Vec<String>,
    /// Subset of `retired_present` whose `SKILL.md` is not what clash last
    /// wrote — removing one would eat someone's edits, so it asks. The rest
    /// go with `sync_unattended`.
    pub retired_edited: Vec<String>,
    /// True when this exact embedded set was already decided on (manifest's
    /// `resolvedFingerprint`) — the popup must not return until the next
    /// upgrade.
    pub resolved: bool,
    /// The one bit the frontend actually branches on: something diverges from
    /// what clash itself wrote, so only the human can settle it.
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

/// One value that changes iff any skill as rendered for `kind` changes — the
/// resolution key of that harness's folder.
fn embedded_fingerprint(kind: AgentKind) -> String {
    let mut acc = String::new();
    for s in SKILLS {
        acc.push_str(s.name);
        acc.push(':');
        acc.push_str(&content_hash(&s.render(kind)));
        acc.push('\n');
    }
    content_hash(&acc)
}

fn read_manifest(skills_dir: &Path) -> Option<Manifest> {
    let raw = std::fs::read_to_string(skills_dir.join(MANIFEST_FILE)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Compare-then-write, like the skills themselves.
fn write_manifest(skills_dir: &Path, manifest: &Manifest) {
    let path = skills_dir.join(MANIFEST_FILE);
    let Ok(json) = serde_json::to_string_pretty(manifest) else {
        return;
    };
    if std::fs::read_to_string(&path).ok().as_deref() == Some(json.as_str()) {
        return;
    }
    if let Err(e) =
        std::fs::create_dir_all(skills_dir).and_then(|()| write_atomic(&path, json.as_bytes()))
    {
        tracing::warn!("skill manifest write failed: {}", e);
    }
}

fn write_skill(skills_dir: &Path, skill: &EmbeddedSkill, text: &str) -> bool {
    let dir = skills_dir.join(skill.name);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("skill install: create {} failed: {}", dir.display(), e);
        return false;
    }
    match write_atomic(&dir.join("SKILL.md"), text.as_bytes()) {
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

/// What an upgrade would touch, computed fresh from disk. `root` holds the
/// `skills/` folder (`<claude_dir>`, `<omp_dir>`). Callers decide over every
/// folder at once with [`plan_folders`].
#[cfg(test)]
pub fn plan_install(root: &Path, kind: AgentKind) -> SkillsPlan {
    plan_install_in(&root.join("skills"), kind)
}

/// [`plan_install`] for a skills directory given directly — Hermes keeps
/// clash's skills in a category folder (`~/.hermes/skills/clash/`).
pub fn plan_install_in(skills_dir: &Path, kind: AgentKind) -> SkillsPlan {
    let manifest = read_manifest(skills_dir);
    let fingerprint = embedded_fingerprint(kind);
    let mut plan = SkillsPlan {
        version: env!("CARGO_PKG_VERSION").to_string(),
        resolved: manifest
            .as_ref()
            .is_some_and(|m| m.resolved_fingerprint == fingerprint),
        fingerprint,
        ..SkillsPlan::default()
    };
    for skill in SKILLS {
        let path = skills_dir.join(skill.name).join("SKILL.md");
        match std::fs::read_to_string(&path) {
            Err(_) => plan.missing.push(skill.name.to_string()),
            Ok(existing) if existing == skill.render(kind) => {}
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
        let dir = skills_dir.join(name);
        if !dir.is_dir() {
            continue;
        }
        plan.retired_present.push(name.to_string());
        let clash_wrote = match std::fs::read_to_string(dir.join("SKILL.md")) {
            // No SKILL.md left: nothing of anyone's in the file to protect.
            Err(_) => true,
            Ok(disk) => manifest
                .as_ref()
                .and_then(|m| m.skills.get(*name))
                .is_some_and(|last| last == &content_hash(&disk)),
        };
        if !clash_wrote {
            plan.retired_edited.push(name.to_string());
        }
    }
    // An upstream-only change is not a decision: `sync_unattended` refreshes
    // what clash itself wrote. Only a divergence from that — a hand-edited
    // skill or retired dir an update would overwrite — needs the human.
    plan.needs_decision =
        !plan.resolved && (!plan.locally_edited.is_empty() || !plan.retired_edited.is_empty());
    plan
}

/// The unattended half, run unconditionally at every startup by both
/// binaries: install what is missing, refresh every skill still holding
/// exactly what clash last wrote, and drop retired directories clash wrote.
/// None of that can lose work, so none of it is worth asking about — what
/// survives it is what [`plan_install`] then reports as a real decision.
///
/// The embedded set is stamped as resolved only when nothing is left to ask,
/// so a pending local-edit decision still reaches the human.
pub fn sync_unattended(root: &Path, kind: AgentKind) -> SkillsReport {
    sync_unattended_in(&root.join("skills"), kind)
}

/// [`sync_unattended_in`] for a directory another tool may have seeded with
/// copies of clash's skills before clash ever wrote there — Hermes's setup
/// imports `~/.claude/skills` into `~/.hermes/skills/clash/`. While the folder
/// has no clash manifest, a copy that differs from the embedded skill is such
/// an import, not a hand edit: it is kept beside as `SKILL.md.pre-clash` and
/// replaced, so nothing is lost and nothing asks. From then on the manifest
/// decides, exactly as everywhere else.
pub fn sync_adopting(skills_dir: &Path, kind: AgentKind) -> SkillsReport {
    if read_manifest(skills_dir).is_none() {
        for skill in SKILLS {
            let file = skills_dir.join(skill.name).join("SKILL.md");
            match std::fs::read_to_string(&file) {
                Ok(disk) if disk != skill.render(kind) => {
                    let kept = file.with_file_name("SKILL.md.pre-clash");
                    if let Err(e) = std::fs::rename(&file, &kept) {
                        tracing::warn!("skill adopt: {} not set aside: {}", file.display(), e);
                    }
                }
                _ => {}
            }
        }
    }
    sync_unattended_in(skills_dir, kind)
}

/// [`sync_unattended`] for a skills directory given directly.
pub fn sync_unattended_in(skills_dir: &Path, kind: AgentKind) -> SkillsReport {
    let stamp = !plan_install_in(skills_dir, kind).needs_decision;
    apply(skills_dir, kind, ApplyMode::Untouched, stamp)
}

/// Apply an update decision and stamp it, so the question only returns when
/// the embedded set changes again. `keep` writes nothing (beyond the stamp)
/// and removes nothing; `untouched` skips hand-edited skills; `all` restores
/// every managed skill to the embedded copy.
#[cfg(test)]
pub fn apply_decision(root: &Path, kind: AgentKind, mode: ApplyMode) -> SkillsReport {
    apply(&root.join("skills"), kind, mode, true)
}

/// Every installed agent CLI with the folder its rendering lives in: Claude
/// Code always, OMP and Hermes only where installed (never created here).
pub fn installed_folders(
    claude_dir: &Path,
    omp_dir: &Path,
    hermes_home: &Path,
) -> Vec<(AgentKind, PathBuf)> {
    let mut out = vec![(AgentKind::Claude, claude_dir.join("skills"))];
    if omp_dir.is_dir() {
        out.push((AgentKind::Omp, omp_dir.join("skills")));
    }
    if hermes_home.is_dir() {
        out.push((
            AgentKind::Hermes,
            crate::infrastructure::hermes::skills_dir(hermes_home),
        ));
    }
    out
}

/// A skill name as the decision shows it: bare with one folder, else tagged
/// with its harness, so "your edits" says which copy was edited.
fn folder_label(name: &str, kind: AgentKind, folders: usize) -> String {
    if folders <= 1 {
        return name.to_string();
    }
    let h = match kind {
        AgentKind::Claude => "Claude Code",
        AgentKind::Omp => "OMP",
        AgentKind::Hermes => "Hermes",
    };
    format!("{name} ({h})")
}

/// The update decision across every folder: one question for all of them.
/// It needs the human when **any** folder holds a hand edit an update would
/// overwrite, and lists each with its harness.
pub fn plan_folders(folders: &[(AgentKind, PathBuf)]) -> SkillsPlan {
    let n = folders.len();
    let mut out = SkillsPlan {
        version: env!("CARGO_PKG_VERSION").to_string(),
        resolved: true,
        ..SkillsPlan::default()
    };
    let mut prints = String::new();
    for (kind, dir) in folders {
        let p = plan_install_in(dir, *kind);
        let tag = |v: Vec<String>| v.into_iter().map(|s| folder_label(&s, *kind, n));
        out.missing.extend(tag(p.missing));
        out.outdated.extend(tag(p.outdated));
        out.locally_edited.extend(tag(p.locally_edited));
        out.retired_present.extend(tag(p.retired_present));
        out.retired_edited.extend(tag(p.retired_edited));
        out.resolved &= p.resolved;
        out.needs_decision |= p.needs_decision;
        prints.push_str(&p.fingerprint);
    }
    out.fingerprint = content_hash(&prints);
    out
}

/// Apply a decision to the folders that are waiting for one, and only those:
/// a folder whose edits were already settled (or that has none) is not
/// overwritten because another harness's copy raised the question.
pub fn apply_decision_folders(folders: &[(AgentKind, PathBuf)], mode: ApplyMode) -> SkillsReport {
    let n = folders.len();
    let mut out = SkillsReport {
        version: env!("CARGO_PKG_VERSION").to_string(),
        ..SkillsReport::default()
    };
    for (kind, dir) in folders {
        if !plan_install_in(dir, *kind).needs_decision {
            continue;
        }
        let r = apply(dir, *kind, mode, true);
        let tag = |v: Vec<String>| v.into_iter().map(|s| folder_label(&s, *kind, n));
        out.updated.extend(tag(r.updated));
        out.locally_edited.extend(tag(r.locally_edited));
        out.kept.extend(tag(r.kept));
        out.removed.extend(tag(r.removed));
    }
    out
}

/// `stamp` records the embedded set as decided on; only `sync_unattended`
/// withholds it, to leave a pending decision visible.
fn apply(skills_dir: &Path, kind: AgentKind, mode: ApplyMode, stamp: bool) -> SkillsReport {
    let plan = plan_install_in(skills_dir, kind);
    let mut manifest = read_manifest(skills_dir).unwrap_or_default();
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
            let text = skill.render(kind);
            if write_skill(skills_dir, skill, &text) {
                manifest.skills.insert(name.clone(), content_hash(&text));
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
            // `untouched` spares a hand-edited retired skill for the same
            // reason it spares a hand-edited current one.
            if mode == ApplyMode::Untouched && plan.retired_edited.contains(name) {
                report.kept.push(name.clone());
                continue;
            }
            let dir = skills_dir.join(name);
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
    if stamp {
        manifest.resolved_fingerprint = plan.fingerprint;
    }
    write_manifest(skills_dir, &manifest);
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// The harness the single-folder tests install for.
    const K: AgentKind = AgentKind::Claude;

    /// An imported copy in a folder clash never managed is replaced, and
    /// kept beside; once clash's manifest is there, an edit is an edit again.
    #[test]
    fn an_imported_copy_is_set_aside_and_replaced_once() {
        let dir = TempDir::new().unwrap();
        let skill = dir.path().join("clash-workflow");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "imported, older").unwrap();
        let report = sync_adopting(dir.path(), AgentKind::Hermes);
        assert!(report.updated.contains(&"clash-workflow".to_string()));
        assert_eq!(
            std::fs::read_to_string(skill.join("SKILL.md")).unwrap(),
            SKILLS[0].render(AgentKind::Hermes)
        );
        assert_eq!(
            std::fs::read_to_string(skill.join("SKILL.md.pre-clash")).unwrap(),
            "imported, older"
        );
        // Managed from now on: a later hand edit is kept, not adopted.
        std::fs::write(skill.join("SKILL.md"), "my edit").unwrap();
        std::fs::remove_file(skill.join("SKILL.md.pre-clash")).unwrap();
        sync_adopting(dir.path(), AgentKind::Hermes);
        assert_eq!(
            std::fs::read_to_string(skill.join("SKILL.md")).unwrap(),
            "my edit"
        );
        assert!(!skill.join("SKILL.md.pre-clash").exists());
        assert!(plan_install_in(dir.path(), AgentKind::Hermes)
            .locally_edited
            .contains(&"clash-workflow".to_string()));
    }

    #[test]
    fn fresh_dir_is_missing_only_and_installs_silently() {
        let dir = TempDir::new().unwrap();
        let plan = plan_install(dir.path(), K);
        assert_eq!(plan.missing.len(), SKILLS.len());
        // A fresh install is not a decision — nothing pre-existing to protect.
        assert!(!plan.needs_decision, "{:?}", plan);

        let report = sync_unattended(dir.path(), K);
        assert_eq!(report.updated.len(), SKILLS.len());
        for skill in SKILLS {
            let path = dir.path().join("skills").join(skill.name).join("SKILL.md");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), skill.render(K));
        }
        // Idempotent, and now fully clean.
        assert!(sync_unattended(dir.path(), K).updated.is_empty());
        let plan = plan_install(dir.path(), K);
        assert!(plan.missing.is_empty() && !plan.needs_decision);
    }

    /// Simulate "a new clash shipped different content": the file on disk
    /// differs from the embedded copy, and the manifest proves whether the
    /// user touched it since clash last wrote it.
    fn simulate(dir: &Path, name: &str, disk: &str, last_written_hash: &str) {
        let d = dir.join("skills").join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), disk).unwrap();
        let mut manifest = read_manifest(&dir.join("skills")).unwrap_or_default();
        manifest
            .skills
            .insert(name.to_string(), last_written_hash.to_string());
        write_manifest(&dir.join("skills"), &manifest);
    }

    /// Simulate "a new clash binary": the fingerprint stamped on disk is the
    /// one a *previous* release resolved, so this embedded set is new. Without
    /// it the plan is already resolved and nothing is a decision — which is
    /// itself the "don't nag me about my own edit" property, pinned below.
    fn simulate_upgrade(dir: &Path) {
        let mut manifest = read_manifest(&dir.join("skills")).unwrap_or_default();
        manifest.resolved_fingerprint = "a previous release".to_string();
        write_manifest(&dir.join("skills"), &manifest);
    }

    #[test]
    fn outdated_vs_locally_edited_is_the_manifest_oracle() {
        let dir = TempDir::new().unwrap();
        sync_unattended(dir.path(), K);
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
        simulate_upgrade(dir.path());
        let plan = plan_install(dir.path(), K);
        assert_eq!(plan.outdated, vec![a.to_string()]);
        assert_eq!(plan.locally_edited, vec![b.to_string()]);
        // The edit is what asks — the outdated one would have gone silently.
        assert!(plan.needs_decision);
    }

    #[test]
    fn an_upstream_only_change_never_asks() {
        // The whole point: a new clash shipping new skill text is applied at
        // startup. Asking there was a popup with nothing at stake in it.
        let dir = TempDir::new().unwrap();
        sync_unattended(dir.path(), K);
        let a = SKILLS[0].name;
        simulate(
            dir.path(),
            a,
            "old shipped content",
            &content_hash("old shipped content"),
        );
        simulate_upgrade(dir.path());
        let plan = plan_install(dir.path(), K);
        assert_eq!(plan.outdated, vec![a.to_string()]);
        assert!(!plan.needs_decision, "{:?}", plan);

        let report = sync_unattended(dir.path(), K);
        assert_eq!(report.updated, vec![a.to_string()]);
        let path = dir.path().join("skills").join(a).join("SKILL.md");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SKILLS[0].render(K));
        // Nothing was left to ask, so the set is stamped and stays quiet.
        let plan = plan_install(dir.path(), K);
        assert!(plan.resolved && plan.outdated.is_empty());
    }

    #[test]
    fn the_unattended_sync_does_not_swallow_a_pending_decision() {
        // It updates the untouched skill and leaves the edited one *and* the
        // question: stamping here would silently drop the human's call.
        let dir = TempDir::new().unwrap();
        sync_unattended(dir.path(), K);
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
        simulate_upgrade(dir.path());
        let report = sync_unattended(dir.path(), K);
        assert_eq!(report.updated, vec![a.to_string()]);
        assert_eq!(report.kept, vec![b.to_string()]);
        let plan = plan_install(dir.path(), K);
        assert!(!plan.resolved, "a pending edit must not be stamped");
        assert_eq!(plan.locally_edited, vec![b.to_string()]);
        assert!(plan.needs_decision);
    }

    #[test]
    fn an_edit_with_nothing_new_shipped_is_left_alone_and_never_asks() {
        // The user's own copy of the *current* skill: clash has nothing to
        // offer, so asking "undo your edit?" every launch is pure nagging —
        // and the unattended sync must not overwrite it either.
        let dir = TempDir::new().unwrap();
        sync_unattended(dir.path(), K);
        let a = SKILLS[0].name;
        simulate(
            dir.path(),
            a,
            "my own variant",
            &content_hash("what clash wrote"),
        );

        let plan = plan_install(dir.path(), K);
        assert_eq!(plan.locally_edited, vec![a.to_string()]);
        assert!(plan.resolved && !plan.needs_decision, "{:?}", plan);

        sync_unattended(dir.path(), K);
        let path = dir.path().join("skills").join(a).join("SKILL.md");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "my own variant");
    }

    #[test]
    fn untouched_mode_updates_outdated_and_keeps_edits() {
        let dir = TempDir::new().unwrap();
        sync_unattended(dir.path(), K);
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

        let report = apply_decision(dir.path(), K, ApplyMode::Untouched);
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
        let plan = plan_install(dir.path(), K);
        assert_eq!(plan.locally_edited, vec![b.to_string()]);
        // …but the decision is stamped: no more popup until the next upgrade.
        assert!(plan.resolved && !plan.needs_decision);
    }

    #[test]
    fn all_mode_overwrites_and_reports_the_edits_it_ate() {
        let dir = TempDir::new().unwrap();
        sync_unattended(dir.path(), K);
        let b = SKILLS[1].name;
        simulate(
            dir.path(),
            b,
            "user's own variant",
            &content_hash("what clash wrote"),
        );

        let report = apply_decision(dir.path(), K, ApplyMode::All);
        assert!(report.updated.contains(&b.to_string()));
        assert_eq!(report.locally_edited, vec![b.to_string()]);
        let b_path = dir.path().join("skills").join(b).join("SKILL.md");
        assert_eq!(
            std::fs::read_to_string(&b_path).unwrap(),
            SKILLS[1].render(K)
        );
        assert!(!plan_install(dir.path(), K).needs_decision);
    }

    #[test]
    fn keep_mode_touches_nothing_but_still_resolves() {
        let dir = TempDir::new().unwrap();
        sync_unattended(dir.path(), K);
        let a = SKILLS[0].name;
        simulate(
            dir.path(),
            a,
            "old shipped content",
            &content_hash("old shipped content"),
        );
        let retired = dir.path().join("skills").join("clash-review");
        std::fs::create_dir_all(&retired).unwrap();

        let report = apply_decision(dir.path(), K, ApplyMode::Keep);
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
        assert!(!plan_install(dir.path(), K).needs_decision);
    }

    #[test]
    fn a_retired_dir_clash_wrote_goes_without_asking() {
        let dir = TempDir::new().unwrap();
        sync_unattended(dir.path(), K);
        // Exactly what clash shipped when the skill still existed.
        simulate(
            dir.path(),
            "clash-review",
            "old harness",
            &content_hash("old harness"),
        );
        simulate_upgrade(dir.path());
        let retired = dir.path().join("skills").join("clash-review");

        let plan = plan_install(dir.path(), K);
        assert_eq!(plan.retired_present, vec!["clash-review".to_string()]);
        assert!(plan.retired_edited.is_empty());
        assert!(!plan.needs_decision, "{:?}", plan);

        let report = sync_unattended(dir.path(), K);
        assert_eq!(report.removed, vec!["clash-review".to_string()]);
        assert!(!retired.exists());
    }

    #[test]
    fn a_hand_edited_retired_dir_asks_and_survives_untouched() {
        let dir = TempDir::new().unwrap();
        sync_unattended(dir.path(), K);
        let retired = dir.path().join("skills").join("clash-review");
        std::fs::create_dir_all(&retired).unwrap();
        std::fs::write(retired.join("SKILL.md"), "my own harness").unwrap();
        simulate_upgrade(dir.path());
        let plan = plan_install(dir.path(), K);
        assert_eq!(plan.retired_edited, vec!["clash-review".to_string()]);
        assert!(plan.needs_decision);

        // The unattended sync must not delete someone's file, and must not
        // stamp the question away either.
        let report = sync_unattended(dir.path(), K);
        assert!(report.removed.is_empty());
        assert!(retired.exists());
        assert!(plan_install(dir.path(), K).needs_decision);

        // `untouched` spares it too; only `all` takes it.
        let report = apply_decision(dir.path(), K, ApplyMode::Untouched);
        assert!(report.removed.is_empty());
        assert_eq!(report.kept, vec!["clash-review".to_string()]);
        assert!(retired.exists());

        let report = apply_decision(dir.path(), K, ApplyMode::All);
        assert_eq!(report.removed, vec!["clash-review".to_string()]);
        assert!(!retired.exists());
    }

    /// Two folders, each with its own `skills/` under `root`: Claude Code's
    /// and Hermes's, synced the way startup syncs them.
    fn two_folders(dir: &Path) -> Vec<(AgentKind, PathBuf)> {
        let folders = vec![
            (AgentKind::Claude, dir.join("cc").join("skills")),
            (AgentKind::Hermes, dir.join("hm").join("skills")),
        ];
        for (kind, d) in &folders {
            sync_unattended_in(d, *kind);
        }
        folders
    }

    #[test]
    fn an_edit_in_another_harness_copy_asks_and_names_its_harness() {
        let dir = TempDir::new().unwrap();
        let folders = two_folders(dir.path());
        assert!(!plan_folders(&folders).needs_decision);
        let a = SKILLS[0].name;
        let hm = dir.path().join("hm");
        simulate(
            &hm,
            a,
            "my Hermes variant",
            &content_hash("what clash wrote"),
        );
        simulate_upgrade(&hm);
        let plan = plan_folders(&folders);
        assert!(plan.needs_decision, "{plan:?}");
        assert_eq!(plan.locally_edited, vec![format!("{a} (Hermes)")]);
        assert!(!plan.resolved);
    }

    #[test]
    fn a_decision_touches_only_the_folders_waiting_for_one() {
        let dir = TempDir::new().unwrap();
        let folders = two_folders(dir.path());
        let a = SKILLS[0].name;
        let (cc, hm) = (dir.path().join("cc"), dir.path().join("hm"));
        // Claude Code: an edit already settled (resolved, nothing pending).
        simulate(
            &cc,
            a,
            "my settled CC edit",
            &content_hash("what clash wrote"),
        );
        // Hermes: an edit an upgrade would overwrite — the pending question.
        simulate(
            &hm,
            a,
            "my Hermes variant",
            &content_hash("what clash wrote"),
        );
        simulate_upgrade(&hm);

        let report = apply_decision_folders(&folders, ApplyMode::All);
        assert_eq!(report.locally_edited, vec![format!("{a} (Hermes)")]);
        let read = |root: &Path| {
            std::fs::read_to_string(root.join("skills").join(a).join("SKILL.md")).unwrap()
        };
        assert_eq!(read(&hm), SKILLS[0].render(AgentKind::Hermes));
        assert_eq!(
            read(&cc),
            "my settled CC edit",
            "a settled folder is not overwritten"
        );
        assert!(!plan_folders(&folders).needs_decision);
    }

    #[test]
    fn one_folder_keeps_bare_names() {
        let dir = TempDir::new().unwrap();
        sync_unattended(dir.path(), K);
        let a = SKILLS[0].name;
        simulate(dir.path(), a, "edit", &content_hash("what clash wrote"));
        simulate_upgrade(dir.path());
        let folders = vec![(K, dir.path().join("skills"))];
        assert_eq!(plan_folders(&folders).locally_edited, vec![a.to_string()]);
    }

    #[test]
    fn installed_folders_lists_only_installed_harnesses() {
        let dir = TempDir::new().unwrap();
        let (cc, omp, hm) = (
            dir.path().join("cc"),
            dir.path().join("omp"),
            dir.path().join("hm"),
        );
        let kinds =
            |f: Vec<(AgentKind, PathBuf)>| f.into_iter().map(|(k, _)| k).collect::<Vec<_>>();
        assert_eq!(
            kinds(installed_folders(&cc, &omp, &hm)),
            vec![AgentKind::Claude]
        );
        std::fs::create_dir_all(&omp).unwrap();
        std::fs::create_dir_all(&hm).unwrap();
        let all = installed_folders(&cc, &omp, &hm);
        assert_eq!(
            kinds(all.clone()),
            vec![AgentKind::Claude, AgentKind::Omp, AgentKind::Hermes]
        );
        assert_eq!(all[2].1, hm.join("skills").join("clash"));
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

    /// Every template renders for every harness, and no rendering names
    /// another harness's tools — that is the whole point of rendering.
    #[test]
    fn every_skill_renders_clean_for_every_harness() {
        let foreign: [(AgentKind, &[&str]); 3] = [
            (
                AgentKind::Claude,
                &["`clarify`", "`delegate_task`", "skill://", "`task` tool"],
            ),
            (
                AgentKind::Omp,
                &[
                    "AskUserQuestion",
                    "`clarify`",
                    "`delegate_task`",
                    "`Agent` tool",
                ],
            ),
            (
                AgentKind::Hermes,
                &["AskUserQuestion", "`Agent` tool", "`task` tool", "skill://"],
            ),
        ];
        for skill in SKILLS {
            for (kind, banned) in foreign {
                let text = skill_template::render(skill.content, kind)
                    .unwrap_or_else(|e| panic!("{} for {:?}: {e}", skill.name, kind));
                for marker in ["{{", "clash:if", "clash:end"] {
                    assert!(
                        !text.contains(marker),
                        "{} for {:?} keeps {marker}",
                        skill.name,
                        kind
                    );
                }
                for word in banned {
                    assert!(
                        !text.contains(word),
                        "{} for {:?} names {word}",
                        skill.name,
                        kind
                    );
                }
            }
        }
    }

    /// Two harnesses' folders hold two renderings, each tracked by its own
    /// manifest: syncing one never makes the other look edited.
    #[test]
    fn each_harness_folder_gets_its_own_rendering() {
        let dir = TempDir::new().unwrap();
        let claude = dir.path().join("claude");
        let hermes = dir.path().join("hermes-clash");
        sync_unattended(&claude, AgentKind::Claude);
        sync_unattended_in(&hermes, AgentKind::Hermes);
        let read =
            |d: &Path| std::fs::read_to_string(d.join(SKILLS[0].name).join("SKILL.md")).unwrap();
        assert_eq!(
            read(&claude.join("skills")),
            SKILLS[0].render(AgentKind::Claude)
        );
        assert_eq!(read(&hermes), SKILLS[0].render(AgentKind::Hermes));
        assert_ne!(read(&claude.join("skills")), read(&hermes));
        for plan in [
            plan_install(&claude, AgentKind::Claude),
            plan_install_in(&hermes, AgentKind::Hermes),
        ] {
            assert!(
                plan.missing.is_empty() && plan.outdated.is_empty(),
                "{plan:?}"
            );
            assert!(plan.locally_edited.is_empty() && plan.resolved, "{plan:?}");
        }
        // Read with the wrong harness, a folder is all differences: callers
        // must pass the harness the folder belongs to.
        assert!(!plan_install(&claude, AgentKind::Hermes).outdated.is_empty());
    }

    #[test]
    fn embedded_skills_have_frontmatter_matching_their_directory() {
        // Each skill must stay loadable by every harness: frontmatter whose name
        // matches the directory it installs into, plus a description (that is
        // what the model matches a request against).
        for skill in SKILLS {
            for kind in skill_template::HARNESSES {
                let content = &skill.render(kind);
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
    }

    /// Every embedded skill is named in the docs.
    ///
    /// The count was spelled out in prose in three places ("the four
    /// skills…"), which is exactly the sentence nobody updates: a fifth skill
    /// shipped while all three still said four. Naming the skills instead of
    /// counting them is what makes the claim checkable, so this asserts each
    /// name appears rather than trying to parse a number word.
    ///
    /// Frontmatter validity is a different property and has its own test
    /// above.
    #[test]
    fn every_embedded_skill_is_named_in_the_docs() {
        const WORKFLOWS_DOC: &str = include_str!("../../docs/workflows.md");
        const README: &str = include_str!("../../README.md");
        for s in SKILLS {
            assert!(
                WORKFLOWS_DOC.contains(s.name),
                "{} is not mentioned in docs/workflows.md",
                s.name
            );
            assert!(
                README.contains(s.name),
                "{} is not mentioned in README.md",
                s.name
            );
        }
    }

    #[test]
    fn every_skill_links_tickets_through_the_kickoff_field() {
        // Every kickoff may carry `Ticket links:`; a skill without the rule
        // would ignore it, or guess a host when the field is absent.
        for skill in SKILLS {
            let content = skill.render(AgentKind::Claude);
            assert!(
                content.contains("## Linking tickets")
                    && content.contains("`Ticket links:`")
                    && content.contains("ask the human for the ticket host"),
                "{} has no ticket-linking rule",
                skill.name
            );
        }
    }

    #[test]
    fn the_executor_knows_every_stacking_clause() {
        // `build_agent_prompt` states stacking in three shapes and says
        // nothing when it is off; the executor opens the PRs, so it must
        // read all four — an unread "off" is how PRs end up stacked anyway.
        let exec = SKILLS
            .iter()
            .find(|s| s.name == "clash-workflow")
            .expect("executor");
        for clause in [
            "**Absent — stacking is off.**",
            "**`Stacking: on.`**",
            "**`Stacking: on (base: <branch>).`**",
            "**`Stacking: on (unclear: <reason>).`**",
        ] {
            assert!(
                exec.content.contains(clause),
                "clash-workflow misses {clause}"
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
