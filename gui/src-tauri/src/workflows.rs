//! Tauri command layer for the Workflows feature.
//!
//! Thin adapters over the shared core: storage goes through the
//! `WorkflowRepository` port on `FsBackend`, pure logic (anchoring, attention
//! transitions) lives in `clash::application::workflow`. Nothing in here owns
//! business rules beyond wiring.

use std::collections::HashSet;
use std::path::Path;

use clash::application::diff::parse_file_diffs;
use clash::application::workflow::anchor_annotations;
use clash::application::workflow::AnchoredAnnotation;
use clash::domain::ports::WorkflowRepository;
use clash::domain::workflow::{
    AnnotationsFile, ReviewDepth, ReviewPublish, ReviewTarget, WorkflowItem, WorkflowMode,
    WorkflowReview, WorkflowStatus,
};
use tauri::{Emitter, Manager, State};

use crate::{native_notify, GuiState};

fn e2s(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Payload for `workflow-attention` events (sidebar badges + toasts).
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkflowAttention {
    project: String,
    slug: String,
    title: String,
    status: WorkflowStatus,
    /// The status the item left — `reviewing` means a round just handed back.
    from: WorkflowStatus,
    /// The round that just finished, when `from` was `reviewing` — lets the
    /// toast state the verdict and what was published instead of a bare
    /// "decision needed".
    #[serde(skip_serializing_if = "Option::is_none")]
    review: Option<clash::domain::workflow::AgentReviewSummary>,
}

/// List every workflow item, decorated with the agent-liveness cross-check,
/// and fire attention notifications for *external* transitions into
/// decision-needed states (the `AttentionLedger` suppresses clash's own
/// writes — see the mutating commands, which pre-seed it).
#[tauri::command]
pub(crate) async fn list_workflow_items(
    app: tauri::AppHandle,
    state: State<'_, GuiState>,
) -> Result<Vec<WorkflowItem>, String> {
    let mut items = state.backend.load_workflow_items().map_err(e2s)?;

    // A dead agent session must not leave an item claiming "agent working"
    // forever: cross-reference against the last session list (the
    // `rebuild_all_members` liveness precedent).
    let live: HashSet<String> = state
        .previous
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.is_running)
        .map(|s| s.id.clone())
        .collect();
    for item in &mut items {
        // `Reviewing` is included so a dead reviewer surfaces the same "the
        // agent is gone" affordance — without it a crashed round would leave
        // the item gated with no visible way out.
        if matches!(
            item.meta.status,
            WorkflowStatus::Planning | WorkflowStatus::Implementing | WorkflowStatus::Reviewing
        ) {
            item.agent_alive = item
                .meta
                .session_id
                .as_ref()
                .is_some_and(|sid| live.contains(sid));
        }
    }

    let events = state.attention.lock().unwrap().observe(&items);
    if !events.is_empty() {
        let focused = app
            .get_webview_window("main")
            .and_then(|w| w.is_focused().ok())
            .unwrap_or(false);
        for ev in events {
            let review = (ev.from == WorkflowStatus::Reviewing)
                .then(|| {
                    items
                        .iter()
                        .find(|i| i.project == ev.project && i.slug == ev.slug)
                        .and_then(|i| i.last_agent_review.clone())
                })
                .flatten();
            let _ = app.emit(
                "workflow-attention",
                WorkflowAttention {
                    project: ev.project.clone(),
                    slug: ev.slug.clone(),
                    title: ev.title.clone(),
                    status: ev.status,
                    from: ev.from,
                    review,
                },
            );
            if !focused
                && state
                    .notify_enabled
                    .load(std::sync::atomic::Ordering::Relaxed)
            {
                let what = if ev.from == WorkflowStatus::Reviewing {
                    "review round finished — read the findings"
                } else {
                    match ev.status {
                        WorkflowStatus::PlanReview => "plan ready for review",
                        WorkflowStatus::DiffReview => "changes ready for review",
                        WorkflowStatus::PrDraft => "draft PR awaiting validation",
                        WorkflowStatus::PrReady => "PR ready — merge or keep iterating",
                        _ => "needs your decision",
                    }
                };
                let title = if ev.title.is_empty() {
                    ev.slug.clone()
                } else {
                    ev.title.clone()
                };
                native_notify(&format!("clash · {}", title), what);
            }
        }
    }

    Ok(items)
}

/// Read `plan.md` or `review.md` (whitelisted by the port).
#[tauri::command]
pub(crate) fn get_workflow_doc(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    doc: String,
) -> Result<String, String> {
    state
        .backend
        .read_workflow_doc(&project, &slug, &doc)
        .map_err(e2s)
}

/// Read `annotations.json` verbatim (the raw threads, unanchored).
#[tauri::command]
pub(crate) fn get_workflow_annotations(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
) -> Result<AnnotationsFile, String> {
    state
        .backend
        .load_workflow_annotations(&project, &slug)
        .map_err(e2s)
}

/// Snapshotted iterations available under `history/`.
#[tauri::command]
pub(crate) fn list_workflow_history(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
) -> Result<Vec<u32>, String> {
    state
        .backend
        .list_workflow_history(&project, &slug)
        .map_err(e2s)
}

