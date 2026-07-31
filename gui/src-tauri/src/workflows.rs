//! Tauri command layer for the Workflows feature.
//!
//! Thin adapters over the shared core: storage goes through the
//! `WorkflowRepository` port on `FsBackend`, pure logic (anchoring, attention
//! transitions) lives in `clash::application::workflow`. Nothing in here owns
//! business rules beyond wiring.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use clash::application::diff::parse_file_diffs;
use clash::application::workflow::anchor_annotations;
use clash::application::workflow::AnchoredAnnotation;
use clash::domain::ports::WorkflowRepository;
use clash::domain::workflow::{AnnotationsFile, WorkflowItem, WorkflowStatus};
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
}

/// Start (or restart) the workflows-directory watcher. Emits
/// `workflows-changed` on any change under `dir` so the sidebar and open
/// workflow tabs stay in sync with agent writes. Same shape as the scratch
/// watcher; dropping the returned watcher stops the watch.
pub(crate) fn start_workflows_watcher(
    app: &tauri::AppHandle,
    dir: PathBuf,
    debounce: std::time::Duration,
) -> Option<clash::infrastructure::fs::watcher::FsWatcher> {
    let _ = std::fs::create_dir_all(&dir);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<PathBuf>>();
    let watcher = clash::infrastructure::fs::watcher::FsWatcher::new(
        std::slice::from_ref(&dir),
        tx,
        debounce,
    )
    .map_err(|e| tracing::warn!("Workflows watcher unavailable: {}", e))
    .ok()?;
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        while rx.recv().await.is_some() {
            let _ = handle.emit("workflows-changed", ());
        }
    });
    Some(watcher)
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
        if matches!(
            item.meta.status,
            WorkflowStatus::Planning | WorkflowStatus::Implementing
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
            let _ = app.emit(
                "workflow-attention",
                WorkflowAttention {
                    project: ev.project.clone(),
                    slug: ev.slug.clone(),
                    title: ev.title.clone(),
                    status: ev.status,
                },
            );
            if !focused
                && state
                    .notify_enabled
                    .load(std::sync::atomic::Ordering::Relaxed)
            {
                let what = match ev.status {
                    WorkflowStatus::PlanReview => "plan ready for review",
                    WorkflowStatus::DiffReview => "changes ready for review",
                    WorkflowStatus::PrDraft => "draft PR awaiting validation",
                    _ => "needs your decision",
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
/// otherwise the live diff of the item's worktree/repo against the
/// merge-base with the origin default branch (committed + uncommitted work).
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
    let base = crate::origin_default_branch(&dir).await;
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
    let trimmed = path.trim();
    let mut config = clash::infrastructure::config::Config::load();

    let effective = if trimmed.is_empty() {
        config.workflows_dir = None;
        config.workflows_dir()
    } else {
        let expanded = crate::expand_tilde(trimmed);
        std::fs::create_dir_all(&expanded)
            .map_err(|e| format!("Cannot use {}: {}", expanded.display(), e))?;
        config.workflows_dir = Some(expanded.clone());
        expanded
    };

    config
        .save()
        .map_err(|e| format!("Save config failed: {}", e))?;
    state.backend.set_workflows_dir(effective.clone());
    *state.workflows_watcher.lock().unwrap() = start_workflows_watcher(
        &app,
        effective.clone(),
        std::time::Duration::from_millis(config.debounce_ms),
    );
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
        WorkflowStatus::ChangesRequested | WorkflowStatus::Implementing
    ) {
        Err("agent is working — annotations are locked until it finishes".to_string())
    } else {
        Ok(())
    }
}

/// Create a new workflow item (status `draft`, iteration 1).
#[tauri::command]
pub(crate) fn create_workflow_item(
    state: State<'_, GuiState>,
    project: String,
    title: String,
    repo_path: String,
) -> Result<WorkflowItem, String> {
    let item = state
        .backend
        .create_workflow_item(&project, &title, &repo_path)
        .map_err(e2s)?;
    seed_local(&state, &item.project, &item.slug, item.meta.status);
    Ok(item)
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

/// Spawn a Claude Code session working on this item, in a dedicated worktree
/// (created on first launch, persisted in meta). The kickoff prompt routes
/// the session to the `clash-workflow` skill with the item directory and the
/// requested phase (`plan` | `revise` | `implement`). Reuses the exact
/// session machinery of `create_new_session`/`create_worktree_session`:
/// registry + name + status files, then a daemon spawn with `--session-id`
/// and the prompt as the positional initial argument.
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

    let session_id = uuid::Uuid::now_v7().to_string();
    let name = format!("wf-{}", slug);
    clash::infrastructure::hooks::registry::register(
        &session_id,
        &name,
        &cwd,
        Some(meta.branch.as_str()),
    );
    clash::infrastructure::hooks::save_session_name(
        state.backend.base_dir(),
        &session_id,
        &name,
        Some(&cwd),
    );
    clash::infrastructure::hooks::write_session_status(
        state.backend.base_dir(),
        &session_id,
        "starting",
    );

    let item_dir = state
        .backend
        .workflows_dir()
        .join(&project)
        .join(&slug)
        .to_string_lossy()
        .into_owned();
    let prompt = clash::application::workflow::build_agent_prompt(&item_dir, &phase);

    {
        let mut control = state.control.lock().await;
        crate::ensure_connected(&mut control).await;
        control
            .create_session(
                &session_id,
                &state.claude_bin,
                &["--session-id".to_string(), session_id.clone(), prompt],
                Some(&cwd),
                Some(name),
                cols,
                rows,
                std::collections::HashMap::new(),
                true, // TUI: Claude sets its own termios
            )
            .await
            .map_err(|e| format!("Failed to spawn session: {}", e))?;
    }

    // Reflect the launch in meta: session id + phase-appropriate status
    // (invalid transitions — e.g. relaunching a dead agent mid-phase — keep
    // the current status).
    meta.session_id = Some(session_id.clone());
    let target = if phase == "plan" {
        WorkflowStatus::Planning
    } else {
        WorkflowStatus::Implementing
    };
    if meta.status.can_transition_to(target) {
        meta.status = target;
    }
    state
        .backend
        .write_workflow_meta(&project, &slug, &meta)
        .map_err(e2s)?;
    seed_local(&state, &project, &slug, meta.status);

    Ok(session_id)
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
    let base = crate::origin_default_branch(&dir).await;
    let pr_title = title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| meta.title.clone());
    let pr_body = body.unwrap_or_default();

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
    let view = tauri::async_runtime::spawn_blocking(move || {
        clash::infrastructure::gh::pr_view(Path::new(&dir), &selector)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(gh_err)?;

    let mut changed = merge_pr_view(&mut meta, &view);
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
