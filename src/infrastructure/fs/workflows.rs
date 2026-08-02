//! Filesystem IO for workflow items.
//!
//! Layout, rooted at the dedicated workflows directory (independent of the
//! scratch tree):
//!
//! ```text
//! <root>/<project>/<slug>/
//! ├── meta.json          # WorkflowMeta — status, PR info, timestamps
//! ├── plan.md            # the plan (freely editable)
//! ├── review.md          # append-only iteration audit trail
//! ├── annotations.json   # AnnotationsFile — line-level diff comments
//! └── history/<NNN>/     # per-iteration snapshots (diff.patch + annotations)
//! ```
//!
//! Unlike scratch notes, file *contents* are read and written here — these
//! are structured, clash-owned documents that the agent co-edits. Every
//! mutation goes through `write_atomic`; `project`/`slug` always pass
//! `sanitize_component`, so nothing can escape the root.

use std::path::{Path, PathBuf};

use crate::domain::error::{DomainError, Result};
use crate::domain::workflow::{
    Annotation, AnnotationStatus, AnnotationsFile, NewWorkflowItem, WorkflowItem, WorkflowMeta,
};
use crate::infrastructure::fs::atomic::write_atomic;
use crate::infrastructure::fs::backend::sanitize_component;

pub const META_FILE: &str = "meta.json";
pub const PLAN_FILE: &str = "plan.md";
pub const REVIEW_FILE: &str = "review.md";
pub const ANNOTATIONS_FILE: &str = "annotations.json";
pub const HISTORY_DIR: &str = "history";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn parse_err(msg: impl Into<String>) -> DomainError {
    DomainError::Parse(msg.into())
}

/// Sanitized absolute item directory. Does not check existence.
fn item_dir(root: &Path, project: &str, slug: &str) -> Result<PathBuf> {
    let project = sanitize_component(project)?;
    let slug = sanitize_component(slug)?;
    Ok(root.join(project).join(slug))
}

/// Existing, sanitized item directory.
fn existing_item_dir(root: &Path, project: &str, slug: &str) -> Result<PathBuf> {
    let dir = item_dir(root, project, slug)?;
    if !dir.is_dir() {
        return Err(parse_err(format!(
            "No workflow item '{}/{}'",
            project, slug
        )));
    }
    Ok(dir)
}

// ── Listing ─────────────────────────────────────────────────────────────

/// Sorted iterations found under `history/` (zero-padded dir names).
fn list_history(dir: &Path) -> Vec<u32> {
    let mut iters: Vec<u32> = std::fs::read_dir(dir.join(HISTORY_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_string_lossy().parse::<u32>().ok())
        .collect();
    iters.sort_unstable();
    iters
}

/// Count `open` annotations in an item dir (0 when the file is absent or
/// malformed — listing must never fail on one bad item).
fn count_open_annotations(dir: &Path) -> usize {
    let path = dir.join(ANNOTATIONS_FILE);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return 0;
    };
    match serde_json::from_str::<AnnotationsFile>(&raw) {
        Ok(file) => file
            .annotations
            .iter()
            .filter(|a| a.status == AnnotationStatus::Open)
            .count(),
        Err(e) => {
            tracing::warn!("malformed {}: {}", path.display(), e);
            0
        }
    }
}

/// Non-empty file check (an empty seeded `review.md` doesn't count as a
/// review yet).
fn has_content(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.len() > 0)
        .unwrap_or(false)
}

/// Build the runtime DTO for one item directory. Identity comes from the
/// directory names, never from `meta.json`.
fn build_item(root: &Path, project: &str, slug: &str) -> Result<WorkflowItem> {
    let dir = existing_item_dir(root, project, slug)?;
    let meta = read_meta(root, project, slug)?;
    // Terminal items skip the per-item annotation/history reads: their open
    // count is irrelevant and the extra IO would grow unboundedly as done
    // items accumulate. Detail views read those files on demand instead.
    let terminal = meta.status.is_terminal();
    let (open_annotations, history_iterations) = if terminal {
        (0, Vec::new())
    } else {
        (count_open_annotations(&dir), list_history(&dir))
    };
    Ok(WorkflowItem {
        project: project.to_string(),
        slug: slug.to_string(),
        path: dir.to_string_lossy().into_owned(),
        has_plan: has_content(&dir.join(PLAN_FILE)),
        has_review: has_content(&dir.join(REVIEW_FILE)),
        open_annotations,
        history_iterations,
        agent_alive: true, // cross-checked against live sessions by the GUI layer
        meta,
    })
}