/// The diff text for an item: a history snapshot when `iteration` is given,
/// otherwise the live diff of the item's worktree/repo against the merge-base
/// with its base branch (committed + uncommitted work).
///
/// The base is `meta.base` when recorded — a review-only item created from a PR
/// carries that PR's target branch, which is not necessarily the repo default —
/// and the origin default branch otherwise.
pub(crate) async fn workflow_diff_text(
    state: &GuiState,
    project: &str,
    slug: &str,
    iteration: Option<u32>,
) -> Result<String, String> {
    if let Some(iter) = iteration {
        return state
            .backend
            .read_workflow_history_diff(project, slug, iter)
            .map_err(e2s);
    }
    let meta = state
        .backend
        .load_workflow_meta(project, slug)
        .map_err(e2s)?;
    let dir = meta
        .worktree
        .clone()
        .filter(|w| !w.is_empty())
        .unwrap_or_else(|| meta.repo_path.clone());
    if dir.is_empty() {
        return Err("No repository directory recorded for this item".to_string());
    }
    let base = if meta.base.trim().is_empty() {
        crate::origin_default_branch(&dir).await
    } else {
        meta.base.clone()
    };
    clash::infrastructure::git::git_diff(
        Path::new(&dir),
        &clash::infrastructure::git::DiffBase::MergeBase(base),
    )
    .await
}

/// Raw unified diff for the item (live or a history iteration).
#[tauri::command]
pub(crate) async fn get_workflow_diff(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    iteration: Option<u32>,
) -> Result<String, String> {
    workflow_diff_text(&state, &project, &slug, iteration).await
}

/// Annotations resolved against the current (or a snapshotted) diff — the
/// single anchor implementation lives in the core; the GUI just renders
/// `currentLine` / `orphaned`.
#[tauri::command]
pub(crate) async fn get_anchored_annotations(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    iteration: Option<u32>,
) -> Result<Vec<AnchoredAnnotation>, String> {
    let diff = workflow_diff_text(&state, &project, &slug, iteration).await?;
    let annotations = state
        .backend
        .load_workflow_annotations(&project, &slug)
        .map_err(e2s)?;
    Ok(anchor_annotations(
        &parse_file_diffs(&diff),
        &annotations.annotations,
    ))
}

/// Current workflows directory (absolute path) for the Settings field.
#[tauri::command]
pub(crate) fn get_workflows_dir(state: State<'_, GuiState>) -> String {
    state.backend.workflows_dir().to_string_lossy().into_owned()
}

/// Set (or reset) the workflows directory. An empty path resets to the
/// default. Persists to the shared `config.toml`, applies live to the
/// running backend, and re-points the watcher. Returns the effective path.
#[tauri::command]
pub(crate) fn set_workflows_dir(
    state: State<'_, GuiState>,
    app: tauri::AppHandle,
    path: String,
) -> Result<String, String> {
    let effective =
        crate::set_path_setting(&state, "paths.workflows_dir", &path, |c| c.workflows_dir())?;
    state.backend.set_workflows_dir(effective.clone());
    // Rebuild the single routed watcher so the new directory is watched.
    crate::rebuild_watcher(&app);
    Ok(effective.to_string_lossy().into_owned())
}

// ── Write path ──────────────────────────────────────────────────────────

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Record a status clash itself just wrote, so the watcher-triggered reload
/// never notifies the user about their own click.
fn seed_local(state: &GuiState, project: &str, slug: &str, status: WorkflowStatus) {
    state
        .attention
        .lock()
        .unwrap()
        .record_local_write(project, slug, status);
}

/// Annotation phase-lock (review A2): the agent owns `annotations.json`
/// while it works; the GUI owns it during review phases. Enforced on every
/// mutating annotation command — the frontend shows the same lock as a
/// banner.
fn ensure_annotations_unlocked(meta: &clash::domain::workflow::WorkflowMeta) -> Result<(), String> {
    if matches!(
        meta.status,
        WorkflowStatus::ChangesRequested | WorkflowStatus::Implementing | WorkflowStatus::Reviewing
    ) {
        Err("agent is working — annotations are locked until it finishes".to_string())
    } else {
        Ok(())
    }
}

/// Create a workflow item in `full` or `from-plan` mode (iteration 1; the mode
/// picks the initial status). `plan_file` seeds `plan.md` from a file on disk —
/// that is how both the "a markdown file" and the "a scratch note" plan sources
/// work (the frontend passes the note's absolute path); `plan` carries pasted
/// text. Review-only items go through [`create_workflow_review`], which has git
/// work to do first.
#[tauri::command]
pub(crate) fn create_workflow_item(
    state: State<'_, GuiState>,
    project: String,
    title: String,
    repo_path: String,
    mode: Option<WorkflowMode>,
    plan: Option<String>,
    plan_file: Option<String>,
) -> Result<WorkflowItem, String> {
    let mode = mode.unwrap_or_default();
    if mode.is_review_only() {
        return Err("review-only items are created by create_workflow_review".to_string());
    }
    let plan = read_plan_seed(plan, plan_file)?;
    if mode == WorkflowMode::FromPlan && plan.trim().is_empty() {
        return Err("A from-plan item needs a plan — the supplied plan is empty".to_string());
    }
    let item = state
        .backend
        .create_workflow_item(&clash::domain::workflow::NewWorkflowItem {
            project,
            title,
            repo_path,
            mode,
            plan,
            ..Default::default()
        })
        .map_err(e2s)?;
    seed_local(&state, &item.project, &item.slug, item.meta.status);
    Ok(item)
}

/// Resolve the plan seed: a file's content when `plan_file` is given, else the
/// pasted text, else empty. Reading happens here rather than in the frontend so
/// no generic read-any-file command has to be exposed to the webview.
fn read_plan_seed(plan: Option<String>, plan_file: Option<String>) -> Result<String, String> {
    let Some(path) = plan_file
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
    else {
        return Ok(plan.unwrap_or_default());
    };
    let path = crate::expand_tilde(&path);
    std::fs::read_to_string(&path).map_err(|e| format!("Cannot read {}: {}", path.display(), e))
}

