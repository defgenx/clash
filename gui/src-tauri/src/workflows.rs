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
