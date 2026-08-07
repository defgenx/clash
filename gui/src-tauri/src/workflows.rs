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
            if !focused
                && state
                    .notify_enabled
                    .load(std::sync::atomic::Ordering::Relaxed)
            {
                native_notify(&format!("clash · {}", title), what);
            }
            // Webhook announcement — the `workflows.notify_webhook` opt-in.
            // Deliberately NOT gated on window focus: a channel ping is team
            // visibility, and "posted only when your window was blurred"
            // makes the channel's record unexplainable. The ledger already
            // limits events to agent-driven transitions, never own clicks.
            {
                use clash::infrastructure::webhook::{self, WebhookKind};
                let cfg = state.config.get();
                if let Some(kind) = WebhookKind::parse(&cfg.workflows.notify_webhook) {
                    let url = match kind {
                        WebhookKind::Slack => cfg.workflows.slack_webhook.clone(),
                        WebhookKind::Discord => cfg.workflows.discord_webhook.clone(),
                    };
                    if !url.trim().is_empty() {
                        let pr_url = items
                            .iter()
                            .find(|i| i.project == ev.project && i.slug == ev.slug)
                            .and_then(|i| i.meta.pr.as_ref())
                            .map(|p| p.url.clone())
                            .filter(|u| !u.is_empty());
                        let text = format!(
                            "⧉ {} — {} · {}/{}{}",
                            title,
                            what,
                            ev.project,
                            ev.slug,
                            pr_url.map(|u| format!("\n{}", u)).unwrap_or_default()
                        );
                        // Fire-and-forget: a slow or dead webhook must never
                        // stall the session-list refresh it piggybacks on.
                        tauri::async_runtime::spawn_blocking(move || {
                            if let Err(e) = webhook::send(kind, &url, &text) {
                                tracing::warn!("notify webhook failed: {}", e);
                            }
                        });
                    }
                }
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

/// Raw unified diff of one of the item's **linked** PRs, fetched from the
/// forge (`gh pr diff --repo …`) — the linked repo has no local checkout, so
/// the forge is the only source. View-only by design: annotations anchor to
/// the item's own diff, never to a linked one.
#[tauri::command]
pub(crate) async fn get_linked_pr_diff(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    url: String,
) -> Result<String, String> {
    let meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    if !meta.linked_prs.iter().any(|p| p.url == url) {
        return Err(format!("This item does not track the linked PR {}", url));
    }
    let (repo, number) = clash::infrastructure::gh::parse_pr_url(&url)
        .ok_or_else(|| format!("Not a GitHub PR URL: {}", url))?;
    let dir = pr_dir(&meta)?;
    let forge = state.forge_for_dir(&dir);
    tauri::async_runtime::spawn_blocking(move || {
        forge.change_diff(Path::new(&dir), number, Some(&repo))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(forge_err)
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

/// The configured PR-creation skill, for the Settings field.
#[tauri::command]
pub(crate) fn get_workflow_pr_skill(state: State<'_, GuiState>) -> String {
    state.config.get().workflows.pr_skill.trim().to_string()
}

/// Set (or reset) the PR-creation skill workflow agents open PRs with.
/// Empty resets to the schema default (`hivebrite-engineering:github-pr`);
/// `none` disables it (repo conventions via `gh`). Persisted to the shared
/// `config.toml` and read live at the next agent launch; items can override
/// per item in their ⚙ Settings tab.
#[tauri::command]
pub(crate) fn set_workflow_pr_skill(
    state: State<'_, GuiState>,
    skill: String,
) -> Result<String, String> {
    let trimmed = skill.trim();
    if trimmed.is_empty() {
        state
            .config
            .reset_values(&["workflows.pr_skill"])
            .map_err(|e| e.to_string())?;
    } else {
        state
            .config
            .set_json(&[(
                "workflows.pr_skill",
                serde_json::Value::String(trimmed.to_string()),
            )])
            .map_err(|e| e.to_string())?;
    }
    Ok(state.config.get().workflows.pr_skill)
}

/// The configured forge override, for the Settings field.
#[tauri::command]
pub(crate) fn get_workflow_forge(state: State<'_, GuiState>) -> String {
    state.config.get().workflows.forge
}

/// Set the forge override: `auto` (detect from the origin remote), `github`,
/// or `none`. `auto` resets to the default. Persisted to the shared
/// `config.toml` and read live at the next forge operation (overrides never
/// consult the detection cache).
#[tauri::command]
pub(crate) fn set_workflow_forge(
    state: State<'_, GuiState>,
    forge: String,
) -> Result<String, String> {
    let value = forge.trim().to_ascii_lowercase();
    if !["auto", "github", "none"].contains(&value.as_str()) {
        return Err(format!("Unknown forge '{}'", forge));
    }
    if value == "auto" {
        state
            .config
            .reset_values(&["workflows.forge"])
            .map_err(|e| e.to_string())?;
    } else {
        state
            .config
            .set_json(&[("workflows.forge", serde_json::Value::String(value))])
            .map_err(|e| e.to_string())?;
    }
    Ok(state.config.get().workflows.forge)
}

/// Patch the per-item settings (the item ⚙ Settings tab). Every field is
/// optional: `Some` sets, `None` leaves the current value — one command for
/// the whole tab, so it grows without new commands per knob.
///
/// - `bare_session_names`: bare job names vs the title-prefixed default.
/// - `pr_skill`: per-item PR-skill override; empty inherits the global
///   setting, `none` disables for this item.
/// - `interaction_default`: `""`/`ask` | `interactive` | `autonomous` — how
///   this item's agent rounds run unless chosen at launch.
/// - `jira_ticket`: the item's Jira ticket (`PROJ-123`); pre-fills and is
///   remembered by the share dialog's Post-to-Jira. Empty clears it.
/// - `description`: the item's free-form intent — the planning agent's
///   primary source. Editable so it can be refined before launching a plan.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) fn set_workflow_item_settings(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    bare_session_names: Option<bool>,
    pr_skill: Option<String>,
    interaction_default: Option<String>,
    jira_ticket: Option<String>,
    description: Option<String>,
) -> Result<clash::domain::workflow::WorkflowMeta, String> {
    let mut meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    if let Some(bare) = bare_session_names {
        meta.bare_session_names = bare;
    }
    if let Some(skill) = pr_skill {
        let skill = skill.trim().to_string();
        if skill.chars().any(char::is_whitespace) {
            return Err(format!("Skill names contain no whitespace: '{}'", skill));
        }
        meta.pr_skill = skill;
    }
    if let Some(mode) = interaction_default {
        let mode = mode.trim().to_ascii_lowercase();
        if !["", "ask", "interactive", "autonomous"].contains(&mode.as_str()) {
            return Err(format!("Unknown interaction mode '{}'", mode));
        }
        meta.interaction_default = if mode == "ask" { String::new() } else { mode };
    }
    if let Some(ticket) = jira_ticket {
        let ticket = ticket.trim().to_ascii_uppercase();
        if !ticket.is_empty() && !clash::infrastructure::jira::valid_ticket_key(&ticket) {
            return Err(format!("Not a Jira ticket key: '{}'", ticket));
        }
        meta.jira_ticket = ticket;
    }
    if let Some(desc) = description {
        meta.description = desc.trim().to_string();
    }
    state
        .backend
        .write_workflow_meta(&project, &slug, &meta)
        .map_err(e2s)?;
    seed_local(&state, &project, &slug, meta.status);
    Ok(meta)
}

/// What a skills upgrade would touch, computed fresh from disk. The frontend
/// asks at boot: `needsDecision` drives the popup (or the automatic
/// application when `general.skills_update` is not `ask`).
#[tauri::command]
pub(crate) fn get_skills_plan(
    state: State<'_, GuiState>,
) -> clash::infrastructure::skills::SkillsPlan {
    clash::infrastructure::skills::plan_install(state.backend.base_dir())
}

/// Apply a skills-update decision (`all` | `untouched` | `keep`) and stamp
/// it, so the question only returns on the next actual upgrade.
#[tauri::command]
pub(crate) fn apply_skills_decision(
    state: State<'_, GuiState>,
    mode: String,
) -> Result<clash::infrastructure::skills::SkillsReport, String> {
    let mode = clash::infrastructure::skills::ApplyMode::parse(&mode)
        .ok_or_else(|| format!("Unknown skills-update mode '{}'", mode))?;
    Ok(clash::infrastructure::skills::apply_decision(
        state.backend.base_dir(),
        mode,
    ))
}

/// The startup policy for changed skills, for the Settings field.
#[tauri::command]
pub(crate) fn get_skills_update_mode(state: State<'_, GuiState>) -> String {
    state.config.get().general.skills_update
}

/// Set the startup policy: `ask` shows the popup, the other three apply
/// silently. `ask` resets to the default.
#[tauri::command]
pub(crate) fn set_skills_update_mode(
    state: State<'_, GuiState>,
    mode: String,
) -> Result<String, String> {
    let value = mode.trim().to_ascii_lowercase();
    if !["ask", "all", "untouched", "keep"].contains(&value.as_str()) {
        return Err(format!("Unknown skills-update mode '{}'", mode));
    }
    if value == "ask" {
        state
            .config
            .reset_values(&["general.skills_update"])
            .map_err(|e| e.to_string())?;
    } else {
        state
            .config
            .set_json(&[("general.skills_update", serde_json::Value::String(value))])
            .map_err(|e| e.to_string())?;
    }
    Ok(state.config.get().general.skills_update)
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
// Tauri command parameters map 1:1 onto the frontend's named invoke args —
// same reasoning as `start_workflow_agent`.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub(crate) fn create_workflow_item(
    state: State<'_, GuiState>,
    project: String,
    title: String,
    repo_path: String,
    mode: Option<WorkflowMode>,
    plan: Option<String>,
    plan_file: Option<String>,
    description: Option<String>,
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
            description: description.unwrap_or_default(),
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
            let forge = state.forge_for_dir(&repo.to_string_lossy());
            let view = tauri::async_runtime::spawn_blocking(move || {
                forge.view(&dir, &sel, scope.as_deref())
            })
            .await
            .map_err(|e| e.to_string())?
            .map_err(forge_err)?;
            if view.head_ref.is_empty() {
                return Err(format!(
                    "the forge did not report a head branch for PR {}",
                    selector
                ));
            }
            let block = clash::domain::workflow::WorkflowPr {
                url: view.url.clone(),
                number: view.number,
                draft: view.draft,
                state: view.state.as_str().to_string(),
                last_checked_at: now_ms(),
                ..Default::default()
            };
            (
                view.head_ref.clone(),
                view.base_ref.clone(),
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
/// table. Carries no note: **every** change request goes through
/// [`workflow_request_changes`], which snapshots the iteration before the
/// note lands — a bare note-append here once let plan revisions bypass the
/// snapshot flow and leave no trace in history.
#[tauri::command]
pub(crate) fn update_workflow_status(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    status: WorkflowStatus,
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
/// (review C2): park the comments the human excluded, snapshot the current
/// diff + annotations into `history/{iteration:03}/`, append the note +
/// open-annotation digest to review.md, then ONE meta write carrying both
/// `iteration+1` and status `changes-requested`. Any failure before the meta
/// write aborts cleanly; a retry overwrites the orphaned snapshot.
///
/// `park` holds annotation ids the composer excluded from this round: they
/// flip to `parked` (kept, but no longer `open`, so the agent contract —
/// address every open annotation — skips them without any skill change).
/// Parking happens *before* the snapshot so `history/<NNN>/annotations.json`
/// records the round exactly as it was sent.
#[tauri::command]
pub(crate) async fn workflow_request_changes(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    note: Option<String>,
    park: Option<Vec<String>>,
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

    let park = park.unwrap_or_default();
    if !park.is_empty() {
        let mut file = state
            .backend
            .load_workflow_annotations(&project, &slug)
            .map_err(e2s)?;
        let mut changed = false;
        for ann in file.annotations.iter_mut() {
            if ann.status == clash::domain::workflow::AnnotationStatus::Open
                && park.iter().any(|id| id == &ann.id)
            {
                ann.status = clash::domain::workflow::AnnotationStatus::Parked;
                ann.updated_at = now_ms();
                changed = true;
            }
        }
        if changed {
            state
                .backend
                .write_workflow_annotations(&project, &slug, &file)
                .map_err(e2s)?;
        }
    }

    // The snapshot is this iteration's only record — freezing an empty
    // `diff.patch` because `git diff` failed would silently erase what the
    // human reviewed. Only a plan-parked item tolerates it: there may be no
    // diffable worktree yet, and the plan (snapshotted below) is the artifact.
    let diff = match workflow_diff_text(&state, &project, &slug, None).await {
        Ok(d) => d,
        Err(_) if meta.status == WorkflowStatus::PlanReview => String::new(),
        Err(e) => return Err(format!("cannot freeze this iteration's diff: {}", e)),
    };
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

/// Everything the Timeline view needs beyond the item DTO: the parsed
/// change-round notes from `review.md`, every agent review round from
/// `agent-review.md`, and which history snapshots carry a frozen plan. The
/// merge/ordering into cards is the frontend's pure `wf-timeline.js` model.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkflowTimeline {
    iterations: Vec<clash::domain::workflow::ReviewIterationNote>,
    reviews: Vec<clash::domain::workflow::AgentReviewSummary>,
    history: Vec<u32>,
    plan_snapshots: Vec<u32>,
}

/// The item's full revision record, for the Timeline sub-view.
#[tauri::command]
pub(crate) fn get_workflow_timeline(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
) -> Result<WorkflowTimeline, String> {
    let review_md = state
        .backend
        .read_workflow_doc(
            &project,
            &slug,
            clash::infrastructure::fs::workflows::REVIEW_FILE,
        )
        .map_err(e2s)?;
    let agent_md = state
        .backend
        .read_workflow_doc(
            &project,
            &slug,
            clash::infrastructure::fs::workflows::AGENT_REVIEW_FILE,
        )
        .map_err(e2s)?;
    let history = state
        .backend
        .list_workflow_history(&project, &slug)
        .map_err(e2s)?;
    let plan_snapshots = history
        .iter()
        .copied()
        .filter(|&it| {
            state
                .backend
                .read_workflow_history_plan(&project, &slug, it)
                .ok()
                .flatten()
                .is_some()
        })
        .collect();
    Ok(WorkflowTimeline {
        iterations: clash::application::workflow::parse_review_iterations(&review_md),
        reviews: clash::application::workflow::all_agent_reviews(&agent_md),
        history,
        plan_snapshots,
    })
}

/// Full text of the plan as frozen at `iteration` — the Timeline's
/// "plan @ it.N" viewer. Empty when the snapshot has no plan copy.
#[tauri::command]
pub(crate) fn get_workflow_history_plan(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    iteration: u32,
) -> Result<String, String> {
    state
        .backend
        .read_workflow_history_plan(&project, &slug, iteration)
        .map(Option::unwrap_or_default)
        .map_err(e2s)
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
    /// Pre-generated session id. The caller persists it into `meta.json`
    /// **before** spawning — a live agent on an item that doesn't record it
    /// has no recovery path, while "recorded but never spawned" is exactly
    /// what the ⚠ relaunch/end-round affordances already handle.
    session_id: &'a str,
    /// Session display name — says what the agent DOES, not just which item
    /// it belongs to (`workflow::executor_session_name` /
    /// `review_session_name`): `wf-auth · implement`, `wf-auth · explain`.
    name: &'a str,
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
) -> Result<(), String> {
    let ItemSessionSpawn {
        project,
        slug,
        session_id,
        name,
        meta,
        cwd,
        model,
        cols,
        rows,
    } = spawn;
    let name = name.to_string();
    clash::infrastructure::hooks::registry::register(
        session_id,
        &name,
        cwd,
        Some(meta.branch.as_str()),
    );
    clash::infrastructure::hooks::save_session_name(
        state.backend.base_dir(),
        session_id,
        &name,
        Some(cwd),
    );
    clash::infrastructure::hooks::write_session_status(
        state.backend.base_dir(),
        session_id,
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
            session_id,
            &claude_bin,
            // `--model` is pinned per phase rather than inherited: see
            // `workflow::model_for_phase`. It precedes the prompt because the
            // prompt is the positional argument.
            &[
                "--session-id".to_string(),
                session_id.to_string(),
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
    Ok(())
}

/// Spawn a Claude Code session working on this item, in a dedicated worktree
/// (created on first launch, persisted in meta). The kickoff prompt routes
/// the session to the executor skill (`clash-workflow`, or the `skill`
/// override — the composer's escape hatch for routing a round through a
/// custom skill that honors the same file contract) with the item directory
/// and the requested phase (`plan` | `revise` | `implement` | `pr`).
/// `interactive` pre-answers the skill's opening question when the human
/// already chose at launch.
// Tauri command parameters map 1:1 onto the frontend's named invoke args —
// same reasoning as `start_workflow_review_agent`.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub(crate) async fn start_workflow_agent(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    phase: String,
    branch: Option<String>,
    skill: Option<String>,
    interactive: Option<bool>,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    // The skill name lands verbatim in the kickoff prompt — one token only.
    let skill = skill
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(s) = &skill {
        if s.chars().any(char::is_whitespace) {
            return Err(format!("Skill names contain no whitespace: '{}'", s));
        }
    }
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

    // Persist the launch BEFORE spawning (session id + phase-appropriate
    // status; invalid transitions — e.g. relaunching a dead agent mid-phase —
    // keep the current status). This ordering is the crash-safety choice: a
    // "recorded but never spawned" item surfaces the existing ⚠ relaunch
    // affordance, while a live agent on an item that never recorded it has no
    // recovery path at all. The worktree/branch created above are persisted by
    // this same write, so a failed spawn can't strand them unrecorded either.
    // A review-only item has no planning phase, so its agent always lands in
    // `implementing`.
    let rollback = meta.clone();
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

    // Item override → global setting; the item's interaction default fills
    // in when the launch surface offered no explicit choice.
    let pr_skill = clash::application::workflow::effective_pr_skill(
        &meta.pr_skill,
        state.config.get().workflow_pr_skill().as_deref(),
    );
    let interactive = interactive
        .or_else(|| clash::application::workflow::interaction_param(&meta.interaction_default));
    let spawned = spawn_item_session(
        &state,
        ItemSessionSpawn {
            project: &project,
            slug: &slug,
            session_id: &session_id,
            name: &clash::application::workflow::workflow_session_name(&meta, &slug, &phase),
            meta: &meta,
            cwd: &cwd,
            model: clash::application::workflow::model_for_phase(&phase),
            cols,
            rows,
        },
        |item_dir| {
            clash::application::workflow::build_agent_prompt(
                item_dir,
                &clash::application::workflow::ExecutorKickoff {
                    phase: &phase,
                    mode: meta.mode,
                    pr_skill: pr_skill.as_deref(),
                    skill: skill.as_deref(),
                    interactive,
                },
            )
        },
    )
    .await;

    if let Err(e) = spawned {
        // Undo the launch record (keep the worktree/branch — they exist on
        // disk). Best-effort: if this write also fails, the item still shows
        // the recoverable "agent gone" state rather than a phantom session.
        let mut restored = rollback;
        restored.worktree = meta.worktree.clone();
        restored.branch = meta.branch.clone();
        if let Err(e2) = state
            .backend
            .write_workflow_meta(&project, &slug, &restored)
        {
            tracing::warn!(
                "launch rollback write failed for {}/{}: {}",
                project,
                slug,
                e2
            );
        } else {
            seed_local(&state, &project, &slug, restored.status);
        }
        return Err(e);
    }

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
// Tauri command parameters map 1:1 onto the frontend's named invoke args, so
// grouping them into a struct would only move the same list into a shape the
// caller must mirror field-for-field.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub(crate) async fn start_workflow_review_agent(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    depth: ReviewDepth,
    publish: ReviewPublish,
    interactive: Option<bool>,
    target: Option<ReviewTarget>,
    pr_url: Option<String>,
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
    // The PR the round talks to: the primary by default, or a specific one
    // the launcher picked (multi-repo items answer reviewers per PR). A pick
    // must be one of the item's recorded PRs — anything else is a typo, not
    // a target.
    let pr_url = pr_url
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());
    if let Some(u) = &pr_url {
        let known = meta.pr.as_ref().is_some_and(|p| &p.url == u)
            || meta.linked_prs.iter().any(|p| &p.url == u);
        if !known {
            return Err(format!("This item does not track the PR {}", u));
        }
    }
    // Fail before spawning rather than letting the agent discover it: a round
    // that talks to the forge needs a PR to talk to. An explicit pick IS that
    // PR, even when the item has no primary (linked-only items).
    if publish.needs_pr() && pr_url.is_none() && meta.pr.as_ref().is_none_or(|p| p.url.is_empty()) {
        return Err("no-pr: this item has no pull request yet".to_string());
    }
    // Plan/diff stay DERIVED from the launch status (a plan review at
    // diff-review has nothing to read); `structure` is the one explicitly
    // requested target — the "Explain changes" button's round, which writes
    // structure.md instead of findings. Any other explicit value is ignored
    // rather than trusted.
    let target = match target {
        Some(ReviewTarget::Structure) => ReviewTarget::Structure,
        _ => ReviewTarget::for_status(meta.status, meta.mode),
    };
    if target == ReviewTarget::Plan && !has_plan_content(&state, &project, &slug) {
        return Err("This item has no plan to review yet".to_string());
    }

    // The item's interaction default fills in when the launch surface offered
    // no explicit choice (the composer pre-selects it, so its answer arrives
    // explicit; the one-click actions don't ask).
    let interactive = interactive
        .or_else(|| clash::application::workflow::interaction_param(&meta.interaction_default));
    let review = WorkflowReview {
        target,
        depth,
        publish,
        return_status: meta.status,
        round: meta.review_round.saturating_add(1),
        interactive,
        pr_url: pr_url.unwrap_or_default(),
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
    let session_id = uuid::Uuid::now_v7().to_string();

    // Persist the round BEFORE spawning — same crash-safety ordering as
    // `start_workflow_agent`. A parked-but-never-spawned round is unwedged by
    // "End round" / the agent-gone cross-check; a live reviewer writing
    // `annotations.json` on an item that isn't in `reviewing` (approval open,
    // annotations unlocked, cancel refusing) has no recovery path.
    let rollback = meta.clone();
    meta.session_id = Some(session_id.clone());
    meta.review_round = review.round;
    meta.review = Some(review.clone());
    if meta.status.can_transition_to(WorkflowStatus::Reviewing) {
        meta.status = WorkflowStatus::Reviewing;
    }
    state
        .backend
        .write_workflow_meta(&project, &slug, &meta)
        .map_err(e2s)?;
    seed_local(&state, &project, &slug, meta.status);

    let spawned = spawn_item_session(
        &state,
        ItemSessionSpawn {
            project: &project,
            slug: &slug,
            session_id: &session_id,
            name: &clash::application::workflow::workflow_session_name(
                &meta,
                &slug,
                &clash::application::workflow::review_job(&review),
            ),
            meta: &meta,
            cwd: &cwd,
            // A reviewer is always a thinking phase, whatever it is reviewing.
            model: clash::application::workflow::model_for_phase("review"),
            cols,
            rows,
        },
        |item_dir| clash::application::workflow::build_review_prompt(item_dir, &review, mode),
    )
    .await;

    if let Err(e) = spawned {
        if let Err(e2) = state
            .backend
            .write_workflow_meta(&project, &slug, &rollback)
        {
            tracing::warn!(
                "review rollback write failed for {}/{}: {}",
                project,
                slug,
                e2
            );
        } else {
            seed_local(&state, &project, &slug, rollback.status);
        }
        return Err(e);
    }

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

/// Map forge errors to the GUI degradation contract: `gh-unavailable:` /
/// `gh-unauthenticated:` prefixes disable PR buttons with a setup hint, and
/// `forge-unsupported:` explains a repo whose forge clash can't drive. The
/// `gh-` spelling is a frontend contract, kept even though the tool name now
/// rides in the error.
fn forge_err(e: clash::domain::forge::ForgeError) -> String {
    use clash::domain::forge::ForgeError;
    match e {
        ForgeError::NotInstalled(tool) => format!("gh-unavailable: {} CLI not installed", tool),
        ForgeError::NotAuthenticated(m) => format!("gh-unauthenticated: {}", m),
        ForgeError::Unsupported(m) => format!("forge-unsupported: {}", m),
        ForgeError::Other(m) => m,
    }
}

/// A recorded PR's number, derived from the URL when the record is URL-only.
///
/// The agent contract says writing `pr.url` is enough — clash fills the rest.
/// The only filler used to be the lazy 60s poll, so any command demanding a
/// number inside that window failed with "refresh it first" against a URL
/// that literally contains the number.
fn recorded_pr_number(pr: &clash::domain::workflow::WorkflowPr) -> u64 {
    if pr.number > 0 {
        pr.number
    } else {
        clash::infrastructure::gh::parse_pr_url(&pr.url)
            .map(|(_, n)| n)
            .unwrap_or(0)
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

/// Fold a forge view into the meta's PR block. Returns true when anything
/// (besides the check timestamp) actually changed — the caller only writes
/// meta on change, so the 60s poll never churns the FS watcher.
fn merge_pr_view(
    meta: &mut clash::domain::workflow::WorkflowMeta,
    view: &clash::domain::forge::ChangeView,
) -> bool {
    let pr = meta.pr.get_or_insert_with(Default::default);
    let changed = pr.url != view.url
        || pr.number != view.number
        || pr.draft != view.draft
        || pr.state != view.state.as_str();
    if changed {
        pr.url = view.url.clone();
        pr.number = view.number;
        pr.draft = view.draft;
        pr.state = view.state.as_str().to_string();
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

    let forge = state.forge_for_dir(&dir);
    let view = tauri::async_runtime::spawn_blocking(move || {
        forge.create_draft(Path::new(&dir), &pr_title, &pr_body, Some(&base))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(forge_err)?;

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

/// Refresh the recorded PR state from `gh pr view` — the primary *and* every
/// linked PR. Throttled in-memory (30s unless `force`); meta is written only
/// when something changed, so polling never feeds the FS watcher. The
/// **primary** PR observed as MERGED moves the item to `done` — linked PRs
/// are tracked, never drivers, with one exception: an item that has *only*
/// linked PRs has no primary to drive it, so all of them merging closes it
/// (see `linked_only_all_merged`). A linked PR that fails to refresh keeps
/// its previous state (best-effort, like the unanswered count).
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
    let primary = meta.pr.clone().filter(|p| !p.url.is_empty());
    if primary.is_none() && meta.linked_prs.is_empty() {
        return Err("No PR recorded for this item".to_string());
    }

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
    // Prefer the number (from the record, or derived from the URL), then the
    // branch — a URL-only record must still be refreshable, it is the state
    // the agent contract deliberately produces.
    let primary_selector = primary.as_ref().map(|pr| {
        let number = recorded_pr_number(pr);
        if number > 0 {
            number.to_string()
        } else {
            meta.branch.clone()
        }
    });
    // A linked PR lives in another repository, so every call about it is
    // scoped by the `owner/repo` its URL names; an unparseable URL is skipped
    // (nothing to ask gh about).
    let linked_ids: Vec<(usize, String, u64)> = meta
        .linked_prs
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            clash::infrastructure::gh::parse_pr_url(&p.url).map(|(repo, n)| (i, repo, n))
        })
        .collect();

    type LinkedRefresh = (usize, clash::domain::forge::ChangeView, Option<u64>);
    let forge = state.forge_for_dir(&dir);
    let dir_owned = dir.clone();
    let (primary_result, linked_results) = tauri::async_runtime::spawn_blocking(move || {
        let d = Path::new(&dir_owned);
        let primary = primary_selector.map(|selector| {
            forge.view(d, &selector, None).map(|view| {
                // Best-effort: a failed count keeps the previous value rather
                // than failing the refresh — a button label, not PR state.
                let unanswered = (view.number > 0)
                    .then(|| forge.unanswered_review_comments(d, view.number, None).ok())
                    .flatten();
                (view, unanswered)
            })
        });
        let linked: Vec<LinkedRefresh> = linked_ids
            .into_iter()
            .filter_map(|(i, repo, n)| {
                let view = forge.view(d, &n.to_string(), Some(&repo)).ok()?;
                let unanswered = forge.unanswered_review_comments(d, n, Some(&repo)).ok();
                Some((i, view, unanswered))
            })
            .collect();
        (primary, linked)
    })
    .await
    .map_err(|e| e.to_string())?;

    let mut changed = false;
    let mut primary_merged = false;
    if let Some(result) = primary_result {
        let (view, unanswered) = result.map_err(forge_err)?;
        changed |= merge_pr_view(&mut meta, &view);
        if let (Some(n), Some(pr)) = (unanswered, meta.pr.as_mut()) {
            if pr.unanswered_comments != Some(n) {
                pr.unanswered_comments = Some(n);
                changed = true;
            }
        }
        primary_merged = view.state == clash::domain::forge::ChangeState::Merged;
    }
    for (i, view, unanswered) in linked_results {
        let Some(pr) = meta.linked_prs.get_mut(i) else {
            continue;
        };
        let state_str = view.state.as_str();
        if pr.number != view.number || pr.draft != view.draft || pr.state != state_str {
            pr.number = view.number;
            pr.draft = view.draft;
            pr.state = state_str.to_string();
            pr.last_checked_at = now_ms();
            changed = true;
        }
        if let Some(n) = unanswered {
            if pr.unanswered_comments != Some(n) {
                pr.unanswered_comments = Some(n);
                changed = true;
            }
        }
    }
    let all_linked_merged = clash::application::workflow::linked_only_all_merged(&meta);
    if (primary_merged || all_linked_merged) && meta.status.can_transition_to(WorkflowStatus::Done)
    {
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
    let number = meta
        .pr
        .as_ref()
        .map(recorded_pr_number)
        .filter(|&n| n > 0)
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
    let forge = state.forge_for_dir(&dir);
    tauri::async_runtime::spawn_blocking(move || forge.comment(Path::new(&dir), number, &body))
        .await
        .map_err(|e| e.to_string())?
        .map_err(forge_err)?;
    Ok(round)
}

/// Flip the draft PR to ready-for-review (`gh pr ready`) — the validation
/// act. Moves the item to `pr-ready`.
#[tauri::command]
pub(crate) async fn mark_workflow_pr_ready(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    include_linked: Option<bool>,
) -> Result<MarkReadyOutcome, String> {
    let mut meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    // Identity-shaped failures carry machine prefixes (`no-pr:` /
    // `pr-number-unknown:`): the frontend turns them into "paste the PR URL"
    // prompts that attach and retry, so a data gap never dead-ends the user.
    let Some(pr) = meta.pr.clone() else {
        return Err("no-pr: this item has no pull request recorded yet".to_string());
    };
    let number = recorded_pr_number(&pr);
    if number == 0 {
        return Err(format!(
            "pr-number-unknown: cannot tell the PR number from '{}'",
            pr.url
        ));
    }
    let dir = pr_dir(&meta)?;
    // Multi-repo validation: the linked drafts to flip alongside the primary,
    // each repo-scoped by its URL. Only the primary's failure fails the
    // command — linked flips are best-effort with the outcomes reported.
    let linked_targets: Vec<(usize, String, u64)> = if include_linked.unwrap_or(false) {
        meta.linked_prs
            .iter()
            .enumerate()
            .filter(|(_, p)| p.draft && p.state != "MERGED" && p.state != "CLOSED")
            .filter_map(|(i, p)| {
                clash::infrastructure::gh::parse_pr_url(&p.url).map(|(repo, n)| (i, repo, n))
            })
            .collect()
    } else {
        Vec::new()
    };
    let forge = state.forge_for_dir(&dir);
    let dir_owned = dir.clone();
    type LinkedFlip = (usize, Result<(), String>);
    let (primary_result, linked_results): (Result<(), _>, Vec<LinkedFlip>) =
        tauri::async_runtime::spawn_blocking(move || {
            let d = Path::new(&dir_owned);
            let primary = forge.mark_ready(d, number, None);
            let linked = linked_targets
                .into_iter()
                .map(|(i, repo, n)| {
                    (
                        i,
                        forge
                            .mark_ready(d, n, Some(&repo))
                            .map_err(|e| e.to_string()),
                    )
                })
                .collect();
            (primary, linked)
        })
        .await
        .map_err(|e| e.to_string())?;
    primary_result.map_err(forge_err)?;

    if let Some(pr) = meta.pr.as_mut() {
        pr.number = number; // heal a URL-only record while we're writing anyway
        pr.draft = false;
        pr.last_checked_at = now_ms();
    }
    let mut linked_flipped = Vec::new();
    let mut linked_failed = Vec::new();
    for (i, result) in linked_results {
        let Some(pr) = meta.linked_prs.get_mut(i) else {
            continue;
        };
        match result {
            Ok(()) => {
                pr.draft = false;
                pr.last_checked_at = now_ms();
                linked_flipped.push(pr.url.clone());
            }
            Err(e) => linked_failed.push(format!("{}: {}", pr.url, e)),
        }
    }
    if meta.status.can_transition_to(WorkflowStatus::PrReady) {
        meta.status = WorkflowStatus::PrReady;
    }
    state
        .backend
        .write_workflow_meta(&project, &slug, &meta)
        .map_err(e2s)?;
    seed_local(&state, &project, &slug, meta.status);
    Ok(MarkReadyOutcome {
        meta,
        linked_flipped,
        linked_failed,
    })
}

/// What `mark_workflow_pr_ready` did: the updated meta plus, when linked
/// drafts were included, which flipped and which failed (best-effort — a
/// failed linked flip never rolls back the primary's).
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MarkReadyOutcome {
    meta: clash::domain::workflow::WorkflowMeta,
    linked_flipped: Vec<String>,
    linked_failed: Vec<String>,
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
    // Attach doubles as the recovery path for identity-shaped PR errors, so
    // it must never move an item BACKWARDS: re-attaching at `pr-ready` (whose
    // → pr-draft edge is legal) keeps the status.
    if meta.status != WorkflowStatus::PrReady
        && meta.status.can_transition_to(WorkflowStatus::PrDraft)
    {
        meta.status = WorkflowStatus::PrDraft;
    }
    state
        .backend
        .write_workflow_meta(&project, &slug, &meta)
        .map_err(e2s)?;
    seed_local(&state, &project, &slug, meta.status);

    // Best-effort detail fill; ignored when the forge tool is unavailable.
    let dir = pr_dir(&meta)?;
    let selector = number.to_string();
    let forge = state.forge_for_dir(&dir);
    if let Ok(Ok(view)) =
        tauri::async_runtime::spawn_blocking(move || forge.view(Path::new(&dir), &selector, None))
            .await
            .map(|r| r.map_err(forge_err))
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

/// Link a PR from *another* repository to this item (a backend/frontend/
/// contract split lands as several PRs; the item tracks all of them). The
/// linked list never drives the item's status — that stays the primary PR's
/// job, which is also why this command, unlike `attach_workflow_pr`, touches
/// no status at all.
#[tauri::command]
pub(crate) async fn attach_workflow_linked_pr(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    url: String,
) -> Result<clash::domain::workflow::WorkflowMeta, String> {
    let url = url.trim().to_string();
    let (repo, number) = clash::infrastructure::gh::parse_pr_url(&url)
        .ok_or_else(|| format!("Not a GitHub PR URL: {}", url))?;
    let mut meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    // Identity is `owner/repo#number` (two URL spellings of one PR must not
    // create two rows) — refuse duplicates of the primary or of the list.
    let same = |u: &str| {
        clash::infrastructure::gh::parse_pr_url(u).is_some_and(|(r, n)| r == repo && n == number)
    };
    if meta.pr.as_ref().is_some_and(|p| same(&p.url)) {
        return Err("That PR is already this item's primary PR".to_string());
    }
    if meta.linked_prs.iter().any(|p| same(&p.url)) {
        return Err("That PR is already linked to this item".to_string());
    }
    meta.linked_prs.push(clash::domain::workflow::WorkflowPr {
        url: url.clone(),
        number,
        ..Default::default()
    });
    state
        .backend
        .write_workflow_meta(&project, &slug, &meta)
        .map_err(e2s)?;
    seed_local(&state, &project, &slug, meta.status);

    // Best-effort detail fill (state, draft) — ignored when gh is
    // unavailable; the next refresh heals it, same as the primary attach.
    if let Ok(dir) = pr_dir(&meta) {
        let forge = state.forge_for_dir(&dir);
        let selector = number.to_string();
        let scope = repo.clone();
        if let Ok(Ok(view)) = tauri::async_runtime::spawn_blocking(move || {
            forge.view(Path::new(&dir), &selector, Some(&scope))
        })
        .await
        {
            if let Some(pr) = meta.linked_prs.iter_mut().find(|p| p.url == url) {
                pr.number = view.number;
                pr.draft = view.draft;
                pr.state = view.state.as_str().to_string();
                pr.last_checked_at = now_ms();
                state
                    .backend
                    .write_workflow_meta(&project, &slug, &meta)
                    .map_err(e2s)?;
            }
        }
    }
    Ok(meta)
}

/// Unlink a linked PR (by its recorded URL). The PR itself is untouched —
/// this only stops clash from tracking it on this item.
#[tauri::command]
pub(crate) fn remove_workflow_linked_pr(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    url: String,
) -> Result<clash::domain::workflow::WorkflowMeta, String> {
    let mut meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    let before = meta.linked_prs.len();
    meta.linked_prs.retain(|p| p.url != url);
    if meta.linked_prs.len() == before {
        return Err(format!("No linked PR with URL {}", url));
    }
    state
        .backend
        .write_workflow_meta(&project, &slug, &meta)
        .map_err(e2s)?;
    seed_local(&state, &project, &slug, meta.status);
    Ok(meta)
}

// ── Share & export ──────────────────────────────────────────────────────

/// The share/notify settings, as one row for the Settings panel.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShareSettings {
    slack_webhook: String,
    discord_webhook: String,
    notify_webhook: String,
    jira_base_url: String,
    jira_email: String,
    jira_api_token: String,
}

fn share_settings(state: &GuiState) -> ShareSettings {
    let cfg = state.config.get();
    ShareSettings {
        slack_webhook: cfg.workflows.slack_webhook.trim().to_string(),
        discord_webhook: cfg.workflows.discord_webhook.trim().to_string(),
        notify_webhook: cfg.workflows.notify_webhook.trim().to_string(),
        jira_base_url: cfg.workflows.jira_base_url.trim().to_string(),
        jira_email: cfg.workflows.jira_email.trim().to_string(),
        jira_api_token: cfg.workflows.jira_api_token.trim().to_string(),
    }
}

#[tauri::command]
pub(crate) fn get_workflow_share_settings(state: State<'_, GuiState>) -> ShareSettings {
    share_settings(&state)
}

/// Patch the share/notify settings (the Settings panel's Workflows group).
/// Every field is optional — `Some` sets, `None` leaves the current value —
/// one command for the group, like `set_workflow_item_settings`. Values equal
/// to the schema default are reset (the key leaves config.toml) rather than
/// written out.
#[tauri::command]
pub(crate) fn set_workflow_share_settings(
    state: State<'_, GuiState>,
    slack_webhook: Option<String>,
    discord_webhook: Option<String>,
    notify_webhook: Option<String>,
    jira_base_url: Option<String>,
    jira_email: Option<String>,
    jira_api_token: Option<String>,
) -> Result<ShareSettings, String> {
    let mut sets: Vec<(&str, serde_json::Value)> = Vec::new();
    let mut resets: Vec<&str> = Vec::new();
    let mut url_setting = |key: &'static str, value: Option<String>| -> Result<(), String> {
        let Some(value) = value else { return Ok(()) };
        let value = value.trim().to_string();
        if value.is_empty() {
            resets.push(key);
        } else if !clash::infrastructure::webhook::valid_url(&value) {
            return Err(format!("Not an http(s) URL: {}", value));
        } else {
            sets.push((key, serde_json::Value::String(value)));
        }
        Ok(())
    };
    url_setting("workflows.slack_webhook", slack_webhook)?;
    url_setting("workflows.discord_webhook", discord_webhook)?;
    url_setting("workflows.jira_base_url", jira_base_url)?;
    let mut text_setting = |key: &'static str, value: Option<String>| {
        let Some(value) = value else { return };
        let value = value.trim().to_string();
        if value.is_empty() {
            resets.push(key);
        } else {
            sets.push((key, serde_json::Value::String(value)));
        }
    };
    text_setting("workflows.jira_email", jira_email);
    text_setting("workflows.jira_api_token", jira_api_token);
    if let Some(notify) = notify_webhook {
        let notify = notify.trim().to_ascii_lowercase();
        if !["off", "slack", "discord"].contains(&notify.as_str()) {
            return Err(format!("Unknown notify destination '{}'", notify));
        }
        if notify == "off" {
            resets.push("workflows.notify_webhook");
        } else {
            sets.push((
                "workflows.notify_webhook",
                serde_json::Value::String(notify),
            ));
        }
    }
    if !sets.is_empty() {
        state.config.set_json(&sets).map_err(|e| e.to_string())?;
    }
    if !resets.is_empty() {
        state
            .config
            .reset_values(&resets)
            .map_err(|e| e.to_string())?;
    }
    Ok(share_settings(&state))
}

/// Compose the share document for an item — the exact markdown every
/// destination sends, built by the pure core builder so the dialog's preview
/// *is* the payload. The diff is only read when the caller asked for it.
#[tauri::command]
pub(crate) async fn build_workflow_share(
    state: State<'_, GuiState>,
    project: String,
    slug: String,
    sections: clash::application::workflow_share::ShareSections,
) -> Result<String, String> {
    let meta = state
        .backend
        .load_workflow_meta(&project, &slug)
        .map_err(e2s)?;
    let read = |doc: &str| {
        state
            .backend
            .read_workflow_doc(&project, &slug, doc)
            .unwrap_or_default()
    };
    let plan = if sections.plan {
        read(clash::infrastructure::fs::workflows::PLAN_FILE)
    } else {
        String::new()
    };
    let review_md = if sections.timeline {
        read(clash::infrastructure::fs::workflows::REVIEW_FILE)
    } else {
        String::new()
    };
    let agent_md = if sections.reviews {
        read(clash::infrastructure::fs::workflows::AGENT_REVIEW_FILE)
    } else {
        String::new()
    };
    let annotations = if sections.annotations {
        state
            .backend
            .load_workflow_annotations(&project, &slug)
            .map(|f| f.annotations)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let diff = if sections.diff {
        workflow_diff_text(&state, &project, &slug, None)
            .await
            .unwrap_or_default()
    } else {
        String::new()
    };
    Ok(clash::application::workflow_share::build_share_markdown(
        &clash::application::workflow_share::ShareInput {
            meta: &meta,
            project: &project,
            slug: &slug,
            plan: &plan,
            review_md: &review_md,
            agent_review_md: &agent_md,
            annotations: &annotations,
            diff: &diff,
        },
        &sections,
    ))
}

/// POST a share document to the configured Slack/Discord webhook. Returns
/// whether the message was truncated to the service's size limit. Only ever
/// called from the share dialog's explicit send — never automatically.
#[tauri::command]
pub(crate) async fn share_workflow_webhook(
    state: State<'_, GuiState>,
    kind: String,
    text: String,
) -> Result<bool, String> {
    use clash::infrastructure::webhook::{self, WebhookKind};
    let kind = WebhookKind::parse(&kind).ok_or_else(|| format!("Unknown webhook '{}'", kind))?;
    let cfg = state.config.get();
    let url = match kind {
        WebhookKind::Slack => cfg.workflows.slack_webhook.clone(),
        WebhookKind::Discord => cfg.workflows.discord_webhook.clone(),
    };
    if url.trim().is_empty() {
        return Err(format!(
            "no {} webhook configured — set it in Settings → Workflows",
            kind.as_str()
        ));
    }
    tauri::async_runtime::spawn_blocking(move || webhook::send(kind, &url, &text))
        .await
        .map_err(|e| e.to_string())?
}

/// POST a share document as one comment on a Jira ticket. The markdown is
/// converted to Jira wiki markup and truncated to the comment limit; returns
/// whether it was truncated. Only ever called from the share dialog's
/// explicit send — never automatically.
#[tauri::command]
pub(crate) async fn share_workflow_jira(
    state: State<'_, GuiState>,
    ticket: String,
    text: String,
) -> Result<bool, String> {
    let cfg = state.config.get();
    let (base, email, token) = (
        cfg.workflows.jira_base_url.clone(),
        cfg.workflows.jira_email.clone(),
        cfg.workflows.jira_api_token.clone(),
    );
    if base.trim().is_empty() || email.trim().is_empty() || token.trim().is_empty() {
        return Err(
            "Jira is not configured — set the site URL, email and API token in Settings → \
             Workflows"
                .to_string(),
        );
    }
    tauri::async_runtime::spawn_blocking(move || {
        clash::infrastructure::jira::post_comment(&base, &email, &token, &ticket, &text)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Write an export (`.md` / `.html`) to `dir` (the folder the user picked;
/// their Downloads folder when omitted). The filename is clash-derived —
/// never caller-supplied, so this stays an export command rather than a
/// generic write-any-file hole. An existing file gets `-2`, `-3`, … instead
/// of being overwritten. Returns the written path.
#[tauri::command]
pub(crate) fn export_workflow_share(
    project: String,
    slug: String,
    format: String,
    content: String,
    dir: Option<String>,
) -> Result<String, String> {
    let ext = match format.as_str() {
        "md" | "html" => format.as_str(),
        other => return Err(format!("Unknown export format '{}'", other)),
    };
    let dir = match dir.map(|d| d.trim().to_string()).filter(|d| !d.is_empty()) {
        Some(d) => crate::expand_tilde(&d),
        None => dirs::download_dir()
            .or_else(dirs::home_dir)
            .ok_or_else(|| "Cannot resolve a folder to export into".to_string())?,
    };
    if !dir.is_dir() {
        return Err(format!("Not a directory: {}", dir.display()));
    }
    // project/slug come from the item listing (directory names), but stay
    // defensive: a path separator in the filename would escape `dir`.
    let clean = |s: &str| {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>()
    };
    let base = format!("clash-{}-{}", clean(&project), clean(&slug));
    let mut path = dir.join(format!("{}.{}", base, ext));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{}-{}.{}", base, n, ext));
        n += 1;
    }
    std::fs::write(&path, content)
        .map_err(|e| format!("Cannot write {}: {}", path.display(), e))?;
    Ok(path.to_string_lossy().into_owned())
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