/// Create a review-only item over code that already exists: a PR (by URL or
/// number) or a local branch. Resolves the PR through `gh`, materializes a
/// checkout of the branch, and lands the item straight in `diff-review` — no
/// planning, no implementation phase, just the review loop.
///
/// The checkout is done *before* creating the item so a failure (missing branch,
/// gh unavailable, dirty worktree) surfaces as "couldn't start the review"
/// instead of leaving a half-usable item on disk.
#[tauri::command]
pub(crate) async fn create_workflow_review(
    state: State<'_, GuiState>,
    project: String,
    repo_path: String,
    pr: Option<String>,
    branch: Option<String>,
    title: Option<String>,
) -> Result<WorkflowItem, String> {
    let repo = crate::expand_tilde(repo_path.trim());
    if !repo.is_dir() {
        return Err(format!("Not a directory: {}", repo.display()));
    }
    let pr_selector = pr.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    let branch = branch
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty());

    // Resolve the source into (branch, base, pr block, fallback title).
    let (head, base, pr_block, pr_title) = match pr_selector {
        Some(selector) => {
            // A PR *number* is only meaningful against a repo. Parse the URL for
            // its `owner/repo` and pass that to gh explicitly — dropping it and
            // resolving the bare number in `repo` would hit a different PR (or
            // none) whenever the link came from another repository.
            let (selector, pr_repo) = match clash::infrastructure::gh::parse_pr_url(&selector) {
                Some((slug, number)) => (number.to_string(), Some(slug)),
                None if selector.chars().all(|c| c.is_ascii_digit()) => (selector, None),
                None => return Err(format!("Not a GitHub PR URL or number: {}", selector)),
            };
            // The review checkout fetches `refs/pull/<n>/head` from *this* repo's
            // origin, so a PR from elsewhere can't be materialized here — say so
            // instead of fetching an unrelated PR that happens to share the number.
            if let (Some(pr_repo), Some(local)) = (
                pr_repo.as_deref(),
                clash::infrastructure::git::review::origin_repo_slug(&repo).await,
            ) {
                if !pr_repo.eq_ignore_ascii_case(&local) {
                    return Err(format!(
                        "That PR is in {} but the selected repository is {} ({}). \
                         Pick the {} repository, or paste a PR from {}.",
                        pr_repo,
                        local,
                        repo.display(),
                        pr_repo,
                        local
                    ));
                }
            }
            let dir = repo.clone();
            let sel = selector.clone();
            let scope = pr_repo.clone();
            let view = tauri::async_runtime::spawn_blocking(move || {
                clash::infrastructure::gh::pr_view_scoped(&dir, &sel, scope.as_deref())
            })
            .await
            .map_err(|e| e.to_string())?
            .map_err(gh_err)?;
            if view.head_ref_name.is_empty() {
                return Err(format!(
                    "gh did not report a head branch for PR {}",
                    selector
                ));
            }
            let block = clash::domain::workflow::WorkflowPr {
                url: view.url.clone(),
                number: view.number,
                draft: view.is_draft,
                state: view.state.clone(),
                last_checked_at: now_ms(),
                ..Default::default()
            };
            (
                view.head_ref_name.clone(),
                view.base_ref_name.clone(),
                Some(block),
                view.title.clone(),
            )
        }
        None => {
            let head = branch.ok_or_else(|| "No PR or branch to review".to_string())?;
            let base = crate::origin_default_branch(&repo.to_string_lossy()).await;
            (head, base, None, String::new())
        }
    };

    let worktree = clash::infrastructure::git::review::checkout_for_review(
        &repo,
        &head,
        pr_block.as_ref().map(|p| p.number),
    )
    .await?;

    let title = title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .or_else(|| (!pr_title.is_empty()).then_some(pr_title))
        .unwrap_or_else(|| head.clone());

    let item = state
        .backend
        .create_workflow_item(&clash::domain::workflow::NewWorkflowItem {
            project,
            title,
            repo_path: repo.to_string_lossy().into_owned(),
            mode: WorkflowMode::ReviewOnly,
            branch: head,
            base,
            worktree: Some(worktree),
            pr: pr_block,
            ..Default::default()
        })
        .map_err(e2s)?;
    seed_local(&state, &item.project, &item.slug, item.meta.status);
    Ok(item)
}

/// Local branches of a repo, newest first — the picker for "review a branch".
#[tauri::command]
pub(crate) async fn list_repo_branches(
    repo_path: String,
) -> Result<Vec<clash::infrastructure::git::review::LocalBranch>, String> {
    clash::infrastructure::git::review::list_local_branches(&crate::expand_tilde(repo_path.trim()))
        .await
}

/// Human-initiated status transition, validated against the transition
/// table. An optional note is appended to the review.md audit trail when the
/// target is `changes-requested` (the plan-review "request changes" path —
/// the diff-review path goes through `workflow_request_changes` instead).
#[tauri::command]
pub(crate) fn update_workflow_status(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    status: WorkflowStatus,
    note: Option<String>,
) -> Result<clash::domain::workflow::WorkflowMeta, String> {
    let mut meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    if !meta.status.can_transition_to(status) {
        return Err(format!(
            "cannot transition from {} to {}",
            meta.status, status
        ));
    }
    let note = note.unwrap_or_default();
    if status == WorkflowStatus::ChangesRequested && !note.trim().is_empty() {
        state
            .backend
            .append_workflow_review_iteration(&project, &slug, meta.iteration, &note, &[])
            .map_err(e2s)?;
    }
    meta.status = status;
    state
        .backend
        .write_workflow_meta(&project, &slug, &meta)
        .map_err(e2s)?;
    seed_local(&state, &project, &slug, status);
    Ok(meta)
}

/// Write `plan.md` or `review.md`.
#[tauri::command]
pub(crate) fn save_workflow_doc(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    doc: String,
    content: String,
) -> Result<(), String> {
    state
        .backend
        .write_workflow_doc(&project, &slug, &doc, &content)
        .map_err(e2s)
}