/// List every workflow item under the root (two-level walk), sorted by
/// project then slug. Directories without a readable `meta.json` are skipped
/// with a warning — one malformed item never fails the list.
pub fn load_items(root: &Path) -> Result<Vec<WorkflowItem>> {
    let mut items = Vec::new();
    if !root.is_dir() {
        return Ok(items);
    }
    for project_entry in std::fs::read_dir(root)?.flatten() {
        if !project_entry.path().is_dir() {
            continue;
        }
        let project = project_entry.file_name().to_string_lossy().into_owned();
        for item_entry in std::fs::read_dir(project_entry.path())?.flatten() {
            if !item_entry.path().is_dir() {
                continue;
            }
            let slug = item_entry.file_name().to_string_lossy().into_owned();
            if !item_entry.path().join(META_FILE).is_file() {
                continue; // not a workflow item
            }
            match build_item(root, &project, &slug) {
                Ok(item) => items.push(item),
                Err(e) => tracing::warn!("skipping workflow item {}/{}: {}", project, slug, e),
            }
        }
    }
    items.sort_by(|a, b| {
        (a.project.as_str(), a.slug.as_str()).cmp(&(b.project.as_str(), b.slug.as_str()))
    });
    Ok(items)
}

// ── Create / delete ─────────────────────────────────────────────────────

/// Create a new item with a slug derived from its title (deduplicated with
/// `-2`, `-3`, … suffixes). Seeds `meta.json` (iteration 1, status from the
/// entry mode), `plan.md` (the request's seed content, usually empty),
/// `review.md`, and an empty `annotations.json`.
pub fn create_item(root: &Path, req: &NewWorkflowItem) -> Result<WorkflowItem> {
    let project = sanitize_component(&req.project)?;
    let base = crate::application::workflow::slugify(&req.title);
    let project_dir = root.join(&project);
    std::fs::create_dir_all(&project_dir)?;

    let mut slug = base.clone();
    let mut n = 1;
    while project_dir.join(&slug).exists() {
        n += 1;
        slug = format!("{}-{}", base, n);
    }
    let dir = project_dir.join(&slug);
    std::fs::create_dir_all(&dir)?;

    let now = now_ms();
    let meta = WorkflowMeta {
        title: req.title.trim().to_string(),
        status: req.mode.initial_status(),
        mode: req.mode,
        repo_path: req.repo_path.clone(),
        branch: req.branch.clone(),
        base: req.base.clone(),
        worktree: req.worktree.clone(),
        pr: req.pr.clone(),
        iteration: 1,
        created_at: now,
        updated_at: now,
        ..WorkflowMeta::default()
    };
    write_atomic(
        &dir.join(META_FILE),
        serde_json::to_string_pretty(&meta)?.as_bytes(),
    )?;
    write_atomic(&dir.join(PLAN_FILE), req.plan.as_bytes())?;
    write_atomic(&dir.join(REVIEW_FILE), b"")?;
    write_atomic(
        &dir.join(ANNOTATIONS_FILE),
        serde_json::to_string_pretty(&AnnotationsFile::default())?.as_bytes(),
    )?;

    build_item(root, &project, &slug)
}