/// Upsert an annotation (by id; empty id = new). The backend owns hashing:
/// `line_content_hash` is always recomputed from `line_content` here, so
/// there is exactly one hash implementation (FNV-1a in the core).
#[tauri::command]
pub(crate) fn save_workflow_annotation(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    mut annotation: clash::domain::workflow::Annotation,
) -> Result<AnnotationsFile, String> {
    let meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    ensure_annotations_unlocked(&meta)?;

    let now = now_ms();
    if annotation.id.trim().is_empty() {
        annotation.id = format!("a-{}", uuid::Uuid::now_v7());
        annotation.created_at = now;
        if annotation.iteration == 0 {
            annotation.iteration = meta.iteration;
        }
    }
    annotation.updated_at = now;
    if annotation.author.is_empty() {
        annotation.author = "user".to_string();
    }
    annotation.line_content_hash =
        clash::application::workflow::line_hash(&annotation.line_content);

    let mut file = state
        .backend
        .load_workflow_annotations(&project, &slug)
        .map_err(e2s)?;
    match file.annotations.iter_mut().find(|a| a.id == annotation.id) {
        Some(existing) => {
            // Preserve creation metadata on edits.
            annotation.created_at = existing.created_at;
            *existing = annotation;
        }
        None => file.annotations.push(annotation),
    }
    state
        .backend
        .write_workflow_annotations(&project, &slug, &file)
        .map_err(e2s)?;
    Ok(file)
}

/// Change one annotation's resolution state (open / addressed / wontfix).
#[tauri::command]
pub(crate) fn set_workflow_annotation_status(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    id: String,
    status: clash::domain::workflow::AnnotationStatus,
) -> Result<AnnotationsFile, String> {
    let meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    ensure_annotations_unlocked(&meta)?;
    let mut file = state
        .backend
        .load_workflow_annotations(&project, &slug)
        .map_err(e2s)?;
    let ann = file
        .annotations
        .iter_mut()
        .find(|a| a.id == id)
        .ok_or_else(|| format!("No annotation '{}'", id))?;
    ann.status = status;
    ann.updated_at = now_ms();
    state
        .backend
        .write_workflow_annotations(&project, &slug, &file)
        .map_err(e2s)?;
    Ok(file)
}

/// Delete an annotation thread.
#[tauri::command]
pub(crate) fn delete_workflow_annotation(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    id: String,
) -> Result<AnnotationsFile, String> {
    let meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    ensure_annotations_unlocked(&meta)?;
    let mut file = state
        .backend
        .load_workflow_annotations(&project, &slug)
        .map_err(e2s)?;
    let before = file.annotations.len();
    file.annotations.retain(|a| a.id != id);
    if file.annotations.len() == before {
        return Err(format!("No annotation '{}'", id));
    }
    state
        .backend
        .write_workflow_annotations(&project, &slug, &file)
        .map_err(e2s)?;
    Ok(file)
}

/// The diff-review "request changes" flow, ordered for crash-safety
/// (review C2): snapshot the current diff + annotations into
/// `history/{iteration:03}/`, append the note + open-annotation digest to
/// review.md, then ONE meta write carrying both `iteration+1` and status
/// `changes-requested`. Any failure before the meta write aborts cleanly; a
/// retry overwrites the orphaned snapshot.
#[tauri::command]
pub(crate) async fn workflow_request_changes(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    note: Option<String>,
) -> Result<clash::domain::workflow::WorkflowMeta, String> {
    let mut meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    if !meta
        .status
        .can_transition_to(WorkflowStatus::ChangesRequested)
    {
        return Err(format!(
            "cannot request changes from status {}",
            meta.status
        ));
    }

    let diff = workflow_diff_text(&state, &project, &slug, None)
        .await
        .unwrap_or_default();
    let snapped = state
        .backend
        .snapshot_workflow_iteration(&project, &slug, &diff)
        .map_err(e2s)?;

    let open: Vec<clash::domain::workflow::Annotation> = state
        .backend
        .load_workflow_annotations(&project, &slug)
        .map_err(e2s)?
        .annotations
        .into_iter()
        .filter(|a| a.status == clash::domain::workflow::AnnotationStatus::Open)
        .collect();
    state
        .backend
        .append_workflow_review_iteration(
            &project,
            &slug,
            snapped,
            note.as_deref().unwrap_or(""),
            &open,
        )
        .map_err(e2s)?;

    meta.iteration = snapped + 1;
    meta.status = WorkflowStatus::ChangesRequested;
    state
        .backend
        .write_workflow_meta(&project, &slug, &meta)
        .map_err(e2s)?;
    seed_local(&state, &project, &slug, WorkflowStatus::ChangesRequested);
    Ok(meta)
}

/// The plan change of one iteration: snapshot `it` against snapshot `it+1`
/// when one exists, else against the current `plan.md`. Empty string when the
/// plan did not change (or the snapshot predates plan snapshotting).
#[tauri::command]
pub(crate) fn get_workflow_plan_diff(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    iteration: u32,
) -> Result<String, String> {
    let old = state
        .backend
        .read_workflow_history_plan(&project, &slug, iteration)
        .map_err(e2s)?;
    let Some(old) = old else {
        return Ok(String::new());
    };
    let (new, new_label) = match state
        .backend
        .read_workflow_history_plan(&project, &slug, iteration + 1)
        .map_err(e2s)?
    {
        Some(next) => (next, format!("plan.md (it.{})", iteration + 1)),
        None => (
            state
                .backend
                .read_workflow_doc(
                    &project,
                    &slug,
                    clash::infrastructure::fs::workflows::PLAN_FILE,
                )
                .map_err(e2s)?,
            "plan.md (current)".to_string(),
        ),
    };
    Ok(clash::application::diff::unified_diff(
        &old,
        &new,
        &format!("plan.md (it.{})", iteration),
        &new_label,
    ))
}

/// Delete a whole workflow item (used by Abandon → Delete).
#[tauri::command]
pub(crate) fn delete_workflow_item(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
) -> Result<(), String> {
    state
        .backend
        .delete_workflow_item(&project, &slug)
        .map_err(e2s)
}

// ── Agent launch ────────────────────────────────────────────────────────

/// Spawn a Claude Code session for this item in `cwd`, with a kickoff prompt
/// built from the item directory. Reuses the exact session machinery of
/// `create_new_session`/`create_worktree_session`: registry + name + status
/// files, then a daemon spawn with `--session-id` and the prompt as the
/// positional initial argument.
///
/// Shared by the executor (`start_workflow_agent`) and the reviewer
/// (`start_workflow_review_agent`) — everything except which skill the prompt
/// names is identical, and the two drifting apart would be a real bug (an
/// unregistered session shows up as wild, a missing status file makes it look
/// dead).
/// Everything a workflow session spawn needs except the prompt. A struct
/// rather than a long parameter list, same reasoning as `NewWorkflowItem`.
struct ItemSessionSpawn<'a> {
    project: &'a str,
    slug: &'a str,
    meta: &'a clash::domain::workflow::WorkflowMeta,
    /// Directory the agent works in: the item's worktree, or its repo.
    cwd: &'a str,
    /// Model to pin the session to — see `workflow::model_for_phase`.
    model: &'a str,
    cols: u16,
    rows: u16,
}

async fn spawn_item_session(
    state: &GuiState,
    spawn: ItemSessionSpawn<'_>,
    prompt: impl FnOnce(&str) -> String,
) -> Result<String, String> {
    let ItemSessionSpawn {
        project,
        slug,
        meta,
        cwd,
        model,
        cols,
        rows,
    } = spawn;
    let session_id = uuid::Uuid::now_v7().to_string();
    let name = format!("wf-{}", slug);
    clash::infrastructure::hooks::registry::register(
        &session_id,
        &name,
        cwd,
        Some(meta.branch.as_str()),
    );
    clash::infrastructure::hooks::save_session_name(
        state.backend.base_dir(),
        &session_id,
        &name,
        Some(cwd),
    );
    clash::infrastructure::hooks::write_session_status(
        state.backend.base_dir(),
        &session_id,
        "starting",
    );

    let item_dir = state
        .backend
        .workflows_dir()
        .join(project)
        .join(slug)
        .to_string_lossy()
        .into_owned();
    let prompt = prompt(&item_dir);

    let claude_bin = state.claude_bin();
    let mut control = state.control.lock().await;
    crate::ensure_connected(&mut control).await;
    control
        .create_session(
            &session_id,
            &claude_bin,
            // `--model` is pinned per phase rather than inherited: see
            // `workflow::model_for_phase`. It precedes the prompt because the
            // prompt is the positional argument.
            &[
                "--session-id".to_string(),
                session_id.clone(),
                "--model".to_string(),
                model.to_string(),
                prompt,
            ],
            if cwd.is_empty() { None } else { Some(cwd) },
            Some(name),
            cols,
            rows,
            std::collections::HashMap::new(),
            true, // TUI: Claude sets its own termios
        )
        .await
        .map_err(|e| format!("Failed to spawn session: {}", e))?;
    Ok(session_id)
}

/// Spawn a Claude Code session working on this item, in a dedicated worktree
/// (created on first launch, persisted in meta). The kickoff prompt routes
/// the session to the `clash-workflow` skill with the item directory and the
/// requested phase (`plan` | `revise` | `implement`).
#[tauri::command]
pub(crate) async fn start_workflow_agent(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    phase: String,
    branch: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    let mut meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    if meta.repo_path.trim().is_empty() {
        return Err("This item has no repository path — set repoPath in meta.json".to_string());
    }

    // First launch: isolate the item in its own worktree + branch. The
    // branch defaults to the slug; when that name is taken the structured
    // `branch-exists:` error makes the GUI ask for another name and retry.
    if meta.worktree.as_deref().unwrap_or("").is_empty() {
        let branch_name = branch
            .as_deref()
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .unwrap_or(&slug)
            .to_string();
        if crate::branch_exists(&meta.repo_path, &branch_name).await {
            return Err(format!("branch-exists:{}", branch_name));
        }
        let (wt, _source_branch) = crate::create_worktree(&meta.repo_path, &branch_name).await?;
        meta.worktree = Some(wt);
        meta.branch = branch_name;
    }
    let cwd = meta.worktree.clone().unwrap_or_default();
    let session_id = spawn_item_session(
        &state,
        ItemSessionSpawn {
            project: &project,
            slug: &slug,
            meta: &meta,
            cwd: &cwd,
            model: clash::application::workflow::model_for_phase(&phase),
            cols,
            rows,
        },
        |item_dir| clash::application::workflow::build_agent_prompt(item_dir, &phase, meta.mode),
    )
    .await?;

    // Reflect the launch in meta: session id + phase-appropriate status
    // (invalid transitions — e.g. relaunching a dead agent mid-phase — keep
    // the current status). A review-only item has no planning phase, so its
    // agent always lands in `implementing`.
    meta.session_id = Some(session_id.clone());
    if !clash::application::workflow::phase_keeps_status(&phase) {
        let target = if phase == "plan" && meta.mode.has_plan_phase() {
            WorkflowStatus::Planning
        } else {
            WorkflowStatus::Implementing
        };
        if meta.status.can_transition_to(target) {
            meta.status = target;
        }
    }
    state
        .backend
        .write_workflow_meta(&project, &slug, &meta)
        .map_err(e2s)?;
    seed_local(&state, &project, &slug, meta.status);

    Ok(session_id)
}