/// Delete an item directory recursively.
pub fn delete_item(root: &Path, project: &str, slug: &str) -> Result<()> {
    let dir = existing_item_dir(root, project, slug)?;
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

// ── meta.json ───────────────────────────────────────────────────────────

pub fn read_meta(root: &Path, project: &str, slug: &str) -> Result<WorkflowMeta> {
    let dir = existing_item_dir(root, project, slug)?;
    let raw = std::fs::read_to_string(dir.join(META_FILE))?;
    Ok(serde_json::from_str(&raw)?)
}

/// Persist `meta.json`, stamping `updated_at`.
pub fn write_meta(root: &Path, project: &str, slug: &str, meta: &WorkflowMeta) -> Result<()> {
    let dir = existing_item_dir(root, project, slug)?;
    let mut meta = meta.clone();
    meta.updated_at = now_ms();
    write_atomic(
        &dir.join(META_FILE),
        serde_json::to_string_pretty(&meta)?.as_bytes(),
    )
    .map_err(DomainError::from)
}

// ── Documents (plan.md / review.md) ─────────────────────────────────────

fn doc_path(dir: &Path, doc: &str) -> Result<PathBuf> {
    match doc {
        PLAN_FILE | REVIEW_FILE => Ok(dir.join(doc)),
        _ => Err(parse_err(format!("Not a workflow document: '{}'", doc))),
    }
}

/// Read `plan.md` or `review.md` (whitelisted). Missing file reads as empty.
pub fn read_doc(root: &Path, project: &str, slug: &str, doc: &str) -> Result<String> {
    let dir = existing_item_dir(root, project, slug)?;
    let path = doc_path(&dir, doc)?;
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}

/// Write `plan.md` or `review.md` (whitelisted), atomically.
pub fn write_doc(root: &Path, project: &str, slug: &str, doc: &str, content: &str) -> Result<()> {
    let dir = existing_item_dir(root, project, slug)?;
    let path = doc_path(&dir, doc)?;
    write_atomic(&path, content.as_bytes()).map_err(DomainError::from)
}

/// Append an iteration section to `review.md` — the human-readable audit
/// trail. Called by request-changes with the user's note and the digest of
/// currently-open annotations.
pub fn append_review_iteration(
    root: &Path,
    project: &str,
    slug: &str,
    iteration: u32,
    note: &str,
    open_annotations: &[Annotation],
) -> Result<()> {
    let existing = read_doc(root, project, slug, REVIEW_FILE)?;
    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    let date = chrono::Local::now().format("%Y-%m-%d %H:%M");
    out.push_str(&format!("\n## Iteration {} — {}\n\n", iteration, date));
    let note = note.trim();
    if !note.is_empty() {
        out.push_str(note);
        out.push('\n');
    }
    if !open_annotations.is_empty() {
        out.push_str("\n### Open annotations\n\n");
        for a in open_annotations {
            out.push_str(&format!("- `{}:{}` — {}\n", a.file, a.line, a.body.trim()));
        }
    }
    write_doc(root, project, slug, REVIEW_FILE, &out)
}

// ── annotations.json ────────────────────────────────────────────────────

/// Read `annotations.json`. Missing file reads as empty; a malformed file is
/// an error (never risk clobbering review data with a blind overwrite).
pub fn read_annotations(root: &Path, project: &str, slug: &str) -> Result<AnnotationsFile> {
    let dir = existing_item_dir(root, project, slug)?;
    match std::fs::read_to_string(dir.join(ANNOTATIONS_FILE)) {
        Ok(raw) => Ok(serde_json::from_str(&raw)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AnnotationsFile::default()),
        Err(e) => Err(e.into()),
    }
}

pub fn write_annotations(
    root: &Path,
    project: &str,
    slug: &str,
    file: &AnnotationsFile,
) -> Result<()> {
    let dir = existing_item_dir(root, project, slug)?;
    write_atomic(
        &dir.join(ANNOTATIONS_FILE),
        serde_json::to_string_pretty(file)?.as_bytes(),
    )
    .map_err(DomainError::from)
}

// ── History snapshots ───────────────────────────────────────────────────

/// Snapshot the current iteration into `history/{iteration:03}/`: the given
/// diff plus a frozen copy of `annotations.json`.
///
/// Does NOT bump `iteration` or touch `meta.json` — the caller performs the
/// single meta write (iteration+1 + status) *after* this succeeds, so a
/// failure here is a clean abort and a retry simply overwrites the orphaned
/// snapshot dir (it is keyed by the un-bumped iteration).
pub fn snapshot_iteration(root: &Path, project: &str, slug: &str, diff: &str) -> Result<u32> {
    let dir = existing_item_dir(root, project, slug)?;
    let meta = read_meta(root, project, slug)?;
    let iter = meta.iteration.max(1);
    let snap_dir = dir.join(HISTORY_DIR).join(format!("{:03}", iter));
    std::fs::create_dir_all(&snap_dir)?;
    write_atomic(&snap_dir.join("diff.patch"), diff.as_bytes())?;
    let annotations = read_annotations(root, project, slug)?;
    write_atomic(
        &snap_dir.join(ANNOTATIONS_FILE),
        serde_json::to_string_pretty(&annotations)?.as_bytes(),
    )?;
    Ok(iter)
}

/// List snapshotted iterations (detail views need this even for terminal
/// items, whose listing DTO skips it).
pub fn history_iterations(root: &Path, project: &str, slug: &str) -> Result<Vec<u32>> {
    let dir = existing_item_dir(root, project, slug)?;
    Ok(list_history(&dir))
}

/// Read a snapshotted diff from `history/{iteration:03}/diff.patch`.
pub fn read_history_diff(root: &Path, project: &str, slug: &str, iteration: u32) -> Result<String> {
    let dir = existing_item_dir(root, project, slug)?;
    let path = dir
        .join(HISTORY_DIR)
        .join(format!("{:03}", iteration))
        .join("diff.patch");
    std::fs::read_to_string(&path).map_err(|e| {
        parse_err(format!(
            "No snapshot for iteration {} ({}): {}",
            iteration,
            path.display(),
            e
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::WorkflowStatus;
    use tempfile::TempDir;

    fn root() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("workflows");
        (dir, root)
    }

    /// Plain full-mode creation request — the shape most tests need.
    fn req(project: &str, title: &str, repo_path: &str) -> NewWorkflowItem {
        NewWorkflowItem {
            project: project.to_string(),
            title: title.to_string(),
            repo_path: repo_path.to_string(),
            ..NewWorkflowItem::default()
        }
    }

    #[test]
    fn create_then_load_round_trip() {
        let (_g, root) = root();
        let item = create_item(&root, &req("clash", "Auth Refactor!", "/w/clash")).unwrap();
        assert_eq!(item.project, "clash");
        assert_eq!(item.slug, "auth-refactor");
        assert_eq!(item.meta.title, "Auth Refactor!");
        assert_eq!(item.meta.status, WorkflowStatus::Draft);
        assert_eq!(item.meta.iteration, 1);
        assert!(!item.has_plan); // seeded empty
        assert!(!item.has_review);
        assert_eq!(item.open_annotations, 0);

        let items = load_items(&root).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].slug, "auth-refactor");
        assert!(items[0].path.ends_with("clash/auth-refactor"));
    }

    #[test]
    fn create_from_plan_seeds_the_plan_and_lands_on_review() {
        use crate::domain::workflow::WorkflowMode;
        let (_g, root) = root();
        let item = create_item(
            &root,
            &NewWorkflowItem {
                mode: WorkflowMode::FromPlan,
                plan: "# Plan\n\nDo the thing.\n".to_string(),
                ..req("p", "Supplied plan", "/w/p")
            },
        )
        .unwrap();
        assert_eq!(item.meta.mode, WorkflowMode::FromPlan);
        assert_eq!(item.meta.status, WorkflowStatus::PlanReview);
        assert!(item.has_plan);
        assert_eq!(
            read_doc(&root, "p", &item.slug, PLAN_FILE).unwrap(),
            "# Plan\n\nDo the thing.\n"
        );
        // One approval away from implementation.
        assert!(item
            .meta
            .status
            .can_transition_to(WorkflowStatus::Implementing));
    }

    #[test]
    fn create_review_only_starts_at_diff_review_with_the_existing_code() {
        use crate::domain::workflow::{WorkflowMode, WorkflowPr};
        let (_g, root) = root();
        let item = create_item(
            &root,
            &NewWorkflowItem {
                mode: WorkflowMode::ReviewOnly,
                branch: "feat/thing".to_string(),
                base: "develop".to_string(),
                worktree: Some("/w/p-worktrees/feat-thing".to_string()),
                pr: Some(WorkflowPr {
                    url: "https://github.com/o/r/pull/9".to_string(),
                    number: 9,
                    ..WorkflowPr::default()
                }),
                ..req("p", "Review the thing", "/w/p")
            },
        )
        .unwrap();
        assert_eq!(item.meta.mode, WorkflowMode::ReviewOnly);
        assert_eq!(item.meta.status, WorkflowStatus::DiffReview);
        assert_eq!(item.meta.branch, "feat/thing");
        assert_eq!(item.meta.base, "develop");
        assert_eq!(
            item.meta.worktree.as_deref(),
            Some("/w/p-worktrees/feat-thing")
        );
        assert_eq!(item.meta.pr.as_ref().unwrap().number, 9);
        assert!(!item.has_plan); // review-only never gets a plan

        // Everything survives the round-trip through meta.json.
        let back = read_meta(&root, "p", &item.slug).unwrap();
        assert_eq!(back.mode, WorkflowMode::ReviewOnly);
        assert_eq!(back.base, "develop");
    }

    #[test]
    fn slug_dedup_appends_counter() {
        let (_g, root) = root();
        assert_eq!(
            create_item(&root, &req("p", "Same", "")).unwrap().slug,
            "same"
        );
        assert_eq!(
            create_item(&root, &req("p", "Same", "")).unwrap().slug,
            "same-2"
        );
        assert_eq!(
            create_item(&root, &req("p", "Same", "")).unwrap().slug,
            "same-3"
        );
    }

    #[test]
    fn sanitization_rejects_traversal() {
        let (_g, root) = root();
        assert!(create_item(&root, &req("../evil", "x", "")).is_err());
        assert!(read_meta(&root, "p", "../../etc").is_err());
        assert!(read_doc(&root, "p/x", "s", PLAN_FILE).is_err());
    }

    #[test]
    fn doc_whitelist_enforced() {
        let (_g, root) = root();
        create_item(&root, &req("p", "item", "")).unwrap();
        assert!(write_doc(&root, "p", "item", "meta.json", "{}").is_err());
        assert!(read_doc(&root, "p", "item", "../plan.md").is_err());
        write_doc(&root, "p", "item", PLAN_FILE, "# Plan").unwrap();
        assert_eq!(read_doc(&root, "p", "item", PLAN_FILE).unwrap(), "# Plan");
        // has_plan flips once content exists.
        assert!(load_items(&root).unwrap()[0].has_plan);
    }

    #[test]
    fn meta_write_stamps_updated_at() {
        let (_g, root) = root();
        let item = create_item(&root, &req("p", "item", "")).unwrap();
        let mut meta = item.meta.clone();
        meta.status = WorkflowStatus::Planning;
        meta.updated_at = 0;
        write_meta(&root, "p", "item", &meta).unwrap();
        let back = read_meta(&root, "p", "item").unwrap();
        assert_eq!(back.status, WorkflowStatus::Planning);
        assert!(back.updated_at > 0);
        assert_eq!(back.created_at, item.meta.created_at);
    }

    #[test]
    fn annotations_round_trip_and_open_count() {
        let (_g, root) = root();
        create_item(&root, &req("p", "item", "")).unwrap();
        let mut file = AnnotationsFile::default();
        file.annotations.push(Annotation {
            id: "a-1".into(),
            file: "src/lib.rs".into(),
            line: 3,
            body: "open one".into(),
            ..Annotation::default()
        });
        file.annotations.push(Annotation {
            id: "a-2".into(),
            status: AnnotationStatus::Addressed,
            ..Annotation::default()
        });
        write_annotations(&root, "p", "item", &file).unwrap();
        let back = read_annotations(&root, "p", "item").unwrap();
        assert_eq!(back.annotations.len(), 2);
        assert_eq!(load_items(&root).unwrap()[0].open_annotations, 1);
    }

    #[test]
    fn malformed_annotations_error_on_read_but_not_listing() {
        let (_g, root) = root();
        let item = create_item(&root, &req("p", "item", "")).unwrap();
        std::fs::write(Path::new(&item.path).join(ANNOTATIONS_FILE), "{not json").unwrap();
        // Direct read errors (a blind save would clobber review data)…
        assert!(read_annotations(&root, "p", "item").is_err());
        // …but the listing survives with a zero count.
        let items = load_items(&root).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].open_annotations, 0);
    }

    #[test]
    fn malformed_meta_skips_item_not_list() {
        let (_g, root) = root();
        create_item(&root, &req("p", "good", "")).unwrap();
        let bad = root.join("p").join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join(META_FILE), "{not json").unwrap();
        // A dir without meta.json is not an item at all.
        std::fs::create_dir_all(root.join("p").join("not-an-item")).unwrap();

        let items = load_items(&root).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].slug, "good");
    }

    #[test]
    fn snapshot_writes_history_and_leaves_meta_alone() {
        let (_g, root) = root();
        let item = create_item(&root, &req("p", "item", "")).unwrap();
        let iter = snapshot_iteration(&root, "p", "item", "diff --git a/x b/x\n").unwrap();
        assert_eq!(iter, 1);
        let snap = Path::new(&item.path).join("history").join("001");
        assert!(snap.join("diff.patch").is_file());
        assert!(snap.join(ANNOTATIONS_FILE).is_file());
        // meta untouched — the caller owns the iteration bump.
        assert_eq!(read_meta(&root, "p", "item").unwrap().iteration, 1);
        assert_eq!(
            read_history_diff(&root, "p", "item", 1).unwrap(),
            "diff --git a/x b/x\n"
        );

        // Retry after a partial failure overwrites the same snapshot dir.
        let again = snapshot_iteration(&root, "p", "item", "diff v2\n").unwrap();
        assert_eq!(again, 1);
        assert_eq!(
            read_history_diff(&root, "p", "item", 1).unwrap(),
            "diff v2\n"
        );

        // Listing picks up the snapshot.
        assert_eq!(load_items(&root).unwrap()[0].history_iterations, vec![1]);
    }

    #[test]
    fn terminal_items_skip_annotation_and_history_reads() {
        let (_g, root) = root();
        let item = create_item(&root, &req("p", "item", "")).unwrap();
        snapshot_iteration(&root, "p", "item", "d").unwrap();
        let mut file = AnnotationsFile::default();
        file.annotations.push(Annotation::default()); // open by default
        write_annotations(&root, "p", "item", &file).unwrap();

        let mut meta = item.meta.clone();
        meta.status = WorkflowStatus::Done;
        write_meta(&root, "p", "item", &meta).unwrap();

        let listed = &load_items(&root).unwrap()[0];
        assert_eq!(listed.open_annotations, 0);
        assert!(listed.history_iterations.is_empty());
    }

    #[test]
    fn review_audit_trail_appends() {
        let (_g, root) = root();
        create_item(&root, &req("p", "item", "")).unwrap();
        let ann = Annotation {
            file: "src/a.rs".into(),
            line: 12,
            body: "rename this".into(),
            ..Annotation::default()
        };
        append_review_iteration(&root, "p", "item", 1, "Tighten the API", &[ann]).unwrap();
        append_review_iteration(&root, "p", "item", 2, "Second round", &[]).unwrap();
        let review = read_doc(&root, "p", "item", REVIEW_FILE).unwrap();
        assert!(review.contains("## Iteration 1"));
        assert!(review.contains("Tighten the API"));
        assert!(review.contains("`src/a.rs:12` — rename this"));
        assert!(review.contains("## Iteration 2"));
        let one = review.find("## Iteration 1").unwrap();
        let two = review.find("## Iteration 2").unwrap();
        assert!(one < two, "append-only ordering");
    }

    #[test]
    fn missing_history_diff_is_a_clear_error() {
        let (_g, root) = root();
        create_item(&root, &req("p", "item", "")).unwrap();
        let err = read_history_diff(&root, "p", "item", 7).unwrap_err();
        assert!(err.to_string().contains("iteration 7"));
    }

    #[test]
    fn delete_item_removes_recursively() {
        let (_g, root) = root();
        create_item(&root, &req("p", "item", "")).unwrap();
        snapshot_iteration(&root, "p", "item", "d").unwrap();
        delete_item(&root, "p", "item").unwrap();
        assert!(load_items(&root).unwrap().is_empty());
        assert!(delete_item(&root, "p", "item").is_err());
    }
}