// ── Agent review rounds ─────────────────────────────────────────────────

/// Launch an agent review round over the item, as many times as the human
/// wants. The round is bounded and self-returning: it records where the item
/// was, parks it in `reviewing` while the agent works, and the agent's last act
/// is to put it back — so round N+1 starts from exactly where round N did.
///
/// The target is *derived* from the current status (a plan review only makes
/// sense at `plan-review`), the depth and publish mode come from the human.
/// The reviewer runs in the item's worktree when it has one and in the repo
/// otherwise — a `from-plan` item parked at `plan-review` has no branch yet,
/// and a plan review needs none.
#[tauri::command]
pub(crate) async fn start_workflow_review_agent(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    depth: ReviewDepth,
    publish: ReviewPublish,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    let mut meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    if meta.repo_path.trim().is_empty() {
        return Err("This item has no repository path — set repoPath in meta.json".to_string());
    }
    if !meta.status.can_request_review() {
        return Err(format!(
            "Can't review an item in '{}' — wait for the current phase to hand back",
            meta.status
        ));
    }
    // Fail before spawning rather than letting the agent discover it: a round
    // that talks to the forge needs a PR to talk to.
    if publish.needs_pr() && meta.pr.as_ref().is_none_or(|p| p.url.is_empty()) {
        return Err("no-pr: this item has no pull request yet".to_string());
    }
    let target = ReviewTarget::for_status(meta.status, meta.mode);
    if target == ReviewTarget::Plan && !has_plan_content(&state, &project, &slug) {
        return Err("This item has no plan to review yet".to_string());
    }

    let review = WorkflowReview {
        target,
        depth,
        publish,
        return_status: meta.status,
        round: meta.review_round.saturating_add(1),
        started_at: now_ms(),
        ..Default::default()
    };

    // The worktree is not created here — a reviewer never makes branches.
    let cwd = meta
        .worktree
        .clone()
        .filter(|w| !w.is_empty())
        .unwrap_or_else(|| meta.repo_path.clone());
    let mode = meta.mode;
    let review_for_prompt = review.clone();
    let session_id = spawn_item_session(
        &state,
        ItemSessionSpawn {
            project: &project,
            slug: &slug,
            meta: &meta,
            cwd: &cwd,
            // A reviewer is always a thinking phase, whatever it is reviewing.
            model: clash::application::workflow::model_for_phase("review"),
            cols,
            rows,
        },
        |item_dir| {
            clash::application::workflow::build_review_prompt(item_dir, &review_for_prompt, mode)
        },
    )
    .await?;

    meta.session_id = Some(session_id.clone());
    meta.review_round = review.round;
    meta.review = Some(review);
    if meta.status.can_transition_to(WorkflowStatus::Reviewing) {
        meta.status = WorkflowStatus::Reviewing;
    }
    state
        .backend
        .write_workflow_meta(&project, &slug, &meta)
        .map_err(e2s)?;
    seed_local(&state, &project, &slug, meta.status);

    Ok(session_id)
}

/// True when `plan.md` has something in it — a plan review with no plan would
/// just have the reviewer report that back after a full session spawn.
fn has_plan_content(state: &GuiState, project: &str, slug: &str) -> bool {
    state
        .backend
        .read_workflow_doc(
            project,
            slug,
            clash::infrastructure::fs::workflows::PLAN_FILE,
        )
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

/// Put a review round's item back where it came from without waiting for the
/// agent — the escape hatch for a reviewer that died or was killed mid-round.
/// Without it a crashed reviewer would wedge the item in `reviewing` with its
/// Approve button disabled, which is exactly the dead-end the gating choice
/// risks.
#[tauri::command]
pub(crate) fn cancel_workflow_review(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
) -> Result<clash::domain::workflow::WorkflowMeta, String> {
    let mut meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    if meta.status != WorkflowStatus::Reviewing {
        return Err("This item is not in a review round".to_string());
    }
    // Fall back to diff-review rather than trusting a missing/unknown return
    // status: it is the state every mode can sit in.
    let back = meta
        .review
        .as_ref()
        .map(|r| r.return_status)
        .filter(|s| !matches!(s, WorkflowStatus::Unknown | WorkflowStatus::Reviewing))
        .unwrap_or(WorkflowStatus::DiffReview);
    meta.status = back;
    state
        .backend
        .write_workflow_meta(&project, &slug, &meta)
        .map_err(e2s)?;
    seed_local(&state, &project, &slug, back);
    Ok(meta)
}

// ── PR lifecycle (gh) ───────────────────────────────────────────────────

/// Map gh errors to the GUI degradation contract: `gh-unavailable:` /
/// `gh-unauthenticated:` prefixes disable PR buttons with a setup hint.
fn gh_err(e: clash::infrastructure::gh::GhError) -> String {
    use clash::infrastructure::gh::GhError;
    match e {
        GhError::NotInstalled => "gh-unavailable: gh CLI not installed".to_string(),
        GhError::NotAuthenticated(m) => format!("gh-unauthenticated: {}", m),
        other => other.to_string(),
    }
}

/// The directory gh commands run in: the item's worktree, else its repo.
fn pr_dir(meta: &clash::domain::workflow::WorkflowMeta) -> Result<String, String> {
    let dir = meta
        .worktree
        .clone()
        .filter(|w| !w.is_empty())
        .unwrap_or_else(|| meta.repo_path.clone());
    if dir.is_empty() {
        Err("No repository directory recorded for this item".to_string())
    } else {
        Ok(dir)
    }
}

/// Fold a `gh pr view` result into the meta's PR block. Returns true when
/// anything (besides the check timestamp) actually changed — the caller only
/// writes meta on change, so the 60s poll never churns the FS watcher.
fn merge_pr_view(
    meta: &mut clash::domain::workflow::WorkflowMeta,
    view: &clash::infrastructure::gh::GhPrView,
) -> bool {
    let pr = meta.pr.get_or_insert_with(Default::default);
    let changed = pr.url != view.url
        || pr.number != view.number
        || pr.draft != view.is_draft
        || pr.state != view.state;
    if changed {
        pr.url = view.url.clone();
        pr.number = view.number;
        pr.draft = view.is_draft;
        pr.state = view.state.clone();
        pr.last_checked_at = now_ms();
    }
    changed
}

/// Create a draft PR for the item's branch via `gh pr create --draft`,
/// record it in meta, and move the item to `pr-draft`.
#[tauri::command]
pub(crate) async fn workflow_create_pr(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    title: Option<String>,
    body: Option<String>,
) -> Result<clash::domain::workflow::WorkflowMeta, String> {
    let mut meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    let dir = pr_dir(&meta)?;
    let base = if meta.base.trim().is_empty() {
        crate::origin_default_branch(&dir).await
    } else {
        meta.base.clone()
    };
    let pr_title = title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| meta.title.clone());
    // An explicit body wins; otherwise transcribe the item's plan rather than
    // opening an empty PR (which is what a bare `unwrap_or_default()` did).
    let pr_body = body.filter(|b| !b.trim().is_empty()).unwrap_or_else(|| {
        state
            .backend
            .read_workflow_doc(
                &project,
                &slug,
                clash::infrastructure::fs::workflows::PLAN_FILE,
            )
            .ok()
            .and_then(|plan| {
                clash::application::workflow::pr_body_from_plan(
                    &plan,
                    meta.iteration,
                    meta.review_round,
                )
            })
            .unwrap_or_default()
    });

    let view = tauri::async_runtime::spawn_blocking(move || {
        clash::infrastructure::gh::pr_create_draft(
            Path::new(&dir),
            &pr_title,
            &pr_body,
            Some(&base),
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(gh_err)?;

    merge_pr_view(&mut meta, &view);
    if meta.status.can_transition_to(WorkflowStatus::PrDraft) {
        meta.status = WorkflowStatus::PrDraft;
    }
    state
        .backend
        .write_workflow_meta(&project, &slug, &meta)
        .map_err(e2s)?;
    seed_local(&state, &project, &slug, meta.status);
    Ok(meta)
}

/// Refresh the recorded PR state from `gh pr view`. Throttled in-memory
/// (30s unless `force`); meta is written only when something changed, so
/// polling never feeds the FS watcher. A PR observed as MERGED moves the
/// item to `done`.
#[tauri::command]
pub(crate) async fn refresh_workflow_pr(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    force: bool,
) -> Result<clash::domain::workflow::WorkflowMeta, String> {
    let mut meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    let Some(pr) = meta.pr.clone() else {
        return Err("No PR recorded for this item".to_string());
    };

    // In-memory throttle: several viewports polling the same item collapse
    // into one gh call per window.
    {
        let mut checked = state.pr_checked.lock().unwrap();
        let key = (project.clone(), slug.clone());
        let now = now_ms();
        if !force && checked.get(&key).is_some_and(|t| now - t < 30_000) {
            return Ok(meta);
        }
        checked.insert(key, now);
    }

    let dir = pr_dir(&meta)?;
    let selector = if pr.number > 0 {
        pr.number.to_string()
    } else {
        meta.branch.clone()
    };
    let (view, unanswered) = tauri::async_runtime::spawn_blocking(move || {
        let view = clash::infrastructure::gh::pr_view(Path::new(&dir), &selector)?;
        // Best-effort: a failed count keeps the previous value rather than
        // failing the refresh — the count is a button label, not PR state.
        let unanswered = (view.number > 0)
            .then(|| {
                clash::infrastructure::gh::pr_unanswered_review_comments(
                    Path::new(&dir),
                    view.number,
                )
                .ok()
            })
            .flatten();
        Ok::<_, clash::infrastructure::gh::GhError>((view, unanswered))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(gh_err)?;

    let mut changed = merge_pr_view(&mut meta, &view);
    if let (Some(n), Some(pr)) = (unanswered, meta.pr.as_mut()) {
        if pr.unanswered_comments != Some(n) {
            pr.unanswered_comments = Some(n);
            changed = true;
        }
    }
    if view.state == "MERGED" && meta.status.can_transition_to(WorkflowStatus::Done) {
        meta.status = WorkflowStatus::Done;
        changed = true;
    }
    if changed {
        state
            .backend
            .write_workflow_meta(&project, &slug, &meta)
            .map_err(e2s)?;
        seed_local(&state, &project, &slug, meta.status);
    }
    Ok(meta)
}

/// Post the latest agent-review round to the item's PR as one comment.
///
/// This is the recovery path for a round whose findings stayed local — a
/// `local` round the human decided is worth sharing, or a
/// `respond-pr-comments` round that found nothing to answer. Publishing was
/// previously only choosable at launch, so getting findings onto the PR
/// afterwards meant burning a whole new review round.
#[tauri::command]
pub(crate) async fn publish_workflow_review(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
) -> Result<u32, String> {
    let meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    let pr = meta
        .pr
        .clone()
        .filter(|p| p.number > 0)
        .ok_or_else(|| "no-pr: this item has no pull request yet".to_string())?;
    let report = state
        .backend
        .read_workflow_doc(
            &project,
            &slug,
            clash::infrastructure::fs::workflows::AGENT_REVIEW_FILE,
        )
        .map_err(e2s)?;
    let (round, section) = clash::application::workflow::latest_agent_review_section(&report)
        .ok_or_else(|| "This item has no agent review round to post".to_string())?;
    let body = format!(
        "### clash · agent review round {}\n\n{}",
        round,
        section
            .strip_prefix(&format!("## Review {}", round))
            .map(|rest| {
                // Drop the duplicated heading line, keep everything after it.
                rest.split_once('\n')
                    .map(|(_, b)| b)
                    .unwrap_or("")
                    .to_string()
            })
            .unwrap_or(section)
    );
    let dir = pr_dir(&meta)?;
    tauri::async_runtime::spawn_blocking(move || {
        clash::infrastructure::gh::pr_comment(Path::new(&dir), pr.number, &body)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(gh_err)?;
    Ok(round)
}

/// Flip the draft PR to ready-for-review (`gh pr ready`) — the validation
/// act. Moves the item to `pr-ready`.
#[tauri::command]
pub(crate) async fn mark_workflow_pr_ready(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
) -> Result<clash::domain::workflow::WorkflowMeta, String> {
    let mut meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    let Some(pr) = meta.pr.clone() else {
        return Err("No PR recorded for this item".to_string());
    };
    if pr.number == 0 {
        return Err("Recorded PR has no number — refresh it first".to_string());
    }
    let dir = pr_dir(&meta)?;
    tauri::async_runtime::spawn_blocking(move || {
        clash::infrastructure::gh::pr_ready(Path::new(&dir), pr.number)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(gh_err)?;

    if let Some(pr) = meta.pr.as_mut() {
        pr.draft = false;
        pr.last_checked_at = now_ms();
    }
    if meta.status.can_transition_to(WorkflowStatus::PrReady) {
        meta.status = WorkflowStatus::PrReady;
    }
    state
        .backend
        .write_workflow_meta(&project, &slug, &meta)
        .map_err(e2s)?;
    seed_local(&state, &project, &slug, meta.status);
    Ok(meta)
}

/// Attach an existing PR by URL (e.g. one the agent created, sniffed from
/// terminal output). Works without gh — state stays unknown until a refresh
/// succeeds.
#[tauri::command]
pub(crate) async fn attach_workflow_pr(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    url: String,
) -> Result<clash::domain::workflow::WorkflowMeta, String> {
    let (_repo, number) = clash::infrastructure::gh::parse_pr_url(&url)
        .ok_or_else(|| format!("Not a GitHub PR URL: {}", url))?;
    let mut meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    let pr = meta.pr.get_or_insert_with(Default::default);
    pr.url = url.trim().to_string();
    pr.number = number;
    if meta.status.can_transition_to(WorkflowStatus::PrDraft) {
        meta.status = WorkflowStatus::PrDraft;
    }
    state
        .backend
        .write_workflow_meta(&project, &slug, &meta)
        .map_err(e2s)?;
    seed_local(&state, &project, &slug, meta.status);

    // Best-effort detail fill; ignored when gh is unavailable.
    let dir = pr_dir(&meta)?;
    let selector = number.to_string();
    if let Ok(Ok(view)) = tauri::async_runtime::spawn_blocking(move || {
        clash::infrastructure::gh::pr_view(Path::new(&dir), &selector)
    })
    .await
    .map(|r| r.map_err(gh_err))
    {
        if merge_pr_view(&mut meta, &view) {
            state
                .backend
                .write_workflow_meta(&project, &slug, &meta)
                .map_err(e2s)?;
        }
    }
    Ok(meta)
}

// ── Skills (visualize the Claude Code skills clash ships/uses) ──────────

/// One skill under `<claude_dir>/skills/` as listed to the GUI.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillInfo {
    name: String,
    /// First `description:` line of the frontmatter (may be long; the GUI
    /// truncates for display).
    description: String,
    /// Absolute path of the SKILL.md (for open-in-editor).
    path: String,
    /// True when this skill is embedded in the clash binary (managed —
    /// local edits are overwritten at startup).
    managed: bool,
    /// For managed skills: whether the installed copy matches the binary.
    up_to_date: bool,
}

/// Pull the `description:` value out of SKILL.md frontmatter (single line).
fn skill_description(content: &str) -> String {
    let mut in_frontmatter = false;
    for line in content.lines() {
        if line.trim() == "---" {
            if in_frontmatter {
                break;
            }
            in_frontmatter = true;
            continue;
        }
        if in_frontmatter {
            if let Some(d) = line.strip_prefix("description:") {
                return d.trim().to_string();
            }
        }
    }
    String::new()
}

/// List every skill directory containing a SKILL.md, managed ones first.
#[tauri::command]
pub(crate) fn list_skills(state: State<'_, GuiState>) -> Vec<SkillInfo> {
    let skills_dir = state.backend.base_dir().join("skills");
    let embedded: std::collections::HashMap<&str, &str> = clash::infrastructure::skills::SKILLS
        .iter()
        .map(|s| (s.name, s.content))
        .collect();
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&skills_dir)
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path().join("SKILL.md");
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let managed = embedded.contains_key(name.as_str());
        out.push(SkillInfo {
            description: skill_description(&content),
            up_to_date: !managed || embedded.get(name.as_str()) == Some(&content.as_str()),
            path: path.to_string_lossy().into_owned(),
            name,
            managed,
        });
    }
    out.sort_by_key(|s| (!s.managed, s.name.clone()));
    out
}

/// Full SKILL.md content for the viewer. `name` is sanitized against
/// traversal the same way workflow components are.
#[tauri::command]
pub(crate) fn get_skill(state: State<'_, GuiState>, name: String) -> Result<String, String> {
    if name.is_empty() || name.contains(['/', '\\', '\0']) || name == "." || name == ".." {
        return Err(format!("Invalid skill name: '{}'", name));
    }
    let path = state
        .backend
        .base_dir()
        .join("skills")
        .join(&name)
        .join("SKILL.md");
    std::fs::read_to_string(&path).map_err(|e| format!("Cannot read {}: {}", path.display(), e))
}
